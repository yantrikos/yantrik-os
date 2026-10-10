//! Local music library scan: walk a folder for audio files and read what a file name alone
//! can tell us.
//!
//! No tag crate is in the workspace `Cargo.toml` (no `lofty`/`id3`/`symphonia` entry), so tags
//! are not read: the file name is the title, the parent directory is the album, and
//! `artist`/`genre` stay empty. `duration_secs` stays `0.0` for the same reason; the duration
//! text is still well-formed.

use std::path::Path;

/// One track found by a library scan. Pure data, no UI types.
pub struct Track {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub genre: String,
    pub duration_secs: f64,
    pub path: String,
}

const AUDIO_EXTENSIONS: &[&str] = &["mp3", "flac", "ogg", "wav", "m4a"];

fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| AUDIO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Walk `root` recursively (an `Artist/Album/track.flac` tree), keeping audio files. The result
/// is sorted by `(artist, album, title)` so the library is stable between scans.
pub fn scan_folder(root: &Path) -> Vec<Track> {
    let mut tracks = Vec::new();
    walk(root, &mut tracks);
    tracks.sort_by(|a, b| {
        (&a.artist, &a.album, &a.title).cmp(&(&b.artist, &b.album, &b.title))
    });
    tracks
}

fn walk(dir: &Path, tracks: &mut Vec<Track>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, tracks);
        } else if is_audio(&path) {
            tracks.push(track_from(&path));
        }
    }
}

fn track_from(path: &Path) -> Track {
    let title = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string();
    let album = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string();
    Track {
        title,
        artist: String::new(),
        album,
        genre: String::new(),
        duration_secs: 0.0,
        path: path.to_string_lossy().into_owned(),
    }
}

/// `m:ss` — minutes unpadded, seconds zero-padded. Anything negative or non-finite is `0:00`.
pub fn duration_text(secs: f64) -> String {
    if !secs.is_finite() || secs < 0.0 {
        return "0:00".to_string();
    }
    let total = secs.round() as u64;
    let minutes = total / 60;
    let seconds = total % 60;
    format!("{}:{:02}", minutes, seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unique temp directory per call, so parallel tests never share a path.
    fn temp_dir(tag: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "yantrik-music-player-{tag}-{}-{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn duration_text_renders_minutes_and_seconds() {
        assert_eq!(duration_text(0.0), "0:00");
        assert_eq!(duration_text(187.0), "3:07");
        assert_eq!(duration_text(725.0), "12:05");
        assert_eq!(duration_text(-1.0), "0:00");
        assert_eq!(duration_text(f64::NAN), "0:00");
        assert_eq!(duration_text(f64::INFINITY), "0:00");
    }

    #[test]
    fn scan_folder_keeps_audio_files_and_reads_album_from_parent() {
        let root = temp_dir("scan");
        let album = root.join("Artist").join("Album");
        std::fs::create_dir_all(&album).expect("album dir");
        std::fs::write(album.join("one.flac"), b"").expect("one.flac");
        std::fs::write(root.join("two.mp3"), b"").expect("two.mp3");
        std::fs::write(root.join("notes.txt"), b"not audio").expect("notes.txt");

        let tracks = scan_folder(&root);
        assert_eq!(tracks.len(), 2, "notes.txt is excluded, the two audio files are kept");

        // Sorted by (artist, album, title): both artists are empty, so album then title.
        // The flac's album is "Album"; the mp3's album is the temp dir's own name.
        let flac = tracks.iter().find(|t| t.title == "one").expect("one.flac kept");
        assert_eq!(flac.album, "Album");
        let mp3 = tracks.iter().find(|t| t.title == "two").expect("two.mp3 kept");
        assert_eq!(mp3.album, root.file_name().unwrap().to_str().unwrap());

        std::fs::remove_dir_all(&root).ok();
    }
}
