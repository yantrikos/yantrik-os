//! What the wire says, as the rows the window draws: folders, messages, the open message, and
//! the composer's fields for a reply or a forward. No window and no I/O here.

use yantrik_ipc_contracts::email::{EmailDetail, EmailFolder, EmailSummary};

use crate::model::{Attachment, Detail, FolderRow, ListItem, Open, Rgb, Row, ThreadEntry};
use crate::state::Draft;
use crate::words;

/// The avatar colours, by place in the list.
const AVATARS: [Rgb; 5] = [(0x4E, 0x79, 0xA7), (0xF2, 0x8E, 0x2C), (0xE1, 0x57, 0x59), (0x76, 0xB7, 0xB2), (0x59, 0xA1, 0x4F)];

/// A sender's name: what comes before the address.
fn name_of(from: &str) -> String {
    from.split('<').next().unwrap_or(from).trim().to_string()
}

fn initial_of(name: &str) -> String {
    name.chars().next().unwrap_or('?').to_uppercase().to_string()
}

/// One listed message, at `idx` in what the folder returned.
pub fn row(s: &EmailSummary, idx: usize, today: chrono::NaiveDate) -> Row {
    let from_name = name_of(&s.from);
    Row {
        id: s.id.clone(),
        folder: s.folder.clone(),
        item: ListItem {
            avatar_initial: initial_of(&from_name),
            from_name,
            from_addr: s.from.clone(),
            subject: s.subject.clone(),
            preview: s.snippet.clone(),
            // "10:35", "Yesterday", "18 Oct": the full date and time are on the message itself.
            date_text: words::list_date(&s.date, today),
            is_read: s.is_read,
            is_flagged: s.is_starred,
            is_selected: false,
            has_attachment: s.has_attachments,
            thread_count: 0,
            thread_id: s.thread_id.clone().unwrap_or_default(),
            avatar: AVATARS[idx % AVATARS.len()],
        },
    }
}

/// A folder's messages as rows, each carrying the folder it was found in.
pub fn rows(messages: &[EmailSummary]) -> Vec<Row> {
    let today = chrono::Local::now().date_naive();
    messages.iter().enumerate().map(|(i, s)| row(s, i, today)).collect()
}

/// The message the service answered with, as the reading pane draws it.
pub fn open(id: &str, folder: &str, d: &EmailDetail) -> Open {
    let from_name = name_of(&d.from);
    let body = if d.body_text.is_empty() { &d.body_html } else { &d.body_text };
    let detail = Detail {
        from_initial: initial_of(&from_name),
        from_name,
        from_addr: d.from.clone(),
        avatar: AVATARS[0],
        to_addr: d.to.join(", "),
        cc_addr: d.cc.join(", "),
        subject: d.subject.clone(),
        date_text: d.date.clone(),
        body: body.clone(),
        ai_summary: String::new(),
        // Read off the message rather than assumed. These were `false` and `true` regardless of
        // what the mail server said, so the star on an open message was always hollow.
        is_flagged: d.is_starred,
        is_read: d.is_read,
        has_attachment: !d.attachments.is_empty(),
        attachment_names: d.attachments.iter().map(|a| a.filename.clone()).collect::<Vec<_>>().join(", "),
        thread_count: d.thread_messages.len() as i32,
        // Whether "Open original" has an original to open: the sender's HTML, which the text
        // above is a reading of.
        has_html: !d.body_html.is_empty(),
    };
    let attachments = d
        .attachments
        .iter()
        .map(|a| Attachment {
            name: a.filename.clone(),
            size_text: human_size(a.size_bytes),
            mime_type: a.mime_type.clone(),
            // There is no download path on this wire, so nothing is ever downloaded and the chip
            // says so by staying in its undownloaded shape. See the note in `email.slint`.
            is_downloaded: false,
        })
        .collect();
    let thread = d
        .thread_messages
        .iter()
        .map(|t| ThreadEntry {
            from_name: t.from.clone(),
            from_addr: t.from.clone(),
            date_text: t.date.clone(),
            body: t.snippet.clone(),
            is_collapsed: true,
        })
        .collect();
    Open {
        id: id.to_string(),
        folder: folder.to_string(),
        detail,
        attachments,
        thread,
        html: (!d.body_html.is_empty()).then(|| d.body_html.clone()),
    }
}

pub fn human_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{} KB", bytes / 1024)
    } else {
        format!("{bytes} B")
    }
}

/// One folder of the sidebar.
pub fn folder(f: &EmailFolder, selected: bool) -> FolderRow {
    let lower = f.name.to_lowercase();
    let icon = match lower.as_str() {
        "inbox" => "\u{1F4E5}",
        "sent" | "sent mail" | "[gmail]/sent mail" => "\u{1F4E4}",
        "drafts" | "[gmail]/drafts" => "\u{1F4DD}",
        "trash" | "[gmail]/trash" => "\u{1F5D1}\u{FE0F}",
        "spam" | "[gmail]/spam" | "junk" => "\u{26A0}\u{FE0F}",
        "starred" | "[gmail]/starred" => "\u{2B50}",
        "archive" | "[gmail]/all mail" => "\u{1F4E6}",
        _ => "\u{1F4C1}",
    };
    let kind = match lower.as_str() {
        "inbox" => "inbox",
        s if s.contains("sent") => "sent",
        s if s.contains("draft") => "drafts",
        s if s.contains("trash") => "trash",
        s if s.contains("spam") || s.contains("junk") => "spam",
        s if s.contains("starred") => "starred",
        s if s.contains("archive") || s.contains("all mail") => "archive",
        _ => "custom",
    };
    // A folder the server did not count has no numbers, not two zeros: the row keeps the fact
    // and the reason, so the sidebar can show a mark and the header can say why (#131).
    FolderRow {
        name: f.name.clone(),
        label: words::folder_label(&f.name),
        icon: icon.to_string(),
        kind: kind.to_string(),
        counts: f.counts.map(Into::into),
        reason: f.reason.clone().unwrap_or_default(),
        selected,
    }
}

/// Put "Re: " (or "Fwd: ") on a subject without stacking it up.
///
/// Replying to a reply to a reply should not produce "Re: Re: Re: lunch".
pub fn prefixed(prefix: &str, subject: &str) -> String {
    let s = subject.trim();
    if s.to_lowercase().starts_with(&prefix.trim().to_lowercase()) {
        return s.to_string();
    }
    format!("{prefix}{s}")
}

/// The composer as a reply to `d`.
///
/// `all` decides whether the other recipients come along. The quote block is the convention
/// every mail client shares, which is the point: a reply from this app has to look like a reply
/// in the client the other person is using.
pub fn reply(d: &Detail, all: bool) -> Draft {
    Draft {
        to: d.from_addr.clone(),
        cc: if all { d.cc_addr.clone() } else { String::new() },
        bcc: String::new(),
        subject: prefixed("Re: ", &d.subject),
        body: format!(
            "\n\nOn {}, {} wrote:\n{}",
            d.date_text,
            d.from_name,
            d.body.lines().map(|l| format!("> {l}")).collect::<Vec<_>>().join("\n")
        ),
    }
}

/// The composer as a forward of `d`.
pub fn forward(d: &Detail) -> Draft {
    Draft {
        to: String::new(),
        cc: String::new(),
        bcc: String::new(),
        subject: prefixed("Fwd: ", &d.subject),
        body: format!(
            "\n\n---------- Forwarded message ----------\nFrom: {} <{}>\nSubject: {}\nDate: {}\n\n{}",
            d.from_name, d.from_addr, d.subject, d.date_text, d.body
        ),
    }
}
