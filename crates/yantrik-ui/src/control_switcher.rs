//! The window overview (Super+Tab) on the shell's control surface, and the glue that draws it.
//!
//! The overview, the keys inside it, a click on a cell and `yos act shell open_switcher` all end
//! in the functions here, so what a pointer can do a mind can ask for.
//!
//! # This is not Alt+Tab
//!
//! Alt+Tab is labwc's own window cycling (`config/labwc/rc.xml`, `NextWindow`/`PreviousWindow`):
//! hold Alt, tap Tab, release Alt to switch, in the compositor, so it works when the shell is
//! busy, hung, held by an approval card or gone. An earlier version bound Alt+Tab to this card and
//! lost all of that: labwc never tells a client that Alt came up, so release did nothing, and a
//! shell that could not answer left no Alt+Tab at all. The look people want from Alt+Tab is the
//! themerc's `osd.*` block. This card is a separate overview on Super+Tab that never stands in for
//! the native path; Enter or a click switches, Escape puts it away.
//!
//! # Off the UI thread
//!
//! Nothing here waits on the compositor on the UI thread. Opening reads the window stream's copy
//! (`toplevel_watch::windows`, no process), or `wlrctl`'s list when the stream is off, then raises
//! the shell; switching activates a window and learns whether it was still there. All of that is
//! a worker's job, finished on the socket's side with `answer_later` and published to the card
//! with `upgrade_in_event_loop`. Every compositor request is bounded (`windows::ask_compositor`,
//! 1.2 s each), so a stuck compositor is an error in the answer, not a frozen desktop.
//!
//! The windows are not captured: the compositor's window stream carries titles and focus, not
//! pixels, and no capture path for a single toplevel exists in the shell. Every cell is the app's
//! tile, its name and the window's title (alt_tab.slint), named by `window_name` as the dock is.
//!
//! Left out on purpose, and said in `describe`: Mind View scope (a desk's own windows and a way
//! back), the workspace in the footer, and previews.

use std::sync::Mutex;

use slint::{ComponentHandle, ModelRc, VecModel, Weak};
use yantrik_app_runtime::control::{Action, App as ControlSurface, Param};

use crate::alt_tab::{Cell, Move, Origin, Switcher};
use crate::windows::SHELL_WINDOW_TITLE;
use crate::{App, SwitcherCell};

/// The one open switcher, if any. A mutex rather than a field of the UI because the control
/// handlers, the workers and the Slint callbacks all reach it, and none holds it across a call out
/// to the compositor.
static OPEN: Mutex<Option<Switcher>> = Mutex::new(None);

fn lock() -> std::sync::MutexGuard<'static, Option<Switcher>> {
    OPEN.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// What `describe shell` says under `window_switcher`: whether it is on the screen, and what it
/// has selected. Drawn, not merely asked for — false where the bar is not.
pub fn for_describe(screen: i32) -> serde_json::Value {
    let open = lock();
    match open.as_ref().filter(|_| crate::control_overlays::bar_is_drawn(screen)) {
        None => serde_json::json!({ "open": false, "close_with": "", "bound_to": "Super+Tab (Alt+Tab is the compositor's own)" }),
        Some(s) => serde_json::json!({
            "open": true,
            "close_with": "switcher_cancel",
            "windows": s.cells().iter().map(described).collect::<Vec<_>>(),
            "selected": s.selected().map(described),
            "page": s.page() + 1,
            "pages": s.pages(),
            "status": s.status(),
            "previews": "none: each cell is the app's tile, its name and the window's title",
            "order_source": if s.cells().iter().all(Cell::is_live) { "compositor focus stream" } else { "the compositor's listing order (the focus stream is off)" },
            "left_out": ["Mind View scope", "workspace in the footer", "previews"],
        }),
    }
}

/// The id the desktop's own cell is drawn under. The shell is not an app, so its tile is the
/// quiet one with no colour.
const DESKTOP_ID: &str = "desktop";

