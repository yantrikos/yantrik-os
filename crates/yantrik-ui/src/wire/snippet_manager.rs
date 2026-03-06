//! Code Snippet Manager wire module — screen 25.
//!
//! SQLite-backed snippet storage at `~/.config/yantrik/snippets.db`.
//! Full-text search across title, code, tags. Tag filter chips.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use rusqlite::Connection;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::app_context::AppContext;
use crate::{App, SnippetData, TagChip};

/// A snippet row from the database.
#[derive(Clone, Debug)]
struct Snippet {
    id: i64,
    title: String,
    language: String,
    code: String,
    tags: String,
    created_at: f64,
    updated_at: f64,
}

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
        );",
    )?;
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
        "SELECT id, title, language, code, tags, created_at, updated_at
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

/// Format a unix timestamp as a date string like "Mar 6, 2026".
fn format_date(ts: f64) -> String {
    let secs = ts as u64;
    // Simple date formatting without chrono
    let days_since_epoch = secs / 86400;
    // Use a basic algorithm to get year/month/day
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

/// Filter snippets by search query (title, code, tags).
fn filter_snippets(snippets: &[Snippet], query: &str) -> Vec<Snippet> {
    let lower = query.to_lowercase();
    snippets
        .iter()
        .filter(|s| {
            s.title.to_lowercase().contains(&lower)
                || s.code.to_lowercase().contains(&lower)
                || s.tags.to_lowercase().contains(&lower)
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

/// Refresh the snippet list and tag chips in the UI.
fn refresh_ui(
    ui: &App,
    snippets: &[Snippet],
    selected_id: i64,
    active_tag: &str,
) {
    // Apply tag filter if active
    let display: Vec<Snippet> = if active_tag.is_empty() {
        snippets.to_vec()
    } else {
        filter_by_tag(snippets, active_tag)
    };

    let items: Vec<SnippetData> = display
        .iter()
        .map(|s| snippet_to_model(s, selected_id))
        .collect();
    let count = items.len() as i32;
    ui.set_snip_snippets(ModelRc::new(VecModel::from(items)));
    ui.set_snip_snippet_count(count);

    // Tag chips — always from full set
    let all_tags = extract_tags(snippets);
    let chips: Vec<TagChip> = all_tags
        .iter()
        .map(|t| TagChip {
            name: SharedString::from(t.as_str()),
            is_active: t.to_lowercase() == active_tag.to_lowercase(),
        })
        .collect();
    ui.set_snip_tag_chips(ModelRc::new(VecModel::from(chips)));
}

/// Wire snippet manager callbacks.
pub fn wire(ui: &App, _ctx: &AppContext) {
    let db = match open_db() {
        Ok(c) => Rc::new(RefCell::new(c)),
        Err(e) => {
            tracing::error!(error = %e, "Failed to open snippets database");
            return;
        }
    };

    let all_snippets: Rc<RefCell<Vec<Snippet>>> = Rc::new(RefCell::new(Vec::new()));
    let selected_id: Rc<RefCell<i64>> = Rc::new(RefCell::new(-1));
    let active_tag: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));

    // Initial load
    {
        let loaded = load_all(&db.borrow());
        *all_snippets.borrow_mut() = loaded;
    }

    // ── Search ──
    {
        let ui_weak = ui.as_weak();
        let snippets = all_snippets.clone();
        let sel = selected_id.clone();
        let tag = active_tag.clone();
        ui.on_snip_search(move |query| {
            let query_str = query.to_string();
            if let Some(ui) = ui_weak.upgrade() {
                let all = snippets.borrow();
                let sid = *sel.borrow();
                if query_str.is_empty() {
                    refresh_ui(&ui, &all, sid, &tag.borrow());
                } else {
                    let filtered = filter_snippets(&all, &query_str);
                    let items: Vec<SnippetData> = filtered
                        .iter()
                        .map(|s| snippet_to_model(s, sid))
                        .collect();
                    let count = items.len() as i32;
                    ui.set_snip_snippets(ModelRc::new(VecModel::from(items)));
                    ui.set_snip_snippet_count(count);
                }
            }
        });
    }

    // ── Select ──
    {
        let ui_weak = ui.as_weak();
        let snippets = all_snippets.clone();
        let sel = selected_id.clone();
        let tag = active_tag.clone();
        ui.on_snip_select(move |id| {
            let id = id as i64;
            *sel.borrow_mut() = id;
            if let Some(ui) = ui_weak.upgrade() {
                let all = snippets.borrow();
                if let Some(s) = all.iter().find(|s| s.id == id) {
                    ui.set_snip_detail_id(s.id as i32);
                    ui.set_snip_detail_title(SharedString::from(&s.title));
                    ui.set_snip_detail_language(SharedString::from(&s.language));
                    ui.set_snip_detail_code(SharedString::from(&s.code));
                    ui.set_snip_detail_tags(SharedString::from(&s.tags));
                }
                refresh_ui(&ui, &all, id, &tag.borrow());
            }
        });
    }

    // ── New snippet ──
    {
        let ui_weak = ui.as_weak();
        let db = db.clone();
        let snippets = all_snippets.clone();
        let sel = selected_id.clone();
        let tag = active_tag.clone();
        ui.on_snip_new(move || {
            let ts = now_ts();
            let conn = db.borrow();
            match conn.execute(
                "INSERT INTO snippets (title, language, code, tags, created_at, updated_at)
                 VALUES ('Untitled', '', '', '', ?1, ?2)",
                rusqlite::params![ts, ts],
            ) {
                Ok(_) => {
                    let new_id = conn.last_insert_rowid();
                    let loaded = load_all(&conn);
                    *snippets.borrow_mut() = loaded;
                    *sel.borrow_mut() = new_id;

                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_snip_detail_id(new_id as i32);
                        ui.set_snip_detail_title("Untitled".into());
                        ui.set_snip_detail_language("".into());
                        ui.set_snip_detail_code("".into());
                        ui.set_snip_detail_tags("".into());
                        refresh_ui(&ui, &snippets.borrow(), new_id, &tag.borrow());
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
        let snippets = all_snippets.clone();
        let sel = selected_id.clone();
        let tag = active_tag.clone();
        ui.on_snip_save(move |id, title, language, code, tags| {
            let ts = now_ts();
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
                    id as i64
                ],
            ) {
                Ok(_) => {
                    let loaded = load_all(&conn);
                    *snippets.borrow_mut() = loaded;

                    if let Some(ui) = ui_weak.upgrade() {
                        refresh_ui(&ui, &snippets.borrow(), *sel.borrow(), &tag.borrow());
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
        let snippets = all_snippets.clone();
        let sel = selected_id.clone();
        let tag = active_tag.clone();
        ui.on_snip_delete(move |id| {
            let conn = db.borrow();
            match conn.execute("DELETE FROM snippets WHERE id=?1", rusqlite::params![id as i64]) {
                Ok(_) => {
                    let loaded = load_all(&conn);
                    *snippets.borrow_mut() = loaded;
                    *sel.borrow_mut() = -1;

                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_snip_detail_id(-1);
                        ui.set_snip_detail_title("".into());
                        ui.set_snip_detail_language("".into());
                        ui.set_snip_detail_code("".into());
                        ui.set_snip_detail_tags("".into());
                        refresh_ui(&ui, &snippets.borrow(), -1, &tag.borrow());
                    }
                    tracing::info!(id, "Snippet deleted");
                }
                Err(e) => {
                    tracing::error!(error = %e, id, "Failed to delete snippet");
                }
            }
        });
    }

    // ── Copy to clipboard ──
    {
        let snippets = all_snippets.clone();
        ui.on_snip_copy(move |id| {
            let all = snippets.borrow();
            if let Some(s) = all.iter().find(|s| s.id == id as i64) {
                let code = s.code.clone();
                copy_to_clipboard(&code, id);
            }
        });
    }

    // ── Tag filter ──
    {
        let ui_weak = ui.as_weak();
        let snippets = all_snippets.clone();
        let sel = selected_id.clone();
        let tag = active_tag.clone();
        ui.on_snip_tag_filter(move |clicked_tag| {
            let clicked = clicked_tag.to_string();
            let current = tag.borrow().clone();
            // Toggle: if same tag clicked again, clear filter
            let new_tag = if current.to_lowercase() == clicked.to_lowercase() {
                String::new()
            } else {
                clicked
            };
            *tag.borrow_mut() = new_tag.clone();

            if let Some(ui) = ui_weak.upgrade() {
                refresh_ui(&ui, &snippets.borrow(), *sel.borrow(), &new_tag);
            }
        });
    }
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
        let items: Vec<SnippetData> = snippets
            .iter()
            .map(|s| snippet_to_model(s, -1))
            .collect();
        let count = items.len() as i32;
        ui.set_snip_snippets(ModelRc::new(VecModel::from(items)));
        ui.set_snip_snippet_count(count);

        let all_tags = extract_tags(&snippets);
        let chips: Vec<TagChip> = all_tags
            .iter()
            .map(|t| TagChip {
                name: SharedString::from(t.as_str()),
                is_active: false,
            })
            .collect();
        ui.set_snip_tag_chips(ModelRc::new(VecModel::from(chips)));

        // Reset detail
        ui.set_snip_detail_id(-1);
        ui.set_snip_detail_title("".into());
        ui.set_snip_detail_language("".into());
        ui.set_snip_detail_code("".into());
        ui.set_snip_detail_tags("".into());
    }
}
