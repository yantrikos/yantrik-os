//! The layout's rules, run against real files in a temporary folder.
use super::*;
use crate::items::Kind;

fn grid() -> Grid {
    // 3 columns by 4 rows with METRICS: 3*104+40 wide, 4*116+16 high.
    Grid::new(3 * 104 + 40, 4 * 116 + 16, 0, METRICS)
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

fn cells(placed: &[Placed]) -> Vec<(&str, Option<(u32, u32)>)> {
    placed.iter().map(|p| (p.name.as_str(), p.cell)).collect()
}

/// The default grid flow: down the first column, then the next, from the top left.
#[test]
fn a_fresh_desktop_fills_column_first_from_the_top_left() {
    let g = grid();
    assert_eq!((g.cols, g.rows), (3, 4));
    let order = names(&["a", "b", "c", "d", "e", "f"]);
    let placed = Store::default().resolve(&g, &order);
    assert_eq!(
        cells(&placed),
        [
            ("a", Some((0, 0))),
            ("b", Some((0, 1))),
            ("c", Some((0, 2))),
            ("d", Some((0, 3))),
            ("e", Some((1, 0))),
            ("f", Some((1, 1))),
        ]
    );
    assert_eq!((placed[0].x, placed[0].y), (METRICS.pad_x, METRICS.pad_y));
    assert_eq!((placed[4].x, placed[4].y), (METRICS.pad_x + METRICS.cell_w, METRICS.pad_y));
}

#[test]
fn a_moved_icon_stays_and_the_rest_flow_around_it() {
    let g = grid();
    let mut s = Store::default();
    s.set(&g.key(), "b", Pos::Cell { col: 0, row: 0 });
    let placed = s.resolve(&g, &names(&["a", "b", "c"]));
    assert_eq!(cells(&placed), [("a", Some((0, 1))), ("b", Some((0, 0))), ("c", Some((0, 2)))]);
}

#[test]
fn a_free_icon_stays_exactly_where_it_was_put_and_holds_no_cell() {
    let g = grid();
    let mut s = Store { snap: false, ..Store::default() };
    s.set(&g.key(), "b", Pos::Free { x: 143, y: 77 });
    let placed = s.resolve(&g, &names(&["a", "b"]));
    assert_eq!((placed[1].x, placed[1].y, placed[1].cell), (143, 77, None));
    assert_eq!(placed[0].cell, Some((0, 0)), "a free icon does not take a cell from the flow");
}

#[test]
fn two_icons_claiming_one_cell_do_not_overlap() {
    let g = grid();
    let mut s = Store::default();
    s.set(&g.key(), "a", Pos::Cell { col: 1, row: 1 });
    s.set(&g.key(), "b", Pos::Cell { col: 1, row: 1 });
    let placed = s.resolve(&g, &names(&["a", "b"]));
    assert_eq!(placed[0].cell, Some((1, 1)));
    assert_eq!(placed[1].cell, Some((0, 0)), "the second takes the next free cell");
}

#[test]
fn a_new_file_takes_the_next_free_cell_and_a_removed_file_frees_its_cell() {
    let g = grid();
    let mut s = Store::default();
    for (n, r) in [("a", 0), ("b", 1), ("c", 2)] {
        s.set(&g.key(), n, Pos::Cell { col: 0, row: r });
    }
    // d arrives: the next free cell, below c, and it is fixed there.
    let placed = s.resolve(&g, &names(&["a", "b", "c", "d"]));
    assert_eq!(placed[3].cell, Some((0, 3)));
    assert!(s.adopt(&g.key(), &placed));
    assert!(!s.adopt(&g.key(), &placed), "adopting twice adds nothing");
    // b goes: its place is dropped when the layout is next written, and e takes it.
    let existing: HashSet<&str> = ["a", "c", "d"].into_iter().collect();
    assert!(s.drop_missing(&existing));
    assert_eq!(s.get(&g.key(), "b"), None);
    let placed = s.resolve(&g, &names(&["a", "c", "d", "e"]));
    assert_eq!(cells(&placed), [("a", Some((0, 0))), ("c", Some((0, 2))), ("d", Some((0, 3))), ("e", Some((0, 1)))], "d stays; the freed cell goes to the newcomer");
    assert!(!s.drop_missing(&existing), "nothing more to drop");
}

#[test]
fn a_file_that_comes_back_before_the_layout_is_written_keeps_its_place() {
    let g = grid();
    let mut s = Store::default();
    s.set(&g.key(), "a", Pos::Cell { col: 2, row: 3 });
    // An editor's save is a delete and a create: resolving with `a` absent changes nothing
    // stored, so resolving again with it back puts it where it was.
    let _ = s.resolve(&g, &names(&["b"]));
    assert_eq!(s.resolve(&g, &names(&["a"]))[0].cell, Some((2, 3)));
}

#[test]
fn renaming_keeps_the_place_at_every_size() {
    let mut s = Store::default();
    s.set("1920x1080", "old.txt", Pos::Cell { col: 4, row: 2 });
    s.set("1280x800", "old.txt", Pos::Free { x: 9, y: 9 });
    s.rename("old.txt", "new.txt");
    assert_eq!(s.get("1920x1080", "new.txt"), Some(Pos::Cell { col: 4, row: 2 }));
    assert_eq!(s.get("1280x800", "new.txt"), Some(Pos::Free { x: 9, y: 9 }));
    assert_eq!(s.get("1920x1080", "old.txt"), None);
}

#[test]
fn a_new_screen_size_starts_from_the_last_and_reflows_what_no_longer_fits() {
    let big = Grid::new(6 * 104 + 40, 6 * 116 + 16, 0, METRICS);
    let small = grid();
    let mut s = Store::default();
    s.set(&big.key(), "near", Pos::Cell { col: 0, row: 1 });
    s.set(&big.key(), "far", Pos::Cell { col: 5, row: 5 });
    s.set(&big.key(), "free", Pos::Free { x: 5000, y: 5000 });
    let placed = s.resolve(&small, &names(&["near", "far", "free"]));
    assert_eq!(placed[0].cell, Some((0, 1)), "a cell that still fits stays");
    assert_eq!(placed[1].cell, Some((0, 0)), "one that does not fit flows to the first free cell");
    assert!(placed[2].x <= small.w - METRICS.cell_w && placed[2].y <= small.h - METRICS.cell_h, "a free icon is pulled back on screen");
    assert_eq!(s.get(&big.key(), "far"), Some(Pos::Cell { col: 5, row: 5 }), "resolving does not rewrite the big size's layout");
}

#[test]
fn a_full_grid_cascades_instead_of_losing_icons() {
    let g = Grid::new(104 + 40, 116 + 16, 0, METRICS);
    assert_eq!(g.capacity(), 1);
    let placed = Store::default().resolve(&g, &names(&["a", "b", "c"]));
    assert_eq!(placed.len(), 3, "every icon is placed");
    assert_eq!(placed[0].cell, Some((0, 0)));
    assert!(placed[1].cell.is_none() && placed[2].cell.is_none());
    assert_ne!((placed[1].x, placed[1].y), (placed[2].x, placed[2].y));
}

#[test]
fn the_panel_takes_columns_but_not_the_layouts_key() {
    let free = Grid::new(6 * 104 + 40, 4 * 116 + 16, 0, METRICS);
    let covered = Grid::new(6 * 104 + 40, 4 * 116 + 16, 3 * 104, METRICS);
    assert_eq!((free.cols, covered.cols), (6, 3));
    assert_eq!(free.key(), covered.key(), "the same layout, so opening the panel rewrites nothing");
    let mut s = Store::default();
    s.set(&free.key(), "far", Pos::Cell { col: 5, row: 0 });
    let placed = s.resolve(&covered, &names(&["far"]));
    assert_eq!(placed[0].cell, Some((0, 0)), "drawn where there is room");
    assert_eq!(s.get(&free.key(), "far"), Some(Pos::Cell { col: 5, row: 0 }), "and still remembered where it was");
    assert!(!s.adopt(&covered.key(), &placed), "a displaced icon is not re-adopted over its saved place");
    assert!(covered.clamp_free(5000, 0).0 <= covered.usable_w - METRICS.cell_w);
}

#[test]
fn the_layout_round_trips_through_the_file() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join(".yantrik/desktop_layout.json");
    let mut s = Store { snap: false, icons: false, ..Store::default() };
    s.set("1920x1080", "a.txt", Pos::Cell { col: 1, row: 2 });
    s.set("1920x1080", "b.png", Pos::Free { x: 300, y: -4 });
    s.set("1280x800", "a.txt", Pos::Cell { col: 0, row: 0 });
    s.save(&path).unwrap();
    let back = Store::load(&path);
    assert!(!back.snap && !back.icons);
    assert_eq!(back.last.as_deref(), Some("1280x800"));
    assert_eq!(back.get("1920x1080", "a.txt"), Some(Pos::Cell { col: 1, row: 2 }));
    assert_eq!(back.get("1920x1080", "b.png"), Some(Pos::Free { x: 300, y: -4 }));
    assert_eq!(back.get("1280x800", "a.txt"), Some(Pos::Cell { col: 0, row: 0 }));
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("\"1920x1080\""), "per resolution, by name: {text}");
}

