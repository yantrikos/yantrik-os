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
//! forward, the keyboard goes to the shell's own window instead. A start that lands in the corner
//! ([`Landing::Corner`]) is never minimised by anything here: only [`Landing::Aside`] (the person
//! closed the last one) sets it aside. Only the person's press on the bar's chip focuses it
//! ([`bring_forward`]).
//!
//! Window rules run once, at map, when the nested compositor's window may not have its final size,
//! so the placement is not a rule: once the window has settled ([`SETTLE`]) the shell asks the
//! person's compositor for it with one key chord (`PLACE_KEYS`, the binding in
//! `config/labwc/rc.xml`), which sizes and moves every window of Mind View's own.
//!
//! Every `wlrctl` call here has a deadline (`wlrctl_bounded`), and every one that decides something
//! is logged at info level, with its arguments (never a window of the person's), exit status and
//! output ([`call`]); so is the landing decision and each minimise, with the path that asked.
//!
//! Only the first map is acted on. A window the person minimised stays minimised, and one they
//! closed is not put back in front by a later turn: [`note_closed_by_person`] is told when the
//! person's close was noticed (`mind_view_closed`), and the next Mind View then starts out of the
//! way until the person asks for it (the bar's "working" chip, Super+Shift+V or the taskbar). A
//! Mind View the shell stopped, or that died, is not the person's close: the next one is shown.

use crate::wlrctl_bounded::wlrctl;

/// The shell's own window, which the keyboard goes to when no other window can have it.
const SHELL: &str = crate::windows::SHELL_WINDOW_TITLE;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// Mind View's window, as the person's compositor names it: labwc's app id, and the title the
/// title library gives it (crates/yantrik-mind-view-title). The window rule in config/labwc/rc.xml
/// matches the same two.
const WINDOW_MATCH: [&str; 2] = ["app_id:labwc", "title:Mind View"];

/// The longest the new window is waited for to take the keyboard before it is handed back. There
/// is no fixed wait: the activation is polled for every [`ACTIVATION_POLL`] and the keyboard goes
/// back the moment the compositor reports Mind View as the focused window. This is only the cap,
/// for a compositor that never focuses it.
const ACTIVATION_WAIT: Duration = Duration::from_millis(400);
const ACTIVATION_POLL: Duration = Duration::from_millis(10);

/// How often the keyboard's holder is read while the window is awaited.
const POLL: Duration = Duration::from_millis(100);

/// How long the window is given to take its real size after it maps, before it is placed.
const SETTLE: Duration = Duration::from_millis(300);

/// The chord the person's compositor binds to place and size Mind View's window
/// (`config/labwc/rc.xml`, key `W-A-m`): typed by the shell through `wlrctl`.
/// The modifier names are the ones `wlrctl` v0.2.2 accepts: it refuses `LOGO` and `MOD4`.
const PLACE_KEYS: [&str; 5] = ["keyboard", "type", "m", "modifiers", "SUPER,ALT"];

/// The chord that raises Mind View above the other windows without focusing it
/// (`config/labwc/rc.xml`, key `W-A-r`). A chord of its own, so each action can be tried alone.
const RAISE_KEYS: [&str; 5] = ["keyboard", "type", "r", "modifiers", "SUPER,ALT"];

/// Whether the person closed Mind View's window since the shell started and has not asked for it
/// back. Not set by the shell stopping Mind View, nor by its dying.
static AWAY: AtomicBool = AtomicBool::new(false);

/// Placements started and not yet finished: a press that started Mind View waits for them, so the
/// request it made is still standing when the map is acted on.
static PLACING: AtomicUsize = AtomicUsize::new(0);

struct Placement;

impl Placement {
    fn begin() -> Placement {
        PLACING.fetch_add(1, Ordering::SeqCst);
        Placement
    }
}

