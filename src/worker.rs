//! The background worker: owns the `ApiClient` on a tokio runtime thread and
//! marshals results back to the GTK main loop. Events are delivered through a
//! thread-safe sink (`EventSink`); posting re-schedules the callback onto the
//! default main context so the GUI is only ever touched from the main thread.
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use gtk4::glib;
use tokio::sync::mpsc;

use crate::api::models::Track;
use crate::api::radio::MY_WAVE;
use crate::api::{ApiClient, Id};
use crate::config;
use crate::state::{AppEvent, WorkerCommand};

/// A thread-safe sink for worker events. The callback is invoked on the GTK
/// main thread (via `glib::source::idle_add`), never on the worker thread.
pub type EventSink = Arc<dyn Fn(AppEvent) + Send + Sync + 'static>;

/// Schedule `event` for delivery on the default main context.
fn post(ev: &EventSink, event: AppEvent) {
    let ev = ev.clone();
    let _ = glib::source::idle_add_once(move || {
        ev(event);
    });
}

/// Handle to the background worker.
#[derive(Clone)]
pub struct Worker {
    cmd: mpsc::Sender<WorkerCommand>,
}

impl Worker {
    /// Spawn the worker thread and return a handle to send it commands.
    pub fn spawn(ev: EventSink) -> Worker {
        let (cmd, rx) = mpsc::channel(64);
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("failed to build tokio runtime");
            rt.block_on(worker_loop(rx, ev));
        });
        Worker { cmd }
    }

    /// Queue a command for the worker.
    pub fn send(&self, cmd: WorkerCommand) {
        let _ = self.cmd.blocking_send(cmd);
    }
}

fn persist_tokens(tokens: Option<crate::api::AuthTokens>) {
    let mut config = config::load_config();
    config.tokens = tokens;
    let _ = config::save_config(&config);
}

