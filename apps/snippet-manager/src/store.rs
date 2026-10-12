//! Where a snippet lives between runs.
//!
//! Until today there was no store at all: `on_snip_save` threw its arguments away with
//! `let _ = (code, tags)`, the model was never updated, and the next selection painted the
//! editor from that stale model — so an edit reverted while you watched, and a restart left
//! an empty window. Everything below exists so that what is on screen and what is on disk
//! cannot disagree.
//!
//! # One file, not one file per snippet
//!
//! Notes keeps one Markdown file per note plus a `.meta` sidecar, and that is right for notes:
//! a note *is* its file. A person opens it in a text editor, greps the vault, syncs the folder,
//! and the id is the filename. A snippet is not that. It is a short fragment plus structured
//! metadata — language, tags, favourite, collection, counters — and every view this app draws
//! (the list, the search box, the tag chips, the counts) needs all of it in one read. Split
//! across files it would need notes' sidecar per snippet and a directory walk per keystroke.
//!
//! So: one versioned JSON file, pretty-printed, written temp-then-rename, the way Download
//! Manager keeps its list. The cost of the choice is that one unreadable file is all of them,
//! which is exactly why a file that cannot be parsed is renamed aside and reported rather than
//! overwritten — the bytes are the only record of what the person had, and this build not being
//! able to read them is not a reason to destroy them.
//!
//! Nothing here knows about Slint. It is plain data and file IO so it can be tested without a
//! desktop — see `tests/snippets-core`.

use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// The schema this build writes and reads. A file from another version is kept aside rather
/// than guessed at.
pub const STATE_VERSION: u32 = 1;

/// The two collections the sidebar always shows. They are filters, not containers: nothing is
/// stored under them, so they are not written to the file and cannot be renamed or deleted.
pub const ALL: i32 = 0;
pub const FAVORITES: i32 = 1;
/// Collections a person makes start here, so a stored id can never be mistaken for a filter.
pub const FIRST_CUSTOM: i32 = 2;

/// Limits, so one enormous paste cannot make the store unloadable.
pub const MAX_CODE: usize = 256 * 1024;
pub const MAX_TITLE: usize = 200;
pub const MAX_TAGS: usize = 400;
pub const MAX_SNIPPETS: usize = 5000;
pub const MAX_FILE: usize = 64 * 1024 * 1024;

/// One kept fragment.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Snippet {
    pub id: i32,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub code: String,
    /// Comma-separated, because the screen's one tags field holds them that way and inventing
    /// a second shape here would mean converting on every read and write.
    #[serde(default)]
    pub tags: String,
    #[serde(default)]
    pub favorite: bool,
    /// [`ALL`] means "in no collection of its own".
    #[serde(default)]
    pub collection: i32,
    #[serde(default)]
    pub created: u64,
    #[serde(default)]
    pub updated: u64,
    /// Unix seconds of the last copy, or 0 for never.
    #[serde(default)]
    pub used: u64,
    #[serde(default)]
    pub use_count: i32,
}

impl Snippet {
    /// The one line the list shows under the title.
    pub fn preview(&self) -> String {
        let line = self
            .code
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .trim_end();
        line.chars().take(120).collect()
    }

    /// The tags, split and trimmed. Empty entries are dropped so `a,,b` is two tags.
    pub fn tag_list(&self) -> Vec<String> {
        self.tags
            .split(',')
            .map(|t| t.trim())
            .filter(|t| !t.is_empty())
            .map(|t| t.to_string())
            .collect()
    }

    fn matches(&self, needle: &str) -> bool {
        self.title.to_lowercase().contains(needle)
            || self.code.to_lowercase().contains(needle)
            || self.tags.to_lowercase().contains(needle)
            || self.language.to_lowercase().contains(needle)
    }
}

/// A collection a person made. The two built-in filters are not in here.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Collection {
    pub id: i32,
    #[serde(default)]
    pub name: String,
}

/// The fields a save may change. Every one is optional, because the editor sends four of them
/// and the control surface may send any subset, and both go through the same path.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Patch {
    pub title: Option<String>,
    pub language: Option<String>,
    pub code: Option<String>,
    pub tags: Option<String>,
    pub favorite: Option<bool>,
    pub collection: Option<i32>,
}

