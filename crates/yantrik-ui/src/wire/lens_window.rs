//! The Lens's own window (#733): shown while the App says the Lens is up, hidden when it is put
//! away, and nothing else moved either way.
//!
//! The Lens used to be drawn inside the shell's one fullscreen window, so every opener raised that
//! window over every app, and an approval card in the Lens did the same: the person's windows all
//! went behind the desktop at once. Now it is `LensWindow` (lens_window.slint), a strip on the
//! right that the compositor keeps above the others (the windowRule for its title in
//! config/labwc/rc.xml), and opening it covers only that strip.
//!
//! The App still holds the Lens's state, so the twenty-odd places that open, close, fill and
//! answer it are unchanged. It reads that state into one value, `lens-view`, and says when it
//! changes; this copies it into the window, shows or hides the window on the edge of `shown`, and
//! forwards each of the window's callbacks to the App's own. One Lens, drawn in one place.
//!
//! Showing it borrows the keyboard from the window the person was using and hiding it gives the
//! keyboard back (lens_focus.rs decides which, from `toplevel_watch`).

use std::cell::RefCell;

use slint::ComponentHandle;

use crate::lens_focus::{self, Opening, ShellShowing};
use crate::{App, LensWindow};

thread_local! {
    static LENS: RefCell<Option<Lens>> = const { RefCell::new(None) };
}

struct Lens {
    window: LensWindow,
    /// Whose keyboard the window borrowed when it last showed.
    opening: Opening,
    /// A press in the Lens just handed the person to one of the shell's own screens or menus,
    /// and raised the shell for it: the close that follows leaves the keyboard with the shell.
    to_shell: bool,
    /// The App's `lens-input-text` as last copied into the field. The field is written only when
    /// the App's value moves off this, so a keystroke whose `input-edited` has not reached the App
    /// yet is never overwritten by a sync that something else (a streamed reply) set off.
    pushed: slint::SharedString,
}

pub fn wire(ui: &App) {
    let window = match LensWindow::new() {
        Ok(w) => w,
        Err(e) => {
            tracing::warn!(error = %e, "no window for the Lens; it cannot be opened this session");
            return;
        }
    };
    // The shell runs under SLINT_FULLSCREEN=1 and every window of this process reads it, as the
    // agents' pop-out found (#231).
    window.window().set_fullscreen(false);
    forward(&window, ui);
    // Closed by the compositor (Alt+F4 reaches it if it ever has focus): the same as Escape.
    window.window().on_close_requested({
        let weak = ui.as_weak();
        move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_lens_open(false);
                ui.invoke_close_lens();
            }
            slint::CloseRequestResponse::HideWindow
        }
    });
    LENS.with(|cell| *cell.borrow_mut() = Some(Lens { window, opening: Opening::default(), to_shell: false, pushed: Default::default() }));
    ui.on_lens_view_changed({
        let weak = ui.as_weak();
        move || {
            if let Some(ui) = weak.upgrade() {
                sync(&ui);
            }
        }
    });
    sync(ui);
}

/// Copy the App's `lens-view` into the window, and show or hide it on the edge of `shown`.
fn sync(ui: &App) {
    let Some(window) = LENS.with(|cell| cell.borrow().as_ref().map(|l| l.window.clone_strong())) else { return };
    let view = ui.get_lens_view();
    super::theme_sync::sync_theme!(ui, window);
    let app_moved = LENS.with(|cell| {
        let mut cell = cell.borrow_mut();
        let lens = cell.as_mut()?;
        (lens.pushed != view.input_text).then(|| lens.pushed = view.input_text.clone())
    });
    if app_moved.is_some() && window.get_input_text() != view.input_text {
        window.set_input_text(view.input_text.clone());
    }
    let shown = view.shown;
    crate::card_watch::lens_up(shown && view.chat_mode);
    window.set_view(view);
    let up = window.window().is_visible();
    if shown && !up {
        show(&window);
    } else if !shown && up {
        hide(&window, crate::control::locked_screen(ui.get_current_screen()));
    }
}

fn show(window: &LensWindow) {
    let opening = crate::toplevel_watch::focus_now()
        .map(|seen| lens_focus::on_open(&seen, crate::windows::SHELL_WINDOW_TITLE))
        .unwrap_or_default();
    // A press on the dock raised the fullscreen shell over the window the person was using; put
    // that window back before the Lens comes up beside it.
    if let Some(id) = opening.put_back {
        crate::toplevel_watch::activate(id);
    }
    LENS.with(|cell| {
        if let Some(lens) = cell.borrow_mut().as_mut() {
            lens.opening = opening;
            lens.to_shell = false;
        }
    });
    if let Err(e) = window.show() {
        tracing::warn!(error = %e, "could not show the Lens's window");
    }
}

