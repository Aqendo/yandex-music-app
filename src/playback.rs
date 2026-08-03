//! Playback queue controller.
//!
//! Owns the [`Player`], the track queue and the cache of resolved stream URLs.
//! Runs entirely on the GTK main thread; URLs are resolved asynchronously by the
//! worker (via `WorkerCommand::PlayTrack`) and handed back through
//! `AppEvent::TrackStreamReady`.
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc::Sender;

use gstreamer::ClockTime;
use gstreamer_play::PlayState;

use crate::api::models::Track;
use crate::player::{Player, PlayerEvent};
use crate::state::{AppEvent, WorkerCommand};
use crate::worker::Worker;

/// Temporary debug logging, enabled with `YM_DEBUG=1`.
fn dbg(msg: String) {
    if std::env::var("YM_DEBUG").is_ok() {
        eprintln!("[dbg] {msg}");
    }
}

/// Callback invoked when the queue runs out (main thread only).
type ExhaustedCallback = Rc<RefCell<Option<Box<dyn Fn()>>>>;

/// Callback invoked whenever a seek is requested, with the target position in
/// seconds (main thread only). Fires for GUI and remote seeks alike.
type SeekCallback = Rc<RefCell<Option<Box<dyn Fn(u64)>>>>;

/// A queue-aware playback controller. Cheap to clone (shares one player).
#[derive(Clone)]
pub struct Playback {
    player: Player,
    worker: Worker,
    ev: Sender<AppEvent>,
    queue: Rc<RefCell<Vec<Track>>>,
    index: Rc<RefCell<usize>>,
    urls: Rc<RefCell<HashMap<String, String>>>,
    pending: Rc<RefCell<Option<Track>>>,
    on_exhausted: ExhaustedCallback,
    on_seek: SeekCallback,
}

impl Playback {
    /// Build the controller. Pipeline events (end-of-stream, errors) are routed
    /// through `ev` so they are handled on the main thread.
    pub fn new(worker: Worker, ev: Sender<AppEvent>) -> Self {
        let ev_for_player = ev.clone();
        let player = Player::new(move |event| {
            let message = match event {
                PlayerEvent::EndOfStream => Some(AppEvent::PlaybackEnded),
                PlayerEvent::Error(message) => Some(AppEvent::PlaybackError { message }),
                PlayerEvent::StateChanged(state) => {
                    Some(AppEvent::PlayStateChanged {
                        playing: state == PlayState::Playing,
                    })
                }
                _ => None,
            };
            if let Some(message) = message {
                let _ = ev_for_player.send(message);
            }
        });
        Self {
            player,
            worker,
            ev,
            queue: Rc::new(RefCell::new(Vec::new())),
            index: Rc::new(RefCell::new(0)),
            urls: Rc::new(RefCell::new(HashMap::new())),
            pending: Rc::new(RefCell::new(None)),
            on_exhausted: Rc::new(RefCell::new(None)),
            on_seek: Rc::new(RefCell::new(None)),
        }
    }

