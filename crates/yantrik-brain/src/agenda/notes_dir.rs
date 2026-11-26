//! Reads the Notes library for the agenda: a directory of `.md` files. Read-only; a file is
//! never written, followed through a link or kept. Only a title, a count and the words of the
//! unticked items leave this module (see `senses::NoteInfo`).

use std::fs;
use std::path::Path;

use super::senses::NoteInfo;

const MAX_DEPTH: usize = 3;
const MAX_VISITED: usize = 2000;
const MAX_BYTES: u64 = 512 * 1024;
const MAX_ITEMS: usize = 200;

/// The recently edited notes under `dir` with at least one unticked item. `None` when the
/// directory cannot be read at all (so the caller leaves the agenda alone).
pub fn read_notes(dir: &Path, now: f64, recent_secs: f64) -> Option<Vec<NoteInfo>> {
    fs::read_dir(dir).ok()?;
    let mut out = Vec::new();
    let mut visited = 0usize;
    walk(dir, dir, 0, now, recent_secs, &mut visited, &mut out);
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Some(out)
}

fn walk(root: &Path, dir: &Path, depth: usize, now: f64, recent: f64, visited: &mut usize, out: &mut Vec<NoteInfo>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        *visited += 1;
        if *visited > MAX_VISITED {
            return;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            if depth < MAX_DEPTH {
                walk(root, &path, depth + 1, now, recent, visited, out);
            }
        } else if ft.is_file() && name.ends_with(".md") {
            if let Some(n) = read_one(root, &path, now, recent) {
                out.push(n);
            }
        }
    }
}

fn read_one(root: &Path, path: &Path, now: f64, recent: f64) -> Option<NoteInfo> {
    let meta = fs::metadata(path).ok()?;
    if meta.len() > MAX_BYTES {
        return None;
    }
    let edited = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs_f64();
    if edited < now - recent {
        return None;
    }
    let text = fs::read_to_string(path).ok()?;
    let (title, items) = parse(&text, path.file_stem()?.to_string_lossy().as_ref());
    if items.is_empty() {
        return None;
    }
    let id = path.strip_prefix(root).ok()?.to_string_lossy().replace('\\', "/");
    Some(NoteInfo::new(&id, &title, edited, &items))
}

/// The title (first `# ` heading, else the file name) and the texts of the unticked items.
pub fn parse(text: &str, stem: &str) -> (String, Vec<String>) {
    let mut title = None;
    let mut items = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if title.is_none() {
            if let Some(h) = t.strip_prefix("# ") {
                title = Some(h.trim().to_string());
            }
        }
        for bullet in ["- [ ]", "* [ ]", "+ [ ]"] {
            if let Some(rest) = t.strip_prefix(bullet) {
                if items.len() < MAX_ITEMS {
                    items.push(rest.trim().to_string());
                }
            }
        }
    }
    (title.filter(|t| !t.is_empty()).unwrap_or_else(|| stem.to_string()), items)
}