impl Patch {
    /// What the editor sends: the four fields it holds.
    pub fn edits(title: &str, language: &str, code: &str, tags: &str) -> Self {
        Self {
            title: Some(title.to_string()),
            language: Some(language.to_string()),
            code: Some(code.to_string()),
            tags: Some(tags.to_string()),
            ..Self::default()
        }
    }

    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// The file, as it is written.
#[derive(Debug, Serialize, Deserialize)]
struct StoredState {
    version: u32,
    #[serde(default)]
    next_id: i32,
    #[serde(default)]
    next_collection: i32,
    #[serde(default)]
    collections: Vec<Collection>,
    #[serde(default)]
    snippets: Vec<Snippet>,
}

/// What an export writes and an import reads. Deliberately a subset of [`StoredState`] with the
/// same field name, so the state file itself can be handed to `import` and works.
#[derive(Debug, Serialize, Deserialize)]
struct Bundle {
    version: u32,
    #[serde(default)]
    exported_at: u64,
    #[serde(default)]
    snippets: Vec<Snippet>,
}

/// What an import turned out to be, so the caller can say which it was.
#[derive(Clone, Debug, PartialEq)]
pub struct Imported {
    pub ids: Vec<i32>,
    /// `bundle` or `file` — an exported set, or a single source file read as one snippet.
    pub kind: &'static str,
}

/// Unix seconds now. Zero if the clock is before the epoch, which is not a reason to fail a save.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Where the snippets live.
///
/// `~/.local/share/yantrik/snippets`, beside every other app's store, overridable with
/// `YANTRIK_SNIPPETS_DIR` the way Notes takes `YANTRIK_NOTES_DIR` and Downloads takes
/// `YANTRIK_DOWNLOADS_DIR` — a conformance probe needs somewhere to work that is not the
/// person's real collection.
pub fn dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("YANTRIK_SNIPPETS_DIR") {
        return PathBuf::from(dir);
    }
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    Path::new(&home).join(".local/share/yantrik/snippets")
}

/// The state file inside a store directory.
pub fn state_path(dir: &Path) -> PathBuf {
    dir.join("snippets.json")
}

/// The language a file extension implies, or `Other`.
///
/// Used by the command line and by `import`: a file named on the command line arrives with no
/// language, and guessing from the extension is better than filing every one of them as `Other`.
pub fn language_for(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "rs" => "Rust",
        "py" | "pyw" => "Python",
        "js" | "mjs" | "cjs" | "jsx" => "JavaScript",
        "ts" | "tsx" => "TypeScript",
        "go" => "Go",
        "c" => "C",
        "cc" | "cpp" | "cxx" | "h" | "hpp" => "C++",
        "sh" | "bash" | "zsh" => "Shell",
        "sql" => "SQL",
        "html" | "htm" => "HTML",
        "css" | "scss" => "CSS",
        "toml" => "TOML",
        "yaml" | "yml" => "YAML",
        "json" => "JSON",
        _ => "Other",
    }
}

/// "just now", "12 minutes ago", "3 days ago", and a plain date once that stops being useful.
pub fn relative(then: u64, now_secs: u64) -> String {
    if then == 0 {
        return String::new();
    }
    let delta = now_secs.saturating_sub(then);
    match delta {
        0..=59 => "just now".to_string(),
        60..=3599 => format!("{} minutes ago", delta / 60),
        3600..=86399 => format!("{} hours ago", delta / 3600),
        86400..=604_799 => format!("{} days ago", delta / 86400),
        _ => date(then),
    }
}