/// `locked`: the Lens went away because the screen locked. Nothing is handed the keyboard then:
/// activating the person's app could bring it over the lock screen (#203).
fn hide(window: &LensWindow, locked: bool) {
    if let Err(e) = window.hide() {
        tracing::warn!(error = %e, "could not hide the Lens's window");
    }
    let (opening, to_shell) = LENS.with(|cell| {
        cell.borrow_mut()
            .as_mut()
            .map(|lens| (std::mem::take(&mut lens.opening), std::mem::take(&mut lens.to_shell)))
            .unwrap_or_default()
    });
    if to_shell || locked {
        return;
    }
    let front = crate::toplevel_watch::focus_now().and_then(|f| f.current).map(|(id, _)| id);
    let Some(id) = lens_focus::on_close(&opening, front) else { return };
    if !crate::toplevel_watch::activate(id) {
        // Not followed, or the id is gone: by its title, off this thread (wlrctl is a process).
        if let Some((_, title)) = opening.return_to {
            std::thread::spawn(move || {
                crate::windows::present(&title);
            });
        }
    }
}

/// What the shell is showing right now: its screen, and whether one of its menus is open.
fn showing(ui: &App) -> ShellShowing {
    ShellShowing { screen: ui.get_current_screen(), menu: ui.get_mind_menu_open() || ui.get_minds_panel_open() }
}

/// Run one of the Lens's presses on the App. If it put one of the shell's screens or menus in
/// front of the person, the shell's window comes forward for it: those are still drawn in the
/// fullscreen window (#733 phase 2 moves the screens out).
fn press(weak: &slint::Weak<App>, act: impl FnOnce(&App)) {
    let Some(ui) = weak.upgrade() else { return };
    let before = showing(&ui);
    act(&ui);
    if !lens_focus::shows_the_shell(before, showing(&ui)) {
        return;
    }
    LENS.with(|cell| {
        if let Some(lens) = cell.borrow_mut().as_mut() {
            lens.to_shell = true;
        }
    });
    std::thread::spawn(|| {
        if let Err(why) = crate::windows::raise_shell() {
            tracing::warn!(%why, "the Lens opened one of the shell's screens, but the shell could not come forward");
        }
    });
}

/// Every callback of the window, to the App's own.
fn forward(window: &LensWindow, ui: &App) {
    // Typing: the App's `lens-input-text` follows the field, as the two-way binding did.
    window.on_input_edited({
        let weak = ui.as_weak();
        move |text| {
            if let Some(ui) = weak.upgrade() {
                if ui.get_lens_input_text() != text {
                    ui.set_lens_input_text(text);
                }
            }
        }
    });
    // Escape, the header's ×: the pair the desktop screen ran.
    window.on_close_lens({
        let weak = ui.as_weak();
        move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_lens_open(false);
                ui.invoke_close_lens();
            }
        }
    });
    macro_rules! to_app {
        ($on:ident => $invoke:ident ( $($arg:ident),* )) => {{
            let weak = ui.as_weak();
            window.$on(move |$($arg),*| press(&weak, |ui| ui.$invoke($($arg),*)));
        }};
    }
    to_app!(on_query_changed => invoke_lens_query(text));
    to_app!(on_submit_query => invoke_lens_submit(text));
    to_app!(on_result_selected => invoke_lens_result_selected(id));
    to_app!(on_send_message => invoke_send_message(text));
    to_app!(on_mic_pressed => invoke_mic_pressed());
    to_app!(on_approval_allow => invoke_approval_allow(id));
    to_app!(on_approval_allow_session => invoke_approval_allow_session(id));
    to_app!(on_approval_deny => invoke_approval_deny(id));
    to_app!(on_question_answer => invoke_lens_question_answer(agent, request, answer, pressed));
    to_app!(on_open_run => invoke_lens_open_run(run));
    to_app!(on_open_in_agents => invoke_lens_open_in_agents());
    to_app!(on_open_history => invoke_lens_open_history());
    to_app!(on_new_chat => invoke_lens_new_chat());
    to_app!(on_mode_clicked => invoke_lens_mode_clicked());
    to_app!(on_mind_clicked => invoke_toggle_minds_panel());
    to_app!(on_view_desk => invoke_lens_view_desk(run));
}

