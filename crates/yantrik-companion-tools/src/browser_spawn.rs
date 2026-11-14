//! How the companion's own Chromium is started (#1124, round 8, finding 1): always through
//! `workshops::browser_command`, so a browser a Mind opens has no view of the workshops
//! (`file://` included). The spawn in `browser.rs` builds its command here and nowhere else. That no other file starts a
//! Chromium is held by `xtask check-architecture` (rule `mind-spawn`), not by a test of one file.

use std::path::Path;
use std::process::Command;

use yantrik_ipc_contracts::home_paths::workshops;

pub use yantrik_browser_ports::quiet_args;

/// `binary args` as a command that runs in the browser view (the workshops base hidden), or the
/// reason it must not run: nothing is started unwrapped on a machine that holds workshops.
pub fn walled(home: &Path, binary: &str, args: &[String]) -> Result<Command, String> {
    let (program, argv) = workshops::browser_command(home, binary, args)?;
    let mut command = Command::new(program);
    command.args(argv);
    Ok(command)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    fn argv(c: &Command) -> Vec<String> {
        c.get_args().map(OsStr::to_string_lossy).map(|s| s.into_owned()).collect()
    }

    /// The command is exactly `browser_command`'s, not a parallel construction.
    #[test]
    fn walled_is_browser_commands_answer() {
        let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
        let args = vec!["--x".to_string()];
        match (walled(&home, "chromium", &args), workshops::browser_command(&home, "chromium", &args)) {
            (Ok(c), Ok((program, want))) => {
                assert_eq!(c.get_program().to_string_lossy(), program);
                assert_eq!(argv(&c), want);
            }
            (Err(e), Err(w)) => assert_eq!(e, w),
            other => panic!("{other:?}"),
        }
    }
}