/// `YYYY-MM-DD` from unix seconds, without pulling a date library into a module that is
/// otherwise std and serde only.
///
/// The civil-from-days arithmetic is Howard Hinnant's, shifting the epoch to 1 March so the leap
/// day lands at the end of the cycle and no month table is needed.
pub fn date(secs: u64) -> String {
    let days = (secs / 86400) as i64;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

/// Everything the app holds, and the only thing that writes the file.
pub struct Store {
    path: PathBuf,
    snippets: Vec<Snippet>,
    collections: Vec<Collection>,
    next_id: i32,
    next_collection: i32,
    notice: String,
}

impl Store {
    /// Read the store, or start empty and say why.
    ///
    /// Never fails: a snippet window that refuses to open is worse than one that has forgotten,
    /// and every way this can go wrong leaves a working app with a notice on it.
    pub fn load(path: PathBuf) -> Self {
        let mut store = Self {
            path,
            snippets: Vec::new(),
            collections: Vec::new(),
            next_id: 1,
            next_collection: FIRST_CUSTOM,
            notice: String::new(),
        };

        let raw = match read_capped(&store.path, MAX_FILE) {
            // Nothing saved yet is the ordinary first run, not a fault worth a notice.
            Ok(None) => return store,
            Ok(Some(raw)) => raw,
            Err(e) => {
                store.notice = format!("Could not read {}: {e}", store.path.display());
                return store;
            }
        };

        match decode(&raw) {
            Ok(state) => {
                store.snippets = state.snippets;
                store.collections = state.collections;
                store.next_id = state.next_id;
                store.next_collection = state.next_collection;
            }
            Err(reason) => store.keep_aside(&reason),
        }
        store
    }

    /// The store as it is on disk now, for a read that must change nothing: `None` when there is
    /// nothing readable, rather than [`Store::load`]'s setting a broken file aside. The approval
    /// card's name for a snippet is read this way (`main.rs`, `delete`).
    pub fn peek(path: PathBuf) -> Option<Self> {
        let raw = read_capped(&path, MAX_FILE).ok()??;
        let state = decode(&raw).ok()?;
        Some(Self {
            path,
            snippets: state.snippets,
            collections: state.collections,
            next_id: state.next_id,
            next_collection: state.next_collection,
            notice: String::new(),
        })
    }

    /// Open the store in its usual place.
    pub fn open() -> Self {
        Self::load(state_path(&dir()))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// What load had to say, if anything. Empty is the normal case.
    pub fn notice(&self) -> &str {
        &self.notice
    }

    pub fn snippets(&self) -> &[Snippet] {
        &self.snippets
    }

    pub fn collections(&self) -> &[Collection] {
        &self.collections
    }

    pub fn get(&self, id: i32) -> Option<&Snippet> {
        self.snippets.iter().find(|s| s.id == id)
    }

    pub fn favorites(&self) -> usize {
        self.snippets.iter().filter(|s| s.favorite).count()
    }

    pub fn collection_name(&self, id: i32) -> String {
        match id {
            ALL => "All Snippets".to_string(),
            FAVORITES => "Favorites".to_string(),
            other => self
                .collections
                .iter()
                .find(|c| c.id == other)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| "All Snippets".to_string()),
        }
    }

    /// A snippet named the way a person or a mind would name one: by id, or by title.
    ///
    /// An unambiguous substring counts, because "copy the docker compose one" is how this gets
    /// asked for; an ambiguous one is an error naming the count rather than a guess.
    pub fn find(&self, needle: &str) -> Result<&Snippet, String> {
        let needle = needle.trim();
        if needle.is_empty() {
            return Err("no snippet was named".into());
        }
        if let Ok(id) = needle.parse::<i32>() {
            if let Some(found) = self.get(id) {
                return Ok(found);
            }
        }
        let lower = needle.to_lowercase();
        if let Some(exact) = self
            .snippets
            .iter()
            .find(|s| s.title.to_lowercase() == lower)
        {
            return Ok(exact);
        }
        let partial: Vec<&Snippet> = self
            .snippets
            .iter()
            .filter(|s| s.title.to_lowercase().contains(&lower))
            .collect();
        match partial.len() {
            1 => Ok(partial[0]),
            0 => Err(format!("no snippet with id or title `{needle}`")),
            n => Err(format!(
                "`{needle}` matches {n} snippets; name one exactly or give its id"
            )),
        }
    }

    /// The rows a view shows, newest edit first.
    ///
    /// `collection` is [`ALL`], [`FAVORITES`] or a stored collection id; `tag` and `query` are
    /// both optional and both case-insensitive. All of it is a scan over a vector held in
    /// memory, which at this app's scale costs nothing and keeps searching honest — there is no
    /// index to fall out of step with the snippets.
    pub fn matching(&self, query: &str, tag: &str, collection: i32) -> Vec<Snippet> {
        let needle = query.trim().to_lowercase();
        let tag = tag.trim().to_lowercase();
        let mut rows: Vec<Snippet> = self
            .snippets
            .iter()
            .filter(|s| match collection {
                ALL => true,
                FAVORITES => s.favorite,
                other => s.collection == other,
            })
            .filter(|s| {
                tag.is_empty() || s.tag_list().iter().any(|t| t.to_lowercase() == tag)
            })
            .filter(|s| needle.is_empty() || s.matches(&needle))
            .cloned()
            .collect();
        rows.sort_by(|a, b| b.updated.cmp(&a.updated).then(b.id.cmp(&a.id)));
        rows
    }

    /// Every tag in use, once each, in the order a person would scan them.
    pub fn tags(&self) -> Vec<String> {
        let mut seen: Vec<String> = Vec::new();
        for snippet in &self.snippets {
            for tag in snippet.tag_list() {
                if !seen.iter().any(|t| t.to_lowercase() == tag.to_lowercase()) {
                    seen.push(tag);
                }
            }
        }
        seen.sort_by_key(|t| t.to_lowercase());
        seen
    }

    // ── The mutations ───────────────────────────────────────────────
    //
    // Every one of these is the only path to the change it makes: the button and the control
    // surface both come through here. Each writes the file and, if the write fails, puts memory
    // back exactly as it was and returns the reason — so the window and the disk cannot end up
    // telling different stories, which is the fault this whole module exists to close.

    /// Keep a new snippet. Answers with the id it was stored under.
    pub fn create(
        &mut self,
        title: &str,
        language: &str,
        code: &str,
        tags: &str,
        collection: i32,
    ) -> Result<i32, String> {
        if self.snippets.len() >= MAX_SNIPPETS {
            return Err(format!("this store already holds {MAX_SNIPPETS} snippets"));
        }
        let stamp = now();
        let snippet = Snippet {
            id: self.next_id,
            title: clean_title(title),
            language: language.trim().to_string(),
            code: code.to_string(),
            tags: clean_tags(tags),
            favorite: false,
            collection: self.known_collection(collection),
            created: stamp,
            updated: stamp,
            used: 0,
            use_count: 0,
        };
        validate(&snippet)?;

        let id = snippet.id;
        self.snippets.push(snippet);
        self.next_id += 1;
        if let Err(e) = self.write() {
            self.snippets.retain(|s| s.id != id);
            self.next_id -= 1;
            return Err(e);
        }
        Ok(id)
    }

    /// Change a snippet. Answers with the snippet as it is now stored, re-read from the store.
    pub fn save(&mut self, id: i32, patch: Patch) -> Result<Snippet, String> {
        let index = self
            .snippets
            .iter()
            .position(|s| s.id == id)
            .ok_or_else(|| format!("no snippet with id {id}"))?;
        let previous = self.snippets[index].clone();

        let mut next = previous.clone();
        if let Some(title) = patch.title {
            next.title = clean_title(&title);
        }
        if let Some(language) = patch.language {
            next.language = language.trim().to_string();
        }
        if let Some(code) = patch.code {
            next.code = code;
        }
        if let Some(tags) = patch.tags {
            next.tags = clean_tags(&tags);
        }
        if let Some(favorite) = patch.favorite {
            next.favorite = favorite;
        }
        if let Some(collection) = patch.collection {
            next.collection = self.known_collection(collection);
        }
        validate(&next)?;

        // A save that changes nothing does not move the clock: an editor that commits on every
        // selection would otherwise reshuffle the list for people who only looked.
        if next == previous {
            return Ok(previous);
        }
        next.updated = now();

        self.snippets[index] = next;
        if let Err(e) = self.write() {
            self.snippets[index] = previous;
            return Err(e);
        }
        Ok(self.snippets[index].clone())
    }

    /// Remove a snippet. Answers with what was removed, so the caller can say what it was.
    pub fn delete(&mut self, id: i32) -> Result<Snippet, String> {
        let index = self
            .snippets
            .iter()
            .position(|s| s.id == id)
            .ok_or_else(|| format!("no snippet with id {id}"))?;
        let removed = self.snippets.remove(index);
        if let Err(e) = self.write() {
            self.snippets.insert(index, removed);
            return Err(e);
        }
        Ok(removed)
    }

    /// Star or unstar. Answers with what it is now.
    pub fn toggle_favorite(&mut self, id: i32) -> Result<bool, String> {
        let current = self
            .get(id)
            .ok_or_else(|| format!("no snippet with id {id}"))?
            .favorite;
        let saved = self.save(
            id,
            Patch {
                favorite: Some(!current),
                ..Patch::default()
            },
        )?;
        Ok(saved.favorite)
    }

    /// Record that a snippet was used. Called after the clipboard actually took it.
    pub fn mark_used(&mut self, id: i32) -> Result<Snippet, String> {
        let index = self
            .snippets
            .iter()
            .position(|s| s.id == id)
            .ok_or_else(|| format!("no snippet with id {id}"))?;
        let previous = self.snippets[index].clone();
        self.snippets[index].used = now();
        self.snippets[index].use_count = previous.use_count.saturating_add(1);
        if let Err(e) = self.write() {
            self.snippets[index] = previous;
            return Err(e);
        }
        Ok(self.snippets[index].clone())
    }

    /// Make a collection. Answers with its id.
    pub fn collection_create(&mut self, name: &str) -> Result<i32, String> {
        let name = clean_collection_name(name)?;
        if self
            .collections
            .iter()
            .any(|c| c.name.to_lowercase() == name.to_lowercase())
        {
            return Err(format!("a collection called `{name}` already exists"));
        }
        let id = self.next_collection;
        self.collections.push(Collection { id, name });
        self.next_collection += 1;
        if let Err(e) = self.write() {
            self.collections.retain(|c| c.id != id);
            self.next_collection -= 1;
            return Err(e);
        }
        Ok(id)
    }

    pub fn collection_rename(&mut self, id: i32, name: &str) -> Result<String, String> {
        if id < FIRST_CUSTOM {
            return Err("the built-in collections cannot be renamed".into());
        }
        let name = clean_collection_name(name)?;
        let index = self
            .collections
            .iter()
            .position(|c| c.id == id)
            .ok_or_else(|| format!("no collection with id {id}"))?;
        if self
            .collections
            .iter()
            .any(|c| c.id != id && c.name.to_lowercase() == name.to_lowercase())
        {
            return Err(format!("a collection called `{name}` already exists"));
        }
        let previous = self.collections[index].name.clone();
        self.collections[index].name = name.clone();
        if let Err(e) = self.write() {
            self.collections[index].name = previous;
            return Err(e);
        }
        Ok(name)
    }

    /// Remove a collection. Answers with how many snippets came out of it.
    ///
    /// Its snippets are moved back to All rather than deleted: a folder is not the thing inside
    /// it, and taking somebody's code away because they tidied up a list would be the worst
    /// surprise in this app.
    pub fn collection_delete(&mut self, id: i32) -> Result<usize, String> {
        if id < FIRST_CUSTOM {
            return Err("the built-in collections cannot be deleted".into());
        }
        let index = self
            .collections
            .iter()
            .position(|c| c.id == id)
            .ok_or_else(|| format!("no collection with id {id}"))?;
        let removed = self.collections.remove(index);
        let moved: Vec<usize> = self
            .snippets
            .iter()
            .enumerate()
            .filter(|(_, s)| s.collection == id)
            .map(|(i, _)| i)
            .collect();
        for i in &moved {
            self.snippets[*i].collection = ALL;
        }
        if let Err(e) = self.write() {
            for i in &moved {
                self.snippets[*i].collection = id;
            }
            self.collections.insert(index, removed);
            return Err(e);
        }
        Ok(moved.len())
    }

    /// Write every snippet to one file a person or another program can read.
    pub fn export_all(&self, path: &Path) -> Result<usize, String> {
        let bundle = Bundle {
            version: STATE_VERSION,
            exported_at: now(),
            snippets: self.snippets.clone(),
        };
        let body = serde_json::to_string_pretty(&bundle).map_err(|e| e.to_string())?;
        write_atomically(path, &body)?;
        Ok(self.snippets.len())
    }

    /// Take a file in: an exported set, or a single source file kept as one snippet.
    ///
    /// The two are told apart by reading it, not by its name: anything that parses as a bundle
    /// of this version is one, and everything else is code.
    pub fn import_file(&mut self, path: &Path, collection: i32) -> Result<Imported, String> {
        let raw = read_capped(path, MAX_FILE)?
            .ok_or_else(|| format!("no file at {}", path.display()))?;

        if let Ok(bundle) = serde_json::from_str::<Bundle>(&raw) {
            if bundle.version == STATE_VERSION && !bundle.snippets.is_empty() {
                return self.import_bundle(bundle, collection);
            }
        }

        let title = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Untitled Snippet".into());
        let id = self.create(&title, language_for(path), &raw, "", collection)?;
        Ok(Imported {
            ids: vec![id],
            kind: "file",
        })
    }

    /// Every snippet in a bundle, under fresh ids, in one write.
    ///
    /// Fresh ids because an import is a copy: an exported file carrying id 4 must not land on
    /// top of the id 4 that is already here. One write because a bundle half imported is worse
    /// than one not imported at all, so a failure rolls all of them back.
    fn import_bundle(&mut self, bundle: Bundle, collection: i32) -> Result<Imported, String> {
        if self.snippets.len() + bundle.snippets.len() > MAX_SNIPPETS {
            return Err(format!(
                "importing {} would take this store past {MAX_SNIPPETS} snippets",
                bundle.snippets.len()
            ));
        }
        let stamp = now();
        let first_id = self.next_id;
        let before = self.snippets.len();
        let mut ids = Vec::new();
        for incoming in bundle.snippets {
            let snippet = Snippet {
                id: self.next_id,
                title: clean_title(&incoming.title),
                language: incoming.language.trim().to_string(),
                code: incoming.code,
                tags: clean_tags(&incoming.tags),
                favorite: incoming.favorite,
                collection: self.known_collection(collection),
                created: if incoming.created > 0 { incoming.created } else { stamp },
                updated: stamp,
                used: 0,
                use_count: 0,
            };
            if let Err(e) = validate(&snippet) {
                self.snippets.truncate(before);
                self.next_id = first_id;
                return Err(e);
            }
            ids.push(snippet.id);
            self.snippets.push(snippet);
            self.next_id += 1;
        }
        if let Err(e) = self.write() {
            self.snippets.truncate(before);
            self.next_id = first_id;
            return Err(e);
        }
        Ok(Imported {
            ids,
            kind: "bundle",
        })
    }

    /// A collection id this store actually has, or [`ALL`].
    ///
    /// [`FAVORITES`] is a filter and not a container, so a snippet cannot be filed under it;
    /// starring is what puts a snippet there.
    fn known_collection(&self, id: i32) -> i32 {
        if id >= FIRST_CUSTOM && self.collections.iter().any(|c| c.id == id) {
            id
        } else {
            ALL
        }
    }

    /// The file, pretty-printed.
    ///
    /// Pretty on purpose, as Downloads is: this is a file somebody may have to read by hand on
    /// the day the app will not start, and the whitespace costs a few hundred bytes.
    fn write(&self) -> Result<(), String> {
        let state = StoredState {
            version: STATE_VERSION,
            next_id: self.next_id,
            next_collection: self.next_collection,
            collections: self.collections.clone(),
            snippets: self.snippets.clone(),
        };
        let body = serde_json::to_string_pretty(&state).map_err(|e| e.to_string())?;
        write_atomically(&self.path, &body)
    }

    /// Move an unreadable file out of the way and say so.
    ///
    /// Kept, never deleted: it is the only record of what the person had, and a mind or a human
    /// can read it even when this build cannot. The app then starts empty, which is the honest
    /// state — it really does not know what was there.
    fn keep_aside(&mut self, reason: &str) {
        let name = self
            .path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "snippets.json".into());
        let kept = self
            .path
            .with_file_name(format!("{name}.corrupt-{}", now()));
        match std::fs::rename(&self.path, &kept) {
            Ok(()) => {
                self.notice = format!(
                    "The saved snippets could not be read ({reason}). They have been kept as {} \
                     and this window is starting empty.",
                    kept.display()
                )
            }
            // The rename failing is worse than the parse failing: the next save would write over
            // the only copy there is. Say exactly that, rather than reporting a rescue that did
            // not happen.
            Err(e) => {
                self.notice = format!(
                    "The saved snippets could not be read ({reason}) and could not be moved aside \
                     ({e}). {} will be overwritten by the next change.",
                    self.path.display()
                )
            }
        }
    }
}

