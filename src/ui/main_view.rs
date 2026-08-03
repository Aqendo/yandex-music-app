//! The main application shell: header row, sidebar navigation, page stack and
//! the playback bar at the bottom.
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{Align, Box as GtkBox, Button, Entry, Image, Label, ListBox, ListBoxRow, Orientation, PolicyType, Scale, ScrolledWindow, SelectionMode, Stack};
use gstreamer::ClockTime;

use crate::api::covers::{album_cover, artist_cover, playlist_cover, track_cover};
use crate::api::models::{Album, Artist, Playlist, Track};
use crate::playback::Playback;
use crate::player::format_time;
use crate::state::WorkerCommand;
use crate::ui::covers::Covers;
use crate::worker::Worker;

/// Milliseconds elapsed since the Unix epoch.
fn now_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// Widgets backing the search page.
#[derive(Clone)]
struct SearchPage {
    root: GtkBox,
    summary: Label,
    tracks_header: Label,
    albums_header: Label,
    artists_header: Label,
    playlists_header: Label,
    tracks_list: ListBox,
    albums_list: ListBox,
    artists_list: ListBox,
    playlists_list: ListBox,
    tracks: Rc<RefCell<Vec<Track>>>,
}

/// "My Wave" mood/energy presets: (API value, button label).
const VIBES: [(&str, &str); 5] = [
    ("all", "Any"),
    ("active", "Energetic"),
    ("fun", "Cheerful"),
    ("calm", "Calm"),
    ("sad", "Sad"),
];

/// The like/dislike widgets for one track, keyed by track id so the UI can
/// flip their state in place when the worker confirms a change. `liked` and
/// `disliked` mirror the last known server state (mutually exclusive).
#[derive(Clone)]
struct RowButtons {
    like: Button,
    dislike: Button,
    liked: Rc<Cell<bool>>,
    disliked: Rc<Cell<bool>>,
}

/// Apply the visual state of a like button (filled star + accent when liked).
fn style_like(button: &Button, liked: bool) {
    button.set_icon_name(if liked {
        "starred-symbolic"
    } else {
        "non-starred-symbolic"
    });
    if liked {
        button.add_css_class("suggested-action");
    } else {
        button.remove_css_class("suggested-action");
    }
}

/// Apply the visual state of a dislike button (red accent when disliked).
/// Uses a distinct "block" glyph so it never looks like the like (star) button.
fn style_dislike(button: &Button, disliked: bool) {
    button.set_icon_name("action-unavailable-symbolic");
    if disliked {
        button.add_css_class("destructive-action");
    } else {
        button.remove_css_class("destructive-action");
    }
}

/// A small icon-only like/dislike button. Deliberately not `flat`: the active
/// states rely on the accent background of `suggested-action`/`destructive-action`,
/// which `flat` can render invisible (see the vibe-selector bug).
fn rating_button() -> Button {
    let button = Button::new();
    button.set_icon_name("non-starred-symbolic");
    button
}

/// The vibe (mood/energy) picker shown on the My Wave page. The active preset
/// is highlighted with the `suggested-action` accent; clicks send `SetVibe`
/// (re-picking the active vibe refreshes the wave) and the UI is re-synced
/// when the worker confirms (`WaveMoodApplied`).
#[derive(Clone)]
struct VibeSelector {
    buttons: Rc<RefCell<Vec<(String, Button)>>>,
}

impl VibeSelector {
    fn new(worker: Worker) -> VibeSelector {
        let buttons = Rc::new(RefCell::new(Vec::new()));
        for &(mood, label) in &VIBES {
            let button = Button::with_label(label);
            let worker = worker.clone();
            button.connect_clicked(move |_| {
                worker.send(WorkerCommand::SetVibe { mood: mood.to_string() });
            });
            buttons.borrow_mut().push((mood.to_string(), button));
        }
        VibeSelector { buttons }
    }

    /// A `GtkBox` holding the label and the vibe buttons.
    fn row(&self) -> GtkBox {
        let row = GtkBox::new(Orientation::Horizontal, 6);
        let label = Label::new(Some("Vibe:"));
        label.add_css_class("dim-label");
        row.append(&label);
        for (_, button) in self.buttons.borrow().iter() {
            row.append(button);
        }
        row
    }

