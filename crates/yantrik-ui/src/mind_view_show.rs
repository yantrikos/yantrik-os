//! How Mind View first appears on the person's desktop (#1347).
//!
//! Mind View is the desk where the person watches a Mind work, so it is shown when a Mind starts
//! working. #427's reason still stands: it must never take the person's keyboard or cover what
//! they are typing in. So the window is placed small, in the bottom-right corner above the dock, by
//! a window rule in the person's compositor (`config/labwc/rc.xml`, the rule titled "Mind View"),
//! and whoever held the keyboard when it mapped gets it straight back, as the on-screen display
//! does (`wire::osd`).
//!
//! labwc 0.8.3, the version on the image (Debian trixie), has no window-rule property that keeps a
//! window from being focused as it maps: `ignoreFocusRequest` stops a window *asking* to be
//! activated (labwc-config(5)), and a window rule's actions are the placement actions only. So the
//! keyboard is handed back after the map, to the window that held it when Mind View mapped: the
//! holder is read every 100 ms while the window is awaited, and the last reading before the map is
//! the one given back to. If that window closed or was minimised meanwhile, or cannot be brought
//! forward, Mind View is minimised instead: it is never left holding a keyboard the person did not
//! give it. Only the person's press on the bar's chip focuses it ([`bring_forward`]).
//!
//! Every `wlrctl` call here has a deadline (`wlrctl_bounded`).
//!
//! Only the first map is acted on. A window the person minimised stays minimised, and one they
//! closed is not put back in front by a later turn: [`note_gone`] is told when the person's close
//! (or its death) was noticed, and the next Mind View then starts out of the way, as it did
//! before this change, until the person asks for it (the bar's "working" chip, Super+Shift+V or
//! the taskbar).

use crate::wlrctl_bounded::wlrctl;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Mind View's window, as the person's compositor names it: labwc's app id, and the title the
/// title library gives it (crates/yantrik-mind-view-title). The window rule in config/labwc/rc.xml
/// matches the same two.
const WINDOW_MATCH: [&str; 2] = ["app_id:labwc", "title:Mind View"];

/// How long the keyboard is left with the new window before it is handed back: long enough for
/// the compositor to have focused it, the same wait the on-screen display uses.
const REFOCUS_AFTER: Duration = Duration::from_millis(150);

/// How often the keyboard's holder is read while the window is awaited.
const POLL: Duration = Duration::from_millis(100);

/// Whether the person closed Mind View, or it died, since the shell started.
static GONE: AtomicBool = AtomicBool::new(false);

/// Set while the person's press on the chip is starting Mind View: the keyboard is theirs to give,
/// so the new window keeps it.
static ASKED_FOR: AtomicBool = AtomicBool::new(false);

/// Where Mind View lands when it maps.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Landing {
    /// In the corner, shown, with the keyboard given back.
    Corner,
    /// Minimised: the person closed the last one, and a later turn does not reopen it.
    Aside,
}

fn landing(gone_before: bool) -> Landing {
    if gone_before {
        Landing::Aside
    } else {
        Landing::Corner
    }
}

/// The running Mind View is gone: the person closed it or it died. The next one starts aside.
pub fn note_gone() {
    GONE.store(true, Ordering::Relaxed);
}

/// What became of the window that held the keyboard when Mind View mapped.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Holder {
    Open,
    Gone,
    Minimised,
    /// The compositor did not say (no `wlrctl`, or no answer in time).
    Unknown,
}

