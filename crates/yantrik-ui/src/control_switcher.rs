//! The Alt+Tab switcher on the shell's control surface, and the glue that draws it.
//!
//! A person's Alt+Tab, the keys inside the switcher, a click on a cell and `yos act shell
//! open_switcher` all end in the functions here, so what a pointer can do a mind can ask for.
//!
//! # Why the shell draws it, and what it cannot do
//!
//! labwc's own window-cycling OSD was what ran before: an unthemed list of titles that the shell
//! could neither style nor read. The shell's overlay (`alt_tab.slint`) can draw the card the spec
//! asks for, so labwc's Alt+Tab binding now runs `yos act shell open_switcher`.
//!
//! What this cannot do is commit on releasing Alt. labwc runs a keybinding when its chord goes
//! down (or, with `onRelease`, when the chord's last key comes up), and never tells a client that
//! a modifier it was not asked about has been let go; the shell is an ordinary client window with
//! no layer-shell surface that could grab the keyboard (a spike is PR #564). So holding Alt and
//! tapping Tab does step the selection — each Tab press is the binding again — but letting go of
//! Alt does nothing. The card is committed with Enter or a click and put away with Escape, and
//! says nothing about release, because it does not do it.
//!
//! The windows are not captured: the compositor's window stream carries titles and focus, not
//! pixels, and no capture path for a single toplevel exists in the shell. Every cell shows its
//! app's icon on a neutral tile (alt_tab.slint).

use std::sync::Mutex;

use slint::{ComponentHandle, ModelRc, VecModel};
use yantrik_app_runtime::control::{Action, App as ControlSurface, Param};

use crate::alt_tab::{Cell, Move, Switcher};
use crate::windows::SHELL_WINDOW_TITLE;
use crate::{App, SwitcherCell};

/// The one open switcher, if any. A mutex rather than a field of the UI because the control
/// handlers and the Slint callbacks both reach it, and neither holds it across a call out.
static OPEN: Mutex<Option<Switcher>> = Mutex::new(None);

fn lock() -> std::sync::MutexGuard<'static, Option<Switcher>> {
    OPEN.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// What `describe shell` says under `window_switcher`: whether it is on the screen, and what it
/// has selected. Drawn, not merely asked for — false where the bar is not.
pub fn for_describe(screen: i32) -> serde_json::Value {
    let open = lock();
    match open.as_ref().filter(|_| crate::control_overlays::bar_is_drawn(screen)) {
        None => serde_json::json!({ "open": false, "close_with": "" }),
        Some(s) => serde_json::json!({
            "open": true,
            "close_with": "switcher_cancel",
            "windows": s.cells().iter().map(|c| c.title.clone()).collect::<Vec<_>>(),
            "selected": s.selected().map(|c| c.title.clone()),
            "page": s.page() + 1,
            "pages": s.pages(),
            "status": s.status(),
            "previews": "unavailable",
        }),
    }
}

/// The open windows as cells: what the compositor lists, the desktop itself, in recency order.
fn gather() -> Vec<Cell> {
    let mut open: Vec<Cell> = crate::windows::shell_windows()
        .into_iter()
        .map(|w| Cell {
            app_name: crate::windows::app_display_name(&w.app_id),
            title: w.title,
            app_id: w.app_id,
        })
        .collect();
    if !open.iter().any(|c| c.title == SHELL_WINDOW_TITLE) {
        open.push(Cell { title: SHELL_WINDOW_TITLE.into(), app_id: "desktop".into(), app_name: "Desktop".into() });
    }
    crate::alt_tab::order(open, &crate::toplevel_watch::recency())
}

/// Hand the page being shown to the card.
fn publish(ui: &App, s: Option<&Switcher>) {
    let Some(s) = s else {
        ui.set_alt_tab_open(false);
        ui.set_alt_tab_cells(ModelRc::default());
        return;
    };
    let cells: Vec<SwitcherCell> = s
        .page_cells()
        .iter()
        .map(|c| SwitcherCell { title: c.title.clone().into(), app_id: c.app_id.clone().into(), app_name: c.app_name.clone().into() })
        .collect();
    ui.set_alt_tab_cells(ModelRc::new(VecModel::from(cells)));
    ui.set_alt_tab_selected(s.selected_on_page().map_or(-1, |i| i as i32));
    ui.set_alt_tab_status(s.status().into());
    ui.set_alt_tab_plate(s.plate().into());
    ui.set_alt_tab_page(s.page() as i32);
    ui.set_alt_tab_pages(s.pages() as i32);
    ui.set_alt_tab_open(true);
}

