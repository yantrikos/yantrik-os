//! The Lens in a window of its own (#733, phase 1 of design/shell-as-windows-2026-10-10.md).
//!
//! The shell is one fullscreen window. The Lens used to be drawn inside it, so opening the chat
//! raised that window over every app, and to the person all their windows vanished. Now the Lens
//! is `LensWindow` (yantrik-ui-slint/ui/lens_window.slint): a 464px strip at the right edge,
//! between the status bar and the dock, that labwc keeps above the apps while it is up (the
//! windowRule for [`LENS_WINDOW_TITLE`] in config/labwc/rc.xml). Opening it shows that window and
//! nothing else; the person's apps stay where they were.
//!
//! The shell keeps owning the state. App's Lens properties are still what every wire module
//! writes and reads (wire/chat.rs, wire/lens.rs, wire/agents.rs, control_approvals.rs); this
//! module copies them into the window and hands the window's presses back to App's callbacks.
//! The models are shared, not copied — a streamed reply grows in the same `VecModel` both see.
//!
//! The window also draws the approval card in front (control_approvals), above whatever app is
//! open, so a card no longer needs the shell raised over the person's work either. With the Lens
//! closed it is just the card.
//!
//! Focus: the window takes the keyboard when it opens for the Lens (labwc focuses a window when
//! it maps; the Lens puts the cursor in its composer). When it closes, or when it went up only for
//! a card, the keyboard goes back to the window that had it, which `toplevel_watch` knows: the
//! Lens never counts as that window (`skipTaskbar` keeps it out of the compositor's list).

use std::cell::{Cell, RefCell};
use std::time::Duration;

use slint::{ComponentHandle, Model, SharedString, Timer, TimerMode};

use crate::windows::LENS_WINDOW_TITLE;
use crate::{AgentsState, App, LensWindow};

/// How often App's state is copied across. A streamed reply needs nothing from this (its model
/// is shared); this carries the rest — a new conversation, the mode label, a card going up.
const TICK: Duration = Duration::from_millis(100);

/// How long a window that has just mapped gets before the keyboard is handed back, as the
/// on-screen display does (wire/osd.rs).
const REFOCUS_AFTER: Duration = Duration::from_millis(150);

/// Theme.status-bar-height and Theme.taskbar-height (the dock): the window is the room between.
/// The tests hold these to theme.slint and to rc.xml's <margin>.
const STATUS_BAR: f32 = 36.0;
const DOCK: f32 = 48.0;

/// What the window is showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Hidden,
    /// Only the approval card in front: the Lens is closed and a card waits.
    Card,
    /// The Lens, with the card over its top when one waits that its conversation does not host.
    Lens,
}

/// The screens the window must never be up on: boot (0), onboarding (2), lock (3) and login (32).
/// It is above every other window, the lock screen included, and a Lens or a card usable there is
/// a way to act on the machine without unlocking it (#203). The same four the shell's corner card
/// keeps off (`approval-corner-shown` in app.slint).
const LOCKED_OUT: [i32; 4] = [0, 2, 3, 32];

/// What the window should show, from App's state.
pub(crate) fn mode(lens_open: bool, card_waiting: bool, screen: i32) -> Mode {
    if LOCKED_OUT.contains(&screen) {
        Mode::Hidden
    } else if lens_open {
        Mode::Lens
    } else if card_waiting {
        Mode::Card
    } else {
        Mode::Hidden
    }
}

/// What to do to the window going from one mode to another.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Step {
    Nothing,
    /// Map it. labwc focuses a window when it maps: the Lens keeps that, a card hands it back.
    Show { give_back: bool },
    /// Unmap and map again: a card's window becoming the Lens has to take the keyboard, and on
    /// Wayland a window cannot take it on its own — mapping is how labwc gives it.
    Remap,
    /// Stay up, as the card alone; the Lens was closed, so the keyboard goes back.
    Shrink,
    /// Unmap it and give the keyboard back to the window that had it.
    Hide,
}

