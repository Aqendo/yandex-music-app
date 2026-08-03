//! `ym-cli` — headless smoke-tester for the API layer.
//!
//! Exercises the same code paths the GUI uses, without the GTK runtime. Useful
//! for verifying the API integration (and the saved token) quickly.
//!
//! Usage:
//!   ym-cli login                start the OAuth device flow
//!   ym-cli search <query>       search the catalogue
//!   ym-cli liked                list your liked tracks
//!   ym-cli playlists            list your playlists
//!   ym-cli wave                 pull one "My Wave" batch
use std::error::Error;

use ymapp::api::radio::MY_WAVE;
use ymapp::api::ApiClient;
use ymapp::config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    let sub = args.get(1).map(|s| s.as_str()).unwrap_or("help");
    match sub {
        "login" => cmd_login().await?,
        "search" => {
            let query = args.get(2).map(|s| s.as_str()).unwrap_or("");
            cmd_search(query).await?
        }
        "liked" => cmd_liked().await?,
        "playlists" => cmd_playlists().await?,
        "wave" => cmd_wave().await?,
        "waveurls" => cmd_waveurls().await?,
        "settings" => cmd_settings().await?,
        "setmood" => {
            let mood = args.get(2).map(|s| s.as_str()).unwrap_or("all");
            cmd_setmood(mood).await?
        }
        "like" => {
            let id = args.get(2).map(|s| s.as_str()).unwrap_or("");
            cmd_like(id).await?
        }
        "unlike" => {
            let id = args.get(2).map(|s| s.as_str()).unwrap_or("");
            cmd_unlike(id).await?
        }
        "dislike" => {
            let id = args.get(2).map(|s| s.as_str()).unwrap_or("");
            cmd_dislike(id).await?
        }
        "undislike" => {
            let id = args.get(2).map(|s| s.as_str()).unwrap_or("");
            cmd_undislike(id).await?
        }
        "help" | "--help" | "-h" => usage(),
        _ => {
            eprintln!("unknown command: {sub}");
            usage();
        }
    }
    Ok(())
}

fn authenticated() -> ApiClient {
    let client = ApiClient::new("ru");
    client.set_tokens(config::load_config().tokens);
    client
}

fn artist_name(track: &ymapp::api::Track) -> &str {
    track.artists.first().map(|a| a.name.as_str()).unwrap_or("Unknown")
}

async fn cmd_login() -> Result<(), Box<dyn Error>> {
    let client = ApiClient::new("ru");
    let tokens = client
        .device_auth(|code| {
            println!();
            println!("1) Open {} in your browser", code.verification_url);
            println!("2) Enter the code:");
            println!();
            println!("   CODE: {}", code.user_code);
            println!();
        })
        .await?;
    let mut cfg = config::load_config();
    cfg.tokens = Some(tokens);
    config::save_config(&cfg)?;
    let status = client.account_status().await?;
    println!(
        "Logged in as {} (uid {})",
        status.account.display_name, status.account.uid.unwrap_or_default()
    );
    Ok(())
}

async fn cmd_search(query: &str) -> Result<(), Box<dyn Error>> {
    let client = authenticated();
    let search = client.search(query).await?;
    if let Some(tracks) = &search.tracks {
        println!("Tracks ({} total):", tracks.total.unwrap_or(0));
        for track in tracks.results.iter().take(10) {
            println!("  {} — {}", track.title, artist_name(track));
        }
    }
    if let Some(artists) = &search.artists {
        println!("Artists ({} total):", artists.total.unwrap_or(0));
        for artist in artists.results.iter().take(5) {
            println!("  {}", artist.name);
        }
    }
    if let Some(albums) = &search.albums {
        println!("Albums ({} total):", albums.total.unwrap_or(0));
        for album in albums.results.iter().take(5) {
            println!("  {} — {}", album.title, album.artists.first().map(|a| a.name.as_str()).unwrap_or("?"));
        }
    }
    Ok(())
}

async fn cmd_liked() -> Result<(), Box<dyn Error>> {
    let client = authenticated();
    let status = client.account_status().await?;
    println!(
        "Account: {} (uid {})",
        status.account.display_name,
        status.account.uid.unwrap_or_default()
    );
    let refs = client.users_likes_tracks().await?;
    println!("Liked tracks: {}", refs.len());
    for track in client.hydrate_tracks(&refs).await? {
        let id = track.id.as_ref().map(|i| i.0.clone()).unwrap_or_default();
        println!("  {id} | {} — {}", track.title, artist_name(&track));
    }
    Ok(())
}

async fn cmd_playlists() -> Result<(), Box<dyn Error>> {
    let client = authenticated();
    let _ = client.account_status().await?;
    for playlist in client.users_playlists_list().await? {
        println!(
            "[{}] {} — {} tracks",
            playlist.kind.unwrap_or(0),
            playlist.title,
            playlist.track_count.unwrap_or(0)
        );
    }
    Ok(())
}

