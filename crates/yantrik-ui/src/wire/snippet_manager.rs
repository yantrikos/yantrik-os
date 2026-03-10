//! Code Snippet Manager wire module — screen 25.
//!
//! SQLite-backed snippet storage at `~/.config/yantrik/snippets.db`.
//! Full-text search across title, code, tags, language. Tag filter chips.
//! Collections/folders, favorites, import/export, metadata tracking.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use rusqlite::Connection;
use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};

use crate::app_context::AppContext;
use crate::{App, CollectionData, LanguageOption, SnippetData, TagChip};

/// A snippet row from the database.
#[derive(Clone, Debug)]
struct Snippet {
    id: i64,
    title: String,
    language: String,
    code: String,
    tags: String,
    is_favorite: bool,
    collection_id: i64,
    created_at: f64,
    updated_at: f64,
    last_used_at: f64,
    use_count: i64,
}

/// A collection row from the database.
#[derive(Clone, Debug)]
struct Collection {
    id: i64,
    name: String,
    is_builtin: bool,
}

/// Built-in collection IDs (negative to avoid collision with user collections).
const COLL_ALL: i64 = -1;
const COLL_FAVORITES: i64 = -2;
const COLL_RECENT: i64 = -3;

/// Supported languages for the dropdown.
const LANGUAGES: &[&str] = &[
    "Rust", "Python", "JavaScript", "TypeScript", "Go", "C", "C++", "Java",
    "C#", "Ruby", "PHP", "Swift", "Kotlin", "Scala", "Shell", "Bash",
    "PowerShell", "SQL", "HTML", "CSS", "YAML", "TOML", "JSON", "XML",
    "Markdown", "Lua", "R", "Perl", "Haskell", "Elixir", "Zig", "Nim",
    "Dart", "Dockerfile", "Makefile", "GLSL", "WGSL",
];

/// Get the snippets database path.
fn db_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    PathBuf::from(home).join(".config/yantrik/snippets.db")
}

/// Open (or create) the snippets database and ensure schema exists.
fn open_db() -> Result<Connection, rusqlite::Error> {
    let path = db_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let conn = Connection::open(&path)?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS snippets (
            id INTEGER PRIMARY KEY,
            title TEXT NOT NULL DEFAULT '',
            language TEXT NOT NULL DEFAULT '',
            code TEXT NOT NULL DEFAULT '',
            tags TEXT NOT NULL DEFAULT '',
            created_at REAL NOT NULL,
            updated_at REAL NOT NULL
        );

        CREATE TABLE IF NOT EXISTS collections (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL DEFAULT '',
            is_builtin INTEGER NOT NULL DEFAULT 0
        );",
    )?;

    // Add new columns if missing (migration-safe).
    let has_favorite = conn
        .prepare("SELECT is_favorite FROM snippets LIMIT 0")
        .is_ok();
    if !has_favorite {
        let _ = conn.execute_batch(
            "ALTER TABLE snippets ADD COLUMN is_favorite INTEGER NOT NULL DEFAULT 0;",
        );
    }

    let has_collection_id = conn
        .prepare("SELECT collection_id FROM snippets LIMIT 0")
        .is_ok();
    if !has_collection_id {
        let _ = conn.execute_batch(
            "ALTER TABLE snippets ADD COLUMN collection_id INTEGER NOT NULL DEFAULT 0;",
        );
    }

    let has_last_used = conn
        .prepare("SELECT last_used_at FROM snippets LIMIT 0")
        .is_ok();
    if !has_last_used {
        let _ = conn.execute_batch(
            "ALTER TABLE snippets ADD COLUMN last_used_at REAL NOT NULL DEFAULT 0;
             ALTER TABLE snippets ADD COLUMN use_count INTEGER NOT NULL DEFAULT 0;",
        );
    }

    // Ensure use_count exists even if last_used_at existed but use_count didn't
    let has_use_count = conn
        .prepare("SELECT use_count FROM snippets LIMIT 0")
        .is_ok();
    if !has_use_count {
        let _ = conn.execute_batch(
            "ALTER TABLE snippets ADD COLUMN use_count INTEGER NOT NULL DEFAULT 0;",
        );
    }

    Ok(conn)
}

/// Current unix timestamp as f64.
fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

/// Load all snippets from the database.
fn load_all(conn: &Connection) -> Vec<Snippet> {
    let mut stmt = match conn.prepare(
        "SELECT id, title, language, code, tags, created_at, updated_at,
                COALESCE(is_favorite, 0), COALESCE(collection_id, 0),
                COALESCE(last_used_at, 0), COALESCE(use_count, 0)
         FROM snippets ORDER BY updated_at DESC",
    ) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "Failed to prepare snippet query");
            return Vec::new();
        }
    };

    let rows = stmt.query_map([], |row| {
        Ok(Snippet {
            id: row.get(0)?,
            title: row.get(1)?,
            language: row.get(2)?,
            code: row.get(3)?,
            tags: row.get(4)?,
            created_at: row.get(5)?,
            updated_at: row.get(6)?,
            is_favorite: row.get::<_, i64>(7)? != 0,
            collection_id: row.get(8)?,
            last_used_at: row.get(9)?,
            use_count: row.get(10)?,
        })
    });

    match rows {
        Ok(iter) => iter.filter_map(|r| r.ok()).collect(),
        Err(e) => {
            tracing::error!(error = %e, "Failed to query snippets");
            Vec::new()
        }
    }
}