pub(crate) fn step(from: Mode, to: Mode) -> Step {
    match (from, to) {
        (a, b) if a == b => Step::Nothing,
        (Mode::Hidden, Mode::Lens) => Step::Show { give_back: false },
        (Mode::Hidden, Mode::Card) => Step::Show { give_back: true },
        (Mode::Card, Mode::Lens) => Step::Remap,
        (Mode::Lens, Mode::Card) => Step::Shrink,
        (_, Mode::Hidden) => Step::Hide,
        _ => Step::Nothing,
    }
}

/// The window to give the keyboard back to: the one in front by the focus stream, by its id —
/// never one of the shell's own surfaces, which the stream does not report but a stream without
/// the labwc rules loaded might.
pub(crate) fn give_back_to(front: Option<(u64, String)>) -> Option<u64> {
    front.filter(|(_, title)| !crate::windows::is_shell_surface(title)).map(|(id, _)| id)
}

/// The room between the bars on a display `height` logical pixels tall.
pub(crate) fn usable_height(height: f32) -> f32 {
    (height - STATUS_BAR - DOCK).max(0.0)
}

struct Lens {
    window: LensWindow,
    mode: Cell<Mode>,
    /// The field's text as last agreed by both windows: whichever side differs from it changed.
    text: RefCell<SharedString>,
}

thread_local! {
    static LENS: RefCell<Option<Lens>> = const { RefCell::new(None) };
    static TIMER: Timer = Timer::default();
}

/// Make the window (hidden), hand its presses to `ui`, and start following App's state.
pub fn wire(ui: &App) {
    let window = match LensWindow::new() {
        Ok(w) => w,
        Err(e) => {
            tracing::error!(error = %e, "no window for the Lens; it cannot be opened");
            return;
        }
    };
    // The shell runs under SLINT_FULLSCREEN=1 and every window of this process reads it, as the
    // agents' pop-out found (#231).
    window.window().set_fullscreen(false);
    // Closing it from the compositor (it has no frame, but a keybinding can) is closing the Lens.
    {
        let weak = ui.as_weak();
        window.window().on_close_requested(move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_lens_open(false);
                ui.invoke_close_lens();
                sync(&ui);
            }
            slint::CloseRequestResponse::KeepWindowShown
        });
    }
    forward(&window, ui);
    LENS.with(|slot| {
        *slot.borrow_mut() =
            Some(Lens { window, mode: Cell::new(Mode::Hidden), text: RefCell::new(ui.get_lens_input_text()) });
    });
    let weak = ui.as_weak();
    TIMER.with(|t| {
        t.start(TimerMode::Repeated, TICK, move || {
            if let Some(ui) = weak.upgrade() {
                sync(&ui);
            }
        })
    });
}

/// Bring the window in line with App now: copy the state across and show, raise or hide it.
/// Cheap and idempotent; the timer calls it, and so does anything that has just opened or closed
/// the Lens or put up a card, so that it does not wait for the next tick.
pub fn sync(ui: &App) {
    LENS.with(|slot| {
        let slot = slot.borrow();
        let Some(lens) = slot.as_ref() else { return };
        let card = ui.get_pending_approvals().row_count() > 0;
        let next = mode(ui.get_lens_open(), card, ui.get_current_screen());
        if next != Mode::Hidden {
            copy(ui, lens);
        }
        let from = lens.mode.replace(next);
        apply(ui, lens, step(from, next));
        if ui.get_lens_window_up() != (next != Mode::Hidden) {
            ui.set_lens_window_up(next != Mode::Hidden);
        }
        crate::card_watch::card_in_lens_window(next != Mode::Hidden && card);
    });
}

