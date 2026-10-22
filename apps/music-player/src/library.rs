//! Local music library scan: walk a folder for audio files and read what a file name alone
//! can tell us.
//!
//! Tags are out of scope: they need a metadata crate. So the file stem is the title and the
//! folder the file sits in is the album, which is how most collections are laid out on disk.
//! A track's length is not read here; mpv reports it once the track plays.
//!
//! The scan runs in a job (`update/library.rs`), so nothing here touches the UI. It never
//! follows a symlinked folder: a link back up the tree would recurse forever, and a link from
//! `~/Music` to `~` would scan the whole home.

use std::path::{Path, PathBuf};

/// One track found by a library scan. Pure data, no UI types.
#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    pub title: String,
    /// The name of the folder the file is in.
    pub album: String,
    pub path: String,
}

const AUDIO_EXTENSIONS: &[&str] = &["flac", "mp3", "ogg", "opus", "wav", "m4a"];

pub fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| AUDIO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Walk `root` recursively, keeping one `Track` per recognised audio file, sorted by title so
/// the result is stable between scans.
#[cfg(test)]
pub fn scan_folder(root: &Path) -> Vec<Track> {
    scan_folder_with(root, &mut |_| {})
}

/// [`scan_folder`], calling `progress` with the running count every [`PROGRESS_EVERY`] tracks.
pub fn scan_folder_with(root: &Path, progress: &mut dyn FnMut(usize)) -> Vec<Track> {
    let mut tracks = Vec::new();
    walk(root, &mut tracks, progress);
    tracks.sort_by(|a, b| a.title.cmp(&b.title).then_with(|| a.path.cmp(&b.path)));
    tracks
}

/// How often a scan says how far it has got.
pub const PROGRESS_EVERY: usize = 100;

fn walk(dir: &Path, tracks: &mut Vec<Track>, progress: &mut dyn FnMut(usize)) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        // The entry's own type, not what a symlink points at: a link back up the tree would
        // recurse until the stack overflows, and a link from ~/Music to ~ would scan the home.
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_dir() {
            walk(&path, tracks, progress);
        } else if is_audio(&path) {
            tracks.push(track_from(&path));
            if tracks.len().is_multiple_of(PROGRESS_EVERY) {
                progress(tracks.len());
            }
        }
    }
}

pub fn track_from(path: &Path) -> Track {
    let title = path.file_stem().map(|s| plain(&s.to_string_lossy())).unwrap_or_default();
    let album = path
        .parent()
        .and_then(|p| p.file_name())
        .map(|s| plain(&s.to_string_lossy()))
        .unwrap_or_default();
    Track { title, album, path: path.to_string_lossy().into_owned() }
}

/// An audio file on disk, by its absolute path: never anything mpv would read as a URL, a
/// playlist or an option. Reads the disk: from a job only.
pub fn audio_file(path: &Path) -> Result<PathBuf, String> {
    let said = |p: &Path| plain(&p.to_string_lossy());
    let path = std::fs::canonicalize(path).map_err(|e| format!("There is no file at {}: {e}.", said(path)))?;
    if !path.is_file() {
        return Err(format!("{} is not a file.", said(&path)));
    }
    if !is_audio(&path) {
        return Err(format!("{} is not a sound file Music plays (mp3, flac, ogg, opus, wav, m4a).", said(&path)));
    }
    Ok(path)
}

/// The title a path is shown by when it is not a track in the queue: what mpv says it loaded.
pub fn title_of(path: &str) -> String {
    Path::new(path).file_stem().map(|s| plain(&s.to_string_lossy())).unwrap_or_default()
}

/// A file or folder name as one line of text. Anyone can name a file, so a name is untrusted: a
/// control character (a newline, an escape) or a direction override (which makes `gpj.exe` read
/// as `exe.jpg`) is shown as U+FFFD, so a name cannot break a line of the window, the notice, a
/// sentence to the Mind or the log, or disguise itself. The path is kept as it is: it is what mpv
/// is handed, as one JSON string.
pub fn plain(name: &str) -> String {
    name.chars().map(|c| if c.is_control() || is_direction_control(c) { '\u{FFFD}' } else { c }).collect()
}