impl Drop for Placement {
    fn drop(&mut self) {
        PLACING.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Set only while the person's press on the chip is being served ([`Asking`]): the keyboard is
/// theirs to give, so a window that maps meanwhile keeps it. Cleared on every way out of
/// `bring_forward`, so a start a Mind triggers later never finds it set.
static ASKED_FOR: AtomicBool = AtomicBool::new(false);

/// Where Mind View lands when it maps.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Landing {
    /// In the corner, shown, with the keyboard given back.
    Corner,
    /// Minimised: the person closed the last one, and a later turn does not reopen it.
    Aside,
}

fn landing(person_closed: bool) -> Landing {
    if person_closed {
        Landing::Aside
    } else {
        Landing::Corner
    }
}

/// The person closed Mind View's window. The next one starts aside.
pub fn note_closed_by_person() {
    AWAY.store(true, Ordering::Relaxed);
}

/// Whether Mind View's window is on the person's desktop (`None`: the compositor did not say).
pub fn window_present() -> Option<bool> {
    found(&WINDOW_MATCH)
}

/// What became of the window that held the keyboard when Mind View mapped.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Holder {
    /// Not minimised as far as is known. Whether it is still there is told by bringing it forward,
    /// which fails for a window that has closed (that too ends in minimising Mind View).
    Open,
    Minimised,
    /// The compositor did not say (no `wlrctl`, or no answer in time).
    Unknown,
}

/// What is done to Mind View once it has mapped.
#[derive(Clone, Debug, PartialEq)]
enum Action {
    /// Leave it: the person asked for it with the chip.
    Keep,
    /// Minimise it: the person closed the last one, so this one is not shown.
    SetAside,
    /// Give the keyboard back to this window.
    HandBack(String),
    /// There is no window known to give the keyboard to: it goes to the shell's own window. Mind
    /// View stays where it landed.
    ToShell(&'static str),
}

impl Action {
    fn name(&self) -> &'static str {
        match self {
            Action::Keep => "keep",
            Action::SetAside => "set_aside",
            Action::HandBack(_) => "hand_back",
            Action::ToShell(_) => "to_shell",
        }
    }
}

