//! The bar's panels, operable as data: Today (the clock's), Quick Settings, the power menu, the
//! clipboard, and the keyboard cheat sheet (Super+/).
//!
//! A person opens them by pressing the bar's network, settings and power buttons or Super+V, and
//! they open over whatever screen is up. A mind can ask for the same panels, so what a pointer
//! can do it can do too, and `describe shell` says which one is on the screen so nothing has to
//! be photographed to find out (`quick_settings`, `power_menu`, `clipboard_panel`, `cheat_sheet`).
//!
//! All ten actions are `safe`. Opening the power menu does not power anything off: it shows the
//! menu, and choosing an entry in it is a separate act that is the person's. Nothing here is
//! chosen, sent, pasted or written; a panel is shown or put away.
//!
//! Each open asks the compositor to bring the shell in front, as `show_screen` does, and says in
//! the answer whether it did: the panels are drawn by the shell's own window, and with an app
//! window over it the flag would be set and nobody would see anything. The `shell-overlay-opened`
//! hook does the same for a click or a keybind; the action asks itself because it has to answer
//! with what happened, and the hook runs a moment after the flag changes.

use slint::ComponentHandle;
use yantrik_app_runtime::control::{Action, App as ControlSurface};

use crate::App;

/// The screens the bar is not drawn on: boot, onboarding, lock and login. A panel flag set there
/// would show the moment the person got past them, so nothing opens one.
const NO_BAR: [i32; 4] = [0, 2, 3, 32];

/// Whether the bar, and so the panels it opens, is on this screen.
pub(crate) fn bar_is_drawn(screen: i32) -> bool {
    !NO_BAR.contains(&screen)
}

#[derive(Clone, Copy, PartialEq)]
enum Panel {
    Today,
    QuickSettings,
    PowerMenu,
    Clipboard,
    CheatSheet,
}

impl Panel {
    const ALL: [Panel; 5] = [Panel::Today, Panel::QuickSettings, Panel::PowerMenu, Panel::Clipboard, Panel::CheatSheet];

    fn name(self) -> &'static str {
        match self {
            Panel::Today => "today panel",
            Panel::QuickSettings => "quick settings",
            Panel::PowerMenu => "power menu",
            Panel::Clipboard => "clipboard panel",
            Panel::CheatSheet => "cheat sheet",
        }
    }

    fn flag(self, ui: &App) -> bool {
        match self {
            Panel::Today => ui.get_today_open(),
            Panel::QuickSettings => ui.get_quick_settings_open(),
            Panel::PowerMenu => ui.get_power_menu_open(),
            Panel::Clipboard => ui.get_clip_panel_open(),
            Panel::CheatSheet => ui.get_cheat_sheet_open(),
        }
    }

    fn set(self, ui: &App, open: bool) {
        match self {
            Panel::Today => ui.set_today_open(open),
            Panel::QuickSettings => ui.set_quick_settings_open(open),
            Panel::PowerMenu => ui.set_power_menu_open(open),
            Panel::Clipboard => ui.set_clip_panel_open(open),
            Panel::CheatSheet => ui.set_cheat_sheet_open(open),
        }
    }
}

/// What `describe shell` says under each panel's key: whether it is on the screen. Drawn, not
/// merely asked for, so it is false where the bar is not.
pub fn panel_for_describe(open: bool, screen: i32, close_with: &str) -> serde_json::Value {
    let drawn = open && bar_is_drawn(screen);
    serde_json::json!({
        "open": drawn,
        "close_with": if drawn { close_with } else { "" },
    })
}

/// What `describe shell` says under `bar_minds`: the top bar's one Minds chip, as a person reads it.
///
/// `needing` counts the minds with an unresolved request and `requests` the requests, the two numbers
/// the chip is drawn from. The label and tooltip are built here from them in the chip's own words
/// ("Minds · 2 need you", "1 mind has 3 requests"), and `opens` says where a click goes: Agents, on
/// Needs you when there is something to answer. A mind that reads this knows what the person is
/// looking at without being told what any request says.
pub fn bar_minds_for_describe(needing: i32, requests: i32) -> serde_json::Value {
    let (needing, requests) = (needing.max(0), requests.max(0));
    let label = match needing {
        0 => "Minds".to_string(),
        1 => "Minds · 1 needs you".to_string(),
        n => format!("Minds · {n} need you"),
    };
    let tooltip = match (needing, requests) {
        (0, _) => "No mind needs you".to_string(),
        (1, 1) => "1 mind has 1 request".to_string(),
        (1, r) => format!("1 mind has {r} requests"),
        (n, r) => format!("{n} minds have {r} requests"),
    };
    serde_json::json!({
        "label": label,
        "tooltip": tooltip,
        "minds_needing_you": needing,
        "requests": requests,
        "opens": if needing > 0 { "agents: needs_you" } else { "agents: workroom" },
    })
}

