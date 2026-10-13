//! The one approval card a pane answers: the oldest waiting request for the pane's agent, drawn
//! pinned above the reply box and outside the transcript's scroll (agents.slint, `PinnedApproval`).
//!
//! A card inside the transcript could be scrolled half out of view by whatever was appended under
//! it — a second request from parallel calls, a tool card whose output grows — while the pane
//! followed the bottom: who was asking, the grade and the action's sentence went off the top, and
//! the arguments and a live Allow stayed in view. The corner hides that agent's card while its
//! pane is on screen (control_approvals.rs, `in_the_pane`), so nothing else showed the head. Now
//! the pane draws one card, the same one the corner would — the oldest — where nothing can scroll
//! it, and every other waiting card for the agent is a count under it and a line in the
//! transcript, never a second live Allow.
//!
//! Plain data from the approval store, so every case is a test. The card itself is the corner's
//! (`control_approvals::row_for`), answered through the same callbacks; nothing here grants.

use crate::approvals::{Card, Status};

/// The card to pin for `agent` and how many more of its requests wait behind it. Drawn from the
/// shell's store, oldest first (`approvals::pending` keeps the order requests arrived in), under
/// the same predicate that hides them from the corner: the verified agent, never anything the
/// request says.
pub(super) fn pinned_for<'a>(agent: &str, pending: &'a [Card]) -> (Option<&'a Card>, usize) {
    let mut waiting = pending.iter().filter(|c| c.status == Status::Pending && !agent.is_empty() && c.verified.agent == agent);
    let first = waiting.next();
    (first, waiting.count())
}

/// The pinned card as the pane draws it: the corner's row, with who the agent works for, which is
/// what the transcript's copy of the card carried.
pub(super) fn pinned_row(card: Option<&Card>, on_behalf: &str) -> crate::ApprovalRequest {
    match card {
        Some(card) => crate::ApprovalRequest { on_behalf: on_behalf.into(), ..crate::control_approvals::row_for(card.clone()) },
        None => crate::ApprovalRequest::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(id: &str, agent: &str, status: Status) -> Card {
        Card {
            id: id.into(),
            requester: "pi 0.87".into(),
            verified: crate::approvals::Verified { agent: agent.into(), ..Default::default() },
            app: "shell".into(),
            action: "agent_run".into(),
            grade: "sensitive".into(),
            purpose: String::new(),
            summary: String::new(),
            args: vec!["command: ls".into()],
            target: String::new(),
            explained: String::new(),
            warning: String::new(),
            said: String::new(),
            caller_says: String::new(),
            can_session: true,
            status,
            record: String::new(),
            decided_at: String::new(),
            session: false,
            age_secs: 4,
        }
    }

    /// Two waiting requests from one agent: one pinned, the oldest, and one more counted behind it.
    #[test]
    fn the_oldest_waiting_card_is_pinned_and_the_rest_are_a_count() {
        let pending = [
            card("appr-1", "pi:c-1", Status::Pending),
            card("appr-x", "deepseek:c-2", Status::Pending),
            card("appr-2", "pi:c-1", Status::Pending),
        ];
        let (first, more) = pinned_for("pi:c-1", &pending);
        assert_eq!(first.map(|c| c.id.as_str()), Some("appr-1"));
        assert_eq!(more, 1, "the second waits behind the first, with no Allow of its own");
    }

    /// Answering the first brings up the second; answering that leaves nothing pinned.
    #[test]
    fn answering_the_pinned_card_brings_up_the_next() {
        let mut pending = vec![card("appr-1", "pi:c-1", Status::Pending), card("appr-2", "pi:c-1", Status::Pending)];
        pending[0].status = Status::Granted;
        let (first, more) = pinned_for("pi:c-1", &pending);
        assert_eq!((first.map(|c| c.id.as_str()), more), (Some("appr-2"), 0));
        pending.remove(1);
        let (first, more) = pinned_for("pi:c-1", &pending);
        assert!(first.is_none() && more == 0);
        assert_eq!(pinned_row(first, "").id, "", "nothing waits: no card is drawn");
    }

    /// Another agent's card, or a card with no agent, is never pinned in this pane: it is not
    /// hidden from the corner either.
    #[test]
    fn only_the_panes_own_agent_is_pinned() {
        let pending = [card("appr-x", "deepseek:c-2", Status::Pending), card("appr-y", "", Status::Pending)];
        assert!(pinned_for("pi:c-1", &pending).0.is_none());
        assert!(pinned_for("", &pending).0.is_none(), "no agent shown, nothing pinned");
    }
}