/// Load all user-created collections from the database.
fn load_collections(conn: &Connection) -> Vec<Collection> {
    let mut stmt = match conn.prepare(
        "SELECT id, name, is_builtin FROM collections ORDER BY name ASC",
    ) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "Failed to prepare collections query");
            return Vec::new();
        }
    };

    let rows = stmt.query_map([], |row| {
        Ok(Collection {
            id: row.get(0)?,
            name: row.get(1)?,
            is_builtin: row.get::<_, i64>(2)? != 0,
        })
    });

    match rows {
        Ok(iter) => iter.filter_map(|r| r.ok()).collect(),
        Err(e) => {
            tracing::error!(error = %e, "Failed to query collections");
            Vec::new()
        }
    }
}

/// Format a unix timestamp as a date string like "Mar 6, 2026".
fn format_date(ts: f64) -> String {
    if ts <= 0.0 {
        return String::new();
    }
    let secs = ts as u64;
    let days_since_epoch = secs / 86400;
    let (y, m, d) = civil_from_days(days_since_epoch as i64);
    let month_name = match m {
        1 => "Jan",
        2 => "Feb",
        3 => "Mar",
        4 => "Apr",
        5 => "May",
        6 => "Jun",
        7 => "Jul",
        8 => "Aug",
        9 => "Sep",
        10 => "Oct",
        11 => "Nov",
        12 => "Dec",
        _ => "???",
    };
    format!("{} {}, {}", month_name, d, y)
}

/// Convert days since Unix epoch to (year, month, day).
/// Algorithm from Howard Hinnant's date library.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// Convert a Snippet to the Slint SnippetData struct.
fn snippet_to_model(s: &Snippet, selected_id: i64) -> SnippetData {
    let preview = s
        .code
        .lines()
        .next()
        .unwrap_or("")
        .chars()
        .take(80)
        .collect::<String>();

    SnippetData {
        id: s.id as i32,
        title: SharedString::from(&s.title),
        language: SharedString::from(&s.language),
        code: SharedString::from(&s.code),
        tags: SharedString::from(&s.tags),
        preview: SharedString::from(&preview),
        date_text: SharedString::from(&format_date(s.updated_at)),
        is_selected: s.id == selected_id,
        is_favorite: s.is_favorite,
        created_date: SharedString::from(&format_date(s.created_at)),
        last_used_date: SharedString::from(&format_date(s.last_used_at)),
        use_count: s.use_count as i32,
        collection_id: s.collection_id as i32,
    }
}

/// Extract unique tags from all snippets.
fn extract_tags(snippets: &[Snippet]) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    for s in snippets {
        for tag in s.tags.split(',') {
            let t = tag.trim().to_string();
            if !t.is_empty() {
                seen.insert(t);
            }
        }
    }
    seen.into_iter().collect()
}

/// Filter snippets by search query (title, code, tags, language).
fn filter_snippets(snippets: &[Snippet], query: &str) -> Vec<Snippet> {
    let lower = query.to_lowercase();
    snippets
        .iter()
        .filter(|s| {
            s.title.to_lowercase().contains(&lower)
                || s.code.to_lowercase().contains(&lower)
                || s.tags.to_lowercase().contains(&lower)
                || s.language.to_lowercase().contains(&lower)
        })
        .cloned()
        .collect()
}

/// Filter snippets by tag.
fn filter_by_tag(snippets: &[Snippet], tag: &str) -> Vec<Snippet> {
    let lower = tag.to_lowercase();
    snippets
        .iter()
        .filter(|s| {
            s.tags
                .split(',')
                .any(|t| t.trim().to_lowercase() == lower)
        })
        .cloned()
        .collect()
}

/// Filter snippets by collection.
fn filter_by_collection(snippets: &[Snippet], collection_id: i64) -> Vec<Snippet> {
    match collection_id {
        COLL_ALL => snippets.to_vec(),
        COLL_FAVORITES => snippets.iter().filter(|s| s.is_favorite).cloned().collect(),
        COLL_RECENT => {
            let mut recent: Vec<Snippet> = snippets
                .iter()
                .filter(|s| s.last_used_at > 0.0)
                .cloned()
                .collect();
            recent.sort_by(|a, b| b.last_used_at.partial_cmp(&a.last_used_at).unwrap());
            recent.truncate(20);
            recent
        }
        cid => snippets
            .iter()
            .filter(|s| s.collection_id == cid)
            .cloned()
            .collect(),
    }
}

