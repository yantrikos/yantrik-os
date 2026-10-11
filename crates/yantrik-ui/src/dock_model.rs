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

/// What a mind's desk (the Mind View window) is called on the dock: what Mind View calls it, or
/// "Mind View" when it says nothing.
pub fn mind_view_label(group: &[Win]) -> String {
    group.iter().map(|w| w.subtitle.clone()).find(|s| !s.is_empty()).unwrap_or_else(|| "Mind View".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mind_view_is_labelled_by_the_mind_or_by_its_own_name() {
        let mut named = win("Mind View", "mind-view");
        named.subtitle = "pi".into();
        assert_eq!(mind_view_label(&[win("Mind View", "mind-view"), named]), "pi");
        assert_eq!(mind_view_label(&[win("Mind View", "mind-view")]), "Mind View");
    }

    /// The dock's bottom edge and the compositor's reserved strip are two numbers in two files:
    /// if they drift, maximised windows slide under the dock or leave a gap above it (#585 S4).
    #[test]
    fn the_compositor_margin_is_the_docks_height() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let rc = std::fs::read_to_string(root.join("config/labwc/rc.xml")).unwrap();
        let at = rc.find("<margin ").expect("rc.xml reserves the shell's strips");
        let tag = &rc[at..at + rc[at..].find("/>").unwrap()];
        let bottom: f32 = tag.split("bottom=\"").nth(1).unwrap().split('"').next().unwrap().parse().unwrap();
        let theme = std::fs::read_to_string(root.join("crates/yantrik-design-tokens/slint/theme.slint")).unwrap();
        let token = theme.lines().find(|l| l.contains("out property <length> taskbar-height:")).expect("the token");
        let px: f32 = token.split(':').nth(1).unwrap().trim().trim_end_matches(';').trim_end_matches("px").parse().unwrap();
        assert_eq!(bottom, px, "rc.xml <margin bottom> must equal Theme.taskbar-height");
        let status = theme.lines().find(|l| l.contains("out property <length> status-bar-height:")).expect("status token");
        let top: f32 = tag.split("top=\"").nth(1).unwrap().split('"').next().unwrap().parse().unwrap();
        assert!(status.contains(&format!("{top}px")), "and <margin top> the status bar's");
    }

    /// The attribute that avoids the rustc ICE sits on `mod windows;` itself, not on whatever
    /// module was added above it (#585 B1).
    #[test]
    fn the_dead_code_allowance_guards_windows() {
        let main = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/main.rs")).unwrap();
        let lines: Vec<&str> = main.lines().collect();
        let at = lines.iter().position(|l| l.trim() == "mod windows;").expect("mod windows;");
        assert_eq!(lines[at - 1].trim(), "#[allow(dead_code)]", "the attribute is on the line above `mod windows;`");
        let d = lines.iter().position(|l| l.trim() == "mod dock_model;").expect("mod dock_model;");
        assert!(d < at - 3, "and dock_model sits above the note, not between it and windows");
    }

    /// The dock's tiles are the spec's 32px (§3, changed from 24px on 3 October: a 24px tile left a
    /// 13px glyph, "not crisp but blunt"), and the token and the spec say the same (#585 S1).
    #[test]
    fn dock_icons_are_the_specs_32px() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let theme = std::fs::read_to_string(root.join("../yantrik-design-tokens/slint/theme.slint")).unwrap();
        let line = theme.lines().find(|l| l.contains("out property <length> dock-icon:")).unwrap();
        assert!(line.contains(": 32px;"), "{line}");
        assert!(!theme.contains("spec allows 24-36px"));
        let spec = std::fs::read_to_string(root.join("../../design/minds-surfaces-spec-2026-10-02.md")).unwrap();
        assert!(spec.contains("40×40 buttons with 32px app tiles"), "the spec says 32px too");
    }

    /// One wheel area under the whole bar, and every dock button forwards the wheel: a button's own
    /// area accepts the scroll, so one that did not forward it would swallow it (#585 S3).
    #[test]
    fn the_wheel_pages_over_the_whole_bar() {
        let src = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../yantrik-ui-slint/ui/components/grounded_dock.slint")).unwrap();
        assert!(src.contains("wheel-bar := YWheelArea"), "a wheel area under the bar");
        let buttons = src.matches("DockBtn {").count();
        assert_eq!(src.matches("wheel(n) => { root.page-by(n); }").count(), buttons + 1, "{buttons} buttons and the bar each page");
    }

    /// "1 windows" (#585 N2).
    #[test]
    fn the_list_header_says_one_window() {
        let src = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../yantrik-ui-slint/ui/components/grounded_dock.slint")).unwrap();
        assert!(src.contains(r#"root.list-count == 1 ? "1 window""#));
    }

    /// The mind is not an app, so its button is not a tile: the ring in the mind's teal on no
    /// fill, at most a 1px teal edge, where it was a solid teal tile beside the app tiles.
    #[test]
    fn the_mind_button_is_a_glyph_not_a_tile() {
        let src = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../yantrik-ui-slint/ui/components/grounded_dock.slint")).unwrap();
        let mind = &src[src.find("// ── Yantrik Mind ──").expect("the mind button")..];
        let mind = &mind[..mind.find("// ── Show desktop ──").expect("the next section")];
        assert!(mind.contains("DockBtn {"), "the same 40px button, so the same hit area and label");
        assert!(mind.contains("width: Theme.dock-icon;") && mind.contains("height: Theme.dock-icon;"), "the same 32px footprint");
        assert!(mind.contains("background: transparent;"), "no fill");
        assert!(mind.contains("border-width: 1px;") && !mind.contains("border-width: 2px"), "at most a 1px edge");
        assert!(mind.contains("Theme.mind :") && mind.contains("commands: Icons.companion;"), "the ring in the mind's teal");
        assert!(!mind.contains("background: root.companion-online"), "not a teal tile");
    }

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
