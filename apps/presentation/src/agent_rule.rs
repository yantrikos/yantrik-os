//! Which files an agent may have yPresent read and write, and which deck it is not shown (#443).
//!
//! yPresent runs as the person. `open` for an agent parsed any file the person can read, and
//! `save_as` and `export_markdown` wrote any file the person can write, `~/.bashrc` and Yantrik's
//! own data folders included (security review of #942, finding 1). An agent now names only files
//! in the person's home outside its protected places, by the rule every side shares
//! (`yantrik_ipc_contracts::home_paths`), the same rule the Text Editor and yDoc keep.
//!
//! The rule sits at one door, [`Gated`], which every action is published through, so an action
//! added tomorrow is held to it without its author having to remember. The person, at the window
//! or calling with no token, is not asked.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use yantrik_app_runtime::control::{agent_is_calling, Action, App};
use yantrik_ipc_contracts::home_paths;

/// How a deck an agent is not shown is named to it, in place of its title and file.
pub const HIDDEN: &str = "(hidden)";

/// Why an agent's action on the deck in front is refused when that deck is hidden.
pub const HIDDEN_REFUSAL: &str = "The deck in front is a file an agent is not shown, and it is left \
                                  alone; `open` another or start a `new_deck`.";

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

fn may_read(path: &Path) -> Result<(), String> {
    home_paths::may_read_file(&path.to_string_lossy(), &home())
}

fn may_write(path: &Path) -> Result<(), String> {
    home_paths::may_write_file(&path.to_string_lossy(), &home())
}

/// Why the deck's slides are kept from this caller, if they are: a deck the person opened from a
/// place an agent may not read is not read back to one. A deck with no file yet is shown.
pub fn hidden_from_caller(path: Option<&Path>) -> Option<String> {
    if !agent_is_calling() {
        return None;
    }
    may_read(path?).err()
}

/// The deck in front, as the rule needs to see it.
pub struct Front<'a> {
    pub path: Option<&'a Path>,
    pub dirty: bool,
}

/// Whether this caller may run `action` with `args` against the deck in front. `expand` turns an
/// argument into the path the action would use, so the path checked is the path used.
///
/// An action not named here is refused while the deck is hidden, so a new action that reads or
/// changes the slides is closed to an agent until it is placed.
pub fn admit(
    action: &str,
    args: &serde_json::Value,
    front: &Front,
    expand: impl Fn(&str) -> PathBuf,
) -> Result<(), String> {
    if !agent_is_calling() {
        return Ok(());
    }
    let named = |key: &str| {
        args[key].as_str().map(str::trim).filter(|v| !v.is_empty()).map(&expand)
    };
    let shown = || match hidden_from_caller(front.path) {
        Some(_) => Err(HIDDEN_REFUSAL.to_string()),
        None => Ok(()),
    };
    // Its own file: a deck with a home is written there, by the write rule. One with none goes to
    // a fresh file of this app's choosing under ~/Documents/Presentations.
    let own_file = || front.path.map_or(Ok(()), may_write);
    match action {
        // Open and New first write a deck with unsaved changes to its own file.
        "open" => {
            if front.dirty {
                own_file()?;
            }
            named("path").map_or(Ok(()), |p| may_read(&p))
        }
        "new_deck" => {
            if front.dirty {
                own_file()?;
            }
            Ok(())
        }
        "show" => Ok(()),
        "save" => own_file(),
        // A copy of a hidden deck somewhere an agent may read is a read of it.
        "save_as" | "export_markdown" => {
            shown()?;
            named("path").map_or(Ok(()), |p| may_write(&p))
        }
        _ => shown(),
    }
}

/// Lends the deck in front to a rule, for the length of one call.
type Lend = Rc<dyn Fn(&mut dyn FnMut(&Front))>;

/// The surface, with every action published through [`admit`].
pub struct Gated {
    surface: App,
    front: Lend,
    refused: Rc<dyn Fn(&str)>,
}

impl Gated {
    /// `front` lends the deck in front to the rule; `refused` says a refusal on the window, as
    /// every other refusal is said.
    pub fn new(
        surface: App,
        front: impl Fn(&mut dyn FnMut(&Front)) + 'static,
        refused: impl Fn(&str) + 'static,
    ) -> Self {
        Self { surface, front: Rc::new(front), refused: Rc::new(refused) }
    }

