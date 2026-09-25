//! Yantrik Email — standalone app binary.
//!
//! Talks to `email-service` over JSON-RPC, and starts it first: the service is registered
//! `autostart: false`, nothing ever started it, and every call this app made failed at connect on
//! every machine. The app read that failure as "no account configured" and drew the onboarding
//! form, which is why an audit of this machine concluded it had no mail account. It may never
//! have been about an account at all. `design/email-2026-09-20.md` has the whole of it.
//!
//! Three things follow from that, and they are the shape of this file:
//!
//! * every service call goes through [`yantrik_app_runtime::service::client`], which asks the
//!   shell to start the service and clears the transport's circuit breaker;
//! * every wrapper returns `Result<T, String>`, because an `Option` is where the reason was lost;
//! * the app distinguishes three states — the service could not be reached, the service is up and
//!   no account is configured, an account exists — and says which, on screen and in `describe`.

mod state;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use yantrik_app_runtime::prelude::*;
use yantrik_ipc_contracts::email::{
    method, AccountSettings, AccountsResult, EmailAccountSummary, EmailDetail, EmailFolder,
    EmailSummary, OAuthBeginResult, OAuthStatus, TestAccountResult,
};

use state::{Counted, Draft, FolderCounts, GoogleOutcome, MailState, MessageRow, Triage};

slint::include_modules!();

/// How long a call that crosses the internet may take.
///
/// `SyncRpcClient`'s default is two seconds, which is right for a local service answering out of
/// memory and wrong for every method here: an IMAP round trip leaves the machine. Under the
/// default, a mailbox that was working perfectly reported itself unreachable.
///
/// Ten seconds is a compromise and worth naming as one. The control surface gives an action three
/// seconds on the UI thread before telling its caller the app did not answer, so a genuinely slow
/// mailbox will produce that message while the work carries on — which is true, and better than
/// the alternative, which is calling a working server dead. The real fix is for the mailbox to
/// stop living on the UI thread at all; the sync below already does, and the rest is more than
/// this pass. Nothing here is on the surface's fast path: `describe` reads Slint properties.
const MAIL_BUDGET: Duration = Duration::from_secs(10);

/// Put "Re: " (or "Fwd: ") on a subject without stacking it up.
///
/// Replying to a reply to a reply should not produce "Re: Re: Re: lunch".
fn prefixed(prefix: &str, subject: &str) -> SharedString {
    let s = subject.trim();
    if s.to_lowercase().starts_with(&prefix.trim().to_lowercase()) {
        return s.into();
    }
    format!("{prefix}{s}").into()
}

/// Open the composer as a reply to whatever is on screen.
///
/// `all` decides whether the other recipients come along. The quote block is the convention
/// every mail client shares, which is the point: a reply from this app has to look like a reply
/// in the client the other person is using.
fn start_reply(ui: &EmailApp, all: bool) {
    let d = ui.get_email_detail();
    if d.id <= 0 {
        return;
    }
    ui.set_is_composing(true);
    ui.set_compose_to(d.from_addr.clone());
    ui.set_compose_cc(if all { d.cc_addr.clone() } else { SharedString::default() });
    ui.set_compose_bcc(SharedString::default());
    ui.set_compose_subject(prefixed("Re: ", &d.subject));
    ui.set_compose_body(
        format!(
            "\n\nOn {}, {} wrote:\n{}",
            d.date_text,
            d.from_name,
            d.body
                .lines()
                .map(|l| format!("> {l}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
        .into(),
    );
}

/// The two companion actions that read the open message.
#[derive(Clone, Copy)]
enum MailAi {
    Summarize,
    SuggestReply,
}

fn wire_mail_ai(app: &EmailApp, which: MailAi) {
    let weak = app.as_weak();
    let handler = move || {
        let Some(ui) = weak.upgrade() else { return };
        let d = ui.get_email_detail();
        if d.id <= 0 || d.body.is_empty() {
            say(&ui, "There is no message open to read.");
            return;
        }
        let prompt = match which {
            MailAi::Summarize => format!(
                "Summarise this email in at most three short lines. Say what is being asked of \
                 me, if anything. Use only what it says.\n\nFrom: {}\nSubject: {}\n\n{}",
                d.from_name, d.subject, d.body
            ),
            MailAi::SuggestReply => format!(
                "Draft a short reply to this email. Match its register. Reply with the body \
                 only.\n\nFrom: {}\nSubject: {}\n\n{}",
                d.from_name, d.subject, d.body
            ),
        };
        ui.set_ai_is_working(true);
        let back = ui.as_weak();
        std::thread::spawn(move || {
            let outcome = companion::ask(&prompt);
            let _ = back.upgrade_in_event_loop(move |ui| {
                ui.set_ai_is_working(false);
                match outcome {
                    Ok(text) => match which {
                        // The summary belongs on the message it is about.
                        MailAi::Summarize => {
                            let mut d = ui.get_email_detail();
                            d.ai_summary = text.into();
                            ui.set_email_detail(d);
                            clear_notice(&ui);
                        }
                        // A suggested reply belongs in the composer, unsent.
                        MailAi::SuggestReply => {
                            start_reply(&ui, false);
                            ui.set_compose_body(text.into());
                            clear_notice(&ui);
                        }
                    },
                    // Said out loud, not logged. A model that could not be reached left the card
                    // exactly as it was and the person waiting at it with no idea why.
                    Err(e) => say(&ui, e.to_string()),
                }
            });
        });
    };
    match which {
        MailAi::Summarize => app.on_summarize_email(handler),
        MailAi::SuggestReply => app.on_ai_reply_suggest(handler),
    }
}

fn main() {
    init_tracing("yantrik-email");

    // One window per app: a second launch defers to the running one (the shell focuses it).
    let Some(_instance) = instance::claim("email") else { return };

    let app = EmailApp::new().unwrap();
    // The window's title bar is the app's own (#256).
    yantrik_app_runtime::window_chrome!(app);

    // Same dark/accent choice as the shell, read from the shell's settings file.
    let theme = theme::load();
    app.global::<ThemeMode>().set_dark(theme.dark);
    app.global::<AccentPreset>().set_index(theme.accent_index);

    wire(&app);
    run_until_closed(&app, "yantrik-email");
}

// ── Saying what went wrong ───────────────────────────────────────────

/// Put a failure where the person can read it. `describe` publishes the same string.
fn say(ui: &EmailApp, text: impl Into<String>) {
    let text = text.into();
    tracing::warn!(notice = %text, "email");
    ui.set_notice(text.into());
}

fn clear_notice(ui: &EmailApp) {
    ui.set_notice(SharedString::new());
}

// ── Service wrappers ─────────────────────────────────────────────────
//
// Every one of these returned `Option` and every call site wrote `let _ =` or `if ….is_some()`,
// which is how a mail server's refusal became silence on a screen. They return `Result` now, and
// the reason is the thing the caller is obliged to do something with.

/// A client for the mail service, started first if it was not running.
fn mail_client() -> Result<SyncRpcClient, String> {
    service::client("email").map(|c| c.with_timeout(MAIL_BUDGET))
}

fn call(method: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
    mail_client()?.call(method, params).map_err(|e| e.message)
}

fn call_typed<T: serde::de::DeserializeOwned>(
    method: &str,
    params: serde_json::Value,
) -> Result<T, String> {
    let value = call(method, params)?;
    serde_json::from_value(value)
        .map_err(|e| format!("the mail service answered with something this app cannot read: {e}"))
}

/// What accounts are configured. Answers without touching a mail server, which is the whole
/// reason it exists: it is the one question whose failure means the service is not there.
fn accounts_via_service() -> Result<AccountsResult, String> {
    call_typed(method::ACCOUNTS, serde_json::json!({}))
}

fn list_folders_via_service(account: &str) -> Result<Vec<EmailFolder>, String> {
    call_typed(method::LIST_FOLDERS, serde_json::json!({ "account_id": account }))
}

fn list_messages_via_service(
    account: &str,
    folder: &str,
    page: u32,
) -> Result<Vec<EmailSummary>, String> {
    call_typed(
        method::LIST_MESSAGES,
        serde_json::json!({ "account_id": account, "folder": folder, "page": page.max(1) }),
    )
}

/// Read one message out of `folder` — the folder its row was listed from. UIDs are per-mailbox,
/// so the folder is part of naming the message, not context the service can assume (#275).
fn get_message_via_service(
    account: &str,
    folder: &str,
    message_id: &str,
) -> Result<EmailDetail, String> {
    call_typed(
        method::GET_MESSAGE,
        serde_json::json!({ "account_id": account, "folder": folder, "message_id": message_id }),
    )
}

fn send_message_via_service(
    account: &str,
    to: &str,
    cc: &str,
    bcc: &str,
    subject: &str,
    body: &str,
) -> Result<(), String> {
    let addresses = |line: &str| -> Vec<String> {
        line.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
    };
    call(
        method::SEND_MESSAGE,
        serde_json::json!({
            "account_id": account,
            "to": addresses(to),
            "cc": addresses(cc),
            "bcc": addresses(bcc),
            "subject": subject,
            "body": body,
        }),
    )
    .map(|_| ())
}

/// Search `folder` — the one on screen. Search ran in INBOX whichever folder was open, so a
/// search from Spam answered with mail nobody was looking at (#288).
fn search_via_service(
    account: &str,
    folder: &str,
    query: &str,
) -> Result<Vec<EmailSummary>, String> {
    call_typed(
        method::SEARCH,
        serde_json::json!({ "account_id": account, "folder": folder, "query": query }),
    )
}

/// Flag one message in `folder`, for [`get_message_via_service`]'s reason: the app marks a
/// message read right after opening it, and flagging INBOX's UID instead would touch a
/// stranger's mail.
fn mark_read_via_service(
    account: &str,
    folder: &str,
    message_id: &str,
    read: bool,
) -> Result<(), String> {
    call(
        method::MARK_READ,
        serde_json::json!({
            "account_id": account,
            "folder": folder,
            "message_id": message_id,
            "read": read,
        }),
    )
    .map(|_| ())
}

/// Star one message in `folder`, for [`mark_read_via_service`]'s reason: starring ran in
/// INBOX whatever folder the row was listed from, and moved the star on a stranger's message
/// (#288).
fn mark_starred_via_service(
    account: &str,
    folder: &str,
    message_id: &str,
    starred: bool,
) -> Result<(), String> {
    call(
        method::MARK_STARRED,
        serde_json::json!({
            "account_id": account,
            "folder": folder,
            "message_id": message_id,
            "starred": starred,
        }),
    )
    .map(|_| ())
}

/// Delete one message from `folder` — always the row's own, never omitted. This cannot be
/// undone, and a UID with no folder beside it can destroy a different message in INBOX
/// (#288).
fn delete_message_via_service(
    account: &str,
    folder: &str,
    message_id: &str,
) -> Result<(), String> {
    call(
        method::DELETE_MESSAGE,
        serde_json::json!({ "account_id": account, "folder": folder, "message_id": message_id }),
    )
    .map(|_| ())
}

/// Move one message out of `folder` into `target_folder`. The source folder is never omitted
/// either, for [`delete_message_via_service`]'s reason (#288).
fn move_message_via_service(
    account: &str,
    folder: &str,
    message_id: &str,
    target_folder: &str,
) -> Result<(), String> {
    call(
        method::MOVE_MESSAGE,
        serde_json::json!({
            "account_id": account,
            "folder": folder,
            "message_id": message_id,
            "target_folder": target_folder,
        }),
    )
    .map(|_| ())
}

fn test_account_via_service(settings: &AccountSettings) -> Result<TestAccountResult, String> {
    let params = serde_json::to_value(settings).map_err(|e| e.to_string())?;
    call_typed(method::TEST_ACCOUNT, params)
}

fn save_account_via_service(settings: &AccountSettings) -> Result<EmailAccountSummary, String> {
    let params = serde_json::to_value(settings).map_err(|e| e.to_string())?;
    call_typed(method::SAVE_ACCOUNT, params)
}

// ── The Google sign-in ───────────────────────────────────────────────
//
// Three calls, because there is a person in the middle of it. `begin` answers in microseconds
// with somewhere to send them; the mail service sits on a loopback socket meanwhile; `status` is
// asked on a worker thread until it stops saying "waiting". A single blocking call would hold
// this app's ten-second budget for the minutes a person takes to choose a Google account, and
// freeze the window for all of them.
//
// No token reaches this process. What comes back from a finished sign-in is the same account
// summary `save_account` answers with — an address and the servers behind it.

fn oauth_begin_via_service() -> Result<OAuthBeginResult, String> {
    call_typed(method::OAUTH_BEGIN, serde_json::json!({ "provider": "google" }))
}

fn oauth_status_via_service(flow_id: &str) -> Result<OAuthStatus, String> {
    call_typed(method::OAUTH_STATUS, serde_json::json!({ "flow_id": flow_id }))
}

fn oauth_cancel_via_service(flow_id: &str) -> Result<(), String> {
    call(method::OAUTH_CANCEL, serde_json::json!({ "flow_id": flow_id })).map(|_| ())
}

/// Put the consent page in front of the person.
///
/// Detached, and bounded. `xdg-open` normally hands the URL to a handler and exits immediately;
/// on a machine with no handler it exits non-zero, which is the case that has to be *said* rather
/// than swallowed — a click that silently opens nothing is the report this whole change came
/// from. So: spawn, wait a moment and a half for an exit code, and treat "still running" as a
/// browser that is starting. The child is reaped on a thread either way, because a zombie for
/// every sign-in is a small leak in a process that stays open all day.
fn open_in_browser(url: &str) -> Result<(), String> {
    use std::process::{Command, Stdio};

    let mut child = Command::new("xdg-open")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("xdg-open could not be run: {e}"))?;

    let deadline = std::time::Instant::now() + Duration::from_millis(1500);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => {
                return Err(format!(
                    "xdg-open exited {} \u{2014} this machine has no handler for a web address",
                    status.code().map(|c| c.to_string()).unwrap_or_else(|| "on a signal".into())
                ))
            }
            Ok(None) if std::time::Instant::now() >= deadline => {
                // Still running after a second and a half: it has a handler and is waiting on the
                // browser it started. That is a success from here.
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                return Ok(());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(format!("xdg-open could not be waited for: {e}")),
        }
    }
}

// ── Conversion helpers ───────────────────────────────────────────────

fn summary_to_list_item(s: &EmailSummary, idx: usize) -> EmailListItem {
    let from_name = s.from.split('<').next().unwrap_or(&s.from).trim().to_string();
    let initial = from_name.chars().next().unwrap_or('?').to_uppercase().to_string();
    let colors = [
        slint::Color::from_rgb_u8(0x4E, 0x79, 0xA7),
        slint::Color::from_rgb_u8(0xF2, 0x8E, 0x2C),
        slint::Color::from_rgb_u8(0xE1, 0x57, 0x59),
        slint::Color::from_rgb_u8(0x76, 0xB7, 0xB2),
        slint::Color::from_rgb_u8(0x59, 0xA1, 0x4F),
    ];
    EmailListItem {
        id: idx as i32,
        from_name: from_name.into(),
        from_addr: s.from.clone().into(),
        subject: s.subject.clone().into(),
        preview: s.snippet.clone().into(),
        date_text: s.date.clone().into(),
        is_read: s.is_read,
        is_flagged: s.is_starred,
        is_selected: false,
        has_attachment: s.has_attachments,
        thread_count: 0,
        thread_id: s.thread_id.clone().unwrap_or_default().into(),
        avatar_initial: initial.into(),
        avatar_color: colors[idx % colors.len()],
    }
}

fn detail_to_ui(d: &EmailDetail) -> EmailDetailData {
    let from_name = d.from.split('<').next().unwrap_or(&d.from).trim().to_string();
    let initial = from_name.chars().next().unwrap_or('?').to_uppercase().to_string();
    let body = if d.body_text.is_empty() { &d.body_html } else { &d.body_text };
    EmailDetailData {
        // Not 0. The detail pane draws on `id > 0`, and every field below it — including
        // `describe`'s account of the open message — is gated on the same thing.
        id: 1,
        from_name: from_name.into(),
        from_addr: d.from.clone().into(),
        from_initial: initial.into(),
        from_avatar_color: slint::Color::from_rgb_u8(0x4E, 0x79, 0xA7),
        to_addr: d.to.join(", ").into(),
        cc_addr: d.cc.join(", ").into(),
        subject: d.subject.clone().into(),
        date_text: d.date.clone().into(),
        body: body.clone().into(),
        ai_summary: SharedString::default(),
        // Read off the message rather than assumed. These were `false` and `true` regardless of
        // what the mail server said, so the star on an open message was always hollow.
        is_flagged: d.is_starred,
        is_read: d.is_read,
        has_attachment: !d.attachments.is_empty(),
        attachment_names: d
            .attachments
            .iter()
            .map(|a| a.filename.clone())
            .collect::<Vec<_>>()
            .join(", ")
            .into(),
        thread_count: d.thread_messages.len() as i32,
        // Whether "Open original" has an original to open: the sender's HTML, which the text
        // above is a reading of.
        has_html: !d.body_html.is_empty(),
    }
}

fn attachments_to_ui(d: &EmailDetail) -> Vec<EmailAttachmentData> {
    d.attachments
        .iter()
        .map(|a| EmailAttachmentData {
            name: a.filename.clone().into(),
            size_text: human_size(a.size_bytes).into(),
            mime_type: a.mime_type.clone().into(),
            // There is no download path on this wire, so nothing is ever downloaded and the chip
            // says so by staying in its undownloaded shape. See the note in `email.slint`.
            is_downloaded: false,
        })
        .collect()
}

fn human_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{} KB", bytes / 1024)
    } else {
        format!("{bytes} B")
    }
}