    /// Highlight `mood` (e.g. `"calm"`); "all" deselects the specific presets.
    fn set_mood(&self, mood: &str) {
        for (m, button) in self.buttons.borrow().iter() {
            if m == mood {
                button.add_css_class("suggested-action");
            } else {
                button.remove_css_class("suggested-action");
            }
        }
    }
}

#[derive(Clone)]
pub struct MainView {
    pub root: GtkBox,
    account_label: Label,
    content: Stack,
    liked_count_label: Label,
    liked_list: ListBox,
    playlists_count_label: Label,
    playlists_list: ListBox,
    liked_tracks: Rc<RefCell<Vec<Track>>>,
    wave_count_label: Label,
    wave_list: ListBox,
    wave_tracks: Rc<RefCell<Vec<Track>>>,
    vibe: VibeSelector,
    search: SearchPage,
    covers: Covers,
    worker: Worker,
    /// Like/dislike widgets per track id, updated in place on worker confirms.
    like_buttons: Rc<RefCell<HashMap<String, RowButtons>>>,
    /// The id of the track in the playback bar (empty = nothing playing).
    current_track_id: Rc<RefCell<String>>,
    now_like: Button,
    now_dislike: Button,
    now_liked: Rc<Cell<bool>>,
    now_disliked: Rc<Cell<bool>>,
    /// (page name, list) pairs whose visible rows should have their covers
    /// requested, checked periodically while that page is shown.
    cover_lists: Vec<(String, ListBox)>,
    now_cover: Image,
    now_playing_label: Label,
    play_button: Button,
    seek_scale: Scale,
    time_label: Label,
    time_total_label: Label,
    duration_secs: Rc<Cell<u64>>,
    updating_seek: Rc<Cell<bool>>,
    last_user_seek: Rc<Cell<u128>>,
}

