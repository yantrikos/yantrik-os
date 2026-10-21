//! What the companion can see of Email without photographing the window: a projection of the
//! model, the same keys and values the app has always published.
//!
//! Never in it: a password (the model never holds one), the account's secrets (the service's
//! summary carries none), or the HTML original. The open message's text is in it, because
//! summarising mail is the most common thing to want and fetching it a second way would be the
//! screenshot problem again.

use serde_json::{json, Value};
use yantrik_app::control::View;

use crate::model::Model;
use crate::state::{self, Counted};

/// A folder's counts as `describe` prints them: the two numbers, or `null` — never `0/0` for a
/// folder that was not counted, because that is what an empty folder prints.
pub fn counts_json(counts: &Counted) -> Value {
    match counts.known() {
        Some(c) => json!({ "unread": c.unread, "total": c.total }),
        None => Value::Null,
    }
}

pub fn view(m: &Model) -> View {
    let mail_state = &m.state;

    // The state the surface always carries, whichever of the three this is. `service` and
    // `notice` are the two that were missing: a caller could not tell a machine with no account
    // from one whose mail service had never been started, because the app could not either.
    let base = View::new(mail_state.summary())
        .with("service", mail_state.service_word())
        .with("notice", m.notice.clone())
        .with("account_store", mail_state.config_path().to_string());

    let base = match mail_state.has_account() {
        // Deliberately null rather than false. With nothing answering, this app has not been
        // told whether an account exists, and `false` is the exact claim that had an audit
        // conclude this machine had no mail account.
        None => return base.with("has_account", Value::Null),
        Some(known) => base.with("has_account", known),
    };
    if mail_state.has_account() == Some(false) {
        return base;
    }

    let folder = m.folder.clone();
    let reading = m.reading();
    let shown = m.shown();
    let header = m.header();

    let summary = if m.composer.open {
        let to = &m.composer.text.to;
        format!("Email — composing to {}", if to.is_empty() { "(nobody yet)" } else { to })
    } else if let Some(open) = reading {
        format!("Email — {folder}, reading \u{201c}{}\u{201d} from {}", open.detail.subject, open.detail.from_name)
    } else {
        // The same two numbers as the folder's entry in `folders` below, by construction: the
        // header is read off that list — and when that entry has none, this says so rather than
        // "0 unread of 0".
        let search = m.search.active.then_some((m.search.query.as_str(), shown.len()));
        state::folder_summary(&folder, &header, search)
    };

    let messages: Vec<Value> = shown
        .iter()
        .take(25)
        .enumerate()
        .map(|(i, (_, r))| {
            json!({
                // The number `open_message which=` takes, said beside the message it opens, so a
                // caller never has to count the list itself.
                "row": i + 1,
                "subject": r.item.subject,
                "from": r.item.from_name,
                "address": r.item.from_addr,
                "date": r.item.date_text,
                "read": r.item.is_read,
                "flagged": r.item.is_flagged,
            })
        })
        .collect();

    // `counts` is `{unread, total}` as the mail server counted the folder, or `null` when it did
    // not count it this time — with `reason` saying why. Never `0/0` for that: an empty folder is
    // `{"unread": 0, "total": 0}`, and a refused STATUS is not an empty folder (#131).
    let folder_rows: Vec<Value> = m
        .folder_records()
        .into_iter()
        .map(|f| {
            json!({
                "name": f.name,
                "counts": f.counts.map(|c| json!({ "unread": c.unread, "total": c.total })),
                "reason": f.reason,
            })
        })
        .collect();

    let open = match reading {
        Some(open) => {
            let d = &open.detail;
            json!({
                "subject": d.subject,
                "from": d.from_name,
                "address": d.from_addr,
                "date": d.date_text,
                "flagged": d.is_flagged,
                "read": d.is_read,
                "body": d.body,
            })
        }
        None => Value::Null,
    };

    let mut view = base
        .with("account", m.account_name.clone())
        .with("folder", folder)
        .with("triage", m.triage.label())
        // `counts` is the open folder's `{unread, total}`, as the mail server counts them — the
        // same numbers as its entry in `folders` — or `null` when the server did not count it,
        // with `counts_reason` saying why. `listed` is how many messages are in hand: the newest
        // page, which is what `messages` is drawn from.
        .with("counts", counts_json(&header))
        .with(
            "counts_reason",
            match &header {
                Counted::Unavailable { reason } => Value::from(reason.as_str()),
                Counted::Known(_) => Value::Null,
            },
        )
        .with("listed", shown.len())
        .with("composing", m.composer.open)
        .with("search_query", m.search.query.clone())
        .with("open_message", open)
        .with("folders", Value::Array(folder_rows))
        .with("messages", Value::Array(messages));
    // The one line a caller surveying every window pays for. It says what is on screen once
    // there is a mailbox; until then `MailState::summary` has already said which of the two other
    // things is true, and that is the more important news.
    view.summary = summary;
    view
}
