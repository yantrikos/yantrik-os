//! Which files an agent may name to Studio (#443).
//!
//! Studio runs as the person. `upscale` copied any picture the person can read into the gallery,
//! where every image tool reads it - the screenshots in `~/Pictures/Screenshots` included -
//! `open` handed any picture to the machine's viewer, `variations` read the record beside any
//! picture, and `set_backend` read any file as a workflow (security review of #942, finding 1).
//! An agent now names only files in the person's home outside its protected places, by the rule
//! every side shares (`yantrik_ipc_contracts::home_paths`). A bare filename names a picture in the
//! gallery, which is Studio's own, and is resolved there as before. The person is not asked.
//!
//! The rule sits at one door, [`admit`], which every action is published through.

use std::path::PathBuf;

use serde_json::Value;
use yantrik_app_runtime::control::agent_is_calling;
use yantrik_ipc_contracts::home_paths;

/// The argument of each action that names a file, if it takes one. `delete` is not here: it
/// moves only Studio's own pictures, inside the gallery, whoever asks.
fn named_file(action: &str) -> Option<&'static str> {
    match action {
        "upscale" | "open" => Some("path"),
        "variations" => Some("of"),
        "set_backend" => Some("workflow"),
        _ => None,
    }
}

/// Whether this caller may run `action` with `args`.
pub fn admit(action: &str, args: &Value) -> Result<(), String> {
    if !agent_is_calling() {
        return Ok(());
    }
    let Some(key) = named_file(action) else { return Ok(()) };
    let named = args.get(key).and_then(Value::as_str).unwrap_or_default().trim();
    // Empty is each action's own refusal; a bare name is a picture in the gallery.
    if named.is_empty() || !(named.contains('/') || named.starts_with('~')) {
        return Ok(());
    }
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    home_paths::may_read_file(named, &home)
        .map_err(|why| format!("an agent names only a file in the person's home, outside its protected places: {why}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use yantrik_app_runtime::control::AgentTokenScope;

    #[test]
    fn an_agent_names_only_files_in_the_home() {
        let outside = std::env::temp_dir().join(format!("studio-agent-rule-{}.png", std::process::id()));
        std::fs::write(&outside, "SENTINEL").unwrap();
        let outside = outside.display().to_string();
        assert_eq!(admit("upscale", &json!({ "path": outside })), Ok(()), "the person is not asked");

        let agent = AgentTokenScope::enter(Some("tok-studio-test".into()));
        for (action, key) in [("upscale", "path"), ("open", "path"), ("variations", "of"), ("set_backend", "workflow")] {
            for asked in [
                outside.as_str(),
                "~/Pictures/Screenshots/shot.png",
                "~/.local/share/yantrik/snippets/snippets.json",
                "/etc/hostname",
            ] {
                let e = admit(action, &json!({ key: asked })).expect_err(asked);
                assert!(e.starts_with("an agent names only a file in the person's home"), "{action} {asked}: {e}");
            }
            // A bare name is a picture in the gallery, resolved there.
            assert_eq!(admit(action, &json!({ key: "studio-1.png" })), Ok(()), "{action}");
        }
        assert_eq!(admit("delete", &json!({ "path": outside })), Ok(()), "delete keeps to the gallery itself");
        assert_eq!(admit("set_backend", &json!({ "kind": "fake" })), Ok(()), "no workflow named");
        drop(agent);
        let _ = std::fs::remove_file(&outside);
    }
}