/// Show one panel, put the others away so only one is up, bring the shell forward, and answer
/// with what was observed afterwards.
fn open_panel(ui: &App, panel: Panel) -> Result<serde_json::Value, String> {
    // The panels are drawn under the approval overlay but over the Lens, so a card shown in the
    // Lens's chat could be covered by one a mind opened (final review of the card fix). Held
    // while a card waits; putting a panel away never is. Each name written out, so the test that
    // reads the source for the hold finds it.
    match panel {
        Panel::Today => crate::card_watch::hold_windows("open_today")?,
        Panel::QuickSettings => crate::card_watch::hold_windows("open_quick_settings")?,
        Panel::PowerMenu => crate::card_watch::hold_windows("open_power_menu")?,
        Panel::Clipboard => crate::card_watch::hold_windows("open_clipboard")?,
        Panel::CheatSheet => crate::card_watch::hold_windows("open_cheat_sheet")?,
    }
    let screen = ui.get_current_screen();
    if !bar_is_drawn(screen) {
        return Err(format!(
            "the {} opens from the bar, and the bar is not on `{}`",
            panel.name(),
            crate::control::screen_name(screen)
        ));
    }
    for other in Panel::ALL {
        if other != panel {
            other.set(ui, false);
        }
    }
    panel.set(ui, true);
    let mut answer = serde_json::json!({
        "open": panel.flag(ui),
        "screen": crate::control::screen_name(screen),
    });
    match crate::windows::raise_shell() {
        Ok(()) => answer["raised"] = true.into(),
        // Not an error: the panel IS open. What it is not is visible, and a caller told "open"
        // is owed that difference.
        Err(why) => {
            answer["raised"] = false.into();
            answer["note"] = format!(
                "the {} is open, but the shell's own window could not be brought to the front, \
                 so an app window may still be covering it: {why}",
                panel.name()
            )
            .into();
        }
    }
    Ok(answer)
}

/// Put one panel away. Nothing is lowered: the person is looking at the shell.
fn close_panel(ui: &App, panel: Panel) -> serde_json::Value {
    let was = panel.flag(ui);
    panel.set(ui, false);
    serde_json::json!({
        "open": panel.flag(ui),
        "closed": was,
        "note": if was {
            format!("the {} is put away; nothing was changed.", panel.name())
        } else {
            format!("the {} was not open; nothing was changed.", panel.name())
        },
    })
}

