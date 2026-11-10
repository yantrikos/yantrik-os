//! What `describe shell` says about the desktop's icons, and the two things a mind may ask the
//! Bench to do.
//!
//! `desktop.items` is names, kinds and places: nothing from inside a file, no size, no date, no
//! path. A mind that wants to know what a file holds asks the file tools, which have their own
//! gate; the desktop is not a second way round it. For a mind caller the whole list is withheld
//! when the desktop folder is somewhere the file tools would not let it look
//! (`control_files_mind`), the same rule Files' listing follows.

use serde_json::{json, Value};
use yantrik_bench::items::display_name;

use super::model::{display_path, Model};
use super::{ops, with, Bench};

/// Items listed in one answer; the rest are counted.
const LISTED: usize = 200;

/// The `desktop` part of `describe shell`.
pub fn for_describe() -> Value {
    with(|b| describe(b)).unwrap_or(Value::Null)
}

fn label(b: &Bench) -> String {
    display_path(&b.home, &b.model.desktop)
}

fn describe(b: &Bench) -> Value {
    let folder = label(b);
    let grid = b.model.grid().map(|g| json!({ "columns": g.cols, "rows": g.rows, "cell_width": g.m.cell_w, "cell_height": g.m.cell_h }));
    let base = json!({
        "folder": folder,
        "snap": b.model.store.snap,
        "icons_shown": b.model.show_icons(),
        "grid": grid,
        "problem": b.model.problem,
    });
    let mut out = base.as_object().cloned().unwrap_or_default();
    if let Some(why) = crate::control_files_mind::hidden_here(&folder) {
        out.insert("hidden".into(), json!(why));
        out.insert("items".into(), json!([]));
        out.insert("count".into(), json!(0));
        return Value::Object(out);
    }
    let items: Vec<Value> = b.model.placed.iter().filter_map(|p| Some((p, b.model.item(&p.name)?))).take(LISTED).map(|(p, i)| item_json(&b.model, p, i)).collect();
    out.insert("count".into(), json!(b.model.listing.len()));
    out.insert("more".into(), json!(b.model.listing.len().saturating_sub(items.len()) + b.model.more));
    out.insert("selected".into(), json!(b.model.selection.iter().map(|n| display_name(n)).collect::<Vec<_>>()));
    out.insert("items".into(), json!(items));
    Value::Object(out)
}

fn item_json(model: &Model, p: &yantrik_bench::layout::Placed, i: &yantrik_bench::items::Item) -> Value {
    json!({
        "name": display_name(&i.name),
        "kind": i.kind.word(),
        "x": p.x,
        "y": p.y,
        "cell": p.cell.map(|(c, r)| json!([c, r])),
        "selected": model.selection.contains(&i.name),
    })
}

// ── What a mind may ask ──

/// `desktop_place`: put one icon in one cell. Refuses a cell that is off the grid or held by
/// another icon, and an icon a mind may not touch. Answers with where it is now, read back.
pub fn place(ui: &crate::App, said: &str, column: i64, row: i64) -> Result<Value, String> {
    with(|b| {
        let name = b.model.find(said).map(|i| i.name.clone()).ok_or_else(|| format!("there is nothing called `{said}` on the desktop; `describe shell` lists desktop.items"))?;
        crate::control_files_mind::may_touch(&label(b), std::slice::from_ref(&name))?;
        let (col, row) = (u32::try_from(column).map_err(|_| "`column` is not a cell")?, u32::try_from(row).map_err(|_| "`row` is not a cell")?);
        b.model.place_in_cell(&name, col, row).map_err(|e| e.to_string())?;
        super::view::publish(ui, b);
        let at = b.model.placed_at(&name).ok_or("the icon is not placed")?;
        Ok(json!({ "placed": display_name(&name), "cell": at.cell.map(|(c, r)| json!([c, r])), "x": at.x, "y": at.y }))
    })
    .unwrap_or_else(|| Err("the desktop is not ready or is busy; try again".into()))
}

