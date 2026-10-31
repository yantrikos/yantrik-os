//! The Bench's rules, run against a real folder in a temporary home.

use super::*;
use yantrik_bench::layout::METRICS;

/// A home with a desktop holding `files` (a name ending `/` is a folder), an icon area of 4
/// columns by 4 rows, and a saver that writes in this thread.
fn desk(files: &[&str]) -> (tempfile::TempDir, Model) {
    let tmp = tempfile::tempdir().unwrap();
    let desktop = tmp.path().join("Desktop");
    std::fs::create_dir(&desktop).unwrap();
    for f in files {
        match f.strip_suffix('/') {
            Some(dir) => std::fs::create_dir(desktop.join(dir)).unwrap(),
            None => std::fs::write(desktop.join(f), format!("contents of {f}")).unwrap(),
        }
    }
    let layout = tmp.path().join(".yantrik/desktop_layout.json");
    let saver = Saver::Direct { layout: layout.clone(), recent: tmp.path().join(".yantrik/last_opened.json") };
    let mut m = Model::new(desktop, Store::load(&layout), saver);
    m.rescan();
    m.set_area(4 * METRICS.cell_w + 2 * METRICS.pad_x, 4 * METRICS.cell_h + 2 * METRICS.pad_y);
    (tmp, m)
}

fn cell(m: &Model, name: &str) -> Option<(u32, u32)> {
    m.placed_at(name).and_then(|p| p.cell)
}

fn names(m: &Model) -> Vec<String> {
    m.selection.iter().cloned().collect()
}

#[test]
fn a_fresh_desktop_lists_what_is_there_in_the_default_flow() {
    let (_t, m) = desk(&["b.txt", "Folder/", "a.txt"]);
    assert_eq!(cell(&m, "Folder"), Some((0, 0)), "folders first");
    assert_eq!(cell(&m, "a.txt"), Some((0, 1)));
    assert_eq!(cell(&m, "b.txt"), Some((0, 2)));
}

#[test]
fn a_new_file_arriving_takes_the_next_free_cell_and_moves_nothing() {
    let (t, mut m) = desk(&["a.txt", "c.txt"]);
    m.place_in_cell("c.txt", 2, 1).unwrap();
    std::fs::write(t.path().join("Desktop/b.txt"), "").unwrap();
    assert!(m.rescan());
    assert_eq!(cell(&m, "a.txt"), Some((0, 0)));
    assert_eq!(cell(&m, "c.txt"), Some((2, 1)), "a moved icon stays");
    assert_eq!(cell(&m, "b.txt"), Some((0, 1)));
    // And it stays there when another goes.
    std::fs::remove_file(t.path().join("Desktop/a.txt")).unwrap();
    m.rescan();
    assert_eq!(cell(&m, "b.txt"), Some((0, 1)), "the arrival was fixed where it landed");
}

#[test]
fn the_layout_is_in_the_file_and_comes_back_next_session() {
    let (t, mut m) = desk(&["a.txt", "b.txt"]);
    m.place_in_cell("b.txt", 3, 3).unwrap();
    let path = t.path().join(".yantrik/desktop_layout.json");
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("b.txt"), "{text}");
    let again = {
        let saver = Saver::Direct { layout: path.clone(), recent: t.path().join("r.json") };
        let mut m2 = Model::new(m.desktop.clone(), Store::load(&path), saver);
        m2.rescan();
        m2.set_area(4 * METRICS.cell_w + 2 * METRICS.pad_x, 4 * METRICS.cell_h + 2 * METRICS.pad_y);
        m2
    };
    assert_eq!(cell(&again, "b.txt"), Some((3, 3)));
}

#[test]
fn a_file_that_goes_is_forgotten_and_a_folder_that_cannot_be_read_forgets_nothing() {
    let (t, mut m) = desk(&["a.txt", "b.txt"]);
    m.place_in_cell("b.txt", 3, 0).unwrap();
    std::fs::remove_file(t.path().join("Desktop/b.txt")).unwrap();
    m.rescan();
    assert!(m.store.get(&m.grid().unwrap().key(), "b.txt").is_none(), "its place is dropped");
    // The folder cannot be listed (here: it is gone): nothing is pruned, the problem is said.
    m.place_in_cell("a.txt", 2, 2).unwrap();
    std::fs::remove_dir_all(t.path().join("Desktop")).unwrap();
    m.rescan();
    assert!(m.problem.is_some());
    assert!(m.store.get(&m.grid().unwrap().key(), "a.txt").is_some(), "a failed listing is not 'everything was deleted'");
}

