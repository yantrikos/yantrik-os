//! What the desktop's right-click menus offer, and what each row means.
//!
//! Pure: a [`Context`] in, rows out, and a row's id back to a [`Pick`]. The shell draws the rows
//! with the kit's YContextMenu and acts on the pick; nothing here touches a window or the disk,
//! so what a menu offers in each state is tested by asking it.
//!
//! Words are plain labels and facts. A row for something that cannot happen is left out (Paste
//! when nothing is on the Files clipboard; Pin to dock for a file), not greyed, because a
//! disabled row says "this exists here" about something that does not.

use yantrik_bench::layout::Sort;

/// One row, before it is drawn.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Row {
    pub id: String,
    pub label: String,
    /// The key that does it, or the state a choice is in ("On").
    pub hint: String,
    pub separator_after: bool,
}

fn row(id: &str, label: &str) -> Row {
    Row { id: id.into(), label: label.into(), ..Row::default() }
}

fn hinted(id: &str, label: &str, hint: &str) -> Row {
    Row { hint: hint.into(), ..row(id, label) }
}

fn sep(mut r: Row) -> Row {
    r.separator_after = true;
    r
}

/// What a menu is asked on.
#[derive(Debug, Clone, Default)]
pub struct Context {
    /// How many icons are selected (the menu of an icon has at least one).
    pub selected: usize,
    pub only_folder: bool,
    pub only_app: bool,
    /// The one selected app shortcut resolves to an app that can be pinned, and whether it is.
    pub pinnable: bool,
    pub pinned: bool,
    /// Something is on the Files clipboard to paste.
    pub can_paste: bool,
    pub snap: bool,
    pub icons_shown: bool,
}

/// What a row asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick {
    NewFolder,
    NewFile,
    Paste,
    ArrangeMenu,
    Snap(bool),
    Sort(Sort),
    ChangeWallpaper,
    ToggleIcons,
    /// One of the shell's own entries, by the id `launch-app` takes.
    Launch(&'static str),
    Open,
    OpenWithMenu,
    OpenWith(String),
    Rename,
    Reveal,
    Archive,
    TogglePin,
    Back,
}

/// The menu of the bare desktop.
pub fn desktop(c: &Context) -> Vec<Row> {
    let mut rows = vec![row("new-folder", "New folder"), row("new-file", "New file")];
    if c.can_paste {
        rows.push(row("paste", "Paste"));
    }
    if let Some(last) = rows.pop() {
        rows.push(sep(last));
    }
    rows.push(row("arrange", "Arrange"));
    rows.push(sep(row("wallpaper", "Change wallpaper")));
    rows.push(sep(hinted("toggle-icons", "Show desktop icons", if c.icons_shown { "On" } else { "Off" })));
    rows.push(row("launch:terminal", "Open Terminal"));
    rows.push(sep(row("launch:files", "Open Files")));
    rows.push(row("launch:launchpad", "All Apps"));
    rows.push(sep(row("launch:devices", "Devices")));
    rows.push(row("launch:settings", "Settings"));
    rows
}

/// The menu of an icon (or of several): opening, then naming and finding, then putting away.
pub fn item(c: &Context) -> Vec<Row> {
    let one = c.selected == 1;
    let mut rows = vec![hinted("open", "Open", "Enter")];
    if one && !c.only_folder && !c.only_app {
        rows.push(row("open-with", "Open with"));
    }
    rows.last_mut().map(|r| r.separator_after = true);
    if one {
        rows.push(hinted("rename", "Rename", "F2"));
    }
    rows.push(row("reveal", "Reveal in Files"));
    if one && c.only_app && c.pinnable {
        rows.push(row("pin", if c.pinned { "Unpin from dock" } else { "Pin to dock" }));
    }
    rows.last_mut().map(|r| r.separator_after = true);
    rows.push(hinted("archive", "Archive", "Delete"));
    rows
}