/// Open the switcher, or — when it is already up — take the next step in the direction asked.
/// That second meaning is what makes a held Alt and repeated Tab work: each press is the
/// compositor's binding running this again.
pub fn open(ui: &App, backwards: bool) -> Result<serde_json::Value, String> {
    // Held while a card waits, as the bar's panels are: this draws over the card's screen and
    // brings the shell forward, and a card behind a window is a decision nobody can make.
    crate::card_watch::hold_windows("open_switcher")?;
    let screen = ui.get_current_screen();
    if !crate::control_overlays::bar_is_drawn(screen) {
        return Err(format!(
            "the switcher opens over the desktop, and `{}` is not the desktop",
            crate::control::screen_name(screen)
        ));
    }
    {
        let mut open = lock();
        if let Some(s) = open.as_mut() {
            s.step(!backwards);
            publish(ui, Some(s));
            return Ok(answer(s, true));
        }
    }
    // The window in front, read BEFORE the shell is brought forward to show the card: afterwards
    // the shell is the window in front, and Escape would have nothing to give back.
    let origin = crate::toplevel_watch::front_title().or_else(crate::windows::in_front);
    let switcher = Switcher::open(gather(), origin, backwards);
    publish(ui, Some(&switcher));
    let mut result = answer(&switcher, false);
    match crate::windows::raise_shell() {
        Ok(()) => result["raised"] = true.into(),
        Err(why) => {
            result["raised"] = false.into();
            result["note"] = format!(
                "the switcher is open, but the shell's own window could not be brought to the front, \
                 so an app window may still be covering it: {why}"
            )
            .into();
        }
    }
    *lock() = Some(switcher);
    Ok(result)
}

fn answer(s: &Switcher, was_open: bool) -> serde_json::Value {
    serde_json::json!({
        "open": true,
        "was_open": was_open,
        "selected": s.selected().map(|c| c.title.clone()),
        "page": s.page() + 1,
        "pages": s.pages(),
        "windows": s.cells().len(),
    })
}

/// Move the selection as a key would. `Err` when the switcher is not open.
pub fn navigate(ui: &App, how: Move, columns: usize) -> Result<serde_json::Value, String> {
    let mut open = lock();
    let Some(s) = open.as_mut() else {
        return Err("the switcher is not open; `open_switcher` opens it".to_string());
    };
    s.go(how, columns);
    publish(ui, Some(s));
    Ok(answer(s, true))
}

/// The pointer moved over a cell of the page.
fn point_at(ui: &App, i: usize) {
    let mut open = lock();
    if let Some(s) = open.as_mut() {
        let before = s.selected_index();
        s.point_at(i);
        if s.selected_index() != before {
            publish(ui, Some(s));
        }
    }
}

/// Put the switcher away without switching, and give back the window that was in front.
pub fn cancel(ui: &App) -> serde_json::Value {
    let taken = lock().take();
    publish(ui, None);
    let Some(s) = taken else {
        return serde_json::json!({ "open": false, "closed": false, "note": "the switcher was not open; nothing was changed." });
    };
    let mut result = serde_json::json!({ "open": false, "closed": true });
    // The shell was raised to show the card. If an app window was in front before, it is given
    // back; if the desktop was, it still is.
    match s.origin() {
        Some(title) if title != SHELL_WINDOW_TITLE => {
            result["restoring"] = title.into();
            bring_forward(title.to_string());
        }
        _ => {}
    }
    result
}

/// Switch to the selected window — or the one on cell `i` of the page, for a click.
pub fn commit(ui: &App, cell: Option<usize>) -> Result<serde_json::Value, String> {
    let title = {
        let mut open = lock();
        let Some(s) = open.as_mut() else {
            return Err("the switcher is not open; `open_switcher` opens it".to_string());
        };
        if let Some(i) = cell {
            s.point_at(i);
        }
        let Some(chosen) = s.selected() else {
            return Err("there is no window to switch to".to_string());
        };
        chosen.title.clone()
    };
    // Bringing an app window over the shell would cover a card waiting in it. The switcher is
    // put away first on a refusal, so it is not itself covering the card.
    if title != SHELL_WINDOW_TITLE {
        if let Err(why) = crate::card_watch::hold_windows("switcher_commit") {
            cancel_without_restoring(ui);
            return Err(why);
        }
    }
    lock().take();
    publish(ui, None);
    bring_forward(title.clone());
    Ok(serde_json::json!({ "open": false, "switching_to": title }))
}

fn cancel_without_restoring(ui: &App) {
    lock().take();
    publish(ui, None);
}

