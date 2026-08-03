//! `yandex-music` — the GTK4 application entry point.
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use ksni::blocking::TrayMethods;
use libadwaita as adw;
use libadwaita::prelude::*;

use ymapp::integrations::{Mpris, Tray};
use ymapp::playback::Playback;
use ymapp::state::{AppEvent, RemoteCommand, WorkerCommand};
use ymapp::ui::{LoginView, MainView};
use ymapp::worker::{EventSink, Worker};

const APP_ID: &str = "dev.ymapp.yandex-music";

/// How long to suppress pipeline-reported positions after a seek while the
/// GStreamer pipeline settles on the new position.
const SEEK_COOLDOWN_MS: u128 = 1500;

fn now_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

const CSS: &str = r#"
.sidebar {
    background-color: @window_bg_color;
    border-right: 1px solid @borders;
}
.sidebar button.nav {
    border-radius: 0;
    padding: 8px 16px;
}
.user-code {
    font-size: 34px;
    font-weight: bold;
    letter-spacing: 6px;
    font-family: monospace;
}
.login {
    margin: 24px;
}
"#;

fn load_css() {
    let Some(display) = gtk4::gdk::Display::default() else {
        return;
    };
    let provider = gtk4::CssProvider::new();
    provider.load_from_string(CSS);
    gtk4::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

#[cfg(target_os = "macos")]
fn setup_macos_bundle_env() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let Some(resources) = exe
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("Resources"))
    else {
        return;
    };
    if !resources.join("lib").is_dir() {
        return;
    }
    let plugin_dir = resources.join("lib/gstreamer-1.0");
    std::env::set_var("GST_PLUGIN_PATH", &plugin_dir);
    std::env::set_var("GST_PLUGIN_SYSTEM_PATH", &plugin_dir);
    if let Some(dir) = exe.parent() {
        std::env::set_var("GST_PLUGIN_SCANNER", dir.join("gst-plugin-scanner"));
    }
    if let Ok(home) = std::env::var("HOME") {
        std::env::set_var(
            "GST_REGISTRY",
            format!("{home}/Library/Caches/dev.ymapp.yandex-music/gstreamer-registry.bin"),
        );
    }
    std::env::set_var(
        "GDK_PIXBUF_MODULE_FILE",
        resources.join("lib/gdk-pixbuf-2.0/2.10.0/loaders.cache"),
    );
    std::env::set_var(
        "GSETTINGS_SCHEMA_DIR",
        resources.join("share/glib-2.0/schemas"),
    );
    std::env::set_var("XDG_DATA_DIRS", resources.join("share"));
    std::env::set_var("FONTCONFIG_FILE", resources.join("etc/fonts.conf"));
}

fn main() {
    #[cfg(target_os = "macos")]
    setup_macos_bundle_env();

    // Render with cairo instead of GL: the GL renderer pulls the GPU driver's
    // shader compiler and device buffers into the process (hundreds of MB of
    // RSS on some stacks), while this UI is simple enough to paint in software.
    // Forced here because a global GSK_RENDERER=gl in the session would
    // otherwise override it.
    std::env::set_var("GSK_RENDERER", "cairo");
    load_css();
    let app = adw::Application::new(Some(APP_ID), gio::ApplicationFlags::empty());
    app.connect_activate(build_ui);
    app.run();
}