async fn cmd_wave() -> Result<(), Box<dyn Error>> {
    let client = authenticated();
    let _ = client.account_status().await?;
    let info = client.rotor_station_info(MY_WAVE).await?;
    for result in info {
        if let Some(station) = &result.station {
            println!("Station: {}", station.name.as_deref().unwrap_or(MY_WAVE));
        }
        if let Some(settings) = &result.settings2 {
            println!(
                "Settings: mood={} diversity={} language={}",
                settings.mood_energy.as_deref().unwrap_or("?"),
                settings.diversity.as_deref().unwrap_or("?"),
                settings.language.as_deref().unwrap_or("?")
            );
        }
    }
    let batch = client.rotor_station_tracks(MY_WAVE, None).await?;
    println!("Batch: {}", batch.batch_id.as_deref().unwrap_or("?"));
    let last_id = batch
        .sequence
        .iter()
        .filter_map(|item| item.track.as_ref())
        .next_back()
        .and_then(|t| t.id.as_ref())
        .map(|id| id.0.clone());
    for item in batch.sequence.iter().take(10) {
        if let Some(track) = &item.track {
            println!("  {} — {}", track.title, artist_name(track));
        }
    }
    let next = client.rotor_station_tracks(MY_WAVE, last_id).await?;
    println!("Next batch: {}", next.batch_id.as_deref().unwrap_or("?"));
    for item in next.sequence.iter().take(5) {
        if let Some(track) = &item.track {
            println!("  {} — {}", track.title, artist_name(track));
        }
    }
    Ok(())
}

async fn cmd_waveurls() -> Result<(), Box<dyn Error>> {
    let client = authenticated();
    let _ = client.account_status().await?;
    let batch = client.rotor_station_tracks(MY_WAVE, None).await?;
    let tracks: Vec<_> = batch.sequence.iter().filter_map(|i| i.track.clone()).collect();
    println!("batch {} | tracks {}", batch.batch_id.unwrap_or_default(), tracks.len());
    for (n, t) in tracks.iter().enumerate() {
        let id = t.id.as_ref().map(|i| i.0.clone()).unwrap_or_default();
        match client.resolve_track_stream(t).await {
            Ok(url) => println!("{n}: {id} | {} — {} | available={} | {url}",
                t.title, artist_name(t), t.available.map(|a| a.to_string()).unwrap_or_default()),
            Err(e) => println!("{n}: {id} | {} — {} | RESOLVE ERROR: {e}",
                t.title, artist_name(t)),
        }
    }
    Ok(())
}

async fn cmd_settings() -> Result<(), Box<dyn Error>> {
    let client = authenticated();
    let _ = client.account_status().await?;
    let info = client.rotor_station_info(MY_WAVE).await?;
    for result in info {
        if let Some(station) = &result.station {
            println!("Station: {}", station.name.as_deref().unwrap_or(MY_WAVE));
            if let Some(restrictions) = &station.restrictions2 {
                if let Some(mood) = &restrictions.mood_energy {
                    println!("Mood available:");
                    for v in &mood.possible_values {
                        println!("  {} ({})", v.name.as_deref().unwrap_or("?"), v.value.as_deref().unwrap_or("?"));
                    }
                }
                if let Some(div) = &restrictions.diversity {
                    println!("Diversity available:");
                    for v in &div.possible_values {
                        println!("  {} ({})", v.name.as_deref().unwrap_or("?"), v.value.as_deref().unwrap_or("?"));
                    }
                }
            }
        }
        if let Some(settings) = &result.settings2 {
            println!(
                "Current: mood={} diversity={} language={}",
                settings.mood_energy.as_deref().unwrap_or("?"),
                settings.diversity.as_deref().unwrap_or("?"),
                settings.language.as_deref().unwrap_or("?")
            );
        }
    }
    Ok(())
}

async fn cmd_setmood(mood: &str) -> Result<(), Box<dyn Error>> {
    let client = authenticated();
    let _ = client.account_status().await?;
    client.rotor_station_settings(MY_WAVE, mood, "default", "any").await?;
    println!("mood set to {mood}");
    let batch = client.rotor_station_tracks(MY_WAVE, None).await?;
    for item in batch.sequence.iter().take(6) {
        if let Some(track) = &item.track {
            println!("  {} — {}", track.title, artist_name(track));
        }
    }
    Ok(())
}

async fn cmd_like(id: &str) -> Result<(), Box<dyn Error>> {
    let client = authenticated();
    let _ = client.account_status().await?;
    let track_id = ymapp::api::Id(id.to_string());
    // The API clears any existing dislike automatically.
    client
        .likes_tracks_add(std::slice::from_ref(&track_id))
        .await?;
    println!("liked {id}");
    Ok(())
}

async fn cmd_unlike(id: &str) -> Result<(), Box<dyn Error>> {
    let client = authenticated();
    let _ = client.account_status().await?;
    let track_id = ymapp::api::Id(id.to_string());
    client
        .likes_tracks_remove(std::slice::from_ref(&track_id))
        .await?;
    println!("unliked {id}");
    Ok(())
}

async fn cmd_dislike(id: &str) -> Result<(), Box<dyn Error>> {
    let client = authenticated();
    let _ = client.account_status().await?;
    let track_id = ymapp::api::Id(id.to_string());
    // The API clears any existing like automatically.
    client
        .dislikes_tracks_add(std::slice::from_ref(&track_id))
        .await?;
    println!("disliked {id}");
    Ok(())
}

async fn cmd_undislike(id: &str) -> Result<(), Box<dyn Error>> {
    let client = authenticated();
    let _ = client.account_status().await?;
    let track_id = ymapp::api::Id(id.to_string());
    client
        .dislikes_tracks_remove(std::slice::from_ref(&track_id))
        .await?;
    println!("undisliked {id}");
    Ok(())
}

fn usage() {
    println!(
        "ym-cli — Yandex Music API smoke tool\n\n\
         Usage:\n  \
         ym-cli login\n  \
         ym-cli search <query>\n  \
         ym-cli liked\n  \
         ym-cli playlists\n  \
         ym-cli wave\n  \
         ym-cli setmood <mood>\n  \
         ym-cli like <id>\n  \
         ym-cli unlike <id>\n  \
         ym-cli dislike <id>\n  \
         ym-cli undislike <id>\n"
    );
}
