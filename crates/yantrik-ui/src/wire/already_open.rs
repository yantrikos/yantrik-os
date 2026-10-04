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

use crate::windows::WindowEntry;

/// What a launch on the person's desktop does, given the windows that are open there.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum OnDesktop {
    /// The app's window is open: bring the one with this title forward.
    Focus(String),
    /// No window of the app is listed: start it.
    Start,
}

/// Whether `app_id` already has a window in `open`.
///
/// By the shell's id, or by the program the launch would run (`windows::same_program`, the rule
/// the window list's merge uses), because the two disagree for a window this shell did not start:
/// a browser opened before a restart is listed as `chromium`, not `browser`, and the registry
/// that would have paired them is gone.
pub(crate) fn on_desktop(app_id: &str, program: Option<&str>, open: &[WindowEntry]) -> OnDesktop {
    open.iter()
        .find(|w| {
            w.app_id == app_id
                || program.is_some_and(|p| crate::windows::same_program(p, &w.wayland_app_id))
        })
        .map(|w| OnDesktop::Focus(w.title.clone()))
        .unwrap_or(OnDesktop::Start)
}

/// One launch on the person's desktop, settled: the window raised, or the app started.
///
/// A window that is listed and then cannot be raised is not a reason to start a second copy. The
/// usual cause is a window that closed between the reading and the raise, or a machine without
/// wlrctl, where nothing can raise anything; a second Blender over the first is the bug this
/// exists to end, so the miss is logged and the person's next launch reads a fresh list.
pub(crate) fn settle(
    app_id: &str,
    program: Option<&str>,
    open: &[WindowEntry],
    raise: impl FnOnce(&str) -> bool,
    start: impl FnOnce(),
) {
    match on_desktop(app_id, program, open) {
        OnDesktop::Focus(title) => {
            if raise(&title) {
                tracing::info!(
                    app = app_id, window = %title,
                    "Already open: brought forward, not launched again"
                );
            } else {
                tracing::warn!(
                    app = app_id, window = %title,
                    "Already open, and could not be brought forward; not launched again"
                );
            }
        }
        OnDesktop::Start => start(),
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
            crate::windows::present,
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
            OnDesktop::Focus("(Unsaved) - Blender 4.3.2".into())
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
            OnDesktop::Focus("New Tab - Chromium".into())
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
        let raise = |t: &str| {
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
        settle("blender", None, &[blender_from_an_earlier_shell()], |_| false, || started = true);
        assert!(!started);
    }

    #[test]
    fn with_no_window_open_the_app_is_started_and_nothing_is_raised() {
        let (mut raised, mut started) = (false, false);
        let raise = |_: &str| {
            raised = true;
            true
        };
        settle("blender", Some("blender"), &[window("Notes", "notes", "")], raise, || started = true);
        assert!(started && !raised);
    }
}
