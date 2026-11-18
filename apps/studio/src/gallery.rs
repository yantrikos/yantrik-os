//! The pictures, the folders they land in, and the sidecar that says how each was made.
//!
//! Everything Studio generates is written under `~/Pictures/Studio/<date>/` with a `.json` beside
//! it holding the prompt, the seed, the backend, the model and the seconds. Two reasons, and the
//! second is the interesting one: a person can look at a picture later and know what produced it,
//! and a mind can read that sidecar back and answer "make another like this" without being told.
//! The sidecar is the part that makes generation something the rest of the OS can reason about
//! rather than a pile of PNGs.
//!
//! Nothing here knows about Slint. Reading and writing files is separated from showing them so
//! the gallery can be tested in a temporary directory.

use std::path::{Path, PathBuf};

use std::time::UNIX_EPOCH;

use chrono::{DateTime, Local, SecondsFormat};
use serde::{Deserialize, Serialize};

/// How many pictures `describe` carries. A gallery is a glance, not a transcript: the caller that
/// wants more can read the folder, which is the point of writing real files there.
pub const LISTING_CAP: usize = 12;

/// How far back the listing looks before giving up. Without a bound, a folder a person has been
/// filling for a year would be walked on every `describe`, which happens on the UI thread.
const SCAN_LIMIT: usize = 400;

/// The longest edge of a thumbnail. Small enough that a gallery of twelve costs a fraction of a
/// megabyte to hold and almost nothing to draw.
pub const THUMBNAIL_EDGE: u32 = 160;

/// What is written beside every picture. Every field is something a person or a mind would ask
/// about a generated image, and nothing is a field that only this app understands.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sidecar {
    /// The sentence that made the picture.
    pub prompt: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub negative: String,
    pub seed: u64,
    /// Which backend made it: `comfyui`, `openai-images` or `fake`.
    pub backend: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    /// How long it took, in seconds, as the person waited for it.
    pub seconds: f64,
    pub width: u32,
    pub height: u32,
    /// The size the backend was actually asked for, when it differs from the file's own. A hosted
    /// service that only accepts three sizes, or a placeholder capped for speed, is recorded here
    /// rather than being quietly resized.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sent: String,
    /// RFC 3339, in local time, because that is what a person reads.
    pub created: String,
    /// Where the picture came from, when it came from another picture: "variation of <name>"
    /// or "upscale of <name>". Empty for a first generation.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub made_from: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cfg: Option<f64>,
    /// Written by this app, so a reader knows the shape it is looking at.
    #[serde(default = "default_made_by")]
    pub made_by: String,
}

fn default_made_by() -> String {
    "yantrik-studio".to_string()
}

impl Sidecar {
    /// The path of the sidecar that belongs beside an image.
    pub fn path_for(image: &Path) -> PathBuf {
        image.with_extension("json")
    }

    /// Write beside the image, atomically. A sidecar is the record of how a picture was made and
    /// a half-written one is worse than none, because it parses as nothing and the picture then
    /// looks like it was not made here.
    pub fn write(&self, image: &Path) -> Result<PathBuf, String> {
        let path = Self::path_for(image);
        let body = serde_json::to_vec_pretty(self)
            .map_err(|e| format!("the sidecar could not be written ({e})"))?;
        crate::safe_write::replace(&path, &body, 0o644)?;
        Ok(path)
    }

    /// Read a sidecar back. A missing or unreadable one is `None`, not an error: a picture in the
    /// gallery without a sidecar is still a picture, and the listing says which ones lack one.
    pub fn read(image: &Path) -> Option<Sidecar> {
        let text = std::fs::read_to_string(Self::path_for(image)).ok()?;
        match serde_json::from_str::<Sidecar>(&text) {
            Ok(sidecar) => Some(sidecar),
            Err(e) => {
                tracing::warn!("{} is not a sidecar this version can read ({e})", image.display());
                None
            }
        }
    }
}

/// One row of the gallery: the file, what its sidecar says, and the thumbnail to show for it.
#[derive(Clone, Debug)]
pub struct Record {
    pub path: PathBuf,
    pub name: String,
    pub prompt: String,
    pub seed: u64,
    pub backend: String,
    pub model: String,
    pub seconds: f64,
    pub width: u32,
    pub height: u32,
    pub created: String,
    pub made_from: String,
    /// Whether a sidecar was found. `false` means the row below is what the filesystem could
    /// say, and the surface says so rather than presenting a guess as a record.
    pub has_sidecar: bool,
    /// Decoded pixels for the grid, as RGB. Built on the worker thread; Slint's own image type is
    /// not `Send`, so raw bytes are what crosses back to the UI.
    pub thumbnail: Option<Thumbnail>,
}