/// The compositor is asked on a thread of its own: `wlrctl` is a process, and a key handler waits
/// for nobody. Whether the window came forward is for `describe shell`'s `in_front` to say.
fn bring_forward(title: String) {
    let spawned = std::thread::Builder::new().name("yos-switch".into()).spawn(move || {
        let ok = if title == SHELL_WINDOW_TITLE { crate::windows::raise_shell().is_ok() } else { crate::windows::present(&title) };
        if !ok {
            tracing::warn!(window = %title, "the switcher could not bring that window forward; it may have closed");
        }
    });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "could not start the thread that switches windows");
    }
}

/// The card's own callbacks: the same functions the actions run.
pub fn wire(ui: &App) {
    let weak = ui.as_weak();
    ui.on_alt_tab_navigate(move |word, columns| {
        let Some(ui) = weak.upgrade() else { return };
        if let Some(how) = Move::parse(word.as_str()) {
            let _ = navigate(&ui, how, columns.max(1) as usize);
        }
    });
    let weak = ui.as_weak();
    ui.on_alt_tab_hover(move |i| {
        if let (Some(ui), true) = (weak.upgrade(), i >= 0) {
            point_at(&ui, i as usize);
        }
    });
    let weak = ui.as_weak();
    ui.on_alt_tab_activate(move |i| {
        let Some(ui) = weak.upgrade() else { return };
        if let Err(why) = commit(&ui, (i >= 0).then_some(i as usize)) {
            tracing::warn!(%why, "the switcher did not switch");
        }
    });
    let weak = ui.as_weak();
    ui.on_alt_tab_cancel(move || {
        if let Some(ui) = weak.upgrade() {
            cancel(&ui);
        }
    });
}

/// The four actions. Opening and moving are `safe`, like the bar's panels: they draw a card and
/// bring the shell forward. Committing moves another window in front and is `standard`, as
/// `focus_window` is.
pub fn actions(surface: ControlSurface, ui: &App) -> ControlSurface {
    let open_weak = ui.as_weak();
    let move_weak = ui.as_weak();
    let commit_weak = ui.as_weak();
    let cancel_weak = ui.as_weak();
    surface
        .action(
            Action::new(
                "open_switcher",
                "Show the Alt+Tab window switcher over whatever screen is up and bring the shell in \
                 front of any app window: the open windows, most recently used first, 8 to a page, \
                 with the previous window selected. When it is already open this steps to the next \
                 window instead, which is what pressing Alt+Tab again does. Nothing is switched \
                 until `switcher_commit`. `describe shell` says what is open and selected, under \
                 `window_switcher`.",
            )
            .risk("safe")
            .arg(Param::text("direction").describe("`back` steps the other way, as Alt+Shift+Tab does. Optional.").optional()),
            move |args| {
                let ui = open_weak.upgrade().ok_or_else(|| "the shell is gone".to_string())?;
                open(&ui, args["direction"].as_str() == Some("back"))
            },
        )
        .action(
            Action::new(
                "switcher_move",
                "Move the open switcher's selection, as its keys do: next, previous, left, right, up, \
                 down, page-next, page-prev. Refused when the switcher is not open.",
            )
            .risk("safe")
            .arg(Param::text("direction").describe(&format!("One of: {}", Move::WORDS))),
            move |args| {
                let ui = move_weak.upgrade().ok_or_else(|| "the shell is gone".to_string())?;
                let word = args["direction"].as_str().unwrap_or_default();
                let how = Move::parse(word)
                    .ok_or_else(|| format!("`{word}` is not a direction; use one of: {}", Move::WORDS))?;
                // A wide card: four columns, the same a desktop-sized screen draws.
                navigate(&ui, how, 4)
            },
        )
        .action(
            Action::new(
                "switcher_commit",
                "Switch to the window the open switcher has selected and put the switcher away, as \
                 pressing Enter does. The window is asked for, not guaranteed: `describe shell` \
                 says which window is in front afterwards.",
            )
            .defers(),
            move |_args| {
                let ui = commit_weak.upgrade().ok_or_else(|| "the shell is gone".to_string())?;
                commit(&ui, None)
            },
        )
        .action(
            Action::new(
                "switcher_cancel",
                "Put the open switcher away without switching, as Escape does, and give back the \
                 window that was in front when it opened.",
            )
            .risk("safe"),
            move |_args| {
                let ui = cancel_weak.upgrade().ok_or_else(|| "the shell is gone".to_string())?;
                Ok(cancel(&ui))
            },
        )
}

#[cfg(test)]
mod tests {
    /// The part of this file above its tests, which is what the shell runs.
    fn code() -> String {
        include_str!("control_switcher.rs").split("#[cfg(test)]").next().unwrap().to_string()
    }

