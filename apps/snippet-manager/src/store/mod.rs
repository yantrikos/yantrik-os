//! What Snippets keeps: the snippets and the collections, as plain data, and every change that
//! can be made to them.
//!
//! Until September there was no store at all: `on_snip_save` threw its arguments away with
//! `let _ = (code, tags)`, the model was never updated, and the next selection painted the editor
//! from that stale model — so an edit reverted while you watched, and a restart left an empty
//! window. Everything below exists so that what is on screen and what is on disk cannot disagree.
//!
//! # One file, not one file per snippet
//!
//! Notes keeps one Markdown file per note, and that is right for notes: a note *is* its file. A
//! snippet is not that. It is a short fragment plus structured metadata — language, tags,
//! favourite, collection, counters — and every view this app draws (the list, the search box, the
//! tag chips, the counts) needs all of it in one read. So the library is one value here and one
//! file on disk (`file.rs`), and it is not a document: nothing is opened, edited in place and
//! saved; every change is a small whole change to the library, written as the whole file.
//!
//! # Changes here, writes elsewhere
//!
//! Every method that changes the library changes memory only and says what it did. Writing the
//! file is the app's write lane (`lane.rs`), off the UI thread; when a write does not land the
//! library is read back from the file, so the window and the disk still tell one story.
//!
//! Nothing here knows about Slint or the disk. It is plain data so it can be tested without a
//! desktop — see `tests.rs`.

use serde::{Deserialize, Serialize};

pub mod file;
mod rules;
pub mod text;
#[cfg(test)]
mod tests;

pub use rules::{MAX_CODE, MAX_SNIPPETS, MAX_TAGS, MAX_TITLE};
use rules::{clean_collection_name, clean_tags, clean_title, validate};

/// The two collections the sidebar always shows. They are filters, not containers: nothing is
/// stored under them, so they are not written to the file and cannot be renamed or deleted.
pub const ALL: i32 = 0;
pub const FAVORITES: i32 = 1;
/// Collections a person makes start here, so a stored id can never be mistaken for a filter.
pub const FIRST_CUSTOM: i32 = 2;

/// One kept fragment.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
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

/// A snippet may hold a password or a key, so its code is never printed, not even by a test's
/// failure message: only how long it is.
impl std::fmt::Debug for Snippet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Snippet")
            .field("id", &self.id)
            .field("title", &self.title)
            .field("language", &self.language)
            .field("code", &format_args!("<{} bytes>", self.code.len()))
            .field("tags", &self.tags)
            .field("favorite", &self.favorite)
            .field("collection", &self.collection)
            .field("use_count", &self.use_count)
            .finish()
    }
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
#[derive(Clone, Default, PartialEq)]
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

/// What an import turned out to be, so the caller can say which it was.
#[derive(Clone, Debug, PartialEq)]
pub struct Imported {
    pub ids: Vec<i32>,
    /// `bundle` or `file` — an exported set, or a single source file read as one snippet.
    pub kind: &'static str,
}

/// Everything the app keeps.
#[derive(Clone, Debug, PartialEq)]
pub struct Library {
    snippets: Vec<Snippet>,
    collections: Vec<Collection>,
    next_id: i32,
    next_collection: i32,
}

impl Default for Library {
    fn default() -> Library {
        Library {
            snippets: Vec::new(),
            collections: Vec::new(),
            next_id: 1,
            next_collection: FIRST_CUSTOM,
        }
    }
}

