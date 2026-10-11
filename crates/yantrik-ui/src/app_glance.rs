//! What an app that was just opened looks like, in one line, and what it can be asked to do.
//!
//! `open_app` answered a mind with where the window appeared and nothing about the app in it. On
//! VM 520 (3 October) a mind that had read the calendar three times was told by the goal note to
//! write the plan, opened the editor, got back the shell's own "files screen" line, and went back
//! to reading the calendar: nothing in the answer said the editor was empty and that `new` and
//! `save_as` are how to fill and keep it. This adds the app's own summary and its actions, with
//! the arguments each needs, to the answer, so the next call can be the right one.

use std::time::Duration;

/// The most actions listed: enough for an editor's or a calendar's whole surface, short enough to
/// read as one line in a mind's work log.
pub const ACTIONS_SHOWN: usize = 14;

/// A window can be listed a moment before its control socket answers; this is how long to keep
/// asking, in `TRIES` looks.
const TRIES: u32 = 8;
const STEP: Duration = Duration::from_millis(250);
const ASK: Duration = Duration::from_millis(800);

/// `{state, actions}` for the app published as `surface`, or `None` when it does not answer.
pub fn glance(surface: &str) -> Option<serde_json::Value> {
    let dir = yantrik_ipc_transport::server::socket_dir();
    for _ in 0..TRIES {
        if let Some(address) = crate::control_approvals::surface_address(&dir, surface) {
            let reply = yantrik_ipc_transport::SyncRpcClient::new(&address)
                .with_timeout(ASK)
                .call("app.describe", serde_json::json!({}))
                .ok()?;
            return Some(shape(&reply));
        }
        std::thread::sleep(STEP);
    }
    None
}

/// The parts of a `describe` answer a caller needs right after opening the app: its summary, and
/// each action as `name {required, optional?}`.
pub fn shape(reply: &serde_json::Value) -> serde_json::Value {
    let actions: Vec<String> = reply["actions"]
        .as_array()
        .map(|list| list.iter().take(ACTIONS_SHOWN).filter_map(action_line).collect())
        .unwrap_or_default();
    serde_json::json!({
        "state": reply["summary"].as_str().unwrap_or_default(),
        "actions": actions,
    })
}

fn action_line(action: &serde_json::Value) -> Option<String> {
    let name = action["name"].as_str()?;
    let required: Vec<&str> = action["parameters"]["required"]
        .as_array()
        .map(|r| r.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    let mut args: Vec<String> = Vec::new();
    if let Some(props) = action["parameters"]["properties"].as_object() {
        for key in props.keys() {
            args.push(if required.contains(&key.as_str()) { key.clone() } else { format!("{key}?") });
        }
    }
    Some(if args.is_empty() { name.to_string() } else { format!("{name} {{{}}}", args.join(", ")) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_opened_app_says_what_it_shows_and_how_to_drive_it() {
        let reply = json!({
            "summary": "Text Editor — Untitled, empty · tab 1 of 1",
            "actions": [
                {"name": "new", "parameters": {"properties": {"text": {}}, "required": []}},
                {"name": "save_as", "parameters": {"properties": {"path": {}}, "required": ["path"]}},
                {"name": "undo", "parameters": {"properties": {}, "required": []}},
            ],
        });
        let g = shape(&reply);
        assert_eq!(g["state"], "Text Editor — Untitled, empty · tab 1 of 1");
        assert_eq!(g["actions"], json!(["new {text?}", "save_as {path}", "undo"]));
    }

    #[test]
    fn a_long_surface_is_cut_to_what_reads_in_one_line() {
        let actions: Vec<_> = (0..40).map(|i| json!({"name": format!("a{i}"), "parameters": {"properties": {}, "required": []}})).collect();
        let g = shape(&json!({"summary": "x", "actions": actions}));
        assert_eq!(g["actions"].as_array().unwrap().len(), ACTIONS_SHOWN);
    }

    #[test]
    fn an_answer_with_nothing_in_it_is_empty_not_a_panic() {
        let g = shape(&json!({}));
        assert_eq!(g, json!({"state": "", "actions": []}));
    }
}
