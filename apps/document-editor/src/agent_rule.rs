//! Which files an agent may have yDoc read and write, and which document it is not shown (#443).
//!
//! yDoc runs as the person, and `describe documents` hands back the first 4000 characters of the
//! document. So `open` for an agent read any file the person can read - the snippets store,
//! `~/.ssh/id_ed25519` - and `describe` gave it back; `save_as` and `export_markdown` wrote any
//! file the person can write (security review of #942, finding 1). An agent now names only files
//! in the person's home outside its protected places, by the rule every side shares
//! (`yantrik_ipc_contracts::home_paths`), the same rule the Text Editor keeps.
//!
//! The rule sits at one door: `admit`, which every published action but `show` passes through
//! before it runs (`publish_control`'s `action!`). The person, choosing a file in the window or
//! calling with no token, is not asked.

use std::path::{Path, PathBuf};

use yantrik_app_runtime::control::agent_is_calling;
use yantrik_ipc_contracts::home_paths;

/// How a document an agent is not shown is named to it, in place of its title and path.
pub const HIDDEN: &str = "(hidden)";

/// Why an agent's action on the document in front is refused when that document is hidden.
pub const HIDDEN_REFUSAL: &str = "The document in front is a file an agent is not shown, and its \
                                  text is left alone; `open` another or start a `new` one.";

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

/// Refuse an agent's `open` of anything but a file in the home, outside its protected places.
pub fn may_read(path: &Path) -> Result<(), String> {
    if !agent_is_calling() {
        return Ok(());
    }
    home_paths::may_read_file(&path.to_string_lossy(), &home())
}

/// Refuse an agent's write to anywhere but a file in the home, outside its protected places, in
/// a folder already there, and not through a link.
pub fn may_write(path: &Path) -> Result<(), String> {
    if !agent_is_calling() {
        return Ok(());
    }
    home_paths::may_write_file(&path.to_string_lossy(), &home())
}

/// Why the document's text is kept from this caller, if it is: a document the person opened
/// from a place an agent may not read is not read back to one. A document with no file yet holds
/// only what was typed or given to it, and is shown.
pub fn hidden_from_caller(path: Option<&Path>) -> Option<String> {
    if !agent_is_calling() {
        return None;
    }
    home_paths::may_read_file(&path?.to_string_lossy(), &home()).err()
}

/// The one door: whether this caller may run `action` with `args`, against the document in
/// front, whose file is `document`. `expand` turns an argument into the path the action would
/// use, so the path checked is the path opened or written.
///
/// An action not named here is refused while the document is hidden, so a new action that reads
/// or changes the text is closed to an agent until it is placed.
pub fn admit(
    action: &str,
    args: &serde_json::Value,
    document: Option<&Path>,
    expand: impl Fn(&str) -> PathBuf,
) -> Result<(), String> {
    if !agent_is_calling() {
        return Ok(());
    }
    let named = |key: &str| args[key].as_str().map(&expand);
    let shown = || match hidden_from_caller(document) {
        Some(_) => Err(HIDDEN_REFUSAL.to_string()),
        None => Ok(()),
    };
    match action {
        // Replacing what is in front reads nothing of it; the file named is what is read.
        "open" => named("path").map_or(Ok(()), |p| may_read(&p)),
        "new" | "show" => Ok(()),
        // Its own file: the write rule decides, as it does for any other path.
        "save" => document.map_or(Ok(()), may_write),
        // A copy of a hidden document somewhere an agent may read is a read of it.
        "save_as" | "export_markdown" => {
            shown()?;
            named("path").map_or(Ok(()), |p| may_write(&p))
        }
        // `find` answers "does it contain X?" by its count; the rest change text never shown.
        _ => shown(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yantrik_app_runtime::control::AgentTokenScope;

    fn expand(value: &str) -> PathBuf {
        match value.strip_prefix("~/") {
            Some(rest) => home().join(rest),
            None => PathBuf::from(value),
        }
    }

    /// One test, because it points `HOME` at a home of its own and nothing else here reads it.
    #[test]
    fn an_agent_names_only_files_in_the_home() {
        let root = std::env::temp_dir().join(format!("ydoc-agent-rule-{}", std::process::id()));
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
        std::env::set_var("HOME", &home_dir);

        let open = |path: &str| admit("open", &serde_json::json!({ "path": path }), None, expand);
        let write = |action: &str, path: &str| {
            admit(action, &serde_json::json!({ "path": path }), None, expand)
        };
        let ends = |got: Result<(), String>, why: &str| {
            let err = got.expect_err(why);
            assert!(err.ends_with(why), "{err}");
        };

        // The person is not asked.
        assert_eq!(open("~/.ssh/id_ed25519"), Ok(()));
        assert_eq!(admit("find", &serde_json::json!({}), Some(&outside), expand), Ok(()));

        let agent = AgentTokenScope::enter(Some("tok-ydoc-test".into()));
        for protected in [
            "~/.local/share/yantrik/snippets/snippets.json",
            "~/.local/state/yantrik/notes/drafts/note.json",
            "~/.ssh/id_ed25519",
        ] {
            ends(open(protected), " is protected");
            ends(write("save_as", protected), " is protected");
            ends(write("export_markdown", protected), " is protected");
        }
        ends(write("save_as", "~/.local/share/yantrik/snippets/new.md"), " is protected");
        ends(write("export_markdown", "~/.bashrc"), " is protected");
        ends(open(&outside.display().to_string()), " is outside");
        ends(open("/etc/hostname"), " is outside");
        assert_eq!(open("~/Documents/plan.md"), Ok(()));
        assert_eq!(write("save_as", "~/Documents/new.md"), Ok(()));
        assert_eq!(write("export_markdown", "~/Documents/copy.md"), Ok(()));

        // The document in front is the person's, from outside the home: an agent neither reads
        // nor changes it, and its own file is not written for it.
        let theirs = Some(outside.as_path());
        assert!(hidden_from_caller(theirs).is_some());
        for action in ["find", "replace_all", "set_content", "append", "save_as", "export_markdown"] {
            let got = admit(action, &serde_json::json!({ "path": "~/Documents/x.md" }), theirs, expand);
            assert_eq!(got, Err(HIDDEN_REFUSAL.to_string()), "{action}");
        }
        ends(admit("save", &serde_json::json!({}), theirs, expand), " is outside");
        assert_eq!(admit("new", &serde_json::json!({}), theirs, expand), Ok(()));
        assert_eq!(open("~/Documents/plan.md"), Ok(()), "opening another is not a read of this one");
        let mine = home_dir.join("Documents/plan.md");
        assert_eq!(hidden_from_caller(Some(&mine)), None);
        assert_eq!(hidden_from_caller(None), None, "a document with no file yet is shown");
        drop(agent);

        assert_eq!(hidden_from_caller(theirs), None, "the person sees their own document");
        let _ = std::fs::remove_dir_all(&root);
    }
}