// ── Plain functions ─────────────────────────────────────────────────

/// Parse a state file, or say why it cannot be used at all.
fn decode(raw: &str) -> Result<StoredState, String> {
    let mut state: StoredState =
        serde_json::from_str(raw).map_err(|e| format!("it is not valid snippet JSON: {e}"))?;
    if state.version != STATE_VERSION {
        return Err(format!(
            "it is version {}, and this build reads version {STATE_VERSION}",
            state.version
        ));
    }
    // One unusable row is not a reason to lose the other nineteen, and an id that repeats would
    // make `save` ambiguous, so a duplicate is dropped rather than guessed at.
    let mut seen: Vec<i32> = Vec::new();
    state.snippets.retain(|s| {
        let keep = s.id > 0 && !seen.contains(&s.id) && validate(s).is_ok();
        if keep {
            seen.push(s.id);
        }
        keep
    });
    state
        .collections
        .retain(|c| c.id >= FIRST_CUSTOM && !c.name.trim().is_empty());

    let highest = state.snippets.iter().map(|s| s.id).max().unwrap_or(0);
    state.next_id = state.next_id.max(highest + 1).max(1);
    let highest_collection = state.collections.iter().map(|c| c.id).max().unwrap_or(0);
    state.next_collection = state
        .next_collection
        .max(highest_collection + 1)
        .max(FIRST_CUSTOM);
    Ok(state)
}