#[test]
fn clicking_selects_one_and_ctrl_adds_and_removes() {
    let (_t, mut m) = desk(&["a", "b", "c"]);
    m.press("a", false);
    m.press("b", true);
    assert_eq!(names(&m), ["a", "b"]);
    m.press("a", true);
    assert_eq!(names(&m), ["b"]);
    m.press("c", false);
    assert_eq!(names(&m), ["c"], "a plain press on another icon replaces the selection");
    m.press("ghost", false);
    assert_eq!(names(&m), ["c"], "an icon that is not there selects nothing");
}

#[test]
fn a_press_on_a_selected_icon_keeps_the_group_to_drag_and_a_click_narrows_it() {
    let (_t, mut m) = desk(&["a", "b", "c"]);
    m.press("a", false);
    m.press("b", true);
    m.press("a", false);
    assert_eq!(names(&m), ["a", "b"], "so the group can be dragged");
    m.click_release("a", false);
    assert_eq!(names(&m), ["a"], "released without a drag: just this one");
    m.press("b", true);
    m.click_release("b", true);
    assert_eq!(names(&m), ["a", "b"], "Ctrl-click never narrows");
}

#[test]
fn the_band_selects_what_it_crosses_and_ctrl_keeps_what_was_selected() {
    let (_t, mut m) = desk(&["a", "b", "c", "d", "e"]);
    // a b c d fill column 0; e is at the top of column 1.
    let (x, y) = (m.placed_at("a").unwrap().x, m.placed_at("a").unwrap().y);
    m.select_rect(x, y, 30, METRICS.cell_h + 10, false);
    assert_eq!(names(&m), ["a", "b"]);
    m.select_rect(x + METRICS.cell_w, y, 20, 20, true);
    assert_eq!(names(&m), ["a", "b", "e"]);
    m.select_rect(0, 0, 1, 1, false);
    assert!(m.selection.is_empty(), "a band over nothing selects nothing");
}

#[test]
fn the_arrow_keys_walk_the_grid() {
    let (_t, mut m) = desk(&["a", "b", "c", "d", "e"]);
    m.arrow(Dir::Down, false);
    assert_eq!(names(&m), ["a"], "with nothing selected, the first");
    m.arrow(Dir::Down, false);
    assert_eq!(names(&m), ["b"]);
    m.arrow(Dir::Right, false);
    assert_eq!(names(&m), ["e"], "the nearest to the right");
    m.arrow(Dir::Up, false);
    assert_eq!(names(&m), ["e"], "nothing above: it stays");
    m.arrow(Dir::Left, false);
    assert_eq!(names(&m), ["a"], "nearest to the left is on the same row or the closest");
    m.arrow(Dir::Down, true);
    assert_eq!(names(&m), ["a", "b"], "Shift extends");
}

#[test]
fn dragging_snaps_to_a_free_cell_and_a_group_keeps_its_shape() {
    let (_t, mut m) = desk(&["a", "b", "c"]);
    m.press("a", false);
    m.press("b", true);
    // Two columns right, from (0,0) and (0,1): lands on (2,0) and (2,1).
    assert!(m.move_selected(2 * METRICS.cell_w + 5, 3));
    assert_eq!((cell(&m, "a"), cell(&m, "b")), (Some((2, 0)), Some((2, 1))));
    assert_eq!(cell(&m, "c"), Some((0, 2)), "what was not dragged did not move");
}

#[test]
fn dropping_on_an_occupied_cell_takes_the_nearest_free_one_and_displaces_nothing() {
    let (_t, mut m) = desk(&["a", "b"]);
    m.press("a", false);
    // Drop a onto b's cell (0, 1).
    assert!(m.move_selected(0, METRICS.cell_h));
    assert_eq!(cell(&m, "b"), Some((0, 1)));
    let a = cell(&m, "a").unwrap();
    assert_ne!(a, (0, 1));
    assert_eq!(a.0.abs_diff(0) + a.1.abs_diff(1), 1, "one cell from where it was dropped: {a:?}");
}

#[test]
fn with_snapping_off_an_icon_stays_where_it_is_let_go() {
    let (_t, mut m) = desk(&["a", "b"]);
    m.set_snap(false);
    m.press("a", false);
    m.move_selected(47, 31);
    let p = m.placed_at("a").unwrap();
    assert_eq!((p.x, p.y, p.cell), (METRICS.pad_x + 47, METRICS.pad_y + 31, None));
    // Turning snapping back on puts it in a cell.
    m.set_snap(true);
    assert!(m.placed_at("a").unwrap().cell.is_some());
}

