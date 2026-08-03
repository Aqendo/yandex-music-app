//! Shared command/event types passed between the UI thread and the worker thread.
use crate::api::models::{Album, Artist, Playlist, Track};

/// Commands sent from the UI (and CLI) to the background worker.
#[derive(Debug, Clone)]
pub enum WorkerCommand {
    /// Load saved tokens and try to authenticate (refreshing if needed).
    Initialize,
    /// Start the OAuth device flow.
    BeginLogin,
    /// Abort a pending device-flow login.
    CancelLogin,
    /// Clear saved credentials.
    Logout,
    /// Search the catalogue.
    Search(String),
    /// Fetch the liked-tracks library (hydrated).
    FetchLiked,
    /// Fetch the user's playlists.
    FetchPlaylists,
    /// Pull the next "My Wave" batch (`queue` = previous batch id).
    FetchWave { queue: Option<String> },
    /// Set the "My Wave" mood/energy preset (one of `radio::MOODS`).
    SetVibe { mood: String },
    /// Set a track's like state (`liked == true` also clears any dislike).
    SetLike { id: String, liked: bool },
    /// Set a track's dislike state (`disliked == true` also clears any like).
    SetDislike { id: String, disliked: bool },
    /// Resolve a track's direct stream URL and prepare it for playback.
    PlayTrack { track: Track },
    /// Download raw cover-art bytes for `url` (best-effort).
    FetchCover { url: String },
}

/// Commands sent by external integrations (MPRIS media keys, system tray)
/// from their own threads to the UI thread for playback control.
#[derive(Debug, Clone, Copy)]
pub enum RemoteCommand {
    Play,
    Pause,
    Toggle,
    Next,
    Previous,
    /// Relative seek in seconds (MPRIS `Seek`).
    SeekRelative {
        offset_seconds: i64,
    },
    /// Absolute seek in seconds (MPRIS `SetPosition`).
    SetPosition {
        seconds: u64,
    },
    SetVolume(f64),
    Quit,
    Raise,
}

/// Events sent from the worker to the UI thread.
#[derive(Debug, Clone)]
pub enum AppEvent {
    /// No saved credentials — show the login screen.
    NeedsLogin,
    /// A device code is ready to display.
    LoginCode {
        user_code: String,
        verification_url: String,
    },
    /// Login (or token refresh) failed.
    LoginFailed { message: String },
    /// The account is authenticated and ready.
    AccountReady { uid: i64, display_name: String },
    /// Search results for a query.
    SearchResults {
        query: String,
        tracks: Vec<Track>,
        albums: Vec<Album>,
        artists: Vec<Artist>,
        playlists: Vec<Playlist>,
    },
    /// The hydrated liked-tracks library.
    LikedTracks { tracks: Vec<Track> },
    /// The user's playlists.
    Playlists { playlists: Vec<Playlist> },
    /// A fresh "My Wave" batch.
    WaveBatch {
        batch_id: Option<String>,
        tracks: Vec<Track>,
    },
    /// The restored "My Wave" mood preset (from saved config), to highlight the
    /// active vibe in the UI.
    WaveMood { mood: String },
    /// A vibe change was applied server-side; restart the wave with it.
    WaveMoodApplied { mood: String },
    /// A track started playing (also updates the now-playing UI).
    NowPlaying { track: Track },
    /// The pipeline play/pause state changed.
    PlayStateChanged { playing: bool },
    /// A like/dislike change was applied server-side.
    TrackLikeChanged {
        id: String,
        liked: bool,
        disliked: bool,
    },
    /// The worker resolved a direct stream URL for a track.
    TrackStreamReady { track: Track, url: String },
    /// The current track reached the end of its stream.
    PlaybackEnded,
    /// Playback (resolution or pipeline) failed.
    PlaybackError { message: String },
    /// A media-key / tray command that must run on the UI thread.
    RemoteCommand(RemoteCommand),
    /// Raw cover-art bytes downloaded by the worker.
    CoverReady { url: String, bytes: Vec<u8> },
    /// Any non-fatal operation failure.
    OperationFailed { message: String },
}