#[derive(Clone, Debug)]
pub struct Thumbnail {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
}

impl Record {
    /// The state fragment for one row. Keys that would say nothing are left out, so the JSON a
    /// caller reads is the JSON that means something.
    pub fn json(&self) -> serde_json::Value {
        let mut row = serde_json::Map::new();
        row.insert("path".into(), serde_json::json!(self.path.display().to_string()));
        if !self.prompt.is_empty() {
            row.insert("prompt".into(), serde_json::json!(self.prompt));
        }
        if self.has_sidecar {
            row.insert("seed".into(), serde_json::json!(self.seed));
            row.insert("backend".into(), serde_json::json!(self.backend));
            row.insert("seconds".into(), serde_json::json!((self.seconds * 10.0).round() / 10.0));
            row.insert("size".into(), serde_json::json!(format!("{}x{}", self.width, self.height)));
            if !self.model.is_empty() {
                row.insert("model".into(), serde_json::json!(self.model));
            }
            if !self.made_from.is_empty() {
                row.insert("made_from".into(), serde_json::json!(self.made_from));
            }
        } else {
            // The honest version of a row with no sidecar. A caller that saw a seed here would
            // believe it, and there is nothing to believe.
            row.insert("sidecar".into(), serde_json::json!("missing"));
        }
        row.insert("created".into(), serde_json::json!(self.created));
        serde_json::Value::Object(row)
    }
}

/// Where the pictures go: `~/Pictures/Studio`, or the folder the configuration names.
pub fn output_root(configured: &str) -> PathBuf {
    let base = match configured.trim() {
        "" => pictures_dir(),
        path => expand_home(path),
    };
    // A configured folder is used as given; the default gets Studio's own subfolder, so this app
    // does not write its output loose into a folder a person already has pictures in.
    if configured.trim().is_empty() {
        base.join("Studio")
    } else {
        base
    }
}

fn pictures_dir() -> PathBuf {
    match std::env::var_os("XDG_PICTURES_DIR") {
        Some(value) if !value.is_empty() => PathBuf::from(value),
        _ => home().join("Pictures"),
    }
}

pub fn home() -> PathBuf {
    match std::env::var_os("HOME") {
        Some(value) if !value.is_empty() => PathBuf::from(value),
        _ => PathBuf::from("/tmp"),
    }
}

pub fn expand_home(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => home().join(rest),
        None => PathBuf::from(path),
    }
}

/// Today's folder, `~/Pictures/Studio/2026-09-22`. A date in the path rather than in the filename
/// alone, because a folder a person can open in Files and browse by day is worth more than a flat
/// directory of a thousand images.
pub fn day_folder(root: &Path, when: DateTime<Local>) -> PathBuf {
    root.join(when.format("%Y-%m-%d").to_string())
}

/// A filename that does not exist yet, in `folder`. The time and a slice of the seed make it
/// readable and very nearly unique; the loop makes it actually unique, because two generations in
/// the same second with the same seed is exactly what a mind asking for four of a thing does.
pub fn unique_file(folder: &Path, when: DateTime<Local>, seed: u64) -> PathBuf {
    let stem = format!("{}-{:05}", when.format("%H%M%S"), seed % 100_000);
    let mut candidate = folder.join(format!("{stem}.png"));
    let mut attempt = 2;
    while candidate.exists() || Sidecar::path_for(&candidate).exists() {
        candidate = folder.join(format!("{stem}-{attempt}.png"));
        attempt += 1;
        if attempt > 999 {
            // Nothing sane left to try; a nanosecond suffix cannot collide with a filename a
            // person chose.
            return folder.join(format!(
                "{stem}-{}.png",
                when.timestamp_subsec_nanos()
            ));
        }
    }
    candidate
}