impl MainView {
    pub fn new(worker: Worker, playback: Playback, account_label: Label) -> Self {
        // Sidebar.
        let sidebar = GtkBox::new(Orientation::Vertical, 0);
        sidebar.add_css_class("sidebar");
        sidebar.set_width_request(170);
        let home_btn = nav_button("Home");
        let wave_btn = nav_button("My Wave");
        let playlists_btn = nav_button("Playlists");
        let search_btn = nav_button("Search");
        sidebar.append(&home_btn);
        sidebar.append(&wave_btn);
        sidebar.append(&playlists_btn);
        sidebar.append(&search_btn);

        // Content stack.
        let content = Stack::new();

        let (home_page, liked_count_label, liked_list, refresh_liked) = build_list_page("Liked tracks");
        content.add_named(&home_page, Some("home"));
        let (wave_page, wave_count_label, wave_list, wave_tracks, vibe) = build_wave_page(
            worker.clone(),
            playback.clone(),
            Rc::new(RefCell::new(Vec::new())),
        );
        content.add_named(&wave_page, Some("wave"));
        let (playlists_page, playlists_count_label, playlists_list, refresh_playlists) =
            build_list_page("Playlists");
        content.add_named(&playlists_page, Some("playlists"));
        let search = build_search_page(worker.clone(), playback.clone());
        content.add_named(&search.root, Some("search"));
        content.set_visible_child_name("home");

        home_btn.connect_clicked({
            let content = content.clone();
            move |_| content.set_visible_child_name("home")
        });
        wave_btn.connect_clicked({
            let content = content.clone();
            move |_| content.set_visible_child_name("wave")
        });
        playlists_btn.connect_clicked({
            let content = content.clone();
            move |_| content.set_visible_child_name("playlists")
        });
        search_btn.connect_clicked({
            let content = content.clone();
            move |_| content.set_visible_child_name("search")
        });

        refresh_liked.connect_clicked({
            let worker = worker.clone();
            move |_| worker.send(WorkerCommand::FetchLiked)
        });
        refresh_playlists.connect_clicked({
            let worker = worker.clone();
            move |_| worker.send(WorkerCommand::FetchPlaylists)
        });

        // Double-clicking a liked track plays it and queues the rest.
        let liked_tracks: Rc<RefCell<Vec<Track>>> = Rc::new(RefCell::new(Vec::new()));
        liked_list.connect_row_activated({
            let liked_tracks = liked_tracks.clone();
            let playback = playback.clone();
            move |_, row| {
                let index = row.index() as usize;
                let tracks = liked_tracks.borrow().clone();
                if index < tracks.len() {
                    playback.play_queue(tracks, index);
                }
            }
        });

        // ---- Playback bar ----
        let covers = Covers::new(worker.clone());
        let now_cover = Image::new();
        now_cover.set_pixel_size(48);

        let now_playing_label = Label::new(Some("Nothing playing"));
        now_playing_label.add_css_class("dim-label");
        now_playing_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        now_playing_label.set_xalign(0.0);

        let prev_button = icon_button("media-skip-backward");
        let play_button = icon_button("media-playback-start");
        let next_button = icon_button("media-skip-forward");

        prev_button.connect_clicked({
            let playback = playback.clone();
            move |_| playback.prev()
        });
        next_button.connect_clicked({
            let playback = playback.clone();
            move |_| {
                playback.next();
            }
        });
        play_button.connect_clicked({
            let playback = playback.clone();
            move |_| playback.toggle()
        });

        let time_label = Label::new(Some("0:00"));
        time_label.set_width_request(48);
        time_label.set_xalign(1.0);
        let time_total_label = Label::new(Some("0:00"));
        time_total_label.set_width_request(48);
        time_total_label.set_xalign(0.0);

        let duration_secs: Rc<Cell<u64>> = Rc::new(Cell::new(0));
        let updating_seek: Rc<Cell<bool>> = Rc::new(Cell::new(false));
        let last_user_seek: Rc<Cell<u128>> = Rc::new(Cell::new(0));

        // Like/dislike for the currently playing track.
        let current_track_id: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));
        let now_liked: Rc<Cell<bool>> = Rc::new(Cell::new(false));
        let now_disliked: Rc<Cell<bool>> = Rc::new(Cell::new(false));
        let now_like = rating_button();
        let now_dislike = rating_button();
        style_like(&now_like, false);
        style_dislike(&now_dislike, false);
        {
            let current_track_id = current_track_id.clone();
            let worker = worker.clone();
            let state = now_liked.clone();
            now_like.connect_clicked(move |_| {
                let id = current_track_id.borrow().clone();
                if id.is_empty() {
                    return;
                }
                worker.send(WorkerCommand::SetLike { id, liked: !state.get() });
            });
        }
        {
            let current_track_id = current_track_id.clone();
            let worker = worker.clone();
            let state = now_disliked.clone();
            now_dislike.connect_clicked(move |_| {
                let id = current_track_id.borrow().clone();
                if id.is_empty() {
                    return;
                }
                worker.send(WorkerCommand::SetDislike { id, disliked: !state.get() });
            });
        }

        let seek_scale = Scale::with_range(Orientation::Horizontal, 0.0, 1000.0, 1.0);
        seek_scale.set_hexpand(true);
        seek_scale.set_draw_value(false);
        {
            let playback = playback.clone();
            let duration_secs = duration_secs.clone();
            let updating_seek = updating_seek.clone();
            let last_user_seek = last_user_seek.clone();
            let time_label = time_label.clone();
            seek_scale.connect_value_changed(move |scale| {
                if updating_seek.get() {
                    return;
                }
                last_user_seek.set(now_millis());
                let duration = duration_secs.get();
                if duration > 0 {
                    let target = ((scale.value() / 1000.0) * duration as f64) as u64;
                    // Update the timecode live while dragging so it tracks the
                    // slider, not the pipeline's lagging position.
                    time_label.set_text(&format_time(Some(ClockTime::from_seconds(target))));
                    playback.seek_seconds(target);
                }
            });
        }

        let saved_volume = crate::config::load_config()
            .volume
            .unwrap_or(100.0)
            .clamp(0.0, 100.0);
        let volume_scale = Scale::with_range(Orientation::Horizontal, 0.0, 100.0, 1.0);
        volume_scale.set_value(saved_volume);
        volume_scale.set_draw_value(false);
        volume_scale.set_width_request(90);
        {
            let playback = playback.clone();
            volume_scale.connect_value_changed(move |scale| {
                let volume = scale.value().clamp(0.0, 100.0) / 100.0;
                playback.set_volume(volume);
                let mut config = crate::config::load_config();
                config.volume = Some(volume * 100.0);
                let _ = crate::config::save_config(&config);
            });
        }
        // Apply the saved volume to the pipeline (GStreamer defaults to 1.0).
        playback.set_volume(saved_volume / 100.0);

        let bar = GtkBox::new(Orientation::Vertical, 4);
        bar.add_css_class("playback-bar");
        bar.set_margin_start(12);
        bar.set_margin_end(12);
        bar.set_margin_top(8);
        bar.set_margin_bottom(8);

        // Top row: track info (ellipsized, so a long title never squeezes the
        // transport controls), like/dislike, prev/play/next and volume.
        let bar_top = GtkBox::new(Orientation::Horizontal, 8);
        now_playing_label.set_hexpand(true);
        bar_top.append(&now_cover);
        bar_top.append(&now_playing_label);
        bar_top.append(&now_like);
        bar_top.append(&now_dislike);
        bar_top.append(&prev_button);
        bar_top.append(&play_button);
        bar_top.append(&next_button);
        bar_top.append(&volume_scale);

        // Bottom row: the seek slider spans the full bar width, with the
        // elapsed/total time on either side.
        let bar_bottom = GtkBox::new(Orientation::Horizontal, 8);
        bar_bottom.append(&time_label);
        seek_scale.set_hexpand(true);
        bar_bottom.append(&seek_scale);
        bar_bottom.append(&time_total_label);

        bar.append(&bar_top);
        bar.append(&bar_bottom);

        let middle = GtkBox::new(Orientation::Horizontal, 0);
        middle.append(&sidebar);
        middle.append(&content);

        let root = GtkBox::new(Orientation::Vertical, 0);
        root.append(&middle);
        root.append(&bar);

        let cover_lists: Vec<(String, ListBox)> = vec![
            ("home".to_string(), liked_list.clone()),
            ("wave".to_string(), wave_list.clone()),
            ("playlists".to_string(), playlists_list.clone()),
            ("search".to_string(), search.tracks_list.clone()),
            ("search".to_string(), search.albums_list.clone()),
            ("search".to_string(), search.artists_list.clone()),
            ("search".to_string(), search.playlists_list.clone()),
        ];

        Self {
            root,
            account_label,
            content,
            liked_count_label,
            liked_list,
            playlists_count_label,
            playlists_list,
            liked_tracks,
            wave_count_label,
            wave_list,
            wave_tracks,
            vibe,
            search,
            covers,
            worker,
            like_buttons: Rc::new(RefCell::new(HashMap::new())),
            current_track_id,
            now_like,
            now_dislike,
            now_liked,
            now_disliked,
            cover_lists,
            now_cover,
            now_playing_label,
            play_button,
            seek_scale,
            time_label,
            time_total_label,
            duration_secs,
            updating_seek,
            last_user_seek,
        }
    }

    /// Show the signed-in account name in the header.
    pub fn set_account(&self, uid: i64, display_name: &str) {
        self.account_label
            .set_text(&format!("{display_name} (uid {uid})"));
    }

    /// Show the currently loaded page by name.
    pub fn show_page(&self, name: &str) {
        self.content.set_visible_child_name(name);
    }

    /// Populate the liked-tracks list on the home page.
    pub fn set_liked(&self, tracks: &[Track]) {
        self.liked_tracks.replace(tracks.to_vec());
        self.liked_list.remove_all();
        self.liked_count_label
            .set_text(&format!("Liked tracks: {}", tracks.len()));
        for track in tracks {
            self.liked_list
                .append(&track_row(track, &self.covers, &self.worker, &self.like_buttons));
        }
    }

    /// Populate the playlists page.
    pub fn set_playlists(&self, playlists: &[Playlist]) {
        self.playlists_list.remove_all();
        self.playlists_count_label
            .set_text(&format!("Playlists: {}", playlists.len()));
        for playlist in playlists {
            let row = playlist_row(playlist, &self.covers);
            self.playlists_list.append(&row);
        }
    }

    /// Populate the My Wave list (first batch).
    pub fn set_wave(&self, tracks: &[Track]) {
        self.wave_tracks.replace(tracks.to_vec());
        self.wave_list.remove_all();
        self.wave_count_label
            .set_text(&format!("My Wave: {}", tracks.len()));
        for track in tracks {
            self.wave_list
                .append(&track_row(track, &self.covers, &self.worker, &self.like_buttons));
        }
    }

    /// Append a continuation batch to the My Wave list.
    pub fn append_wave(&self, tracks: &[Track]) {        {
            let mut all = self.wave_tracks.borrow_mut();
            for track in tracks {
                let id = track.id.as_ref().map(|id| id.0.clone()).unwrap_or_default();
                if !all.iter().any(|t| t.id.as_ref().map(|x| x.0.clone()).unwrap_or_default() == id) {
                    all.push(track.clone());
                    self.wave_list.append(&track_row(
                        track,
                        &self.covers,
                        &self.worker,
                        &self.like_buttons,
                    ));
                }
            }
        }
        self.wave_count_label
            .set_text(&format!("My Wave: {}", self.wave_tracks.borrow().len()));
    }

    /// Highlight the active vibe preset on the My Wave page.
    pub fn set_wave_mood(&self, mood: &str) {
        self.vibe.set_mood(mood);
    }

    /// Apply a confirmed like/dislike state to every widget showing the track.
    pub fn set_track_like(&self, id: &str, liked: bool, disliked: bool) {
        if let Some(buttons) = self.like_buttons.borrow().get(id) {
            buttons.liked.set(liked);
            buttons.disliked.set(disliked);
            style_like(&buttons.like, liked);
            style_dislike(&buttons.dislike, disliked);
        }
        if *self.current_track_id.borrow() == id {
            self.now_liked.set(liked);
            self.now_disliked.set(disliked);
            style_like(&self.now_like, liked);
            style_dislike(&self.now_dislike, disliked);
        }
    }

    /// Show the search results for a query across all entity types.
    pub fn set_search_results(
        &self,
        query: &str,
        tracks: &[Track],
        albums: &[Album],
        artists: &[Artist],
        playlists: &[Playlist],
    ) {
        self.search.tracks.replace(tracks.to_vec());
        self.search
            .summary
            .set_text(&format!("Results for \"{query}\""));
        self.search.summary.set_visible(true);
        self.search
            .tracks_header
            .set_text(&format!("Tracks ({})", tracks.len()));
        self.search
            .albums_header
            .set_text(&format!("Albums ({})", albums.len()));
        self.search
            .artists_header
            .set_text(&format!("Artists ({})", artists.len()));
        self.search
            .playlists_header
            .set_text(&format!("Playlists ({})", playlists.len()));

        self.search.tracks_list.remove_all();
        self.search.albums_list.remove_all();
        self.search.artists_list.remove_all();
        self.search.playlists_list.remove_all();

        set_section(&self.search.tracks_list, &self.search.tracks_header, tracks.len(), |list| {
            for track in tracks {
                list.append(&track_row(track, &self.covers, &self.worker, &self.like_buttons));
            }
        });
        set_section(&self.search.albums_list, &self.search.albums_header, albums.len(), |list| {
            for album in albums {
                list.append(&album_row(album, &self.covers));
            }
        });
        set_section(&self.search.artists_list, &self.search.artists_header, artists.len(), |list| {
            for artist in artists {
                list.append(&artist_row(artist, &self.covers));
            }
        });
        set_section(
            &self.search.playlists_list,
            &self.search.playlists_header,
            playlists.len(),
            |list| {
                for playlist in playlists {
                    list.append(&playlist_row(playlist, &self.covers));
                }
            },
        );
    }

    /// Show which track is currently playing.
    pub fn set_now_playing(&self, track: &Track) {
        let artist = track
            .artists
            .first()
            .map(|a| a.name.as_str())
            .unwrap_or("Unknown");
        let title = if track.title.is_empty() {
            "Untitled".to_string()
        } else {
            track.title.clone()
        };
        self.now_playing_label
            .set_text(&format!("{title} — {artist}"));
        self.now_playing_label.remove_css_class("dim-label");
        let id = track.id.as_ref().map(|i| i.0.clone()).unwrap_or_default();
        *self.current_track_id.borrow_mut() = id;
        self.now_liked.set(track.liked.unwrap_or(false));
        self.now_disliked.set(track.disliked.unwrap_or(false));
        style_like(&self.now_like, self.now_liked.get());
        style_dislike(&self.now_dislike, self.now_disliked.get());
        let cover = track_cover(track, 200);
        self.covers.apply(&self.now_cover, cover.clone());
        if let Some(url) = cover {
            // The bar is always visible, so load its art right away.
            self.covers.request(&url);
        }
    }

    /// Deliver a downloaded cover to the loader (main thread).
    pub fn on_cover_ready(&self, url: &str, bytes: Vec<u8>) {
        self.covers.on_ready(url, bytes);
    }

    /// Periodically request covers for the rows that are currently on screen.
    pub fn poll_covers(&self) {
        let Some(current) = self.content.visible_child_name() else {
            return;
        };
        for (page, list) in &self.cover_lists {
            if *page != current {
                continue;
            }
            request_visible_covers(list, &self.covers);
        }
    }

    /// Update the play/pause button to match the pipeline state.
    pub fn set_playing(&self, playing: bool) {
        self.play_button.set_icon_name(if playing {
            "media-playback-pause"
        } else {
            "media-playback-start"
        });
    }

    /// Refresh the seek scale and time label from the player position.
    pub fn update_progress(&self, position: Option<ClockTime>, duration: Option<ClockTime>) {
        let duration_secs = duration.map(|d| d.seconds()).unwrap_or(0);
        self.duration_secs.set(duration_secs);
        if self.updating_seek.get() {
            return;
        }
        // While the user is dragging the slider (or just released it) the live
        // timecode in the scale handler wins, so don't overwrite it with the
        // pipeline's stale position.
        let recently_interacted = now_millis().saturating_sub(self.last_user_seek.get()) < 1000;
        if recently_interacted {
            return;
        }
        self.time_label.set_text(&format_time(position));
        self.time_total_label.set_text(&format_time(duration));
        if let Some(position) = position {
            if duration_secs > 0 {
                let fraction = position.seconds() as f64 / duration_secs as f64;
                self.updating_seek.set(true);
                self.seek_scale.set_value(fraction * 1000.0);
                self.updating_seek.set(false);
            }
        }
    }
}