/// Build the collections model for the sidebar.
fn build_collections_model(
    db_collections: &[Collection],
    snippets: &[Snippet],
    selected_coll: i64,
) -> Vec<CollectionData> {
    let mut result = Vec::new();

    // Built-in: All Snippets
    result.push(CollectionData {
        id: COLL_ALL as i32,
        name: "All Snippets".into(),
        snippet_count: snippets.len() as i32,
        is_builtin: true,
        is_selected: selected_coll == COLL_ALL,
        icon: "\u{1F4C4}".into(), // page
    });

    // Built-in: Favorites
    let fav_count = snippets.iter().filter(|s| s.is_favorite).count();
    result.push(CollectionData {
        id: COLL_FAVORITES as i32,
        name: "Favorites".into(),
        snippet_count: fav_count as i32,
        is_builtin: true,
        is_selected: selected_coll == COLL_FAVORITES,
        icon: "\u{2605}".into(), // star
    });

    // Built-in: Recent
    let recent_count = snippets.iter().filter(|s| s.last_used_at > 0.0).count().min(20);
    result.push(CollectionData {
        id: COLL_RECENT as i32,
        name: "Recent".into(),
        snippet_count: recent_count as i32,
        is_builtin: true,
        is_selected: selected_coll == COLL_RECENT,
        icon: "\u{1F552}".into(), // clock
    });

    // User-created collections
    for c in db_collections {
        let count = snippets
            .iter()
            .filter(|s| s.collection_id == c.id)
            .count();
        result.push(CollectionData {
            id: c.id as i32,
            name: SharedString::from(&c.name),
            snippet_count: count as i32,
            is_builtin: false,
            is_selected: selected_coll == c.id,
            icon: "\u{1F4C1}".into(), // folder
        });
    }

    result
}

/// Build the language options list.
fn build_language_options() -> Vec<LanguageOption> {
    LANGUAGES
        .iter()
        .map(|name| LanguageOption {
            name: SharedString::from(*name),
        })
        .collect()
}

/// A version entry: (timestamp_label, code_content).
#[derive(Clone, Debug)]
struct VersionEntry {
    label: String,
    code: String,
}

/// State shared between closures.
struct SnippetState {
    all_snippets: Vec<Snippet>,
    db_collections: Vec<Collection>,
    selected_id: i64,
    active_tag: String,
    active_collection: i64,
    search_query: String,
    /// Per-snippet version history: snippet_id -> list of previous versions (newest first).
    version_history: std::collections::HashMap<i64, Vec<VersionEntry>>,
}

/// Refresh the snippet list, tag chips, and collections in the UI.
fn refresh_ui(ui: &App, state: &SnippetState) {
    // Apply collection filter first
    let coll_filtered = filter_by_collection(&state.all_snippets, state.active_collection);

    // Apply tag filter if active
    let tag_filtered = if state.active_tag.is_empty() {
        coll_filtered
    } else {
        filter_by_tag(&coll_filtered, &state.active_tag)
    };

    // Apply search filter if active
    let display = if state.search_query.is_empty() {
        tag_filtered
    } else {
        filter_snippets(&tag_filtered, &state.search_query)
    };

    let items: Vec<SnippetData> = display
        .iter()
        .map(|s| snippet_to_model(s, state.selected_id))
        .collect();
    let count = items.len() as i32;
    ui.set_snip_snippets(ModelRc::new(VecModel::from(items)));
    ui.set_snip_snippet_count(count);

    // Set match count only when searching
    if !state.search_query.is_empty() {
        ui.set_snip_match_count(count);
    } else {
        ui.set_snip_match_count(-1);
    }

    // Tag chips — from collection-filtered set
    let coll_for_tags = filter_by_collection(&state.all_snippets, state.active_collection);
    let all_tags = extract_tags(&coll_for_tags);
    let chips: Vec<TagChip> = all_tags
        .iter()
        .map(|t| TagChip {
            name: SharedString::from(t.as_str()),
            is_active: t.to_lowercase() == state.active_tag.to_lowercase(),
        })
        .collect();
    ui.set_snip_tag_chips(ModelRc::new(VecModel::from(chips)));

    // Collections sidebar
    let colls = build_collections_model(
        &state.db_collections,
        &state.all_snippets,
        state.active_collection,
    );
    ui.set_snip_collections(ModelRc::new(VecModel::from(colls)));
}

/// Update detail panel properties from a snippet.
fn set_detail(ui: &App, s: &Snippet) {
    ui.set_snip_detail_id(s.id as i32);
    ui.set_snip_detail_title(SharedString::from(&s.title));
    ui.set_snip_detail_language(SharedString::from(&s.language));
    ui.set_snip_detail_code(SharedString::from(&s.code));
    ui.set_snip_detail_tags(SharedString::from(&s.tags));
    ui.set_snip_detail_is_favorite(s.is_favorite);
    ui.set_snip_detail_created_date(SharedString::from(&format_date(s.created_at)));
    ui.set_snip_detail_last_used_date(SharedString::from(&format_date(s.last_used_at)));
    ui.set_snip_detail_use_count(s.use_count as i32);
}

