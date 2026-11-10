//! The question "may this mail go out?", asked in the shell's own words (#1026, 520 gate).
//!
//! The companion's mail tools send through `email-service` and nothing in that path asked the
//! person: a tool graded `standard` runs under the companion's own ceiling, and the policy
//! engine's "ask" only writes a log line. On VM 520 three replies left the person's mailbox with
//! no card in `auto` and in `ask`. The Email app publishes no `send` on purpose (a draft can be
//! read, mail that has gone cannot be taken back), so there was no action for a card to be about.
//!
//! This is that action, and the whole of it: `send_mail` is **sensitive** and its description says
//! it cannot be undone, which is what makes every mode but full bypass raise a card
//! (`gate::decide`, the same reading as `calendar.delete_event`), and plan mode refuse. The shell
//! does not send anything: the handler refuses, always. The caller asks with `request_approval`,
//! waits for the person, spends the grant with `consume_approval`, and only then sends. One
//! approvals module, one card, one place a person says yes.
//!
//! The card shows the whole mail: every recipient, the whole subject and the whole body, each
//! quoted so text inside one cannot pose as another field. A mail the card cannot show whole is
//! refused (`approval_bounds`: "shown whole or not asked about"), never clipped: what the person
//! cannot read, they cannot allow. The grant is bound to exactly these values.

use yantrik_app_runtime::control::{Action, App as ControlSurface, Param};

/// The action's name, as the companion asks about it.
pub const ACTION: &str = "send_mail";

/// The grade it is published at: above `standard`, so a `tool_permission: standard` ceiling refuses it.
pub const GRADE: &str = "sensitive";

/// What a person reads on the card. "Cannot be undone" is the phrase `gate::unrecoverable` reads.
pub const PURPOSE: &str = "Send an email from one of the person's accounts, to people outside this machine. \
                           Once sent it cannot be undone. This action sends nothing itself: the mind asks it \
                           to put a card in front of the person, and sends only after their Allow.";

pub fn actions(surface: ControlSurface) -> ControlSurface {
    surface.action(
        Action::new(ACTION, PURPOSE)
            .risk(GRADE)
            .arg(Param::text("account").describe("The address it is sent from").optional())
            .arg(Param::text("to").describe("Recipients, in full").optional())
            .arg(Param::text("cc").describe("Copied recipients, in full").optional())
            .arg(Param::text("bcc").describe("Blind-copied recipients, in full").optional())
            .arg(Param::text("subject").describe("The whole subject, quoted").optional())
            .arg(Param::text("body").describe("The whole body, quoted").optional()),
        |_| {
            Err("send_mail is a question and not a way to send: the mind asks it, waits for the person's \
                 Allow, and sends the mail itself. Nothing was sent."
                .to_string())
        },
    )
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use yantrik_ipc_transport::gate::{self, Authority, Mode as GateMode};

    use super::*;
    use crate::mind_mode::{Decision, Mode, Modes};

    /// The grade and sentence the surface publishes: read from the declaration itself, so a
    /// regrade in `actions` (not only in this test's copy of it) is what the tests decide on.
    fn published() -> (String, String) {
        let production = include_str!("control_mail.rs").split("#[cfg(test)]").next().unwrap();
        let declaration = production.split("Action::new(").nth(1).expect("the declaration");
        assert!(declaration.starts_with("ACTION, PURPOSE)"), "the action is not declared with ACTION and PURPOSE");
        assert!(declaration.contains(".risk(GRADE)"), "the action is not published at GRADE");
        (GRADE.to_string(), PURPOSE.to_string())
    }

    fn authority(mode: &str) -> Authority {
        Authority { ceiling: "dangerous".into(), mode: GateMode::named(mode), granted: false, asks_above: None }
    }

    #[test]
    fn it_is_published_as_sensitive_and_as_what_cannot_be_undone() {
        let (grade, said) = published();
        assert_eq!(grade, "sensitive", "the grade under the machine ceiling");
        assert!(gate::unrecoverable(&said), "{said}");
        assert!(crate::approvals::shown_whole(&grade, &said), "the card shows it whole or not at all");
    }

    /// 520 gate, 30 Oct: mail went out in `auto` and in `ask` with no card. The shell's real
    /// decision table, on the values the surface really publishes.
    #[test]
    fn the_shell_asks_in_ask_auto_and_bypass_refuses_in_plan_and_runs_only_in_full_bypass() {
        let (grade, said) = published();
        let cannot_be_undone = gate::unrecoverable(&said);
        let decide = |mode| Modes::new(mode).decide(&grade, "shell", ACTION, cannot_be_undone, "dangerous", Instant::now());
        for mode in [Mode::Ask, Mode::Auto, Mode::Bypass] {
            assert!(matches!(decide(mode), Decision::Ask), "{mode:?} did not ask");
        }
        assert!(matches!(decide(Mode::Plan), Decision::Refuse { .. }), "plan");
        // Full bypass is time-boxed and cannot be entered by construction here; the wire gate
        // answers for it below: the person's own word, the one mode that runs it unasked.
        // The wire gate agrees, on the same values.
        for mode in ["ask", "auto", "bypass", "plan"] {
            assert!(gate::decide(&authority(mode), "shell", ACTION, &grade, &said).is_err(), "{mode}");
        }
        assert!(gate::decide(&authority("bypass_all"), "shell", ACTION, &grade, &said).is_ok(), "full bypass");
        let granted = Authority { granted: true, ..authority("auto") };
        assert!(gate::decide(&granted, "shell", ACTION, &grade, &said).is_ok());
    }

    /// What the card will do with a mail: a short one is put up; one it cannot show whole is not.
    #[test]
    fn a_short_mail_fits_the_card_and_a_long_one_is_refused_rather_than_clipped() {
        let (grade, said) = published();
        let whole = crate::approvals::shown_whole(&grade, &said);
        let quoted = |t: &str| serde_json::Value::String(serde_json::Value::String(t.to_string()).to_string());
        let mail = |body: &str| {
            serde_json::json!({
                "account": "ada@example.com", "to": quoted("priya@example.com"),
                "subject": quoted("Plans; to: someone@else.example"), "body": quoted(body),
            })
        };
        assert_eq!(crate::approval_bounds::refusal(&mail("Thanks, Monday works."), None, whole), None);
        let long = "x".repeat(400);
        assert!(crate::approval_bounds::refusal(&mail(&long), None, whole).is_some(), "a body the card cannot show whole");
    }
}
