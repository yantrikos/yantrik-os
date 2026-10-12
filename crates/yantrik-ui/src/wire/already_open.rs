//! A launch by name for an app whose window is already on the person's desktop brings that window
//! forward and starts nothing.
//!
//! "One window per app" was never the shell's to keep. Each of our own apps holds a pid file
//! (`yantrik_app_runtime::instance`), so a second copy exits at once and the reaper raises the
//! window that is open (`dock::exit_verdict`). Anything that does not hold one got a process per
//! launch. On VM 520 (4 October) `open_app name=blender` started a second Blender beside the one
//! an earlier shell had opened on 29 September, then a third: Blender has no such guard, and the
//! launcher never looked at the windows before it spawned. `describe shell` listed the first
//! Blender under `windows` the whole time, with app id `blender`, because the window list asks the
//! compositor, which outlives a shell restart. The in-process registry is empty after a restart
//! and never knew a window a person opened from a terminal, so it is the list, not the registry,
//! that is asked here.
//!
//! The decision is a pure function of the window list ([`on_desktop`]) and the work around it
//! takes the list, the raise and the start as arguments ([`settle`]), so the cases are tested
//! without a compositor. [`focus_or_start`] is the real one.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::windows::WindowEntry;

/// What a launch on the person's desktop does, given the windows that are open there.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum OnDesktop {
    /// The app's window is open: bring it forward. `title` is what the compositor calls it, and
    /// `app_id` the `wayland_app_id` it declared, so the raise can fall back to the app_id when
    /// the title has moved on since the list was read.
    Focus { title: String, app_id: String },
    /// The app was launched and its window has not appeared to the compositor yet: the click is
    /// remembered and the window is brought forward when it shows up, rather than starting a
    /// second copy.
    Launching,
    /// No window of the app is listed: start it.
    Start,
}

/// Whether `app_id` already has a window in `open`.
///
/// Only a window the compositor reported counts as open: a registry-only entry stands in for a
/// window the compositor has not listed yet (or never will under that name), and asking the
/// compositor for it by the shell's display name matches nothing. Such an entry is [`OnDesktop::Launching`].
///
/// By the shell's id, or by the program the launch would run (`windows::same_program`, the rule
/// the window list's merge uses), because the two disagree for a window this shell did not start:
/// a browser opened before a restart is listed as `chromium`, not `browser`, and the registry
/// that would have paired them is gone.
pub(crate) fn on_desktop(app_id: &str, program: Option<&str>, open: &[WindowEntry]) -> OnDesktop {
    if let Some(w) = open.iter().find(|w| {
        w.seen_by_compositor
            && (w.app_id == app_id
                || program.is_some_and(|p| crate::windows::same_program(p, &w.wayland_app_id)))
    }) {
        return OnDesktop::Focus { title: w.title.clone(), app_id: w.wayland_app_id.clone() };
    }
    if open.iter().any(|w| !w.seen_by_compositor && w.app_id == app_id) {
        return OnDesktop::Launching;
    }
    OnDesktop::Start
}

/// Programs whose second launch hands over to the window that is already open, so a listed window
/// that cannot be raised is a reason to start again rather than to refuse.
///
/// Chromium is a singleton: `chromium --new-window` opens a new window in the running process at
/// once, so starting it again is the same thing as raising it. Our own apps hold a pid file
/// (`yantrik_app_runtime::instance`), so a second copy exits at once and the reaper raises the
/// open window (`dock::exit_verdict`). Blender is deliberately NOT here: it has no such guard, and
/// a second Blender over the first is the bug `settle` exists to stop.
const HANDS_OVER: &[&str] = &[
    "chromium",
    "chromium-browser",
    "google-chrome",
    "google-chrome-stable",
    "chrome",
    "yantrik-arcade",
    "yantrik-calendar",
    "yantrik-container-manager",
    "yantrik-document-editor",
    "yantrik-download-manager",
    "yantrik-email",
    "yantrik-image-viewer",
    "yantrik-music-player",
    "yantrik-network-manager",
    "yantrik-notes",
    "yantrik-presentation",
    "yantrik-snippet-manager",
    "yantrik-spreadsheet",
    "yantrik-studio",
    "yantrik-system-monitor",
    "yantrik-terminal",
    "yantrik-text-editor",
    "yantrik-weather",
];

