//! Email under test: headless through yantrik-testing, against a scripted mail service
//! (`backend::fake`) and a fake Mind, with its files under the test's own temporary root. Jobs
//! run when a test says, and the clock moves when a test moves it.

use std::sync::Arc;

use serde_json::{json, Value};
use yantrik_app::mind::fake::FakeMind;
use yantrik_app::{App, Paths, Services};
use yantrik_testing::TestApp;

use crate::backend::fake::{self, FakeMail};
use crate::Email;

pub(crate) fn start_with(mail: &FakeMail, mind: FakeMind) -> (TestApp<Email>, Arc<FakeMind>) {
    start_at(Paths::temporary(Email::ID), mail, mind)
}

pub(crate) fn start_at(paths: Paths, mail: &FakeMail, mind: FakeMind) -> (TestApp<Email>, Arc<FakeMind>) {
    fake::install(&paths, mail);
    let mind = Arc::new(mind);
    let app = TestApp::<Email>::start_at_with(paths, Vec::new(), mind.clone(), Services::fakes());
    (app, mind)
}

/// Started, with the first look landed.
pub(crate) fn started(mail: &FakeMail) -> TestApp<Email> {
    let app = start_with(mail, FakeMind::new()).0;
    quiet(&app);
    app
}

/// Runs jobs until none is left: a landing may start the next job (a mark after an open, the
/// next draft write).
pub(crate) fn quiet(app: &TestApp<Email>) {
    const ROUNDS: usize = 16;
    for _ in 0..ROUNDS {
        if app.jobs_waiting() == 0 {
            return;
        }
        app.run_jobs();
    }
    panic!("jobs still waiting after {ROUNDS} rounds");
}

/// One call through the real dispatch, answered once its work has landed.
pub(crate) fn act(app: &TestApp<Email>, name: &str, args: Value) -> Result<Value, String> {
    let pending = app.act(name, args)?;
    quiet(app);
    pending.settle()
}

pub(crate) fn state(app: &TestApp<Email>) -> Value {
    app.describe()["state"].clone()
}

pub(crate) fn summary(app: &TestApp<Email>) -> String {
    app.describe()["summary"].as_str().unwrap_or_default().to_string()
}

pub(crate) fn notice(app: &TestApp<Email>) -> String {
    app.model(|m| m.notice.clone())
}

/// One listed message, as the service answers `list_messages` or `search` with it.
pub(crate) fn message(id: &str, folder: &str, subject: &str, from: &str, read: bool, starred: bool) -> Value {
    json!({
        "id": id, "from": from, "to": ["you@example.com"], "subject": subject,
        "snippet": format!("about {subject}"), "date": "2026-10-20 10:35", "is_read": read,
        "is_starred": starred, "has_attachments": false, "folder": folder, "thread_id": null,
    })
}

/// Three messages in INBOX: two unread, one starred.
pub(crate) fn inbox() -> Value {
    json!([
        message("101", "INBOX", "Lunch on Friday", "Priya R. <priya@example.com>", false, false),
        message("102", "INBOX", "Invoice R0093", "Billing <billing@example.net>", true, true),
        message("103", "INBOX", "Kernel review notes", "Ananya S. <ananya@example.com>", false, false),
    ])
}

/// The message the service answers `get_message` with.
pub(crate) fn detail(id: &str, subject: &str, from: &str, read: bool, html: &str) -> Value {
    json!({
        "id": id, "from": from, "to": ["you@example.com"], "cc": ["team@example.com"], "bcc": [],
        "subject": subject, "body_html": html, "body_text": format!("The text of {subject}.\nSecond line."),
        "date": "2026-10-20 10:35", "attachments": [{ "filename": "notes.pdf", "mime_type": "application/pdf", "size_bytes": 2048 }],
        "thread_messages": [
            { "id": "t1", "from": "Priya R.", "date": "Mon", "snippet": "first" },
            { "id": "t2", "from": "You", "date": "Tue", "snippet": "second" },
        ],
        "is_read": read, "is_starred": false,
    })
}

/// A ready account with [`inbox`] listed: a search finds all three, a message opens as the
/// first one, already read, and every change is agreed to.
pub(crate) fn with_inbox() -> FakeMail {
    FakeMail::ready()
        .answering("email.list_messages", inbox())
        .answering("email.search", inbox())
        .answering("email.get_message", detail("101", "Lunch on Friday", "Priya R. <priya@example.com>", true, ""))
        .answering("email.mark_read", json!({}))
        .answering("email.mark_starred", json!({}))
}

/// What the window shows of a list, in order: each row's id and subject.
pub(crate) fn listed(app: &TestApp<Email>) -> Vec<(i32, String)> {
    use slint::Model as _;
    app.ui(|ui| {
        let list = ui.get_email_list();
        (0..list.row_count()).filter_map(|i| list.row_data(i)).map(|r| (r.id, r.subject.to_string())).collect()
    })
}

mod account_file;
mod ai;
mod compose;
mod logout;
mod mailbox;
mod markup;
mod message;
mod parity;
mod private_mode;
mod robust;
mod setup;
mod unsent;
