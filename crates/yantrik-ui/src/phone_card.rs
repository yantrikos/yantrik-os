//! What the person's phone is told about a card a turn from it raised, and whether it may be
//! answered there (`channels::card_raised`).
//!
//! The phone sees what the desk sees, in plain one-line text: the app's own sentence, the line
//! naming a handle, every row the app named the target with (security review of #652, L1 — the
//! named rows used to stop at the desk, so a phone allowed a kill it was shown only as a pid), and
//! the arguments the grant binds. A card the desk could only decline — a destructive call whose
//! target the app could not name — is never given a code.

use crate::approvals::{Card, Named};

/// The card as the phone shows it: who asks for what, then everything the desk names it by, one
/// line each, escaped as the card escapes it (`approval_wording::visible`), never cut.
pub fn what(mind: &str, card: &Card, published: &str) -> String {
    let line = |s: &str| crate::approval_wording::visible(s.trim());
    let mut what = format!("{} asks to run {}.{} ({}).", line(mind), line(&card.app), line(&card.action), line(&card.grade));
    if !published.trim().is_empty() {
        what.push_str(&format!("\n{}", line(published)));
    }
    if !card.target.trim().is_empty() {
        what.push_str(&format!("\n{}", line(&card.target)));
    }
    if let Named::Resolved(target) = &card.named {
        for (label, value) in &target.rows {
            what.push_str(&format!("\n{}: {}", line(label), line(value)));
        }
    }
    if card.target_blocked {
        what.push_str(&format!(
            "\n{}",
            crate::approval_target::unavailable(&crate::approval_target::verb_of(&card.action))
        ));
    }
    for row in &card.args {
        what.push_str(&format!("\n  {}", line(row)));
    }
    what
}

/// Why the card waits at the machine rather than taking a code, or `None` when the phone may
/// answer it. `refused_here` is the app's sentence saying it cannot be undone, or an action no
/// phone answers (`channels::never_from_a_phone`).
pub fn waits(card: &Card, refused_here: bool) -> Option<&'static str> {
    // A code only for a card the phone is shown whole: an argument cut short, or more of them
    // than fit, is a grant bound to what the person did not see.
    let whole = card.args.iter().all(|r| !r.ends_with('…') && !r.starts_with('…'));
    if card.target_blocked {
        Some("This one can only be declined: the app could not say what it would act on.")
    } else if refused_here {
        Some("This one waits for you at the machine: it cannot be undone, or it runs commands as you.")
    } else if !whole {
        Some("This one waits for you at the machine: it is too long to show here in full.")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approval_target::Resolved;
    use crate::approvals::{Asked, Store, Verified};

    fn card(named: &Named, action: &str, published: &str) -> Card {
        let mut store = Store::new();
        let now = std::time::Instant::now();
        store
            .raise(
                "pi",
                Verified::default(),
                Asked { app: "system-monitor", action, grade: "dangerous", purpose: "", published, target: "", explained: "", named },
                serde_json::json!({ "pid": 4242 }),
                now,
                "10:00",
            )
            .expect("asked");
        store.cards(now).pop().expect("the card")
    }

    /// L1: the rows the app named the target with reach the phone, escaped and a line each.
    #[test]
    fn the_phone_is_shown_the_named_rows_escaped_one_to_a_line() {
        let named = Named::Resolved(Resolved {
            rows: vec![
                ("Process".into(), "firefox, pid 4242".into()),
                ("Command".into(), "firefox\nUndo: possible \u{202e}x".into()),
            ],
            series: false,
            identity: "pid 4242 started 1".into(),
            handles: vec!["pid".into()],
        });
        let shown = what("Hermes", &card(&named, "kill_process", "End a running process by pid"), "End a running process by pid");
        let lines: Vec<&str> = shown.lines().collect();
        assert!(lines.contains(&"Process: firefox, pid 4242"), "{shown}");
        assert!(lines.contains(&"Command: firefox\\nUndo: possible <U+202E>x"), "{shown}");
        assert!(!lines.iter().any(|l| l.starts_with("Undo")), "no row starts a line of its own: {shown}");
        assert!(lines.contains(&"  pid: 4242"), "and the argument the grant binds: {shown}");
        assert!(!shown.contains("started 1"), "the identity is never shown");
    }

    /// A destructive card whose target the app could not name is never given a code: the phone
    /// is told it can only be declined, and why.
    #[test]
    fn an_unnamed_destructive_target_is_never_answered_from_the_phone() {
        let blocked = card(&Named::Unresolved, "kill_process", "End a running process by pid");
        assert!(blocked.target_blocked);
        assert_eq!(waits(&blocked, false), Some("This one can only be declined: the app could not say what it would act on."));
        assert!(what("Hermes", &blocked, "").contains("Target details unavailable"));

        let named = Named::Resolved(Resolved {
            rows: vec![("Process".into(), "firefox, pid 4242".into())],
            identity: "pid 4242 started 1".into(),
            ..Resolved::default()
        });
        assert_eq!(waits(&card(&named, "kill_process", "End a running process by pid"), false), None, "named: the phone may answer");
        assert!(waits(&card(&named, "kill_process", "End it. It cannot be undone"), true).is_some());
    }
}
