//! What `app_action` may do with nobody there: nothing an app says it does with the internet or a command line.
//!
//! `reach` classes a call by its tool name, and `app_action` is one name for every app's every action,
//! so what it reaches is in the action. The apps already say it: an action that takes an address
//! declares `.fetches_address(param)` (the download manager's `add`), one that runs a line it is
//! given declares `.open_ended()` (the terminal's `run` and `send_input`), and both sentences end up
//! in the description `describe` publishes. An unattended call to such an action is refused with the
//! words every other unattended reach is refused with, and the person's own turn is not held back.
//!
//! When `describe` fails there is no declaration to read, so an unattended call is refused unless the
//! app is one of `reach::LOCAL_APPS`, which have no such action (a source test in `tests/` holds the
//! list to that).

use serde_json::Value;
use yantrik_companion_core::reach;
use yantrik_ipc_transport::gate;

/// One action as the app publishes it in `describe`, if it publishes one by that name.
pub fn published<'a>(view: &'a Value, action: &str) -> Option<&'a Value> {
    view.get("actions")
        .and_then(Value::as_array)
        .and_then(|actions| actions.iter().find(|a| a.get("name").and_then(Value::as_str) == Some(action)))
}

/// The app's own sentence for the action, which is what the card says if one has to go up.
pub fn purpose_of(view: &Value, action: &str) -> String {
    published(view, action).and_then(|a| a.get("description").and_then(Value::as_str)).unwrap_or_default().to_string()
}

/// What to tell the model when this unattended call is not allowed, or `None` to go on. `view` is the
/// app's `describe`, or `None` when it did not answer.
pub fn refused_unattended(app: &str, action: &str, view: Option<&Value>) -> Option<String> {
    let reaches_out = match view {
        Some(view) => {
            let purpose = purpose_of(view, action);
            gate::fetches_address(&purpose).is_some() || gate::open_ended(&purpose)
        }
        None => !reach::LOCAL_APPS.contains(&reach::app_id(app).as_str()),
    };
    if reaches_out {
        reach::check_declared("app_action").err()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use yantrik_companion_core::background_research;
    use yantrik_ipc_contracts::control_surface::Action;

    fn view(actions: &[Action]) -> Value {
        json!({ "actions": actions.iter().map(|a| json!({ "name": a.name, "description": a.description })).collect::<Vec<_>>() })
    }

    #[test]
    fn an_action_that_declares_an_address_or_a_command_line_is_refused_unattended() {
        let v = view(&[
            Action::new("add", "Download a URL to this machine").fetches_address("url").arg(yantrik_ipc_contracts::control_surface::Param::text("url")),
            Action::new("run", "Type a command line into the shell").open_ended(),
            Action::new("rename", "Rename the open note"),
        ]);
        for action in ["add", "run"] {
            let said = refused_unattended("any-app", action, Some(&v)).expect(action);
            assert!(said.starts_with("Refused:") && said.contains("app_action"), "{said}");
        }
        assert_eq!(refused_unattended("any-app", "rename", Some(&v)), None);
        assert_eq!(refused_unattended("any-app", "not-published", Some(&v)), None, "the app refuses what it does not publish");
    }

    #[test]
    fn a_person_is_not_held_back() {
        let v = view(&[Action::new("run", "Type a command line into the shell").open_ended()]);
        let _mine = background_research::person_turn();
        assert_eq!(refused_unattended("terminal", "run", Some(&v)), None);
        assert_eq!(refused_unattended("terminal", "run", None), None);
    }

    #[test]
    fn a_describe_that_failed_refuses_unless_the_app_is_known_to_be_local() {
        assert!(refused_unattended("download-manager", "add", None).is_some());
        assert!(refused_unattended("some-new-app", "anything", None).is_some(), "an unknown app fails closed");
        for app in reach::LOCAL_APPS {
            assert_eq!(refused_unattended(app, "anything", None), None, "{app}");
        }
        assert_eq!(refused_unattended("App-Notes", "anything", None), None);
    }
}
