//! MPRIS integration.
//!
//! Exports the player on the session D-Bus as
//! `org.mpris.MediaPlayer2.YandexMusic`, so media keys and tools like
//! `playerctl` can control playback. All D-Bus work runs on a dedicated thread
//! with its own tokio runtime because `mpris-server`'s `Player` is not `Send`;
//! the main thread only pushes state updates through an unbounded channel.
use std::sync::mpsc::Sender;

use mpris_server::{Metadata, PlaybackStatus, Player, Time, TrackId, Volume};
use tokio::sync::mpsc::unbounded_channel;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::api::models::Track;
use crate::state::{AppEvent, RemoteCommand};

/// State updates pushed from the main thread to the MPRIS service thread.
enum Update {
    Status(PlaybackStatus),
    NowPlaying {
        title: String,
        artists: Vec<String>,
        album: String,
        duration_ms: u64,
        track_id: String,
        art_url: Option<String>,
    },
    Position(Time),
    Volume(Volume),
    Seeked(Time),
}

/// A handle for pushing playback state to the MPRIS service. Cheap to clone.
#[derive(Clone)]
pub struct Mpris {
    tx: tokio::sync::mpsc::UnboundedSender<Update>,
}

impl Mpris {
    /// Spawn the MPRIS service on a background thread. D-Bus registration
    /// failures (e.g. no session bus) are logged and the thread exits silently.
    pub fn spawn(ev: Sender<AppEvent>) -> Self {
        let (tx, rx) = unbounded_channel::<Update>();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to build the MPRIS runtime");
            runtime.block_on(service(rx, ev));
        });
        Self { tx }
    }

    pub fn set_playing(&self, playing: bool) {
        let status = if playing {
            PlaybackStatus::Playing
        } else {
            PlaybackStatus::Paused
        };
        let _ = self.tx.send(Update::Status(status));
    }

    pub fn set_stopped(&self) {
        let _ = self.tx.send(Update::Status(PlaybackStatus::Stopped));
    }

    pub fn set_track(&self, track: &Track) {
        let _ = self.tx.send(Update::NowPlaying {
            title: track.title.clone(),
            artists: track.artists.iter().map(|a| a.name.clone()).collect(),
            album: track
                .albums
                .first()
                .map(|a| a.title.clone())
                .unwrap_or_default(),
            duration_ms: track.duration_ms.unwrap_or(0).max(0) as u64,
            track_id: track.id.as_ref().map(|id| id.0.clone()).unwrap_or_default(),
            art_url: crate::api::covers::track_cover(track, 400),
        });
    }

    pub fn set_position_secs(&self, seconds: u64) {
        let _ = self
            .tx
            .send(Update::Position(Time::from_secs(seconds as i64)));
    }

    pub fn set_volume(&self, volume: f64) {
        let _ = self.tx.send(Update::Volume(volume));
    }

    /// Announce a user-visible seek (MPRIS `Seeked` signal).
    pub fn seeked(&self, seconds: u64) {
        let _ = self
            .tx
            .send(Update::Seeked(Time::from_secs(seconds as i64)));
    }
}

async fn service(mut rx: UnboundedReceiver<Update>, ev: Sender<AppEvent>) {
    let player = match Player::builder("YandexMusic")
        .identity("Yandex Music")
        .desktop_entry("yandex-music")
        .can_play(true)
        .can_pause(true)
        .can_go_next(true)
        .can_go_previous(true)
        .can_seek(true)
        .build()
        .await
    {
        Ok(player) => player,
        Err(error) => {
            eprintln!("MPRIS unavailable: {error}");
            return;
        }
    };

    {
        let ev = ev.clone();
        player.connect_play_pause(move |_| {
            let _ = ev.send(AppEvent::RemoteCommand(RemoteCommand::Toggle));
        });
    }
    {
        let ev = ev.clone();
        player.connect_play(move |_| {
            let _ = ev.send(AppEvent::RemoteCommand(RemoteCommand::Play));
        });
    }
    {
        let ev = ev.clone();
        player.connect_pause(move |_| {
            let _ = ev.send(AppEvent::RemoteCommand(RemoteCommand::Pause));
        });
    }
    {
        let ev = ev.clone();
        player.connect_next(move |_| {
            if std::env::var("YM_DEBUG").is_ok() {
                eprintln!("[dbg] mpris Next method dispatched");
            }
            let _ = ev.send(AppEvent::RemoteCommand(RemoteCommand::Next));
        });
    }
    {
        let ev = ev.clone();
        player.connect_previous(move |_| {
            let _ = ev.send(AppEvent::RemoteCommand(RemoteCommand::Previous));
        });
    }
    {
        let ev = ev.clone();
        player.connect_seek(move |_, offset| {
            let _ = ev.send(AppEvent::RemoteCommand(RemoteCommand::SeekRelative {
                offset_seconds: offset.as_secs(),
            }));
        });
    }
    {
        let ev = ev.clone();
        player.connect_set_position(move |_, _track_id, position| {
            let _ = ev.send(AppEvent::RemoteCommand(RemoteCommand::SetPosition {
                seconds: position.as_secs().max(0) as u64,
            }));
        });
    }
    {
        let ev = ev.clone();
        player.connect_set_volume(move |_, volume| {
            let _ = ev.send(AppEvent::RemoteCommand(RemoteCommand::SetVolume(volume)));
        });
    }
    {
        let ev = ev.clone();
        player.connect_quit(move |_| {
            let _ = ev.send(AppEvent::RemoteCommand(RemoteCommand::Quit));
        });
    }
    {
        let ev = ev.clone();
        player.connect_raise(move |_| {
            let _ = ev.send(AppEvent::RemoteCommand(RemoteCommand::Raise));
        });
    }

    let mut run_task = player.run();

    loop {
        tokio::select! {
            _ = &mut run_task => break,
            update = rx.recv() => {
                let Some(update) = update else { break };
                let result = match update {
                    Update::Status(status) => player.set_playback_status(status).await,
                    Update::NowPlaying { title, artists, album, duration_ms, track_id, art_url } => {
                        player.set_metadata(metadata_for(title, artists, album, duration_ms, track_id, art_url)).await
                    }
                    Update::Position(position) => {
                        player.set_position(position);
                        Ok(())
                    }
                    Update::Volume(volume) => player.set_volume(volume).await,
                    Update::Seeked(position) => player.seeked(position).await,
                };
                if let Err(error) = result {
                    eprintln!("MPRIS update failed: {error}");
                }
            }
        }
    }
}

fn metadata_for(
    title: String,
    artists: Vec<String>,
    album: String,
    duration_ms: u64,
    track_id: String,
    art_url: Option<String>,
) -> Metadata {
    let safe_id = track_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect::<String>();
    let safe_id: &str = if safe_id.is_empty() {
        "unknown"
    } else {
        safe_id.as_str()
    };
    let track_id =
        TrackId::try_from(format!("/dev/ymapp/track/{safe_id}")).unwrap_or(TrackId::NO_TRACK);
    let mut builder = Metadata::builder()
        .title(title)
        .artist(artists)
        .album(album)
        .length(Time::from_millis(duration_ms as i64))
        .trackid(track_id);
    if let Some(art_url) = art_url {
        builder = builder.art_url(art_url);
    }
    builder.build()
}
