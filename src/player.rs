//! GStreamer playback wrapper around `gst-play` (the `Play` convenience API).
//!
//! The player is created on the GTK main thread and controlled from there. All
//! asynchronous signals (end-of-stream, errors, position/duration updates) are
//! delivered to the `on_message` callback, which the caller wires to the main
//! loop event channel so UI updates always happen on the main thread.
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer::ClockTime;
use gstreamer_play::{Play, PlayMessage, PlayState};
use gtk4::glib;

/// Events raised by the underlying `Play` pipeline.
#[derive(Clone, Debug)]
pub enum PlayerEvent {
    EndOfStream,
    Error(String),
    StateChanged(PlayState),
    Position(ClockTime),
    Duration(ClockTime),
}

/// A thin, clonable handle around a `gst-play` pipeline.
#[derive(Clone)]
pub struct Player {
    play: Play,
    /// Keeps the bus watch attached for the lifetime of the player.
    _bus_watch: std::sync::Arc<gstreamer::bus::BusWatchGuard>,
}

impl Player {
    /// Create the player. `on_message` receives asynchronous pipeline events and
    /// must be `Send` (it is invoked from the GStreamer bus watch thread).
    pub fn new(on_message: impl Fn(PlayerEvent) + Send + Sync + 'static) -> Self {
        gst::init().expect("failed to initialize GStreamer");
        let play = Play::new(None::<gstreamer_play::PlayVideoRenderer>);
        let on_message = std::sync::Arc::new(on_message);
        let bus_watch = play
            .message_bus()
            .add_watch({
                let on_message = std::sync::Arc::clone(&on_message);
                move |_bus, msg| {
                    // The element that raised the error (e.g. `playbin3`,
                    // `souphttpsrc`), so the message is actionable.
                    let element = msg
                        .src()
                        .map(|src| src.name().to_string())
                        .unwrap_or_default();
                    if let Ok(play_msg) = PlayMessage::parse(msg) {
                        let event = match play_msg {
                            PlayMessage::EndOfStream => Some(PlayerEvent::EndOfStream),
                            PlayMessage::Error { error, details } => {
                                let mut message = if element.is_empty() {
                                    error.to_string()
                                } else {
                                    format!("{element}: {}", error)
                                };
                                if let Some(details) = details {
                                    // Debug info is multi-line and noisy; keep
                                    // just the first line and cap its length.
                                    let first = details
                                        .to_string()
                                        .lines()
                                        .next()
                                        .unwrap_or("")
                                        .trim()
                                        .to_string();
                                    let capped = if first.len() > 100 {
                                        let mut s: String = first.chars().take(100).collect();
                                        s.push('…');
                                        s
                                    } else {
                                        first
                                    };
                                    if !capped.is_empty() {
                                        message.push_str(&format!(" ({capped})"));
                                    }
                                }
                                Some(PlayerEvent::Error(message))
                            }
                            PlayMessage::StateChanged { state } => {
                                if std::env::var("YM_DEBUG").is_ok() {
                                    eprintln!("[dbg] state changed: {state:?}");
                                }
                                Some(PlayerEvent::StateChanged(state))
                            }
                            PlayMessage::PositionUpdated { position } => {
                                if std::env::var("YM_DEBUG").is_ok() {
                                    if let Some(p) = position {
                                        eprintln!("[dbg] position: {}s", p.seconds());
                                    }
                                }
                                position.map(PlayerEvent::Position)
                            }
                            PlayMessage::DurationChanged { duration } => {
                                if std::env::var("YM_DEBUG").is_ok() {
                                    if let Some(d) = duration {
                                        eprintln!("[dbg] duration: {}s", d.seconds());
                                    }
                                }
                                duration.map(PlayerEvent::Duration)
                            }
                            _ => None,
                        };
                        if let Some(event) = event {
                            on_message(event);
                        }
                    }
                    glib::ControlFlow::Continue
                }
            })
            .expect("failed to attach bus watch");

        Self {
            play,
            _bus_watch: std::sync::Arc::new(bus_watch),
        }
    }

    /// Load a URI and start playing it.
    pub fn play_url(&self, url: &str) {
        if std::env::var("YM_DEBUG").is_ok() {
            let (_, state, _) = self.play.pipeline().state(Some(ClockTime::from_seconds(0)));
            eprintln!(
                "[dbg] play_url: state={state:?} before set_uri({})",
                &url[..url.len().min(60)]
            );
        }
        // Reset the pipeline before switching URIs. `set_uri` on its own can be
        // a silent no-op when called while the pipeline is playing (a known
        // gst-play race), which leaves the previous track audible while the UI
        // has already moved on. Clearing the URI first forces a full teardown
        // and reload regardless of the pipeline's current state or whether the
        // target URI happens to equal the current one.
        self.play.set_uri(None::<&str>);
        self.play.set_uri(Some(url));
        self.play.play();
        if std::env::var("YM_DEBUG").is_ok() {
            let uri = self.play.uri();
            let same = uri.as_deref() == Some(url);
            eprintln!("[dbg] after set_uri: play.uri() same-as-requested={same}");
        }
    }

    /// Pause playback (position is preserved).
    pub fn pause(&self) {
        self.play.pause();
    }

    /// Resume playback after a pause.
    pub fn resume(&self) {
        self.play.play();
    }

    /// Toggle play/pause based on the current pipeline state.
    pub fn toggle(&self) {
        if self.is_playing() {
            self.pause();
        } else {
            self.resume();
        }
    }

    /// Stop playback and drop the loaded URI.
    pub fn stop(&self) {
        self.play.stop();
    }

    /// Seek to an absolute position.
    pub fn seek(&self, position: ClockTime) {
        self.play.seek(position);
    }

    /// Temporary: playback speed for headless reproduction.
    pub fn set_rate(&self, rate: f64) -> bool {
        let pipeline = self.play.pipeline();
        let position = pipeline
            .query_position::<gst::ClockTime>()
            .unwrap_or(gst::ClockTime::ZERO);
        pipeline
            .seek(
                rate,
                gst::SeekFlags::FLUSH,
                gst::SeekType::Set,
                position,
                gst::SeekType::None,
                gst::ClockTime::NONE,
            )
            .is_ok()
    }

    /// Set volume in `[0.0, 1.0]`.
    pub fn set_volume(&self, volume: f64) {
        self.play.set_volume(volume);
    }

    /// Current volume in `[0.0, 1.0]`.
    pub fn volume(&self) -> f64 {
        self.play.volume()
    }

    /// Current playback position, if known.
    pub fn position(&self) -> Option<ClockTime> {
        self.play.position()
    }

    /// Duration of the current media, if known.
    pub fn duration(&self) -> Option<ClockTime> {
        self.play.duration()
    }

    /// Whether the pipeline is currently in the playing state.
    pub fn is_playing(&self) -> bool {
        let (_, current, _) = self.play.pipeline().state(Some(ClockTime::from_seconds(0)));
        current == gst::State::Playing
    }
}

/// Format a position as `m:ss`.
pub fn format_time(time: Option<ClockTime>) -> String {
    let seconds = time.map(|t| t.seconds()).unwrap_or(0);
    let minutes = seconds / 60;
    format!("{minutes}:{:02}", seconds % 60)
}
