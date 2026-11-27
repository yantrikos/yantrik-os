//! The actions every mode asks about, except full bypass (#1171; full bypass asks nothing, #1353).
//!
//! Full bypass is "stop asking me anything", an answer a person gives for an hour. A few actions
//! last longer than the hour or cannot be taken back, and what they do is the person's own
//! decision, not something done on their behalf:
//!
//! - `allow_location_lookup` writes a standing consent that lasts for good, and the address it
//!   sends cannot be recalled;
//! - `allow_background_research` writes another, and the searches it allows go on after the hour;
//! - `set_prepare_for_me` switches on background web searches and drafts that send the person's
//!   calendar notes and agenda titles to their model account;
//! - `report_problem` posts a record as a public issue.
//!
//! The companion's four questions (`delete_for_good`, `send_chat_message`, `plan_work`, `change_item`)
//! are listed too: text the companion has read can tell it to do any of them, so only a card the
//! person pressed counts, in every mode and under every rule.
//!
//! Outward acts are carded by effect (#164): an act that sends the person's data off the machine
//! asks in every mode, with its whole payload shown, unless it is on a named list of acts that
//! send nothing. `send_mail` is listed by name; every browser action but the read-only set
//! ([`BROWSER_SILENT`]) is asked, one added later included. The gate sees the app and the action,
//! not the arguments, so `go` and the keys are judged by name here and ask; the bridge's reading
//! of the arguments (`yos_mcp_web.browser_is_silent`) is the taint's input, not a way past this.
//!
//! So they join what every mode but full bypass asks about (full bypass asks nothing, #1353,
//! decision D, and records what ran). The two decisions that read the table, `gate::decide` (the door every
//! app's dispatch uses) and the shell's `Modes::decide` (what `request_approval` answers), both
//! read this list and treat a listed action as one that cannot be undone: asked in every mode but
//! full bypass, no session or standing rule covers it, plan mode still refuses it.

/// `(app, action)`: the shell's own actions that ask in every mode. A shell test keeps the four
/// questions here equal to `control_questions::QUESTIONS`.
pub const ALWAYS_ASKED: [(&str, &str); 10] = [
    ("shell", "allow_location_lookup"),
    ("shell", "allow_background_research"),
    ("shell", "report_problem"),
    // The companion's four questions (#1313, round 4): the effect is approved, not the route to
    // it, so for these only a card the person pressed counts, in every mode.
    ("shell", "delete_for_good"),
    ("shell", "send_chat_message"),
    ("shell", "plan_work"),
    ("shell", "change_item"),
    // A command that names places of the home the view hides (#1265): the person allows each one,
    // naming the place and the command, in every mode. `agent_run` itself has no such argument.
    ("shell", "agent_run_reach"),
    // Prepare for me lets the OS search the web and send calendar notes and agenda titles to the
    // person's model account on its own (#1416): turning it on is the person's call, on a card.
    ("shell", "set_prepare_for_me"),
    // Mail leaves the machine and cannot be recalled (#164): the card shows the whole mail.
    ("shell", "send_mail"),
];

/// The names the browser surface answers to, as the bridge's `BROWSER_NAMES` and the app's id.
const BROWSER_APPS: [&str; 3] = ["browser", "chromium", "app-browser"];

/// The browser's actions that run without a card: they look, or let go, and send nothing. Every
/// other action asks in every mode, so one added to the browser later asks until it is named here.
/// Kept equal to `yos_mcp_web.BROWSER_SILENT` by a test over the file itself.
pub const BROWSER_SILENT: [&str; 12] = [
    "describe", "read", "find", "text", "console", "screenshot", "tabs", "media", "wait", "scroll",
    "back", "key_up",
];