    /// Register a callback invoked (on the main thread) whenever the queue runs
    /// out — either from `next()` or after end-of-stream. Infinite sources use
    /// this to fetch a continuation batch.
    pub fn set_on_exhausted<F: Fn() + 'static>(&self, on_exhausted: F) {
        *self.on_exhausted.borrow_mut() = Some(Box::new(on_exhausted));
    }

    /// Register a callback invoked (on the main thread) whenever a seek is
    /// requested, with the target position in seconds. External integrations
    /// (MPRIS) use this to announce the seek immediately instead of waiting for
    /// the pipeline to report the new position.
    pub fn set_on_seek<F: Fn(u64) + 'static>(&self, on_seek: F) {
        *self.on_seek.borrow_mut() = Some(Box::new(on_seek));
    }

    /// Replace the queue with `tracks` and start playing at `start`.
    pub fn play_queue(&self, tracks: Vec<Track>, start: usize) {
        let len = tracks.len();
        self.queue.replace(tracks);
        self.index.replace(start.min(len.saturating_sub(1)));
        self.urls.borrow_mut().clear();
        self.pending.replace(None);
        dbg(format!("play_queue len={len} start={start}"));
        self.play_current();
    }

    /// Play the track at the current index, resolving its URL if needed.
    fn play_current(&self) {
        let Some(track) = self.current_track() else {
            return;
        };
        self.announce(&track);
        let key = track_id(&track);
        match self.urls.borrow().get(&key).cloned() {
            Some(url) => self.start_url(url),
            None => {
                dbg(format!("play_current {key}: url NOT cached, pending it"));
                *self.pending.borrow_mut() = Some(track.clone());
                self.worker.send(WorkerCommand::PlayTrack { track });
            }
        }
    }

    /// Handle a resolved stream URL coming back from the worker.
    pub fn on_stream_ready(&self, track: Track, url: String) {
        self.urls.borrow_mut().insert(track_id(&track), url.clone());
        let is_pending = self
            .pending
            .borrow()
            .as_ref()
            .map(|p| p.id == track.id)
            .unwrap_or(false);
        dbg(format!(
            "on_stream_ready {} is_pending={is_pending}",
            track_id(&track)
        ));
        if is_pending {
            self.pending.replace(None);
            self.start_url(url);
        }
    }

    /// Begin playback of `url` and prefetch the next track's URL.
    fn start_url(&self, url: String) {
        dbg(format!("start_url {}", &url[..url.len().min(60)]));
        self.player.play_url(&url);
        self.prefetch_next();
    }

    /// Ask the worker to resolve the next track's URL ahead of time.
    fn prefetch_next(&self) {
        let Some(next_index) = self.index.borrow().checked_add(1) else {
            return;
        };
        if let Some(next) = self.track_at(next_index) {
            if !self.urls.borrow().contains_key(&track_id(&next)) {
                self.worker.send(WorkerCommand::PlayTrack { track: next });
            }
        }
    }

    /// Advance to the next track, if any. Returns `false` when the queue is
    /// exhausted; the `on_exhausted` callback (if any) is then invoked so the
    /// caller can stop or fetch a continuation batch.
    pub fn next(&self) -> bool {
        let next = self.index.borrow().checked_add(1);
        let Some(next) = next else {
            dbg(format!(
                "next: index overflow, exhausted (len={})",
                self.queue.borrow().len()
            ));
            self.notify_exhausted();
            return false;
        };
        if next < self.queue.borrow().len() {
            dbg(format!(
                "next: {} -> {} (len={})",
                self.index.borrow(),
                next,
                self.queue.borrow().len()
            ));
            self.index.replace(next);
            self.play_current();
            true
        } else {
            dbg(format!(
                "next: at end ({} >= {}), exhausted",
                next,
                self.queue.borrow().len()
            ));
            self.notify_exhausted();
            false
        }
    }

    fn notify_exhausted(&self) {
        if let Some(callback) = self.on_exhausted.borrow().as_ref() {
            callback();
        }
    }

    /// Append tracks to the end of the queue, skipping duplicates.
    pub fn append_tracks(&self, tracks: &[Track]) {
        let mut queue = self.queue.borrow_mut();
        let before = queue.len();
        for track in tracks {
            let id = track_id(track);
            if !queue.iter().any(|t| track_id(t) == id) {
                queue.push(track.clone());
            }
        }
        dbg(format!("append_tracks: {} -> {} tracks", before, queue.len()));
    }

    /// Keep playing after a continuation batch: advance past the finished track
    /// and start the first newly appended one (if any).
    pub fn continue_after_batch(&self) {
        let next = self.index.borrow().checked_add(1);
        let Some(next) = next else {
            dbg("continue_after_batch: no next index".to_string());
            return;
        };
        if next < self.queue.borrow().len() {
            dbg(format!(
                "continue_after_batch: {} -> {} (len={})",
                self.index.borrow(),
                next,
                self.queue.borrow().len()
            ));
            self.index.replace(next);
            self.play_current();
        } else {
            dbg(format!(
                "continue_after_batch: no track at {} (len={})",
                next,
                self.queue.borrow().len()
            ));
        }
    }

    /// Stop the pipeline without touching the queue.
    pub fn stop(&self) {
        self.player.stop();
    }

    /// Go to the previous track (or restart the current one).
    pub fn prev(&self) {
        let i = self.index.borrow().checked_sub(1);
        let Some(prev) = i else {
            self.seek_seconds(0);
            self.player.resume();
            return;
        };
        if prev < self.queue.borrow().len() {
            self.index.replace(prev);
            self.play_current();
        } else {
            self.seek_seconds(0);
            self.player.resume();
        }
    }

    /// Toggle play/pause.
    pub fn toggle(&self) {
        self.player.toggle();
    }

    /// Resume playback if paused.
    pub fn play(&self) {
        self.player.resume();
    }

    /// Pause playback, preserving the position.
    pub fn pause(&self) {
        self.player.pause();
    }

    /// Seek to an absolute position in seconds.
    pub fn seek_seconds(&self, seconds: u64) {
        self.player.seek(ClockTime::from_seconds(seconds));
        if let Some(callback) = self.on_seek.borrow().as_ref() {
            callback(seconds);
        }
    }

    /// Seek by a (possibly negative) number of seconds relative to the
    /// current position, clamped to the start of the track.
    pub fn seek_relative(&self, offset_seconds: i64) {
        let current = self.position().map(|p| p.seconds()).unwrap_or(0) as i64;
        let target = (current + offset_seconds).max(0) as u64;
        self.seek_seconds(target);
    }

    /// Set volume in `[0.0, 1.0]`.
    pub fn set_volume(&self, volume: f64) {
        self.player.set_volume(volume);
    }

    /// Current volume in `[0.0, 1.0]`.
    pub fn volume(&self) -> f64 {
        self.player.volume()
    }

    /// Current playback position.
    pub fn position(&self) -> Option<ClockTime> {
        self.player.position()
    }

    /// Temporary: playback speed for headless reproduction.
    pub fn set_rate(&self, rate: f64) -> bool {
        self.player.set_rate(rate)
    }

    /// Duration of the current media.
    pub fn duration(&self) -> Option<ClockTime> {
        self.player.duration()
    }

    /// Whether playback is currently in the playing state.
    pub fn is_playing(&self) -> bool {
        self.player.is_playing()
    }

    /// The track at the current queue position, if any.
    pub fn current_track(&self) -> Option<Track> {
        self.track_at(*self.index.borrow())
    }

    /// Number of tracks in the queue.
    pub fn queue_len(&self) -> usize {
        self.queue.borrow().len()
    }

    fn track_at(&self, index: usize) -> Option<Track> {
        self.queue.borrow().get(index).cloned()
    }

    fn announce(&self, track: &Track) {
        let _ = self.ev.send(AppEvent::NowPlaying { track: track.clone() });
    }
}

fn track_id(track: &Track) -> String {
    track.id.as_ref().map(|id| id.0.clone()).unwrap_or_default()
}
