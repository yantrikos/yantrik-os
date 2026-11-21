//! `agent_run_reach`: one command with named places of the home visible to it (#1265).
//!
//! A command a Mind runs sees the person's visible folders and a named toolchain list; its settings,
//! keys and app data are not in the view. When a command needs one (`git push` over SSH needs
//! `~/.ssh`), the Mind asks with this action, which is `agent_run` plus a required `reach`. It is a
//! separate action, as `agent_run_sealed` is, because the gate grades and asks per action, not per
//! argument: `agent_run_reach` is on `ALWAYS_ASKED` and declares `always_asks`, so the person allows
//! each call on a card, in every mode, and no session rule or standing rule answers for it. The card
//! names the places and the command. Allow binds those places read-only into that one job's view.

use std::path::PathBuf;

use serde_json::Value;
use yantrik_app_runtime::control::{Action, App as ControlSurface, Param};
use yantrik_ipc_contracts::home_paths::outside_view;

use crate::control_agent_terminal::{run_job, text, wait_arg, Call, Prepared};
use crate::workshop;

pub(crate) fn spec() -> Action {
    Action::new(
        "agent_run_reach",
        "Run one command line like `agent_run`, with named places of the person's home that a command \
         does not see (settings, keys, app data: `~/.ssh` for `git push`, `~/.config/gh` for `gh`) \
         visible to it, read-only, for this one command. Name the fewest places you need, at most 4, \
         each written `~/…` or absolute. A place that holds other protected places (`~/.config`), the \
         home itself or a place that is not there is refused and nothing runs. Same answer as \
         `agent_run`. Acts for the agent named by the agent token your call carries — never an argument.",
    )
    .risk("sensitive")
    // The person allows each one on a card, in every mode, and no session rule covers it.
    .always_asks()
    .defers()
    .arg(Param::text("command").describe("One command line, as it would be typed. It runs under bash"))
    .arg(Param::array("reach", "string").describe("The places to make visible, read-only: [\"~/.ssh\"]"))
    .arg(Param::text("cwd").optional().describe("Where to run it, absolute or relative to your current directory. Left out: where your last command ended"))
    .arg(Param::number("wait").optional().describe(
        "A number of seconds, e.g. 30. How long to wait before answering `running: true`. Default 120, at most 600",
    ))
    .explain(explain)
}

/// The places as written: a list, or one text for a Mind that sends a single place bare.
fn written(args: &Value) -> Vec<String> {
    match args.get("reach") {
        Some(Value::Array(items)) => items.iter().filter_map(Value::as_str).map(String::from).collect(),
        Some(Value::String(one)) => vec![one.clone()],
        _ => Vec::new(),
    }
}

fn explain(args: &Value) -> String {
    outside_view::card_text(&text(args, "command"), &written(args))
}

fn agent_run_reach(args: &Value) -> Result<Value, String> {
    let command = text(args, "command");
    // Refused here, before any job exists: an unknown or too-wide place runs nothing.
    let reach = outside_view::reach_list(&workshop::home(), &written(args))?;
    let cwd = args.get("cwd").and_then(Value::as_str).map(str::trim).filter(|c| !c.is_empty()).map(PathBuf::from);
    let wait = wait_arg(args)?;
    run_job(command, wait, Call::current(), Box::new(move |_| Ok(Prepared { cwd, sealed: None, reach })))
}

pub fn actions(surface: ControlSurface) -> ControlSurface {
    surface.action(spec(), agent_run_reach)
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use yantrik_ipc_transport::always_asked::always_asked;
    use yantrik_ipc_transport::gate::{always_asks, decide, Authority, Mode};

    fn authority(mode: &str) -> Authority {
        Authority { ceiling: "dangerous".into(), mode: Mode::named(mode), granted: false, asks_above: None }
    }

    /// A call that names `reach` asks in every mode, full bypass included, and a rule does not answer it; a
    /// person's Allow does. `agent_run` itself has no `reach`.
    #[test]
    fn a_call_with_reach_asks_in_full_bypass_and_no_rule_answers_it() {
        let s = spec();
        assert!(always_asked("shell", "agent_run_reach") && always_asks(&s.description));
        assert_eq!(s.permission, "sensitive");
        for mode in ["ask", "auto", "bypass", "bypass_all"] {
            let said = decide(&authority(mode), "shell", "agent_run_reach", "sensitive", &s.description);
            assert!(said.unwrap_err().starts_with("GRANT:"), "{mode} ran it unasked");
        }
        let mut full = authority("bypass_all");
        full.mode.session_rules.push(("shell".into(), "agent_run_reach".into()));
        full.mode.standing_rules.push(("shell".into(), "agent_run_reach".into()));
        assert!(decide(&full, "shell", "agent_run_reach", "sensitive", &s.description).is_err(), "a rule does not cover it");
        let granted = Authority { granted: true, ..authority("bypass_all") };
        assert!(decide(&granted, "shell", "agent_run_reach", "sensitive", &s.description).is_ok(), "Allow runs it");
        assert!(crate::control_agent_terminal::specs().iter().all(|a| !a.params.iter().any(|p| p.name == "reach")), "agent_run has no reach");
    }

    #[test]
    fn the_card_names_the_places_and_the_command() {
        let said = explain(&serde_json::json!({"command": "git push origin main", "reach": ["~/.ssh"]}));
        assert!(said.contains("`git push origin main`") && said.contains("`~/.ssh`") && said.contains("Only this command"), "{said}");
        let one = explain(&serde_json::json!({"command": "ls", "reach": "~/.ssh"}));
        assert!(one.contains("`~/.ssh`"), "{one}");
    }

    #[test]
    fn an_unknown_or_malformed_place_is_refused_before_anything_runs() {
        let home = std::env::temp_dir().join(format!("yantrik-reach-{}", std::process::id()));
        std::fs::create_dir_all(home.join(".ssh")).unwrap();
        std::fs::create_dir_all(home.join(".config/gh")).unwrap();
        let home = home.canonicalize().unwrap();
        let ask = |w: &[&str]| outside_view::reach_list(&home, &w.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert!(ask(&["~/.ssh"]).is_ok());
        for bad in [&["~/.nothere"][..], &["~"], &["~/../x"], &["relative/x"], &["~/.config"], &["/etc"], &[]] {
            assert!(ask(bad).is_err(), "{bad:?}");
        }
        let _ = std::fs::remove_dir_all(&home);
    }
}