/// Whether the binary `program` would run hands a second launch over to its open window.
fn hands_over(program: &str) -> bool {
    let name = std::path::Path::new(program)
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    HANDS_OVER.contains(&name.as_str())
}

/// How long a pending raise is kept. A window that has not appeared within this is not coming, and
/// the next click should read a fresh list rather than wait on a launch that died.
const PENDING_RAISE_TTL: Duration = Duration::from_secs(30);

/// The clicks that landed while an app was still opening, one per app id, with when it was
/// recorded so an entry can expire.
fn pending() -> &'static Mutex<HashMap<String, Instant>> {
    static PENDING: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Remember that a click asked for `app_id` while it was still opening, at `now`.
fn record_pending_raise_at(app_id: &str, now: Instant) {
    if let Ok(mut map) = pending().lock() {
        map.insert(app_id.to_string(), now);
    }
}

/// Take the pending raise for `app_id` if one is still live at `now`, clearing it either way.
fn take_pending_raise_at(app_id: &str, now: Instant) -> bool {
    let Ok(mut map) = pending().lock() else { return false };
    match map.get(app_id) {
        Some(at) if now.saturating_duration_since(*at) <= PENDING_RAISE_TTL => {
            map.remove(app_id);
            true
        }
        Some(_) => {
            map.remove(app_id);
            false
        }
        None => false,
    }
}

/// One launch on the person's desktop, settled: the window raised, or the app started.
///
/// A window that is listed and then cannot be raised is not a reason to start a second copy —
/// except for a program that hands a second launch over to its open window, where starting again
/// IS the raise. The usual cause of a miss is a window that closed between the reading and the
/// raise, or a machine without wlrctl, where nothing can raise anything; a second Blender over the
/// first is the bug this exists to end, so the miss is logged and the person's next launch reads a
/// fresh list.
pub(crate) fn settle(
    app_id: &str,
    program: Option<&str>,
    open: &[WindowEntry],
    raise: impl FnOnce(&str, &str) -> bool,
    start: impl FnOnce(),
) {
    match on_desktop(app_id, program, open) {
        OnDesktop::Focus { title, app_id: wayland_app_id } => {
            if raise(&title, &wayland_app_id) {
                tracing::info!(
                    app = app_id, window = %title,
                    "Already open: brought forward, not launched again"
                );
            } else if program.is_some_and(hands_over) {
                tracing::info!(
                    app = app_id, window = %title,
                    "Already open, and could not be brought forward; starting again to hand over"
                );
                start();
            } else {
                tracing::warn!(
                    app = app_id, window = %title,
                    "Already open, and could not be brought forward; not launched again"
                );
            }
        }
        OnDesktop::Launching => {
            record_pending_raise_at(app_id, Instant::now());
            tracing::info!(
                app = app_id,
                "still opening; it will be brought forward when its window appears"
            );
        }
        OnDesktop::Start => start(),
    }
}

/// The window for `app_id` has appeared to the compositor. If a click was waiting for it, bring it
/// forward now and clear the wait.
pub(crate) fn window_appeared(app_id: &str) {
    window_appeared_at(app_id, Instant::now(), |id| {
        crate::windows::present_app(id);
    });
}

/// [`window_appeared`] with an injectable clock and raise, so the cases are tested without a
/// compositor.
fn window_appeared_at(app_id: &str, now: Instant, raise: impl FnOnce(&str)) {
    if take_pending_raise_at(app_id, now) {
        raise(app_id);
    }
}