/// Is this action one that asks in every mode but full bypass, which asks nothing (decision D).
pub fn always_asked(app: &str, action: &str) -> bool {
    ALWAYS_ASKED.contains(&(app, action)) || (BROWSER_APPS.contains(&app) && !BROWSER_SILENT.contains(&action))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gate::{decide, Authority, Mode};

    const SAID: &str = "A sentence that says nothing about permanence.";

    fn authority(mode: &str) -> Authority {
        Authority { ceiling: "dangerous".into(), mode: Mode::named(mode), granted: false, asks_above: None }
    }

    /// Full bypass asks nothing (#1353, decision D): the listed actions run there. Every other mode
    /// asks, and a person's Allow runs them anywhere.
    #[test]
    fn the_listed_actions_ask_in_every_mode_but_full_bypass() {
        for (app, action) in ALWAYS_ASKED {
            assert!(always_asked(app, action));
            assert!(decide(&authority("bypass_all"), app, action, "sensitive", SAID).is_ok(), "{app}.{action}: full bypass asks nothing");
            for mode in ["ask", "auto", "bypass"] {
                let asked = decide(&authority(mode), app, action, "sensitive", SAID);
                assert!(asked.unwrap_err().starts_with("GRANT:"), "{app}.{action} ran in {mode}");
            }
            let granted = Authority { granted: true, ..authority("bypass") };
            assert!(decide(&granted, app, action, "sensitive", SAID).is_ok(), "a person's Allow runs {action}");
        }
        assert!(decide(&authority("bypass_all"), "shell", "set_theme", "sensitive", SAID).is_ok(), "other actions: unchanged");
        assert!(!always_asked("notes", "report_problem") && !always_asked("notes", "send_mail")
            && !always_asked("mail", "send_mail"), "keyed on the shell, where the question is published");
    }

    #[test]
    fn send_mail_asks_in_every_mode_but_full_bypass() {
        assert!(decide(&authority("bypass_all"), "shell", "send_mail", "sensitive", SAID).is_ok());
        for mode in ["ask", "auto", "bypass"] {
            assert!(decide(&authority(mode), "shell", "send_mail", "sensitive", SAID).unwrap_err().starts_with("GRANT:"), "{mode}");
        }
    }

    #[test]
    fn an_unknown_browser_action_asks_outside_full_bypass_and_fails_closed() {
        for name in ["browser", "chromium", "app-browser"] {
            assert!(decide(&authority("bypass_all"), name, "a_future_action", "sensitive", SAID).is_ok(), "{name}: full bypass asks nothing");
            for mode in ["ask", "auto", "bypass"] {
                let asked = decide(&authority(mode), name, "a_future_action", "sensitive", SAID);
                assert!(asked.unwrap_err().starts_with("GRANT:"), "{name} in {mode}: an action nobody named ran");
            }
        }
    }

    #[test]
    fn each_silent_browser_action_runs_unasked_and_every_other_real_one_asks_outside_full_bypass() {
        for action in BROWSER_SILENT {
            assert!(!always_asked("browser", action), "{action} is silent");
            assert!(decide(&authority("bypass_all"), "browser", action, "sensitive", SAID).is_ok(), "{action}");
        }
        for action in ["go", "click", "type", "press", "key_down", "select", "dialog", "drag",
                       "context_click", "commit", "forward", "reload", "switch_tab", "close_tab"] {
            assert!(always_asked("browser", action), "{action} asks");
            assert!(decide(&authority("bypass_all"), "browser", action, "sensitive", SAID).is_ok(), "{action}: full bypass asks nothing");
            for mode in ["ask", "auto", "bypass"] {
                assert!(decide(&authority(mode), "browser", action, "sensitive", SAID).is_err(), "{action} in {mode}");
            }
        }
    }

    #[test]
    fn the_bridges_silent_set_is_this_one() {
        let py = include_str!("../../../deploy/yantrik-os/yos_mcp_web.py");
        let line = py.split("BROWSER_SILENT = {").nth(1).unwrap().split('}').next().unwrap();
        let mut theirs: Vec<&str> = line.split(',').map(|w| w.trim().trim_matches('"')).filter(|w| !w.is_empty()).collect();
        let mut ours = BROWSER_SILENT.to_vec();
        theirs.sort();
        ours.sort();
        assert_eq!(theirs, ours);
    }

    /// The Python gate keeps a copy of the table (`sdk/python/yantrik_surface/always_asked.py`); it must
    /// be this one, item by item (the Python side has the same test against this file).
    #[test]
    fn the_python_gates_table_is_this_one() {
        let py = include_str!("../../../sdk/python/yantrik_surface/always_asked.py");
        let block = |name: &str| -> Vec<String> {
            let body = py.split(&format!("\n{name} = (")).nth(1).unwrap().split("\n)\n").next().unwrap();
            let body = body.split(")\n").next().unwrap_or(body);
            body.split('"').skip(1).step_by(2).map(String::from).collect()
        };
        let pairs: Vec<String> = block("ALWAYS_ASKED");
        let ours: Vec<String> = ALWAYS_ASKED.iter().flat_map(|(a, b)| [a.to_string(), b.to_string()]).collect();
        assert_eq!(pairs, ours);
        assert_eq!(block("BROWSER_APPS"), BROWSER_APPS.map(String::from).to_vec());
        assert_eq!(block("BROWSER_SILENT"), BROWSER_SILENT.map(String::from).to_vec());
    }

    #[test]
    fn no_session_rule_answers_one() {
        let mut bypass = authority("bypass");
        bypass.mode.session_rules.push(("shell".into(), "report_problem".into()));
        bypass.mode.standing_rules.push(("shell".into(), "report_problem".into()));
        assert!(decide(&bypass, "shell", "report_problem", "sensitive", SAID).is_err(), "a rule does not cover it");
    }
}