/// The gallery, newest first, up to `limit` rows. Reads a sidecar only for the rows it returns.
pub fn listing(root: &Path, limit: usize, thumbnails: bool) -> Vec<Record> {
    let Ok(days) = std::fs::read_dir(root) else { return Vec::new() };
    // Date folders sort lexicographically in date order, so walking them backwards is walking
    // time backwards without reading a single file's metadata to find out.
    let mut folders: Vec<PathBuf> = days
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    folders.sort();
    folders.reverse();

    let mut found: Vec<PathBuf> = Vec::new();
    let mut examined = 0;
    for folder in folders {
        let Ok(entries) = std::fs::read_dir(&folder) else { continue };
        let mut files: Vec<PathBuf> = entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| {
                path.is_file()
                    && path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(yantrik_image_core::is_image)
            })
            .collect();
        files.sort();
        files.reverse();
        for file in files {
            if examined >= SCAN_LIMIT || found.len() >= limit {
                break;
            }
            examined += 1;
            found.push(file);
        }
        if examined >= SCAN_LIMIT || found.len() >= limit {
            break;
        }
    }

    let mut records: Vec<Record> = found.iter().map(|path| record(path, thumbnails)).collect();
    // Within a day the filenames already run in time order, but a picture copied in from
    // somewhere else has its own, so the gallery is sorted by what the file says about itself.
    //
    // The tie-break runs backwards on purpose. `created` is precise to a second, and a batch is
    // several pictures inside one second, so without a tie-break the order of a batch would depend
    // on the order `read_dir` happened to return; with an ascending one it would depend on the
    // seed. Either way a re-read of the folder could disagree with the engine, which puts each new
    // shot at the front as it lands — and then `gallery.newest` would name a different picture as
    // the newest depending on whether the folder had been read again since. Filenames start with
    // the time they were written, so descending on the path is descending on the moment, and a
    // collision suffix (`-2`, `-3`) sorts later, which is also true of when it was made.
    records.sort_by(|a, b| b.created.cmp(&a.created).then_with(|| b.path.cmp(&a.path)));
    records.truncate(limit);
    records
}

/// One row, from the file and its sidecar. Never fails: a picture with no sidecar is still a
/// picture, and the row says which of the two it is.
pub fn record(path: &Path, thumbnail: bool) -> Record {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    let modified = std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|stamp| stamp.duration_since(UNIX_EPOCH).ok())
        // `from_timestamp` answers in UTC, and the dates here are read by a person and filed under
        // a folder named for their own day, so the one that is displayed is the local one.
        .and_then(|elapsed| DateTime::from_timestamp(elapsed.as_secs() as i64, 0))
        .map(|stamp| stamp.with_timezone(&Local))
        .unwrap_or_else(Local::now);
    let created = modified.to_rfc3339_opts(SecondsFormat::Secs, false);

    let (width, height) = match Sidecar::read(path) {
        Some(sidecar) => {
            return Record {
                prompt: sidecar.prompt,
                seed: sidecar.seed,
                backend: sidecar.backend,
                model: sidecar.model,
                seconds: sidecar.seconds,
                width: sidecar.width,
                height: sidecar.height,
                created: if sidecar.created.is_empty() { created } else { sidecar.created },
                made_from: sidecar.made_from,
                has_sidecar: true,
                path: path.to_path_buf(),
                name,
                thumbnail: if thumbnail { read_thumbnail(path) } else { None },
            }
        }
        None => dimensions(path),
    };

    Record {
        path: path.to_path_buf(),
        name,
        prompt: String::new(),
        seed: 0,
        backend: String::new(),
        model: String::new(),
        seconds: 0.0,
        width: width.unwrap_or(0),
        height: height.unwrap_or(0),
        created,
        made_from: String::new(),
        has_sidecar: false,
        thumbnail: if thumbnail { read_thumbnail(path) } else { None },
    }
}

/// The pixel size of a file, without decoding the whole of it.
pub fn dimensions(path: &Path) -> (Option<u32>, Option<u32>) {
    let Ok(reader) = image::ImageReader::open(path) else { return (None, None) };
    // The two Results in this chain carry different errors, one IO and one the decoder's, and
    // neither is something a caller of this can act on, so both collapse to "unknown".
    match reader.with_guessed_format().ok().and_then(|reader| reader.into_dimensions().ok()) {
        Some((width, height)) => (Some(width), Some(height)),
        None => (None, None),
    }
}

/// The pixel size of encoded image bytes, for a picture that has not been written yet.
pub fn png_size(bytes: &[u8]) -> (Option<u32>, Option<u32>) {
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes));
    match reader.with_guessed_format().ok().and_then(|reader| reader.into_dimensions().ok()) {
        Some((width, height)) => (Some(width), Some(height)),
        None => (None, None),
    }
}