/// `desktop_archive`: move one thing to `~/Archive/<date>/`. It is never deleted, and the person
/// is shown a toast with Undo.
pub fn archive(ui: &crate::App, said: &str) -> Result<Value, String> {
    with(|b| {
        let name = b.model.find(said).map(|i| i.name.clone()).ok_or_else(|| format!("there is nothing called `{said}` on the desktop; `describe shell` lists desktop.items"))?;
        crate::control_files_mind::may_touch(&label(b), std::slice::from_ref(&name))?;
        let done = ops::archive_names(b, std::slice::from_ref(&name))?;
        let moved_to = done.first().map(|a| crate::filebrowser::collapse_home(&a.to)).unwrap_or_default();
        ops::offer_undo(ui, b, done);
        super::view::publish(ui, b);
        Ok(json!({
            "archived": display_name(&name),
            "moved_to": moved_to,
            "deleted": false,
            "undo": "the person has an Undo on screen for a few seconds; after that it is moved back by taking it from the archive folder in Files",
        }))
    })
    .unwrap_or_else(|| Err("the desktop is not ready or is busy; try again".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bench::persist::Saver;
    use crate::bench::pictures::Pictures;
    use crate::bench::shortcuts::Shortcuts;
    use yantrik_bench::layout::{Store, METRICS};

    /// A Bench over a temporary home whose desktop holds `files`, each with a secret inside.
    pub(crate) fn bench_with(files: &[&str]) -> (tempfile::TempDir, Bench) {
        let tmp = tempfile::tempdir().unwrap();
        let desktop = tmp.path().join("Desktop");
        std::fs::create_dir(&desktop).unwrap();
        for f in files {
            std::fs::write(desktop.join(f), "TOP-SECRET-CONTENTS Exec=/bin/evil").unwrap();
        }
        let layout = tmp.path().join(".yantrik/desktop_layout.json");
        let saver = Saver::Direct { layout: layout.clone(), recent: tmp.path().join(".yantrik/last_opened.json") };
        let mut model = Model::new(desktop, Store::load(&layout), saver);
        model.rescan();
        model.set_area(4 * METRICS.cell_w + 2 * METRICS.pad_x, 4 * METRICS.cell_h + 2 * METRICS.pad_y);
        let bench = Bench {
            model,
            pictures: Pictures::none(),
            shortcuts: Shortcuts::default(),
            home: tmp.path().to_path_buf(),
            catalogue: crate::apps::Catalogue::shared(),
            last_opened: None,
            archived: Vec::new(),
            root: None,
        };
        (tmp, bench)
    }

    /// `desktop.items` is names, kinds and places. Not a byte of what is inside a file, not a
    /// size, a date or a path.
    #[test]
    fn describe_carries_names_kinds_and_places_and_nothing_from_inside_a_file() {
        let (tmp, b) = bench_with(&["notes.txt", "photo.png", "run.desktop"]);
        let said = describe(&b).to_string();
        for name in ["notes.txt", "photo.png", "run.desktop"] {
            assert!(said.contains(name), "{name} is listed: {said}");
        }
        for leak in ["TOP-SECRET", "/bin/evil", "Exec=", tmp.path().to_str().unwrap(), "mtime", "modified", "\"size\"", "bytes"] {
            assert!(!said.contains(leak), "describe leaks {leak:?}: {said}");
        }
        let v = describe(&b);
        let first = &v["items"][0];
        let mut keys: Vec<_> = first.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, ["cell", "kind", "name", "selected", "x", "y"], "exactly these fields");
        assert_eq!(v["folder"], "~/Desktop");
        assert_eq!(v["count"], 3);
        assert_eq!(v["grid"]["columns"], 4);
    }

    #[test]
    fn a_name_that_fakes_another_is_described_cleaned() {
        let (_t, b) = bench_with(&["gpj.\u{202e}fdp"]);
        let said = describe(&b).to_string();
        assert!(!said.contains('\u{202e}'), "{said}");
    }

    #[test]
    fn a_long_desktop_is_listed_in_part_and_counted_whole() {
        let names: Vec<String> = (0..LISTED + 25).map(|i| format!("f{i:04}.txt")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let (_t, b) = bench_with(&refs);
        let v = describe(&b);
        assert_eq!(v["items"].as_array().unwrap().len(), LISTED);
        assert_eq!(v["count"], LISTED + 25);
        assert_eq!(v["more"], 25);
    }

    #[test]
    fn placing_moves_an_icon_and_says_where_it_is_now_and_refuses_what_is_taken() {
        let (_t, mut b) = bench_with(&["a.txt", "b.txt"]);
        b.model.place_in_cell("a.txt", 3, 2).unwrap();
        let at = b.model.placed_at("a.txt").unwrap();
        assert_eq!(at.cell, Some((3, 2)));
        assert!(b.model.place_in_cell("b.txt", 3, 2).is_err());
    }

    /// The point of the archive actions: nothing is deleted, and it goes where the answer says.
    #[test]
    fn archiving_through_the_actions_path_moves_the_file_and_deletes_nothing() {
        let (tmp, mut b) = bench_with(&["a.txt", "b.txt"]);
        let done = ops::archive_names(&mut b, &["a.txt".to_string()]).unwrap();
        assert_eq!(done.len(), 1);
        assert!(!tmp.path().join("Desktop/a.txt").exists());
        assert_eq!(std::fs::read_to_string(&done[0].to).unwrap(), "TOP-SECRET-CONTENTS Exec=/bin/evil");
        assert!(done[0].to.starts_with(tmp.path().join("Archive")));
        assert!(b.model.item("a.txt").is_none() && b.model.item("b.txt").is_some(), "the desktop lists what is left");
        // And back.
        let back = yantrik_bench::archive::undo(&done[0]).unwrap();
        assert_eq!(back, tmp.path().join("Desktop/a.txt"));
    }

    #[test]
    fn archiving_a_name_that_is_not_on_the_desktop_moves_nothing() {
        let (tmp, mut b) = bench_with(&["a.txt"]);
        std::fs::write(tmp.path().join("outside.txt"), "x").unwrap();
        for bad in ["../outside.txt", "/etc/hostname", "ghost"] {
            assert!(ops::archive_names(&mut b, &[bad.to_string()]).is_err(), "{bad}");
        }
        assert!(tmp.path().join("outside.txt").exists() && tmp.path().join("Desktop/a.txt").exists());
    }

    /// A caller names an icon by what describe says, and a name that is no icon is refused.
    #[test]
    fn an_icon_is_found_by_its_listed_name_only() {
        let (_t, b) = bench_with(&["a.txt"]);
        assert!(b.model.find("a.txt").is_some());
        assert!(b.model.find("../a.txt").is_none());
        assert!(b.model.find("").is_none());
    }
}
