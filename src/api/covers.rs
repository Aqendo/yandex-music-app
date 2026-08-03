//! Cover-art URL helpers.
//!
//! The API sends cover URIs as templates with a `%%` placeholder that must be
//! replaced with a concrete pixel size (e.g. `400x400`). Templates have no
//! scheme; the full `https://` URL is assembled here. Mosaic covers (playlists)
//! carry tile templates in `itemsUri`; we pick the first tile.
use crate::api::models::{Album, Artist, Cover, Playlist, Track};

/// Cover sizes the CDN actually serves; any other size 404s.
const VALID_SIZES: [u32; 6] = [100, 150, 200, 300, 400, 600];

/// Snap a requested size to the nearest served size.
fn normalize_size(size: u32) -> u32 {
    VALID_SIZES
        .iter()
        .copied()
        .min_by_key(|s| s.abs_diff(size))
        .unwrap_or(200)
}

/// Replace the `%%` placeholder with `{size}x{size}` and prefix `https://`.
fn resolve(uri: Option<&str>, size: u32) -> Option<String> {
    let uri = uri?.trim();
    if uri.is_empty() {
        return None;
    }
    let url = if uri.starts_with("http://") || uri.starts_with("https://") {
        uri.to_string()
    } else {
        format!("https://{uri}")
    };
    let size = normalize_size(size);
    Some(url.replace("%%", &format!("{size}x{size}")))
}

/// Resolve a standalone `Cover` model (artist / playlist) to an image URL.
pub fn cover_url(cover: &Cover, size: u32) -> Option<String> {
    cover
        .items_uri
        .first()
        .map(|uri| uri.as_str())
        .or(cover.uri.as_deref())
        .and_then(|uri| resolve(Some(uri), size))
}

/// Cover for a track: the track's own art, else its first album's.
pub fn track_cover(track: &Track, size: u32) -> Option<String> {
    resolve(track.cover_uri.as_deref(), size)
        .or_else(|| track.albums.first().and_then(|a| album_cover(a, size)))
}

/// Cover for an album.
pub fn album_cover(album: &Album, size: u32) -> Option<String> {
    resolve(album.cover_uri.as_deref(), size)
}

/// Cover for a playlist (mosaic: first tile).
pub fn playlist_cover(playlist: &Playlist, size: u32) -> Option<String> {
    playlist.cover.as_ref().and_then(|c| cover_url(c, size))
}

/// Cover for an artist.
pub fn artist_cover(artist: &Artist, size: u32) -> Option<String> {
    artist.cover.as_ref().and_then(|c| cover_url(c, size))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_replaces_placeholder_and_adds_scheme() {
        assert_eq!(
            resolve(Some("avatars.yandex.net/get-mpic/123/%%"), 400),
            Some("https://avatars.yandex.net/get-mpic/123/400x400".to_string())
        );
    }

    #[test]
    fn resolve_snaps_to_served_sizes() {
        assert_eq!(
            resolve(Some("x/%%"), 128),
            Some("https://x/150x150".to_string())
        );
        assert_eq!(
            resolve(Some("x/%%"), 96),
            Some("https://x/100x100".to_string())
        );
        assert_eq!(
            resolve(Some("x/%%"), 500),
            Some("https://x/400x400".to_string())
        );
    }

    #[test]
    fn resolve_tolerates_empty_and_full_urls() {
        assert_eq!(resolve(None, 100), None);
        assert_eq!(resolve(Some("  "), 100), None);
        assert_eq!(
            resolve(Some("https://host/x/%%"), 50),
            Some("https://host/x/100x100".to_string())
        );
    }

    #[test]
    fn track_cover_falls_back_to_album() {
        let with_own = Track {
            cover_uri: Some("own/%%".into()),
            ..Default::default()
        };
        assert_eq!(
            track_cover(&with_own, 64),
            Some("https://own/100x100".to_string())
        );

        let album = Album {
            cover_uri: Some("alb/%%".into()),
            ..Default::default()
        };
        let fallback = Track {
            cover_uri: None,
            albums: vec![album],
            ..Default::default()
        };
        assert_eq!(
            track_cover(&fallback, 64),
            Some("https://alb/100x100".to_string())
        );

        let empty = Track::default();
        assert_eq!(track_cover(&empty, 64), None);
    }

    #[test]
    fn playlist_cover_uses_first_mosaic_tile() {
        let cover = Cover {
            kind: Some("mosaic".into()),
            items_uri: vec!["tile1/%%".into(), "tile2/%%".into()],
            ..Default::default()
        };
        let playlist = Playlist {
            cover: Some(cover),
            ..Default::default()
        };
        assert_eq!(
            playlist_cover(&playlist, 200),
            Some("https://tile1/200x200".to_string())
        );
    }

    #[test]
    fn artist_cover_uses_single_uri() {
        let artist = Artist {
            cover: Some(Cover {
                kind: Some("pic".into()),
                uri: Some("art/%%".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(
            artist_cover(&artist, 300),
            Some("https://art/300x300".to_string())
        );
    }
}