fn folder_to_ui(f: &EmailFolder, selected: bool) -> EmailFolderData {
    let icon = match f.name.to_lowercase().as_str() {
        "inbox" => "\u{1F4E5}",
        "sent" | "sent mail" | "[gmail]/sent mail" => "\u{1F4E4}",
        "drafts" | "[gmail]/drafts" => "\u{1F4DD}",
        "trash" | "[gmail]/trash" => "\u{1F5D1}\u{FE0F}",
        "spam" | "[gmail]/spam" | "junk" => "\u{26A0}\u{FE0F}",
        "starred" | "[gmail]/starred" => "\u{2B50}",
        "archive" | "[gmail]/all mail" => "\u{1F4E6}",
        _ => "\u{1F4C1}",
    };
    let folder_type = match f.name.to_lowercase().as_str() {
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
    let counts = f.counts.unwrap_or_default();
    EmailFolderData {
        name: f.name.clone().into(),
        icon: icon.into(),
        unread_count: counts.unread,
        total_count: counts.total,
        counts_known: f.counts.is_some(),
        counts_reason: f.reason.clone().unwrap_or_default().into(),
        is_selected: selected,
        folder_type: folder_type.into(),
    }
}

// ── What the app is holding ──────────────────────────────────────────

/// The app's own state, beside the window's.
///
/// `MailState` is here rather than inferred from Slint properties because a `bool` cannot say
/// "not known": with the service unreachable this app has not been told whether an account
/// exists, and `has_account: false` for that is exactly the claim this whole change removes.
struct Mail {
    state: RefCell<MailState>,
    /// The folder on screen.
    folder: RefCell<String>,
    /// Every message the folder returned, before the triage tabs filter it. Held so that
    /// switching tabs is a filter over what is in hand rather than another question to the mail
    /// server. Each row is `(id, folder it was listed from, the row on screen)`: the folder
    /// travels with the row because IMAP UIDs are per-mailbox, and the same id in another
    /// mailbox is another message — everything done to a row has to ask the mailbox that
    /// listed it (#275, #288).
    all_rows: RefCell<Vec<(String, String, EmailListItem)>>,
    /// The rows actually on screen, as `(id, folder it was listed from)`, parallel to the list
    /// model.
    shown_rows: RefCell<Vec<(String, String)>>,
    triage: Cell<Triage>,
    /// True while a connection test or a save is in flight, so the two buttons cannot be pressed
    /// on top of each other.
    setting_up: Cell<bool>,
    /// True while a sync worker is running, so Refresh cannot stack.
    syncing: Cell<bool>,
    /// The Google sign-in this window is in the middle of, if any.
    ///
    /// Held so that a status answer arriving from a flow that was cancelled — or from one the
    /// person abandoned and started again — is dropped rather than applied. A poll and a Cancel
    /// race by a second at most, and the loser must not be the Cancel.
    google_flow: RefCell<Option<String>>,
    draft_path: std::path::PathBuf,
    /// The open message's HTML original, as `(id, html)`, held for the toolbar's "Open
    /// original" so the button does not go back to the mail server for what the reading pane
    /// was just given. `None` for a plain-text message, whose button is not drawn.
    open_original: RefCell<Option<(String, String)>>,
}

thread_local! {
    /// The app's state, reachable from a closure that has come back from a worker thread.
    ///
    /// An `Rc` cannot cross a thread boundary, and the outcome of a sync or of a sign-in has to
    /// be applied on the UI thread. `upgrade_in_event_loop` puts the closure on this thread,
    /// which is the only one that ever reads this.
    static MAIL: RefCell<Option<Rc<Mail>>> = const { RefCell::new(None) };
}

fn with_mail<T>(f: impl FnOnce(&Rc<Mail>) -> T) -> Option<T> {
    MAIL.with(|cell| cell.borrow().as_ref().map(f))
}

impl Mail {
    fn new() -> Self {
        Self {
            state: RefCell::new(MailState::Unreachable {
                reason: "the mail service has not been asked yet".to_string(),
            }),
            folder: RefCell::new("INBOX".to_string()),
            all_rows: RefCell::new(Vec::new()),
            shown_rows: RefCell::new(Vec::new()),
            triage: Cell::new(Triage::All),
            setting_up: Cell::new(false),
            syncing: Cell::new(false),
            google_flow: RefCell::new(None),
            draft_path: state::draft_path(),
            open_original: RefCell::new(None),
        }
    }

    fn account(&self) -> String {
        self.state.borrow().account_id()
    }

    /// The reason the service could not be used, if that is the state.
    fn unreachable_reason(&self) -> Option<String> {
        match &*self.state.borrow() {
            MailState::Unreachable { reason } => Some(reason.clone()),
            _ => None,
        }
    }

    /// The `(id, folder it was listed from)` of a row on screen.
    ///
    /// The folder belongs to the row, not to whatever folder is on display: opening a message
    /// has to SELECT the mailbox that listed it, because the same UID in another folder is
    /// another message (#275).
    fn row_at(&self, row: usize) -> Option<(String, String)> {
        self.shown_rows.borrow().get(row).cloned()
    }
}

// ── Everything the mail service is asked at once ─────────────────────

/// What one look at the mail service found.
struct Loaded {
    state: MailState,
    folders: Vec<EmailFolder>,
    messages: Vec<EmailSummary>,
    /// The account is configured and the mailbox still would not open. A different failure from
    /// the service being unreachable, and it has to be said differently.
    mailbox_error: Option<String>,
}

/// Ask the service what it has, and then — only if there is an account — open a folder.
///
/// This is the one path both the first load and the Refresh button take, so the screen cannot
/// come to believe two different things about the same machine depending on which one ran. It
/// touches no Slint: the sync runs it on a worker thread.
fn look_at_mail(folder: &str) -> Loaded {
    let state = state::decide(accounts_via_service());
    let account = state.account_id();
    if account.is_empty() {
        return Loaded { state, folders: Vec::new(), messages: Vec::new(), mailbox_error: None };
    }

    let folders = match list_folders_via_service(&account) {
        Ok(folders) => folders,
        Err(e) => {
            return Loaded {
                state,
                folders: Vec::new(),
                messages: Vec::new(),
                mailbox_error: Some(e),
            }
        }
    };
    let messages = match list_messages_via_service(&account, folder, 1) {
        Ok(messages) => messages,
        Err(e) => {
            return Loaded { state, folders, messages: Vec::new(), mailbox_error: Some(e) }
        }
    };
    Loaded { state, folders, messages, mailbox_error: None }
}

/// Put a [`Loaded`] on screen, whichever of the three states it turned out to be.
fn apply_loaded(ui: &EmailApp, mail: &Rc<Mail>, loaded: Loaded) {
    let folder = mail.folder.borrow().clone();
    *mail.state.borrow_mut() = loaded.state.clone();

    ui.set_service_state(loaded.state.service_word().into());
    ui.set_has_account(loaded.state.has_account().unwrap_or(false));
    ui.set_account_name(loaded.state.account_name().into());
    ui.set_setup_note(
        state::password_storage_note(
            loaded.state.config_path(),
            loaded.state.secrets_are_plaintext(),
        )
        .into(),
    );

    // Whether the setup screen can offer a Google sign-in at all, in the service's own words.
    // The button is not drawn unless this is true, because a "Sign in with Google" that cannot
    // work is the dead control this flow replaced — and the note is set either way, so a build
    // with no OAuth client says so instead of showing an unexplained gap.
    let google = loaded.state.google_sign_in();
    ui.set_google_available(google.available);
    ui.set_google_note(google.note.into());

    // The folder that is open, highlighted by name. `is_selected: idx == 0` was hardcoded, so
    // after switching mailboxes the sidebar still pointed at whichever folder came back first.
    let selected = loaded
        .folders
        .iter()
        .position(|f| f.name.eq_ignore_ascii_case(&folder))
        .unwrap_or(0);
    let folder_models: Vec<EmailFolderData> = loaded
        .folders
        .iter()
        .enumerate()
        .map(|(i, f)| folder_to_ui(f, i == selected))
        .collect();
    ui.set_folders(ModelRc::new(VecModel::from(folder_models)));
    ui.set_selected_folder_index(selected as i32);

    set_rows(ui, mail, &loaded.messages);

    match (&loaded.state, &loaded.mailbox_error) {
        (MailState::Ready { .. }, Some(e)) => {
            ui.set_email_sync_status("Mailbox did not open".into());
            say(ui, format!("The account is configured and the mailbox did not open: {e}"));
        }
        (MailState::Ready { .. }, None) => {
            ui.set_email_sync_status("Synced".into());
            clear_notice(ui);
        }
        (MailState::NoAccount { .. }, _) => {
            ui.set_email_sync_status("No account".into());
            clear_notice(ui);
        }
        (MailState::Unreachable { .. }, _) => {
            ui.set_email_sync_status("Service unreachable".into());
            say(ui, loaded.state.notice());
        }
    }
}

/// Put a folder's messages into the model, through the triage filter.
fn set_rows(ui: &EmailApp, mail: &Rc<Mail>, messages: &[EmailSummary]) {
    let rows: Vec<(String, String, EmailListItem)> = messages
        .iter()
        .enumerate()
        .map(|(i, s)| (s.id.clone(), s.folder.clone(), summary_to_list_item(s, i)))
        .collect();
    *mail.all_rows.borrow_mut() = rows;
    show_rows(ui, mail);
}

/// Redraw the list from what is held, applying the triage tab.
fn show_rows(ui: &EmailApp, mail: &Rc<Mail>) {
    let triage = mail.triage.get();
    let all = mail.all_rows.borrow();
    let kept: Vec<&(String, String, EmailListItem)> =
        all.iter().filter(|(_, _, it)| triage.keeps(it.is_read, it.is_flagged)).collect();

    *mail.shown_rows.borrow_mut() =
        kept.iter().map(|(id, folder, _)| (id.clone(), folder.clone())).collect();
    let items: Vec<EmailListItem> = kept.iter().map(|(_, _, it)| it.clone()).collect();

    // The counts are of the folder, not of the tab: "3 unread of 128" is about the mailbox, and
    // it would be a strange thing for pressing Unread to change.
    ui.set_email_triage_view(triage.index());
    ui.set_email_list(ModelRc::new(VecModel::from(items)));
    show_folder_counts(ui, mail);
}

/// Put the open folder's counts on the header, off the folder list.
///
/// They were counted here over the rows in hand, which is one page: "9 unread of 21" for a
/// folder the list in the sidebar — and two lines below in `describe` — said held 35. The header
/// now reads that same entry, so the two cannot disagree; when the mail server agrees to a
/// change, [`change_counts_of`] alters the entry first and this follows.
fn show_folder_counts(ui: &EmailApp, mail: &Rc<Mail>) {
    let folder = mail.folder.borrow().clone();
    let all = mail.all_rows.borrow();
    let loaded_unread = all.iter().filter(|(_, _, it)| !it.is_read).count();
    match Counted::of(&folder_records(ui), &folder, loaded_unread, all.len()) {
        Counted::Known(counts) => {
            ui.set_email_folder_unread(counts.unread);
            ui.set_email_folder_total(counts.total);
            ui.set_email_folder_counts_known(true);
            ui.set_email_folder_counts_reason(SharedString::default());
        }
        Counted::Unavailable { reason } => {
            // The two numbers mean nothing with `known` false; zero is what they hold, not
            // what they say, and nothing reads them without the flag.
            ui.set_email_folder_unread(0);
            ui.set_email_folder_total(0);
            ui.set_email_folder_counts_known(false);
            ui.set_email_folder_counts_reason(reason.into());
        }
    }
}

/// What the header holds about the open folder, read back off its properties.
fn header_counts(ui: &EmailApp) -> Counted {
    if ui.get_email_folder_counts_known() {
        Counted::Known(FolderCounts {
            unread: ui.get_email_folder_unread(),
            total: ui.get_email_folder_total(),
        })
    } else {
        Counted::Unavailable { reason: ui.get_email_folder_counts_reason().to_string() }
    }
}

/// The folder list as the wire had it, read back off the sidebar's model.
fn folder_records(ui: &EmailApp) -> Vec<EmailFolder> {
    let folders = ui.get_folders();
    (0..folders.row_count()).filter_map(|i| folders.row_data(i)).map(folder_record).collect()
}

/// One row of the sidebar as the wire had it.
fn folder_record(f: EmailFolderData) -> EmailFolder {
    if f.counts_known {
        EmailFolder::counted(f.name.to_string(), f.unread_count, f.total_count)
    } else {
        EmailFolder::uncounted(f.name.to_string(), f.counts_reason.to_string())
    }
}

/// A folder's counts as `describe` prints them: the two numbers, or `null` — never `0/0` for
/// a folder that was not counted, because that is what an empty folder prints.
fn counts_json(counts: &Counted) -> serde_json::Value {
    match counts.known() {
        Some(c) => serde_json::json!({ "unread": c.unread, "total": c.total }),
        None => serde_json::Value::Null,
    }
}

/// Change one folder's entry in the list, for the cases where the mail server has already agreed
/// — a message read, deleted or moved — and put the header right afterwards. The next sync
/// replaces the whole list with the server's count; until then this is what is known. A folder
/// that is not listed is left alone, and the header counts what is in hand. A folder that is
/// listed without counts stays without them: there is no number to take one from.
fn change_counts_of(
    ui: &EmailApp,
    mail: &Rc<Mail>,
    folder: &str,
    change: impl FnOnce(FolderCounts) -> FolderCounts,
) {
    let folders = ui.get_folders();
    let at = (0..folders.row_count()).find(|i| {
        folders.row_data(*i).map(|f| f.name.eq_ignore_ascii_case(folder)).unwrap_or(false)
    });
    if let Some((i, mut row)) = at.and_then(|i| folders.row_data(i).map(|row| (i, row))) {
        let before = if row.counts_known {
            Counted::Known(FolderCounts { unread: row.unread_count, total: row.total_count })
        } else {
            Counted::Unavailable { reason: row.counts_reason.to_string() }
        };
        if let Counted::Known(counts) = before.after(change) {
            row.unread_count = counts.unread;
            row.total_count = counts.total;
            folders.set_row_data(i, row);
        }
    }
    show_folder_counts(ui, mail);
}

/// One flag of a row in hand, read before the mail server is asked to change it. A row that is
/// not there reads as read, so nothing is later taken off a count for it.
fn row_flag(mail: &Rc<Mail>, id: &str, flag: impl Fn(&EmailListItem) -> bool) -> bool {
    mail.all_rows
        .borrow()
        .iter()
        .find(|(row_id, _, _)| row_id == id)
        .map(|(_, _, it)| flag(it))
        .unwrap_or(true)
}

/// Open a folder, reporting what happened.
fn load_folder(ui: &EmailApp, mail: &Rc<Mail>, folder: &str) -> Result<usize, String> {
    let account = mail.account();
    if account.is_empty() {
        return Err(match mail.unreachable_reason() {
            Some(reason) => format!("the mail service could not be reached: {reason}"),
            None => "no email account is configured".to_string(),
        });
    }
    ui.set_is_loading(true);
    let outcome = list_messages_via_service(&account, folder, 1);
    ui.set_is_loading(false);

    match outcome {
        Ok(messages) => {
            *mail.folder.borrow_mut() = folder.to_string();
            let count = messages.len();
            set_rows(ui, mail, &messages);
            ui.set_email_sync_status("Synced".into());
            clear_notice(ui);
            Ok(count)
        }
        Err(e) => {
            ui.set_email_sync_status("Mailbox did not open".into());
            say(ui, format!("Could not open {folder}: {e}"));
            Err(e)
        }
    }
}

// ── The things a button and an action both do ────────────────────────
//
// One path each, returning what happened, so an action cannot answer something the button would
// not have done. `mark_read`, `flag`, `delete`, `archive`, `search` and `sync` all used to have
// two halves that could disagree: the callback discarded the result and the action answered
// success regardless of it.

/// Open a message and mark it read, reporting the flags the mail server has afterwards.
fn open_message(ui: &EmailApp, mail: &Rc<Mail>, row: usize) -> Result<EmailDetail, String> {
    let account = mail.account();
    let (id, row_folder) =
        mail.row_at(row).ok_or_else(|| format!("there is no message at row {}", row + 1))?;

    // From the folder the row was listed from: the same UID read out of INBOX was a different
    // message, which is what opening mail outside INBOX used to show (#275).
    let detail = get_message_via_service(&account, &row_folder, &id)
        .map_err(|e| format!("Could not open that message: {e}"))?;

    // The HTML original, kept for the toolbar's "Open original".
    *mail.open_original.borrow_mut() = if detail.body_html.is_empty() {
        None
    } else {
        Some((id.clone(), detail.body_html.clone()))
    };

    ui.set_email_detail(detail_to_ui(&detail));
    ui.set_email_attachments(ModelRc::new(VecModel::from(attachments_to_ui(&detail))));
    let thread: Vec<EmailThreadMessage> = detail
        .thread_messages
        .iter()
        .map(|t| EmailThreadMessage {
            id: 0,
            from_name: t.from.clone().into(),
            from_addr: t.from.clone().into(),
            date_text: t.date.clone().into(),
            body: t.snippet.clone().into(),
            is_collapsed: true,
        })
        .collect();
    ui.set_email_thread_messages(ModelRc::new(VecModel::from(thread)));

    // Reading a message marks it read on the mail server. `let _ =` here meant a failure to do
    // that was invisible, and the row kept its unread dot with no explanation.
    if !detail.is_read {
        // The flag goes to the mailbox the message is in, and the unread count that moves is
        // that mailbox's — the same reason the fetch above names its folder.
        match mark_read_via_service(&account, &row_folder, &id, true) {
            Ok(()) => {
                mark_row_locally(mail, &id, |it| it.is_read = true);
                change_counts_of(ui, mail, &row_folder, |c| c.after_read_change(false, true));
                show_rows(ui, mail);
                clear_notice(ui);
            }
            Err(e) => say(ui, format!("Opened the message; could not mark it read: {e}")),
        }
    } else {
        clear_notice(ui);
    }
    Ok(detail)
}

/// Mark a row read or unread and report what the mail server says afterwards.
///
/// The folder is listed again rather than the row updated by hand, because the answer has to be
/// an observation. `mark_read` answered `{"marked": row}` whether the call succeeded, failed or
/// was never made, which is the same shape as the calendar's fabricated `add_event`.
fn set_read(ui: &EmailApp, mail: &Rc<Mail>, row: usize, read: bool) -> Result<bool, String> {
    let account = mail.account();
    let (id, row_folder) =
        mail.row_at(row).ok_or_else(|| format!("there is no message at row {}", row + 1))?;
    let was_read = row_flag(mail, &id, |it| it.is_read);
    mark_read_via_service(&account, &row_folder, &id, read).map_err(|e| {
        let text = format!("Could not mark that message read: {e}");
        say(ui, text.clone());
        text
    })?;
    let observed = observe_flag(ui, mail, &id, |it| it.is_read)?;
    change_counts_of(ui, mail, &row_folder, |c| c.after_read_change(was_read, observed));
    clear_notice(ui);
    Ok(observed)
}

fn set_flagged(ui: &EmailApp, mail: &Rc<Mail>, row: usize, flagged: bool) -> Result<bool, String> {
    let account = mail.account();
    let (id, row_folder) =
        mail.row_at(row).ok_or_else(|| format!("there is no message at row {}", row + 1))?;
    // In the mailbox the row was listed from: the same UID starred in INBOX was a different
    // message's star (#288).
    mark_starred_via_service(&account, &row_folder, &id, flagged).map_err(|e| {
        let text = format!("Could not flag that message: {e}");
        say(ui, text.clone());
        text
    })?;
    let observed = observe_flag(ui, mail, &id, |it| it.is_flagged)?;
    clear_notice(ui);
    Ok(observed)
}

/// List the folder again and read one message's flag back off what came.
fn observe_flag(
    ui: &EmailApp,
    mail: &Rc<Mail>,
    id: &str,
    read: impl Fn(&EmailListItem) -> bool,
) -> Result<bool, String> {
    let folder = mail.folder.borrow().clone();
    let account = mail.account();
    let messages = list_messages_via_service(&account, &folder, 1).map_err(|e| {
        let text = format!("The change was made and the folder could not be read back: {e}");
        say(ui, text.clone());
        text
    })?;
    set_rows(ui, mail, &messages);
    let all = mail.all_rows.borrow();
    all.iter()
        .find(|(row_id, _, _)| row_id == id)
        .map(|(_, _, it)| read(it))
        .ok_or_else(|| format!("the message is no longer in {folder}"))
}

/// Change a row in hand, for the cases where the mail server has already agreed.
fn mark_row_locally(mail: &Rc<Mail>, id: &str, change: impl Fn(&mut EmailListItem)) {
    let mut all = mail.all_rows.borrow_mut();
    if let Some((_, _, item)) = all.iter_mut().find(|(row_id, _, _)| row_id == id) {
        change(item);
    }
}

/// Delete a message, and check it is gone.
///
/// `if delete_message_via_service(&msg_id).is_some() { … }` had no else, so a delete the mail
/// server refused left the list exactly as it was and said nothing — which reads as a delete
/// that worked.
fn delete_message(ui: &EmailApp, mail: &Rc<Mail>, row: usize) -> Result<String, String> {
    let account = mail.account();
    let (id, row_folder) =
        mail.row_at(row).ok_or_else(|| format!("there is no message at row {}", row + 1))?;
    let subject = mail
        .all_rows
        .borrow()
        .iter()
        .find(|(row_id, _, _)| *row_id == id)
        .map(|(_, _, it)| it.subject.to_string())
        .unwrap_or_default();

    let was_read = row_flag(mail, &id, |it| it.is_read);
    // From the mailbox the row was listed from, and never left to the service's INBOX
    // default: deleting a Spam row could destroy a different INBOX message (#288).
    delete_message_via_service(&account, &row_folder, &id).map_err(|e| {
        let text = format!("Could not delete \u{201c}{subject}\u{201d}: {e}");
        say(ui, text.clone());
        text
    })?;
    confirm_gone(ui, mail, &id, "delete", &subject)?;
    let folder = mail.folder.borrow().clone();
    change_counts_of(ui, mail, &folder, |c| c.after_removal(was_read));
    Ok(subject)
}

/// Move a message to another folder, and check it left this one.
fn move_message(
    ui: &EmailApp,
    mail: &Rc<Mail>,
    row: usize,
    target: &str,
) -> Result<String, String> {
    let account = mail.account();
    let (id, row_folder) =
        mail.row_at(row).ok_or_else(|| format!("there is no message at row {}", row + 1))?;
    let subject = mail
        .all_rows
        .borrow()
        .iter()
        .find(|(row_id, _, _)| *row_id == id)
        .map(|(_, _, it)| it.subject.to_string())
        .unwrap_or_default();

    let was_read = row_flag(mail, &id, |it| it.is_read);
    // Out of the mailbox the row was listed from, for delete's reason (#288).
    move_message_via_service(&account, &row_folder, &id, target).map_err(|e| {
        let text = format!("Could not move \u{201c}{subject}\u{201d} to {target}: {e}");
        say(ui, text.clone());
        text
    })?;
    confirm_gone(ui, mail, &id, "move", &subject)?;
    // Both ends of the move: it left this folder and arrived in that one.
    let folder = mail.folder.borrow().clone();
    change_counts_of(ui, mail, target, |c| c.after_arrival(was_read));
    change_counts_of(ui, mail, &folder, |c| c.after_removal(was_read));
    Ok(subject)
}

/// The folder read back, and the message that was supposed to leave it checked for.
fn confirm_gone(
    ui: &EmailApp,
    mail: &Rc<Mail>,
    id: &str,
    what: &str,
    subject: &str,
) -> Result<(), String> {
    let folder = mail.folder.borrow().clone();
    let account = mail.account();
    let messages = list_messages_via_service(&account, &folder, 1).map_err(|e| {
        let text = format!("The {what} was made and {folder} could not be read back: {e}");
        say(ui, text.clone());
        text
    })?;
    set_rows(ui, mail, &messages);
    if mail.all_rows.borrow().iter().any(|(row_id, _, _)| row_id == id) {
        let text = format!(
            "The mail server reported the {what} of \u{201c}{subject}\u{201d} and it is still in \
             {folder}."
        );
        say(ui, text.clone());
        return Err(text);
    }
    // Only if it was the message being read. Clearing the pane for a row somebody deleted
    // further down the list would take a message off the screen that is still in the mailbox.
    if ui.get_email_detail().subject == subject {
        ui.set_email_detail(EmailDetailData::default());
        ui.set_email_attachments(ModelRc::new(VecModel::<EmailAttachmentData>::from(Vec::new())));
        // The pane is clear, so there is no original to open either; the button follows
        // `has-html` off the default detail and the held copy goes with it.
        *mail.open_original.borrow_mut() = None;
    }
    clear_notice(ui);
    Ok(())
}

/// Search, or say why there are no results.
///
/// `if let Some(results) = search_via_service(&q)` had no else, so a search the mail server
/// refused left whatever was on screen and read as a mailbox with no matches.
fn run_search(ui: &EmailApp, mail: &Rc<Mail>, query: &str) -> Result<usize, String> {
    let query = query.trim();
    if query.is_empty() {
        ui.set_email_search_active(false);
        let folder = mail.folder.borrow().clone();
        return load_folder(ui, mail, &folder);
    }
    let account = mail.account();
    if account.is_empty() {
        let text = match mail.unreachable_reason() {
            Some(reason) => format!("Cannot search: the mail service could not be reached: {reason}"),
            None => "Cannot search: no email account is configured.".to_string(),
        };
        say(ui, text.clone());
        return Err(text);
    }

    ui.set_email_search_active(true);
    // The folder on screen is the folder searched: the results are shown beside it, and each
    // row carries the folder it was found in for whatever is done to it next (#288).
    let folder = mail.folder.borrow().clone();
    match search_via_service(&account, &folder, query) {
        Ok(results) => {
            let count = results.len();
            set_rows(ui, mail, &results);
            ui.set_email_search_count(count as i32);
            clear_notice(ui);
            Ok(count)
        }
        Err(e) => {
            let text = format!("Could not search for \u{201c}{query}\u{201d}: {e}");
            say(ui, text.clone());
            Err(text)
        }
    }
}

/// The rows as a caller would name them.
fn message_rows(ui: &EmailApp) -> Vec<MessageRow> {
    let model = ui.get_email_list();
    (0..model.row_count())
        .filter_map(|i| model.row_data(i))
        .map(|it| MessageRow::new(&it.subject, &it.from_name, &it.from_addr))
        .collect()
}

// ── The Google sign-in, on this side of the wire ─────────────────────

/// Start one: ask the service, open a browser, and leave a poll running.
///
/// Everything slow is on a worker. `oauth_begin` is a socket bind and answers at once, but it is
/// still a service call, and opening a browser is a process spawn with a bounded wait on it —
/// neither belongs on the thread that draws the window.
fn start_google_sign_in(ui: &EmailApp, mail: &Rc<Mail>) -> Result<(), String> {
    if mail.setting_up.get() || mail.google_flow.borrow().is_some() {
        return Err("A sign-in is already in progress.".to_string());
    }
    if !ui.get_google_available() {
        // The reason is already on the screen, from `email.accounts`. Returned as well, so a
        // caller of the action gets the same sentence a person is reading.
        return Err(ui.get_google_note().to_string());
    }

    mail.setting_up.set(true);
    ui.set_google_waiting(true);
    ui.set_google_url(SharedString::default());
    ui.set_setup_ok(false);
    ui.set_setup_status("Opening the Google sign-in\u{2026}".into());

    let back = ui.as_weak();
    std::thread::spawn(move || {
        let begun = oauth_begin_via_service();
        // The browser is opened here rather than after the hop back, so the window is never held
        // waiting on a process spawn.
        let opened = begun.as_ref().ok().map(|b| open_in_browser(&b.auth_url));
        let _ = back.upgrade_in_event_loop(move |ui| {
            let Some(mail) = with_mail(|m| m.clone()) else { return };
            mail.setting_up.set(false);
            match begun {
                Err(e) => {
                    ui.set_google_waiting(false);
                    let text = format!("The Google sign-in could not be started: {e}");
                    ui.set_setup_status(text.clone().into());
                    say(&ui, text);
                }
                Ok(begun) => {
                    *mail.google_flow.borrow_mut() = Some(begun.flow_id.clone());
                    match opened {
                        Some(Ok(())) => {
                            ui.set_google_url(SharedString::default());
                            ui.set_setup_status("Waiting for Google\u{2026}".into());
                            clear_notice(&ui);
                        }
                        Some(Err(why)) => {
                            // Not a dead end: the address goes on the screen so the sign-in can
                            // be finished from anywhere with a browser.
                            let text = state::browser_failed_note(&why);
                            ui.set_google_url(begun.auth_url.clone().into());
                            ui.set_setup_status(text.clone().into());
                            say(&ui, text);
                        }
                        None => {}
                    }
                    watch_google(ui.as_weak(), begun.flow_id, begun.expires_in_secs);
                }
            }
        });
    });
    Ok(())
}

/// Ask the mail service where a sign-in has got to, once a second, until it stops saying
/// "waiting".
///
/// On a worker, always. The person is in a browser and this window has to stay alive — the
/// control surface gives an action three seconds on the UI thread, and a poll loop there would
/// blow that on the first iteration.
fn watch_google(weak: slint::Weak<EmailApp>, flow_id: String, budget_secs: u64) {
    std::thread::spawn(move || {
        // A little past the service's own deadline, so the service's sentence about giving up
        // is the one that is shown rather than this app's.
        let deadline = std::time::Instant::now() + Duration::from_secs(budget_secs + 20);
        loop {
            std::thread::sleep(Duration::from_secs(1));
            if std::time::Instant::now() >= deadline {
                let id = flow_id.clone();
                let _ = weak.upgrade_in_event_loop(move |ui| {
                    apply_google(
                        &ui,
                        &id,
                        GoogleOutcome::Stopped(
                            "The Google sign-in was not finished in time, so this window stopped \
                             waiting for it. Nothing was saved."
                                .to_string(),
                        ),
                    );
                });
                return;
            }
            let outcome = state::google_outcome(oauth_status_via_service(&flow_id));
            if outcome == GoogleOutcome::Waiting {
                continue;
            }
            let id = flow_id.clone();
            let _ = weak
                .upgrade_in_event_loop(move |ui| apply_google(&ui, &id, outcome));
            return;
        }
    });
}

/// Put a finished sign-in on the screen.
fn apply_google(ui: &EmailApp, flow_id: &str, outcome: GoogleOutcome) {
    let Some(mail) = with_mail(|m| m.clone()) else { return };

    // A flow the window is no longer waiting on: cancelled, or abandoned and restarted. Applying
    // it would put an answer to an old question over the top of a new one.
    if mail.google_flow.borrow().as_deref() != Some(flow_id) {
        return;
    }
    *mail.google_flow.borrow_mut() = None;
    ui.set_google_waiting(false);
    ui.set_google_url(SharedString::default());

    match outcome {
        // Should not reach here — `watch_google` keeps polling on Waiting — and if it does, it is
        // said rather than left as a window stuck on a spinner.
        GoogleOutcome::Waiting => {
            let text = "The Google sign-in is still waiting; this window stopped following it."
                .to_string();
            ui.set_setup_ok(false);
            ui.set_setup_status(text.clone().into());
            say(ui, text);
        }
        GoogleOutcome::SignedIn(email) => {
            ui.set_setup_ok(true);
            ui.set_setup_status(format!("Signed in as {email}").into());
            ui.set_setup_password(SharedString::default());
            ui.set_setup_open(false);
            clear_notice(ui);
            // Through the sync, which runs on a worker: loading a mailbox here would hold the UI
            // thread for as long as the mail server takes, right after a sign-in.
            ui.invoke_sync_emails();
        }
        // Said twice, like every other failure in this app: on the form the person is looking at,
        // and in `describe.notice` for whatever is driving the window.
        GoogleOutcome::Stopped(reason) => {
            ui.set_setup_ok(false);
            ui.set_setup_status(reason.clone().into());
            say(ui, reason);
        }
    }
}

/// Give up on one, and tell the mail service to close the socket it is waiting on.
fn cancel_google_sign_in(ui: &EmailApp, mail: &Rc<Mail>) {
    let flow = mail.google_flow.borrow_mut().take();
    ui.set_google_waiting(false);
    ui.set_google_url(SharedString::default());
    ui.set_setup_ok(false);
    match flow {
        Some(flow_id) => {
            ui.set_setup_status("The Google sign-in was cancelled.".into());
            clear_notice(ui);
            // On a worker: the service stops a listener thread, which is fast, but this is still
            // a socket call from a button handler.
            std::thread::spawn(move || {
                if let Err(e) = oauth_cancel_via_service(&flow_id) {
                    tracing::warn!(error = %e, "the mail service could not be told to cancel");
                }
            });
        }
        // Nothing to cancel. The panel is closed anyway rather than left up, because the window
        // saying it is waiting for something it is not waiting for is its own small lie.
        None => ui.set_setup_status(SharedString::default()),
    }
}

// ── The control surface ──────────────────────────────────────────────
//
// What the companion can see of Email, and what it can ask Email to do, without photographing
// the window. See `yantrik_app_runtime::control`.

fn publish_control(app: &EmailApp, mail: &Rc<Mail>) {
    use yantrik_app_runtime::control::{Action, App, Param, View};

    let describe = {
        let weak = app.as_weak();
        let mail = mail.clone();
        move || {
            let Some(ui) = weak.upgrade() else {
                return View::new("Email — closing");
            };
            let mail_state = mail.state.borrow().clone();

            // The state the surface always carries, whichever of the three this is. `service`
            // and `notice` are the two that were missing: a caller could not tell a machine with
            // no account from one whose mail service had never been started, because the app
            // could not either.
            let base = View::new(mail_state.summary())
                .with("service", mail_state.service_word())
                .with("notice", ui.get_notice().to_string())
                .with("account_store", mail_state.config_path().to_string());

            let base = match mail_state.has_account() {
                // Deliberately null rather than false. With nothing answering, this app has not
                // been told whether an account exists, and `false` is the exact claim that had
                // an audit conclude this machine had no mail account.
                None => return base.with("has_account", serde_json::Value::Null),
                Some(known) => base.with("has_account", known),
            };
            if mail_state.has_account() == Some(false) {
                return base;
            }

            let folders = ui.get_folders();
            let folder = mail.folder.borrow().clone();

            let detail = ui.get_email_detail();
            let reading = detail.id > 0 && !detail.subject.is_empty();

            let summary = if ui.get_is_composing() {
                format!("Email — composing to {}", {
                    let to = ui.get_compose_to().to_string();
                    if to.is_empty() { "(nobody yet)".to_string() } else { to }
                })
            } else if reading {
                format!(
                    "Email — {folder}, reading \u{201c}{}\u{201d} from {}",
                    detail.subject, detail.from_name
                )
            } else {
                // The same two numbers as the folder's entry in `folders` below, by
                // construction: `show_folder_counts` reads them off that list — and when that
                // entry has none, this says so rather than "0 unread of 0".
                let counts = header_counts(&ui);
                let query = ui.get_email_search_query().to_string();
                let search = ui
                    .get_email_search_active()
                    .then(|| (query.as_str(), ui.get_email_list().row_count()));
                state::folder_summary(&folder, &counts, search)
            };

            let list = ui.get_email_list();
            let messages: Vec<serde_json::Value> = (0..list.row_count().min(25))
                .filter_map(|i| list.row_data(i))
                .enumerate()
                .map(|(i, m)| {
                    serde_json::json!({
                        // The number `open_message which=` takes, said beside the message it
                        // opens, so a caller never has to count the list itself.
                        "row": i + 1,
                        "subject": m.subject.to_string(),
                        "from": m.from_name.to_string(),
                        "address": m.from_addr.to_string(),
                        "date": m.date_text.to_string(),
                        "read": m.is_read,
                        "flagged": m.is_flagged,
                    })
                })
                .collect();

            // `counts` is `{unread, total}` as the mail server counted the folder, or `null`
            // when it did not count it this time — with `reason` saying why. Never `0/0` for
            // that: an empty folder is `{"unread": 0, "total": 0}`, and a refused STATUS is
            // not an empty folder (#131).
            let folder_rows: Vec<serde_json::Value> = (0..folders.row_count())
                .filter_map(|i| folders.row_data(i))
                .map(folder_record)
                .map(|f| {
                    serde_json::json!({
                        "name": f.name,
                        "counts": f.counts.map(|c| serde_json::json!({
                            "unread": c.unread,
                            "total": c.total,
                        })),
                        "reason": f.reason,
                    })
                })
                .collect();

            let open = if reading {
                serde_json::json!({
                    "subject": detail.subject.to_string(),
                    "from": detail.from_name.to_string(),
                    "address": detail.from_addr.to_string(),
                    "date": detail.date_text.to_string(),
                    "flagged": detail.is_flagged,
                    "read": detail.is_read,
                    // The body, because summarising mail is the most common thing to want and
                    // fetching it a second way would be the screenshot problem again.
                    "body": detail.body.to_string(),
                })
            } else {
                serde_json::Value::Null
            };

            let header = header_counts(&ui);
            let mut view = base
                .with("account", ui.get_account_name().to_string())
                .with("folder", folder)
                .with("triage", mail.triage.get().label())
                // `counts` is the open folder's `{unread, total}`, as the mail server counts
                // them — the same numbers as its entry in `folders` — or `null` when the server
                // did not count it, with `counts_reason` saying why. `listed` is how many
                // messages are in hand: the newest page, which is what `messages` is drawn from.
                .with("counts", counts_json(&header))
                .with(
                    "counts_reason",
                    match &header {
                        Counted::Unavailable { reason } => serde_json::Value::from(reason.as_str()),
                        Counted::Known(_) => serde_json::Value::Null,
                    },
                )
                .with("listed", list.row_count())
                .with("composing", ui.get_is_composing())
                .with("search_query", ui.get_email_search_query().to_string())
                .with("open_message", open)
                .with("folders", serde_json::Value::Array(folder_rows))
                .with("messages", serde_json::Value::Array(messages));
            // The one line a caller surveying every window pays for. It says what is on screen
            // once there is a mailbox; until then `MailState::summary` has already said which of
            // the two other things is true, and that is the more important news.
            view.summary = summary;
            view
        }
    };

    let weak = app.as_weak();
    let ui_for = move || weak.upgrade().ok_or_else(|| "Email window is gone".to_string());

    let open_ui = ui_for.clone();
    let folder_ui = ui_for.clone();
    let search_ui = ui_for.clone();
    let read_ui = ui_for.clone();
    let flag_ui = ui_for.clone();
    let compose_ui = ui_for.clone();
    let google_ui = ui_for;

    let open_mail = mail.clone();
    let folder_mail = mail.clone();
    let search_mail = mail.clone();
    let read_mail = mail.clone();
    let flag_mail = mail.clone();
    let google_mail = mail.clone();

    App::new("email")
        .describe(describe)
        .action(
            // Every action below is graded explicitly, and none of them is `safe`.
            //
            // `safe` in this OS's ladder is for something that changes nothing and stays on the
            // machine. Every action on this surface reaches a mail server across the network —
            // selecting a folder and searching both do, which is not obvious from their names —
            // and opening a message marks it read out there, which is a change someone else can
            // see. So: `standard` throughout, deliberately, rather than by default.
            //
            // Nothing here is above `standard` because nothing here sends or deletes mail.
            // Anything that did would be `sensitive` at least: mail that has gone cannot be
            // taken back, and a deleted message is not on the server any more. That is why
            // `send`, `delete` and `archive` are not published at all, and why `compose`
            // deliberately stops at a draft.
            Action::new("open_message", "Open a message by row number, subject or sender")
                .risk("standard")
                .arg(Param::text("which").describe(
                    "The row number from `messages`, counting from 1 — or part of the subject, \
                     the sender's name, or their address",
                )),
            move |args| {
                let ui = open_ui()?;
                // Accepts a number as well as a string. `which=1` used to arrive as a JSON
                // number, `as_str()` gave `""`, and the answer was `nothing in this folder
                // matches ""` — a search for nothing, reported as if it were what was asked.
                let want = arg_text(&args["which"]);
                let row = state::resolve_which(&message_rows(&ui), &want)?;
                let detail = open_message(&ui, &open_mail, row)?;
                Ok(serde_json::json!({
                    "row": row + 1,
                    "subject": detail.subject,
                    "from": detail.from,
                    "read": detail.is_read,
                }))
            },
        )
        .action(
            // It looks local and is not: switching mailbox asks the mail server for that
            // folder's messages.
            Action::new("select_folder", "Switch to another mailbox")
                .risk("standard")
                .arg(Param::text("folder").describe("Folder name, e.g. INBOX, Sent, Archive")),
            move |args| {
                let ui = folder_ui()?;
                let want = arg_text(&args["folder"]).trim().to_lowercase();
                if want.is_empty() {
                    return Err("no folder was named".to_string());
                }
                let folders = ui.get_folders();
                let row = (0..folders.row_count())
                    .find(|i| {
                        folders
                            .row_data(*i)
                            .map(|f| {
                                f.name.to_lowercase() == want
                                    || f.folder_type.to_lowercase() == want
                            })
                            .unwrap_or(false)
                    })
                    .ok_or_else(|| {
                        let names: Vec<String> = (0..folders.row_count())
                            .filter_map(|i| folders.row_data(i))
                            .map(|f| f.name.to_string())
                            .collect();
                        if names.is_empty() {
                            format!("no folder called \u{201c}{want}\u{201d}; no folders are listed")
                        } else {
                            format!(
                                "no folder called \u{201c}{want}\u{201d}; there is: {}",
                                names.join(", ")
                            )
                        }
                    })?;
                let name = folders.row_data(row).map(|f| f.name.to_string()).unwrap_or_default();
                ui.set_selected_folder_index(row as i32);
                let count = load_folder(&ui, &folder_mail, &name)?;
                Ok(serde_json::json!({
                    "folder": name,
                    "messages": count,
                    "counts": counts_json(&header_counts(&ui)),
                }))
            },
        )
        .action(
            // It changes nothing, and it still leaves the machine: the query goes to the mail
            // server as an IMAP SEARCH.
            Action::new("search", "Search the mailbox").risk("standard").arg(Param::text("query")),
            move |args| {
                let ui = search_ui()?;
                let query = arg_text(&args["query"]);
                ui.set_email_search_query(query.clone().into());
                // A search that could not be run is a refusal. It used to leave whatever was on
                // screen and report the row count, so a mail server that refused the query and a
                // mailbox with no matches were the same answer.
                let matched = run_search(&ui, &search_mail, &query)?;
                let list = ui.get_email_list();
                let hits: Vec<String> = (0..list.row_count().min(25))
                    .filter_map(|i| list.row_data(i))
                    .map(|m| m.subject.to_string())
                    .collect();
                Ok(serde_json::json!({ "matched": matched, "subjects": hits }))
            },
        )
        .action(
            Action::new("mark_read", "Mark a message as read")
                .risk("standard")
                .arg(
                    Param::text("which")
                        .describe("Row number, subject or sender; omit for the open message")
                        .optional(),
                ),
            move |args| {
                let ui = read_ui()?;
                let row = row_from_args(&ui, args)?;
                // The flag as the mail server reports it after the change, not the row that was
                // asked about. `{"marked": row}` was true of the request and said nothing about
                // the mailbox.
                let read = set_read(&ui, &read_mail, row, true)?;
                Ok(serde_json::json!({
                    "row": row + 1,
                    "subject": subject_at(&ui, row),
                    "read": read,
                }))
            },
        )
        .action(
            Action::new("flag", "Flag a message")
                .risk("standard")
                .arg(
                    Param::text("which")
                        .describe("Row number, subject or sender; omit for the open message")
                        .optional(),
                ),
            move |args| {
                let ui = flag_ui()?;
                let row = row_from_args(&ui, args)?;
                let flagged = set_flagged(&ui, &flag_mail, row, true)?;
                Ok(serde_json::json!({
                    "row": row + 1,
                    "subject": subject_at(&ui, row),
                    "flagged": flagged,
                }))
            },
        )
        .action(
            Action::new(
                "compose",
                "Open the composer with a draft filled in. Does NOT send — the user reviews and sends it.",
            )
            .risk("standard")
            .arg(Param::text("to"))
            .arg(Param::text("subject"))
            .arg(Param::text("body")),
            move |args| {
                let ui = compose_ui()?;
                ui.invoke_compose_new();
                ui.set_compose_to(arg_text(&args["to"]).into());
                ui.set_compose_subject(arg_text(&args["subject"]).into());
                ui.set_compose_body(arg_text(&args["body"]).into());
                // Sending is not offered on this surface on purpose: a draft can be read before
                // it leaves, and mail that has already gone cannot be taken back.
                Ok(serde_json::json!({
                    "drafted": true,
                    "note": "waiting for the user to send it",
                }))
            },
        )
        .action(
            // The one setup step that can be on this surface, because it is the one that takes
            // no credential: it opens Google's own consent page and stops.
            //
            // `sensitive` rather than `standard`, and the grade is about what it does to the
            // person rather than to the machine. By itself it changes nothing — no account is
            // written, no mailbox is touched, and the flow cannot complete without somebody
            // choosing an account and reading a consent screen that says full access to Gmail.
            // But it puts that screen in front of them, unasked, on their own display, and a
            // consent page a person did not go looking for is exactly the shape of the thing
            // they should not be trained to click through. So it is above `standard`, where a
            // `tool_permission: standard` ceiling refuses it, and the default ceiling allows it.
            //
            // It takes no arguments on purpose. There is nothing to parameterise: the provider
            // is Google because the scope and the servers are Google's, and the address comes
            // from the sign-in rather than from a caller.
            Action::new(
                "begin_google_sign_in",
                "Open Google's sign-in page to add a Gmail account. Cannot finish on its own — \
                 the user chooses the account and grants access in their browser.",
            )
            .risk("sensitive"),
            move |_args| {
                let ui = google_ui()?;
                start_google_sign_in(&ui, &google_mail)?;
                // What was observed, which is that a browser was asked to open. Not "signed in":
                // whether this works is decided in a browser by a person, minutes from now, and
                // `describe` carries the answer when it arrives.
                Ok(serde_json::json!({
                    "opened": true,
                    "note": "Google's consent page was opened. The user has to choose an account \
                             and allow access; ask describe again for the outcome.",
                }))
            },
        )
        .serve();

    // Setting an account up with a password is deliberately absent from the list above.
    //
    // `docs/app-control.md` is explicit that the transcript a mind works in is readable. An
    // action that accepted a password would put a mail credential in it, in the clear, for every
    // caller of `app.describe` afterwards. `save_account` and `test_connection` are therefore not
    // on this surface and never will be.
    //
    // `begin_google_sign_in` is the exception that proves the rule rather than a hole in it: it
    // accepts nothing, carries nothing back, and the credential it ends in never enters this
    // process at all — the mail service holds the token and the app is told an address.
}

/// An argument that may arrive as a string or as a number.
///
/// `args["which"].as_str().unwrap_or_default()` is where `which=1` became `""`.
fn arg_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        _ => String::new(),
    }
}

