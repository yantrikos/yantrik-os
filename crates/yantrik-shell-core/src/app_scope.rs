//! The systemd scope an app the dock launches is born in (#736).
//!
//! On the Mac mini, `app-org.chromium.Chromium-<pid>.scope` units ended `failed
//! (Result: resources)` with "Couldn't move process <pid> to requested cgroup ... No such
//! process". The shell started Chromium as a bare child, so nothing had put it in a scope, and
//! Chromium asks systemd for one itself, naming its own pid. A launch that hands over to a
//! Chromium already running (or that re-execs and exits) is gone by the time systemd tries to
//! move that pid, and the scope is left failed in `systemctl --user list-units --failed`.
//!
//! So the launch is made inside a scope from the start: `systemd-run --user --scope` creates the
//! unit for its own pid and then execs the app in place, so the process is in the cgroup before
//! the app's first instruction runs, and the pid the shell waits on is the app's own. If an app
//! still asks for a scope of its own for a pid that then exits, the reaper clears that failed
//! unit for exactly the pid it launched ([`reset_failed_args`]).
//!
//! Everything here that decides something is pure, so it is tested without a user manager; the
//! two helpers at the bottom only look at the filesystem.

use std::path::Path;

/// The launcher field of the unit name, per the XDG "desktop entries and systemd" naming
/// (`app-<launcher>-<app id>-<random>.scope`), so a reader can tell the shell started it.
const LAUNCHER: &str = "yantrik";

/// The scope unit one launch runs in: `app-yantrik-<app id>-<shell pid>_<seq>.scope`.
///
/// `shell_pid` and `seq` together keep the name unique across launches and across shell
/// restarts, so a launch never asks for a unit name an earlier one still holds. A `-` inside the
/// app id is escaped (`\x2d`) as the naming convention asks, because `-` separates the fields;
/// anything systemd does not allow in a unit name becomes `_`.
pub fn unit_name(app_id: &str, shell_pid: u32, seq: u64) -> String {
    let mut id = String::with_capacity(app_id.len());
    for c in app_id.chars() {
        match c {
            '-' => id.push_str("\\x2d"),
            c if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':') => id.push(c),
            _ => id.push('_'),
        }
    }
    if id.is_empty() {
        id.push_str("app");
    }
    format!("app-{LAUNCHER}-{id}-{shell_pid}_{seq}.scope")
}

/// The `systemd-run` arguments that go before the app's own program and arguments.
///
/// `--scope` runs the command in this process after the unit exists rather than forking it off
/// under the manager, which is what keeps the shell's pid, exit status and reaper meaning what
/// they did. `--collect` lets systemd drop the unit even when it ends failed, so a scope that
/// does fail does not stay in the failed list. `--quiet` keeps "Running as unit" out of the
/// app's stderr.
pub fn systemd_run_args(unit: &str) -> Vec<String> {
    vec![
        "--user".into(),
        "--scope".into(),
        "--quiet".into(),
        "--collect".into(),
        format!("--unit={unit}"),
        "--".into(),
    ]
}

/// The `systemctl` arguments that clear what a launch may have left failed once its process has
/// exited: the shell's own scope, and any scope the app made for that same pid itself
/// (`app-*-<pid>.scope`, the name Chromium uses). `reset-failed` takes glob patterns and is a no-op
/// for units that are not failed, so this is safe to run after every exit. The app's own scope
/// goes first: the shell's is usually already collected (`--collect`), and an error about that
/// name must not be what stops the other from being cleared.
pub fn reset_failed_args(unit: &str, pid: u32) -> Vec<String> {
    vec![
        "--user".into(),
        "reset-failed".into(),
        format!("app-*-{pid}.scope"),
        unit.to_string(),
    ]
}

