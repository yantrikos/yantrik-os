//! Where an agent may have the download manager save a file.
//!
//! The download manager runs as the person, so `add` was writing a file the agent named the
//! source of into any folder the agent named: `save_dir=~/.config/autostart` with a URL ending
//! in `.desktop` is a program run as the person at their next login. An agent now saves only
//! where it may write a file at all, by the rule every side shares
//! (`yantrik_ipc_contracts::home_paths`, the one the editor's `save_as` uses): in the person's
//! home, outside its protected and hidden places, in a folder already there, not through a link.
//! The person, choosing a folder in the window or calling with no agent token, is not asked.

use std::path::{Path, PathBuf};

use yantrik_app_runtime::control::agent_is_calling;
use yantrik_ipc_contracts::home_paths;

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

/// Refuse an agent's download to anywhere it may not write `name` inside `dir`.
pub fn may_save(dir: &Path, name: &str) -> Result<(), String> {
    if !agent_is_calling() {
        return Ok(());
    }
    may_save_as_agent(dir, name, &home())
}

/// The rule itself, apart from who is calling, so it can be tested without a token.
pub fn may_save_as_agent(dir: &Path, name: &str, home: &Path) -> Result<(), String> {
    home_paths::may_write_file(&dir.join(name).to_string_lossy(), home)
        .map_err(|why| format!("an agent may not save a download there: {why}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home_with(dirs: &[&str]) -> (PathBuf, impl Drop) {
        struct Gone(PathBuf);
        impl Drop for Gone {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        // One folder per call: tests run in parallel, and two homes with the same shape must
        // not share a folder that the first to finish removes.
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("yantrik-dl-rule-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for d in dirs {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        let home = root.canonicalize().unwrap();
        (home.clone(), Gone(home))
    }

    #[test]
    fn an_agent_saves_into_an_ordinary_folder_of_the_home() {
        let (home, _g) = home_with(&["Downloads"]);
        assert!(may_save_as_agent(&home.join("Downloads"), "paper.pdf", &home).is_ok());
    }

    #[test]
    fn an_agent_never_saves_where_the_person_runs_programs_or_keeps_settings() {
        let (home, _g) = home_with(&[".config/autostart", ".local/share/applications", ".ssh", "Downloads"]);
        for (dir, name) in [
            (".config/autostart", "evil.desktop"),
            (".local/share/applications", "evil.desktop"),
            (".ssh", "authorized_keys"),
            ("", ".bashrc"),
            ("", ".profile"),
        ] {
            let at = home.join(dir);
            assert!(may_save_as_agent(&at, name, &home).is_err(), "{dir}/{name} must be refused");
        }
    }

    #[test]
    fn an_agent_never_saves_outside_the_home_or_into_a_folder_that_is_not_there() {
        let (home, _g) = home_with(&["Downloads"]);
        assert!(may_save_as_agent(Path::new("/etc"), "x.conf", &home).is_err());
        assert!(may_save_as_agent(Path::new("/tmp"), "x.sh", &home).is_err());
        assert!(may_save_as_agent(&home.join("new/deep/folder"), "x.txt", &home).is_err());
    }
}