fn decide(landing: Landing, asked_for: bool, held: Option<&str>, holder_now: impl FnOnce(&str) -> Holder) -> Action {
    if asked_for {
        return Action::Keep;
    }
    if landing == Landing::Aside {
        return Action::SetAside;
    }
    let Some(title) = held else {
        return Action::ToShell("nothing was known to hold the keyboard before it mapped");
    };
    match holder_now(title) {
        Holder::Open => Action::HandBack(title.to_string()),
        Holder::Minimised => Action::ToShell("the window that held the keyboard was minimised meanwhile"),
        Holder::Unknown => Action::ToShell("the compositor did not say whether the keyboard's window is still open"),
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
    /// Bring Mind View itself forward, restoring it if minimised. Only by [`WINDOW_MATCH`]: a
    /// window that merely has its title is not it.
    fn forward(&self) -> bool;
    /// Start Mind View if it is not running (`mind_view::ensure`).
    fn ensure(&self) -> Result<(), String>;
    /// Minimise Mind View. `path` names the code that asked, for the log.
    fn minimise_mind_view(&self, path: &'static str) -> bool;
    /// Size and move Mind View's window to the corner, once it has settled.
    fn place_window(&self) -> bool;
    /// Raise Mind View above the other windows, without giving it the keyboard. Done after the
    /// keyboard has been handed back, because handing it to a window raises that window (the
    /// shell's is full screen) over Mind View.
    fn raise_window(&self) -> bool;
    /// Whether a placement is still waiting for the window or acting on it.
    fn placing(&self) -> bool;
}

struct Wlrctl;

/// One `wlrctl` call that decides something, logged: what it was for, its arguments, its exit
/// status and its output. The arguments are Mind View's own names and fixed words, never a title
/// of the person's (a call that has one passes a stand-in in `shown`).
fn call(what: &str, args: &[&str], shown: &[&str]) -> Option<std::process::Output> {
    let answer = wlrctl(args).ok().flatten();
    match &answer {
        Some(o) => tracing::info!(what, args = %shown.join(" "), exit = ?o.status.code(), output = %String::from_utf8_lossy(&o.stdout).trim().chars().take(200).collect::<String>(), "Mind View wlrctl call"),
        None => tracing::info!(what, args = %shown.join(" "), "Mind View wlrctl call: no answer (could not run, or past its deadline)"),
    }
    answer
}

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
    /// One `wlrctl` call: minimised, or not.
    fn holder(&self, title: &str) -> Holder {
        let spec = format!("title:{title}");
        let answer = call("holder_minimised", &["toplevel", "find", &spec, "state:minimized"], &["toplevel", "find", "title:<holder>", "state:minimized"]).map(|o| o.status.success());
        match answer {
            Some(true) => Holder::Minimised,
            Some(false) => Holder::Open,
            None => Holder::Unknown,
        }
    }
    fn present(&self, title: &str) -> bool {
        let shell = title == crate::windows::SHELL_WINDOW_TITLE;
        let ok = if shell { crate::windows::raise_shell().is_ok() } else { crate::windows::present(title) };
        let target_window = if shell { "the shell" } else { "the keyboard's last holder" };
        tracing::info!(target_window, ok, "Mind View gave the keyboard back");
        ok
    }
    fn forward(&self) -> bool {
        // Both by `WINDOW_MATCH`; the restore only touches a minimised window, and its failure
        // (nothing minimised) is not one.
        let [restore, focus] = forward_commands();
        let run = |what: &str, args: Vec<String>| {
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            call(what, &args, &args).is_some_and(|o| o.status.success())
        };
        run("chip_restore", restore);
        run("chip_focus", focus)
    }
    fn place_window(&self) -> bool {
        call("place", &PLACE_KEYS, &PLACE_KEYS).is_some_and(|o| o.status.success())
    }
    fn raise_window(&self) -> bool {
        call("raise", &RAISE_KEYS, &RAISE_KEYS).is_some_and(|o| o.status.success())
    }
    fn ensure(&self) -> Result<(), String> {
        crate::mind_view::ensure().map(|_| ())
    }
    fn minimise_mind_view(&self, path: &'static str) -> bool {
        let mut args = vec!["toplevel", "minimize"];
        args.extend_from_slice(&WINDOW_MATCH);
        tracing::info!(path, "Mind View is being minimised");
        call("minimise", &args, &args).is_some_and(|o| o.status.success())
    }
    fn placing(&self) -> bool {
        PLACING.load(Ordering::SeqCst) > 0
    }
}

/// The two commands that bring Mind View forward: restore, then focus. Each names the window by
/// [`WINDOW_MATCH`], app id and title together, never by title alone.
fn forward_commands() -> [Vec<String>; 2] {
    let with = |verb: &str, extra: &[&str]| -> Vec<String> {
        ["toplevel", verb].into_iter().chain(WINDOW_MATCH).chain(extra.iter().copied()).map(String::from).collect()
    };
    [with("maximize", &["state:minimized"]), with("focus", &[])]
}

/// Ends a [`when_mapped`] watch whose window will never come (the start failed).
pub struct Placing(std::sync::Arc<AtomicBool>);

impl Placing {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// Place Mind View as it first appears. Call it before the nested compositor is started, so that
/// the keyboard's holder is read from before the window can exist, and cancel it if the start
/// fails: a watch left running would poll for ten seconds and could act on a later start's window.
/// Off the UI thread: it waits for the window, and wlrctl is a process.
pub fn when_mapped(wait: Duration) -> Placing {
    let landing = landing(AWAY.load(Ordering::Relaxed));
    let cancel = std::sync::Arc::new(AtomicBool::new(false));
    let watch = cancel.clone();
    let placement = Placement::begin();
    std::thread::spawn(move || {
        let _placement = placement;
        place(&Wlrctl, landing, wait, POLL, ACTIVATION_WAIT, SETTLE, &ASKED_FOR, &watch)
    });
    Placing(cancel)
}

fn place(c: &dyn Compositor, landing: Landing, wait: Duration, poll: Duration, activation_wait: Duration, settle: Duration, asked_for: &AtomicBool, cancel: &AtomicBool) {
    let deadline = Instant::now() + wait;
    let mut held: Option<String> = None;
    while Instant::now() < deadline && !cancel.load(Ordering::Relaxed) {
        // Before the window can have taken it: the last reading before the map is who had it then.
        if let Some(title) = c.focused() {
            if title != crate::mind_view::TITLE {
                held = Some(title);
            }
        }
        if c.mind_view_mapped() {
            // From here until the action is done, the person's keys can reach Mind View's display.
            let mapped = Instant::now();
            while mapped.elapsed() < activation_wait && c.focused().as_deref() != Some(crate::mind_view::TITLE) {
                std::thread::sleep(ACTIVATION_POLL);
            }
            // The window has had its time to take its real size; sized and moved from here, not
            // by a rule at map. Done while it still holds the keyboard, so the chord is the
            // compositor's alone.
            std::thread::sleep(settle);
            let placed = c.place_window();
            let asked = asked_for.swap(false, Ordering::Relaxed);
            let action = decide(landing, asked, held.as_deref(), |t| c.holder(t));
            let reason = match &action {
                Action::Keep => "the person asked for it with the chip",
                Action::SetAside => "the person closed the last one",
                Action::HandBack(_) => "the keyboard's last holder is open",
                Action::ToShell(why) => *why,
            };
            tracing::info!(landing = ?landing, closed = landing == Landing::Aside, holder_known = held.is_some(), asked_for = asked, placed, action = action.name(), reason, "Mind View landing");
            let action_name = action.name();
            match action {
                Action::Keep => {}
                Action::SetAside => {
                    if !c.minimise_mind_view("landing: the person closed the last one") {
                        tracing::warn!("Mind View appeared but could not be set aside; it may be over the person's work");
                    }
                }
                Action::HandBack(title) => {
                    if !c.present(&title) && !c.present(SHELL) {
                        tracing::warn!("the keyboard could not be given back to anyone; Mind View is left where it landed");
                    }
                }
                Action::ToShell(_) => {
                    if !c.present(SHELL) {
                        tracing::warn!("the keyboard could not be given to the shell; Mind View is left where it landed");
                    }
                }
            }
            // The hand-back put a window in front (the shell's, full screen, when it was the last
            // holder): Mind View is raised above it, not focused, so the keyboard stays where it was
            // given. A set-aside window is minimised and stays down; a chip press already has it.
            if matches!(action_name, "hand_back" | "to_shell") {
                let raised = c.raise_window();
                tracing::info!(raised, after = action_name, "Mind View raised above the window the keyboard went to, not focused");
            }
            tracing::info!(held_ms = mapped.elapsed().as_millis() as u64, "Mind View held the keyboard from its map until the keyboard was settled");
            return;
        }
        std::thread::sleep(poll);
    }
    tracing::debug!("Mind View's window did not appear in time to be placed, or its start was cancelled");
}

/// Marks the person's press on the chip as being served, and ends it however `bring_forward` is
/// left.
struct Asking<'a>(&'a AtomicBool);

impl<'a> Asking<'a> {
    fn new(flag: &'a AtomicBool) -> Self {
        flag.store(true, Ordering::Relaxed);
        Asking(flag)
    }
}

impl Drop for Asking<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Relaxed);
    }
}

