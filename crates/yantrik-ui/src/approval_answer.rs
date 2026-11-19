//! How an answer to a request for approval was reached (#1311, round 2).
//!
//! A caller that wants "a person pressed Allow on a card for this call" and nothing weaker must
//! be able to tell it from the other ways a call goes ahead: the mode runs it without a card, a
//! session rule the person made earlier answers it, an administrator's standing rule answers it.
//! The shell says so in its own answers, in one field, `answered`, and a caller cannot set it:
//!
//! - `pressed`: a grant, which only the UI mints (`Store::grant`, `Store::grant_for_session`: a
//!   person pressed Allow, or Allow for this session, on this card). Said by `approval_status`
//!   of a granted request and by `consume_approval`, which succeeds only on such a grant.
//! - `mode`, `session_rule`, `standing_rule`: said by `request_approval` when it answers
//!   `not_needed`, with nobody asked.
//!
//! Split from `control_approvals.rs`, which is over its size limit and may not grow.

use crate::approvals::Status;

/// Who answered a request that needed no card.
pub fn not_needed_by(standing_rule: bool, session_rule_listed: bool) -> &'static str {
    if standing_rule {
        "standing_rule"
    } else if session_rule_listed {
        "session_rule"
    } else {
        "mode"
    }
}

/// How a request that went to a card was answered, if it was answered by a person.
pub fn pressed(status: Status) -> Option<&'static str> {
    matches!(status, Status::Granted | Status::Consumed).then_some("pressed")
}

/// `request_approval`'s answer when the mode, a session rule or a standing rule runs the action
/// with no card.
pub fn not_needed(app: &str, action: &str, grade: &str, declared: crate::mind_mode::Declared) -> serde_json::Value {
    let standing = crate::mind_mode::standing_rule_answers(grade, app, action, declared);
    let listed = crate::mind_mode::snapshot()["session_rules"]
        .as_array()
        .is_some_and(|rules| rules.iter().any(|r| r["app"] == app && r["action"] == action));
    let by = not_needed_by(standing, listed);
    let mut answer = serde_json::json!({
        "status": "not_needed",
        "app": app,
        "action": action,
        "grade": grade,
        "mode": crate::mind_mode::current().as_str(),
        "answered": by,
    });
    answer["next"] = if standing {
        answer["standing_rule"] = true.into();
        "nobody was asked and nobody needs to be: this machine's administrator allows this action without a card \
         (/etc/yantrik/standing-rules.conf). Run the action, then call record_unasked_action with mode `standing_rule`."
    } else {
        "nobody was asked and nobody needs to be: this desktop's current mode runs this without a card. Run the \
         action. If it ran unasked, call record_unasked_action afterwards."
    }
    .into();
    answer
}

/// `approval_status`'s answer.
pub fn status_json(id: &str, status: Status) -> serde_json::Value {
    let mut answer = serde_json::json!({ "request_id": id, "status": status.as_str() });
    if let Some(how) = pressed(status) {
        answer["answered"] = how.into();
    }
    answer
}

/// `consume_approval`'s answer: only a pressed grant can be spent.
pub fn consumed_json(id: &str, app: &str, action: &str) -> serde_json::Value {
    serde_json::json!({
        "request_id": id,
        "consumed": true,
        "answered": "pressed",
        "authorises": format!("{app}.{action}"),
        "note": "one action, once. Run it now; this grant is spent.",
    })
}

#[cfg(test)]
#[path = "approval_answer_tests.rs"]
mod tests;