/// The open windows as cells, most recently used first. On a worker: when the window stream is
/// off this runs `wlrctl`.
///
/// The stream hands over the app id each window DECLARED, and our Slint windows declare none. Fed
/// to the namer as it was, that left Calendar and the desktop itself with no name and a bare
/// window glyph (the test VM, 4 October). So the id is resolved first, by the rule the window list uses
/// (`windows::stream_app_id`), and then named by the one function every surface names windows with.
fn gather() -> Vec<Cell> {
    let installed = crate::apps::Catalogue::shared().get();
    let app_of = |declared: &str, title: &str| {
        if title == SHELL_WINDOW_TITLE { DESKTOP_ID.to_string() } else { crate::windows::stream_app_id(declared, title) }
    };
    let name = |app_id: &str, title: &str| crate::window_name::display_name(app_id, title, &installed);
    let mut open: Vec<Cell> = match crate::toplevel_watch::windows() {
        Some(list) => list
            .into_iter()
            .map(|w| {
                let app_id = app_of(&w.app_id, &w.title);
                Cell { id: w.id, app_name: name(&app_id, &w.title), title: w.title, app_id }
            })
            .collect(),
        None => crate::windows::shell_windows()
            .into_iter()
            .enumerate()
            .map(|(n, w)| {
                let app_id = if w.title == SHELL_WINDOW_TITLE { DESKTOP_ID.to_string() } else { w.app_id };
                let app_name = name(&app_id, &w.title);
                Cell::listed(n, w.title, app_id, app_name)
            })
            .collect(),
    };
    if !open.iter().any(|c| c.title == SHELL_WINDOW_TITLE) {
        open.push(Cell::listed(open.len(), SHELL_WINDOW_TITLE.into(), DESKTOP_ID.into(), name(DESKTOP_ID, SHELL_WINDOW_TITLE)));
    }
    crate::alt_tab::order(open, &crate::toplevel_watch::recency_ids())
}

/// One cell as `describe shell` and the actions' answers report it: the same name and title the
/// card draws, and the app the window belongs to.
fn described(c: &Cell) -> serde_json::Value {
    serde_json::json!({ "name": c.app_name, "title": c.title, "app": c.app_id })
}

/// The window in front, read before the shell is brought forward. On a worker.
fn front_window() -> Option<Origin> {
    match crate::toplevel_watch::front() {
        Some((id, title)) => Some(Origin { id: Some(id), title }),
        None => crate::windows::in_front().map(|title| Origin { id: None, title }),
    }
}

/// Hand the page being shown to the card. On the UI thread.
fn publish(ui: &App, s: Option<&Switcher>) {
    let Some(s) = s else {
        ui.set_alt_tab_open(false);
        ui.set_alt_tab_cells(ModelRc::default());
        return;
    };
    // The dock's tile for each window: the same glyph and colour, or the same theme icon.
    let tr = ui.global::<crate::Tr>();
    let installed = crate::apps::Catalogue::shared().get();
    let cells: Vec<SwitcherCell> = s
        .page_cells()
        .iter()
        .map(|c| {
            let icon = crate::wire::dock_bar::theme_icon(&tr, &c.app_id, &installed);
            SwitcherCell {
                title: c.title.clone().into(),
                tile_id: crate::window_name::tile_id(&c.app_id).into(),
                app_name: c.app_name.clone().into(),
                has_icon: icon.is_some(),
                icon: icon.unwrap_or_default(),
            }
        })
        .collect();
    ui.set_alt_tab_cells(ModelRc::new(VecModel::from(cells)));
    ui.set_alt_tab_selected(s.selected_on_page().map_or(-1, |i| i as i32));
    ui.set_alt_tab_status(s.status().into());
    ui.set_alt_tab_page(s.page() as i32);
    ui.set_alt_tab_pages(s.pages() as i32);
    ui.set_alt_tab_open(true);
}

/// Publish whatever is open now, from any thread.
fn republish(weak: &Weak<App>) {
    let _ = weak.upgrade_in_event_loop(|ui| publish(&ui, lock().as_ref()));
}

