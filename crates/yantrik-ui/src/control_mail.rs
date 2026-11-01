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
//! The card shows what is arguments, not the whole mail: recipients and subject whole, the first
//! words of the body, its length and a digest of all of it. The grant is bound to those exact
//! values, so a body changed after the person looked cannot go out under their Allow.

use yantrik_app_runtime::control::{Action, App as ControlSurface, Param};

/// The action's name, as the companion asks about it.
pub const ACTION: &str = "send_mail";

/// What a person reads on the card. "Cannot be undone" is the phrase `gate::unrecoverable` reads.
pub const PURPOSE: &str = "Send an email from one of the person's accounts, to people outside this machine. \
                           Once sent it cannot be undone. This action sends nothing itself: the mind asks it \
                           to put a card in front of the person, and sends only after their Allow.";

pub fn actions(surface: ControlSurface) -> ControlSurface {
    surface.action(
        Action::new(ACTION, PURPOSE)
            .risk("sensitive")
            .arg(Param::text("account").describe("The address it is sent from").optional())
            .arg(Param::text("to").describe("Recipients, in full").optional())
            .arg(Param::text("cc").describe("Copied recipients, in full").optional())
            .arg(Param::text("bcc").describe("Blind-copied recipients, in full").optional())
            .arg(Param::text("subject").describe("The subject").optional())
            .arg(Param::text("body_start").describe("The first words of the body").optional())
            .arg(Param::text("body_chars").describe("How long the whole body is").optional())
            .arg(Param::text("body_digest").describe("A digest of the whole body: the grant is bound to it").optional()),
        |_| {
            Err("send_mail is a question and not a way to send: the mind asks it, waits for the person's \
                 Allow, and sends the mail itself. Nothing was sent."
                .to_string())
        },
    )
}

#[cfg(test)]
mod tests {
    use yantrik_ipc_transport::gate::{self, Authority, Mode};

    use super::*;

    fn authority(mode: &str) -> Authority {
        Authority { ceiling: "dangerous".into(), mode: Mode::named(mode), granted: false, asks_above: None }
    }

    /// 520 gate, 30 Oct: mail went out in `auto` and in `ask` with no card.
    #[test]
    fn sending_mail_asks_in_auto_and_in_ask_and_is_refused_in_plan() {
        for mode in ["ask", "auto", "bypass"] {
            let answer = gate::decide(&authority(mode), "shell", ACTION, "sensitive", PURPOSE);
            assert!(answer.is_err(), "{mode} ran a send without a card");
        }
        assert!(gate::decide(&authority("plan"), "shell", ACTION, "sensitive", PURPOSE).is_err(), "plan");
        // Only the person's explicit full bypass stops the asking, as for every other act that
        // cannot be undone.
        assert!(gate::decide(&authority("bypass_all"), "shell", ACTION, "sensitive", PURPOSE).is_ok());
        // And a Allow for exactly this call runs it.
        let granted = Authority { granted: true, ..authority("auto") };
        assert!(gate::decide(&granted, "shell", ACTION, "sensitive", PURPOSE).is_ok());
    }

    #[test]
    fn the_description_says_it_cannot_be_undone_and_the_grade_is_above_standard() {
        assert!(gate::unrecoverable(PURPOSE));
        assert_eq!(gate::grade("sensitive"), Some(2));
    }
}
