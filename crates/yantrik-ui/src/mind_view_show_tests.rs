//! Tests for `mind_view_show`.

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
    for now in [Holder::Minimised, Holder::Unknown] {
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
    /// What `forward` answers, and whether `ensure` starts anything.
    forwards: bool,
    /// `forward`'s answers in order when set (the last repeats), before `forwards`.
    forward_seq: RefCell<Vec<bool>>,
    /// How many readings `placing` says a placement is still running, and the request flag as it
    /// stood at each one.
    placing_for: RefCell<usize>,
    watch: Option<std::sync::Arc<AtomicBool>>,
    calls: RefCell<Vec<String>>,
}

impl Fake {
    fn new(focus: &[Option<&str>], mapped_after: usize, holder: Holder, presents: bool) -> Fake {
        Fake { focus: RefCell::new(focus.iter().map(|f| f.map(String::from)).collect()), mapped_after, reads: RefCell::new(0), holder, presents, forwards: true, forward_seq: RefCell::new(vec![]), placing_for: RefCell::new(0), watch: None, calls: RefCell::new(vec![]) }
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
    fn forward(&self) -> bool {
        self.calls.borrow_mut().push("forward".into());
        let mut seq = self.forward_seq.borrow_mut();
        if seq.len() > 1 { seq.remove(0) } else { seq.first().copied().unwrap_or(self.forwards) }
    }
    fn ensure(&self) -> Result<(), String> {
        self.calls.borrow_mut().push("ensure".into());
        Ok(())
    }
    fn minimise_mind_view(&self) -> bool {
        self.calls.borrow_mut().push("minimise".into());
        true
    }
    fn placing(&self) -> bool {
        let mut left = self.placing_for.borrow_mut();
        let asked = self.watch.as_ref().map(|w| w.load(Ordering::Relaxed));
        self.calls.borrow_mut().push(format!("placing asked={asked:?}"));
        if *left == 0 {
            return false;
        }
        *left -= 1;
        true
    }
}

fn run(c: &Fake, landing: Landing, asked: bool) {
    let asked = AtomicBool::new(asked);
    place(c, landing, Duration::from_secs(5), Duration::from_millis(1), Duration::ZERO, &asked, &AtomicBool::new(false));
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
    for holder in [Holder::Minimised, Holder::Unknown] {
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
    place(&c, Landing::Corner, Duration::from_millis(100), Duration::from_millis(5), Duration::ZERO, &AtomicBool::new(false), &AtomicBool::new(false));
    assert!(started.elapsed() < Duration::from_secs(2) && c.calls().is_empty());
}

/// M2: after a press whose `present` failed while Mind View was already running, no start
/// consumed the request; a start a Mind triggers later must not find it and keep the keyboard.
#[test]
fn a_press_that_failed_leaves_no_request_for_a_later_start_to_find() {
    let mut c = Fake::new(&[Some("Notes")], 1, Holder::Open, true);
    c.forwards = false; // nothing to bring forward: present failed
    let asked = AtomicBool::new(false);
    serve_press(&c, &asked, Duration::from_millis(50), Duration::from_millis(1));
    assert!(!asked.load(Ordering::Relaxed), "cleared on the way out");
    // A later start, triggered by a Mind: it does not keep the keyboard.
    let later = Fake::new(&[Some("Notes")], 1, Holder::Open, true);
    place(&later, Landing::Corner, Duration::from_secs(5), Duration::from_millis(1), Duration::ZERO, &asked, &AtomicBool::new(false));
    assert_eq!(later.calls(), ["present Notes"], "the keyboard went back to the person's window");
}

#[test]
fn a_press_that_cannot_start_mind_view_also_leaves_nothing_set() {
    struct NoStart;
    impl Compositor for NoStart {
        fn focused(&self) -> Option<String> { None }
        fn mind_view_mapped(&self) -> bool { false }
        fn holder(&self, _: &str) -> Holder { Holder::Unknown }
        fn present(&self, _: &str) -> bool { false }
        fn forward(&self) -> bool { false }
        fn ensure(&self) -> Result<(), String> { Err("no labwc".into()) }
        fn minimise_mind_view(&self) -> bool { false }
        fn placing(&self) -> bool { false }
    }
    let asked = AtomicBool::new(false);
    serve_press(&NoStart, &asked, Duration::from_millis(10), Duration::from_millis(1));
    assert!(!asked.load(Ordering::Relaxed));
}

/// While the press is served, the request is set, so a window that maps for it keeps the keyboard.
#[test]
fn the_request_is_set_while_the_press_is_served_and_clears_after() {
    let flag = AtomicBool::new(false);
    {
        let _a = Asking::new(&flag);
        assert!(flag.load(Ordering::Relaxed));
    }
    assert!(!flag.load(Ordering::Relaxed));
}

/// M3: Mind View is brought forward by app id and title together. A window that has only the
/// title (a page a Mind opened) matches neither command.
#[test]
fn mind_view_is_brought_forward_by_app_id_and_title_never_by_title_alone() {
    for cmd in forward_commands() {
        assert!(cmd.iter().any(|a| a == "app_id:labwc") && cmd.iter().any(|a| a == "title:Mind View"), "{cmd:?}");
    }
    let [restore, focus] = forward_commands();
    assert_eq!(restore[1], "maximize");
    assert!(restore.contains(&"state:minimized".to_string()));
    assert_eq!(focus[1], "focus");
}

/// The cancelled watch gives up without acting.
#[test]
fn a_cancelled_watch_does_nothing() {
    let c = Fake::new(&[Some("Notes")], 0, Holder::Open, true);
    let cancel = AtomicBool::new(true);
    place(&c, Landing::Corner, Duration::from_secs(5), Duration::from_millis(1), Duration::ZERO, &AtomicBool::new(false), &cancel);
    assert!(c.calls().is_empty());
}

/// M1: the keyboard goes back as soon as Mind View reports as focused, not after a fixed wait.
#[test]
fn the_keyboard_goes_back_when_the_activation_changes_not_after_a_fixed_wait() {
    let c = Fake::new(&[Some("Notes"), Some("Notes"), Some("Mind View")], 1, Holder::Open, true);
    let started = Instant::now();
    let asked = AtomicBool::new(false);
    place(&c, Landing::Corner, Duration::from_secs(5), Duration::from_millis(1), Duration::from_secs(5), &asked, &AtomicBool::new(false));
    assert_eq!(c.calls(), ["present Notes"]);
    assert!(started.elapsed() < Duration::from_secs(1), "did not sit out the 5 s cap");
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

/// The chip's press that itself started Mind View (the VM's check 6): the request it made must
/// still stand while the placement that saw the map decides, and the window is brought forward
/// once that has finished, not before.
#[test]
fn a_press_that_starts_mind_view_waits_for_its_placement_then_brings_it_forward() {
    let asked = std::sync::Arc::new(AtomicBool::new(false));
    let mut c = Fake::new(&[Some("Notes")], 1, Holder::Open, true);
    c.forward_seq = RefCell::new(vec![false, true]);
    c.placing_for = RefCell::new(2);
    c.watch = Some(asked.clone());
    serve_press(&c, &asked, Duration::from_secs(5), Duration::from_millis(1));
    let calls = c.calls();
    assert_eq!(calls[..2], ["forward", "ensure"], "{calls:?}");
    assert_eq!(calls.last().unwrap(), "forward", "brought forward after the placement ended: {calls:?}");
    let waits: Vec<_> = calls.iter().filter(|c| c.starts_with("placing")).collect();
    assert_eq!(waits.len(), 3, "waited while the placement ran: {calls:?}");
    assert!(waits.iter().all(|w| w.as_str() == "placing asked=Some(true)"), "the request stood throughout: {waits:?}");
    assert!(!asked.load(Ordering::Relaxed), "and is cleared on the way out");
}

/// Mind View already running, minimised: the press restores and focuses it, and starts nothing.
#[test]
fn a_press_on_a_running_mind_view_brings_it_forward_and_starts_nothing() {
    let c = Fake::new(&[Some("Notes")], 1, Holder::Open, true);
    serve_press(&c, &AtomicBool::new(false), Duration::from_secs(5), Duration::from_millis(1));
    assert_eq!(c.calls(), ["forward"]);
}

/// Only the person's close sends the next start aside, and asking for Mind View back (the chip)
/// ends that: the shell stopping or losing Mind View is never noted, the person's press clears it.
#[test]
fn only_the_persons_close_sends_a_start_aside_and_the_chip_ends_it() {
    assert_eq!(landing(false), Landing::Corner, "a first start, after a shell restart too");
    assert_eq!(landing(true), Landing::Aside, "after the person closed it");
    note_closed_by_person();
    serve_press(&Fake::new(&[None], 0, Holder::Open, true), &AtomicBool::new(false), Duration::from_millis(10), Duration::from_millis(1));
    assert!(!AWAY.load(Ordering::Relaxed), "the press cleared it");
    let ensure = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/mind_view.rs")).unwrap();
    let ensure = &ensure[ensure.find("pub fn ensure()").unwrap()..];
    let ensure = &ensure[..ensure.find("\n}\n").unwrap()];
    assert!(!ensure.contains("note_gone"));
    assert_eq!(ensure.matches("note_closed_by_person").count(), 1);
    let before = &ensure[..ensure.find("crate::mind_view_show::note_closed_by_person").unwrap()];
    assert!(before.trim_end().ends_with("if closed {"), "only under the person's close: {before}");
}

/// What the compositor-side rule sizes Mind View to is what the seat script sizes its output to.
#[test]
fn the_rule_sizes_the_window_to_the_shares_the_seat_script_uses() {
    let rc = include_str!("../../../config/labwc/rc.xml");
    let start = rc.find(r#"<windowRule identifier="labwc" title="Mind View">"#).unwrap();
    let rule = &rc[start..start + rc[start..].find("</windowRule>").unwrap()];
    let resize = rule.find("ResizeTo").expect("the host window is sized by the rule: a wayland output's custom mode does not size it");
    assert!(resize < rule.find("MoveToEdge").unwrap(), "sized before it is moved to the corner");
    let want = format!(r#"width="{}%" height="{}%""#, crate::mind_view::WIDTH_PERCENT, crate::mind_view::HEIGHT_PERCENT);
    assert!(rule.contains(&want), "{rule}");
}
