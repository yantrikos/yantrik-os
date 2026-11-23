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

/// The chord the shell types places labwc's own window titled as Mind View's is, and no other: a
/// page or app that sets the same title on the person's desktop is not placed by it. There is no
/// window rule for Mind View any more: a rule runs at map, before the window has its real size.
#[test]
fn the_chord_places_labwcs_own_window_in_the_corner_and_there_is_no_map_time_rule() {
    let rc = include_str!("../../../config/labwc/rc.xml");
    assert!(!rc.contains(r#"title="Mind View">"#), "no window rule for Mind View");
    let start = rc.find(r#"<keybind key="W-A-m">"#).expect("the placement keybind");
    let bind = &rc[start..start + rc[start..].find("</keybind>").unwrap()];
    let app_id = WINDOW_MATCH[0].strip_prefix("app_id:").unwrap();
    assert!(bind.contains(&format!(r#"identifier="{app_id}""#)) && bind.contains(&format!(r#"title="{}""#, crate::mind_view::TITLE)), "{bind}");
    assert!(bind.contains("ForEach"), "reaches the window by its names, not the focused one");
    // The seat script sizes the nested compositor's output, which is this window: resizing it
    // from here ended Mind View on the VM, so the chord only moves.
    assert!(!bind.contains("ResizeTo") && !bind.contains("UnMaximize"), "position only: {bind}");
    assert!(bind.contains(r#"direction="right""#) && bind.contains(r#"direction="down""#) && bind.contains("MoveRelative"));
    assert!(!bind.contains("Iconify") && !bind.contains("ToggleAlwaysOnTop"), "shown, and not over what is being typed in");
    // The shell types exactly this chord: Super+Alt+m.
    assert_eq!(PLACE_KEYS, ["keyboard", "type", "m", "modifiers", "SUPER,ALT"]);
}

/// `wlrctl` v0.2.2 refuses `LOGO` and `MOD4`; it takes `SUPER` and `ALT`.
#[test]
fn the_chords_use_the_modifier_names_wlrctl_accepts() {
    for keys in [PLACE_KEYS, RAISE_KEYS] {
        let mods = keys[4];
        assert!(mods.split(',').all(|m| ["SUPER", "ALT", "CTRL", "SHIFT"].contains(&m)), "{mods}");
        assert!(!mods.contains("LOGO") && !mods.contains("MOD4"));
    }
}

/// The raise chord raises Mind View by its own names and does nothing else: no focus, no resize.
#[test]
fn the_raise_chord_raises_only_and_does_not_focus() {
    let rc = include_str!("../../../config/labwc/rc.xml");
    let start = rc.find(r#"<keybind key="W-A-r">"#).expect("the raise keybind");
    let bind = &rc[start..start + rc[start..].find("</keybind>").unwrap()];
    assert!(bind.contains(r#"identifier="labwc""#) && bind.contains(r#"title="Mind View""#) && bind.contains(r#"<action name="Raise" />"#), "{bind}");
    for no in ["Focus", "ResizeTo", "Iconify", "Move"] {
        assert!(!bind.contains(no), "{no}: {bind}");
    }
    assert_eq!(RAISE_KEYS[2], "r");
}

/// The shell's own window was the keyboard's last holder: it gets the keyboard back, and Mind View
/// is raised after that, so the full-screen shell is not left over it.
#[test]
fn when_the_shell_held_the_keyboard_mind_view_is_raised_after_the_hand_back() {
    let c = Fake::new(&[Some("Yantrik OS")], 1, Holder::Open, true);
    run(&c, Landing::Corner, false);
    assert_eq!(c.calls(), ["place", "present Yantrik OS", "raise"]);
    let src = include_str!("mind_view_show.rs");
    assert!(src.contains(r#""Mind View raised above the window the keyboard went to, not focused""#), "logged");
    // A set-aside or chip-asked start is not raised.
    for (landing, asked) in [(Landing::Aside, false), (Landing::Corner, true)] {
        let c = Fake::new(&[Some("Yantrik OS")], 1, Holder::Open, true);
        run(&c, landing, asked);
        assert!(!c.calls().contains(&"raise".to_string()), "{:?}", c.calls());
    }
}

fn open(_: &str) -> Holder {
    Holder::Open
}

#[test]
fn the_keyboard_goes_back_to_the_window_that_held_it_and_only_if_that_is_still_there() {
    assert_eq!(decide(Landing::Corner, false, Some("Notes"), open), Action::HandBack("Notes".into()));
    for now in [Holder::Minimised, Holder::Unknown] {
        assert!(matches!(decide(Landing::Corner, false, Some("Notes"), |_| now), Action::ToShell(_)), "{now:?}");
    }
    assert!(matches!(decide(Landing::Corner, false, None, open), Action::ToShell(_)), "nobody known to hold it");
    assert_eq!(decide(Landing::Aside, false, Some("Notes"), open), Action::SetAside);
    assert_eq!(decide(Landing::Corner, true, Some("Notes"), open), Action::Keep, "the person pressed the chip");
    assert_eq!(decide(Landing::Aside, true, None, open), Action::Keep);
}

/// Every input a start that lands in the corner can have: none of them minimises Mind View, here
/// or in the hand-back's failure.
#[test]
fn a_corner_start_is_never_minimised_whatever_the_inputs() {
    let focus: [&[Option<&str>]; 3] = [&[None], &[Some("Notes")], &[Some("Yantrik OS")]];
    for focus in focus {
        for holder in [Holder::Open, Holder::Minimised, Holder::Unknown] {
            for presents in [true, false] {
                for asked in [false, true] {
                    let c = Fake::new(focus, 1, holder, presents);
                    run(&c, Landing::Corner, asked);
                    assert!(!c.calls().iter().any(|x| x.starts_with("minimise")), "{focus:?} {holder:?} presents={presents} asked={asked}: {:?}", c.calls());
                }
            }
        }
    }
}

/// Only the Aside landing minimises, and it says which path asked. The code that minimises Mind
/// View is `minimise_mind_view`, called from `place` once.
#[test]
fn the_only_caller_of_minimise_is_the_aside_landing() {
    let src = include_str!("mind_view_show.rs");
    let code = src.split("#[cfg(test)]").next().unwrap();
    let calls: Vec<_> = code.lines().filter(|l| l.contains(".minimise_mind_view(")).collect();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert!(calls[0].contains("landing:"), "names its path: {calls:?}");
    assert!(!code.contains("windows::minimise(") && !code.contains("step_aside"), "no other path");
    let rc = include_str!("../../../config/labwc/rc.xml");
    let mind_rc = include_str!("../../../config/labwc-mind/rc.xml");
    assert!(!mind_rc.contains("Iconify"), "nothing in Mind View's own compositor minimises it");
    let place = &rc[rc.find(r#"<keybind key="W-A-m">"#).unwrap()..];
    assert!(!place[..place.find("</keybind>").unwrap()].contains("Iconify"));
}

/// Every call that decides something is logged with what it was for (item 1).
#[test]
fn every_deciding_call_and_the_landing_are_logged_at_info() {
    let src = include_str!("mind_view_show.rs");
    let code = src.split("#[cfg(test)]").next().unwrap();
    assert!(code.contains(r#""Mind View landing""#) && code.contains("closed = landing == Landing::Aside") && code.contains("asked_for = asked") && code.contains("holder_known"));
    assert!(code.contains(r#""Mind View is being minimised""#) && code.contains("path,"));
    assert!(code.contains(r#""Mind View wlrctl call""#) && code.contains("exit = ?o.status.code()") && code.contains("output ="));
    // No bare `wlrctl(` call outside the polling `found` and the logged `call`.
    let bare: Vec<_> = code.lines().filter(|l| l.contains("wlrctl(") && !l.trim_start().starts_with("//")).collect();
    assert_eq!(bare.len(), 2, "found and call only: {bare:?}");
    // A window of the person's is never logged by title.
    assert!(code.contains("title:<holder>"));
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
    fn minimise_mind_view(&self, path: &'static str) -> bool {
        self.calls.borrow_mut().push(format!("minimise {path}"));
        true
    }
    fn place_window(&self) -> bool {
        self.calls.borrow_mut().push("place".into());
        true
    }
    fn raise_window(&self) -> bool {
        self.calls.borrow_mut().push("raise".into());
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
    place(c, landing, Duration::from_secs(5), Duration::from_millis(1), Duration::ZERO, Duration::ZERO, &asked, &AtomicBool::new(false));
    assert!(!asked.load(Ordering::Relaxed), "the request is used up by the map it was for");
}

/// The person switched windows while Mind View was starting: the one that has the keyboard
/// when it maps gets it back, not the one that had it when the start began.
#[test]
fn the_window_read_last_before_the_map_is_the_one_given_back_to() {
    let c = Fake::new(&[Some("Notes"), Some("Terminal"), Some("Browser")], 2, Holder::Open, true);
    run(&c, Landing::Corner, false);
    assert_eq!(c.calls(), ["place", "present Browser", "raise"]);
}

#[test]
fn mind_view_itself_is_never_the_one_given_the_keyboard() {
    let c = Fake::new(&[Some("Notes"), Some("Mind View")], 2, Holder::Open, true);
    run(&c, Landing::Corner, false);
    assert_eq!(c.calls(), ["place", "present Notes", "raise"]);
}

#[test]
fn a_window_that_closed_or_was_minimised_or_cannot_be_shown_leaves_the_keyboard_with_no_one_new() {
    for holder in [Holder::Minimised, Holder::Unknown] {
        let c = Fake::new(&[Some("Notes")], 1, holder, true);
        run(&c, Landing::Corner, false);
        assert_eq!(c.calls(), ["place", "present Yantrik OS", "raise"], "{holder:?}: to the shell, and shown");
    }
    let c = Fake::new(&[None], 1, Holder::Open, true);
    run(&c, Landing::Corner, false);
    assert_eq!(c.calls(), ["place", "present Yantrik OS", "raise"], "no holder known (the lock screen, or no wlrctl)");
    let c = Fake::new(&[Some("Notes")], 1, Holder::Open, false);
    run(&c, Landing::Corner, false);
    assert_eq!(c.calls(), ["place", "present Notes", "present Yantrik OS", "raise"], "present failed: the shell is tried, Mind View is not minimised");
}

#[test]
fn a_mind_view_the_person_closed_starts_aside_and_one_they_asked_for_keeps_the_keyboard() {
    let c = Fake::new(&[Some("Notes")], 1, Holder::Open, true);
    run(&c, Landing::Aside, false);
    assert_eq!(c.calls(), ["place", "minimise landing: the person closed the last one"]);
    let c = Fake::new(&[Some("Notes")], 1, Holder::Open, true);
    run(&c, Landing::Corner, true);
    assert_eq!(c.calls(), ["place"]);
}

#[test]
fn a_window_that_never_appears_is_given_up_on_at_the_deadline() {
    let c = Fake::new(&[Some("Notes")], usize::MAX, Holder::Open, true);
    let started = Instant::now();
    place(&c, Landing::Corner, Duration::from_millis(100), Duration::from_millis(5), Duration::ZERO, Duration::ZERO, &AtomicBool::new(false), &AtomicBool::new(false));
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
    place(&later, Landing::Corner, Duration::from_secs(5), Duration::from_millis(1), Duration::ZERO, Duration::ZERO, &asked, &AtomicBool::new(false));
    assert_eq!(later.calls(), ["place", "present Notes", "raise"], "the keyboard went back to the person's window");
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
        fn minimise_mind_view(&self, _: &'static str) -> bool { false }
        fn place_window(&self) -> bool { false }
        fn raise_window(&self) -> bool { false }
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
    place(&c, Landing::Corner, Duration::from_secs(5), Duration::from_millis(1), Duration::ZERO, Duration::ZERO, &AtomicBool::new(false), &cancel);
    assert!(c.calls().is_empty());
}

/// M1: the keyboard goes back as soon as Mind View reports as focused, not after a fixed wait.
#[test]
fn the_keyboard_goes_back_when_the_activation_changes_not_after_a_fixed_wait() {
    let c = Fake::new(&[Some("Notes"), Some("Notes"), Some("Mind View")], 1, Holder::Open, true);
    let started = Instant::now();
    let asked = AtomicBool::new(false);
    place(&c, Landing::Corner, Duration::from_secs(5), Duration::from_millis(1), Duration::from_secs(5), Duration::ZERO, &asked, &AtomicBool::new(false));
    assert_eq!(c.calls(), ["place", "present Notes", "raise"]);
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
    assert_eq!(calls[calls.len() - 2..], ["forward", "place"], "brought forward after the placement ended, then placed: {calls:?}");
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
    assert_eq!(c.calls(), ["forward", "place"], "restored and focused, then placed again; nothing started");
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