fn is_direction_control(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200E}' | '\u{200F}' | '\u{061C}')
        // The line and paragraph separators, which some renderers break a line on.
        || matches!(c, '\u{2028}' | '\u{2029}')
}

/// What scanning every library folder found.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Library {
    /// Every track once, by path, sorted by title.
    pub tracks: Vec<Track>,
    /// How many tracks each folder holds, in the order the folders were given. A track inside
    /// two listed folders (one nested in the other) counts in both, and is in `tracks` once.
    pub per_folder: Vec<usize>,
    /// Folders that could not be read at all, with why.
    pub unreadable: Vec<(String, String)>,
}

/// Scan every folder in `folders` and merge the results. `progress` gets the running total.
pub fn scan_all(folders: &[String], progress: &mut dyn FnMut(usize)) -> Library {
    let mut seen = std::collections::HashSet::new();
    let mut library = Library::default();
    let mut done = 0;
    for folder in folders {
        let root = Path::new(folder);
        if let Err(e) = std::fs::read_dir(root) {
            library.unreadable.push((folder.clone(), e.to_string()));
            library.per_folder.push(0);
            continue;
        }
        let found = scan_folder_with(root, &mut |n| progress(done + n));
        done += found.len();
        library.per_folder.push(found.len());
        for track in found {
            if seen.insert(track.path.clone()) {
                library.tracks.push(track);
            }
        }
    }
    library.tracks.sort_by(|a, b| a.title.cmp(&b.title).then_with(|| a.path.cmp(&b.path)));
    library
}

/// `1 track`, `2 tracks`, `0 folders`.
pub fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The sentence a finished scan leaves on the status line. `name` says how each folder that could
/// not be read is named in it.
pub fn summary(library: &Library, folders: usize, name: impl Fn(&str) -> String) -> String {
    let mut said = format!(
        "{} in {}",
        count(library.tracks.len(), "track", "tracks"),
        count(folders, "folder", "folders")
    );
    if !library.unreadable.is_empty() {
        let names: Vec<String> = library.unreadable.iter().map(|(f, _)| plain(&name(f))).collect();
        said.push_str(&format!(
            " — could not read {}: {}",
            if names.len() == 1 { "this folder" } else { "these folders" },
            names.join(", ")
        ));
    }
    said
}

