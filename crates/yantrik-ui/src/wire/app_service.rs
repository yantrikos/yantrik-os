//! `start_app`: the shell starts an app of the framework on demand, with no window, for an agent
//! (design #978 step 1).
//!
//! `yos` asks it when an agent's act or describe reaches an app whose surface does not answer.
//! The instance is the person's: started here, in the person's session, on their display, with
//! `--service`, so the app loads its store and serves intents and `describe` and shows nothing
//! until the person opens it. What to do and what a finished start means is
//! `yantrik_shell_core::app_service`; this module resolves the app, looks at its socket and runs
//! the process.
//!
//! The answer is a state, not an error, so a caller can tell an app that does not start on demand
//! (`not_on_demand`) from one that is starting or could not start:
//! `{ app, name, state: running | starting | failed | not_on_demand, reason? }`.

use std::sync::Mutex;
use std::time::Instant;

use serde_json::{json, Value};
use yantrik_shell_core::app_service::{answers, service_command, Plan, Starts};
use yantrik_shell_core::framework::{self, FrameworkApp};

use crate::apps::{Catalogue, DesktopEntry};
use yantrik_app_runtime::control::{service_id_for, Action, Param};

static STARTS: Mutex<Option<Starts>> = Mutex::new(None);

fn with_starts<T>(f: impl FnOnce(&mut Starts) -> T) -> T {
    let mut guard = STARTS.lock().unwrap_or_else(|p| p.into_inner());
    f(guard.get_or_insert_with(Starts::default))
}

/// The action and its handler, for the shell's surface.
pub fn action(installed: Catalogue) -> (Action, impl Fn(&Value) -> Result<Value, String> + 'static) {
    let action = Action::new(
        "start_app",
        "Start an app's instance with no window, so its actions and describe answer, if it is not \
         running; the answer says whether it is running, starting or could not start. For an app \
         that does not start on demand it says so. Opens nothing for the person.",
    )
    .risk("safe")
    .arg(Param::text("name").describe("App id, e.g. notes, calendar, image-viewer"));
    let handler = move |args: &Value| {
        let name = args["name"].as_str().unwrap_or_default().trim().to_string();
        if name.is_empty() {
            return Err("`name` is empty".into());
        }
        Ok(start(&name, &installed.get()))
    };
    (action, handler)
}

/// The app of the framework `name` means (its id, an alias, or the name it is called), if any.
fn resolve<'a>(name: &str, installed: &'a [DesktopEntry]) -> Option<(&'static FrameworkApp, Option<&'a DesktopEntry>)> {
    if let Some((_, entry)) = crate::surfaces::find(name, installed) {
        if let Some(app) = framework::by_program(&entry.app_id) {
            return Some((app, Some(entry)));
        }
    }
    let key = name.trim().to_lowercase().replace('_', "-");
    framework::by_id(&key).or_else(|| framework::by_program(&key)).map(|app| (app, None))
}

/// The name a person calls the app by: the shell's own, as every window and refusal says it.
fn display_name(app: &FrameworkApp, entry: Option<&DesktopEntry>) -> String {
    let id = super::app_grid::icon_id_for(entry.map_or(app.program, |e| e.app_id.as_str()));
    crate::windows::app_display_name(&id)
}

/// Whether the app's surface answers: asked without blocking (`yantrik_shell_core::app_service::
/// answers`), because this runs on the UI thread, outside the lock on the starts.
fn answering(id: &str) -> bool {
    answers(std::path::Path::new(&yantrik_ipc_transport::server::RpcServer::default_address(&service_id_for(id))))
}

/// Start the app `name` means if it is not running, and say where it stands.
pub fn start(name: &str, installed: &[DesktopEntry]) -> Value {
    let Some((app, entry)) = resolve(name, installed) else {
        return json!({ "state": "not_on_demand", "reason": format!("`{name}` is not an app that starts on demand") });
    };
    let shown = display_name(app, entry);
    let said = |state: &str, reason: Option<String>| answer(app, &shown, state, reason);
    let up = answering(app.id);
    match with_starts(|starts| starts.plan(app.id, up, Instant::now())) {
        Plan::Running => said("running", None),
        Plan::Starting => said("starting", None),
        Plan::Failed(why) => said("failed", Some(why)),
        Plan::Spawn => match spawn(app) {
            Ok(()) => said("starting", None),
            Err(why) => {
                with_starts(|starts| starts.not_spawned(app.id, &why, Instant::now()));
                said("failed", Some(why))
            }
        },
    }
}