/// Whether a launch whose scope reports `active_state` (systemd's `ActiveState`) can be counted
/// as a running app.
///
/// A `failed` scope never held a running app, whatever the shell's registry last recorded, so the
/// dock must not show it as open. Every other state is left to the pid check the registry already
/// makes: `inactive` is also what an app that moved itself into a scope of its own leaves behind
/// (and what a collected unit reads as), so it is not evidence the app is gone.
pub fn scope_counts_as_running(active_state: &str) -> bool {
    active_state.trim() != "failed"
}

/// Whether the person's systemd user manager can be asked for a scope: its runtime directory is
/// there and the session bus `systemd-run --user` talks to is reachable. Without both, wrapping a
/// launch in `systemd-run` would turn a working launch into a failed one, so the caller launches
/// the app bare instead.
pub fn user_manager_reachable(runtime_dir: Option<&Path>, session_bus_address_set: bool) -> bool {
    let Some(runtime) = runtime_dir else { return false };
    runtime.join("systemd").is_dir() && (session_bus_address_set || runtime.join("bus").exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_name_follows_the_app_launcher_id_random_shape() {
        assert_eq!(unit_name("chromium", 1234, 7), "app-yantrik-chromium-1234_7.scope");
        assert_eq!(
            unit_name("org.chromium.Chromium", 1, 0),
            "app-yantrik-org.chromium.Chromium-1_0.scope"
        );
    }

    #[test]
    fn unit_name_escapes_dashes_and_replaces_what_systemd_refuses() {
        assert_eq!(unit_name("google-chrome", 9, 2), "app-yantrik-google\\x2dchrome-9_2.scope");
        assert_eq!(unit_name("my app/€", 9, 2), "app-yantrik-my_app__-9_2.scope");
        assert_eq!(unit_name("", 9, 2), "app-yantrik-app-9_2.scope");
    }

    #[test]
    fn two_launches_never_share_a_unit_name() {
        assert_ne!(unit_name("chromium", 10, 1), unit_name("chromium", 10, 2));
        assert_ne!(unit_name("chromium", 10, 1), unit_name("chromium", 11, 1));
    }

    /// The child is born in the scope: `--scope` (exec in place, no fork under the manager), the
    /// user manager, the unit named, and `--` last so an app argument is never read as an option.
    #[test]
    fn the_argv_puts_the_app_inside_the_scope() {
        let unit = unit_name("chromium", 42, 1);
        let args = systemd_run_args(&unit);
        assert_eq!(
            args,
            ["--user", "--scope", "--quiet", "--collect", "--unit=app-yantrik-chromium-42_1.scope", "--"]
        );
        assert_eq!(args.last().map(String::as_str), Some("--"));
    }

    #[test]
    fn reset_failed_clears_the_shells_scope_and_the_apps_own_for_that_pid() {
        assert_eq!(
            reset_failed_args("app-yantrik-chromium-42_1.scope", 5150),
            ["--user", "reset-failed", "app-*-5150.scope", "app-yantrik-chromium-42_1.scope"]
        );
    }

    #[test]
    fn a_failed_scope_is_never_a_running_app() {
        assert!(!scope_counts_as_running("failed"));
        assert!(!scope_counts_as_running("failed\n"));
        assert!(scope_counts_as_running("active"));
        assert!(scope_counts_as_running("activating"));
        // An app that moved into a scope of its own empties ours; that is not a failed launch.
        assert!(scope_counts_as_running("inactive"));
        // Unknown is not failure: the pid check decides.
        assert!(scope_counts_as_running(""));
    }

    #[test]
    fn no_user_manager_means_no_scope() {
        assert!(!user_manager_reachable(None, true));
        let dir = std::env::temp_dir().join(format!("yos-scope-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!user_manager_reachable(Some(&dir), true), "no systemd dir: no user manager");
        std::fs::create_dir_all(dir.join("systemd")).unwrap();
        assert!(!user_manager_reachable(Some(&dir), false), "no bus to reach it on");
        assert!(user_manager_reachable(Some(&dir), true));
        std::fs::write(dir.join("bus"), b"").unwrap();
        assert!(user_manager_reachable(Some(&dir), false));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