fn nav_button(label: &str) -> Button {
    let button = Button::with_label(label);
    button.set_halign(Align::Start);
    button.add_css_class("flat");
    button.add_css_class("nav");
    button
}

fn icon_button(icon: &str) -> Button {
    let button = Button::new();
    button.set_icon_name(icon);
    button.add_css_class("flat");
    button
}

fn track_row(
    track: &Track,
    covers: &Covers,
    worker: &Worker,
    buttons: &Rc<RefCell<HashMap<String, RowButtons>>>,
) -> ListBoxRow {
    let artist = track
        .artists
        .first()
        .map(|a| a.name.as_str())
        .unwrap_or("Unknown");
    let title = if track.title.is_empty() {
        "Untitled".to_string()
    } else {
        track.title.clone()
    };
    let text = format!("{title} — {artist}");
    let label = Label::new(Some(&text));
    label.set_xalign(0.0);
    label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    let image = covers.image(track_cover(track, 200), 36);

    let row_content = GtkBox::new(Orientation::Horizontal, 8);
    row_content.append(&image);
    row_content.append(&label);

    if let Some(id) = track.id.as_ref().map(|id| id.0.clone()) {
        let spacer = GtkBox::new(Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        row_content.append(&spacer);

        let liked = Rc::new(Cell::new(track.liked.unwrap_or(false)));
        let disliked = Rc::new(Cell::new(track.disliked.unwrap_or(false)));
        let like = rating_button();
        let dislike = rating_button();
        style_like(&like, liked.get());
        style_dislike(&dislike, disliked.get());

        {
            let id = id.clone();
            let worker = worker.clone();
            let state = liked.clone();
            like.connect_clicked(move |_| {
                let target = !state.get();
                worker.send(WorkerCommand::SetLike { id: id.clone(), liked: target });
            });
        }
        {
            let id = id.clone();
            let worker = worker.clone();
            let state = disliked.clone();
            dislike.connect_clicked(move |_| {
                let target = !state.get();
                worker.send(WorkerCommand::SetDislike { id: id.clone(), disliked: target });
            });
        }

        buttons.borrow_mut().insert(
            id.clone(),
            RowButtons {
                like: like.clone(),
                dislike: dislike.clone(),
                liked,
                disliked,
            },
        );
        row_content.append(&like);
        row_content.append(&dislike);
    }

    let row = ListBoxRow::new();
    row.set_child(Some(&row_content));
    row
}

fn playlist_row(playlist: &Playlist, covers: &Covers) -> ListBoxRow {
    let count = playlist.track_count.unwrap_or(0);
    let text = format!("{}  ({count} tracks)", playlist.title);
    let label = Label::new(Some(&text));
    label.set_xalign(0.0);
    label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    let image = covers.image(playlist_cover(playlist, 200), 36);

    let row_content = GtkBox::new(Orientation::Horizontal, 8);
    row_content.append(&image);
    row_content.append(&label);
    let row = ListBoxRow::new();
    row.set_child(Some(&row_content));
    row
}

fn build_list_page(title: &str) -> (GtkBox, Label, ListBox, Button) {
    let header = GtkBox::new(Orientation::Horizontal, 8);
    let count_label = Label::new(Some(title));
    count_label.add_css_class("heading");
    let refresh = Button::with_label("Refresh");
    header.append(&count_label);
    header.append(&refresh);

    let list = ListBox::new();
    list.set_selection_mode(SelectionMode::None);

    let scroller = ScrolledWindow::new();
    scroller.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    scroller.set_child(Some(&list));
    scroller.set_vexpand(true);

    let page = GtkBox::new(Orientation::Vertical, 8);
    page.set_margin_start(16);
    page.set_margin_end(16);
    page.set_margin_top(12);
    page.set_margin_bottom(12);
    page.append(&header);
    page.append(&scroller);
    (page, count_label, list, refresh)
}

fn build_wave_page(
    worker: Worker,
    playback: Playback,
    wave_tracks: Rc<RefCell<Vec<Track>>>,
) -> (GtkBox, Label, ListBox, Rc<RefCell<Vec<Track>>>, VibeSelector) {
    let count_label = Label::new(Some("My Wave"));
    count_label.add_css_class("heading");
    let play = Button::with_label("Play");
    let header = GtkBox::new(Orientation::Horizontal, 8);
    header.append(&count_label);
    header.append(&play);

    let vibe = VibeSelector::new(worker.clone());
    vibe.set_mood("all");

    play.connect_clicked({
        let wave_tracks = wave_tracks.clone();
        let playback = playback.clone();
        let worker = worker.clone();
        move |_| {
            let tracks = wave_tracks.borrow().clone();
            if tracks.is_empty() {
                worker.send(WorkerCommand::FetchWave { queue: None });
            } else {
                playback.play_queue(tracks, 0);
            }
        }
    });

    let list = ListBox::new();
    list.set_selection_mode(SelectionMode::None);
    list.connect_row_activated({
        let wave_tracks = wave_tracks.clone();
        let playback = playback.clone();
        move |_, row| {
            let index = row.index() as usize;
            let tracks = wave_tracks.borrow().clone();
            if index < tracks.len() {
                playback.play_queue(tracks, index);
            }
        }
    });

    let scroller = ScrolledWindow::new();
    scroller.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    scroller.set_child(Some(&list));
    scroller.set_vexpand(true);

    let page = GtkBox::new(Orientation::Vertical, 8);
    page.set_margin_start(16);
    page.set_margin_end(16);
    page.set_margin_top(12);
    page.set_margin_bottom(12);
    page.append(&header);
    page.append(&vibe.row());
    page.append(&scroller);
    (page, count_label, list, wave_tracks, vibe)
}

fn album_row(album: &Album, covers: &Covers) -> ListBoxRow {
    let artists = album
        .artists
        .iter()
        .map(|a| a.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let year = album.year.map(|y| y.to_string()).unwrap_or_default();
    let title = if album.title.is_empty() {
        "Untitled".to_string()
    } else {
        album.title.clone()
    };
    let text = format!("{title} — {artists} ({year})");
    let label = Label::new(Some(&text));
    label.set_xalign(0.0);
    label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    let image = covers.image(album_cover(album, 200), 36);

    let row_content = GtkBox::new(Orientation::Horizontal, 8);
    row_content.append(&image);
    row_content.append(&label);
    let row = ListBoxRow::new();
    row.set_child(Some(&row_content));
    row
}

fn artist_row(artist: &Artist, covers: &Covers) -> ListBoxRow {
    let text = if artist.name.is_empty() {
        "Unknown artist".to_string()
    } else {
        artist.name.clone()
    };
    let label = Label::new(Some(&text));
    label.set_xalign(0.0);
    label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    let image = covers.image(artist_cover(artist, 200), 36);

    let row_content = GtkBox::new(Orientation::Horizontal, 8);
    row_content.append(&image);
    row_content.append(&label);
    let row = ListBoxRow::new();
    row.set_child(Some(&row_content));
    row
}

fn section_header(text: &str) -> Label {
    let label = Label::new(Some(text));
    label.add_css_class("heading");
    label.set_halign(Align::Start);
    label
}

/// The cover `Image` inside a row (rows are `[cover, label]` in an hbox).
fn row_cover_image(row: &ListBoxRow) -> Option<gtk4::Image> {
    let hbox = row.first_child()?.downcast::<GtkBox>().ok()?;
    let image = hbox.first_child()?;
    image.downcast::<gtk4::Image>().ok()
}

/// Request covers for the rows of `list` that intersect the visible viewport.
/// Rows outside the current page or not yet laid out report no coordinates and
/// are skipped, which is what keeps downloads limited to what is on screen.
fn request_visible_covers(list: &ListBox, covers: &Covers) {
    let Some(win) = list
        .ancestor(gtk4::ScrolledWindow::static_type())
        .and_downcast::<gtk4::ScrolledWindow>()
    else {
        return;
    };
    let height = win.height();
    if height <= 0 {
        return;
    }
    let mut index = 0;
    while let Some(row) = list.row_at_index(index) {
        index += 1;
        let Some(point) = row.compute_point(&win, &gtk4::graphene::Point::new(0.0, 0.0)) else {
            continue;
        };
        let y = point.y() as f64;
        let h = row.height();
        if y + h as f64 <= 0.0 {
            continue;
        }
        if y >= height as f64 {
            break;
        }
        if let Some(image) = row_cover_image(&row) {
            covers.request_image(&image);
        }
    }
}

/// Show or hide a results section and fill it with rows.
fn set_section(list: &ListBox, header: &Label, count: usize, fill: impl FnOnce(&ListBox)) {
    let empty = count == 0;
    header.set_visible(!empty);
    list.set_visible(!empty);
    if !empty {
        fill(list);
    }
}

fn build_search_page(worker: Worker, playback: Playback) -> SearchPage {
    let entry = Entry::new();
    entry.set_placeholder_text(Some("Search tracks, albums, artists…"));
    entry.set_hexpand(true);
    let button = Button::with_label("Search");

    let search_row = GtkBox::new(Orientation::Horizontal, 8);
    search_row.append(&entry);
    search_row.append(&button);

    let summary = Label::new(None);
    summary.add_css_class("dim-label");
    summary.set_halign(Align::Start);
    summary.set_visible(false);

    let tracks_header = section_header("Tracks");
    let albums_header = section_header("Albums");
    let artists_header = section_header("Artists");
    let playlists_header = section_header("Playlists");

    let tracks_list = ListBox::new();
    tracks_list.set_selection_mode(SelectionMode::None);
    let albums_list = ListBox::new();
    albums_list.set_selection_mode(SelectionMode::None);
    let artists_list = ListBox::new();
    artists_list.set_selection_mode(SelectionMode::None);
    let playlists_list = ListBox::new();
    playlists_list.set_selection_mode(SelectionMode::None);

    let tracks: Rc<RefCell<Vec<Track>>> = Rc::new(RefCell::new(Vec::new()));
    tracks_list.connect_row_activated({
        let tracks = tracks.clone();
        let playback = playback.clone();
        move |_, row| {
            let index = row.index() as usize;
            let all = tracks.borrow().clone();
            if index < all.len() {
                playback.play_queue(all, index);
            }
        }
    });

    entry.connect_activate({
        let worker = worker.clone();
        move |entry| {
            let text = entry.text().trim().to_string();
            if !text.is_empty() {
                worker.send(WorkerCommand::Search(text));
            }
        }
    });
    button.connect_clicked({
        let entry = entry.clone();
        let worker = worker.clone();
        move |_| {
            let text = entry.text().trim().to_string();
            if !text.is_empty() {
                worker.send(WorkerCommand::Search(text));
            }
        }
    });

    let scroller = ScrolledWindow::new();
    scroller.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    scroller.set_vexpand(true);
    let page = GtkBox::new(Orientation::Vertical, 4);
    page.set_margin_start(16);
    page.set_margin_end(16);
    page.set_margin_top(12);
    page.set_margin_bottom(12);
    page.append(&search_row);
    page.append(&summary);
    page.append(&tracks_header);
    page.append(&tracks_list);
    page.append(&albums_header);
    page.append(&albums_list);
    page.append(&artists_header);
    page.append(&artists_list);
    page.append(&playlists_header);
    page.append(&playlists_list);
    scroller.set_child(Some(&page));

    let root = GtkBox::new(Orientation::Vertical, 0);
    root.append(&scroller);

    SearchPage {
        root,
        summary,
        tracks_header,
        albums_header,
        artists_header,
        playlists_header,
        tracks_list,
        albums_list,
        artists_list,
        playlists_list,
        tracks,
    }
}