#[test]
fn a_save_leaves_no_temporary_file_and_replaces_a_link_rather_than_following_it() {
    let tmp = tempfile::tempdir().unwrap();
    let victim = tmp.path().join("victim");
    std::fs::write(&victim, "precious").unwrap();
    let path = tmp.path().join("desktop_layout.json");
    std::os::unix::fs::symlink(&victim, &path).unwrap();
    let mut s = Store::default();
    s.set("1x1", "a", Pos::Cell { col: 0, row: 0 });
    s.save(&path).unwrap();
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "precious", "the link's target is untouched");
    assert!(Store::load(&path).get("1x1", "a").is_some());
    let left: Vec<_> = std::fs::read_dir(tmp.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(left.len(), 2, "only the layout and the victim remain: {left:?}");
}

#[test]
fn a_planted_temporary_name_is_never_written_through() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("desktop_layout.json");
    let victim = tmp.path().join("victim");
    std::fs::write(&victim, "precious").unwrap();
    let tmp_name = tmp.path().join(format!(".desktop_layout.json.tmp{}", std::process::id()));
    std::os::unix::fs::symlink(&victim, &tmp_name).unwrap();
    let mut s = Store::default();
    assert!(s.save(&path).is_err(), "create_new refuses the planted name");
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "precious");
}