/// The answer: where the app stands, and the sentences a caller says for each case, made once here
/// from the framework's list so `yos` and the bridge repeat them word for word.
fn answer(app: &FrameworkApp, shown: &str, state: &str, reason: Option<String>) -> Value {
    let why = reason.clone().unwrap_or_else(|| "it did not start".to_string());
    let mut answer = json!({
        "app": app.id,
        "name": shown,
        "state": state,
        "say": {
            "act_starting": framework::act_while_starting(shown),
            "act_failed": framework::act_start_failed(shown, &why),
            "describe_starting": framework::describe_while_starting(shown),
            "describe_failed": framework::describe_start_failed(shown, &why),
        },
    });
    if let Some(reason) = reason {
        answer["reason"] = reason.into();
    }
    answer
}

/// Run the instance and watch it, so a start that ends is told to the next ask.
fn spawn(app: &'static FrameworkApp) -> Result<(), String> {
    let path = super::dock::resolve_app_binary(app.program);
    let mut command = service_command(&path);
    for (key, value) in super::dock::session_env() {
        command.env(key, value);
    }
    let mut child = command.spawn().map_err(|e| format!("{} could not be run: {e}", path.display()))?;
    tracing::info!(app = app.id, pid = child.id(), "started on demand, with no window");
    std::thread::spawn(move || {
        let status = child.wait();
        let (success, how) = match &status {
            Ok(status) => (status.success(), format!("it ended with {status}")),
            Err(e) => (false, format!("it could not be waited for: {e}")),
        };
        let up = answering(app.id);
        with_starts(|starts| starts.exited(app.id, success, up, &how, Instant::now()));
        tracing::info!(app = app.id, success, up, "an on-demand start ended: {how}");
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(program: &str, name: &str, surface: &str) -> DesktopEntry {
        DesktopEntry {
            app_id: program.into(),
            exec: format!("/opt/yantrik/bin/{program}"),
            name: name.into(),
            surface: Some(surface.into()),
            ..Default::default()
        }
    }

    /// Every app of the framework is found by the id it publishes, and one that is not is told so.
    #[test]
    fn every_framework_app_is_found_by_its_id_and_nothing_else_is() {
        for app in framework::FRAMEWORK_APPS {
            let (found, _) = resolve(app.id, &[]).expect(app.id);
            assert_eq!(found.id, app.id);
        }
        assert!(resolve("terminal", &[]).is_none());
        assert!(resolve("files", &[]).is_none());
        let said = start("terminal", &[]);
        assert_eq!(said["state"], "not_on_demand");
    }

    #[test]
    fn the_app_is_called_what_the_shell_calls_it() {
        let installed = vec![entry("yantrik-image-viewer", "Images", "image-viewer")];
        let (app, found) = resolve("image-viewer", &installed).unwrap();
        assert_eq!(display_name(app, found), "Images");
        let (app, found) = resolve("notes", &[]).unwrap();
        assert_eq!(display_name(app, found), "Notes");
    }
}

#[cfg(test)]
mod answer_tests {
    use super::*;

    /// The sentences `yos` repeats are these, word for word: the selftest of `yos` pins the same.
    #[test]
    fn the_answer_carries_the_sentences_a_caller_repeats() {
        let notes = framework::by_id("notes").unwrap();
        let said = answer(notes, "Notes", "failed", Some("it ended with exit status: 1".into()));
        assert_eq!(said["state"], "failed");
        assert_eq!(said["say"]["act_starting"], "Notes is starting; nothing was done \u{2014} send it again in a moment.");
        assert_eq!(said["say"]["act_failed"], "Notes could not be started (it ended with exit status: 1); nothing was done.");
        assert_eq!(said["say"]["describe_starting"], "Notes is starting; ask again in a moment.");
        assert_eq!(
            said["say"]["describe_failed"],
            "Notes is not running: the desktop could not start it (it ended with exit status: 1)."
        );
    }
}
