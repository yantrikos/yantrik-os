//! What the grounded dock holds, and in which order (design/minds-surfaces-spec §3).
//!
//! The dock draws APPS, not windows: a pinned app, an app that is running, or both, with a count
//! when it has several windows. The old taskbar drew one entry per window, so a person with eleven
//! terminals had eleven entries and no room for anything else. Everything here is pure — no
//! Slint, no compositor — so the order, the grouping and the paging are tested without a screen.
//!
//! The paging itself (how many buttons fit) is done by the component, which knows the window's
//! width; [`page`] is the same arithmetic for `describe shell`, so a caller is told the page the
//! person is looking at.

use crate::control::SCREEN_ENTRY_PREFIX;

/// One window as the dock sees it: its title, and the app it belongs to.
#[derive(Clone, Debug, PartialEq)]
pub struct Win {
    pub title: String,
    pub app_id: String,
    /// What the window calls itself where its title is not the name to show (Mind View).
    pub subtitle: String,
}

/// An app the person pinned: its id (`pins::pin_id`) and the name to show.
#[derive(Clone, Debug, PartialEq)]
pub struct Pin {
    pub app_id: String,
    pub label: String,
}

/// One button of the dock's middle.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub app_id: String,
    pub label: String,
    pub pinned: bool,
    /// Titles, most recently used first. Empty: not running.
    pub windows: Vec<String>,
    /// The window in front is one of this app's.
    pub focused: bool,
}

impl Entry {
    pub fn running(&self) -> bool {
        !self.windows.is_empty()
    }
}

/// The id a window's app goes by in the dock: a screen of the shell itself (`shell:files`) is the
/// Files app, so Files in the shell and Files as a window are one button, not two.
pub fn dock_id(window_app_id: &str) -> &str {
    window_app_id.strip_prefix(SCREEN_ENTRY_PREFIX).unwrap_or(window_app_id)
}

/// The order apps were first seen running, so a running app that is not pinned keeps its place
/// while other windows come and go. "Pinned apps first, then running apps in launch order" — and
/// never in focus order, which would shuffle the dock under the pointer every time a window
/// is clicked.
#[derive(Default, Debug)]
pub struct LaunchOrder(Vec<String>);

impl LaunchOrder {
    /// Take note of who is running now: newcomers go last, those who left are forgotten.
    fn observe(&mut self, running: &[&str]) {
        self.0.retain(|id| running.contains(&id.as_str()));
        for id in running {
            if !self.0.iter().any(|seen| seen == id) {
                self.0.push((*id).to_string());
            }
        }
    }

    fn position(&self, id: &str) -> usize {
        self.0.iter().position(|seen| seen == id).unwrap_or(usize::MAX)
    }
}

/// The dock's middle: the pins in the order the person made them, then every other running app in
/// launch order. `wins` is the compositor's order — the window in front first — and `front` is the
/// app whose window that is.
///
/// `name_of` gives the label for an app that is running but not pinned.
pub fn entries(
    pins: &[Pin],
    wins: &[Win],
    front: Option<&str>,
    order: &mut LaunchOrder,
    name_of: impl Fn(&str, &[Win]) -> String,
) -> Vec<Entry> {
    // Windows grouped by app, in the order given, keeping each app's first appearance.
    let mut running: Vec<(&str, Vec<&Win>)> = Vec::new();
    for w in wins {
        let id = dock_id(&w.app_id);
        match running.iter_mut().find(|(seen, _)| *seen == id) {
            Some((_, group)) => group.push(w),
            None => running.push((id, vec![w])),
        }
    }
    let ids: Vec<&str> = running.iter().map(|(id, _)| *id).collect();
    order.observe(&ids);

    let titles_of = |id: &str| -> Vec<String> {
        running
            .iter()
            .find(|(seen, _)| *seen == id)
            .map(|(_, group)| group.iter().map(|w| w.title.clone()).collect())
            .unwrap_or_default()
    };
    let focused = |id: &str| front.is_some_and(|f| dock_id(f) == id);

    let mut out: Vec<Entry> = pins
        .iter()
        .map(|p| Entry {
            app_id: p.app_id.clone(),
            label: p.label.clone(),
            pinned: true,
            windows: titles_of(&p.app_id),
            focused: focused(&p.app_id),
        })
        .collect();

    let mut others: Vec<&str> = ids.iter().copied().filter(|id| !pins.iter().any(|p| p.app_id == *id)).collect();
    others.sort_by_key(|id| order.position(id));
    for id in others {
        let group = &running.iter().find(|(seen, _)| *seen == id).expect("it was just listed").1;
        let owned: Vec<Win> = group.iter().map(|w| (*w).clone()).collect();
        out.push(Entry {
            app_id: id.to_string(),
            label: name_of(id, &owned),
            pinned: false,
            windows: group.iter().map(|w| w.title.clone()).collect(),
            focused: focused(id),
        });
    }
    out
}

