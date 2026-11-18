//! What the Mind, keys and menus may ask Containers by name: the same seven actions, graded as
//! they were, each the same command a button runs.
//!
//! Stopping and restarting are `sensitive`: recoverable, but they interrupt whatever the
//! container was serving. Removing is `dangerous`: no undo, and the container's writable layer
//! goes with it. Every action that runs the runtime is answered when its job has landed and the
//! list it changed has been re-read, never from the exit status alone.

use std::time::Duration;

use serde_json::json;
use yantrik_app::control::{Action, Param};
use yantrik_app::intents::{Args, Intent, Intents, Settle};
use yantrik_app::Refused;

use crate::model::{Command, Model, Verb, TABS};
use crate::Containers;

/// How long the Mind waits for the runtime: a `docker stop` alone may take ten seconds before it
/// kills, and the answer waits for the listing after it. The framework's own ceiling.
const RUNTIME_WITHIN: Duration = yantrik_app::intents::MAX_SETTLE_WAIT;

pub fn table(table: &mut Intents<Containers>) {
    table.add(
        Intent::new(Action::new("refresh", "Re-read containers, images and volumes"), |_| {
            Ok(Command::Refresh)
        })
        .settles(Settle::Explicit { within: RUNTIME_WITHIN })
        .reply(|m: &Model| {
            json!({
                "running": m.running(),
                "total": m.total(),
                "runtime_available": m.health.is_ready(),
                "runtime_state": m.health.state_name(),
            })
        }),
    );
    table.add(
        Intent::new(
            Action::new("show_tab", "Switch the window to the containers, images or volumes list")
                .arg(Param::text("tab").describe("containers | images | volumes")),
            // What the tab bar does when it is clicked: the screen draws the list for
            // `active-tab`, and `ct-tab` tells the model.
            |a| tab_index(raw(a, "tab")).map(Command::ShowTab),
        )
        .reply(|m: &Model| json!({ "tab": TABS[m.tab] })),
    );
    named(table, Action::new("start", "Start a stopped container"), Verb::Start);
    named(
        table,
        Action::new("stop", "Stop a running container").risk("sensitive"),
        Verb::Stop,
    );
    named(
        table,
        Action::new("restart", "Restart a container").risk("sensitive"),
        Verb::Restart,
    );
    named(
        table,
        Action::new("show_logs", "Open the log pane for a container, and read the tail"),
        Verb::Logs,
    );
    named(
        table,
        Action::new("remove", "Delete a container and its writable layer. It cannot be undone.")
            .risk("dangerous"),
        Verb::Remove,
    );
}

/// An action on one container, named by `container`: refused when no container here has that
/// name or id, answered with what the re-read list shows once the command has landed.
fn named(table: &mut Intents<Containers>, action: Action, verb: Verb) {
    table.add(
        Intent::new(action.arg(Param::text("container")), move |a| {
            Ok(Command::Named { verb, needle: raw(a, "container").to_string() })
        })
        .settles(Settle::Explicit { within: RUNTIME_WITHIN })
        .reply(|m: &Model| m.answer.clone()),
    );
}

/// A text argument exactly as it came, as the actions always read it: a blank one is answered
/// in the action's own words ("no container here is called …"), not refused as missing.
fn raw<'a>(a: &Args<'a>, name: &str) -> &'a str {
    a.raw().get(name).and_then(serde_json::Value::as_str).unwrap_or_default()
}

/// The tab a caller named, as the screen's `active-tab` index.
pub fn tab_index(name: &str) -> Result<usize, Refused> {
    let name = name.trim().to_lowercase();
    TABS.iter().position(|t| *t == name).ok_or_else(|| {
        Refused::new("no_tab", format!("no `{name}` tab; use containers, images or volumes"))
    })
}

#[cfg(test)]
mod grade_tests {
    //! The grades, read from this file's text: a test that would otherwise need the registry.

    use super::*;

    fn declaration(name: &str) -> String {
        let src = include_str!("intents.rs");
        let src = src.split("#[cfg(test)]").next().unwrap_or_default();
        let quoted = format!("Action::new(\"{name}\"");
        let from = src
            .find(&quoted)
            .unwrap_or_else(|| panic!("containers no longer publishes {name}"));
        let rest = &src[from..];
        let end = rest.find("Verb::").unwrap_or(rest.len());
        rest[..end].to_string()
    }

    #[test]
    fn what_cannot_be_taken_back_is_graded_for_it() {
        assert!(declaration("remove").contains(".risk(\"dangerous\")"));
        for name in ["stop", "restart"] {
            assert!(declaration(name).contains(".risk(\"sensitive\")"), "{name}");
        }
        // Starting is recoverable; showing and reading change nothing on the machine.
        for name in ["start", "show_logs"] {
            assert!(!declaration(name).contains(".risk("), "{name}");
        }
    }

    #[test]
    fn a_tab_is_named_the_way_describe_reports_it() {
        assert_eq!(tab_index("containers"), Ok(0));
        assert_eq!(tab_index(" Images "), Ok(1));
        assert_eq!(tab_index("volumes"), Ok(2));
        assert_eq!(
            tab_index("networks").unwrap_err().sentence,
            "no `networks` tab; use containers, images or volumes"
        );
        assert!(tab_index("").is_err());
    }
}