/// Arrange's rows, with a way back to the menu it came from.
pub fn arrange(snap: bool) -> Vec<Row> {
    vec![
        sep(row("back", "Back")),
        hinted("keep", "Keep my placement", if snap { "" } else { "On" }),
        sep(hinted("snap", "Snap to grid", if snap { "On" } else { "" })),
        row("sort-name", "Sort by name"),
        row("sort-type", "Sort by type"),
        row("sort-date", "Sort by date"),
    ]
}

/// Open with's rows: the apps the one MIME rule offers, the default first.
pub fn open_with(apps: &[(String, String, bool)]) -> Vec<Row> {
    let mut rows = vec![sep(row("back", "Back"))];
    if apps.is_empty() {
        rows.push(row("none", "No other app opens this"));
    }
    rows.extend(apps.iter().map(|(id, name, default)| hinted(&format!("with:{id}"), name, if *default { "Default" } else { "" })));
    rows
}

/// A row's id as the thing it asks for. `None` for a row that asks for nothing.
pub fn pick(id: &str) -> Option<Pick> {
    if let Some(app) = id.strip_prefix("with:") {
        return Some(Pick::OpenWith(app.to_string()));
    }
    Some(match id {
        "new-folder" => Pick::NewFolder,
        "new-file" => Pick::NewFile,
        "paste" => Pick::Paste,
        "arrange" => Pick::ArrangeMenu,
        "keep" => Pick::Snap(false),
        "snap" => Pick::Snap(true),
        "sort-name" => Pick::Sort(Sort::Name),
        "sort-type" => Pick::Sort(Sort::Type),
        "sort-date" => Pick::Sort(Sort::Date),
        "wallpaper" => Pick::ChangeWallpaper,
        "toggle-icons" => Pick::ToggleIcons,
        "launch:terminal" => Pick::Launch("terminal"),
        "launch:files" => Pick::Launch("files"),
        "launch:launchpad" => Pick::Launch("launchpad"),
        "launch:devices" => Pick::Launch("devices"),
        "launch:settings" => Pick::Launch("settings"),
        "open" => Pick::Open,
        "open-with" => Pick::OpenWithMenu,
        "rename" => Pick::Rename,
        "reveal" => Pick::Reveal,
        "archive" => Pick::Archive,
        "pin" => Pick::TogglePin,
        "back" => Pick::Back,
        _ => return None,
    })
}

