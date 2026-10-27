//! Which files an agent may have a document app read and write (#443).
//!
//! An editor runs as the person, and `describe` hands back the first 4000 characters of
//! the document in front. So `open` for an agent was reading any file the person can read
//! (`~/.ssh/id_ed25519`, then `describe`), and `save_as` was writing any file the person can
//! write: `~/.bashrc` with `overwrite=true` is code run as the person at their next login. An
//! agent names only files in the person's home outside its protected places, by the rule every
//! side shares (`yantrik_ipc_contracts::home_paths`). The person, choosing a file in the window
//! or with no token on the call, is not asked.
//!
//! `save_as` stays graded `standard`: a grade is fixed per action, and raising it would put the
//! ordinary new-document-then-`save_as` a mind uses to write its work behind an approval. What made
//! overwriting dangerous was where, not whether: the files that run as the person are on the
//! protected list, so an agent may replace a file only where it may write one at all.
//!
//! Each rule looks at the disk (whether the path is there, a file, a link), so each runs in the
//! job that opens or saves, never on the window's thread.

use std::path::{Path, PathBuf};

use yantrik_ipc_contracts::home_paths;

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

/// Refuse an agent's `open` of anything but a file in the home, outside its protected places.
pub fn may_read(agent: bool, path: &Path) -> Result<(), String> {
    may_read_in(&home(), agent, path)
}

/// [`may_read`] for this home. The rule itself, with the home passed in so nothing has to set
/// the process's environment to test it.
pub fn may_read_in(home: &Path, agent: bool, path: &Path) -> Result<(), String> {
    if !agent {
        return Ok(());
    }
    home_paths::may_read_file(&path.to_string_lossy(), home)
}

/// Refuse an agent's `save_as` or `save` to anywhere but a file in the home, outside its
/// protected places, in a folder already there, and not through a link.
pub fn may_write(agent: bool, path: &Path) -> Result<(), String> {
    may_write_in(&home(), agent, path)
}

/// [`may_write`] for this home.
pub fn may_write_in(home: &Path, agent: bool, path: &Path) -> Result<(), String> {
    if !agent {
        return Ok(());
    }
    home_paths::may_write_file(&path.to_string_lossy(), home)
}

/// Whether a document's file is one no agent is shown: a place an agent may not read. Decided when the
/// document takes the file (opened, saved as, followed after a move) and kept with the document. A document with
/// no file holds only what was typed or given to it, and is shown.
pub fn hidden_from_agents(path: &Path) -> bool {
    hidden_from_agents_in(&home(), path)
}

/// [`hidden_from_agents`] for this home.
pub fn hidden_from_agents_in(home: &Path, path: &Path) -> bool {
    home_paths::may_read_file(&path.to_string_lossy(), home).is_err()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rule for a home of this test's own: passed in, never set in the process's environment.
    #[test]
    fn an_agent_names_only_files_in_the_home() {
        let root = std::env::temp_dir().join(format!("document-agent-rule-{}", std::process::id()));
        let home_dir = root.join("home");
        let own = home_dir.join(".local/share/yantrik/snippets");
        std::fs::create_dir_all(&own).unwrap();
        std::fs::create_dir_all(home_dir.join("Documents")).unwrap();
        std::fs::create_dir_all(home_dir.join(".ssh")).unwrap();
        std::fs::create_dir_all(home_dir.join(".local/state/yantrik/notes/drafts")).unwrap();
        for file in [
            own.join("snippets.json"),
            home_dir.join(".ssh/id_ed25519"),
            home_dir.join(".local/state/yantrik/notes/drafts/note.json"),
            home_dir.join("Documents/plan.md"),
        ] {
            std::fs::write(&file, "SENTINEL").unwrap();
        }
        let outside = root.join("outside.md");
        std::fs::write(&outside, "SENTINEL").unwrap();
        let home = home_dir.as_path();

        // The person is not asked.
        assert_eq!(may_read_in(home, false, &home_dir.join(".ssh/id_ed25519")), Ok(()));
        assert_eq!(may_write_in(home, false, &outside), Ok(()));
        assert!(hidden_from_agents_in(home, &outside) && hidden_from_agents_in(home, &home_dir.join(".ssh/id_ed25519")));

        let ends = |got: Result<(), String>, why: &str| {
            let err = got.expect_err(why);
            assert!(err.ends_with(why), "{err}");
        };
        for protected in [
            own.join("snippets.json"),
            home_dir.join(".local/state/yantrik/notes/drafts/note.json"),
            home_dir.join(".ssh/id_ed25519"),
        ] {
            ends(may_read_in(home, true, &protected), " is protected");
            ends(may_write_in(home, true, &protected), " is protected");
        }
        ends(may_write_in(home, true, &own.join("new.md")), " is protected");
        ends(may_write_in(home, true, &home_dir.join(".bashrc")), " is protected");
        ends(may_read_in(home, true, &outside), " is outside");
        ends(may_read_in(home, true, Path::new("/etc/hostname")), " is outside");
        assert_eq!(may_read_in(home, true, &home_dir.join("Documents/plan.md")), Ok(()));
        assert_eq!(may_write_in(home, true, &home_dir.join("Documents/new.md")), Ok(()));
        assert!(!hidden_from_agents_in(home, &home_dir.join("Documents/plan.md")));
        let _ = std::fs::remove_dir_all(&root);
    }
}
