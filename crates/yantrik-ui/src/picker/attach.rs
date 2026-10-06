//! Files handed over with the next message: chosen in the picker's browser or dropped on the
//! window, kept until the message is sent, and sent to the mind as `options.attachments` with
//! their provenance — the person handed them over, here, now — which the mind's egress planner
//! reads as a valid source, as it reads the person's own words.
//!
//! A file is read once, when it is added: its size, its SHA-256, and for a small one its content
//! (1 MiB each, 4 MiB a message) so a mind running as an account of its own, which cannot open a
//! path in the person's home, still receives it.

use std::io::Read;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use sha2::{Digest, Sha256};
use yantrik_harness::protocol::Attachment;

/// The most of a file carried inline.
pub const INLINE_EACH: u64 = 1024 * 1024;
/// The most carried inline in one message.
pub const INLINE_TOTAL: u64 = 4 * 1024 * 1024;
/// The most files one message hands over.
pub const MAX_FILES: usize = 10;
/// The largest file that may be handed over at all (its path and hash; no content above 1 MiB).
pub const MAX_SIZE: u64 = 512 * 1024 * 1024;

/// "12 KB", "3.4 MB".
pub fn size_words(n: u64) -> String {
    match n {
        0..=1023 => format!("{n} B"),
        1024..=1_048_575 => format!("{} KB", n / 1024),
        _ => format!("{:.1} MB", n as f64 / 1_048_576.0),
    }
}

/// A best guess at the file's type from its name, for the mind; never relied on.
fn mime(path: &Path) -> String {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "txt" | "log" => "text/plain",
        "md" => "text/markdown",
        "json" => "application/json",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "rs" | "py" | "js" | "ts" | "sh" | "toml" | "yaml" | "yml" | "c" | "h" | "go" => "text/plain",
        _ => "application/octet-stream",
    }
    .to_string()
}

/// Read a file into an attachment: a regular file (a link is followed to what it names, which is
/// what the person pointed at), at most [`MAX_SIZE`]. `inline_left` is how much of this message's
/// inline allowance is unspent; it is spent here.
pub fn read(path: &Path, via: &str, at: &str, inline_left: &mut u64) -> Result<Attachment, String> {
    let real = std::fs::canonicalize(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let meta = std::fs::metadata(&real).map_err(|e| format!("{}: {e}", path.display()))?;
    if !meta.is_file() {
        return Err(format!("{} is not a file", path.display()));
    }
    if meta.len() > MAX_SIZE {
        return Err(format!("{} is larger than {}", path.display(), size_words(MAX_SIZE)));
    }
    let mut f = std::fs::File::open(&real).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let keep = meta.len() <= INLINE_EACH && meta.len() <= *inline_left;
    let mut kept = Vec::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf).map_err(|e| format!("{}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        if keep {
            kept.extend_from_slice(&buf[..n]);
        }
    }
    if keep {
        *inline_left -= kept.len() as u64;
    }
    Ok(Attachment {
        name: real.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        path: real.display().to_string(),
        mime: mime(&real),
        size: meta.len(),
        sha256: hasher.finalize().iter().map(|b| format!("{b:02x}")).collect(),
        handed_over_by: "person".into(),
        via: via.into(),
        at: at.into(),
        content_b64: if keep { base64::engine::general_purpose::STANDARD.encode(&kept) } else { String::new() },
    })
}

/// One entry of the attach browser.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub folder: bool,
    pub size: u64,
}

/// A folder's entries for the browser: folders first, then files, by name; hidden ones left out.
pub fn list(dir: &Path) -> Result<Vec<Entry>, String> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let Ok(meta) = std::fs::metadata(e.path()) else { continue };
        out.push(Entry { path: e.path(), name, folder: meta.is_dir(), size: meta.len() });
        if out.len() >= 500 {
            break;
        }
    }
    out.sort_by(|a, b| b.folder.cmp(&a.folder).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(out)
}

/// Where the browser goes for a path it was asked to show: `…/..` is the folder above, and a path
/// that is not a folder is the person's home.
pub fn resolve_dir(asked: &str, home: &Path) -> PathBuf {
    let p = if asked.trim().is_empty() { home.to_path_buf() } else { PathBuf::from(asked) };
    let p = if p.ends_with("..") { p.parent().and_then(Path::parent).map(Path::to_path_buf).unwrap_or_else(|| home.to_path_buf()) } else { p };
    match std::fs::canonicalize(&p) {
        Ok(real) if real.is_dir() => real,
        _ => home.to_path_buf(),
    }
}

/// A path dropped on the window: winit gives a path; some sources give `file://` URIs, one a line.
pub fn dropped_paths(text: &str) -> Vec<PathBuf> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.strip_prefix("file://").unwrap_or(l))
        .map(|l| PathBuf::from(percent_decode(l)))
        .filter(|p| p.is_absolute())
        .collect()
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