    fn function(name: &str) -> String {
        let src = code();
        let from = src.find(&format!("fn {name}(")).unwrap_or_else(|| panic!("no fn {name}"));
        let body = &src[from..];
        body[..body.find("\n}\n").unwrap()].to_string()
    }

    fn declaration(name: &str) -> String {
        let src = code();
        // The name on its own line, as `Action::new(` is written, not the same string in `describe`.
        let at = src.find(&format!("\n                \"{name}\",\n")).unwrap_or_else(|| panic!("`{name}` is no longer published"));
        let rest = &src[at..];
        rest[..rest.find("Action::new(").unwrap_or(rest.len())].to_string()
    }

    /// Opening and committing bring a window over a card that may be waiting, so each asks
    /// `hold_windows` first — by its own name, which the source-scan convention looks for — and
    /// before anything is drawn or moved. Putting the switcher away never waits.
    #[test]
    fn opening_and_committing_wait_for_a_card_and_cancelling_never_does() {
        let open = function("open");
        let held = open.find("hold_windows(\"open_switcher\")").expect("open asks hold_windows");
        assert!(held < open.find("publish(").unwrap() && held < open.find("raise_shell").unwrap());
        let commit = function("commit");
        let held = commit.find("hold_windows(\"switcher_commit\")").expect("commit asks hold_windows");
        assert!(held < commit.find("bring_forward(").unwrap(), "asked before any window is moved");
        assert!(!function("cancel").contains("hold_windows"), "putting the switcher away is never held");
    }

    /// A refused commit must not leave the switcher drawn over the card that caused the refusal.
    #[test]
    fn a_held_commit_puts_the_switcher_away() {
        let commit = function("commit");
        let refusal = &commit[commit.find("hold_windows").unwrap()..];
        assert!(refusal[..refusal.find("bring_forward").unwrap()].contains("cancel_without_restoring"));
    }

    /// Honest grades: showing, moving and cancelling are `safe`; committing moves another window
    /// in front, so it is not, and it says it is deferred because focus is the compositor's.
    #[test]
    fn the_four_actions_are_published_with_honest_grades() {
        for name in ["open_switcher", "switcher_move", "switcher_cancel"] {
            assert!(declaration(name).contains(".risk(\"safe\")"), "`{name}` must be graded safe");
        }
        let commit = declaration("switcher_commit");
        assert!(!commit.contains(".risk(\"safe\")"), "committing moves a window in front of the shell");
        assert!(commit.contains(".defers()"), "the window is asked for, not guaranteed");
    }

    /// Escape gives back the window that was in front, and it was read before the shell was
    /// raised — afterwards the shell is what is in front and there would be nothing to give back.
    #[test]
    fn the_window_to_give_back_is_read_before_the_shell_is_raised() {
        let open = function("open");
        assert!(open.find("front_title()").unwrap() < open.find("raise_shell()").unwrap());
        assert!(function("cancel").contains("bring_forward"));
    }

    /// The switcher runs the compositor on a thread, never the UI thread.
    #[test]
    fn switching_windows_runs_off_the_ui_thread() {
        let body = function("bring_forward");
        assert!(body.contains("thread::Builder") && body.contains("present(&title)"));
        assert!(!function("commit").contains("present("), "commit hands the title to bring_forward");
    }

    #[test]
    fn describe_shell_publishes_the_switcher_and_the_surface_has_its_actions() {
        let control = include_str!("control.rs");
        let control: String = control.split("#[cfg(test)]").next().unwrap().split_whitespace().collect();
        assert!(control.contains(".with(\"window_switcher\",crate::control_switcher::for_describe(screen))"));
        assert!(control.contains("crate::control_switcher::actions(surface,ui)"));
    }

    /// The overlay is the only switcher: the compositor's own is not bound to Alt+Tab.
    #[test]
    fn labwc_alt_tab_opens_the_shells_switcher() {
        let rc = include_str!("../../../config/labwc/rc.xml");
        for (key, command) in [("A-Tab", "open_switcher"), ("A-S-Tab", "open_switcher direction=back")] {
            let at = rc.find(&format!("<keybind key=\"{key}\">")).unwrap_or_else(|| panic!("no {key} binding"));
            let binding = &rc[at..at + rc[at..].find("</keybind>").unwrap()];
            assert!(binding.contains(&format!("yos act shell {command}")), "{key} must run `{command}`: {binding}");
            assert!(!binding.contains("NextWindow") && !binding.contains("PreviousWindow"), "{key} still cycles labwc's own OSD");
        }
    }
}