/// Bring Mind View forward and give it the keyboard: the person pressed the bar's chip, which is
/// their own act and the only thing that focuses it. Restores it first if it was minimised, and
/// starts it if there is no window (the chip can show for a turn that opened no app). A process
/// and a start: off the UI thread.
pub fn bring_forward() {
    std::thread::spawn(|| serve_press(&Wlrctl, &ASKED_FOR, Duration::from_secs(5), POLL));
}

fn serve_press(c: &dyn Compositor, asked: &AtomicBool, wait: Duration, poll: Duration) {
    // The person wants it back: a Mind View they closed is shown again from here on.
    AWAY.store(false, Ordering::Relaxed);
    if c.forward() {
        // Restoring can leave it at another size: placed again from here.
        c.place_window();
        return;
    }
    // Cleared when this returns by any road, including a start that was not needed (Mind View was
    // already running, so no `place` ever consumes it).
    let _asking = Asking::new(asked);
    if let Err(why) = c.ensure() {
        tracing::warn!(reason = %why, "Mind View could not be started from the chip");
        return;
    }
    let deadline = Instant::now() + wait;
    while Instant::now() < deadline && !c.mind_view_mapped() {
        std::thread::sleep(poll);
    }
    // The placement that saw it map decides what becomes of the window (it keeps it, for this
    // press); the request is cleared only once it has, or a press that started Mind View could end
    // with it set aside (the VM's chip check).
    while Instant::now() < deadline && c.placing() {
        std::thread::sleep(poll);
    }
    if c.forward() {
        c.place_window();
    } else {
        tracing::warn!("Mind View was started from the chip but has no window to bring forward");
    }
}

#[cfg(test)]
#[path = "mind_view_show_tests.rs"]
mod tests;
