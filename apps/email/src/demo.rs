//! Design fixture: `YANTRIK_EMAIL_DEMO=1` fills the three panes with realistic sample mail so the
//! UI can be judged (and screenshotted) without an account or the email service running.

use yantrik_ipc_contracts::email::EmailAccountSummary;

use crate::model::{Attachment, Detail, FolderRow, ListItem, Model, Open, Row, ThreadEntry};
use crate::state::{FolderCounts, MailState};

/// Whether this run is the fixture. Never under `cargo test`.
pub fn asked() -> bool {
    !cfg!(test) && std::env::var_os("YANTRIK_EMAIL_DEMO").is_some()
}

const BODY: &str = "Hi Alex,

Went through the observation-tier design end to end. The split between the AT-SPI event stream, the compositor damage hints and the on-demand vision tier is the right shape — most of what an agent needs is in tier one, and the model only pays for pixels when something is opaque.

Two questions before I sign off:

1. Commit gate. When the browser tier proposes an action on an indexed element, who owns the timeout? If the page re-renders between index and commit, the index is stale — do we re-index or reject?

2. Per-app scopes. The scoped-control token is minted by the shell, but revocation seems to happen only on app exit. A long-running terminal keeps its grant forever.

Small thing: the world model's epistemic states read well. \"Believed\" vs \"observed\" is exactly the distinction the planner needs.

— Priya";

pub fn populate(m: &mut Model) {
    m.state = MailState::Ready {
        account: EmailAccountSummary {
            id: "demo".into(),
            email: "you@example.com".into(),
            display_name: "Demo".into(),
            provider: "demo".into(),
            imap_server: "localhost".into(),
            imap_port: 993,
            smtp_server: "localhost".into(),
            smtp_port: 587,
            uses_oauth: false,
        },
        config_path: "(design fixture)".into(),
        secrets_are_plaintext: false,
        google: Default::default(),
    };
    m.looked = true;
    m.loading = false;
    m.account_name = "you@example.com".into();
    m.sync_status = "Synced 2 min ago".into();

    // `Some` counts are the server's; `None` is a folder it did not count, which the sidebar
    // marks rather than badges with zero.
    let folders = [
        ("Inbox", "inbox", Some((3, 128))), ("Starred", "starred", Some((0, 9))),
        ("Sent", "sent", Some((0, 341))), ("Drafts", "drafts", Some((0, 2))),
        ("Archive", "archive", Some((0, 2210))), ("Spam", "spam", Some((12, 12))),
        ("Trash", "trash", Some((0, 40))), ("Receipts", "custom", None),
    ];
    m.folders = folders
        .iter()
        .enumerate()
        .map(|(i, (name, kind, counts))| FolderRow {
            name: (*name).into(),
            label: (*name).into(),
            icon: String::new(),
            kind: (*kind).into(),
            counts: counts.map(|(unread, total)| FolderCounts { unread, total }),
            reason: if counts.is_some() { String::new() } else { "the mail server did not report it: No Response: STATUS timed out".into() },
            selected: i == 0,
        })
        .collect();

    let rows = [
        ("Priya R.", "priya@example.com", "Kernel perception API — review notes", "Left comments on the observation tiers doc. The event-driven path looks right; two questions on the commit gate…", "09:42", false, true, true, 4, "P", (0x4E, 0x79, 0xA7)),
        ("GitHub", "noreply@github.com", "[yantrik-os] CI passed on rebase/yantrikdb-0.10", "All 212 checks passed. Build time 6m 12s (fast profile).", "09:10", false, false, false, 1, "G", (0x59, 0xA1, 0x4F)),
        ("Ananya S.", "ananya@example.com", "Re: Launcher grid — category rail", "Agree on 5 columns. Can we keep the search box pinned when the grid scrolls?", "Yesterday", false, false, false, 3, "A", (0xE1, 0x57, 0x59)),
        ("Example Hosting", "billing@example.net", "Invoice 0000-0001 for September", "Your invoice for your server is attached. Amount due: €10.00.", "Yesterday", true, false, true, 1, "H", (0xF2, 0x8E, 0x2B)),
        ("Marcus W.", "marcus@example.org", "Talk proposal: AI-native desktops", "Would you be up for a 25-minute slot in the systems track? Abstract deadline is the 19th.", "Tue", true, true, false, 2, "M", (0x76, 0xB7, 0xB2)),
        ("Slint Weekly", "weekly@example.org", "1.17: software renderer partial repaints", "This release brings dirty-region repaints to the software renderer and a new Path API…", "Mon", true, false, false, 1, "S", (0xB0, 0x7A, 0xA1)),
        ("Ravi K.", "ravi@example.com", "Fonts landed on the design tokens crate", "Barlow at 400/500/600 and JetBrains Mono 400/500 are embedded now; no more DejaVu fallback.", "Mon", true, false, false, 1, "R", (0x9C, 0x75, 0x5F)),
    ];
    m.rows = rows
        .iter()
        .enumerate()
        .map(|(i, r)| Row {
            id: format!("demo-{i}"),
            folder: "INBOX".into(),
            item: ListItem {
                from_name: r.0.into(), from_addr: r.1.into(), subject: r.2.into(), preview: r.3.into(),
                date_text: r.4.into(), is_read: r.5, is_flagged: r.6, is_selected: i == 0, has_attachment: r.7,
                thread_count: r.8, thread_id: format!("t{i}"), avatar_initial: r.9.into(), avatar: r.10,
            },
        })
        .collect();

    let thread = |from_name: &str, from_addr: &str, date: &str, body: &str, collapsed: bool| ThreadEntry {
        from_name: from_name.into(),
        from_addr: from_addr.into(),
        date_text: date.into(),
        body: body.into(),
        is_collapsed: collapsed,
    };
    m.open = Some(Open {
        id: "demo-0".into(),
        folder: "INBOX".into(),
        detail: Detail {
            from_name: "Priya R.".into(),
            from_addr: "priya@example.com".into(),
            from_initial: "P".into(),
            avatar: (0x4E, 0x79, 0xA7),
            to_addr: "you@example.com".into(),
            cc_addr: "ananya@example.com".into(),
            subject: "Kernel perception API — review notes".into(),
            date_text: "Today, 09:42".into(),
            body: BODY.into(),
            ai_summary: String::new(),
            is_flagged: true,
            is_read: true,
            has_attachment: true,
            attachment_names: "perception-tiers-v3.pdf, commit-gate.png".into(),
            thread_count: 4,
            // The demo body is plain text, so there is no HTML original to open.
            has_html: false,
        },
        attachments: vec![
            Attachment { name: "perception-tiers-v3.pdf".into(), size_text: "412 KB".into(), mime_type: "application/pdf".into(), is_downloaded: true },
            Attachment { name: "commit-gate.png".into(), size_text: "88 KB".into(), mime_type: "image/png".into(), is_downloaded: false },
        ],
        thread: vec![
            thread("Alex", "you@example.com", "Mon, 14:05", "Sharing v3 of the perception tiers doc. Main change: tier two is event-driven now.", true),
            thread("Ananya S.", "ananya@example.com", "Mon, 16:40", "The damage-hint idea is neat. Does labwc expose that today or do we need a protocol extension?", true),
            thread("Alex", "you@example.com", "Tue, 08:12", "wlr-screencopy gives us damage regions per frame; no extension needed.", true),
            thread("Priya R.", "priya@example.com", "Today, 09:42", BODY, false),
        ],
        html: None,
    });
}