/// Open the overview, or — when it is already up — take the next step in the direction asked.
pub fn open(ui: &App, backwards: bool) -> Result<serde_json::Value, String> {
    // Held while a card waits, as the bar's panels are: this draws over the card's screen and
    // brings the shell forward, and a card behind a window is a decision nobody can make.
    crate::card_watch::hold_windows("open_switcher")?;
    let screen = ui.get_current_screen();
    if !crate::control_overlays::bar_is_drawn(screen) {
        return Err(format!(
            "the switcher opens over the desktop, and `{}` has no status bar yet",
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
    let weak = ui.as_weak();
    let work = move || open_on_worker(&weak, backwards);
    yantrik_app_runtime::control::answer_later(work)
        .map(|()| serde_json::json!({ "answering": "off the UI thread" }))
        .or_else(|work| work())
}

/// The slow half of opening: read the windows, draw the card, then bring the shell forward.
fn open_on_worker(weak: &Weak<App>, backwards: bool) -> Result<serde_json::Value, String> {
    // The window in front, read BEFORE the shell is brought forward to show the card: afterwards
    // the shell is the window in front, and Escape would have nothing to give back.
    let origin = front_window();
    let switcher = Switcher::open(gather(), origin, backwards);
    let mut result = {
        let mut open = lock();
        // Two presses can both find it closed; the second one is a step, not a second card.
        if let Some(s) = open.as_mut() {
            s.step(!backwards);
            let result = answer(s, true);
            drop(open);
            republish(weak);
            return Ok(result);
        }
        let result = answer(&switcher, false);
        *open = Some(switcher);
        result
    };
    republish(weak);
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
    Ok(result)
}

fn answer(s: &Switcher, was_open: bool) -> serde_json::Value {
    serde_json::json!({
        "open": true,
        "was_open": was_open,
        "selected": s.selected().map(described),
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
    // back — unless a card is waiting, because that window would cover it: then the card is put
    // away and the window stays where it was. If the desktop was in front, it still is.
    if let Some(origin) = s.origin().filter(|o| o.title != SHELL_WINDOW_TITLE) {
        match crate::card_watch::hold_windows("switcher_cancel") {
            Ok(()) => {
                result["restoring"] = origin.title.clone().into();
                let origin = origin.clone();
                spawn("yos-switch-back", move || {
                    if !activate_origin(&origin) {
                        tracing::warn!(window = %origin.title, "the switcher could not give that window back; it may have closed");
                    }
                });
            }
            Err(why) => result["note"] = format!("the previous window was not brought back: {why}").into(),
        }
    }
    result
}

fn activate_origin(origin: &Origin) -> bool {
    origin.id.is_some_and(crate::toplevel_watch::activate) || crate::windows::present(&origin.title)
}

/// Ask the compositor for `cell`: by its id when the stream named it, else by title.
fn activate_cell(cell: &Cell) -> bool {
    if cell.title_is_shell() {
        crate::windows::raise_shell().is_ok()
    } else if cell.is_live() {
        crate::toplevel_watch::activate(cell.id)
    } else {
        crate::windows::present(&cell.title)
    }
}

/// Switch to the selected window — or the one on cell `i` of the page, for a click. The answer
/// is finished off the UI thread, because it says whether the window was still there.
pub fn commit(ui: &App, cell: Option<usize>) -> Result<serde_json::Value, String> {
    let chosen = {
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
        chosen.clone()
    };
    // Bringing an app window over the shell would cover a card waiting in it. The switcher is
    // put away first on a refusal, so it is not itself covering the card.
    if !chosen.title_is_shell() {
        if let Err(why) = crate::card_watch::hold_windows("switcher_commit") {
            lock().take();
            publish(ui, None);
            return Err(why);
        }
    }
    let weak = ui.as_weak();
    let work = move || switch_to(&weak, &chosen);
    yantrik_app_runtime::control::answer_later(work)
        .map(|()| serde_json::json!({ "answering": "off the UI thread" }))
        .or_else(|work| work())
}

/// The worker's half of committing. The card goes away only once the compositor has taken the
/// request; when the window is gone it is taken off the card, which stays up for another choice.
fn switch_to(weak: &Weak<App>, chosen: &Cell) -> Result<serde_json::Value, String> {
    if activate_cell(chosen) {
        lock().take();
        republish(weak);
        return Ok(serde_json::json!({ "open": false, "switching_to": chosen.title }));
    }
    let left = {
        let mut open = lock();
        let left = open.as_mut().map(|s| {
            s.remove(chosen.id);
            s.cells().len()
        });
        if left == Some(0) {
            *open = None;
        }
        left.unwrap_or(0)
    };
    republish(weak);
    tracing::warn!(window = %chosen.title, "the switcher could not bring that window forward; it may have closed");
    Err(format!(
        "`{}` is no longer there and was taken off the card ({left} window{} left); choose another or put the switcher away",
        chosen.title,
        if left == 1 { "" } else { "s" }
    ))
}

fn spawn(name: &str, work: impl FnOnce() + Send + 'static) {
    if let Err(e) = std::thread::Builder::new().name(name.into()).spawn(work) {
        tracing::warn!(error = %e, "could not start a thread for the switcher");
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
        // Not through `commit`: with no control call in flight there is nothing to answer later,
        // and its fallback would run the compositor on this thread.
        let chosen = {
            let mut open = lock();
            let Some(s) = open.as_mut() else { return };
            if i >= 0 {
                s.point_at(i as usize);
            }
            match s.selected() {
                Some(c) => c.clone(),
                None => return,
            }
        };
        if !chosen.title_is_shell() {
            if let Err(why) = crate::card_watch::hold_windows("switcher_commit") {
                lock().take();
                publish(&ui, None);
                tracing::warn!(%why, "the switcher did not switch");
                return;
            }
        }
        let weak = ui.as_weak();
        spawn("yos-switch", move || {
            if let Err(why) = switch_to(&weak, &chosen) {
                tracing::warn!(%why, "the switcher did not switch");
            }
        });
    });
    let weak = ui.as_weak();
    ui.on_alt_tab_cancel(move || {
        if let Some(ui) = weak.upgrade() {
            cancel(&ui);
        }
    });
}

/// The four actions. Opening, moving and cancelling are `safe`, like the bar's panels: they draw
/// a card and bring the shell forward (cancel gives a window back only when no card waits).
/// Committing moves another window in front and is `standard`, as `focus_window` is.
pub fn actions(surface: ControlSurface, ui: &App) -> ControlSurface {
    let open_weak = ui.as_weak();
    let move_weak = ui.as_weak();
    let commit_weak = ui.as_weak();
    let cancel_weak = ui.as_weak();
    surface
        .action(
            Action::new(
                "open_switcher",
                "Show the window overview (Super+Tab) over whatever screen is up and bring the shell \
                 in front of any app window: the open windows, most recently used first, 8 to a page, \
                 with the previous window selected. When it is already open this steps to the next \
                 window instead. Alt+Tab is not this: it is the compositor's own cycling and keeps \
                 working without the shell. Nothing is switched until `switcher_commit`. `describe \
                 shell` says what is open and selected, under `window_switcher`. Windows that share \
                 a title are listed as \"Title\", \"Title (2)\".",
            )
            .risk("safe")
            .arg(Param::text("direction").describe("`back` steps the other way, as Super+Shift+Tab does. Optional.").optional()),
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
            .arg(Param::text("direction").describe(&format!("One of: {}", Move::WORDS)))
            .arg(
                Param::text("columns")
                    .describe("How many columns the card is drawn with (1 to 4; default 4, the desktop width), for up and down. Optional.")
                    .optional(),
            ),
            move |args| {
                let ui = move_weak.upgrade().ok_or_else(|| "the shell is gone".to_string())?;
                let word = args["direction"].as_str().unwrap_or_default();
                let how = Move::parse(word)
                    .ok_or_else(|| format!("`{word}` is not a direction; use one of: {}", Move::WORDS))?;
                navigate(&ui, how, columns_arg(&args["columns"]))
            },
        )
        .action(
            Action::new(
                "switcher_commit",
                "Switch to the window the open switcher has selected and put the switcher away, as \
                 pressing Enter does. The window is asked for, not guaranteed: `describe shell` \
                 says which window is in front afterwards. A window that has closed is taken off the \
                 card and an error says so; the card stays up.",
            )
            .risk("standard")
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
                 window that was in front when it opened — unless a card is waiting, when the window \
                 stays where it was so it cannot cover the card.",
            )
            .risk("safe"),
            move |_args| {
                let ui = cancel_weak.upgrade().ok_or_else(|| "the shell is gone".to_string())?;
                Ok(cancel(&ui))
            },
        )
}

/// `columns` as given, 1 to 4, or 4. A number or a string of one.
fn columns_arg(v: &serde_json::Value) -> usize {
    let n = v.as_u64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())).unwrap_or(4);
    n.clamp(1, 4) as usize
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
        // Up to the next action, so the whole declaration (its grade) is in it.
        rest[1..].find("Action::new(").map_or(rest, |n| &rest[..=n]).to_string()
    }

    /// Opening and committing bring a window over a card that may be waiting, so each asks
    /// `hold_windows` first — by its own name, which the source-scan convention looks for — and
    /// before anything is drawn or moved. Cancelling asks too, but only when it is about to give
    /// a window back (review finding 6).
    #[test]
    fn opening_committing_and_giving_a_window_back_wait_for_a_card() {
        let open = function("open");
        let held = open.find("hold_windows(\"open_switcher\")").expect("open asks hold_windows");
        assert!(held < open.find("publish(").unwrap() && held < open.find("answer_later").unwrap());
        let commit = function("commit");
        let held = commit.find("hold_windows(\"switcher_commit\")").expect("commit asks hold_windows");
        assert!(held < commit.find("answer_later").unwrap(), "asked before any window is moved");
        let cancel = function("cancel");
        let held = cancel.find("hold_windows(\"switcher_cancel\")").expect("cancel asks hold_windows before giving a window back");
        assert!(held < cancel.find("spawn(").unwrap(), "asked before the window is brought forward");
    }

    /// The bug: a mind opens the switcher, a card then appears, and `switcher_cancel` raised the
    /// previous app window over the waiting card. A refused hold now puts the switcher away and
    /// leaves the window where it was, saying so.
    #[test]
    fn a_held_cancel_puts_the_switcher_away_without_raising_a_window() {
        let cancel = function("cancel");
        let at = cancel.find("hold_windows").unwrap();
        let refusal = &cancel[at..];
        let err_arm = &refusal[refusal.find("Err(why)").unwrap()..];
        assert!(!err_arm[..err_arm.find("\n    }").unwrap_or(err_arm.len())].contains("spawn("), "the refusal arm raises nothing");
        assert!(err_arm.contains("was not brought back"));
        // The desktop being the origin needs no hold: nothing is raised over the card.
        assert!(cancel.contains("o.title != SHELL_WINDOW_TITLE"));
    }

    /// A refused commit must not leave the switcher drawn over the card that caused the refusal.
    #[test]
    fn a_held_commit_puts_the_switcher_away() {
        for body in [function("commit")] {
            let refusal = &body[body.find("hold_windows").unwrap()..];
            let refusal = &refusal[..refusal.find("answer_later").unwrap()];
            assert!(refusal.contains("lock().take()") && refusal.contains("publish(ui, None)"));
        }
    }

    /// Honest grades, each stated: showing, moving and cancelling are `safe`; committing moves
    /// another window in front, is `standard` by its own word and says it is deferred.
    #[test]
    fn the_four_actions_are_published_with_honest_grades() {
        for name in ["open_switcher", "switcher_move", "switcher_cancel"] {
            assert!(declaration(name).contains(".risk(\"safe\")"), "`{name}` must be graded safe");
        }
        let commit = declaration("switcher_commit");
        assert!(commit.contains(".risk(\"standard\")"), "the grade is written down so it cannot drift");
        assert!(commit.contains(".defers()"), "the window is asked for, not guaranteed");
    }

    /// Escape gives back the window that was in front, and it was read before the shell was
    /// raised — afterwards the shell is what is in front and there would be nothing to give back.
    #[test]
    fn the_window_to_give_back_is_read_before_the_shell_is_raised() {
        let open = function("open_on_worker");
        assert!(open.find("front_window()").unwrap() < open.find("raise_shell()").unwrap());
        assert!(function("cancel").contains("activate_origin"));
    }

    /// Blocking calls — the window list, the front-window lookup, the raise, the activation —
    /// run on a worker, never in the control handler or a Slint callback (review finding 3). The
    /// handlers hand the work to `answer_later`; the UI callback spawns a thread.
    #[test]
    fn nothing_blocking_runs_in_a_handler_or_a_callback() {
        let blocking = ["gather(", "front_window(", "raise_shell(", "wlrctl", "present(", "activate_cell(", "shell_windows("];
        for name in ["open", "commit", "navigate", "point_at", "cancel", "publish"] {
            let body = function(name);
            for call in blocking {
                assert!(!body.contains(call), "`{name}` is on the UI thread and must not call {call}");
            }
        }
        let open = function("open");
        assert!(open.contains("answer_later(") && open.contains("open_on_worker"));
        assert!(function("commit").contains("answer_later(") && function("commit").contains("switch_to"));
        let wire = function("wire");
        assert!(wire.contains("spawn(\"yos-switch\"") && !wire.contains("commit(&ui"), "a click switches on a thread of its own");
        assert!(!wire.contains("activate_cell("), "and does not touch the compositor itself");
        // `cancel` hands the window back on a thread.
        assert!(function("cancel").contains("spawn("));
    }

    /// The cells are keyed by the compositor's toplevel id, not by title: the stream's copy is
    /// what is read, and activation goes by id (review finding 4).
    #[test]
    fn windows_are_keyed_by_the_toplevel_id_not_the_title() {
        let g = function("gather");
        assert!(g.contains("toplevel_watch::windows()") && g.contains("id: w.id"));
        assert!(g.contains("recency_ids()"), "ordered by id, so same-titled windows keep their own places");
        let a = function("activate_cell");
        assert!(a.find("toplevel_watch::activate(cell.id)").unwrap() < a.find("present(&cell.title)").unwrap());
    }

    /// A window that closed under the card: the activation fails, the window is taken off the
    /// card by id, and the card stays up (finding 5).
    #[test]
    fn a_closed_window_is_taken_off_the_card_which_stays_up() {
        let body = function("switch_to");
        assert!(body.contains("s.remove(chosen.id)") && body.contains("republish("));
        assert!(body.contains("is no longer there"), "the answer says so, not just a log line");
        assert!(body.find("lock().take()").unwrap() < body.find("s.remove(").unwrap(), "put away only on success");
    }

    #[test]
    fn switcher_move_reads_the_columns_the_card_is_drawn_with() {
        assert_eq!(super::columns_arg(&serde_json::json!(2)), 2);
        assert_eq!(super::columns_arg(&serde_json::json!("3")), 3);
        assert_eq!(super::columns_arg(&serde_json::json!(null)), 4);
        assert_eq!(super::columns_arg(&serde_json::json!(40)), 4);
        assert_eq!(super::columns_arg(&serde_json::json!(0)), 1);
    }

    /// `describe` says where the order comes from and what is left out, and the closed answer
    /// says what Alt+Tab is.
    #[test]
    fn describe_is_honest_about_order_and_what_is_left_out() {
        let d = super::for_describe(1);
        assert_eq!(d["open"], false);
        assert!(d["bound_to"].as_str().unwrap().contains("Alt+Tab is the compositor's own"));
        let src = code();
        assert!(src.contains("order_source") && src.contains("focus stream is off") && src.contains("left_out"));
    }

    /// The design sign-off (4 October): a cell is the app's tile, its name and the window's title,
    /// never preview text; the card is at most 640 wide, and no plate repeats the title under it.
    #[test]
    fn a_cell_is_the_apps_tile_name_and_title_with_no_preview_text() {
        let card = include_str!("../../yantrik-ui-slint/ui/components/alt_tab.slint");
        let code: String = card.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
        assert!(!code.contains("Preview unavailable") && !code.contains("plate"), "no preview text and no plate");
        assert!(code.contains("AppTile {") && code.contains("size: Theme.sw-tile;"), "the kit's tile, not a copy of it");
        assert!(code.contains("root.selected ? Theme.accent"), "the selected cell is ringed in the accent");
        let theme = include_str!("../../yantrik-design-tokens/slint/theme.slint");
        let token = |name: &str| -> f32 {
            let line = theme.lines().find(|l| l.contains(&format!("<length> {name}:"))).unwrap_or_else(|| panic!("no {name}"));
            line.split(':').nth(1).unwrap().trim().trim_end_matches(';').trim_end_matches("px").parse().unwrap()
        };
        assert!(token("sw-card-max-w") <= 640.0, "the card is at most 640 wide");
        assert_eq!(token("sw-tile"), 48.0);
    }

    /// `describe shell` and the actions' answers give each cell the name and title the card draws.
    #[test]
    fn describe_reports_the_name_and_title_the_card_draws() {
        let cell = super::Cell { id: 3, title: "Budget — LibreOffice Calc".into(), app_id: "libreoffice-calc".into(), app_name: "LibreOffice Calc".into() };
        let v = super::described(&cell);
        assert_eq!((v["name"].as_str(), v["title"].as_str(), v["app"].as_str()), (Some("LibreOffice Calc"), Some("Budget — LibreOffice Calc"), Some("libreoffice-calc")));
        let s = super::Switcher::open(vec![cell.clone(), cell], None, false);
        assert_eq!(super::answer(&s, false)["selected"]["name"], "LibreOffice Calc");
        let src = code();
        let publish = function("publish");
        assert!(publish.contains("app_name: c.app_name.clone()") && publish.contains("title: c.title.clone()"), "the card draws the cell's own name and title");
        assert!(src.contains("\"windows\": s.cells().iter().map(described)"), "describe lists every cell that way");
    }

    #[test]
    fn describe_shell_publishes_the_switcher_and_the_surface_has_its_actions() {
        let control = include_str!("control.rs");
        let control: String = control.split("#[cfg(test)]").next().unwrap().split_whitespace().collect();
        assert!(control.contains(".with(\"window_switcher\",crate::control_switcher::for_describe(screen))"));
        assert!(control.contains("crate::control_switcher::actions(surface,ui)"));
    }

    /// The bug: Alt+Tab was handed to the shell, so release did nothing and a hung shell or a
    /// waiting approval card left no Alt+Tab at all. Alt+Tab is labwc's own cycling again, the
    /// shell's card is a separate Super+Tab, and the OSD is themed (finding 1, 2).
    #[test]
    fn alt_tab_is_labwcs_own_and_the_card_is_a_separate_super_tab() {
        let rc = include_str!("../../../config/labwc/rc.xml");
        let binding = |key: &str| {
            let at = rc.find(&format!("<keybind key=\"{key}\">")).unwrap_or_else(|| panic!("no {key} binding"));
            rc[at..at + rc[at..].find("</keybind>").unwrap()].to_string()
        };
        for (key, action) in [("A-Tab", "NextWindow"), ("A-S-Tab", "PreviousWindow")] {
            let b = binding(key);
            assert!(b.contains(&format!("<action name=\"{action}\"")), "{key} must be labwc's {action}: {b}");
            assert!(!b.contains("yos"), "{key} must never go through the shell: {b}");
        }
        for (key, command) in [("W-Tab", "open_switcher"), ("W-S-Tab", "open_switcher direction=back")] {
            assert!(binding(key).contains(&format!("yos act shell {command}")), "{key} opens the overview");
        }
        // No binding anywhere puts Alt+Tab through `yos`.
        assert!(!rc.lines().any(|l| l.contains("open_switcher") && l.contains("A-Tab")));
        // The OSD is themed to the shell's charcoal card, with no teal.
        assert!(rc.contains("<windowSwitcher"));
        let theme = include_str!("../../../config/labwc/themerc");
        assert!(theme.contains("osd.bg.color: #151A1E") && theme.contains("osd.window-switcher.item.active.border.color: #ffffff"));
        let osd = &theme[theme.find("On-screen display").unwrap()..];
        assert!(!osd.to_lowercase().contains("4ecdc4"), "teal is for minds");
    }
}