/// Clear the detail panel.
fn clear_detail(ui: &App) {
    ui.set_snip_detail_id(-1);
    ui.set_snip_detail_title("".into());
    ui.set_snip_detail_language("".into());
    ui.set_snip_detail_code("".into());
    ui.set_snip_detail_tags("".into());
    ui.set_snip_detail_is_favorite(false);
    ui.set_snip_detail_created_date("".into());
    ui.set_snip_detail_last_used_date("".into());
    ui.set_snip_detail_use_count(0);
}

/// Wire snippet manager callbacks.
pub fn wire(ui: &App, ctx: &AppContext) {
    let db = match open_db() {
        Ok(c) => Rc::new(RefCell::new(c)),
        Err(e) => {
            tracing::error!(error = %e, "Failed to open snippets database");
            return;
        }
    };

    let state: Rc<RefCell<SnippetState>> = Rc::new(RefCell::new(SnippetState {
        all_snippets: Vec::new(),
        db_collections: Vec::new(),
        selected_id: -1,
        active_tag: String::new(),
        active_collection: COLL_ALL,
        search_query: String::new(),
        version_history: std::collections::HashMap::new(),
    }));

    // Set language options (static)
    ui.set_snip_language_options(ModelRc::new(VecModel::from(build_language_options())));

    // Initial load
    {
        let conn = db.borrow();
        let mut s = state.borrow_mut();
        s.all_snippets = load_all(&conn);
        s.db_collections = load_collections(&conn);
    }

    // ── Search ──
    {
        let ui_weak = ui.as_weak();
        let st = state.clone();
        ui.on_snip_search(move |query| {
            let query_str = query.to_string();
            if let Some(ui) = ui_weak.upgrade() {
                let mut s = st.borrow_mut();
                s.search_query = query_str;
                refresh_ui(&ui, &s);
            }
        });
    }

    // ── Select ──
    {
        let ui_weak = ui.as_weak();
        let st = state.clone();
        let db = db.clone();
        ui.on_snip_select(move |id| {
            let id = id as i64;
            if let Some(ui) = ui_weak.upgrade() {
                let mut s = st.borrow_mut();
                s.selected_id = id;
                if let Some(snip) = s.all_snippets.iter_mut().find(|sn| sn.id == id) {
                    // Update use count and last_used
                    snip.use_count += 1;
                    snip.last_used_at = now_ts();
                    let conn = db.borrow();
                    let _ = conn.execute(
                        "UPDATE snippets SET use_count=?1, last_used_at=?2 WHERE id=?3",
                        rusqlite::params![snip.use_count, snip.last_used_at, id],
                    );
                    set_detail(&ui, snip);
                }
                refresh_ui(&ui, &s);
            }
        });
    }

    // ── New snippet ──
    {
        let ui_weak = ui.as_weak();
        let db = db.clone();
        let st = state.clone();
        ui.on_snip_new(move || {
            let ts = now_ts();
            let conn = db.borrow();
            // Assign to current collection if it's a user collection, else 0
            let coll_id = {
                let s = st.borrow();
                if s.active_collection > 0 {
                    s.active_collection
                } else {
                    0
                }
            };
            match conn.execute(
                "INSERT INTO snippets (title, language, code, tags, created_at, updated_at, is_favorite, collection_id, last_used_at, use_count)
                 VALUES ('Untitled', '', '', '', ?1, ?2, 0, ?3, 0, 0)",
                rusqlite::params![ts, ts, coll_id],
            ) {
                Ok(_) => {
                    let new_id = conn.last_insert_rowid();
                    let mut s = st.borrow_mut();
                    s.all_snippets = load_all(&conn);
                    s.selected_id = new_id;

                    if let Some(ui) = ui_weak.upgrade() {
                        if let Some(snip) = s.all_snippets.iter().find(|sn| sn.id == new_id) {
                            set_detail(&ui, snip);
                        }
                        refresh_ui(&ui, &s);
                    }
                    tracing::info!(id = new_id, "New snippet created");
                }
                Err(e) => {
                    tracing::error!(error = %e, "Failed to create snippet");
                }
            }
        });
    }

    // ── Save ──
    {
        let ui_weak = ui.as_weak();
        let db = db.clone();
        let st = state.clone();
        ui.on_snip_save(move |id, title, language, code, tags| {
            let ts = now_ts();
            let id64 = id as i64;

            // Store previous version before saving
            {
                let old_info = {
                    let s = st.borrow();
                    s.all_snippets.iter().find(|sn| sn.id == id64).and_then(|old| {
                        if !old.code.is_empty() && old.code != code.to_string() {
                            Some((old.code.clone(), old.updated_at))
                        } else {
                            None
                        }
                    })
                };
                if let Some((old_code, old_updated_at)) = old_info {
                    let mut s = st.borrow_mut();
                    let label = format_date(old_updated_at);
                    let versions = s.version_history.entry(id64).or_insert_with(Vec::new);
                    versions.insert(0, VersionEntry {
                        label,
                        code: old_code,
                    });
                    // Keep at most 20 versions per snippet
                    versions.truncate(20);
                }
            }

            let conn = db.borrow();
            match conn.execute(
                "UPDATE snippets SET title=?1, language=?2, code=?3, tags=?4, updated_at=?5
                 WHERE id=?6",
                rusqlite::params![
                    title.to_string(),
                    language.to_string(),
                    code.to_string(),
                    tags.to_string(),
                    ts,
                    id64
                ],
            ) {
                Ok(_) => {
                    let mut s = st.borrow_mut();
                    s.all_snippets = load_all(&conn);

                    if let Some(ui) = ui_weak.upgrade() {
                        // Update version list UI
                        update_version_list(&ui, &s, id64);
                        refresh_ui(&ui, &s);
                    }
                    tracing::info!(id, "Snippet saved");
                }
                Err(e) => {
                    tracing::error!(error = %e, id, "Failed to save snippet");
                }
            }
        });
    }

    // ── Delete ──
    {
        let ui_weak = ui.as_weak();
        let db = db.clone();
        let st = state.clone();
        ui.on_snip_delete(move |id| {
            let conn = db.borrow();
            match conn.execute("DELETE FROM snippets WHERE id=?1", rusqlite::params![id as i64]) {
                Ok(_) => {
                    let mut s = st.borrow_mut();
                    s.all_snippets = load_all(&conn);
                    s.selected_id = -1;

                    if let Some(ui) = ui_weak.upgrade() {
                        clear_detail(&ui);
                        refresh_ui(&ui, &s);
                    }
                    tracing::info!(id, "Snippet deleted");
                }
                Err(e) => {
                    tracing::error!(error = %e, id, "Failed to delete snippet");
                }
            }
        });
    }

    // ── Copy to clipboard with feedback ──
    {
        let ui_weak = ui.as_weak();
        let st = state.clone();
        ui.on_snip_copy(move |id| {
            let s = st.borrow();
            if let Some(snip) = s.all_snippets.iter().find(|sn| sn.id == id as i64) {
                let code = snip.code.clone();
                copy_to_clipboard(&code, id);

                // Show "Copied!" feedback
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_snip_copy_feedback_visible(true);
                    let ui_weak2 = ui.as_weak();
                    let timer = Timer::default();
                    timer.start(TimerMode::SingleShot, std::time::Duration::from_millis(1500), move || {
                        if let Some(ui) = ui_weak2.upgrade() {
                            ui.set_snip_copy_feedback_visible(false);
                        }
                    });
                    // Keep timer alive by leaking it (it's a one-shot, fine)
                    std::mem::forget(timer);
                }
            }
        });
    }

    // ── Tag filter ──
    {
        let ui_weak = ui.as_weak();
        let st = state.clone();
        ui.on_snip_tag_filter(move |clicked_tag| {
            let clicked = clicked_tag.to_string();
            let mut s = st.borrow_mut();
            // Toggle: if same tag clicked again, clear filter
            if s.active_tag.to_lowercase() == clicked.to_lowercase() {
                s.active_tag = String::new();
            } else {
                s.active_tag = clicked;
            }

            if let Some(ui) = ui_weak.upgrade() {
                refresh_ui(&ui, &s);
            }
        });
    }

    // ── Toggle favorite ──
    {
        let ui_weak = ui.as_weak();
        let db = db.clone();
        let st = state.clone();
        ui.on_snip_toggle_favorite(move |id| {
            let conn = db.borrow();
            let mut s = st.borrow_mut();
            if let Some(snip) = s.all_snippets.iter().find(|sn| sn.id == id as i64) {
                let new_fav = if snip.is_favorite { 0i64 } else { 1i64 };
                let _ = conn.execute(
                    "UPDATE snippets SET is_favorite=?1 WHERE id=?2",
                    rusqlite::params![new_fav, id as i64],
                );
                s.all_snippets = load_all(&conn);
                if let Some(ui) = ui_weak.upgrade() {
                    // Update detail if this snippet is selected
                    if let Some(updated) = s.all_snippets.iter().find(|sn| sn.id == id as i64) {
                        if s.selected_id == id as i64 {
                            ui.set_snip_detail_is_favorite(updated.is_favorite);
                        }
                    }
                    refresh_ui(&ui, &s);
                }
            }
        });
    }

    // ── Collection select ──
    {
        let ui_weak = ui.as_weak();
        let st = state.clone();
        ui.on_snip_collection_select(move |coll_id| {
            let mut s = st.borrow_mut();
            s.active_collection = coll_id as i64;
            s.active_tag = String::new(); // Clear tag filter on collection switch

            if let Some(ui) = ui_weak.upgrade() {
                refresh_ui(&ui, &s);
            }
        });
    }

    // ── Collection create ──
    {
        let ui_weak = ui.as_weak();
        let db = db.clone();
        let st = state.clone();
        ui.on_snip_collection_create(move |name| {
            let conn = db.borrow();
            match conn.execute(
                "INSERT INTO collections (name, is_builtin) VALUES (?1, 0)",
                rusqlite::params![name.to_string()],
            ) {
                Ok(_) => {
                    let new_id = conn.last_insert_rowid();
                    let mut s = st.borrow_mut();
                    s.db_collections = load_collections(&conn);
                    s.active_collection = new_id;

                    if let Some(ui) = ui_weak.upgrade() {
                        refresh_ui(&ui, &s);
                    }
                    tracing::info!(id = new_id, "Collection created");
                }
                Err(e) => {
                    tracing::error!(error = %e, "Failed to create collection");
                }
            }
        });
    }

    // ── Collection rename ──
    {
        let ui_weak = ui.as_weak();
        let db = db.clone();
        let st = state.clone();
        ui.on_snip_collection_rename(move |coll_id, new_name| {
            let conn = db.borrow();
            let _ = conn.execute(
                "UPDATE collections SET name=?1 WHERE id=?2 AND is_builtin=0",
                rusqlite::params![new_name.to_string(), coll_id as i64],
            );
            let mut s = st.borrow_mut();
            s.db_collections = load_collections(&conn);
            if let Some(ui) = ui_weak.upgrade() {
                refresh_ui(&ui, &s);
            }
        });
    }

    // ── Collection delete ──
    {
        let ui_weak = ui.as_weak();
        let db = db.clone();
        let st = state.clone();
        ui.on_snip_collection_delete(move |coll_id| {
            let conn = db.borrow();
            // Move snippets in this collection to "uncategorized" (collection_id=0)
            let _ = conn.execute(
                "UPDATE snippets SET collection_id=0 WHERE collection_id=?1",
                rusqlite::params![coll_id as i64],
            );
            let _ = conn.execute(
                "DELETE FROM collections WHERE id=?1 AND is_builtin=0",
                rusqlite::params![coll_id as i64],
            );
            let mut s = st.borrow_mut();
            s.db_collections = load_collections(&conn);
            s.all_snippets = load_all(&conn);
            s.active_collection = COLL_ALL;

            if let Some(ui) = ui_weak.upgrade() {
                refresh_ui(&ui, &s);
            }
            tracing::info!(id = coll_id, "Collection deleted");
        });
    }

    // ── Set language (from dropdown) ──
    {
        let ui_weak = ui.as_weak();
        ui.on_snip_set_language(move |lang| {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_snip_detail_language(lang);
            }
        });
    }

    // ── Export all snippets ──
    {
        let st = state.clone();
        ui.on_snip_export_all(move || {
            let s = st.borrow();
            let export = export_snippets_json(&s.all_snippets);
            let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
            let export_path = PathBuf::from(&home).join("snippets_export.json");
            match std::fs::write(&export_path, &export) {
                Ok(_) => {
                    tracing::info!(path = %export_path.display(), count = s.all_snippets.len(), "Snippets exported");
                }
                Err(e) => {
                    tracing::error!(error = %e, "Failed to export snippets");
                }
            }
        });
    }

    // ── Import snippets ──
    {
        let ui_weak = ui.as_weak();
        let db = db.clone();
        let st = state.clone();
        ui.on_snip_import(move || {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
            let import_path = PathBuf::from(&home).join("snippets_import.json");
            match std::fs::read_to_string(&import_path) {
                Ok(data) => {
                    let conn = db.borrow();
                    let count = import_snippets_json(&conn, &data);
                    let mut s = st.borrow_mut();
                    s.all_snippets = load_all(&conn);
                    if let Some(ui) = ui_weak.upgrade() {
                        refresh_ui(&ui, &s);
                    }
                    tracing::info!(imported = count, "Snippets imported from {}", import_path.display());
                }
                Err(e) => {
                    tracing::error!(error = %e, path = %import_path.display(), "Failed to read import file");
                }
            }
        });
    }

    // ── AI Explain callback ──
    let bridge = ctx.bridge.clone();
    let ai_state = super::ai_assist::AiAssistState::new();
    let ui_weak = ui.as_weak();
    let ai_st = ai_state.clone();
    ui.on_snip_ai_explain(move || {
        let Some(ui) = ui_weak.upgrade() else { return };

        let title = ui.get_snip_detail_title().to_string();
        let lang = ui.get_snip_detail_language().to_string();
        let code = ui.get_snip_detail_code().to_string();
        if code.is_empty() { return; }

        let prompt = super::ai_assist::snippet_explain_prompt(&title, &lang, &code);

        super::ai_assist::ai_request(
            &ui.as_weak(),
            &bridge,
            &ai_st,
            super::ai_assist::AiAssistRequest {
                prompt,
                timeout_secs: 30,
                set_working: Box::new(|ui, v| ui.set_snip_ai_is_working(v)),
                set_response: Box::new(|ui, s| ui.set_snip_ai_response(s.into())),
                get_response: Box::new(|ui| ui.get_snip_ai_response().to_string()),
            },
        );
    });

    let ui_weak = ui.as_weak();
    ui.on_snip_ai_dismiss(move || {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_snip_ai_panel_open(false);
        }
    });

    // ── View version callback ──
    {
        let ui_weak = ui.as_weak();
        let st = state.clone();
        ui.on_snip_view_version(move |idx| {
            let s = st.borrow();
            if let Some(versions) = s.version_history.get(&s.selected_id) {
                if let Some(ver) = versions.get(idx as usize) {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_snip_version_content(SharedString::from(&ver.code));
                    }
                }
            }
        });
    }

    // ── Restore version callback ──
    {
        let ui_weak = ui.as_weak();
        let st = state.clone();
        ui.on_snip_restore_version(move |idx| {
            let code_to_restore = {
                let s = st.borrow();
                s.version_history
                    .get(&s.selected_id)
                    .and_then(|versions| versions.get(idx as usize))
                    .map(|ver| ver.code.clone())
            };

            if let Some(code) = code_to_restore {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_snip_detail_code(SharedString::from(&code));
                    ui.set_snip_version_content(SharedString::default());
                    tracing::info!("Version restored to detail panel");
                }
            }
        });
    }
}