fn validate(snippet: &Snippet) -> Result<(), String> {
    if snippet.code.len() > MAX_CODE {
        return Err(format!(
            "a snippet can hold up to {} KiB of code",
            MAX_CODE / 1024
        ));
    }
    if snippet.title.chars().count() > MAX_TITLE {
        return Err(format!("a title can be up to {MAX_TITLE} characters"));
    }
    if snippet.tags.chars().count() > MAX_TAGS {
        return Err(format!("tags can be up to {MAX_TAGS} characters"));
    }
    if snippet.title.contains('\0') || snippet.code.contains('\0') {
        return Err("a snippet must be text, and this one contains a null byte".into());
    }
    Ok(())
}

/// A title always says something: an empty one becomes the same words the New button uses.
fn clean_title(title: &str) -> String {
    let title = title.replace(['\r', '\n'], " ");
    let title = title.trim();
    if title.is_empty() {
        "Untitled Snippet".to_string()
    } else {
        title.chars().take(MAX_TITLE).collect()
    }
}

/// Tags as the screen shows them: comma separated, trimmed, no empties, no repeats.
fn clean_tags(tags: &str) -> String {
    let mut kept: Vec<String> = Vec::new();
    for tag in tags.replace(['\r', '\n'], " ").split(',') {
        let tag = tag.trim();
        if tag.is_empty() || kept.iter().any(|t| t.to_lowercase() == tag.to_lowercase()) {
            continue;
        }
        kept.push(tag.to_string());
    }
    kept.join(", ").chars().take(MAX_TAGS).collect()
}