fn build_ui(app: &adw::Application) {
    let (ev_tx, ev_rx) = std::sync::mpsc::channel::<AppEvent>();

    // Desktop integrations: MPRIS (media keys / playerctl) and the system
    // tray. Both fail softly when the session doesn't provide the service.
    let mpris = Mpris::spawn(ev_tx.clone());
    let tray_handle = Tray::new(ev_tx.clone()).spawn().ok();

    let ev_tx_for_playback = ev_tx.clone();
    let sink: EventSink = Arc::new(move |event| {
        let _ = ev_tx.send(event);
    });
    let worker = Worker::spawn(sink);
    let playback = Playback::new(worker.clone(), ev_tx_for_playback);

    // Window titlebar: an `AdwHeaderBar` gives the window client-side window
    // controls (minimize/maximize/close) and a draggable region, regardless of
    // the compositor. The account label and sign-out live here too.
    let account_label = gtk4::Label::new(Some(""));
    account_label.add_css_class("dim-label");
    let sign_out = gtk4::Button::with_label("Sign out");
    sign_out.add_css_class("flat");
    sign_out.set_visible(false);
    sign_out.connect_clicked({
        let worker = worker.clone();
        move |_| worker.send(WorkerCommand::Logout)
    });
    let headerbar = adw::HeaderBar::new();
    let header_title = gtk4::Label::new(Some("Yandex Music"));
    header_title.add_css_class("title");
    headerbar.set_title_widget(Some(&header_title));
    headerbar.pack_end(&sign_out);
    headerbar.pack_end(&account_label);

    let login = LoginView::new(worker.clone());
    let main = MainView::new(worker.clone(), playback.clone(), account_label.clone());

    let stack = gtk4::Stack::new();
    stack.add_named(&login.root, Some("login"));
    stack.add_named(&main.root, Some("main"));
    stack.set_visible_child_name("login");

    let toast_overlay = adw::ToastOverlay::new();
    toast_overlay.set_child(Some(&stack));

    // AdwApplicationWindow does not allow gtk_window_set_titlebar(). Window
    // controls and drag-to-move are provided by the headerbar living in the
    // top bar of an AdwToolbarView, which libadwaita treats as the titlebar.
    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&headerbar);
    toolbar_view.set_content(Some(&toast_overlay));

    let window = adw::ApplicationWindow::new(app);
    window.set_title(Some("Yandex Music"));
    window.set_default_size(1000, 700);
    window.set_content(Some(&toolbar_view));
    window.present();

    let worker_for_events = worker.clone();
    let toast_for_events = toast_overlay.clone();
    let stack_for_events = stack.clone();
    let login_for_events = login.clone();
    let main_for_events = main.clone();
    let playback_for_events = playback.clone();
    let sign_out_for_events = sign_out.clone();
    let app_for_events = app.clone();
    let window_for_events = window.clone();
    let mpris_for_events = mpris.clone();
    // My Wave is an infinite queue: whenever the queue runs out (end of stream
    // or the user skipping past the last track), fetch the next rotor batch
    // seeded with the last played track id and keep playing.
    let wave_active: Rc<Cell<bool>> = Rc::new(Cell::new(false));
    let wave_started: Rc<Cell<bool>> = Rc::new(Cell::new(false));
    let wave_continue: Rc<Cell<bool>> = Rc::new(Cell::new(false));
    // Set when a vibe change is requested: the next wave batch replaces the
    // queue (and restarts playback) instead of being appended.
    let wave_restart: Rc<Cell<bool>> = Rc::new(Cell::new(false));

    {
        let wave_active = wave_active.clone();
        let wave_continue = wave_continue.clone();
        let worker = worker.clone();
        let playback_inner = playback.clone();
        playback.set_on_exhausted(move || {
            if wave_active.get() {
                wave_continue.set(true);
                let last_id = playback_inner
                    .current_track()
                    .and_then(|t| t.id)
                    .map(|id| id.0);
                worker.send(WorkerCommand::FetchWave { queue: last_id });
            } else {
                playback_inner.stop();
            }
        });
    }
    // Keep MPRIS position in sync when the user seeks (from the playback bar or
    // from an external client): push the exact target immediately and announce
    // it with the `Seeked` signal, since the pipeline only reports the new
    // position asynchronously.
    let last_seek_ms: Rc<Cell<u128>> = Rc::new(Cell::new(0));
    {
        let mpris = mpris.clone();
        let last_seek_ms = last_seek_ms.clone();
        playback.set_on_seek(move |target_secs| {
            mpris.set_position_secs(target_secs);
            mpris.seeked(target_secs);
            last_seek_ms.set(now_millis());
        });
    }
    let handle_event = move |event: AppEvent| match event {
        AppEvent::NeedsLogin => {
            stack_for_events.set_visible_child_name("login");
        }
        AppEvent::LoginCode {
            user_code,
            verification_url,
        } => {
            login_for_events.show_code(&user_code, &verification_url);
            stack_for_events.set_visible_child_name("login");
        }
        AppEvent::LoginFailed { message } => {
            login_for_events.set_error(&message);
            stack_for_events.set_visible_child_name("login");
        }
        AppEvent::AccountReady { uid, display_name } => {
            login_for_events.reset();
            main_for_events.set_account(uid, &display_name);
            sign_out_for_events.set_visible(true);
            stack_for_events.set_visible_child_name("main");
            worker_for_events.send(WorkerCommand::FetchLiked);
            if std::env::var("YM_AUTOWAVE").is_ok() {
                worker_for_events.send(WorkerCommand::FetchWave { queue: None });
            }
        }
        AppEvent::LikedTracks { tracks } => main_for_events.set_liked(&tracks),
        AppEvent::Playlists { playlists } => main_for_events.set_playlists(&playlists),
        AppEvent::SearchResults {
            query,
            tracks,
            albums,
            artists,
            playlists,
        } => main_for_events.set_search_results(&query, &tracks, &albums, &artists, &playlists),
        AppEvent::NowPlaying { track } => {
            main_for_events.set_now_playing(&track);
            main_for_events.set_playing(true);
            mpris_for_events.set_track(&track);
            mpris_for_events.set_playing(true);
            if let Some(handle) = tray_handle.as_ref() {
                handle.update(|tray| tray.set_track(&track));
                handle.update(|tray| tray.set_playing(true));
            }
        }
        AppEvent::PlayStateChanged { playing } => {
            main_for_events.set_playing(playing);
            mpris_for_events.set_playing(playing);
            if let Some(handle) = tray_handle.as_ref() {
                handle.update(|tray| tray.set_playing(playing));
            }
        }
        AppEvent::RemoteCommand(command) => match command {
            RemoteCommand::Play => playback_for_events.play(),
            RemoteCommand::Pause => playback_for_events.pause(),
            RemoteCommand::Toggle => playback_for_events.toggle(),
            RemoteCommand::Next => {
                playback_for_events.next();
            }
            RemoteCommand::Previous => playback_for_events.prev(),
            RemoteCommand::SeekRelative { offset_seconds } => {
                playback_for_events.seek_relative(offset_seconds);
            }
            RemoteCommand::SetPosition { seconds } => {
                playback_for_events.seek_seconds(seconds);
            }
            RemoteCommand::SetVolume(volume) => playback_for_events.set_volume(volume),
            RemoteCommand::Quit => app_for_events.quit(),
            RemoteCommand::Raise => window_for_events.present(),
        },
        AppEvent::TrackStreamReady { track, url } => {
            playback_for_events.on_stream_ready(track, url);
        }
        AppEvent::PlaybackEnded => {
            if std::env::var("YM_DEBUG").is_ok() {
                eprintln!("[dbg] EOS -> next()");
            }
            playback_for_events.next();
        }
        AppEvent::WaveBatch {
            batch_id: _,
            tracks,
        } => {
            wave_active.set(true);
            if wave_restart.replace(false) || !wave_started.replace(true) {
                main_for_events.set_wave(&tracks);
                playback_for_events.play_queue(tracks, 0);
            } else {
                main_for_events.append_wave(&tracks);
                playback_for_events.append_tracks(&tracks);
                if wave_continue.replace(false) {
                    playback_for_events.continue_after_batch();
                }
            }
        }
        AppEvent::WaveMood { mood } => {
            main_for_events.set_wave_mood(&mood);
        }
        AppEvent::WaveMoodApplied { mood } => {
            main_for_events.set_wave_mood(&mood);
            wave_restart.set(true);
            worker_for_events.send(WorkerCommand::FetchWave { queue: None });
        }
        AppEvent::TrackLikeChanged {
            id,
            liked,
            disliked,
        } => {
            main_for_events.set_track_like(&id, liked, disliked);
        }
        AppEvent::PlaybackError { message } => {
            let toast = adw::Toast::new(&message);
            toast.set_title("Playback error");
            toast.set_timeout(8);
            toast_for_events.add_toast(toast);
        }
        AppEvent::CoverReady { url, bytes } => {
            main_for_events.on_cover_ready(&url, bytes);
        }
        AppEvent::OperationFailed { message } => {
            let toast = adw::Toast::new(&message);
            toast.set_timeout(5);
            toast_for_events.add_toast(toast);
        }
    };

    glib::source::timeout_add_local(Duration::from_millis(30), move || {
        while let Ok(event) = ev_rx.try_recv() {
            handle_event(event);
        }
        glib::ControlFlow::Continue
    });

    // Refresh the playback bar from the pipeline twice a second, and mirror
    // position/volume to MPRIS so media-key clients stay in sync. Position
    // mirrors are suppressed briefly after a seek: the pipeline can report a
    // stale/transient position until the seek settles, which would otherwise
    // overwrite the exact position we just announced via `Seeked`.
    let mpris_for_poll = mpris.clone();
    let last_seek_ms_for_poll = last_seek_ms.clone();
    let last_volume: Rc<Cell<f64>> = Rc::new(Cell::new(-1.0));
    let main_for_poll = main.clone();
    let main_for_covers = main.clone();
    glib::source::timeout_add_local(Duration::from_millis(500), move || {
        let position = playback.position();
        main_for_poll.update_progress(position, playback.duration());
        if let Some(position) = position {
            let settled =
                now_millis().saturating_sub(last_seek_ms_for_poll.get()) > SEEK_COOLDOWN_MS;
            if settled {
                mpris_for_poll.set_position_secs(position.seconds());
            }
        }
        let volume = playback.volume();
        if (volume - last_volume.get()).abs() > 0.001 {
            last_volume.set(volume);
            mpris_for_poll.set_volume(volume);
        }
        glib::ControlFlow::Continue
    });

    // Request covers for list rows as they scroll into view.
    glib::source::timeout_add_local(Duration::from_millis(150), move || {
        main_for_covers.poll_covers();
        glib::ControlFlow::Continue
    });

    // Show the login page unless we already have saved credentials.
    worker.send(WorkerCommand::Initialize);
}