fn apply(ui: &App, lens: &Lens, step: Step) {
    let w = &lens.window;
    match step {
        Step::Nothing => {}
        Step::Show { give_back } => {
            show(ui, w);
            if give_back {
                give_focus_back(REFOCUS_AFTER);
            }
        }
        Step::Remap => {
            let _ = w.hide();
            show(ui, w);
        }
        Step::Shrink => give_focus_back(Duration::ZERO),
        Step::Hide => {
            if let Err(e) = w.hide() {
                tracing::warn!(error = %e, "could not hide the Lens's window");
            }
            give_focus_back(Duration::ZERO);
        }
    }
}

fn show(ui: &App, w: &LensWindow) {
    let size = ui.window().size().to_logical(ui.window().scale_factor());
    if size.height > 0.0 {
        w.set_usable_height(usable_height(size.height));
    }
    match w.show() {
        // A surface came up where a click aimed at the window under it can land: the press guard
        // on Allow starts, as it does whenever the shell comes forward (card_watch).
        Ok(()) => crate::card_watch::shell_came_forward(),
        Err(e) => tracing::warn!(error = %e, window = LENS_WINDOW_TITLE, "could not show the Lens's window"),
    }
}

/// Hand the keyboard back to the window the person was in. Off the UI thread: the delay, and the
/// wlrctl fallback, which is a process.
fn give_focus_back(after: Duration) {
    std::thread::spawn(move || {
        std::thread::sleep(after);
        let front = crate::toplevel_watch::front();
        match give_back_to(front.clone()) {
            Some(id) if crate::toplevel_watch::activate(id) => {}
            _ => {
                // No focus stream: the compositor's own answer, by title, if it has one.
                if let Some(title) = front.map(|(_, t)| t).filter(|t| !crate::windows::is_shell_surface(t)) {
                    crate::windows::present(&title);
                }
            }
        }
    });
}

/// App's Lens state into the window, writing only what changed.
fn copy(ui: &App, lens: &Lens) {
    let w = &lens.window;
    macro_rules! mirror {
        ($($get:ident => $set:ident),* $(,)?) => {$({
            let want = ui.$get();
            if w.$get() != want {
                w.$set(want);
            }
        })*};
    }
    mirror!(
        get_lens_open => set_lens_open,
        get_lens_results => set_lens_results,
        get_lens_chat_mode => set_lens_chat_mode,
        get_lens_destination => set_lens_destination,
        get_lens_suggestions => set_lens_suggestions,
        get_lens_recents => set_lens_recents,
        get_messages => set_messages,
        get_is_generating => set_is_generating,
        get_is_thinking => set_is_thinking,
        get_companion_status => set_companion_status,
        get_companion_online => set_companion_online,
        get_active_harness_name => set_active_harness_name,
        get_mind_mode_label => set_mind_mode_label,
        get_lens_can_open_in_agents => set_lens_can_open_in_agents,
        get_lens_questions => set_lens_questions,
        get_approvals => set_approvals,
        get_pending_approvals => set_pending_approvals,
        get_approvals_waiting => set_approvals_waiting,
        get_approval_in_lens => set_approval_in_lens,
    );
    let g = ui.global::<AgentsState>();
    macro_rules! mirror_agents {
        ($($get:ident => $set:ident),* $(,)?) => {$({
            let want = g.$get();
            if w.$get() != want {
                w.$set(want);
            }
        })*};
    }
    mirror_agents!(
        get_lens_cards => set_lens_cards,
        get_lens_strip => set_lens_strip,
        get_lens_live => set_lens_live,
        get_lens_waiting => set_lens_waiting,
        get_lens_identity => set_lens_identity,
    );
    agree_on_text(ui, lens);
    crate::theme_sync::sync_theme!(ui, w);
}

