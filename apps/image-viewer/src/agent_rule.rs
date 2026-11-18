//! Which pictures an agent may have the viewer open (#443).
//!
//! `open` shows the picture and reads the folder around it, and `describe` hands back the
//! file's name, what its metadata says, and the name of every picture beside it. The viewer runs
//! as the person, so for an agent that was a listing of any folder the person can read. An agent
//! now names only files in the person's home outside its protected places, by the rule every
//! side shares (`yantrik_ipc_contracts::home_paths`). The person is not asked.
//!
//! The rule reads the disk, so it is asked off the UI thread: once in an agent's `open`, before
//! anything else, and once for every picture loaded, kept beside it in the model as its
//! [`Verdict`](crate::model::Verdict). What an agent is shown is decided from that verdict;
//! while a picture is still loading there is none yet, and an agent is shown nothing of it.

use std::path::{Path, PathBuf};

use yantrik_app::Refused;
use yantrik_ipc_contracts::home_paths;

use crate::model::{Answer, Model};

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

/// Whether an agent may be shown the picture at `path` and the folder it is in. Reads the disk:
/// from a job only.
pub fn verdict(path: &Path) -> Result<(), String> {
    home_paths::may_read_file(&path.to_string_lossy(), &home())
}

/// Refuse an agent's `open` of anything but a file in the home, outside its protected places.
/// Asked before the viewer looks at the path at all, so a refusal says nothing about whether a
/// file is there. From a job only.
pub fn may_open(path: &Path) -> Result<(), String> {
    verdict(path)
}

/// Why an agent is told nothing while the picture on screen is still being read.
pub const STILL_OPENING: &str = "the picture on screen is still being opened; ask again in a moment";

/// Why an agent may not step through the folder on screen.
pub const NOT_STEPPED: &str =
    "the picture on screen is one an agent is not shown; `open` one in the person's home first";

/// Why the picture on screen, and the folder around it, are kept from an agent, if they are:
/// one the person opened from a place an agent may not read, or one not read yet.
pub fn hidden(m: &Model) -> Option<String> {
    let current = m.gallery.current()?;
    match &m.verdict {
        Some(v) if &v.path == current => v.shown_to_agents.clone().err(),
        _ => Some(STILL_OPENING.into()),
    }
}

/// Refuse an agent stepping to the next or previous picture of a folder it may not see: each
/// step would name one more picture in it.
pub fn may_step(m: &Model) -> Result<(), Refused> {
    match hidden(m) {
        None => Ok(()),
        Some(why) if why == STILL_OPENING => Err(Refused::new("still_opening", STILL_OPENING)),
        Some(_) => Err(Refused::new("not_shown", NOT_STEPPED)),
    }
}

/// What `show`, `next` and `previous` answer: the picture on screen by name and place, unless it
/// is one this caller is not shown — checked after the move too, since the folder's next
/// picture may be a link or a second name for a file elsewhere.
pub fn answer(m: &Model, agent: bool, position: Option<usize>) -> Answer {
    Answer {
        showing: m.shown.name.clone(),
        position,
        hidden: agent && hidden(m).is_some(),
    }
}

/// An [`Answer`] as the call's reply.
pub fn answer_json(a: &Answer) -> serde_json::Value {
    if a.hidden {
        return serde_json::json!({ "showing": "(hidden)", "hidden": "the picture is one an agent is not shown" });
    }
    match a.position {
        Some(position) => serde_json::json!({ "showing": a.showing, "position": position }),
        None => serde_json::json!({ "showing": a.showing }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_agent_opens_pictures_only_in_the_home() {
        for (asked, why) in [("/etc/hostname", " is outside"), ("~/.ssh/id_ed25519", " is protected")] {
            let path = if let Some(rest) = asked.strip_prefix("~/") { home().join(rest) } else { PathBuf::from(asked) };
            let err = may_open(&path).unwrap_err();
            assert!(err.ends_with(why), "{asked}: {err}");
            assert!(verdict(&path).is_err_and(|e| e.ends_with(why)), "{asked}");
        }
    }

    #[test]
    fn a_hidden_answer_names_nothing() {
        let said = answer_json(&Answer { showing: "secret-plans.png".into(), position: Some(2), hidden: true });
        assert_eq!(said["showing"], "(hidden)", "{said}");
        assert!(said.get("position").is_none(), "{said}");
        let said = answer_json(&Answer { showing: "b.png".into(), position: Some(2), hidden: false });
        assert_eq!(said, serde_json::json!({ "showing": "b.png", "position": 2 }));
        let said = answer_json(&Answer { showing: "b.png".into(), position: None, hidden: false });
        assert_eq!(said, serde_json::json!({ "showing": "b.png" }));
    }
}