    pub fn action(
        self,
        spec: Action,
        f: impl Fn(&serde_json::Value) -> Result<serde_json::Value, String> + 'static,
    ) -> Self {
        let name = spec.name.clone();
        let (front, refused) = (self.front.clone(), self.refused.clone());
        let surface = self.surface.action(spec, move |args| {
            let mut verdict = Ok(());
            front(&mut |deck: &Front| verdict = admit(&name, args, deck, crate::expanded));
            if let Err(why) = verdict {
                refused(&why);
                return Err(why);
            }
            f(args)
        });
        Self { surface, ..self }
    }

    pub fn serve(self) {
        self.surface.serve();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yantrik_app_runtime::control::AgentTokenScope;

    /// One test, because it points `HOME` at a home of its own and nothing else here reads it.
    #[test]
    fn an_agent_names_only_files_in_the_home() {
        let root = std::env::temp_dir().join(format!("ypresent-agent-rule-{}", std::process::id()));
        let home_dir = root.join("home");
        for dir in [".local/share/yantrik/snippets", ".local/state/yantrik", ".ssh", "Documents"] {
            std::fs::create_dir_all(home_dir.join(dir)).unwrap();
        }
        for file in [".local/share/yantrik/snippets/snippets.json", ".ssh/id_ed25519", "Documents/talk.ydeck"] {
            std::fs::write(home_dir.join(file), "SENTINEL").unwrap();
        }
        let outside = root.join("theirs.ydeck");
        std::fs::write(&outside, "SENTINEL").unwrap();
        std::env::set_var("HOME", &home_dir);

        let blank = Front { path: None, dirty: false };
        let call = |action: &str, path: &str, front: &Front| {
            admit(action, &serde_json::json!({ "path": path }), front, crate::expanded)
        };
        let ends = |got: Result<(), String>, why: &str| {
            let err = got.expect_err(why);
            assert!(err.ends_with(why), "{err}");
        };

        // The person is not asked.
        assert_eq!(call("open", "~/.ssh/id_ed25519", &blank), Ok(()));

        let agent = AgentTokenScope::enter(Some("tok-ypresent-test".into()));
        for protected in ["~/.local/share/yantrik/snippets/snippets.json", "~/.ssh/id_ed25519"] {
            for action in ["open", "save_as", "export_markdown"] {
                ends(call(action, protected, &blank), " is protected");
            }
        }
        ends(call("save_as", "~/.local/state/yantrik/new.ydeck", &blank), " is protected");
        ends(call("export_markdown", "~/.bashrc", &blank), " is protected");
        ends(call("open", "/etc/hostname", &blank), " is outside");
        assert_eq!(call("open", "~/Documents/talk.ydeck", &blank), Ok(()));
        assert_eq!(call("save_as", "~/Documents/new.ydeck", &blank), Ok(()));
        assert_eq!(call("save", "", &blank), Ok(()), "a deck with no home goes to a fresh file");

        // The deck in front is the person's, from outside the home: an agent neither reads nor
        // changes it, nor writes its file - not by Save, nor by the save before Open and New.
        let theirs = Front { path: Some(outside.as_path()), dirty: true };
        for action in ["go_to", "next", "set_slide", "search", "present", "save_as", "export_markdown"] {
            assert_eq!(call(action, "~/Documents/x.md", &theirs), Err(HIDDEN_REFUSAL.to_string()), "{action}");
        }
        ends(call("save", "", &theirs), " is outside");
        ends(call("open", "~/Documents/talk.ydeck", &theirs), " is outside");
        ends(call("new_deck", "", &theirs), " is outside");
        let clean = Front { path: Some(outside.as_path()), dirty: false };
        assert_eq!(call("open", "~/Documents/talk.ydeck", &clean), Ok(()), "nothing of theirs is written");
        assert_eq!(call("show", "", &theirs), Ok(()));
        drop(agent);

        assert_eq!(hidden_from_caller(Some(&outside)), None, "the person sees their own deck");
        let _ = std::fs::remove_dir_all(&root);
    }
}