#[cfg(test)]
mod tests {
    fn read(path: &str) -> String {
        let at = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
        std::fs::read_to_string(&at).unwrap_or_else(|e| panic!("cannot read {}: {e}", at.display()))
    }

    /// The compositor's rule for the Lens's window (labwc 0.8, labwc-config(5), WINDOW RULES).
    fn rule() -> String {
        let rc = read("../../config/labwc/rc.xml");
        let title = crate::windows::LENS_WINDOW_TITLE;
        let after = rc
            .split(&format!("<windowRule title=\"{title}\""))
            .nth(1)
            .unwrap_or_else(|| panic!("rc.xml has a windowRule for title=\"{title}\""));
        after.split("</windowRule>").next().unwrap().to_string()
    }

    /// One name for the window in the three places that must agree: the window's own title, the
    /// shell's constant (which keeps it out of every window list), and the rule that places it.
    #[test]
    fn the_window_the_rule_and_the_window_list_name_the_same_lens() {
        let slint = read("../yantrik-ui-slint/ui/lens_window.slint");
        let title = crate::windows::LENS_WINDOW_TITLE;
        assert!(slint.contains(&format!("title: \"{title}\";")), "lens_window.slint's title is {title}");
        assert_eq!(title, "yantrik-lens");
        assert!(crate::windows::is_shell_furniture(title), "the window lists never offer it");
        assert!(!rule().is_empty());
    }

    /// No title bar, never in the taskbar or Alt+Tab, above the person's windows, and in the
    /// strip on the right between the bar and the dock — each with labwc 0.8's own spelling.
    #[test]
    fn the_rule_keeps_the_lens_on_top_undecorated_and_on_the_right() {
        let rule = rule();
        for property in ["serverDecoration=\"no\"", "skipTaskbar=\"yes\"", "skipWindowSwitcher=\"yes\""] {
            assert!(rule.contains(property), "the rule sets {property}:\n{rule}");
        }
        for action in [
            "<action name=\"ToggleAlwaysOnTop\" />",
            "<action name=\"MoveToEdge\" direction=\"right\" snapWindows=\"no\" />",
            "<action name=\"MoveToEdge\" direction=\"up\" snapWindows=\"no\" />",
        ] {
            assert!(rule.contains(action), "the rule runs {action}:\n{rule}");
        }
    }

    /// The window's size is the strip between the bar and the dock that rc.xml's <margin> keeps
    /// clear, at the panel's width plus the gap it keeps from the edge.
    #[test]
    fn the_window_is_the_strip_between_the_bar_and_the_dock() {
        let slint = read("../yantrik-ui-slint/ui/lens_window.slint");
        assert!(slint.contains("width: Theme.chat-panel-width + Theme.chat-panel-gap;"));
        assert!(slint.contains("root.view.screen-height - Theme.status-bar-height - Theme.taskbar-height"));
        let rc = read("../../config/labwc/rc.xml");
        assert!(rc.contains("<margin top=\"36\" bottom=\"48\""), "the bar and dock heights the margin keeps");
    }

    /// One Lens: the App no longer draws one, the desktop screen no longer draws one, and the
    /// window is the only place IntentLens is put on screen in the shell.
    #[test]
    fn the_lens_is_drawn_once_in_its_own_window() {
        let ui = |f: &str| read(&format!("../yantrik-ui-slint/ui/{f}"));
        assert!(!ui("desktop.slint").contains("IntentLens {"), "the desktop screen no longer draws the Lens");
        assert!(!ui("app.slint").contains("IntentLens {"), "nor does the App");
        assert_eq!(ui("lens_window.slint").matches("IntentLens {").count(), 1);
        assert!(ui("app.slint").contains("export { LensWindow, LensView } from \"lens_window.slint\";"));
    }

    /// Showing the Lens raises nothing of the shell's: neither the App's handler for it nor the
    /// control surface's `open_lens` asks the compositor to bring the shell forward.
    #[test]
    fn opening_the_lens_does_not_raise_the_shell() {
        let lens = read("src/wire/lens.rs");
        let open = lens.split("ui.on_open_lens(").nth(1).unwrap().split("ui.on_close_lens(").next().unwrap();
        assert!(!open.contains("raise_shell"), "on_open_lens raises nothing:\n{open}");
        let control = read("src/control.rs");
        let action = control.split("Action::new(\"open_lens\"").nth(1).unwrap().split(".action(").next().unwrap();
        assert!(!action.contains("wlrctl") && !action.contains("raise_shell"), "open_lens raises nothing:\n{action}");
        assert!(!action.contains("set_current_screen"), "and leaves the screen where it is:\n{action}");
    }
}