impl Library {
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
        if let Some(exact) = self.snippets.iter().find(|s| s.title.to_lowercase() == lower) {
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
            n => Err(format!("`{needle}` matches {n} snippets; name one exactly or give its id")),
        }
    }

    /// The rows a view shows, newest edit first.
    ///
    /// `collection` is [`ALL`], [`FAVORITES`] or a stored collection id; `tag` and `query` are
    /// both optional and both case-insensitive. All of it is a scan over a vector held in memory,
    /// which at this app's scale costs nothing and keeps searching honest — there is no index to
    /// fall out of step with the snippets.
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
            .filter(|s| tag.is_empty() || s.tag_list().iter().any(|t| t.to_lowercase() == tag))
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

    /// Whether saving `patch` over snippet `id` would change what is kept: the editor holds
    /// typing the store has not taken yet. False for a snippet that is not here.
    pub fn differs(&self, id: i32, patch: &Patch) -> bool {
        self.get(id).is_some_and(|s| self.patched(s, patch.clone()) != *s)
    }

    // ── The changes ─────────────────────────────────────────────────
    //
    // Every one of these is the only path to the change it makes: the button and the control
    // surface both come through here. Each validates before it changes anything, so a refusal
    // leaves the library exactly as it was.

    /// Keep a new snippet. Answers with the id it was given.
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
        let stamp = text::now();
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
        Ok(id)
    }

    /// Change a snippet. Answers with the snippet as it now is.
    pub fn save(&mut self, id: i32, patch: Patch) -> Result<Snippet, String> {
        let index = self.index_of(id)?;
        let previous = self.snippets[index].clone();
        let mut next = self.patched(&previous, patch);
        validate(&next)?;
        // A save that changes nothing does not move the clock: an editor that commits on every
        // selection would otherwise reshuffle the list for people who only looked.
        if next == previous {
            return Ok(previous);
        }
        next.updated = text::now();
        self.snippets[index] = next;
        Ok(self.snippets[index].clone())
    }

    /// Remove a snippet. Answers with what was removed, so the caller can say what it was.
    pub fn delete(&mut self, id: i32) -> Result<Snippet, String> {
        let index = self.index_of(id)?;
        Ok(self.snippets.remove(index))
    }

    /// Star or unstar. Answers with what it is now.
    pub fn toggle_favorite(&mut self, id: i32) -> Result<bool, String> {
        let current = self.get(id).ok_or_else(|| format!("no snippet with id {id}"))?.favorite;
        let saved = self.save(id, Patch { favorite: Some(!current), ..Patch::default() })?;
        Ok(saved.favorite)
    }

    /// Record that a snippet was used. Called after the clipboard actually took it.
    pub fn mark_used(&mut self, id: i32) -> Result<Snippet, String> {
        let index = self.index_of(id)?;
        let snippet = &mut self.snippets[index];
        snippet.used = text::now();
        snippet.use_count = snippet.use_count.saturating_add(1);
        Ok(snippet.clone())
    }

    /// Make a collection. Answers with its id.
    pub fn collection_create(&mut self, name: &str) -> Result<i32, String> {
        let name = clean_collection_name(name)?;
        if self.collections.iter().any(|c| c.name.to_lowercase() == name.to_lowercase()) {
            return Err(format!("a collection called `{name}` already exists"));
        }
        let id = self.next_collection;
        self.collections.push(Collection { id, name });
        self.next_collection += 1;
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
        self.collections[index].name = name.clone();
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
        self.collections.remove(index);
        let mut moved = 0;
        for snippet in self.snippets.iter_mut().filter(|s| s.collection == id) {
            snippet.collection = ALL;
            moved += 1;
        }
        Ok(moved)
    }

    /// Take in what an import read: a source file as one snippet, or every snippet of a bundle.
    pub fn import(&mut self, incoming: file::Incoming, collection: i32) -> Result<Imported, String> {
        match incoming {
            file::Incoming::File { title, language, text } => {
                let id = self.create(&title, language, &text, "", collection)?;
                Ok(Imported { ids: vec![id], kind: "file" })
            }
            file::Incoming::Bundle(snippets) => self.import_bundle(snippets, collection),
        }
    }

    /// Every snippet in a bundle, under fresh ids, all or none.
    ///
    /// Fresh ids because an import is a copy: an exported file carrying id 4 must not land on top
    /// of the id 4 that is already here. All or none because a bundle half imported is worse than
    /// one not imported at all, so a row that will not validate refuses the whole bundle.
    fn import_bundle(&mut self, bundle: Vec<Snippet>, collection: i32) -> Result<Imported, String> {
        if self.snippets.len() + bundle.len() > MAX_SNIPPETS {
            return Err(format!(
                "importing {} would take this store past {MAX_SNIPPETS} snippets",
                bundle.len()
            ));
        }
        let stamp = text::now();
        let collection = self.known_collection(collection);
        let mut incoming = Vec::new();
        for (n, row) in bundle.into_iter().enumerate() {
            let snippet = Snippet {
                id: self.next_id + n as i32,
                title: clean_title(&row.title),
                language: row.language.trim().to_string(),
                code: row.code,
                tags: clean_tags(&row.tags),
                favorite: row.favorite,
                collection,
                created: if row.created > 0 { row.created } else { stamp },
                updated: stamp,
                used: 0,
                use_count: 0,
            };
            validate(&snippet)?;
            incoming.push(snippet);
        }
        let ids: Vec<i32> = incoming.iter().map(|s| s.id).collect();
        self.next_id += ids.len() as i32;
        self.snippets.extend(incoming);
        Ok(Imported { ids, kind: "bundle" })
    }

    /// Never give an id or a collection id `other` has already given: a library read back after a
    /// refused write takes the counters of the one it replaces when they are higher, so an id a
    /// refused create used, and a draft may be named by, is never given to another snippet. The
    /// next write keeps the higher counters on disk (#938 review, M3).
    pub fn keep_counters_from(&mut self, other: &Library) {
        self.next_id = self.next_id.max(other.next_id);
        self.next_collection = self.next_collection.max(other.next_collection);
    }

    /// Never give `id` again: a draft from an earlier run names it.
    pub fn reserve(&mut self, id: i32) {
        self.next_id = self.next_id.max(id.saturating_add(1));
    }

    fn index_of(&self, id: i32) -> Result<usize, String> {
        self.snippets
            .iter()
            .position(|s| s.id == id)
            .ok_or_else(|| format!("no snippet with id {id}"))
    }

    /// `snippet` with `patch` laid over it and tidied, as a save would keep it.
    fn patched(&self, snippet: &Snippet, patch: Patch) -> Snippet {
        let mut next = snippet.clone();
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
        next
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
}