/// The row an action means: what it named, or the message that is open.
fn row_from_args(ui: &EmailApp, args: &serde_json::Value) -> Result<usize, String> {
    let rows = message_rows(ui);
    let want = arg_text(&args["which"]);
    if !want.trim().is_empty() {
        return state::resolve_which(&rows, &want);
    }
    let open = ui.get_email_detail();
    if open.id <= 0 || open.subject.is_empty() {
        return Err("no message is open, and none was named".to_string());
    }
    state::resolve_which(&rows, &open.subject.to_string())
}

fn subject_at(ui: &EmailApp, row: usize) -> String {
    ui.get_email_list().row_data(row).map(|it| it.subject.to_string()).unwrap_or_default()
}

// ── Wire all callbacks ───────────────────────────────────────────────

fn wire(app: &EmailApp) {
    let mail = Rc::new(Mail::new());
    MAIL.with(|cell| *cell.borrow_mut() = Some(mail.clone()));

    // Initial load — one look at the mail service, which starts it if it is not running.
    if std::env::var_os("YANTRIK_EMAIL_DEMO").is_some() {
        demo::populate(app);
        *mail.state.borrow_mut() = MailState::Ready {
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
        app.set_service_state("up".into());
    } else {
        let folder = mail.folder.borrow().clone();
        let loaded = look_at_mail(&folder);
        apply_loaded(app, &mail, loaded);
    }

    // Published once the mailbox is loaded, so the first `app.describe` is not of an empty
    // window.
    publish_control(app, &mail);

    // ── Folder clicked ──
    {
        let weak = app.as_weak();
        let mail = mail.clone();
        app.on_folder_clicked(move |idx| {
            let Some(ui) = weak.upgrade() else { return };
            let model = ui.get_folders();
            if idx < 0 || idx as usize >= model.row_count() {
                return;
            }
            let folder = model.row_data(idx as usize).unwrap();
            ui.set_selected_folder_index(idx);
            let _ = load_folder(&ui, &mail, &folder.name.to_string());
        });
    }

    // ── Email selected ──
    {
        let weak = app.as_weak();
        let mail = mail.clone();
        app.on_email_selected(move |idx| {
            let Some(ui) = weak.upgrade() else { return };
            if idx < 0 {
                return;
            }
            if let Err(e) = open_message(&ui, &mail, idx as usize) {
                say(&ui, e);
            }
        });
    }

    // ── Triage tabs ──
    //
    // A filter over the rows already held: no question to the mail server, and no tab that
    // filters on something no message carries.
    {
        let weak = app.as_weak();
        let mail = mail.clone();
        app.on_triage_filter(move |idx| {
            let Some(ui) = weak.upgrade() else { return };
            match Triage::from_index(idx) {
                Some(t) => {
                    mail.triage.set(t);
                    show_rows(&ui, &mail);
                }
                None => say(&ui, format!("There is no triage tab {idx}.")),
            }
        });
    }

    // ── Compose new ──
    {
        let weak = app.as_weak();
        let mail = mail.clone();
        app.on_compose_new(move || {
            let Some(ui) = weak.upgrade() else { return };
            ui.set_is_composing(true);
            // Whatever was being written when the composer was last closed. The composer used to
            // open empty every time and the half-written message was simply gone.
            match state::load_draft(&mail.draft_path) {
                Ok(Some(draft)) => {
                    ui.set_compose_to(draft.to.into());
                    ui.set_compose_cc(draft.cc.into());
                    ui.set_compose_bcc(draft.bcc.into());
                    ui.set_compose_subject(draft.subject.into());
                    ui.set_compose_body(draft.body.into());
                    ui.set_email_draft_status("Draft restored".into());
                }
                Ok(None) => {
                    ui.set_compose_to(SharedString::default());
                    ui.set_compose_cc(SharedString::default());
                    ui.set_compose_bcc(SharedString::default());
                    ui.set_compose_subject(SharedString::default());
                    ui.set_compose_body(SharedString::default());
                    ui.set_email_draft_status(SharedString::default());
                }
                Err(e) => {
                    ui.set_email_draft_status(SharedString::default());
                    say(&ui, format!("Could not read the saved draft: {e}"));
                }
            }
        });
    }

    // ── Send email ──
    //
    // The one handler in this file that was already right: a real else, and a reason on screen.
    // It is the shape every other one now has.
    {
        let weak = app.as_weak();
        let mail = mail.clone();
        app.on_send_email(move |to, cc, bcc, subject, body| {
            let Some(ui) = weak.upgrade() else { return };
            let account = mail.account();
            if account.is_empty() {
                say(&ui, "Cannot send: no email account is configured.");
                return;
            }
            match send_message_via_service(&account, &to, &cc, &bcc, &subject, &body) {
                Ok(()) => {
                    ui.set_is_composing(false);
                    ui.set_email_sync_status("Message sent".into());
                    ui.set_email_draft_status(SharedString::default());
                    // The draft was this message; it has gone.
                    if let Err(e) = state::clear_draft(&mail.draft_path) {
                        say(&ui, format!("The message was sent; the draft is still on disk: {e}"));
                    } else {
                        clear_notice(&ui);
                    }
                    let folder = mail.folder.borrow().clone();
                    let _ = load_folder(&ui, &mail, &folder);
                }
                Err(e) => {
                    // The composer stays open, holding what was typed.
                    ui.set_email_sync_status("Not sent".into());
                    say(&ui, format!("Could not send the message: {e}"));
                }
            }
        });
    }

    // ── Cancel compose ──
    //
    // Keeps what was typed. Closing the composer used to discard it silently, which is the worst
    // thing a mail client can do to a message that is not finished.
    {
        let weak = app.as_weak();
        let mail = mail.clone();
        app.on_cancel_compose(move || {
            let Some(ui) = weak.upgrade() else { return };
            ui.set_is_composing(false);
            let draft = draft_on_screen(&ui);
            if draft.is_empty() {
                let _ = state::clear_draft(&mail.draft_path);
                ui.set_email_draft_status(SharedString::default());
                return;
            }
            match state::save_draft(&mail.draft_path, &draft) {
                Ok(()) => {
                    ui.set_email_draft_status("Draft saved".into());
                    say(&ui, "The unsent message was kept as a draft; Compose reopens it.");
                }
                Err(e) => say(&ui, format!("Could not keep the unsent message: {e}")),
            }
        });
    }

    // ── Save draft ──
    {
        let weak = app.as_weak();
        let mail = mail.clone();
        app.on_save_draft(move || {
            let Some(ui) = weak.upgrade() else { return };
            let draft = draft_on_screen(&ui);
            if draft.is_empty() {
                ui.set_email_draft_status(SharedString::default());
                say(&ui, "There is nothing in the composer to save.");
                return;
            }
            match state::save_draft(&mail.draft_path, &draft) {
                Ok(()) => {
                    ui.set_email_draft_status("Draft saved".into());
                    clear_notice(&ui);
                }
                Err(e) => {
                    ui.set_email_draft_status(SharedString::default());
                    say(&ui, format!("Could not save the draft: {e}"));
                }
            }
        });
    }

    // ── Delete email ──
    {
        let weak = app.as_weak();
        let mail = mail.clone();
        app.on_delete_email(move |idx| {
            let Some(ui) = weak.upgrade() else { return };
            if idx < 0 {
                return;
            }
            match delete_message(&ui, &mail, idx as usize) {
                Ok(subject) => tracing::info!(subject = %subject, "message deleted"),
                // Already on screen: every failure path in `delete_message` says so before it
                // returns. Logged here so the reason is in the app's own log too.
                Err(e) => tracing::warn!(error = %e, "delete failed"),
            }
        });
    }

    // ── Archive email ──
    {
        let weak = app.as_weak();
        let mail = mail.clone();
        app.on_archive_email(move |idx| {
            let Some(ui) = weak.upgrade() else { return };
            if idx < 0 {
                return;
            }
            // The archive folder as this account names it, not Gmail's, which was hardcoded:
            // "[Gmail]/All Mail" is not a folder on any other provider, so archiving on an
            // Outlook or a private IMAP account failed every time and said nothing.
            let target = archive_folder(&ui);
            match move_message(&ui, &mail, idx as usize, &target) {
                Ok(subject) => tracing::info!(subject = %subject, to = %target, "message moved"),
                Err(e) => tracing::warn!(error = %e, "archive failed"),
            }
        });
    }

    // ── Mark read ──
    {
        let weak = app.as_weak();
        let mail = mail.clone();
        app.on_mark_read(move |idx| {
            let Some(ui) = weak.upgrade() else { return };
            if idx < 0 {
                return;
            }
            let _ = set_read(&ui, &mail, idx as usize, true);
        });
    }

    // ── Mark flagged ──
    {
        let weak = app.as_weak();
        let mail = mail.clone();
        app.on_mark_flagged(move |idx| {
            let Some(ui) = weak.upgrade() else { return };
            if idx < 0 {
                return;
            }
            // The row's own flag, turned round. It was hardcoded to `true`, so the star could be
            // put on and never taken off.
            let now = ui.get_email_list().row_data(idx as usize).map(|it| it.is_flagged);
            let want = !now.unwrap_or(false);
            let _ = set_flagged(&ui, &mail, idx as usize, want);
        });
    }

    // ── Search ──
    {
        let weak = app.as_weak();
        let mail = mail.clone();
        app.on_search_emails(move |query| {
            let Some(ui) = weak.upgrade() else { return };
            let _ = run_search(&ui, &mail, &query.to_string());
        });
    }

    // ── Sync ──
    //
    // On a worker thread. A mailbox sync crosses the internet, and the control surface gives an
    // action three seconds on the UI thread before telling its caller the app did not answer —
    // so a sync that ran here would make every concurrent `describe` time out. It said
    // "Syncing...", called the loader, and then set "Synced" unconditionally: the word appeared
    // whether anything had synced, failed, or never been attempted.
    {
        let weak = app.as_weak();
        let mail = mail.clone();
        app.on_sync_emails(move || {
            let Some(ui) = weak.upgrade() else { return };
            if mail.syncing.get() {
                return;
            }
            mail.syncing.set(true);
            ui.set_email_sync_status("Syncing\u{2026}".into());
            let folder = mail.folder.borrow().clone();
            let back = ui.as_weak();
            std::thread::spawn(move || {
                let loaded = look_at_mail(&folder);
                let _ = back.upgrade_in_event_loop(move |ui| {
                    let Some(mail) = with_mail(|m| m.clone()) else { return };
                    mail.syncing.set(false);
                    let count = loaded.messages.len();
                    let ok = matches!(loaded.state, MailState::Ready { .. })
                        && loaded.mailbox_error.is_none();
                    apply_loaded(&ui, &mail, loaded);
                    if ok {
                        // The count it returned, because that is what was observed.
                        ui.set_email_sync_status(
                            format!("Synced \u{00b7} {count} in {folder}").into(),
                        );
                    }
                });
            });
        });
    }

    // ── Toggle thread message ──
    {
        let weak = app.as_weak();
        app.on_toggle_thread_message(move |idx| {
            let Some(ui) = weak.upgrade() else { return };
            let model = ui.get_email_thread_messages();
            let idx = idx as usize;
            if idx >= model.row_count() {
                return;
            }
            if let Some(mut msg) = model.row_data(idx) {
                msg.is_collapsed = !msg.is_collapsed;
                if let Some(vec_model) =
                    model.as_any().downcast_ref::<VecModel<EmailThreadMessage>>()
                {
                    vec_model.set_row_data(idx, msg);
                }
            }
        });
    }

    // ── Reply, reply-all, forward ──
    //
    // These three logged a line and returned, which is a strange thing for a mail client not to
    // do. None of them needs a model or a network call: the message is already on screen, and
    // replying is opening the composer with the right fields filled in. The quoting convention
    // is the one every client has used since mail was text.
    {
        let weak = app.as_weak();
        app.on_reply_email(move || {
            if let Some(ui) = weak.upgrade() {
                start_reply(&ui, false);
            }
        });
    }
    {
        let weak = app.as_weak();
        app.on_reply_all_email(move || {
            if let Some(ui) = weak.upgrade() {
                start_reply(&ui, true);
            }
        });
    }
    {
        let weak = app.as_weak();
        app.on_forward_email(move || {
            let Some(ui) = weak.upgrade() else { return };
            let d = ui.get_email_detail();
            ui.set_is_composing(true);
            ui.set_compose_to(SharedString::default());
            ui.set_compose_cc(SharedString::default());
            ui.set_compose_bcc(SharedString::default());
            ui.set_compose_subject(prefixed("Fwd: ", &d.subject));
            ui.set_compose_body(
                format!(
                    "\n\n---------- Forwarded message ----------\nFrom: {} <{}>\nSubject: {}\nDate: {}\n\n{}",
                    d.from_name, d.from_addr, d.subject, d.date_text, d.body
                )
                .into(),
            );
        });
    }

    // ── Open original ──
    //
    // The sender's HTML, handed to a browser, for the mail whose layout the reading pane's text
    // cannot carry. The file the browser gets has every remote image taken out first: a loaded
    // pixel tells the sender this message was opened, when, and from which address, and a click
    // that only meant "show me the layout" must not send that. The notice says what was removed,
    // because a layout that opens without its pictures is a change the person is owed an
    // explanation of.
    {
        let weak = app.as_weak();
        let mail = mail.clone();
        app.on_open_original(move || {
            let Some(ui) = weak.upgrade() else { return };
            let Some((id, html)) = mail.open_original.borrow().clone() else {
                say(&ui, "There is no message open with an HTML original.");
                return;
            };
            let (stripped, removed) = state::strip_remote_images(&html);
            let dir = state::original_html_dir();
            let path = state::original_html_file(&dir, &id);
            let written = std::fs::create_dir_all(&dir)
                .map_err(|e| format!("could not make {}: {e}", dir.display()))
                .and_then(|()| {
                    std::fs::write(&path, stripped)
                        .map_err(|e| format!("could not write {}: {e}", path.display()))
                });
            if let Err(e) = written {
                say(&ui, format!("The original HTML could not be written for the browser: {e}"));
                return;
            }
            match open_in_browser(&path.to_string_lossy()) {
                Ok(()) => say(&ui, state::original_opened_note(removed)),
                // Not silence: the file is there, and a person on a machine with no xdg-open
                // handler can still open it by hand if the screen says where it is.
                Err(e) => say(
                    &ui,
                    format!("The original is at {} but the browser could not be opened: {e}", path.display()),
                ),
            }
        });
    }

    // ── What the companion is for ──
    //
    // Summarising a thread, drafting from an instruction, suggesting a reply, and sorting one
    // message into a bucket: four things a model is genuinely better at than a rule. Each one
    // reads the message that is open and says so on the card.
    wire_mail_ai(app, MailAi::Summarize);
    wire_mail_ai(app, MailAi::SuggestReply);
    {
        let weak = app.as_weak();
        app.on_ai_draft(move |instruction| {
            let Some(ui) = weak.upgrade() else { return };
            let instruction = instruction.to_string();
            if instruction.trim().is_empty() {
                say(&ui, "Say what the message should be about.");
                return;
            }
            ui.set_ai_is_working(true);
            let back = ui.as_weak();
            std::thread::spawn(move || {
                let outcome = companion::ask(&format!(
                    "Draft an email for this instruction. Reply with the body only, no subject \
                     line and no commentary.\n\n{instruction}"
                ));
                let _ = back.upgrade_in_event_loop(move |ui| {
                    ui.set_ai_is_working(false);
                    match outcome {
                        Ok(text) => {
                            ui.set_compose_body(text.into());
                            clear_notice(&ui);
                        }
                        Err(e) => say(&ui, e.to_string()),
                    }
                });
            });
        });
    }
    {
        let weak = app.as_weak();
        app.on_enhance_text(move |style| {
            let Some(ui) = weak.upgrade() else { return };
            let body = ui.get_compose_body().to_string();
            if body.trim().is_empty() {
                say(&ui, "There is nothing in the composer to rewrite.");
                return;
            }
            let style = style.to_string();
            ui.set_ai_is_working(true);
            let back = ui.as_weak();
            std::thread::spawn(move || {
                let outcome = companion::ask(&format!(
                    "Rewrite this email to be {style}. Keep every fact and every commitment. \
                     Reply with the body only.\n\n{body}"
                ));
                let _ = back.upgrade_in_event_loop(move |ui| {
                    ui.set_ai_is_working(false);
                    match outcome {
                        Ok(text) => {
                            ui.set_compose_body(text.into());
                            clear_notice(&ui);
                        }
                        Err(e) => say(&ui, e.to_string()),
                    }
                });
            });
        });
    }

    // ── Classify the open message ──
    //
    // Through the same `companion::ask` the three above use, and the answer is checked against
    // the four labels the screen can draw rather than pasted into the badge. A model that says
    // something else is reported, not rendered as a category nobody defined.
    {
        let weak = app.as_weak();
        app.on_ai_classify(move || {
            let Some(ui) = weak.upgrade() else { return };
            let d = ui.get_email_detail();
            if d.id <= 0 || d.body.is_empty() {
                say(&ui, "There is no message open to classify.");
                return;
            }
            ui.set_ai_is_working(true);
            let back = ui.as_weak();
            std::thread::spawn(move || {
                let outcome = companion::ask(&format!(
                    "Put this email in exactly one of these categories: action-needed, fyi, \
                     meeting, newsletter. Reply with the category and nothing else.\n\nFrom: \
                     {}\nSubject: {}\n\n{}",
                    d.from_name, d.subject, d.body
                ));
                let _ = back.upgrade_in_event_loop(move |ui| {
                    ui.set_ai_is_working(false);
                    match outcome {
                        Ok(text) => {
                            let label = text.trim().trim_matches('.').to_lowercase();
                            const KNOWN: [&str; 4] =
                                ["action-needed", "fyi", "meeting", "newsletter"];
                            if KNOWN.contains(&label.as_str()) {
                                ui.set_email_ai_intent(label.into());
                                clear_notice(&ui);
                            } else {
                                ui.set_email_ai_intent(SharedString::default());
                                say(
                                    &ui,
                                    format!(
                                        "The companion answered \u{201c}{}\u{201d}, which is not \
                                         one of the four categories this screen knows.",
                                        text.trim()
                                    ),
                                );
                            }
                        }
                        Err(e) => say(&ui, e.to_string()),
                    }
                });
            });
        });
    }

    // ── Adding another account ──
    {
        let weak = app.as_weak();
        app.on_add_account(move || {
            let Some(ui) = weak.upgrade() else { return };
            ui.set_setup_open(true);
            ui.set_setup_status(SharedString::default());
            ui.set_setup_ok(false);
            ui.set_setup_email(SharedString::default());
            ui.set_setup_password(SharedString::default());
            ui.set_setup_display_name(SharedString::default());
            ui.set_setup_provider(SharedString::default());
            ui.set_setup_advanced_mode(false);
            // A form opened fresh is not waiting for anything. Left over from a previous visit,
            // the waiting panel would hide the fields and offer a Cancel for a flow that ended.
            ui.set_google_waiting(false);
            ui.set_google_url(SharedString::default());
        });
    }
    // ── Sign in with Google ──
    //
    // The button that was removed in September for calling a stub. It is back because there is
    // something behind it now: `email.oauth_begin` in the mail service, PKCE and a loopback
    // redirect, and an IMAP sign-in with the token before any account is written.
    //
    // The whole of it is off the UI thread. A person goes to a browser in the middle of this and
    // takes as long as they take; what this window does meanwhile is say so, and offer Cancel.
    {
        let weak = app.as_weak();
        let mail = mail.clone();
        app.on_sign_in_with_google(move || {
            let Some(ui) = weak.upgrade() else { return };
            if let Err(e) = start_google_sign_in(&ui, &mail) {
                // Already on screen for the "no client id" case, because the note is drawn from
                // `email.accounts`. Said again here so a refusal from a click is never silent.
                ui.set_setup_ok(false);
                ui.set_setup_status(e.clone().into());
                say(&ui, e);
            }
        });
    }
    {
        let weak = app.as_weak();
        let mail = mail.clone();
        app.on_cancel_google_sign_in(move || {
            let Some(ui) = weak.upgrade() else { return };
            cancel_google_sign_in(&ui, &mail);
        });
    }

    {
        let weak = app.as_weak();
        let mail = mail.clone();
        app.on_cancel_setup(move || {
            let Some(ui) = weak.upgrade() else { return };
            // A setup form closed while a Google sign-in is in flight leaves a listener open on
            // this machine and a consent page in a browser that leads nowhere. Closed properly.
            if mail.google_flow.borrow().is_some() {
                cancel_google_sign_in(&ui, &mail);
            }
            ui.set_setup_open(false);
            // Not kept anywhere, on purpose: a password left in a property is a password in the
            // process for as long as the window is open.
            ui.set_setup_password(SharedString::default());
            ui.set_setup_status(SharedString::default());
            ui.set_setup_ok(false);
        });
    }

    // ── Test connection, and save the account ──
    //
    // Both take all eight fields of a form that used to log them and return. Both run on a
    // worker thread: they sign in to a mail server, which is seconds, and a form that freezes
    // the window it is on is its own kind of lie about what is happening.
    //
    // Neither is on the control surface. The password is the most sensitive thing this app
    // handles, and an action that accepted one would put it in a transcript a mind can read.
    {
        let weak = app.as_weak();
        let mail = mail.clone();
        app.on_test_connection(move |email, password, provider, imap_server, imap_port| {
            let Some(ui) = weak.upgrade() else { return };
            if mail.setting_up.get() {
                return;
            }
            // Test Connection is given five of the eight fields by the screen. The SMTP half is
            // read off the window, so both halves of the account are tried by the one button.
            let settings = match state::account_settings_from_form(
                &email,
                &password,
                &ui.get_setup_display_name(),
                &provider,
                &imap_server,
                &imap_port,
                &ui.get_setup_smtp_server(),
                &ui.get_setup_smtp_port(),
            ) {
                Ok(settings) => settings,
                Err(e) => {
                    ui.set_setup_ok(false);
                    ui.set_setup_status(e.into());
                    return;
                }
            };

            mail.setting_up.set(true);
            ui.set_setup_testing(true);
            ui.set_setup_status("Signing in\u{2026}".into());
            ui.set_setup_ok(false);
            let back = ui.as_weak();
            std::thread::spawn(move || {
                let outcome = test_account_via_service(&settings);
                let _ = back.upgrade_in_event_loop(move |ui| {
                    with_mail(|m| m.setting_up.set(false));
                    ui.set_setup_testing(false);
                    match outcome {
                        Ok(result) => {
                            ui.set_setup_ok(result.imap_ok && result.smtp_ok);
                            ui.set_setup_status(
                                state::test_summary(
                                    result.imap_ok,
                                    &result.imap,
                                    result.smtp_ok,
                                    &result.smtp,
                                )
                                .into(),
                            );
                        }
                        Err(e) => {
                            ui.set_setup_ok(false);
                            ui.set_setup_status(format!("The mail service could not try it: {e}").into());
                        }
                    }
                });
            });
        });
    }
    {
        let weak = app.as_weak();
        let mail = mail.clone();
        app.on_save_account(
            move |email, password, display_name, provider, imap_server, imap_port, smtp_server, smtp_port| {
                let Some(ui) = weak.upgrade() else { return };
                if mail.setting_up.get() {
                    return;
                }
                let settings = match state::account_settings_from_form(
                    &email,
                    &password,
                    &display_name,
                    &provider,
                    &imap_server,
                    &imap_port,
                    &smtp_server,
                    &smtp_port,
                ) {
                    Ok(settings) => settings,
                    Err(e) => {
                        ui.set_setup_ok(false);
                        ui.set_setup_status(e.into());
                        return;
                    }
                };

                mail.setting_up.set(true);
                ui.set_setup_testing(true);
                ui.set_setup_status("Signing in\u{2026}".into());
                ui.set_setup_ok(false);
                let back = ui.as_weak();
                std::thread::spawn(move || {
                    // The service signs in before it writes anything, so an account that lands in
                    // the config file is one that worked at least once.
                    let outcome = save_account_via_service(&settings);
                    let _ = back.upgrade_in_event_loop(move |ui| {
                        let Some(mail) = with_mail(|m| m.clone()) else { return };
                        mail.setting_up.set(false);
                        ui.set_setup_testing(false);
                        match outcome {
                            Ok(saved) => {
                                ui.set_setup_ok(true);
                                ui.set_setup_status(format!("Signed in as {}", saved.email).into());
                                // The password leaves the window the moment it is not needed.
                                ui.set_setup_password(SharedString::default());
                                ui.set_setup_open(false);
                                // Through the sync, which runs on a worker: loading a mailbox
                                // here would hold the UI thread for as long as the mail server
                                // takes, right after a button press.
                                ui.invoke_sync_emails();
                            }
                            Err(e) => {
                                // The form stays up, holding what was typed, with the mail
                                // server's own answer above it.
                                ui.set_setup_ok(false);
                                ui.set_setup_status(e.into());
                            }
                        }
                    });
                });
            },
        );
    }
}

/// What is in the composer right now.
fn draft_on_screen(ui: &EmailApp) -> Draft {
    Draft {
        to: ui.get_compose_to().to_string(),
        cc: ui.get_compose_cc().to_string(),
        bcc: ui.get_compose_bcc().to_string(),
        subject: ui.get_compose_subject().to_string(),
        body: ui.get_compose_body().to_string(),
    }
}

/// The folder this account archives into.
///
/// Read off the folder list the mail server gave us, because "[Gmail]/All Mail" — which was
/// hardcoded — exists on exactly one provider. `Archive` is the IMAP convention and what
/// everyone else uses.
fn archive_folder(ui: &EmailApp) -> String {
    let folders = ui.get_folders();
    (0..folders.row_count())
        .filter_map(|i| folders.row_data(i))
        .find(|f| f.folder_type == "archive")
        .map(|f| f.name.to_string())
        .unwrap_or_else(|| "Archive".to_string())
}

/// Design fixture: `YANTRIK_EMAIL_DEMO=1` fills the three panes with realistic sample mail so
/// the UI can be judged (and screenshotted) without an account or the email service running.
mod demo {
    use super::*;

    fn color(r: u8, g: u8, b: u8) -> slint::Color { slint::Color::from_rgb_u8(r, g, b) }

    pub fn populate(app: &EmailApp) {
        // `Some` counts are the server's; `None` is a folder it did not count, which the
        // sidebar marks rather than badges with zero.
        let folders = [
            ("Inbox", "inbox", Some((3, 128))), ("Starred", "starred", Some((0, 9))),
            ("Sent", "sent", Some((0, 341))), ("Drafts", "drafts", Some((0, 2))),
            ("Archive", "archive", Some((0, 2210))), ("Spam", "spam", Some((12, 12))),
            ("Trash", "trash", Some((0, 40))), ("Receipts", "custom", None),
        ];
        let folders: Vec<EmailFolderData> = folders.iter().enumerate().map(|(i, (n, t, counts))| EmailFolderData {
            name: (*n).into(), icon: SharedString::default(),
            unread_count: counts.map(|(u, _)| u).unwrap_or(0),
            total_count: counts.map(|(_, c)| c).unwrap_or(0),
            counts_known: counts.is_some(),
            counts_reason: if counts.is_some() { SharedString::default() } else {
                "the mail server did not report it: No Response: STATUS timed out".into()
            },
            is_selected: i == 0, folder_type: (*t).into(),
        }).collect();
        app.set_folders(ModelRc::new(VecModel::from(folders)));

        let rows = [
            ("Priya R.", "priya@example.com", "Kernel perception API — review notes", "Left comments on the observation tiers doc. The event-driven path looks right; two questions on the commit gate…", "09:42", false, true, true, 4, "P", (0x4E, 0x79, 0xA7)),
            ("GitHub", "noreply@github.com", "[yantrik-os] CI passed on rebase/yantrikdb-0.10", "All 212 checks passed. Build time 6m 12s (fast profile).", "09:10", false, false, false, 1, "G", (0x59, 0xA1, 0x4F)),
            ("Ananya S.", "ananya@example.com", "Re: Launcher grid — category rail", "Agree on 5 columns. Can we keep the search box pinned when the grid scrolls?", "Yesterday", false, false, false, 3, "A", (0xE1, 0x57, 0x59)),
            ("Example Hosting", "billing@example.net", "Invoice 0000-0001 for September", "Your invoice for your server is attached. Amount due: €10.00.", "Yesterday", true, false, true, 1, "H", (0xF2, 0x8E, 0x2B)),
            ("Marcus W.", "marcus@example.org", "Talk proposal: AI-native desktops", "Would you be up for a 25-minute slot in the systems track? Abstract deadline is the 19th.", "Tue", true, true, false, 2, "M", (0x76, 0xB7, 0xB2)),
            ("Slint Weekly", "weekly@example.org", "1.17: software renderer partial repaints", "This release brings dirty-region repaints to the software renderer and a new Path API…", "Mon", true, false, false, 1, "S", (0xB0, 0x7A, 0xA1)),
            ("Ravi K.", "ravi@example.com", "Fonts landed on the design tokens crate", "Barlow at 400/500/600 and JetBrains Mono 400/500 are embedded now; no more DejaVu fallback.", "Mon", true, false, false, 1, "R", (0x9C, 0x75, 0x5F)),
        ];
        let list: Vec<EmailListItem> = rows.iter().enumerate().map(|(i, r)| EmailListItem {
            id: i as i32 + 1, from_name: r.0.into(), from_addr: r.1.into(), subject: r.2.into(),
            preview: r.3.into(), date_text: r.4.into(), is_read: r.5, is_flagged: r.6, is_selected: i == 0,
            has_attachment: r.7, thread_count: r.8, thread_id: format!("t{i}").into(),
            avatar_initial: r.9.into(), avatar_color: color(r.10 .0, r.10 .1, r.10 .2),
        }).collect();
        app.set_email_list(ModelRc::new(VecModel::from(list)));

        app.set_email_detail(EmailDetailData {
            id: 1,
            from_name: "Priya R.".into(),
            from_addr: "priya@example.com".into(),
            from_initial: "P".into(),
            from_avatar_color: color(0x4E, 0x79, 0xA7),
            to_addr: "you@example.com".into(),
            cc_addr: "ananya@example.com".into(),
            subject: "Kernel perception API — review notes".into(),
            date_text: "Today, 09:42".into(),
            body: "Hi Alex,

Went through the observation-tier design end to end. The split between the AT-SPI event stream, the compositor damage hints and the on-demand vision tier is the right shape — most of what an agent needs is in tier one, and the model only pays for pixels when something is opaque.

Two questions before I sign off:

1. Commit gate. When the browser tier proposes an action on an indexed element, who owns the timeout? If the page re-renders between index and commit, the index is stale — do we re-index or reject?

2. Per-app scopes. The scoped-control token is minted by the shell, but revocation seems to happen only on app exit. A long-running terminal keeps its grant forever.

Small thing: the world model's epistemic states read well. \"Believed\" vs \"observed\" is exactly the distinction the planner needs.

— Priya".into(),
            ai_summary: SharedString::default(),
            is_flagged: true,
            is_read: true,
            has_attachment: true,
            attachment_names: "perception-tiers-v3.pdf, commit-gate.png".into(),
            thread_count: 4,
            // The demo body is plain text, so there is no HTML original to open.
            has_html: false,
        });
        app.set_email_attachments(ModelRc::new(VecModel::from(vec![
            EmailAttachmentData { name: "perception-tiers-v3.pdf".into(), size_text: "412 KB".into(), mime_type: "application/pdf".into(), is_downloaded: true },
            EmailAttachmentData { name: "commit-gate.png".into(), size_text: "88 KB".into(), mime_type: "image/png".into(), is_downloaded: false },
        ])));
        app.set_email_thread_messages(ModelRc::new(VecModel::from(vec![
            EmailThreadMessage { id: 11, from_name: "Alex".into(), from_addr: "you@example.com".into(), date_text: "Mon, 14:05".into(), body: "Sharing v3 of the perception tiers doc. Main change: tier two is event-driven now.".into(), is_collapsed: true },
            EmailThreadMessage { id: 12, from_name: "Ananya S.".into(), from_addr: "ananya@example.com".into(), date_text: "Mon, 16:40".into(), body: "The damage-hint idea is neat. Does labwc expose that today or do we need a protocol extension?".into(), is_collapsed: true },
            EmailThreadMessage { id: 13, from_name: "Alex".into(), from_addr: "you@example.com".into(), date_text: "Tue, 08:12".into(), body: "wlr-screencopy gives us damage regions per frame; no extension needed.".into(), is_collapsed: true },
            EmailThreadMessage { id: 14, from_name: "Priya R.".into(), from_addr: "priya@example.com".into(), date_text: "Today, 09:42".into(), body: app.get_email_detail().body, is_collapsed: false },
        ])));

        app.set_account_name("you@example.com".into());
        app.set_email_folder_total(128);
        app.set_email_folder_unread(3);
        app.set_email_folder_counts_known(true);
        app.set_email_sync_status("Synced 2 min ago".into());
        app.set_has_account(true);
    }
}

#[cfg(test)]
mod enhance_tests {
    /// The Enhance buttons used to hand the composer's body to the callback, and the handler —
    /// which reads the body from the composer itself — put it into the prompt a second time, so
    /// every rewrite request carried the email twice. The defect lives in the markup, so the
    /// markup is what this pins: a button may name the tone, never the body.
    #[test]
    fn enhance_buttons_pass_a_tone_and_never_the_body() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../crates/yantrik-ui-slint/ui/email.slint"
        );
        let src = std::fs::read_to_string(path).expect("the shared email markup");
        let calls: Vec<&str> = src.lines().filter(|l| l.contains("enhance-text(")).collect();
        assert!(calls.len() >= 4, "the four Enhance buttons should call enhance-text");
        for call in &calls {
            assert!(
                !call.contains("compose-body"),
                "an Enhance button hands the body to the handler, which sends it twice: {call}"
            );
        }
    }
}
