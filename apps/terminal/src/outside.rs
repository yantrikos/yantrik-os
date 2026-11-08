//! The shell a Mind's keys reach (#1124, H1).
//!
//! A Mind never types into the person's own shell: that shell sees the workshops root as the
//! person does, and it has `sudo`. A tab a Mind opens is started in the outside view instead
//! (`bwrap` over the host with a tmpfs over the whole of `/var/lib/yantrik/workshops` and over
//! `/run/user/<uid>`, `home_paths::workshops::outside_command`), so the workshops do not exist for
//! anything typed into it, hard links into it cannot be made, a `cd` in the same line or a changed
//! working directory is no way back in, and the person's session (the bus, `systemd/private`, the
//! Yantrik sockets) is not there to reach: a Mind's tab cannot call `yos`. The person's own tabs
//! stay as they were, with `sudo`.

use std::path::{Path, PathBuf};

use yantrik_ipc_contracts::home_paths::workshops;

use crate::session::{Notify, Session};

/// The program and arguments of an outside shell for the home `home`.
pub fn command(home: &Path, shell: &str) -> Result<(String, Vec<String>), String> {
    workshops::outside_command(home, shell, &["-i".to_string()])
}

/// The folder an outside shell starts in: `dir`, unless that is in a workshop, which `bwrap` cannot
/// enter (it is masked) and which nothing here should be started in.
pub fn start_dir(home: &Path, dir: PathBuf) -> PathBuf {
    match workshops::holds(&dir, home) {
        true => home.to_path_buf(),
        false => dir,
    }
}

/// Start an interactive shell in the outside view.
pub fn spawn(dir: &Path, rows: u16, cols: u16, notify: Notify) -> anyhow::Result<Session> {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| "/".into()));
    let (program, args) = command(&home, &Session::login_shell()).map_err(anyhow::Error::msg)?;
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    Session::spawn_shell(&program, &args, &start_dir(&home, dir.to_path_buf()), rows, cols, notify)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn an_outside_shell_is_the_interactive_shell_behind_the_mask() {
        workshops::roots_under_home_for_tests();
        let h = std::env::temp_dir().join(format!("yantrik-terminal-outside-{}", std::process::id()));
        let inside = workshops::root(&h).join("hermes/shop");
        std::fs::create_dir_all(&inside).unwrap();
        match command(&h, "/bin/bash") {
            Ok((program, args)) => {
                assert!(program.ends_with("bwrap"));
                let base = workshops::base(&h).display().to_string();
                assert!(args.windows(2).any(|w| w[0] == "--tmpfs" && w[1] == base), "the whole base is masked");
                assert_eq!(&args[args.len() - 2..], ["/bin/bash", "-i"]);
            }
            // No bwrap here: a shell is not started unhidden over a root that exists.
            Err(e) => assert!(e.contains("nothing was run"), "{e}"),
        }
        let home = h.canonicalize().unwrap();
        assert_eq!(start_dir(&home, inside), home, "never started in a workshop");
        assert_eq!(start_dir(&home, home.join("Documents")), home.join("Documents"));
        let _ = std::fs::remove_dir_all(&h);
    }
}