/// `m:ss`, or `h:mm:ss` past an hour; empty when mpv has not said.
pub fn clock(secs: Option<f64>) -> String {
    let Some(secs) = secs.filter(|s| s.is_finite() && *s >= 0.0) else { return String::new() };
    let total = secs.floor() as u64;
    let (h, m, s) = (total / 3600, (total / 60) % 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
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
    fn scan_finds_audio_and_ignores_the_rest() {
        let root = temp_dir("scan");
        std::fs::write(root.join("a.flac"), b"").expect("a.flac");
        std::fs::write(root.join("b.MP3"), b"").expect("b.MP3");
        std::fs::write(root.join("c.txt"), b"not audio").expect("c.txt");
        let sub = root.join("sub");
        std::fs::create_dir_all(&sub).expect("sub dir");
        std::fs::write(sub.join("d.opus"), b"").expect("d.opus");

        let tracks = scan_folder(&root);
        let titles: Vec<&str> = tracks.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, vec!["a", "b", "d"], "c.txt is excluded, the three audio files are kept");

        let a = tracks.iter().find(|t| t.title == "a").expect("a.flac kept");
        assert_eq!(a.title, "a");
        assert!(a.path.ends_with("a.flac"), "path is the file's own: {}", a.path);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn the_album_is_the_folder_the_file_is_in() {
        let root = temp_dir("album");
        std::fs::create_dir_all(root.join("Artist/Album")).unwrap();
        std::fs::write(root.join("Artist/Album/one.flac"), b"").unwrap();
        let tracks = scan_folder(&root);
        assert_eq!(tracks[0].album, "Album");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn every_folder_gets_its_real_count_and_a_shared_track_is_listed_once() {
        let root = temp_dir("counts");
        std::fs::create_dir_all(root.join("inner")).unwrap();
        for n in 0..3 {
            std::fs::write(root.join(format!("{n}.mp3")), b"").unwrap();
        }
        std::fs::write(root.join("inner/x.ogg"), b"").unwrap();
        let missing = root.join("nope").to_string_lossy().into_owned();
        let folders = vec![
            root.to_string_lossy().into_owned(),
            root.join("inner").to_string_lossy().into_owned(),
            missing.clone(),
        ];
        let mut ticks = Vec::new();
        let lib = scan_all(&folders, &mut |n| ticks.push(n));
        assert_eq!(lib.per_folder, vec![4, 1, 0]);
        assert_eq!(lib.tracks.len(), 4, "inner/x.ogg is in both folders and listed once");
        assert_eq!(lib.unreadable.len(), 1);
        assert_eq!(lib.unreadable[0].0, missing);
        let said = summary(&lib, folders.len(), str::to_string);
        assert!(said.starts_with("4 tracks in 3 folders — could not read this folder: "), "{said}");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_large_scan_reports_progress_as_it_goes() {
        let root = temp_dir("progress");
        for n in 0..250 {
            std::fs::write(root.join(format!("{n:03}.mp3")), b"").unwrap();
        }
        let mut ticks = Vec::new();
        let lib = scan_all(&[root.to_string_lossy().into_owned()], &mut |n| ticks.push(n));
        assert_eq!(lib.tracks.len(), 250);
        assert_eq!(ticks, vec![100, 200]);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn counts_are_plural_only_when_they_should_be() {
        assert_eq!(count(1, "track", "tracks"), "1 track");
        assert_eq!(count(0, "track", "tracks"), "0 tracks");
        assert_eq!(count(2, "folder", "folders"), "2 folders");
        let one = Library { tracks: vec![track_from(Path::new("/m/a/b.mp3"))], per_folder: vec![1], unreadable: vec![] };
        assert_eq!(summary(&one, 1, str::to_string), "1 track in 1 folder");
    }

    #[test]
    fn the_clock_reads_what_mpv_said() {
        assert_eq!(clock(None), "");
        assert_eq!(clock(Some(0.0)), "0:00");
        assert_eq!(clock(Some(187.9)), "3:07");
        assert_eq!(clock(Some(3725.0)), "1:02:05");
        assert_eq!(clock(Some(f64::NAN)), "");
    }

    #[test]
    fn scan_of_a_missing_folder_is_empty() {
        let missing = temp_dir("missing").join("does-not-exist");
        let tracks = scan_folder(&missing);
        assert!(tracks.is_empty(), "a missing folder yields no tracks, not a panic");
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_loop_is_not_followed() {
        let dir = temp_dir("loop");
        std::fs::create_dir_all(dir.join("album")).unwrap();
        std::fs::write(dir.join("album/song.mp3"), b"").unwrap();
        std::os::unix::fs::symlink(&dir, dir.join("album/back-up")).unwrap();
        let tracks = scan_folder(&dir);
        assert_eq!(tracks.len(), 1, "the song once, the loop not walked");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_name_cannot_break_a_line_or_turn_itself_around() {
        let t = track_from(Path::new("/m/Al\nbum/one\u{1b}[2J\u{202E}3pm.mp3"));
        assert_eq!(t.title, "one\u{FFFD}[2J\u{FFFD}3pm");
        assert_eq!(t.album, "Al\u{FFFD}bum");
        assert_eq!(t.path, "/m/Al\nbum/one\u{1b}[2J\u{202E}3pm.mp3", "mpv is handed the file's own path");
        assert_eq!(title_of("/x/Ünïcode — fine.flac"), "Ünïcode — fine");
        assert_eq!(plain("a\u{2028}b\u{2029}c"), "a\u{FFFD}b\u{FFFD}c", "the separators some renderers break on");
    }
}