fn clean_collection_name(name: &str) -> Result<String, String> {
    let name: String = name
        .replace(['\r', '\n'], " ")
        .trim()
        .chars()
        .take(80)
        .collect();
    if name.is_empty() {
        return Err("a collection needs a name".into());
    }
    Ok(name)
}

/// Read a file, refusing one too large to be a snippet store.
fn read_capped(path: &Path, limit: usize) -> Result<Option<String>, String> {
    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() {
        return Err(format!("{} is not a regular file", path.display()));
    }
    if metadata.len() > limit as u64 {
        return Err(format!("{} is larger than {} MiB", path.display(), limit / 1024 / 1024));
    }
    let mut text = String::new();
    file.read_to_string(&mut text)
        .map_err(|e| format!("{} is not UTF-8 text: {e}", path.display()))?;
    Ok(Some(text))
}

/// Replace a file in one step, or leave the old one exactly as it was.
///
/// Writing over the live file means a power cut half way through leaves an unreadable store,
/// which is precisely the file the corrupt path then has to rescue. The temp name carries the
/// pid so two processes cannot scribble over each other's half-written copy, and it sits in the
/// same directory so the rename stays on one filesystem — a rename across devices is a copy, and
/// a copy is not atomic. The temp file is removed on every failing path, because a directory
/// full of half-written stores is its own kind of mess.
pub fn write_atomically(path: &Path, body: &str) -> Result<(), String> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "snippets.json".into());
    let temp = dir.join(format!("{name}.tmp-{}", std::process::id()));

    let written = (|| -> std::io::Result<()> {
        let mut file = File::create(&temp)?;
        file.write_all(body.as_bytes())?;
        // The rename is only atomic with respect to a file whose bytes have landed.
        file.sync_all()
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("{}: {e}", temp.display()));
    }
    if let Err(e) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("{}: {e}", path.display()));
    }
    Ok(())
}