/// Whether a pick opens another menu in place of this one (the menu stays up).
pub fn is_submenu(p: &Pick) -> bool {
    matches!(p, Pick::ArrangeMenu | Pick::OpenWithMenu | Pick::Back)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(rows: &[Row]) -> Vec<&str> {
        rows.iter().map(|r| r.id.as_str()).collect()
    }

    fn desk() -> Context {
        Context { snap: true, icons_shown: true, ..Context::default() }
    }

    #[test]
    fn the_desktop_menu_has_what_the_issue_lists_and_the_old_entries() {
        let rows = desktop(&desk());
        assert_eq!(
            ids(&rows),
            ["new-folder", "new-file", "arrange", "wallpaper", "toggle-icons", "launch:terminal", "launch:files", "launch:launchpad", "launch:devices", "launch:settings"]
        );
    }

    #[test]
    fn paste_is_offered_only_when_there_is_something_to_paste() {
        assert!(!ids(&desktop(&desk())).contains(&"paste"));
        let with = desktop(&Context { can_paste: true, ..desk() });
        assert_eq!(ids(&with)[..3], ["new-folder", "new-file", "paste"]);
        assert!(with[2].separator_after && !with[1].separator_after);
    }

    #[test]
    fn show_desktop_icons_says_which_state_it_is_in() {
        let on = desktop(&desk());
        let off = desktop(&Context { icons_shown: false, ..desk() });
        let hint = |rows: &[Row]| rows.iter().find(|r| r.id == "toggle-icons").unwrap().hint.clone();
        assert_eq!((hint(&on).as_str(), hint(&off).as_str()), ("On", "Off"));
    }

    #[test]
    fn a_file_menu_opens_renames_reveals_and_archives() {
        let rows = item(&Context { selected: 1, ..desk() });
        assert_eq!(ids(&rows), ["open", "open-with", "rename", "reveal", "archive"]);
        assert_eq!(rows[0].hint, "Enter");
        assert_eq!(rows[2].hint, "F2");
        assert_eq!(rows[4].hint, "Delete");
        assert!(!ids(&rows).contains(&"pin"), "a file cannot be pinned to the dock");
    }

    #[test]
    fn a_folder_has_nothing_to_open_with_and_an_app_can_be_pinned() {
        let folder = item(&Context { selected: 1, only_folder: true, ..desk() });
        assert!(!ids(&folder).contains(&"open-with"));
        let app = item(&Context { selected: 1, only_app: true, pinnable: true, ..desk() });
        assert_eq!(ids(&app), ["open", "rename", "reveal", "pin", "archive"]);
        assert_eq!(app.iter().find(|r| r.id == "pin").unwrap().label, "Pin to dock");
        let pinned = item(&Context { selected: 1, only_app: true, pinnable: true, pinned: true, ..desk() });
        assert_eq!(pinned.iter().find(|r| r.id == "pin").unwrap().label, "Unpin from dock");
        let unknown = item(&Context { selected: 1, only_app: true, pinnable: false, ..desk() });
        assert!(!ids(&unknown).contains(&"pin"), "a shortcut to an app this machine does not have cannot be pinned");
    }

    #[test]
    fn several_icons_offer_what_can_be_done_to_all_of_them() {
        let rows = item(&Context { selected: 3, ..desk() });
        assert_eq!(ids(&rows), ["open", "reveal", "archive"]);
    }

    #[test]
    fn arrange_says_which_placement_is_on_and_has_a_way_back() {
        let snapping = arrange(true);
        assert_eq!(ids(&snapping), ["back", "keep", "snap", "sort-name", "sort-type", "sort-date"]);
        assert_eq!((snapping[1].hint.as_str(), snapping[2].hint.as_str()), ("", "On"));
        let free = arrange(false);
        assert_eq!((free[1].hint.as_str(), free[2].hint.as_str()), ("On", ""));
    }

    #[test]
    fn open_with_lists_the_apps_and_says_so_when_there_are_none() {
        let rows = open_with(&[("a.desktop".into(), "A".into(), true), ("b.desktop".into(), "B".into(), false)]);
        assert_eq!(ids(&rows), ["back", "with:a.desktop", "with:b.desktop"]);
        assert_eq!(rows[1].hint, "Default");
        assert_eq!(pick("with:b.desktop"), Some(Pick::OpenWith("b.desktop".into())));
        assert_eq!(ids(&open_with(&[])), ["back", "none"]);
    }

    #[test]
    fn every_row_a_menu_draws_means_something_but_the_note_that_nothing_opens() {
        let all = [
            desktop(&Context { can_paste: true, ..desk() }),
            item(&Context { selected: 1, only_app: true, pinnable: true, ..desk() }),
            item(&Context { selected: 1, ..desk() }),
            arrange(true),
            open_with(&[("x.desktop".into(), "X".into(), false)]),
        ];
        for rows in all {
            for r in rows {
                assert!(pick(&r.id).is_some() || r.id == "none", "`{}` asks for nothing", r.id);
            }
        }
        assert_eq!(pick("nonsense"), None);
    }

    #[test]
    fn only_arrange_open_with_and_back_keep_the_menu_up() {
        for id in ["arrange", "open-with", "back"] {
            assert!(is_submenu(&pick(id).unwrap()), "{id}");
        }
        for id in ["new-folder", "archive", "snap", "sort-name", "with:x.desktop", "launch:files"] {
            assert!(!is_submenu(&pick(id).unwrap()), "{id}");
        }
    }
}
