//! System tray integration via `ksni` (StatusNotifierItem).
//!
//! The tray owns no playback state of its own: menu activations are forwarded
//! to the UI thread as [`AppEvent::RemoteCommand`], and the UI thread refreshes
//! the tray (now-playing title, play/pause label) through its `Handle`.
use std::sync::mpsc::Sender;

use ksni::menu::{MenuItem, StandardItem};
use ksni::Tray as KsniTray;

use crate::api::models::Track;
use crate::state::{AppEvent, RemoteCommand};

/// Tray state. Owned by the `ksni` service thread; updated via `Handle::update`.
pub struct Tray {
    now_playing: String,
    playing: bool,
    ev: Sender<AppEvent>,
}

impl Tray {
    pub fn new(ev: Sender<AppEvent>) -> Self {
        Self {
            now_playing: "Yandex Music".to_string(),
            playing: false,
            ev,
        }
    }

    pub fn set_track(&mut self, track: &Track) {
        let title = track.title.clone();
        let artist = track
            .artists
            .first()
            .map(|a| a.name.clone())
            .unwrap_or_default();
        self.now_playing = if artist.is_empty() {
            title
        } else {
            format!("{title} — {artist}")
        };
    }

    pub fn set_playing(&mut self, playing: bool) {
        self.playing = playing;
    }
}

impl KsniTray for Tray {
    fn id(&self) -> String {
        "yandex-music".into()
    }

    fn title(&self) -> String {
        self.now_playing.clone()
    }

    fn icon_name(&self) -> String {
        "audio-x-generic".into()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: self.now_playing.clone(),
            description: "Yandex Music".into(),
            ..Default::default()
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            StandardItem {
                label: if self.playing { "Pause" } else { "Play" }.into(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.ev.send(AppEvent::RemoteCommand(RemoteCommand::Toggle));
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Next".into(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.ev.send(AppEvent::RemoteCommand(RemoteCommand::Next));
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Previous".into(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.ev.send(AppEvent::RemoteCommand(RemoteCommand::Previous));
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.ev.send(AppEvent::RemoteCommand(RemoteCommand::Quit));
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}
