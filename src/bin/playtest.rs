//! Temporary headless reproduction harness: drives the real `Playback` code
//! through a My Wave batch and reports any pipeline errors verbatim.
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use gtk4::glib;

use ymapp::api::radio::MY_WAVE;
use ymapp::api::ApiClient;
use ymapp::config;
use ymapp::playback::Playback;
use ymapp::state::{AppEvent, WorkerCommand};
use ymapp::worker::{EventSink, Worker};

fn artist(track: &ymapp::api::models::Track) -> String {
    track.artists.first().map(|a| a.name.clone()).unwrap_or_default()
}

fn main() {
    let (ev_tx, ev_rx) = mpsc::channel::<AppEvent>();
    let ev_tx_for_playback = ev_tx.clone();
    let sink: EventSink = Arc::new(move |event| {
        let _ = ev_tx.send(event);
    });
    let worker = Worker::spawn(sink);
    let playback = Playback::new(worker.clone(), ev_tx_for_playback);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let client = ApiClient::new("ru");
    let saved = config::load_config();
    client.set_tokens(saved.tokens);
    let batch = rt
        .block_on(client.rotor_station_tracks(MY_WAVE, None))
        .expect("fetch wave");
    let tracks: Vec<_> = batch
        .sequence
        .iter()
        .filter_map(|item| item.track.clone())
        .collect();
    println!("[playtest] batch {} tracks {}", batch.batch_id.unwrap_or_default(), tracks.len());

    let errors: std::rc::Rc<std::cell::RefCell<Vec<String>>> =
        std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let wave_continue = std::rc::Rc::new(std::cell::Cell::new(false));
    let errors_for_final = errors.clone();

    {
        let worker = worker.clone();
        let playback_inner = playback.clone();
        let wave_continue = wave_continue.clone();
        playback.set_on_exhausted(move || {
            println!("[playtest] EXHAUSTED -> fetch continuation");
            wave_continue.set(true);
            let last_id = playback_inner
                .current_track()
                .and_then(|t| t.id)
                .map(|id| id.0);
            worker.send(WorkerCommand::FetchWave { queue: last_id });
        });
    }

    let rate: f64 = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(1.0);
    let next_at: Option<f64> = std::env::args().nth(2).and_then(|a| a.parse().ok());
    if let Some(secs) = next_at {
        let playback_for_next = playback.clone();
        glib::timeout_add_local(Duration::from_secs_f64(secs), move || {
            println!("[playtest] MANUAL NEXT CLICK -> playback.next()");
            playback_for_next.next();
            glib::ControlFlow::Break
        });
    }
    fn reapply(playback: &Playback, rate: f64, attempt: u32) {
        if playback.set_rate(rate) {
            return;
        }
        if attempt > 60 {
            println!("[playtest] set_rate gave up");
            return;
        }
        let playback = playback.clone();
        glib::timeout_add_local(Duration::from_millis(1500), move || {
            reapply(&playback, rate, attempt + 1);
            glib::ControlFlow::Break
        });
    }
    reapply(&playback, rate, 0);
    playback.play_queue(tracks, 0);
    let main_loop = glib::MainLoop::new(None, false);
    let loop_for_timeout = main_loop.clone();
    let last_log: std::rc::Rc<std::cell::RefCell<std::time::Instant>> =
        std::rc::Rc::new(std::cell::RefCell::new(std::time::Instant::now()));
    glib::timeout_add_local(Duration::from_millis(20), move || {
        while let Ok(event) = ev_rx.try_recv() {
            match event {
                AppEvent::NowPlaying { track } => {
                    let id = track.id.as_ref().map(|i| i.0.clone()).unwrap_or_default();
                    println!("[playtest] NOW {id} | {} — {}", track.title, artist(&track));
                    reapply(&playback, rate, 0);
                }
                AppEvent::TrackStreamReady { track, url } => {
                    playback.on_stream_ready(track, url);
                }
                AppEvent::PlaybackEnded => {
                    println!("[playtest] EOS -> next()");
                    playback.next();
                }
                AppEvent::PlaybackError { message } => {
                    errors.borrow_mut().push(message.clone());
                    println!("[playtest] ERROR: {message}");
                }
                AppEvent::WaveBatch { tracks, .. } => {
                    println!("[playtest] WAVE BATCH {} tracks", tracks.len());
                    playback.append_tracks(&tracks);
                    if wave_continue.replace(false) {
                        playback.continue_after_batch();
                    }
                }
                _ => {}
            }
        }
        let last_log_inner = last_log.clone();
        let playback_for_log = playback.clone();
        let now = std::time::Instant::now();
        if now.duration_since(*last_log_inner.borrow()) > Duration::from_secs(3) {
            *last_log_inner.borrow_mut() = now;
            let id = playback_for_log
                .current_track()
                .and_then(|t| t.id)
                .map(|i| i.0.clone())
                .unwrap_or_default();
            let pos = playback_for_log.position().map(|p| p.seconds()).unwrap_or(0);
            let dur = playback_for_log.duration().map(|d| d.seconds()).unwrap_or(0);
            println!(
                "[playtest] pos {pos}s / {dur}s track {id} playing={}",
                playback_for_log.is_playing()
            );
        }
        if !errors.borrow().is_empty() {
            println!("[playtest] FAILED: {:#?}", errors.borrow());
            loop_for_timeout.quit();
        }
        glib::ControlFlow::Continue
    });

    // Watchdog: quit after 20 minutes regardless.
    let loop_for_watchdog = main_loop.clone();
    glib::timeout_add_local(Duration::from_secs(1200), move || {
        loop_for_watchdog.quit();
        glib::ControlFlow::Continue
    });

    main_loop.run();
    if errors_for_final.borrow().is_empty() {
        println!("[playtest] OK — completed without pipeline errors");
    } else {
        std::process::exit(1);
    }
}
