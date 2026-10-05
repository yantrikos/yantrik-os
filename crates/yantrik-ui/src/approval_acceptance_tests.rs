//! Acceptance evidence for the approval card's safety rules that live in the store and the request
//! path, rather than in pixels (the outside design review asked for checks a build runs, not
//! stills). The pixel half — keys press nothing, Decline in reach while the card scrolls, a click
//! on an expired card answers nothing — is in tests/ui-preview (`verify-approval-pointer-only`,
//! `verify-approval-fit`, `verify-approval-expired`).
use super::*;
use std::time::{Duration, Instant};

fn ask(store: &mut Store, now: Instant) -> String {
    store
        .request(
            "pi",
            Verified::default(),
            "calendar",
            "delete_event",
            serde_json::json!({ "id": "evt-3" }),
            "sensitive",
            "Delete an event from the calendar. It is not recoverable.",
            "",
            "",
            now,
            "12:03",
        )
        .expect("a first request is accepted")
        .id
}

/// Expiry, then a click: the card leaves the screen, the late Allow is refused, the act it would
/// have allowed cannot be run, and the asker hears "not answered", never "allowed".
#[test]
fn after_expiry_the_card_is_gone_and_a_late_allow_cannot_run_the_act() {
    let mut store = Store::new();
    let now = Instant::now();
    let id = ask(&mut store, now);
    let args = serde_json::json!({ "id": "evt-3" });
    assert_eq!(store.pending(now).len(), 1, "the card is up while it waits");

    let later = now + REQUEST_TTL + Duration::from_secs(1);
    assert!(store.pending(later).is_empty(), "an expired card is no longer one to answer");
    assert_eq!(store.status(&id, later), Some(Status::Expired));

    // The click that comes in after: the Allow callback's own call, `grant`.
    let late = store.grant(&id, later, "12:06").expect_err("a click after expiry grants nothing");
    assert!(late.contains("expired"), "{late}");
    assert!(store.grant_for_session(&id, later, "12:06").is_err(), "nor does the session row");
    // And the act the card was about: the app's dispatch spends a grant, and there is none.
    let run = store.consume(&id, "calendar", "delete_event", &args, later).expect_err("nothing to spend");
    assert!(run.contains("expired"), "{run}");
    assert_eq!(store.status(&id, later), Some(Status::Expired), "refusals change nothing");
    let (outcome, _) = store.outcome(&id, later).expect("an expired card has an outcome");
    assert!(matches!(outcome, Outcome::Unanswered), "the asker hears it was not answered, not allowed");

    // A deny after expiry is refused too: the record stays what the clock made it.
    assert!(store.deny(&id, later, "12:06").is_err());
}

/// A card answered with Deny cannot be turned into a yes by a second, later click on Allow.
#[test]
fn a_declined_card_cannot_be_allowed_by_a_later_click() {
    let mut store = Store::new();
    let now = Instant::now();
    let id = ask(&mut store, now);
    store.deny(&id, now, "12:03").expect("Decline answers");
    let soon = now + Duration::from_millis(300);
    assert!(store.grant(&id, soon, "12:03").is_err(), "a double click's second half grants nothing");
    let args = serde_json::json!({ "id": "evt-3" });
    assert!(store.consume(&id, "calendar", "delete_event", &args, soon).is_err());
}

/// An oversized or obfuscated request is refused before any card exists. Two halves: the rule
/// refuses each shape, and in `request_approval` that refusal comes before the store is asked to
/// raise a card, before the phone is told, before a pane draws it and before the screen syncs.
#[test]
fn an_oversized_or_obfuscated_request_is_refused_before_any_card_exists() {
    let refuse = |args: serde_json::Value| crate::approval_bounds::refusal(&args, None, false);
    for (shape, args) in [
        ("an argument name past the limit", serde_json::json!({ "k".repeat(41): 1 })),
        ("a right-to-left override in a name", serde_json::json!({ "pa\u{202E}th": 1 })),
        ("a zero-width space in a name", serde_json::json!({ "pa\u{200B}th": 1 })),
        ("a control character in a name", serde_json::json!({ "pa\u{1}th": 1 })),
        ("arguments past the card's budget", serde_json::json!({ "a": "x".repeat(200), "b": "y".repeat(200), "c": "z".repeat(200), "d": "w".repeat(200), "e": "v".repeat(200), "f": "u".repeat(200), "g": "t".repeat(200), "h": "s".repeat(200), "i": "r".repeat(200), "j": "q".repeat(200), "k": "p".repeat(200), "l": "o".repeat(200) })),
    ] {
        assert!(refuse(args).is_some(), "{shape} is refused");
    }
    let whole = |args: serde_json::Value| crate::approval_bounds::refusal(&args, None, true);
    assert!(whole(serde_json::json!({ "command": "x".repeat(400) })).is_some(), "a value too long to show whole");
    assert!(refuse(serde_json::json!({ "path": "/tmp/a" })).is_none(), "an ordinary request is asked");

    let src = include_str!("control_approvals.rs");
    let handler = src.find("Action::new(\"request_approval\"").expect("request_approval is published");
    let body = &src[handler..];
    let at = |needle: &str| body.find(needle).unwrap_or_else(|| panic!("request_approval no longer has {needle:?}"));
    let refused = at("crate::approval_bounds::refusal(&parsed");
    for later in ["approvals::request(", "crate::channels::card_raised(", "draw_in_pane(&agent", "sync(&ui);"] {
        assert!(at(later) > refused, "{later:?} comes before the size refusal: a refused request would leave a card");
    }
}