/// Update the version list UI for a given snippet.
fn update_version_list(ui: &App, state: &SnippetState, snippet_id: i64) {
    if let Some(versions) = state.version_history.get(&snippet_id) {
        let labels: Vec<slint::SharedString> = versions
            .iter()
            .map(|v| SharedString::from(&v.label))
            .collect();
        ui.set_snip_version_list(ModelRc::new(VecModel::from(labels)));
    } else {
        ui.set_snip_version_list(ModelRc::new(VecModel::default()));
    }
    ui.set_snip_version_content(SharedString::default());
}

/// Export snippets to JSON string.
fn export_snippets_json(snippets: &[Snippet]) -> String {
    // Manual JSON building to avoid serde dependency.
    let mut out = String::from("[\n");
    for (i, s) in snippets.iter().enumerate() {
        if i > 0 {
            out.push_str(",\n");
        }
        out.push_str("  {\n");
        out.push_str(&format!("    \"title\": {},\n", json_escape(&s.title)));
        out.push_str(&format!("    \"language\": {},\n", json_escape(&s.language)));
        out.push_str(&format!("    \"code\": {},\n", json_escape(&s.code)));
        out.push_str(&format!("    \"tags\": {},\n", json_escape(&s.tags)));
        out.push_str(&format!("    \"is_favorite\": {},\n", s.is_favorite));
        out.push_str(&format!("    \"created_at\": {},\n", s.created_at));
        out.push_str(&format!("    \"updated_at\": {},\n", s.updated_at));
        out.push_str(&format!("    \"last_used_at\": {},\n", s.last_used_at));
        out.push_str(&format!("    \"use_count\": {}\n", s.use_count));
        out.push_str("  }");
    }
    out.push_str("\n]\n");
    out
}