async fn worker_loop(mut rx: mpsc::Receiver<WorkerCommand>, ev: EventSink) {
    let client = Arc::new(ApiClient::new("ru"));
    let saved = config::load_config();
    client.set_tokens(saved.tokens);
    // Restore the last "My Wave" vibe so the UI highlights the active preset.
    // A fresh install defaults to "Any" (`all`); the server-side setting is
    // applied lazily in `FetchWave` so no vibe-specific batch is ever returned.
    let vibe_applied = saved.wave_mood.is_some();
    post(
        &ev,
        AppEvent::WaveMood {
            mood: saved.wave_mood.clone().unwrap_or_else(|| "all".into()),
        },
    );
    let mut login_task: Option<tokio::task::AbortHandle> = None;
    let mut vibe_applied = vibe_applied;
    // Cache of the user's liked track ids so search results can show the like
    // state (the search API does not report it). Refreshed by `FetchLiked`.
    let liked_ids: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
    let mut likes_loaded = false;
    // Cap concurrent cover downloads so a large list doesn't flood the network.
    let cover_slots = Arc::new(tokio::sync::Semaphore::new(8));

    while let Some(cmd) = rx.recv().await {
        match cmd {
            WorkerCommand::Initialize => {
                let Some(tokens) = client.tokens() else {
                    post(&ev, AppEvent::NeedsLogin);
                    continue;
                };
                match client.account_status().await {
                    Ok(status) => {
                        post(
                            &ev,
                            AppEvent::AccountReady {
                                uid: status.account.uid.unwrap_or_default(),
                                display_name: status.account.display_name,
                            },
                        );
                    }
                    Err(err) if err.is_auth() => {
                        let refreshed = match tokens.refresh_token.as_deref() {
                            Some(refresh) => client.refresh_access_token(refresh).await,
                            None => Err(crate::api::ApiError::Auth(
                                "no refresh token; re-login required".into(),
                            )),
                        };
                        match refreshed {
                            Ok(new_tokens) => {
                                client.set_tokens(Some(new_tokens));
                                persist_tokens(client.tokens());
                                match client.account_status().await {
                                    Ok(status) => {
                                        post(
                                            &ev,
                                            AppEvent::AccountReady {
                                                uid: status.account.uid.unwrap_or_default(),
                                                display_name: status.account.display_name,
                                            },
                                        );
                                    }
                                    Err(e) => {
                                        client.set_tokens(None);
                                        persist_tokens(None);
                                        post(
                                            &ev,
                                            AppEvent::LoginFailed {
                                                message: e.to_string(),
                                            },
                                        );
                                        post(&ev, AppEvent::NeedsLogin);
                                    }
                                }
                            }
                            Err(e) => {
                                client.set_tokens(None);
                                persist_tokens(None);
                                post(
                                    &ev,
                                    AppEvent::LoginFailed {
                                        message: e.to_string(),
                                    },
                                );
                                post(&ev, AppEvent::NeedsLogin);
                            }
                        }
                    }
                    Err(e) => {
                        post(
                            &ev,
                            AppEvent::OperationFailed {
                                message: e.to_string(),
                            },
                        );
                    }
                }
            }
            WorkerCommand::BeginLogin => {
                if login_task.is_some() {
                    continue;
                }
                let client = Arc::clone(&client);
                let ev = ev.clone();
                let handle = tokio::spawn(async move {
                    let result = client
                        .device_auth(|code| {
                            post(
                                &ev,
                                AppEvent::LoginCode {
                                    user_code: code.user_code.clone(),
                                    verification_url: code.verification_url.clone(),
                                },
                            );
                        })
                        .await;
                    match result {
                        Ok(tokens) => {
                            client.set_tokens(Some(tokens));
                            persist_tokens(client.tokens());
                            match client.account_status().await {
                                Ok(status) => {
                                    post(
                                        &ev,
                                        AppEvent::AccountReady {
                                            uid: status.account.uid.unwrap_or_default(),
                                            display_name: status.account.display_name,
                                        },
                                    );
                                }
                                Err(e) => {
                                    post(
                                        &ev,
                                        AppEvent::LoginFailed {
                                            message: e.to_string(),
                                        },
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            post(
                                &ev,
                                AppEvent::LoginFailed {
                                    message: e.to_string(),
                                },
                            );
                        }
                    }
                });
                login_task = Some(handle.abort_handle());
            }
            WorkerCommand::CancelLogin => {
                if let Some(handle) = login_task.take() {
                    handle.abort();
                }
            }
            WorkerCommand::Logout => {
                client.set_tokens(None);
                client.set_uid(None);
                persist_tokens(None);
                post(&ev, AppEvent::NeedsLogin);
            }
            WorkerCommand::Search(text) => {
                if !likes_loaded {
                    if let Ok(refs) = client.users_likes_tracks().await {
                        *liked_ids.lock().unwrap() = refs
                            .iter()
                            .filter_map(|t| t.id.as_ref().map(|id| id.0.clone()))
                            .collect();
                        likes_loaded = true;
                    }
                }
                match client.search(&text).await {
                    Ok(search) => {
                        let mut tracks = search.tracks.map(|b| b.results).unwrap_or_default();
                        {
                            let liked = liked_ids.lock().unwrap();
                            for track in &mut tracks {
                                if let Some(id) = track.id.as_ref() {
                                    if liked.contains(&id.0) {
                                        track.liked = Some(true);
                                    }
                                }
                            }
                        }
                        post(
                            &ev,
                            AppEvent::SearchResults {
                                query: text,
                                tracks,
                                albums: search.albums.map(|b| b.results).unwrap_or_default(),
                                artists: search.artists.map(|b| b.results).unwrap_or_default(),
                                playlists: search.playlists.map(|b| b.results).unwrap_or_default(),
                            },
                        );
                    }
                    Err(e) => {
                        post(
                            &ev,
                            AppEvent::OperationFailed {
                                message: e.to_string(),
                            },
                        );
                    }
                }
            }
            WorkerCommand::FetchLiked => match client.users_likes_tracks().await {
                Ok(refs) => {
                    *liked_ids.lock().unwrap() = refs
                        .iter()
                        .filter_map(|t| t.id.as_ref().map(|id| id.0.clone()))
                        .collect();
                    likes_loaded = true;
                    match client.hydrate_tracks(&refs).await {
                        Ok(tracks) => {
                            let tracks = tracks
                                .into_iter()
                                .map(|mut t| {
                                    t.liked = Some(true);
                                    t
                                })
                                .collect();
                            post(&ev, AppEvent::LikedTracks { tracks });
                        }
                        Err(e) => {
                            post(
                                &ev,
                                AppEvent::OperationFailed {
                                    message: e.to_string(),
                                },
                            );
                        }
                    }
                }
                Err(e) => {
                    post(
                        &ev,
                        AppEvent::OperationFailed {
                            message: e.to_string(),
                        },
                    );
                }
            },
            WorkerCommand::FetchPlaylists => match client.users_playlists_list().await {
                Ok(playlists) => {
                    post(&ev, AppEvent::Playlists { playlists });
                }
                Err(e) => {
                    post(
                        &ev,
                        AppEvent::OperationFailed {
                            message: e.to_string(),
                        },
                    );
                }
            },
            WorkerCommand::PlayTrack { track } => match client.resolve_track_stream(&track).await {
                Ok(url) => post(&ev, AppEvent::TrackStreamReady { track, url }),
                Err(e) => post(
                    &ev,
                    AppEvent::PlaybackError {
                        message: e.to_string(),
                    },
                ),
            },
            WorkerCommand::SetLike { id, liked } => {
                let id_ref = Id(id.clone());
                let result = if liked {
                    // The API clears any existing dislike automatically.
                    client.likes_tracks_add(std::slice::from_ref(&id_ref)).await
                } else {
                    client
                        .likes_tracks_remove(std::slice::from_ref(&id_ref))
                        .await
                };
                match result {
                    Ok(()) => {
                        let mut ids = liked_ids.lock().unwrap();
                        if liked {
                            ids.insert(id.clone());
                        } else {
                            ids.remove(&id);
                        }
                        post(
                            &ev,
                            AppEvent::TrackLikeChanged {
                                id,
                                liked,
                                disliked: false,
                            },
                        );
                    }
                    Err(e) => post(
                        &ev,
                        AppEvent::OperationFailed {
                            message: e.to_string(),
                        },
                    ),
                }
            }
            WorkerCommand::SetDislike { id, disliked } => {
                let id_ref = Id(id.clone());
                let result = if disliked {
                    // The API clears any existing like automatically.
                    client
                        .dislikes_tracks_add(std::slice::from_ref(&id_ref))
                        .await
                } else {
                    client
                        .dislikes_tracks_remove(std::slice::from_ref(&id_ref))
                        .await
                };
                match result {
                    Ok(()) => {
                        liked_ids.lock().unwrap().remove(&id);
                        post(
                            &ev,
                            AppEvent::TrackLikeChanged {
                                id,
                                liked: false,
                                disliked,
                            },
                        );
                    }
                    Err(e) => post(
                        &ev,
                        AppEvent::OperationFailed {
                            message: e.to_string(),
                        },
                    ),
                }
            }
            WorkerCommand::SetVibe { mood } => {
                let current = config::load_config();
                let diversity = current.wave_diversity.unwrap_or_else(|| "default".into());
                let language = current.wave_language.unwrap_or_else(|| "any".into());
                match client
                    .rotor_station_settings(MY_WAVE, &mood, &diversity, &language)
                    .await
                {
                    Ok(()) => {
                        let mut cfg = config::load_config();
                        cfg.wave_mood = Some(mood.clone());
                        cfg.wave_diversity = Some(diversity);
                        cfg.wave_language = Some(language);
                        let _ = config::save_config(&cfg);
                        vibe_applied = true;
                        post(&ev, AppEvent::WaveMoodApplied { mood });
                    }
                    Err(e) => {
                        post(
                            &ev,
                            AppEvent::OperationFailed {
                                message: e.to_string(),
                            },
                        );
                    }
                }
            }
            WorkerCommand::FetchWave { queue } => {
                if !vibe_applied {
                    let current = config::load_config();
                    let diversity = current.wave_diversity.unwrap_or_else(|| "default".into());
                    let language = current.wave_language.unwrap_or_else(|| "any".into());
                    if client
                        .rotor_station_settings(MY_WAVE, "all", &diversity, &language)
                        .await
                        .is_ok()
                    {
                        vibe_applied = true;
                    }
                }
                match client.rotor_station_tracks(MY_WAVE, queue).await {
                    Ok(result) => {
                        if !likes_loaded {
                            if let Ok(refs) = client.users_likes_tracks().await {
                                *liked_ids.lock().unwrap() = refs
                                    .iter()
                                    .filter_map(|t| t.id.as_ref().map(|id| id.0.clone()))
                                    .collect();
                                likes_loaded = true;
                            }
                        }
                        let mut tracks: Vec<Track> = result
                            .sequence
                            .iter()
                            .filter_map(|item| {
                                let mut track = item.track.clone()?;
                                track.liked = item.liked.or(track.liked);
                                Some(track)
                            })
                            .collect();
                        // The wave API does not reliably report likes; the liked
                        // library cache is the source of truth.
                        let liked = liked_ids.lock().unwrap();
                        for track in &mut tracks {
                            if let Some(id) = track.id.as_ref() {
                                if liked.contains(&id.0) {
                                    track.liked = Some(true);
                                }
                            }
                        }
                        post(
                            &ev,
                            AppEvent::WaveBatch {
                                batch_id: result.batch_id,
                                tracks,
                            },
                        );
                    }
                    Err(e) => {
                        post(
                            &ev,
                            AppEvent::OperationFailed {
                                message: e.to_string(),
                            },
                        );
                    }
                }
            }
            WorkerCommand::FetchCover { url } => {
                // Spawn so the command loop keeps draining the channel (the UI
                // sends one request per row) while the semaphore caps concurrency.
                let client = Arc::clone(&client);
                let ev = ev.clone();
                let slots = Arc::clone(&cover_slots);
                tokio::spawn(async move {
                    let Ok(_permit) = slots.acquire().await else {
                        return;
                    };
                    if let Ok(bytes) = client.raw_get(&url).await {
                        post(&ev, AppEvent::CoverReady { url, bytes });
                    }
                });
            }
        }
    }
}
