//! Which pictures an agent may have the viewer open (#443).
//!
//! `open` shows the picture and reads the folder around it, and `describe` hands back the
//! file's name, what its metadata says, and the name of every picture beside it. The viewer runs
//! as the person, so for an agent that was a listing of any folder the person can read. An agent
//! now names only files in the person's home outside its protected places, by the rule every
//! side shares (`yantrik_ipc_contracts::home_paths`). The person is not asked.

use std::path::{Path, PathBuf};

use yantrik_app_runtime::control::agent_is_calling;
use yantrik_ipc_contracts::home_paths;

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

/// Refuse an agent's `open` of anything but a file in the home, outside its protected places.
/// Asked before the viewer looks at the path at all, so a refusal says nothing about whether a
/// file is there.
pub fn may_open(path: &Path) -> Result<(), String> {
    if !agent_is_calling() {
        return Ok(());
    }
    home_paths::may_read_file(&path.to_string_lossy(), &home())
}

/// Why the picture on screen, and the folder around it, are kept out of what this caller is
/// shown, if they are: one the person opened from a place an agent may not read.
pub fn hidden_from_caller(path: Option<&Path>) -> Option<String> {
    let path = path?;
    if !agent_is_calling() {
        return None;
    }
    home_paths::may_read_file(&path.to_string_lossy(), &home()).err()
}

/// Refuse an agent stepping to the next or previous picture of a folder it may not see: each
/// step would name one more picture in it.
pub fn may_step(current: Option<&Path>) -> Result<(), String> {
    match hidden_from_caller(current) {
        Some(_) => Err("the picture on screen is one an agent is not shown; `open` one in the person's home first".to_string()),
        None => Ok(()),
    }
}

/// What `show`, `next` and `previous` answer: the picture on screen by name and place, unless it
/// is one this caller is not shown - checked after the move too, since the folder's next
/// picture may be a link or a second name for a file elsewhere.
pub fn answer(current: Option<&Path>, name: String, position: Option<usize>) -> serde_json::Value {
    if hidden_from_caller(current).is_some() {
        return serde_json::json!({ "showing": "(hidden)", "hidden": "the picture is one an agent is not shown" });
    }
    match position {
        Some(position) => serde_json::json!({ "showing": name, "position": position }),
        None => serde_json::json!({ "showing": name }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yantrik_app_runtime::control::AgentTokenScope;

    #[test]
    fn an_agent_opens_pictures_only_in_the_home() {
        let _agent = AgentTokenScope::enter(Some("tok-viewer-test".into()));
        for (asked, why) in [("/etc/hostname", " is outside"), ("~/.ssh/id_ed25519", " is protected")] {
            let path = if let Some(rest) = asked.strip_prefix("~/") { home().join(rest) } else { PathBuf::from(asked) };
            let err = may_open(&path).unwrap_err();
            assert!(err.ends_with(why), "{asked}: {err}");
            assert!(hidden_from_caller(Some(&path)).is_some_and(|e| e.ends_with(why)), "{asked}");
        }
        assert_eq!(hidden_from_caller(None), None, "nothing on screen hides nothing");
    }

    #[test]
    fn the_person_opens_pictures_anywhere() {
        assert!(may_open(Path::new("/etc/hostname")).is_ok());
        assert_eq!(hidden_from_caller(Some(Path::new("/etc/hostname"))), None);
    }
}

#[cfg(test)]
mod step_tests {
    use super::*;
    use yantrik_app_runtime::control::AgentTokenScope;
    use yantrik_image_core::Gallery;

    #[test]
    fn next_in_a_folder_an_agent_may_not_see_names_nothing() {
        // The person opened a picture in the temp directory, outside the home.
        let dir = std::env::temp_dir().join(format!("yantrik-viewer-step-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["a.png", "b.png", "secret-plans.png"] {
            std::fs::write(dir.join(name), "x").unwrap();
        }
        let mut g = Gallery::open(&dir.join("a.png"));
        assert!(may_step(g.current().map(|p| p.as_path())).is_ok(), "the person steps anywhere");

        let _agent = AgentTokenScope::enter(Some("tok-viewer-step".into()));
        assert!(may_step(g.current().map(|p| p.as_path())).is_err());
        g.next();
        let said = answer(g.current().map(|p| p.as_path()), "b.png".into(), Some(2));
        assert_eq!(said["showing"], "(hidden)", "{said}");
        assert!(said.get("position").is_none(), "{said}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
