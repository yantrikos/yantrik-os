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

pub(crate) fn spec() -> Action {
    Action::new(
        "agent_run_reach",
        "Run one command line like `agent_run`, with named places of the person's home that a command \
         does not see (settings, keys, app data: `~/.ssh` for `git push`, `~/.config/gh` for `gh`) \
         visible to it, read-only, for this one command. Name the fewest places you need, at most 4, \
         each written `~/…` or absolute. A place that holds other protected places (`~/.config`), the \
         home itself, a place inside a visible folder (it is already visible) or a place that is not there is \
         refused and nothing runs. Same answer as \
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

/// Text of the call as the card shows it: control and format characters (direction overrides,
/// zero-width) left out, the way `explain_export` does, and a cut marked, never silent.
fn clean(s: &str, most: usize) -> String {
    let kept: Vec<char> = s.trim().chars().filter(|c| !c.is_control() && !crate::approvals::is_format_char(*c)).collect();
    let cut = kept.len() > most;
    let mut shown: String = kept.into_iter().take(most).collect();
    if cut {
        shown.push_str("…(cut)");
    }
    shown
}

/// The card: the command, and each place **as it resolves** (the path that will be bound), so the
/// person is not shown a name for another place. A place that cannot be bound says it will be refused.
fn explain_in(home: &std::path::Path, args: &Value) -> String {
    let (mut places, mut refused) = (Vec::new(), Vec::new());
    for w in written(args).iter().take(outside_view::MAX_REACH) {
        match outside_view::reach_place(home, &outside_view::expand(home, w)) {
            Ok(real) => places.push(format!("`{}`", clean(&real.to_string_lossy(), 240))),
            Err(why) => {
                places.push(format!("`{}`", clean(w, 120)));
                refused.push(clean(&why, 300));
            }
        }
    }
    let mut said = format!(
        "A Mind wants to run `{}` with {} visible to it, read-only. Only this command. It can copy or send what it reads.",
        clean(&text(args, "command"), 300),
        places.join(", ")
    );
    if let Some(why) = refused.first() {
        said.push_str(&format!(" This will be refused: {why}"));
    }
    said
}

fn explain(args: &Value) -> String {
    match outside_view::account_home() {
        Ok(home) => explain_in(&home, args),
        Err(why) => format!("This will be refused: {why}."),
    }
}

fn agent_run_reach(args: &Value) -> Result<Value, String> {
    let command = text(args, "command");
    // Refused here, before any job exists: an unknown, linked or too-wide place runs nothing. The
    // list holds the resolved paths, the ones the card named, and the bind checks them again.
    let home = outside_view::account_home().map_err(|e| format!("Error: {e}. Nothing was run."))?;
    let reach = outside_view::reach_list(&home, &written(args))?;
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

    fn home_with(dirs: &[&str]) -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let top = std::env::temp_dir().join(format!("yantrik-card-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        for d in dirs {
            std::fs::create_dir_all(top.join("home").join(d)).unwrap();
        }
        top.join("home").canonicalize().unwrap()
    }

    #[test]
    fn the_card_names_the_resolved_place_the_command_and_what_it_can_do() {
        let home = home_with(&[".ssh"]);
        let said = explain_in(&home, &serde_json::json!({"command": "git push origin main", "reach": ["~/.ssh"]}));
        let ssh = home.join(".ssh").display().to_string();
        assert!(said.contains("`git push origin main`") && said.contains(&format!("`{ssh}`")) && said.contains("Only this command"), "{said}");
        assert!(said.contains("It can copy or send what it reads."), "{said}");
        assert!(!said.contains("refused"), "{said}");
        let one = explain_in(&home, &serde_json::json!({"command": "ls", "reach": "~/.ssh"}));
        assert!(one.contains(&ssh), "{one}");
        let _ = std::fs::remove_dir_all(home.parent().unwrap());
    }

    /// A link is not named as the place it leads to: the card says so and shows the real place.
    #[test]
    fn a_card_for_a_link_says_it_will_be_refused_and_names_where_it_leads() {
        let home = home_with(&[".ssh", "Documents"]);
        std::os::unix::fs::symlink(home.join(".ssh"), home.join("Documents/harmless")).unwrap();
        let link = home.join("Documents/harmless");
        let said = explain_in(&home, &serde_json::json!({"command": "ls", "reach": [link.display().to_string()]}));
        assert!(said.contains("This will be refused") && said.contains("link") && said.contains(&home.join(".ssh").display().to_string()), "{said}");
        let _ = std::fs::remove_dir_all(home.parent().unwrap());
    }

    #[test]
    fn the_card_drops_format_characters_and_marks_a_cut() {
        let home = home_with(&[".ssh"]);
        let said = explain_in(&home, &serde_json::json!({"command": format!("ls\u{202e}\u{200b}{}", "x".repeat(400)), "reach": ["~/.ssh"]}));
        assert!(!said.contains('\u{202e}') && !said.contains('\u{200b}'), "{said:?}");
        assert!(said.contains("…(cut)"), "{said}");
        let _ = std::fs::remove_dir_all(home.parent().unwrap());
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