/// The field's text is the one property both sides write: the person types in the window, and
/// `open_lens text=…` or the Minds panel prefill App's. Whichever moved since they last agreed wins.
fn agree_on_text(ui: &App, lens: &Lens) {
    let mut agreed = lens.text.borrow_mut();
    let (here, there) = (lens.window.get_lens_input_text(), ui.get_lens_input_text());
    match text_winner(&agreed, &here, &there) {
        Some(Side::Window) => {
            ui.set_lens_input_text(here.clone());
            *agreed = here;
        }
        Some(Side::Shell) => {
            lens.window.set_lens_input_text(there.clone());
            *agreed = there;
        }
        None => {}
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Side {
    Window,
    Shell,
}

/// Which side's text to keep, given what both last agreed on: the window's typing first, since a
/// person typing is the one write that must never be lost; `None` when they agree.
pub(crate) fn text_winner(agreed: &str, window: &str, shell: &str) -> Option<Side> {
    if window == shell {
        None
    } else if window != agreed {
        Some(Side::Window)
    } else {
        Some(Side::Shell)
    }
}

/// The window's presses, each handed to the App callback the Lens called when it was drawn there.
fn forward(w: &LensWindow, ui: &App) {
    let weak = w.as_weak();
    let shell = ui.as_weak();
    // The shell, with the field's text brought across first, for the presses that read it.
    let with = move |f: &dyn Fn(&App)| {
        if let Some(ui) = shell.upgrade() {
            if let Some(w) = weak.upgrade() {
                let text = w.get_lens_input_text();
                if ui.get_lens_input_text() != text {
                    ui.set_lens_input_text(text.clone());
                }
                // Agreed now, so the next tick does not read the shell's own change to the
                // field (a submit clearing it) as the window's.
                LENS.with(|slot| {
                    if let Some(lens) = slot.borrow().as_ref() {
                        *lens.text.borrow_mut() = text;
                    }
                });
            }
            f(&ui);
        }
    };
    let with = std::rc::Rc::new(with);
    macro_rules! on {
        ($on:ident, |$ui:ident $(, $arg:ident)*| $body:expr) => {{
            let with = with.clone();
            w.$on(move |$($arg),*| (*with)(&|$ui: &App| { $(let $arg = $arg.clone();)* $body; }));
        }};
    }
    on!(on_lens_query, |ui, text| ui.invoke_lens_query(text));
    on!(on_lens_submit, |ui, text| ui.invoke_lens_submit(text));
    on!(on_lens_result_selected, |ui, id| ui.invoke_lens_result_selected(id));
    on!(on_close_lens, |ui| {
        ui.set_lens_open(false);
        ui.invoke_close_lens();
        sync(ui)
    });
    on!(on_send_message, |ui, text| ui.invoke_send_message(text));
    on!(on_mic_pressed, |ui| ui.invoke_mic_pressed());
    on!(on_approval_allow, |ui, id| ui.invoke_approval_allow(id));
    on!(on_approval_allow_session, |ui, id| ui.invoke_approval_allow_session(id));
    on!(on_approval_deny, |ui, id| ui.invoke_approval_deny(id));
    // A pointer's press on an answer, and the only caller of this function (tested in
    // wire/agents.rs, `nothing_a_mind_can_call_answers_a_question`).
    on!(on_question_answer, |ui, agent, request, answer, pressed| ui.invoke_lens_question_answer(agent, request, answer, pressed));
    on!(on_lens_new_chat, |ui| ui.invoke_lens_new_chat());
    // Mind View is an ordinary window on the person's desktop; the switcher brings it in front.
    on!(on_view_desk, |ui, _run| ui.invoke_switch_window(crate::mind_view::TITLE.into()));
    // These open something drawn in the shell's own window — the Agents screen, the mode menu, the
    // Minds panel — which is still one fullscreen window until phase 2 of #733, so it is brought
    // forward for them, as `show_screen` does. The Lens stays up beside it.
    on!(on_open_run, |ui, run| {
        let leaving_private = run.as_str() == crate::private_mode::LEAVE_LINK;
        ui.invoke_lens_open_run(run);
        if !leaving_private {
            raise_shell_for("a run from the Lens");
        }
    });
    on!(on_lens_open_in_agents, |ui| {
        ui.invoke_lens_open_in_agents();
        raise_shell_for("the Agents screen");
    });
    on!(on_lens_open_history, |ui| {
        ui.invoke_lens_open_history();
        raise_shell_for("the conversation history");
    });
    on!(on_mode_clicked, |ui| {
        ui.invoke_toggle_mind_menu();
        if ui.get_mind_menu_open() {
            raise_shell_for("the mode menu");
        }
    });
    on!(on_mind_clicked, |ui| {
        ui.invoke_toggle_minds_panel();
        if ui.get_minds_panel_open() {
            raise_shell_for("the Minds panel");
        }
    });
}

/// Off the UI thread: wlrctl is a process.
fn raise_shell_for(what: &'static str) {
    std::thread::spawn(move || {
        if let Err(why) = crate::windows::raise_shell() {
            tracing::warn!(%why, "{what} opened in the shell's window, which could not be brought in front");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lens_open_is_the_lens_and_a_card_alone_is_the_card() {
        assert_eq!(mode(true, false, 1), Mode::Lens);
        assert_eq!(mode(true, true, 7), Mode::Lens, "a card waiting while the Lens is open is drawn in it");
        assert_eq!(mode(false, true, 8), Mode::Card);
        assert_eq!(mode(false, false, 1), Mode::Hidden);
    }

    /// The window is above every other, the lock screen's included: up there, it would be a way
    /// past the lock (#203).
    #[test]
    fn never_on_the_boot_onboarding_lock_or_login_screens() {
        for screen in LOCKED_OUT {
            assert_eq!(mode(true, true, screen), Mode::Hidden, "screen {screen}");
        }
        let app = include_str!("../../yantrik-ui-slint/ui/app.slint");
        let corner = app.split("property <bool> approval-corner-shown:").nth(1).unwrap().split(';').next().unwrap();
        for screen in LOCKED_OUT {
            assert!(corner.contains(&format!("root.current-screen != {screen}")), "the corner keeps off {screen} too");
        }
    }

    #[test]
    fn opening_the_lens_takes_the_keyboard_and_a_card_hands_it_back() {
        assert_eq!(step(Mode::Hidden, Mode::Lens), Step::Show { give_back: false });
        assert_eq!(step(Mode::Hidden, Mode::Card), Step::Show { give_back: true });
        assert_eq!(step(Mode::Card, Mode::Lens), Step::Remap);
    }

    #[test]
    fn closing_the_lens_gives_the_keyboard_back() {
        assert_eq!(step(Mode::Lens, Mode::Hidden), Step::Hide);
        assert_eq!(step(Mode::Card, Mode::Hidden), Step::Hide);
        assert_eq!(step(Mode::Lens, Mode::Card), Step::Shrink, "closed with a card waiting, the card stays");
        for m in [Mode::Hidden, Mode::Card, Mode::Lens] {
            assert_eq!(step(m, m), Step::Nothing, "a tick that changes nothing does nothing");
        }
    }

    #[test]
    fn the_keyboard_goes_back_to_the_window_that_had_it_never_to_a_shell_surface() {
        assert_eq!(give_back_to(Some((7, "Notes".into()))), Some(7));
        assert_eq!(give_back_to(Some((1, crate::windows::SHELL_WINDOW_TITLE.into()))), Some(1), "the desktop, when that is where they were");
        assert_eq!(give_back_to(Some((9, LENS_WINDOW_TITLE.into()))), None);
        assert_eq!(give_back_to(Some((9, crate::windows::OSD_WINDOW_TITLE.into()))), None);
        assert_eq!(give_back_to(None), None);
    }

    #[test]
    fn typing_in_the_lens_wins_over_a_stale_shell_and_a_prefill_reaches_the_lens() {
        assert_eq!(text_winner("", "hel", ""), Some(Side::Window));
        assert_eq!(text_winner("", "", "what is using my disk"), Some(Side::Shell));
        assert_eq!(text_winner("a", "ab", "ac"), Some(Side::Window), "both moved: the person's typing is kept");
        assert_eq!(text_winner("a", "b", "b"), None);
    }

    #[test]
    fn the_window_fills_the_room_between_the_bars() {
        assert_eq!(usable_height(800.0), 716.0);
        let theme = include_str!("../../yantrik-design-tokens/slint/theme.slint");
        assert!(theme.contains(&format!("status-bar-height: {}px;", STATUS_BAR as i32)));
        assert!(theme.contains(&format!("taskbar-height: {}px;", DOCK as i32)));
        assert!(theme.contains("chat-panel-width: 440px;"), "the Lens is 440px wide");
        let rc = include_str!("../../../config/labwc/rc.xml");
        assert!(rc.contains(&format!("<margin top=\"{}\" bottom=\"{}\"", STATUS_BAR as i32, DOCK as i32)));
    }

    /// The name is the one thing the window, the rule and the filters must agree on.
    #[test]
    fn the_window_the_rule_and_the_filters_use_one_name() {
        let slint = include_str!("../../yantrik-ui-slint/ui/lens_window.slint");
        assert!(slint.contains(&format!("title: \"{LENS_WINDOW_TITLE}\";")));
        let rc = include_str!("../../../config/labwc/rc.xml");
        let rule = rc
            .split("<windowRule ")
            .find(|r| r.starts_with(&format!("title=\"{LENS_WINDOW_TITLE}\"")))
            .expect("rc.xml has a windowRule for the Lens's window")
            .split("</windowRule>")
            .next()
            .unwrap();
        for wanted in [
            "serverDecoration=\"no\"",
            "skipTaskbar=\"yes\"",
            "skipWindowSwitcher=\"yes\"",
            "<action name=\"ToggleAlwaysOnTop\" />",
            "<action name=\"MoveToEdge\" direction=\"right\" snapWindows=\"no\" />",
            "<action name=\"MoveToEdge\" direction=\"up\" snapWindows=\"no\" />",
        ] {
            assert!(rule.contains(wanted), "the Lens's rule has {wanted}:\n{rule}");
        }
        assert!(crate::windows::is_shell_surface(LENS_WINDOW_TITLE));
    }

    /// One Lens: `IntentLens` is drawn by its window and nowhere else in the shell.
    #[test]
    fn the_lens_is_drawn_in_its_own_window_and_only_there() {
        let ui = concat!(env!("CARGO_MANIFEST_DIR"), "/../yantrik-ui-slint/ui");
        let mut hosts = Vec::new();
        for entry in walk(std::path::Path::new(ui)) {
            let text = std::fs::read_to_string(&entry).unwrap();
            if text.lines().any(|l| l.trim_start().starts_with("IntentLens {") || l.contains(":= IntentLens {")) {
                hosts.push(entry.strip_prefix(ui).unwrap().display().to_string());
            }
        }
        assert_eq!(hosts, vec!["lens_window.slint".to_string()]);
        let app = include_str!("../../yantrik-ui-slint/ui/app.slint");
        assert!(app.contains("export { LensWindow } from \"lens_window.slint\";"));
    }

    fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(walk(&p));
            } else if p.extension().is_some_and(|x| x == "slint") {
                out.push(p);
            }
        }
        out.sort();
        out
    }

    /// Opening the Lens raises nothing but the Lens: the shell's window stays where it is.
    #[test]
    fn opening_the_lens_does_not_raise_the_shell() {
        let lens = include_str!("wire/lens.rs");
        let open = lens.split("ui.on_open_lens(").nth(1).unwrap().split("ui.on_close_lens(").next().unwrap();
        assert!(!open.contains("raise_shell"), "open_lens must not raise the shell:\n{open}");
        assert!(open.contains("crate::lens_window::sync(&ui)"));
        let control = include_str!("control.rs");
        let action = control.split("Action::new(\"open_lens\"").nth(1).unwrap().split(".action(").next().unwrap();
        assert!(!action.contains("raise_shell") && !action.contains("title:Yantrik OS"), "nor does `open_lens`");
    }
}
