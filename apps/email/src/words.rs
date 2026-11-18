//! What the mailbox says in a person's words rather than a protocol's (the art direction of 20 Oct
//! 2026, design/art-direction-2026-10-20.md, H): a folder's name without the provider's namespace,
//! and a message's date the way a person says one.

use chrono::{Datelike, NaiveDate, NaiveDateTime};

/// A folder's name as the sidebar says it. The server's name stays the folder's id (selection,
/// moves and counts all use it); this is only what is drawn.
///
/// `[Gmail]/All Mail` is "All mail" (Gmail's own meaning, not "Archive"), `[Gmail]/Sent Mail` is
/// "Sent", `INBOX` is "Inbox", and a Courier-style `INBOX.Receipts` is "Receipts". A folder of the
/// person's own keeps its name.
pub fn folder_label(name: &str) -> String {
    let bare = ["[Gmail]/", "[Google Mail]/", "INBOX.", "INBOX/"]
        .iter()
        .find_map(|p| name.strip_prefix(p))
        .unwrap_or(name);
    match bare.to_lowercase().as_str() {
        "inbox" => "Inbox".into(),
        "sent mail" | "sent" | "sent items" | "sent messages" => "Sent".into(),
        "all mail" => "All mail".into(),
        "drafts" | "draft" => "Drafts".into(),
        "spam" | "junk" | "junk e-mail" | "junk mail" => "Spam".into(),
        "trash" | "bin" | "deleted items" | "deleted messages" => "Trash".into(),
        "starred" | "flagged" => "Starred".into(),
        "important" => "Important".into(),
        "archive" => "Archive".into(),
        // Gmail's container for its own folders, listed on its own.
        "[gmail]" | "[google mail]" => "Gmail".into(),
        _ => bare.to_string(),
    }
}

/// A message's date in the list: the time for today, "Yesterday", the day and month this year,
/// and the year as well before that. `date` is the service's "2026-10-19 22:35"; anything else is
/// shown as it came, since a wrong guess at a date is worse than an untidy one.
pub fn list_date(date: &str, today: NaiveDate) -> String {
    let Ok(at) = NaiveDateTime::parse_from_str(date.trim(), "%Y-%m-%d %H:%M") else {
        return date.to_string();
    };
    let day = at.date();
    if day == today {
        at.format("%H:%M").to_string()
    } else if today.pred_opt() == Some(day) {
        "Yesterday".into()
    } else if day.year() == today.year() {
        format!("{} {}", day.day(), day.format("%b"))
    } else {
        format!("{} {} {}", day.day(), day.format("%b"), day.year())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_is_named_without_its_providers_namespace() {
        assert_eq!(folder_label("[Gmail]/All Mail"), "All mail", "Gmail's All Mail is not an archive");
        assert_eq!(folder_label("[Gmail]/Sent Mail"), "Sent");
        assert_eq!(folder_label("[Gmail]/Drafts"), "Drafts");
        assert_eq!(folder_label("INBOX"), "Inbox");
        assert_eq!(folder_label("INBOX.Receipts"), "Receipts");
        assert_eq!(folder_label("Work/Clients"), "Work/Clients", "the person's own keeps its name");
        assert_eq!(folder_label("Junk E-mail"), "Spam");
        assert_eq!(folder_label("[Gmail]"), "Gmail");
    }

    #[test]
    fn a_date_is_said_the_way_a_person_says_one() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 20).unwrap();
        assert_eq!(list_date("2026-10-20 10:35", today), "10:35");
        assert_eq!(list_date("2026-10-19 22:35", today), "Yesterday");
        assert_eq!(list_date("2026-10-18 09:00", today), "18 Oct");
        assert_eq!(list_date("2025-12-31 23:59", today), "31 Dec 2025");
        assert_eq!(list_date("Tue, 14:05", today), "Tue, 14:05", "not ours to reparse");
    }
}