#[test]
fn a_file_that_is_not_a_layout_is_set_aside_not_destroyed() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("desktop_layout.json");
    std::fs::write(&path, "{ this is not json").unwrap();
    let mut s = Store::load(&path);
    assert_eq!(s.resolve(&grid(), &names(&["a"]))[0].cell, Some((0, 0)), "it starts as a new desktop");
    s.set("1x1", "a", Pos::Cell { col: 0, row: 0 });
    s.save(&path).unwrap();
    assert_eq!(std::fs::read_to_string(tmp.path().join("desktop_layout.json.corrupt")).unwrap(), "{ this is not json");
    assert!(Store::load(&path).get("1x1", "a").is_some());
}

#[test]
fn a_file_that_cannot_be_read_is_never_written_over() {
    let tmp = tempfile::tempdir().unwrap();
    // A directory where the file should be: opening it reads as an error that is not "not found".
    let path = tmp.path().join("desktop_layout.json");
    std::fs::create_dir(&path).unwrap();
    let mut s = Store::load(&path);
    s.set("1x1", "a", Pos::Cell { col: 0, row: 0 });
    assert!(matches!(s.save(&path), Err(LayoutError::Unreadable(_))));
    assert!(path.is_dir(), "what was there is still there");
}

#[test]
fn a_huge_file_is_not_read_whole_and_is_not_a_layout() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("desktop_layout.json");
    std::fs::write(&path, vec![b' '; (MAX_FILE_BYTES + 10) as usize]).unwrap();
    let s = Store::load(&path);
    assert!(s.corrupt);
}

#[test]
fn dropping_snaps_to_the_free_cell_nearest_where_it_was_let_go() {
    let g = grid();
    let mut taken = HashSet::new();
    let (x, y) = g.origin(1, 1);
    assert_eq!(drop_place(&g, &taken, x + 20, y - 15, true), Pos::Cell { col: 1, row: 1 });
    taken.insert((1, 1));
    let Pos::Cell { col, row } = drop_place(&g, &taken, x + 20, y - 15, true) else { panic!() };
    assert_eq!(col.abs_diff(1) + row.abs_diff(1), 1, "one cell away from the busy one");
    assert_eq!(drop_place(&g, &taken, 143, 77, false), Pos::Free { x: 143, y: 77 }, "snapping off keeps it exactly");
    assert_eq!(drop_place(&g, &taken, -50, 9000, false), Pos::Free { x: 0, y: g.h - METRICS.cell_h }, "but on the screen");
}

#[test]
fn turning_snapping_on_moves_free_icons_to_cells_without_stacking_them() {
    let g = grid();
    let placed = vec![
        Placed { name: "a".into(), x: 20, y: 8, cell: Some((0, 0)) },
        Placed { name: "b".into(), x: 22, y: 10, cell: None },
        Placed { name: "c".into(), x: 23, y: 12, cell: None },
    ];
    let moved = snap_all(&g, &placed);
    assert_eq!(moved.len(), 2);
    let mut seen: HashSet<(u32, u32)> = HashSet::from([(0, 0)]);
    for (_, pos) in moved {
        let Pos::Cell { col, row } = pos else { panic!("snapped") };
        assert!(seen.insert((col, row)), "no two on one cell");
    }
}

#[test]
fn arrange_sorts_column_first_by_name_type_or_date() {
    let g = grid();
    let t = |s: u64| Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(s));
    let items = vec![
        Item { name: "b.txt".into(), kind: Kind::Text, size: 0, mtime: t(10) },
        Item { name: "A.png".into(), kind: Kind::Image, size: 0, mtime: t(30) },
        Item { name: "dir".into(), kind: Kind::Folder, size: 0, mtime: t(20) },
    ];
    let order = |sort| arrange(&items, sort, &g).into_iter().map(|(n, _)| n).collect::<Vec<_>>();
    assert_eq!(order(Sort::Name), ["A.png", "b.txt", "dir"]);
    assert_eq!(order(Sort::Type), ["dir", "A.png", "b.txt"]);
    assert_eq!(order(Sort::Date), ["A.png", "dir", "b.txt"]);
    assert_eq!(arrange(&items, Sort::Name, &g)[1].1, Pos::Cell { col: 0, row: 1 });
}

#[test]
fn at_most_a_few_screen_sizes_are_remembered() {
    let mut s = Store::default();
    for i in 0..20 {
        s.set(&format!("{i}x{i}"), "a", Pos::Cell { col: 0, row: 0 });
    }
    assert!(s.layouts.len() <= MAX_LAYOUTS);
    assert_eq!(s.last.as_deref(), Some("19x19"));
    assert!(s.get("19x19", "a").is_some(), "the one in use is never the one evicted");
}