/// Decode and shrink one picture. Done on a worker thread, because decoding twelve JPEGs is not
/// something to do on the thread that owns the window.
pub fn read_thumbnail(path: &Path) -> Option<Thumbnail> {
    let image = image::open(path).ok()?;
    let resized = image.thumbnail(THUMBNAIL_EDGE, THUMBNAIL_EDGE).to_rgb8();
    let (width, height) = resized.dimensions();
    Some(Thumbnail { width, height, rgb: resized.into_raw() })
}

/// The longest edge `upscale` will produce. Large enough to be useful on a 4K screen, small
/// enough that a resample stays inside a second and the result stays a file a person wants.
pub const LONGEST_EDGE: u32 = 8192;

/// Decode, resize by a factor, and hand back the encoded PNG. This is what `upscale` does, and it
/// is a resample rather than a model pass — the action's purpose says so plainly, because a
/// button labelled "upscale" that quietly runs a diffusion model and one that resamples are not
/// the same promise.
pub fn resample(path: &Path, factor: u32, longest_edge: u32) -> Result<Vec<u8>, String> {
    let image = image::open(path)
        .map_err(|e| format!("{} could not be opened as a picture ({e})", path.display()))?;
    let (width, height) = (image.width(), image.height());
    let factor = factor.clamp(1, 8);
    let mut target = (
        width.saturating_mul(factor).max(width),
        height.saturating_mul(factor).max(height),
    );
    let longest = target.0.max(target.1);
    if longest > longest_edge {
        let scale = longest_edge as f64 / longest as f64;
        target = (
            ((target.0 as f64) * scale).round().max(1.0) as u32,
            ((target.1 as f64) * scale).round().max(1.0) as u32,
        );
    }
    if target == (width, height) {
        return Err(format!(
            "{} is already {width}x{height}, which is as large as resampling it by {factor} can make it within {longest_edge} pixels",
            path.display()
        ));
    }
    let resized = image::imageops::resize(
        &image,
        target.0,
        target.1,
        // Lanczos3 is the slowest of the filters and the one that keeps an edge intact at 2x. A
        // resample of a picture this size takes well under a second, so there is no reason to
        // pick a cheaper one.
        image::imageops::FilterType::Lanczos3,
    );
    let mut bytes = Vec::new();
    resized
        .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
        .map_err(|e| format!("the resized picture could not be encoded ({e})"))?;
    Ok(bytes)
}

/// Whether a path is inside Studio's own output tree. `delete` and `upscale` are checked against
/// this: an app that will move any file on the machine to the Trash because a caller named it is
/// a different thing from an app that manages its own gallery, and only the second is what was
/// asked for.
pub fn is_inside(path: &Path, root: &Path) -> bool {
    let Ok(path) = std::fs::canonicalize(path) else { return false };
    let Ok(root) = std::fs::canonicalize(root) else { return false };
    path.starts_with(&root)
}


/// The folder's own answer to "did anything change", from two directory timestamps. A stat costs
/// microseconds, which is what makes it cheap enough to ask every few seconds; walking a gallery
/// and decoding thumbnails is not.
///
/// What this does not notice is a file being overwritten under the same name, which moves no
/// directory timestamp. Nothing this app writes does that (every shot gets a fresh name), and
/// `refresh` is there for the rest.
pub fn cheap_stamp(root: &Path) -> String {
    let today = day_folder(root, Local::now());
    format!("{}|{}", modified(root), modified(&today))
}

fn modified(path: &Path) -> u128 {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0)
}

/// A fingerprint of a listing. Whether the record is there belongs in it: a picture whose sidecar
/// was deleted has not moved and its timestamps have not changed, so a stamp of path and date
/// alone says "nothing changed" and the gallery goes on quoting a record that is gone.
pub fn content_stamp(rows: &[Record]) -> String {
    rows.iter()
        .map(|row| {
            let record = if row.has_sidecar { "" } else { "#no-record" };
            format!("{}@{}{record}", row.path.display(), row.created)
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// The gallery as a worker read it, with what it was read from.
#[derive(Clone, Debug)]
pub struct Listed {
    pub stamp: String,
    pub content: String,
    pub rows: Vec<Record>,
}

/// Read the whole gallery, thumbnails included. Stamped before the listing rather than after: a
/// file that arrives while this is reading then leaves the stamp older than the folder, and the
/// next poll reads again; stamped afterwards, that file would be missed until something else moved.
pub fn read_all(root: &Path) -> Listed {
    let stamp = cheap_stamp(root);
    let rows = listing(root, LISTING_CAP, true);
    let content = content_stamp(&rows);
    Listed { stamp, content, rows }
}

#[cfg(test)]
#[path = "gallery_tests.rs"]
mod tests;
