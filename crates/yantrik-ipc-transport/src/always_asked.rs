//! The actions every mode asks about, full bypass included (#1171).
//!
//! Full bypass is "stop asking me anything", an answer a person gives for an hour. A few actions
//! last longer than the hour or cannot be taken back, and what they do is the person's own
//! decision, not something done on their behalf:
//!
//! - `allow_location_lookup` writes a standing consent that lasts for good, and the address it
//!   sends cannot be recalled;
//! - `allow_background_research` writes another, and the searches it allows go on after the hour;
//! - `report_problem` posts a record as a public issue.
//!
//! So they join what full bypass still asks about (a connected mind's command after a private
//! read, and so on). The two decisions that read the table, `gate::decide` (the door every
//! app's dispatch uses) and the shell's `Modes::decide` (what `request_approval` answers), both
//! read this list and treat a listed action as one that cannot be undone: asked in every mode, no
//! session or standing rule covers it, plan mode still refuses it.

/// `(app, action)`: the shell's own actions that ask in every mode.
pub const ALWAYS_ASKED: [(&str, &str); 3] = [
    ("shell", "allow_location_lookup"),
    ("shell", "allow_background_research"),
    ("shell", "report_problem"),
];

/// Does this action ask even in full bypass.
pub fn always_asked(app: &str, action: &str) -> bool {
    ALWAYS_ASKED.contains(&(app, action))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gate::{decide, Authority, Mode};

    const SAID: &str = "A sentence that says nothing about permanence.";

    fn authority(mode: &str) -> Authority {
        Authority { ceiling: "dangerous".into(), mode: Mode::named(mode), granted: false, asks_above: None }
    }

    #[test]
    fn the_three_actions_ask_in_full_bypass_and_everything_else_runs() {
        for (app, action) in ALWAYS_ASKED {
            assert!(always_asked(app, action));
            let asked = decide(&authority("bypass_all"), app, action, "sensitive", SAID);
            assert!(asked.unwrap_err().starts_with("GRANT:"), "{app}.{action} ran in full bypass");
            assert!(decide(&authority("bypass"), app, action, "sensitive", SAID).is_err(), "{action} in bypass");
            let granted = Authority { granted: true, ..authority("bypass_all") };
            assert!(decide(&granted, app, action, "sensitive", SAID).is_ok(), "a person's Allow runs {action}");
        }
        assert!(decide(&authority("bypass_all"), "shell", "set_theme", "sensitive", SAID).is_ok(), "other actions: unchanged");
        assert!(!always_asked("notes", "report_problem") && !always_asked("shell", "send_mail"));
    }

    #[test]
    fn no_session_rule_answers_one() {
        let mut full = authority("bypass_all");
        full.mode.session_rules.push(("shell".into(), "report_problem".into()));
        full.mode.standing_rules.push(("shell".into(), "report_problem".into()));
        assert!(decide(&full, "shell", "report_problem", "sensitive", SAID).is_err(), "a rule does not cover it");
    }
}
