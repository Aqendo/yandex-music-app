//! Desktop integrations: MPRIS (media keys / `playerctl`) and the system tray.
//!
//! Both components run off the GTK main thread and talk to it exclusively
//! through [`AppEvent::RemoteCommand`]; the main thread pushes state changes
//! back into them.
pub mod mpris;
pub mod tray;

pub use mpris::Mpris;
pub use tray::Tray;