/// Escape a string for JSON.
fn json_escape(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Import snippets from a JSON string. Returns count of imported snippets.
fn import_snippets_json(conn: &Connection, data: &str) -> usize {
    // Simple JSON array parser — expects the format produced by export_snippets_json.
    let mut count = 0;
    let ts = now_ts();

    // Use a basic approach: split by objects.
    // We'll look for each { ... } block inside the array.
    let trimmed = data.trim();
    if !trimmed.starts_with('[') || !trimmed.ends_with(']') {
        tracing::error!("Import file is not a JSON array");
        return 0;
    }

    let inner = &trimmed[1..trimmed.len() - 1];

    // Parse objects by tracking brace depth.
    let mut depth = 0;
    let mut obj_start = None;
    let mut in_string = false;
    let mut escape_next = false;

    for (i, c) in inner.char_indices() {
        if escape_next {
            escape_next = false;
            continue;
        }
        if c == '\\' && in_string {
            escape_next = true;
            continue;
        }
        if c == '"' {
            in_string = !in_string;
            continue;
        }
        if in_string {
            continue;
        }
        if c == '{' {
            if depth == 0 {
                obj_start = Some(i);
            }
            depth += 1;
        } else if c == '}' {
            depth -= 1;
            if depth == 0 {
                if let Some(start) = obj_start {
                    let obj_str = &inner[start..=i];
                    if import_one_snippet(conn, obj_str, ts) {
                        count += 1;
                    }
                }
                obj_start = None;
            }
        }
    }

    count
}

/// Parse and import a single snippet JSON object.
fn import_one_snippet(conn: &Connection, obj: &str, fallback_ts: f64) -> bool {
    let title = extract_json_string(obj, "title").unwrap_or_default();
    let language = extract_json_string(obj, "language").unwrap_or_default();
    let code = extract_json_string(obj, "code").unwrap_or_default();
    let tags = extract_json_string(obj, "tags").unwrap_or_default();
    let is_favorite = extract_json_bool(obj, "is_favorite");
    let created_at = extract_json_number(obj, "created_at").unwrap_or(fallback_ts);
    let updated_at = extract_json_number(obj, "updated_at").unwrap_or(fallback_ts);
    let last_used_at = extract_json_number(obj, "last_used_at").unwrap_or(0.0);
    let use_count = extract_json_number(obj, "use_count").unwrap_or(0.0) as i64;

    conn.execute(
        "INSERT INTO snippets (title, language, code, tags, created_at, updated_at, is_favorite, collection_id, last_used_at, use_count)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?8, ?9)",
        rusqlite::params![title, language, code, tags, created_at, updated_at, is_favorite as i64, last_used_at, use_count],
    ).is_ok()
}

/// Extract a string value from a JSON object by key (simple parser).
fn extract_json_string(obj: &str, key: &str) -> Option<String> {
    let search = format!("\"{}\"", key);
    let pos = obj.find(&search)?;
    let after_key = &obj[pos + search.len()..];
    // Skip whitespace and colon
    let after_colon = after_key.trim_start().strip_prefix(':')?;
    let after_colon = after_colon.trim_start();

    if !after_colon.starts_with('"') {
        return None;
    }

    // Parse the string value
    let content = &after_colon[1..];
    let mut result = String::new();
    let mut chars = content.chars();
    loop {
        match chars.next()? {
            '"' => break,
            '\\' => match chars.next()? {
                '"' => result.push('"'),
                '\\' => result.push('\\'),
                'n' => result.push('\n'),
                'r' => result.push('\r'),
                't' => result.push('\t'),
                'u' => {
                    let hex: String = chars.by_ref().take(4).collect();
                    if let Ok(cp) = u32::from_str_radix(&hex, 16) {
                        if let Some(c) = char::from_u32(cp) {
                            result.push(c);
                        }
                    }
                }
                c => {
                    result.push('\\');
                    result.push(c);
                }
            },
            c => result.push(c),
        }
    }
    Some(result)
}

/// Extract a number value from a JSON object by key.
fn extract_json_number(obj: &str, key: &str) -> Option<f64> {
    let search = format!("\"{}\"", key);
    let pos = obj.find(&search)?;
    let after_key = &obj[pos + search.len()..];
    let after_colon = after_key.trim_start().strip_prefix(':')?;
    let after_colon = after_colon.trim_start();

    // Read digits, dots, minus, e, E
    let num_str: String = after_colon
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-' || *c == 'e' || *c == 'E' || *c == '+')
        .collect();
    num_str.parse().ok()
}

/// Extract a boolean value from a JSON object by key.
fn extract_json_bool(obj: &str, key: &str) -> bool {
    let search = format!("\"{}\"", key);
    if let Some(pos) = obj.find(&search) {
        let after_key = &obj[pos + search.len()..];
        if let Some(after_colon) = after_key.trim_start().strip_prefix(':') {
            let after_colon = after_colon.trim_start();
            return after_colon.starts_with("true");
        }
    }
    false
}

/// Copy text to the system clipboard using available tools.
fn copy_to_clipboard(text: &str, id: i32) {
    use std::io::Write;
    use std::process::{Command, Stdio};

    // Try wl-copy (Wayland) — accepts text via stdin
    let try_wl = Command::new("wl-copy")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .and_then(|mut child| {
            if let Some(ref mut stdin) = child.stdin {
                stdin.write_all(text.as_bytes())?;
            }
            child.wait()
        });

    if try_wl.is_ok() {
        tracing::info!(id, "Snippet copied to clipboard via wl-copy");
        return;
    }

    // Try xclip (X11)
    let try_xclip = Command::new("xclip")
        .args(["-selection", "clipboard"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .and_then(|mut child| {
            if let Some(ref mut stdin) = child.stdin {
                stdin.write_all(text.as_bytes())?;
            }
            child.wait()
        });

    if try_xclip.is_ok() {
        tracing::info!(id, "Snippet copied to clipboard via xclip");
        return;
    }

    // Try xsel
    let try_xsel = Command::new("xsel")
        .args(["--clipboard", "--input"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .and_then(|mut child| {
            if let Some(ref mut stdin) = child.stdin {
                stdin.write_all(text.as_bytes())?;
            }
            child.wait()
        });

    if try_xsel.is_ok() {
        tracing::info!(id, "Snippet copied to clipboard via xsel");
        return;
    }

    tracing::warn!("No clipboard tool found (tried wl-copy, xclip, xsel)");
}

/// Load snippets when navigating to screen 25.
pub fn load_snippets(ui: &App) {
    if let Ok(conn) = open_db() {
        let snippets = load_all(&conn);
        let db_collections = load_collections(&conn);

        let state = SnippetState {
            all_snippets: snippets,
            db_collections,
            selected_id: -1,
            active_tag: String::new(),
            active_collection: COLL_ALL,
            search_query: String::new(),
            version_history: std::collections::HashMap::new(),
        };

        refresh_ui(ui, &state);
        clear_detail(ui);

        // Set language options
        ui.set_snip_language_options(ModelRc::new(VecModel::from(build_language_options())));
    }
}
