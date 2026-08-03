//! `ymapp` — the Yandex Music native GTK4 player library.
//!
//! The GUI binary (`yandex-music`) and the headless smoke-test tool (`ym-cli`)
//! share this crate.
pub mod api;
pub mod config;
pub mod integrations;
pub mod playback;
pub mod player;
pub mod state;
pub mod ui;
pub mod worker;