/// What is done to Mind View once it has mapped.
#[derive(Clone, Debug, PartialEq)]
enum Action {
    /// Leave it: the person asked for it with the chip.
    Keep,
    /// Minimise it: it is not to be shown.
    SetAside,
    /// Give the keyboard back to this window.
    HandBack(String),
    /// There is no window to give the keyboard to: minimise Mind View so that it does not hold it.
    Release(&'static str),
}

fn decide(landing: Landing, asked_for: bool, held: Option<&str>, holder_now: impl FnOnce(&str) -> Holder) -> Action {
    if asked_for {
        return Action::Keep;
    }
    if landing == Landing::Aside {
        return Action::SetAside;
    }
    let Some(title) = held else {
        return Action::Release("nothing was known to hold the keyboard before it mapped");
    };
    match holder_now(title) {
        Holder::Open => Action::HandBack(title.to_string()),
        Holder::Gone => Action::Release("the window that held the keyboard has closed"),
        Holder::Minimised => Action::Release("the window that held the keyboard was minimised meanwhile"),
        Holder::Unknown => Action::Release("the compositor did not say whether the keyboard's window is still open"),
    }
}

/// The compositor, as far as placing Mind View asks of it. A trait so the decision and the order
/// of the calls run in tests without one.
trait Compositor {
    /// The window holding the keyboard now (`windows::focus_to_restore`).
    fn focused(&self) -> Option<String>;
    fn mind_view_mapped(&self) -> bool;
    fn holder(&self, title: &str) -> Holder;
    /// Bring a window forward as it is (`windows::present`).
    fn present(&self, title: &str) -> bool;
    fn minimise_mind_view(&self) -> bool;
}

struct Wlrctl;

fn found(spec: &[&str]) -> Option<bool> {
    let mut args = vec!["toplevel", "find"];
    args.extend_from_slice(spec);
    wlrctl(&args).ok().flatten().map(|o| o.status.success())
}

impl Compositor for Wlrctl {
    fn focused(&self) -> Option<String> {
        crate::windows::focus_to_restore()
    }
    fn mind_view_mapped(&self) -> bool {
        found(&WINDOW_MATCH) == Some(true)
    }
    fn holder(&self, title: &str) -> Holder {
        let title = format!("title:{title}");
        match found(&[&title]) {
            None => Holder::Unknown,
            Some(false) => Holder::Gone,
            Some(true) => match found(&[&title, "state:minimized"]) {
                Some(false) => Holder::Open,
                Some(true) => Holder::Minimised,
                None => Holder::Unknown,
            },
        }
    }
    fn present(&self, title: &str) -> bool {
        crate::windows::present(title)
    }
    fn minimise_mind_view(&self) -> bool {
        let mut args = vec!["toplevel", "minimize"];
        args.extend_from_slice(&WINDOW_MATCH);
        wlrctl(&args).ok().flatten().is_some_and(|o| o.status.success())
    }
}

/// Place Mind View as it first appears. Call it before the nested compositor is started, so that
/// the keyboard's holder is read from before the window can exist. Off the UI thread: it waits for
/// the window, and wlrctl is a process.
pub fn when_mapped(wait: Duration) {
    let landing = landing(GONE.load(Ordering::Relaxed));
    std::thread::spawn(move || place(&Wlrctl, landing, wait, POLL, REFOCUS_AFTER, &ASKED_FOR));
}

fn place(c: &dyn Compositor, landing: Landing, wait: Duration, poll: Duration, settle: Duration, asked_for: &AtomicBool) {
    let deadline = Instant::now() + wait;
    let mut held: Option<String> = None;
    while Instant::now() < deadline {
        // Before the window can have taken it: the last reading before the map is who had it then.
        if let Some(title) = c.focused() {
            if title != crate::mind_view::TITLE {
                held = Some(title);
            }
        }
        if c.mind_view_mapped() {
            std::thread::sleep(settle);
            let action = decide(landing, asked_for.swap(false, Ordering::Relaxed), held.as_deref(), |t| c.holder(t));
            match action {
                Action::Keep => {}
                Action::SetAside => {
                    if !c.minimise_mind_view() {
                        tracing::warn!("Mind View appeared but could not be set aside; it may be over the person's work");
                    }
                }
                Action::HandBack(title) => {
                    if !c.present(&title) {
                        tracing::warn!(window = %title, "the keyboard could not be given back; Mind View is minimised so it does not hold it");
                        c.minimise_mind_view();
                    }
                }
                Action::Release(why) => {
                    tracing::warn!("{why}; Mind View is minimised so it does not hold the keyboard");
                    c.minimise_mind_view();
                }
            }
            return;
        }
        std::thread::sleep(poll);
    }
    tracing::debug!("Mind View's window did not appear in time to be placed");
}

/// Bring Mind View forward and give it the keyboard: the person pressed the bar's chip, which is
/// their own act and the only thing that focuses it. Restores it first if it was minimised, and
/// starts it if there is no window (the chip can show for a turn that opened no app). A process
/// and a start: off the UI thread.
pub fn bring_forward() {
    std::thread::spawn(|| {
        if crate::windows::present_app(crate::mind_view::APP_ID) {
            return;
        }
        ASKED_FOR.store(true, Ordering::Relaxed);
        if let Err(why) = crate::mind_view::ensure() {
            ASKED_FOR.store(false, Ordering::Relaxed);
            tracing::warn!(reason = %why, "Mind View could not be started from the chip");
            return;
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline && !Wlrctl.mind_view_mapped() {
            std::thread::sleep(POLL);
        }
        if !crate::windows::present_app(crate::mind_view::APP_ID) {
            tracing::warn!("Mind View was started from the chip but has no window to bring forward");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn a_first_mind_view_is_shown_and_one_the_person_closed_is_not_reopened() {
        assert_eq!(landing(false), Landing::Corner);
        assert_eq!(landing(true), Landing::Aside);
    }

    #[test]
    fn the_window_is_matched_by_the_names_the_rule_and_the_title_library_use() {
        assert!(WINDOW_MATCH.contains(&"title:Mind View"));
        assert_eq!(crate::mind_view::TITLE, "Mind View");
    }

    /// The rule that puts the window in the bottom-right corner is for labwc's own windows titled
    /// as Mind View's is, and for no other: a page or app that sets the same title on the person's
    /// desktop is not placed by it.
    #[test]
    fn the_rule_is_for_labwcs_own_window_and_places_it_in_the_corner() {
        let rc = include_str!("../../../config/labwc/rc.xml");
        let title = format!(r#"title="{}""#, crate::mind_view::TITLE);
        let start = rc.match_indices("<windowRule ").map(|(i, _)| i).find(|&i| rc[i..].split('>').next().unwrap().contains(&title)).expect("a window rule for Mind View");
        let tag = rc[start..].split('>').next().unwrap();
        let app_id = WINDOW_MATCH[0].strip_prefix("app_id:").unwrap();
        assert!(tag.contains(&format!(r#"identifier="{app_id}""#)), "restricted to labwc's app id: {tag}");
        let rule = &rc[start..start + rc[start..].find("</windowRule>").unwrap()];
        assert!(rule.contains(r#"direction="right""#) && rule.contains(r#"direction="down""#) && rule.contains("MoveRelative"));
        assert!(!rule.contains("Iconify") && !rule.contains("ToggleAlwaysOnTop"), "shown, and not over what is being typed in");
    }

    fn open(_: &str) -> Holder {
        Holder::Open
    }

    #[test]
    fn the_keyboard_goes_back_to_the_window_that_held_it_and_only_if_that_is_still_there() {
        assert_eq!(decide(Landing::Corner, false, Some("Notes"), open), Action::HandBack("Notes".into()));
        for (now, _) in [(Holder::Gone, "closed"), (Holder::Minimised, "minimised"), (Holder::Unknown, "unknown")] {
            assert!(matches!(decide(Landing::Corner, false, Some("Notes"), |_| now), Action::Release(_)), "{now:?}");
        }
        assert!(matches!(decide(Landing::Corner, false, None, open), Action::Release(_)), "nobody known to hold it");
        assert_eq!(decide(Landing::Aside, false, Some("Notes"), open), Action::SetAside);
        assert_eq!(decide(Landing::Corner, true, Some("Notes"), open), Action::Keep, "the person pressed the chip");
        assert_eq!(decide(Landing::Aside, true, None, open), Action::Keep);
    }

    /// A compositor that says what the test scripts it to, and keeps what it was asked.
    struct Fake {
        /// What `focused` answers, one per reading; the last repeats.
        focus: RefCell<Vec<Option<String>>>,
        mapped_after: usize,
        reads: RefCell<usize>,
        holder: Holder,
        presents: bool,
        calls: RefCell<Vec<String>>,
    }

    impl Fake {
        fn new(focus: &[Option<&str>], mapped_after: usize, holder: Holder, presents: bool) -> Fake {
            Fake { focus: RefCell::new(focus.iter().map(|f| f.map(String::from)).collect()), mapped_after, reads: RefCell::new(0), holder, presents, calls: RefCell::new(vec![]) }
        }
        fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
    }

    impl Compositor for Fake {
        fn focused(&self) -> Option<String> {
            let mut f = self.focus.borrow_mut();
            if f.len() > 1 { f.remove(0) } else { f[0].clone() }
        }
        fn mind_view_mapped(&self) -> bool {
            *self.reads.borrow_mut() += 1;
            *self.reads.borrow() > self.mapped_after
        }
        fn holder(&self, _: &str) -> Holder {
            self.holder
        }
        fn present(&self, title: &str) -> bool {
            self.calls.borrow_mut().push(format!("present {title}"));
            self.presents
        }
        fn minimise_mind_view(&self) -> bool {
            self.calls.borrow_mut().push("minimise".into());
            true
        }
    }

    fn run(c: &Fake, landing: Landing, asked: bool) {
        let asked = AtomicBool::new(asked);
        place(c, landing, Duration::from_secs(5), Duration::from_millis(1), Duration::ZERO, &asked);
        assert!(!asked.load(Ordering::Relaxed), "the request is used up by the map it was for");
    }

    /// The person switched windows while Mind View was starting: the one that has the keyboard
    /// when it maps gets it back, not the one that had it when the start began.
    #[test]
    fn the_window_read_last_before_the_map_is_the_one_given_back_to() {
        let c = Fake::new(&[Some("Notes"), Some("Terminal"), Some("Browser")], 2, Holder::Open, true);
        run(&c, Landing::Corner, false);
        assert_eq!(c.calls(), ["present Browser"]);
    }

    #[test]
    fn mind_view_itself_is_never_the_one_given_the_keyboard() {
        let c = Fake::new(&[Some("Notes"), Some("Mind View")], 2, Holder::Open, true);
        run(&c, Landing::Corner, false);
        assert_eq!(c.calls(), ["present Notes"]);
    }

    #[test]
    fn a_window_that_closed_or_was_minimised_or_cannot_be_shown_leaves_the_keyboard_with_no_one_new() {
        for holder in [Holder::Gone, Holder::Minimised, Holder::Unknown] {
            let c = Fake::new(&[Some("Notes")], 1, holder, true);
            run(&c, Landing::Corner, false);
            assert_eq!(c.calls(), ["minimise"], "{holder:?}");
        }
        let c = Fake::new(&[None], 1, Holder::Open, true);
        run(&c, Landing::Corner, false);
        assert_eq!(c.calls(), ["minimise"], "no holder known (the lock screen, or no wlrctl)");
        let c = Fake::new(&[Some("Notes")], 1, Holder::Open, false);
        run(&c, Landing::Corner, false);
        assert_eq!(c.calls(), ["present Notes", "minimise"], "present failed");
    }

    #[test]
    fn a_mind_view_the_person_closed_starts_aside_and_one_they_asked_for_keeps_the_keyboard() {
        let c = Fake::new(&[Some("Notes")], 1, Holder::Open, true);
        run(&c, Landing::Aside, false);
        assert_eq!(c.calls(), ["minimise"]);
        let c = Fake::new(&[Some("Notes")], 1, Holder::Open, true);
        run(&c, Landing::Corner, true);
        assert!(c.calls().is_empty());
    }

    #[test]
    fn a_window_that_never_appears_is_given_up_on_at_the_deadline() {
        let c = Fake::new(&[Some("Notes")], usize::MAX, Holder::Open, true);
        let started = Instant::now();
        place(&c, Landing::Corner, Duration::from_millis(100), Duration::from_millis(5), Duration::ZERO, &AtomicBool::new(false));
        assert!(started.elapsed() < Duration::from_secs(2) && c.calls().is_empty());
    }

    /// The chip's press is the only thing that gives Mind View the keyboard: nothing a Mind can call
    /// (the control actions) or press for (the accessibility service) reaches `bring_forward`
    /// except through the Slint callback of the chip.
    #[test]
    fn bring_forward_is_reached_only_from_the_chips_slint_callback() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut callers = Vec::new();
        let mut stack = vec![src.clone()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let text = std::fs::read_to_string(&path).unwrap();
                    let code = text.split("#[cfg(test)]").next().unwrap_or("");
                    if code.lines().any(|l| !l.trim_start().starts_with("//") && l.contains("mind_view_show::bring_forward")) {
                        callers.push(path.strip_prefix(&src).unwrap().display().to_string());
                    }
                }
            }
        }
        assert_eq!(callers, ["wire/agents_working.rs"], "control actions and the accessibility path must not focus Mind View");
        let wire = std::fs::read_to_string(src.join("wire/agents_working.rs")).unwrap();
        assert!(wire.contains("on_pressed(crate::mind_view_show::bring_forward)"), "only the Slint callback");
    }
}
