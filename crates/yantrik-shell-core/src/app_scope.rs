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

/// The most bytes the escaped app id may take in a unit name. With the prefix, two pids' worth of
/// digits and `.scope`, this keeps the whole name well under systemd's 255.
pub const MAX_ESCAPED_ID: usize = 150;

/// The scope unit one launch runs in: `app-yantrik-<app id>-<shell pid>_<seq>.scope`.
///
/// `shell_pid` and `seq` together keep the name unique across launches and across shell
/// restarts, so a launch never asks for a unit name an earlier one still holds. A `-` inside the
/// app id is escaped (`\x2d`) as the naming convention asks, because `-` separates the fields;
/// anything systemd does not allow in a unit name becomes `_`.
///
/// systemd refuses a unit name over 255 bytes, and every `-` costs four, so the escaped id is cut
/// at [`MAX_ESCAPED_ID`] bytes, between whole escapes: a refused name would make `systemd-run`
/// exit without running the app at all.
pub fn unit_name(app_id: &str, shell_pid: u32, seq: u64) -> String {
    let mut id = String::with_capacity(app_id.len().min(MAX_ESCAPED_ID));
    let mut buf = [0u8; 4];
    for c in app_id.chars() {
        let piece: &str = match c {
            '-' => "\\x2d",
            c if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':') => c.encode_utf8(&mut buf),
            _ => "_",
        };
        if id.len() + piece.len() > MAX_ESCAPED_ID {
            break;
        }
        id.push_str(piece);
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
/// exited: any scope the app made for that same pid itself (`app-*-<pid>.scope`, the name
/// Chromium uses), whether or not the shell gave it a scope, and the shell's own scope when it
/// did. `reset-failed` takes glob patterns and is a no-op for units that are not failed, so this
/// is safe to run after every exit. The app's own scope goes first: the shell's is usually
/// already collected (`--collect`), and an error about that name must not be what stops the
/// other from being cleared.
pub fn reset_failed_args(unit: Option<&str>, pid: u32) -> Vec<String> {
    let mut args = vec!["--user".into(), "reset-failed".into(), format!("app-*-{pid}.scope")];
    args.extend(unit.map(str::to_string));
    args
}

/// Whether a scoped launch's exit means `systemd-run` could not make the scope, so the app should
/// be started once more without one rather than recorded as an app that cannot start.
///
/// `systemd-run` exits non-zero without running the app when the user manager refuses the unit
/// (a manager that is stopping, a bus that is not systemd's). From the outside that is a non-zero
/// exit inside the grace period, the same as an app that crashed at once, so the retry is made
/// once and the caller stops scoping for the rest of the session: an app that really cannot start
/// then fails bare, and is recorded as it always was.
pub fn retry_bare(scoped: bool, lived_ms: u64, grace_ms: u64, status_success: bool) -> bool {
    scoped && lived_ms < grace_ms && !status_success
}

/// Whether the person's systemd user manager can be asked for a scope: its private socket is
/// there and the session bus `systemd-run --user` talks to is reachable. Without both, wrapping a
/// launch in `systemd-run` would turn a working launch into a failed one, so the caller launches
/// the app bare instead. The socket, not the `systemd` directory: the directory outlives a
/// manager that crashed or is stopping.
pub fn user_manager_reachable(runtime_dir: Option<&Path>, session_bus_address_set: bool) -> bool {
    let Some(runtime) = runtime_dir else { return false };
    runtime.join("systemd/private").exists() && (session_bus_address_set || runtime.join("bus").exists())
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
            reset_failed_args(Some("app-yantrik-chromium-42_1.scope"), 5150),
            ["--user", "reset-failed", "app-*-5150.scope", "app-yantrik-chromium-42_1.scope"]
        );
    }

    /// A launch without a scope of ours (Mind View, no user manager) still has the app's own
    /// hand-over scope for its pid cleared.
    #[test]
    fn reset_failed_without_a_scope_of_ours_still_clears_the_apps_own() {
        assert_eq!(reset_failed_args(None, 77), ["--user", "reset-failed", "app-*-77.scope"]);
    }

    #[test]
    fn a_long_dashed_id_is_cut_between_whole_escapes_and_stays_under_systemds_limit() {
        let id = "a-".repeat(200);
        let name = unit_name(&id, u32::MAX, u64::MAX);
        assert!(name.len() <= 255, "{} bytes: systemd would refuse it", name.len());
        let escaped = name
            .strip_prefix("app-yantrik-")
            .and_then(|n| n.rsplit_once('-'))
            .map(|(id, _)| id)
            .unwrap();
        assert!(escaped.len() <= MAX_ESCAPED_ID);
        // Never a half escape at the cut: the id is whole `a\x2d` pairs, maybe one trailing `a`.
        let rest = escaped.replace("a\\x2d", "");
        assert!(rest.is_empty() || rest == "a", "cut through an escape: {escaped}");
    }

    /// Only a scoped launch that died non-zero inside the grace is retried bare; a clean close, an
    /// app that lived, and an unscoped launch are what they always were.
    #[test]
    fn a_scoped_launch_that_failed_at_once_is_retried_bare() {
        assert!(retry_bare(true, 40, 3_000, false));
        assert!(!retry_bare(true, 40, 3_000, true), "a clean exit is a handover or a close");
        assert!(!retry_bare(true, 5_000, 3_000, false), "an app that lived is not a scope failure");
        assert!(!retry_bare(false, 40, 3_000, false), "a bare launch has nothing to retry without");
    }

    #[test]
    fn no_user_manager_means_no_scope() {
        assert!(!user_manager_reachable(None, true));
        let dir = std::env::temp_dir().join(format!("yos-scope-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!user_manager_reachable(Some(&dir), true), "no systemd dir: no user manager");
        std::fs::create_dir_all(dir.join("systemd")).unwrap();
        assert!(
            !user_manager_reachable(Some(&dir), true),
            "the directory without the private socket is a manager that is gone or going"
        );
        std::fs::write(dir.join("systemd/private"), b"").unwrap();
        assert!(!user_manager_reachable(Some(&dir), false), "no bus to reach it on");
        assert!(user_manager_reachable(Some(&dir), true));
        std::fs::write(dir.join("bus"), b"").unwrap();
        assert!(user_manager_reachable(Some(&dir), false));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