/// Add the ten panel actions to the shell's surface.
pub fn actions(surface: ControlSurface, ui: &App) -> ControlSurface {
    // Each action carries its own literal name, so the control tests that read the source for
    // `Action::new("name"` find them; the handlers are the same two functions.
    let handlers = |panel: Panel| {
        let open_weak = ui.as_weak();
        let close_weak = ui.as_weak();
        (
            move |_args: &serde_json::Value| {
                let ui = open_weak.upgrade().ok_or_else(|| "the shell is gone".to_string())?;
                open_panel(&ui, panel)
            },
            move |_args: &serde_json::Value| {
                let ui = close_weak.upgrade().ok_or_else(|| "the shell is gone".to_string())?;
                Ok(close_panel(&ui, panel))
            },
        )
    };
    let (open_td, close_td) = handlers(Panel::Today);
    let (open_qs, close_qs) = handlers(Panel::QuickSettings);
    let (open_pm, close_pm) = handlers(Panel::PowerMenu);
    let (open_cb, close_cb) = handlers(Panel::Clipboard);
    let (open_cs, close_cs) = handlers(Panel::CheatSheet);

    surface
        .action(
            Action::new(
                "open_today",
                "Show Today, the panel that drops from the clock: the date, this month's calendar, \
                 today's events, the five newest notifications with their buttons and the Do Not \
                 Disturb switch, over whatever screen is up, and bring the shell in front of any app \
                 window. Nothing is changed: the switch and the buttons are the person's to press. \
                 Only one of the bar's panels is up at a time. `describe shell` says whether it is \
                 open, under `today`.",
            )
            .risk("safe"),
            open_td,
        )
        .action(
            Action::new(
                "close_today",
                "Put away Today. It changes nothing else and does not send the shell behind an app \
                 window.",
            )
            .risk("safe"),
            close_td,
        )
        .action(
            Action::new(
                "open_quick_settings",
                "Show Quick Settings, the network, volume, brightness and battery panel that drops \
                 from the bar, over whatever screen is up, and bring the shell in front of any app \
                 window. Only one of the bar's panels is up at a time. `describe shell` says \
                 whether it is open, under `quick_settings`.",
            )
            .risk("safe"),
            open_qs,
        )
        .action(
            Action::new(
                "close_quick_settings",
                "Put away Quick Settings. It changes nothing else and does not send the shell \
                 behind an app window.",
            )
            .risk("safe"),
            close_qs,
        )
        .action(
            Action::new(
                "open_power_menu",
                "Show the power menu (lock, suspend, restart, shut down), over whatever screen is \
                 up, and bring the shell in front of any app window. Showing it does none of them: \
                 choosing an entry is the person's. `describe shell` says whether it is open, \
                 under `power_menu`.",
            )
            .risk("safe"),
            open_pm,
        )
        .action(
            Action::new(
                "close_power_menu",
                "Put away the power menu without choosing anything.",
            )
            .risk("safe"),
            close_pm,
        )
        .action(
            Action::new(
                "open_clipboard",
                "Show the clipboard history panel, newest first with its search empty, over \
                 whatever screen is up, and bring the shell in front of any app window. Nothing is \
                 pasted. `describe shell` says whether it is open, under `clipboard_panel`.",
            )
            .risk("safe"),
            open_cb,
        )
        .action(
            Action::new(
                "close_clipboard",
                "Put away the clipboard history panel without pasting anything.",
            )
            .risk("safe"),
            close_cb,
        )
        .action(
            Action::new(
                "open_cheat_sheet",
                "Show the keyboard cheat sheet: every key the desktop binds, grouped as Windows, \
                 Snap, Workspaces, Shell, Minds and Capture, over whatever screen is up, and bring \
                 the shell in front of any app window. It is read from the compositor's own key \
                 file, so it lists exactly the keys that work. `describe shell` says whether it is \
                 open, under `cheat_sheet`.",
            )
            .risk("safe"),
            open_cs,
        )
        .action(
            Action::new(
                "close_cheat_sheet",
                "Put away the keyboard cheat sheet. It changes nothing else.",
            )
            .risk("safe"),
            close_cs,
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The part of this file above its tests, which is what the shell runs.
    fn code() -> String {
        let whole = include_str!("control_overlays.rs");
        whole.split("#[cfg(test)]").next().unwrap().to_string()
    }

    /// The declaration of one action, to the next one.
    fn declaration(name: &str) -> String {
        let src = code();
        let at = src
            .find(&format!("\"{name}\","))
            .unwrap_or_else(|| panic!("`{name}` is no longer published"));
        let rest = &src[at..];
        rest[..rest.find("Action::new(").unwrap_or(rest.len())].to_string()
    }

    /// The function `name`, to its closing brace.
    fn function(name: &str) -> String {
        let src = code();
        let from = src.find(&format!("fn {name}(")).unwrap_or_else(|| panic!("no fn {name}"));
        let body = &src[from..];
        body[..body.find("\n}\n").unwrap()].to_string()
    }

    /// The panels are the bar's, so they have no business on the screens the bar is not on, and
    /// the lock and login screens are held shut by the dispatch before any of this runs.
    #[test]
    fn the_bar_is_on_every_screen_but_the_four_before_the_desktop() {
        for gone in [0, 2, 3, 32] {
            assert!(!bar_is_drawn(gone), "screen {gone} has no bar");
        }
        // The desktop and the screens the bug was seen on: Files, Settings, Agents.
        for shown in [1, 7, 8, 34] {
            assert!(bar_is_drawn(shown), "screen {shown} has the bar");
        }
    }

    /// Asked for is not on the screen: where the bar is not drawn the flag draws nothing, so
    /// `describe` must not say open.
    #[test]
    fn describe_says_open_only_when_the_panel_is_drawn() {
        let on_files = panel_for_describe(true, 8, "close_power_menu");
        assert_eq!(on_files["open"], true);
        assert_eq!(on_files["close_with"], "close_power_menu");
        let shut = panel_for_describe(false, 8, "close_power_menu");
        assert_eq!(shut["open"], false);
        assert_eq!(shut["close_with"], "");
        let on_lock = panel_for_describe(true, 3, "close_power_menu");
        assert_eq!(on_lock["open"], false, "a flag set under the lock screen is not a panel anyone sees");
    }

    /// All ten are `safe`: they show or put away a panel and nothing else.
    #[test]
    fn all_ten_are_published_and_safe() {
        for name in [
            "open_today",
            "close_today",
            "open_quick_settings",
            "close_quick_settings",
            "open_power_menu",
            "close_power_menu",
            "open_clipboard",
            "close_clipboard",
            "open_cheat_sheet",
            "close_cheat_sheet",
        ] {
            assert!(declaration(name).contains(".risk(\"safe\")"), "`{name}` must be graded safe");
        }
    }

    /// Final review of the card fix: a panel opened over the Lens covered the card in its chat.
    /// Opening waits for the card, before anything is set; putting a panel away uncovers it and
    /// is never held.
    #[test]
    fn opening_a_panel_waits_for_a_card_and_closing_one_never_does() {
        let open = function("open_panel");
        let held = open.find("card_watch::hold_windows(").expect("open_panel asks hold_windows");
        assert!(held < open.find("panel.set(ui, true)").unwrap(), "asked before the panel is drawn");
        assert!(!function("close_panel").contains("hold_windows"), "putting a panel away is never held");
    }

    /// A panel opened behind an app window is a panel nobody sees, and a caller told "open"
    /// would be believed. Every open asks the compositor and answers whether it worked.
    #[test]
    fn opening_a_panel_raises_the_shell_and_says_so_when_it_could_not() {
        let body = function("open_panel");
        assert!(body.contains("windows::raise_shell()"), "open_panel must raise the shell. As written:\n{body}");
        assert!(body.contains("\"raised\""), "the answer must carry whether the shell came forward. As written:\n{body}");
        assert!(body.contains("answer[\"note\"]"), "when the raise fails the answer says so in words. As written:\n{body}");
        // And every open goes through it, each with the panel it is named for.
        let src = code();
        let handlers = &src[src.find("let handlers").unwrap()..src.find("let (open_qs").unwrap()];
        assert!(handlers.contains("open_panel(&ui, panel)"), "the opens go through `open_panel`");
        for (name, opener) in [
            ("open_today", "open_td"),
            ("open_quick_settings", "open_qs"),
            ("open_power_menu", "open_pm"),
            ("open_clipboard", "open_cb"),
            ("open_cheat_sheet", "open_cs"),
        ] {
            assert!(declaration(name).contains(opener), "`{name}` must use its own opener `{opener}`");
        }
    }

    /// The chip says what it draws: minds counted, requests in the tooltip, and the door it opens.
    #[test]
    fn the_minds_chip_is_described_in_its_own_words() {
        let none = bar_minds_for_describe(0, 0);
        assert_eq!((none["label"].as_str(), none["opens"].as_str()), (Some("Minds"), Some("agents: workroom")));
        let one = bar_minds_for_describe(1, 3);
        assert_eq!(one["label"], "Minds · 1 needs you");
        assert_eq!(one["tooltip"], "1 mind has 3 requests");
        assert_eq!(one["opens"], "agents: needs_you");
        let two = bar_minds_for_describe(2, 5);
        assert_eq!((two["label"].as_str(), two["tooltip"].as_str()), (Some("Minds · 2 need you"), Some("2 minds have 5 requests")));
    }

    /// Opening one puts the others away, so the answer and `describe` are unambiguous.
    #[test]
    fn opening_one_panel_puts_the_others_away() {
        let body = function("open_panel");
        assert!(body.contains("other.set(ui, false)"), "the other panels are closed. As written:\n{body}");
    }

    /// Closing a panel must not lower the shell: the person is looking at it.
    #[test]
    fn closing_a_panel_does_not_lower_anything_or_open_anything() {
        let body = function("close_panel");
        assert!(!body.contains("windows::"), "closing touches no window. As written:\n{body}");
        assert!(!body.contains("true)"), "closing opens nothing. As written:\n{body}");
    }

    /// The shell publishes each panel's state, so a mind reads it rather than photographing.
    #[test]
    fn describe_shell_publishes_the_five_panels() {
        let control = include_str!("control.rs");
        let control: String = control.split("#[cfg(test)]").next().unwrap().split_whitespace().collect();
        for key in ["today", "quick_settings", "power_menu", "clipboard_panel", "cheat_sheet"] {
            assert!(
                control.contains(&format!(".with(\"{key}\",crate::control_overlays::panel_for_describe(")),
                "describe shell must publish `{key}`"
            );
        }
        assert!(
            control.contains("crate::control_overlays::actions(surface,ui)"),
            "the ten actions must be added to the shell's surface"
        );
    }
}