#[test]
fn placing_in_a_cell_refuses_what_is_off_the_grid_or_taken() {
    let (_t, mut m) = desk(&["a", "b"]);
    assert!(m.place_in_cell("a", 9, 0).unwrap_err().to_string().contains("off the desktop"));
    assert!(m.place_in_cell("a", 0, 1).unwrap_err().to_string().contains("holds `b`"));
    assert!(matches!(m.place_in_cell("ghost", 1, 1), Err(ModelError::NotThere(_))));
    assert_eq!(cell(&m, "a"), Some((0, 0)), "nothing moved on a refusal");
    m.place_in_cell("a", 3, 3).unwrap();
    assert_eq!(cell(&m, "a"), Some((3, 3)));
}

#[test]
fn arrange_sorts_and_writes_the_places() {
    let (_t, mut m) = desk(&["b.txt", "a.png", "Zed/"]);
    m.sort(Sort::Name);
    assert_eq!((cell(&m, "a.png"), cell(&m, "b.txt"), cell(&m, "Zed")), (Some((0, 0)), Some((0, 1)), Some((0, 2))));
    m.sort(Sort::Type);
    assert_eq!(cell(&m, "Zed"), Some((0, 0)), "folders before pictures before text");
}

#[test]
fn a_smaller_screen_flows_what_does_not_fit_and_the_big_one_is_remembered() {
    let (_t, mut m) = desk(&["a", "b"]);
    m.place_in_cell("b", 3, 3).unwrap();
    let big = m.grid().unwrap().key();
    m.set_area(2 * METRICS.cell_w + 2 * METRICS.pad_x, 2 * METRICS.cell_h + 2 * METRICS.pad_y);
    assert_eq!(cell(&m, "b"), Some((0, 1)), "reflowed into the small grid");
    m.set_area(4 * METRICS.cell_w + 2 * METRICS.pad_x, 4 * METRICS.cell_h + 2 * METRICS.pad_y);
    assert_eq!(m.grid().unwrap().key(), big);
    assert_eq!(cell(&m, "b"), Some((3, 3)), "and back where the person put it");
}

#[test]
fn rename_never_replaces_and_takes_its_place_with_it() {
    let (t, mut m) = desk(&["a.txt", "b.txt"]);
    m.place_in_cell("a.txt", 2, 2).unwrap();
    assert!(matches!(m.rename("a.txt", "b.txt"), Err(ModelError::Taken(_))));
    assert_eq!(std::fs::read_to_string(t.path().join("Desktop/b.txt")).unwrap(), "contents of b.txt");
    for bad in ["", "..", "x/y", ".hidden", "two\nlines", "esc\u{1b}[2J"] {
        assert!(matches!(m.rename("a.txt", bad), Err(ModelError::BadName(_))), "{bad:?}");
    }
    m.press("a.txt", false);
    m.rename("a.txt", "  renamed.txt ").unwrap();
    assert_eq!(cell(&m, "renamed.txt"), Some((2, 2)));
    assert_eq!(names(&m), ["renamed.txt"], "the selection follows it");
    assert!(m.item("a.txt").is_none());
}

#[test]
fn new_names_count_up() {
    let (t, m) = desk(&["New folder/", "New folder 2/"]);
    assert_eq!(m.free_name("New folder", ""), "New folder 3");
    assert_eq!(m.free_name("New file", ".txt"), "New file.txt");
    std::fs::write(t.path().join("Desktop/New file.txt"), "").unwrap();
    assert_eq!(m.free_name("New file", ".txt"), "New file 2.txt");
}

#[test]
fn a_caller_may_use_the_drawn_name_when_it_is_unambiguous() {
    let (_t, m) = desk(&["plain.txt", "tab\there"]);
    assert!(m.find("plain.txt").is_some());
    assert_eq!(m.find("tab\u{fffd}here").map(|i| i.name.as_str()), Some("tab\there"));
    assert!(m.find("nope").is_none());
}

#[test]
fn hiding_the_icons_is_remembered_and_does_not_touch_the_files() {
    let (t, mut m) = desk(&["a.txt"]);
    m.set_show_icons(false);
    assert!(!m.show_icons());
    assert!(t.path().join("Desktop/a.txt").exists());
    let text = std::fs::read_to_string(t.path().join(".yantrik/desktop_layout.json")).unwrap();
    assert!(text.contains("\"icons\": false"), "{text}");
}