/// Start an app by name, unless its window is already on the person's desktop.
///
/// `app_id` is the id the window is listed under, `program` the binary the launch would run, and
/// `start` the launch itself. Called on the UI thread, inside the handler that asked: a mind's
/// call is routed here, while its caller is in scope (`mind_view::route_now`), and goes straight
/// to `start`, whose `spawn_launch` already sends it to Mind View or, for an app open on the
/// person's desktop, starts nothing and does not raise it. The person's launch reads the window
/// list on a worker, because reading it runs wlrctl and the UI thread never does that
/// (`windows::wlrctl_windows`). `start` then runs on that worker, where no caller is installed, so
/// the route it takes again is the person's desktop, as it was here.
pub(crate) fn focus_or_start(app_id: &str, program: Option<&str>, start: impl FnOnce() + Send + 'static) {
    let route = crate::mind_view::route_now(app_id);
    if route.mind_view.is_some() || !route.spawn {
        return start();
    }
    let (app_id, program) = (app_id.to_string(), program.map(str::to_string));
    let spawned = std::thread::Builder::new().name("yos-launch".into()).spawn(move || {
        settle(
            &app_id,
            program.as_deref(),
            &crate::windows::list_windows(),
            crate::windows::present_with_app_id,
            start,
        )
    });
    if let Err(e) = spawned {
        tracing::error!(error = %e, "Could not start a thread to open the app");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(title: &str, app_id: &str, wayland_app_id: &str) -> WindowEntry {
        WindowEntry {
            title: title.into(),
            app_id: app_id.into(),
            wayland_app_id: wayland_app_id.into(),
            icon_char: String::new(),
            subtitle: String::new(),
            seen_by_compositor: true,
        }
    }

    /// What `describe shell` listed on VM 520: a Blender an earlier shell started, known to the
    /// compositor and not to this shell's registry.
    fn blender_from_an_earlier_shell() -> WindowEntry {
        window("(Unsaved) - Blender 4.3.2", "blender", "Blender")
    }

    #[test]
    fn a_blender_the_shell_did_not_start_is_focused_not_launched_again() {
        let open = vec![window("Notes", "notes", ""), blender_from_an_earlier_shell()];
        assert_eq!(
            on_desktop("blender", Some("/usr/bin/blender"), &open),
            OnDesktop::Focus {
                title: "(Unsaved) - Blender 4.3.2".into(),
                app_id: "Blender".into()
            }
        );
    }

    #[test]
    fn an_app_with_no_window_listed_is_started() {
        let open = vec![window("Notes", "notes", "")];
        assert_eq!(on_desktop("blender", Some("/usr/bin/blender"), &open), OnDesktop::Start);
        assert_eq!(on_desktop("blender", None, &[]), OnDesktop::Start);
    }

    /// A browser opened before a restart comes back from the compositor as `chromium`; the
    /// program the launch would run is what names it as the browser.
    #[test]
    fn a_window_is_found_by_the_program_when_the_id_differs() {
        let open = vec![window("New Tab - Chromium", "chromium", "chromium")];
        assert_eq!(
            on_desktop("browser", Some("chromium"), &open),
            OnDesktop::Focus { title: "New Tab - Chromium".into(), app_id: "chromium".into() }
        );
        assert_eq!(on_desktop("browser", None, &open), OnDesktop::Start);
        // A window that declared no id is never matched by program: every one of ours is such a
        // window, and an empty id would otherwise answer to any binary.
        let ours = [window("Notes", "notes", "")];
        assert_eq!(on_desktop("editor", Some("yantrik-text-editor"), &ours), OnDesktop::Start);
    }

    #[test]
    fn an_open_window_is_raised_and_nothing_is_started() {
        let open = vec![blender_from_an_earlier_shell()];
        let (mut raised, mut started) = (None, false);
        let raise = |t: &str, _: &str| {
            raised = Some(t.to_string());
            true
        };
        settle("blender", Some("blender"), &open, raise, || started = true);
        assert_eq!(raised.as_deref(), Some("(Unsaved) - Blender 4.3.2"));
        assert!(!started, "a second Blender was started over the first");
    }

    #[test]
    fn a_window_that_will_not_come_forward_is_still_not_launched_again() {
        let mut started = false;
        settle("blender", None, &[blender_from_an_earlier_shell()], |_, _| false, || started = true);
        assert!(!started);
    }

    #[test]
    fn with_no_window_open_the_app_is_started_and_nothing_is_raised() {
        let (mut raised, mut started) = (false, false);
        let raise = |_: &str, _: &str| {
            raised = true;
            true
        };
        settle("blender", Some("blender"), &[window("Notes", "notes", "")], raise, || started = true);
        assert!(started && !raised);
    }

    /// A registry-only entry for the app is a third outcome: the app was launched and its window
    /// has not appeared to the compositor yet, so the click is remembered rather than answered
    /// with a second copy or a refusal.
    #[test]
    fn a_registry_only_browser_is_launching_not_open() {
        let mut registry_only = window("Browser", "browser", "");
        registry_only.seen_by_compositor = false;
        let open = vec![registry_only];
        assert_eq!(
            on_desktop("browser", Some("chromium"), &open),
            OnDesktop::Launching
        );
    }

    /// The Mac mini's case: the registry's "Browser" (not seen) sits beside the compositor's real
    /// Chromium window, and the focus names the compositor's title with its app_id so the raise can
    /// fall back to `app_id:chromium` when the title has moved on.
    #[test]
    fn the_mac_mini_case() {
        let mut registry_only = window("Browser", "browser", "");
        registry_only.seen_by_compositor = false;
        let open = vec![
            registry_only,
            window("New Tab - Chromium", "chromium", "chromium"),
        ];
        assert_eq!(
            on_desktop("browser", Some("chromium"), &open),
            OnDesktop::Focus { title: "New Tab - Chromium".into(), app_id: "chromium".into() }
        );
        // A present whose title misses falls back to app_id:chromium.
        assert_eq!(
            crate::windows::matchspecs_with_app_id("New Tab - Chromium", "chromium", &open),
            ["title:New Tab - Chromium", "app_id:chromium"]
        );
    }

    /// A click while the app is still opening records a pending raise and starts nothing; the
    /// window appearing later raises once, and a second appearance does nothing.
    #[test]
    fn a_click_while_launching_is_raised_when_the_window_comes() {
        let mut registry_only = window("Browser", "browser", "");
        registry_only.seen_by_compositor = false;
        let open = vec![registry_only];
        let mut started = false;
        settle("browser", Some("chromium"), &open, |_, _| true, || started = true);
        assert!(!started, "a second copy must not be started while the first is opening");

        let mut raised = 0;
        window_appeared_at("browser", Instant::now(), |_| raised += 1);
        assert_eq!(raised, 1, "the pending click is raised once the window appears");
        window_appeared_at("browser", Instant::now(), |_| raised += 1);
        assert_eq!(raised, 1, "a second appearance has no pending click to raise");
    }

    /// A pending raise is dropped once it is older than 30 seconds.
    #[test]
    fn a_pending_raise_expires_after_30_s() {
        let mut registry_only = window("Browser", "browser", "");
        registry_only.seen_by_compositor = false;
        let open = vec![registry_only];
        settle("browser", Some("chromium"), &open, |_, _| true, || {});

        let at = Instant::now();
        let mut raised = 0;
        window_appeared_at("browser", at + Duration::from_secs(31), |_| raised += 1);
        assert_eq!(raised, 0, "a raise older than 30 s is not honoured");
    }

    /// A listed window that cannot be raised is started again for a program that hands a second
    /// launch over to its open window, and never for Blender.
    #[test]
    fn an_unraisable_chromium_is_started_again_but_blender_never() {
        let chromium = window("New Tab - Chromium", "chromium", "chromium");
        let mut started = false;
        settle("browser", Some("chromium"), &[chromium], |_, _| false, || started = true);
        assert!(started, "Chromium's singleton opens a new window in the running process");

        let mut started = false;
        settle("blender", Some("blender"), &[blender_from_an_earlier_shell()], |_, _| false, || started = true);
        assert!(!started, "a second Blender is the bug settle exists to stop");
    }
}