/// Which page `describe shell` reports: the buttons on screen and what is out of sight either side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Page {
    /// Index of the first button shown.
    pub first: usize,
    /// How many are shown.
    pub shown: usize,
    /// Apps out of sight before and after ("‹ +2", "› +6").
    pub before: usize,
    pub after: usize,
}

/// The page a dock of `total` buttons is showing, with room for `capacity` of them and the page
/// starting at `first`. The component clamps the same way: the last page is full rather than
/// ending in a gap.
pub fn page(total: usize, capacity: usize, first: usize) -> Page {
    if capacity >= total {
        return Page { first: 0, shown: total, before: 0, after: 0 };
    }
    let capacity = capacity.max(1);
    let first = first.min(total - capacity);
    Page { first, shown: capacity, before: first, after: total - first - capacity }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(title: &str, app: &str) -> Win {
        Win { title: title.into(), app_id: app.into(), subtitle: String::new() }
    }
    fn pin(app: &str) -> Pin {
        Pin { app_id: app.into(), label: app.to_uppercase() }
    }
    fn build(pins: &[Pin], wins: &[Win], front: Option<&str>, order: &mut LaunchOrder) -> Vec<Entry> {
        entries(pins, wins, front, order, |id, _| id.to_string())
    }

    /// Eleven terminals are one button with a count, not eleven entries (the taskbar's failure).
    #[test]
    fn several_windows_of_one_app_are_one_button() {
        let wins: Vec<Win> = (0..3).map(|i| win(&format!("Terminal {i}"), "terminal")).collect();
        let got = build(&[pin("files"), pin("terminal")], &wins, Some("terminal"), &mut LaunchOrder::default());
        assert_eq!(got.len(), 2);
        assert_eq!(got[1].app_id, "terminal");
        assert_eq!(got[1].windows.len(), 3);
        assert!(got[1].focused && got[1].running());
        assert!(!got[0].running() && !got[0].focused, "a pin that is not open is just a pin");
    }

    /// The pins come first in the person's order; the rest follow in the order they were launched,
    /// and clicking another window never reshuffles them.
    #[test]
    fn pinned_first_then_running_in_launch_order_not_focus_order() {
        let mut order = LaunchOrder::default();
        let pins = [pin("files"), pin("notes")];
        let first = build(&pins, &[win("B", "blender"), win("G", "gimp")], Some("blender"), &mut order);
        let ids: Vec<&str> = first.iter().map(|e| e.app_id.as_str()).collect();
        assert_eq!(ids, ["files", "notes", "blender", "gimp"]);
        // gimp comes to the front: the compositor now lists it first. The dock does not move.
        let second = build(&pins, &[win("G", "gimp"), win("B", "blender")], Some("gimp"), &mut order);
        let ids: Vec<&str> = second.iter().map(|e| e.app_id.as_str()).collect();
        assert_eq!(ids, ["files", "notes", "blender", "gimp"]);
        assert!(second[3].focused && !second[2].focused);
        // blender closes; gimp keeps its place and a newcomer goes last.
        let third = build(&pins, &[win("G", "gimp"), win("K", "krita")], Some("krita"), &mut order);
        let ids: Vec<&str> = third.iter().map(|e| e.app_id.as_str()).collect();
        assert_eq!(ids, ["files", "notes", "gimp", "krita"]);
    }

    /// Files as one of the shell's own screens and Files as a window are the same app.
    #[test]
    fn a_shell_screen_is_its_app() {
        assert_eq!(dock_id("shell:files"), "files");
        assert_eq!(dock_id("blender"), "blender");
        let got = build(&[pin("files")], &[win("Files", "shell:files")], Some("shell:files"), &mut LaunchOrder::default());
        assert_eq!(got.len(), 1, "one Files button, not a pin and a window");
        assert!(got[0].running() && got[0].focused && got[0].pinned);
    }

    /// An empty desktop is just the pins; nothing is running and nothing is focused.
    #[test]
    fn nothing_open_is_just_the_pins() {
        let got = build(&[pin("files")], &[], None, &mut LaunchOrder::default());
        assert_eq!(got.len(), 1);
        assert!(!got[0].running() && !got[0].focused);
    }

    /// 20 apps in room for 16: the first page is 16 with "› +4", and the last page is full.
    #[test]
    fn paging_reports_what_is_out_of_sight() {
        assert_eq!(page(5, 17, 0), Page { first: 0, shown: 5, before: 0, after: 0 });
        assert_eq!(page(20, 16, 0), Page { first: 0, shown: 16, before: 0, after: 4 });
        let last = page(20, 16, 16);
        assert_eq!(last, Page { first: 4, shown: 16, before: 4, after: 0 }, "the last page is full, not 4 and a gap");
        assert_eq!(page(20, 16, 99), last);
        assert_eq!(page(3, 0, 0).shown, 1, "never zero slots");
    }
}
