//! The half of the Email app that has no window in it.
//!
//! Three things live here, and they are here because each one was wrong in a way that could not
//! be seen from either the screen or the service on its own.
//!
//! **Which of three states the app is in.** A dead service and an unconfigured machine used to
//! be the same picture: `list_folders` returned `None` for both, and `None` meant
//! `set_has_account(false)`, which draws the onboarding form. An audit looked at that form and
//! concluded the machine had no mail account. It may never have been about an account at all —
//! `email-service` is registered `autostart: false` and nothing started it, so every call failed
//! at connect on every machine, every time. [`MailState`] is the distinction, made once.
//!
//! **Which message a caller means.** `open_message which=1` asked for `args["which"].as_str()`,
//! got `""` from a number, and answered `nothing in this folder matches ""` — a sentence with a
//! quoted nothing in it. [`resolve_which`] takes a row number or text.
//!
//! **What happens to a half-written message.** Closing the composer threw it away and Save Draft
//! logged a line. [`Draft`] is a file.
//!
//! Nothing in this file imports Slint or opens a socket, so `tests/email-core` can exercise all
//! of it on a machine with no mailbox.

use std::path::{Path, PathBuf};

use yantrik_ipc_contracts::email::{
    AccountSettings, AccountsResult, EmailAccountSummary, EmailFolder,
    FolderCounts as WireCounts, GoogleSignIn, OAuthStatus,
};

// ── The three states ─────────────────────────────────────────────────

/// What the app has been told about the mail service, and about the account behind it.
#[derive(Debug, Clone, PartialEq)]
pub enum MailState {
    /// The service could not be started or could not be answered. Carries the reason, which is
    /// the thing that used to be thrown away.
    Unreachable { reason: String },
    /// The service answered, and holds no account.
    NoAccount {
        config_path: String,
        secrets_are_plaintext: bool,
        google: GoogleSignIn,
    },
    /// The service answered, and holds at least one.
    Ready {
        account: EmailAccountSummary,
        config_path: String,
        secrets_are_plaintext: bool,
        google: GoogleSignIn,
    },
}

impl MailState {
    /// What `describe` reports under `service`.
    pub fn service_word(&self) -> &'static str {
        match self {
            MailState::Unreachable { .. } => "unreachable",
            _ => "up",
        }
    }

    /// Whether an account is configured — or `None`, which is the honest answer when the service
    /// could not be reached.
    ///
    /// `false` would be the original lie in a new place: with nothing answering, this app has not
    /// been told whether an account exists, and saying it has none is a claim it cannot support.
    pub fn has_account(&self) -> Option<bool> {
        match self {
            MailState::Unreachable { .. } => None,
            MailState::NoAccount { .. } => Some(false),
            MailState::Ready { .. } => Some(true),
        }
    }

    /// The failure a person should see on screen and a mind should read in `describe.notice`.
    /// Empty when there is nothing wrong.
    pub fn notice(&self) -> String {
        match self {
            MailState::Unreachable { reason } => {
                format!("The mail service could not be reached: {reason}")
            }
            _ => String::new(),
        }
    }

    /// The one line a caller surveying every window pays for.
    pub fn summary(&self) -> String {
        match self {
            MailState::Unreachable { reason } => {
                format!("Email — the mail service could not be reached: {reason}")
            }
            MailState::NoAccount { config_path, .. } => {
                format!("Email — the mail service is running and no account is configured in {config_path}")
            }
            MailState::Ready { account, .. } => format!("Email — {}", account.email),
        }
    }

    pub fn config_path(&self) -> &str {
        match self {
            MailState::Unreachable { .. } => "",
            MailState::NoAccount { config_path, .. } => config_path,
            MailState::Ready { config_path, .. } => config_path,
        }
    }

    pub fn secrets_are_plaintext(&self) -> bool {
        match self {
            MailState::Unreachable { .. } => false,
            MailState::NoAccount { secrets_are_plaintext, .. } => *secrets_are_plaintext,
            MailState::Ready { secrets_are_plaintext, .. } => *secrets_are_plaintext,
        }
    }

    /// Whether a Google sign-in can be started, in the mail service's own words.
    ///
    /// Unreachable is *not available*, and the note says which of the two reasons it is. The
    /// distinction matters because the two look identical on the screen otherwise: a machine with
    /// no Google OAuth client and a machine whose mail service is not running both draw no
    /// button, and only one of them is fixed by setting a client id.
    pub fn google_sign_in(&self) -> GoogleSignIn {
        match self {
            MailState::Unreachable { .. } => GoogleSignIn {
                available: false,
                note: "The mail service is not running, so this app cannot tell whether Google \
                       sign-in is available on this build."
                    .to_string(),
            },
            MailState::NoAccount { google, .. } => google.clone(),
            MailState::Ready { google, .. } => google.clone(),
        }
    }

    pub fn account_id(&self) -> String {
        match self {
            MailState::Ready { account, .. } => account.id.clone(),
            _ => String::new(),
        }
    }

    pub fn account_name(&self) -> String {
        match self {
            MailState::Ready { account, .. } => account.email.clone(),
            _ => String::new(),
        }
    }
}

/// Turn the service's answer into the state the app is in.
///
/// The `Err` arm is everything between this process and an answer: the shell refusing to start
/// the service, the service not coming up inside its budget, a socket that is not there, a
/// method an older binary does not have. All of them are "unreachable, and here is why" — none
/// of them is "no account".
pub fn decide(answer: Result<AccountsResult, String>) -> MailState {
    match answer {
        Err(reason) => MailState::Unreachable { reason },
        Ok(result) => {
            let google = result.google_sign_in;
            match result.accounts.into_iter().next() {
                None => MailState::NoAccount {
                    config_path: result.config_path,
                    secrets_are_plaintext: result.secrets_are_plaintext,
                    google,
                },
                Some(account) => MailState::Ready {
                    account,
                    config_path: result.config_path,
                    secrets_are_plaintext: result.secrets_are_plaintext,
                    google,
                },
            }
        }
    }
}

// ── What the header says about the open folder ───────────────────────

/// The two numbers in "INBOX, 12 unread of 35", as the mail server counts them.
///
/// They come from the folder list — the same list `describe` prints two lines under the header
/// — and from nowhere else. They used to be counted over the page of messages in hand: "9 unread
/// of 21" for a folder the list beside it said held 35, where 21 was one page and 9 the unread
/// among those, so one reply gave two answers to one question (#74, #123). The comment on that
/// code already said the counts were "of the folder, not of the tab"; now they are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FolderCounts {
    pub unread: i32,
    pub total: i32,
}

impl From<WireCounts> for FolderCounts {
    fn from(c: WireCounts) -> Self {
        FolderCounts { unread: c.unread, total: c.total }
    }
}

/// What the header can say about the open folder: its two numbers, or that it has none.
///
/// A folder whose counts the mail server would not give — a STATUS it refused, a reply that did
/// not come — used to reach the header as `0 unread of 0`, which is what an empty folder says
/// too, so a failure to read the mailbox was reported as a fact about it (#131). The list now
/// carries "not counted" as its own value, and the header carries it through rather than
/// inventing a zero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Counted {
    /// The server's numbers — or, for a folder the list does not carry, what is in hand.
    Known(FolderCounts),
    /// The folder is listed and the server did not count it this time; `reason` is why, in the
    /// list's own words.
    Unavailable { reason: String },
}

impl Counted {
    /// The counts for `folder`, off the list the server gave.
    ///
    /// A folder the list does not have — a mailbox opened by a name LIST did not return — has no
    /// server count, and then the page in hand is all there is: `loaded` messages, `loaded_unread`
    /// of them unread. A lower bound, and the only honest number left; there is no row in the
    /// list for it to contradict. A folder the list *does* have, uncounted, is a different thing:
    /// the server was asked and did not say, and the page in hand is not offered in place of an
    /// answer it refused.
    pub fn of(folders: &[EmailFolder], folder: &str, loaded_unread: usize, loaded: usize) -> Self {
        match folders.iter().find(|f| f.name.eq_ignore_ascii_case(folder)) {
            Some(EmailFolder { counts: Some(counts), .. }) => Counted::Known((*counts).into()),
            Some(EmailFolder { reason, .. }) => Counted::Unavailable {
                reason: reason
                    .clone()
                    .unwrap_or_else(|| "the mail server did not count this folder".to_string()),
            },
            None => Counted::Known(FolderCounts { unread: loaded_unread as i32, total: loaded as i32 }),
        }
    }

    /// The counts, if there are any.
    pub fn known(&self) -> Option<FolderCounts> {
        match self {
            Counted::Known(counts) => Some(*counts),
            Counted::Unavailable { .. } => None,
        }
    }

    /// The same folder after a change the server agreed to, when there are numbers to change.
    /// Counts that were never read stay unread: one message fewer than "not known" is still not
    /// known.
    pub fn after(self, change: impl FnOnce(FolderCounts) -> FolderCounts) -> Self {
        match self {
            Counted::Known(counts) => Counted::Known(change(counts)),
            unavailable => unavailable,
        }
    }
}

impl FolderCounts {
    /// The same folder after one of its messages was marked read or unread, and the mail server
    /// agreed. Reading a message marks it read; a Refresh asks the server again and replaces
    /// this, but between the two the header must not say twelve unread over a list with eleven
    /// unread dots in it.
    pub fn after_read_change(self, was_read: bool, now_read: bool) -> Self {
        let unread = match (was_read, now_read) {
            (false, true) => self.unread - 1,
            (true, false) => self.unread + 1,
            _ => self.unread,
        };
        FolderCounts { unread: unread.max(0), total: self.total }
    }

    /// The same folder after a message left it — deleted, or moved elsewhere — and the server
    /// confirmed it is gone.
    pub fn after_removal(self, was_read: bool) -> Self {
        FolderCounts {
            unread: if was_read { self.unread } else { (self.unread - 1).max(0) },
            total: (self.total - 1).max(0),
        }
    }

    /// The folder a message was moved into, after it arrived.
    pub fn after_arrival(self, is_read: bool) -> Self {
        FolderCounts {
            unread: if is_read { self.unread } else { self.unread + 1 },
            total: self.total + 1,
        }
    }
}

/// The one line over a folder that is open, when nothing is being read or written.
///
/// With a search on, the list under the header is the results and not the folder, and the
/// header says so rather than putting the folder's counts over a list they do not describe.
/// With no counts, it says that — and why — rather than "0 unread of 0", which is a different
/// claim and, for this folder, an unverified one.
pub fn folder_summary(folder: &str, counts: &Counted, search: Option<(&str, usize)>) -> String {
    match (search, counts) {
        (Some((query, hits)), _) => format!(
            "Email — {folder}, {hits} {} for \u{201c}{query}\u{201d}",
            if hits == 1 { "result" } else { "results" }
        ),
        (None, Counted::Known(counts)) => {
            format!("Email — {folder}, {} unread of {}", counts.unread, counts.total)
        }
        (None, Counted::Unavailable { reason }) => {
            format!("Email — {folder}, counts unavailable ({reason})")
        }
    }
}

// ── Which message a caller means ─────────────────────────────────────

/// As much of a listed message as naming one needs.
#[derive(Debug, Clone, Default)]
pub struct MessageRow {
    pub subject: String,
    pub from_name: String,
    pub from_addr: String,
}

impl MessageRow {
    pub fn new(subject: &str, from_name: &str, from_addr: &str) -> Self {
        Self {
            subject: subject.to_string(),
            from_name: from_name.to_string(),
            from_addr: from_addr.to_string(),
        }
    }
}

/// The row a caller meant, or a refusal that names what it asked for.
///
/// A number is a position in the list as it is shown, counting from one, because that is how a
/// caller that has just read `describe.messages` refers to what it read. Text is matched against
/// the subject, the sender's name and their address, exact before partial, because that is how a
/// person names mail.
///
/// No refusal here quotes an empty string. `nothing in this folder matches ""` was the answer to
/// `open_message which=1`, and it is worse than useless: it reports a search for nothing, which
/// is not what was asked, so the caller retries the same call.
pub fn resolve_which(rows: &[MessageRow], which: &str) -> Result<usize, String> {
    let want = which.trim();
    if want.is_empty() {
        return Err("no message was named: give a row number, or part of a subject or sender"
            .to_string());
    }

    // A number is a row number and nothing else. It is deliberately not tried as text
    // afterwards: "9" is a substring of "Invoice R0093", so a caller that asked for the ninth
    // message of a folder holding three would be handed the second one and told nothing. A
    // subject that is only digits is reachable by any other part of it, or by its sender.
    if let Ok(n) = want.parse::<i64>() {
        if n < 1 {
            return Err(format!("row numbers count from 1, so there is no message {n}"));
        }
        let n = n as usize;
        if n <= rows.len() {
            return Ok(n - 1);
        }
        return Err(match rows.len() {
            0 => format!("there is no message {n}: this folder is showing none"),
            1 => format!("there is no message {n}: this folder is showing 1"),
            many => format!("there is no message {n}: this folder is showing {many}"),
        });
    }

    let lower = want.to_lowercase();
    let matches = |row: &MessageRow, exact: bool| {
        let subject = row.subject.to_lowercase();
        let from = row.from_name.to_lowercase();
        let addr = row.from_addr.to_lowercase();
        if exact {
            subject == lower || from == lower || addr == lower
        } else {
            subject.contains(&lower) || from.contains(&lower) || addr.contains(&lower)
        }
    };
    if let Some(at) = rows.iter().position(|r| matches(r, true)) {
        return Ok(at);
    }
    if let Some(at) = rows.iter().position(|r| matches(r, false)) {
        return Ok(at);
    }

    Err(format!("nothing in this folder matches \u{201c}{want}\u{201d}"))
}

// ── The triage tabs ──────────────────────────────────────────────────

/// Which of the listed messages the tabs above the list keep.
///
/// There were four tabs and the fourth was Priority, which nothing in this app or its wire could
/// have computed: no message carries an importance, and `email-priority-count` was an `in`
/// property nothing ever set, so the badge beside it was always absent and the tab always empty.
/// It is gone. These three are filters over the rows already in hand, which is why they can be
/// applied without asking the mail server anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Triage {
    All,
    Unread,
    Flagged,
}

impl Triage {
    pub fn from_index(index: i32) -> Option<Triage> {
        match index {
            0 => Some(Triage::All),
            1 => Some(Triage::Unread),
            2 => Some(Triage::Flagged),
            _ => None,
        }
    }

    pub fn index(&self) -> i32 {
        match self {
            Triage::All => 0,
            Triage::Unread => 1,
            Triage::Flagged => 2,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Triage::All => "All",
            Triage::Unread => "Unread",
            Triage::Flagged => "Flagged",
        }
    }

    pub fn keeps(&self, is_read: bool, is_flagged: bool) -> bool {
        match self {
            Triage::All => true,
            Triage::Unread => !is_read,
            Triage::Flagged => is_flagged,
        }
    }
}

// ── The draft that used to be lost ───────────────────────────────────

/// A message that was being written when the composer was closed.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Draft {
    #[serde(default)]
    pub to: String,
    #[serde(default)]
    pub cc: String,
    #[serde(default)]
    pub bcc: String,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub body: String,
}

impl Draft {
    /// Nothing was typed. A draft of nothing is not kept, so reopening the composer after
    /// cancelling an empty one does not restore an empty one.
    pub fn is_empty(&self) -> bool {
        self.to.trim().is_empty()
            && self.cc.trim().is_empty()
            && self.bcc.trim().is_empty()
            && self.subject.trim().is_empty()
            && self.body.trim().is_empty()
    }
}

/// Where the unsent message lives: one file, beside the rest of this OS's per-app state.
pub fn draft_path() -> PathBuf {
    if let Ok(explicit) = std::env::var("YANTRIK_EMAIL_DRAFT") {
        return PathBuf::from(explicit);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".local/share/yantrik/email/draft.json")
}

pub fn save_draft(path: &Path, draft: &Draft) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("could not make {}: {e}", dir.display()))?;
    }
    let text = serde_json::to_string_pretty(draft)
        .map_err(|e| format!("could not write the draft: {e}"))?;
    std::fs::write(path, text).map_err(|e| format!("could not write {}: {e}", path.display()))
}

/// The kept draft, or `None` when there is not one. A file that is there and unreadable is an
/// error rather than a shrug: a draft silently treated as absent is the loss this exists to stop.
pub fn load_draft(path: &Path) -> Result<Option<Draft>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("could not read {}: {e}", path.display())),
    };
    if text.trim().is_empty() {
        return Ok(None);
    }
    let draft: Draft = serde_json::from_str(&text)
        .map_err(|e| format!("{} is not a draft: {e}", path.display()))?;
    if draft.is_empty() {
        Ok(None)
    } else {
        Ok(Some(draft))
    }
}

pub fn clear_draft(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("could not remove {}: {e}", path.display())),
    }
}

// ── The setup form ───────────────────────────────────────────────────

/// The servers a named provider uses, as `(imap, imap_port, smtp, smtp_port)`.
///
/// The provider buttons on the setup screen hide the server fields, so a form filled in with
/// Gmail selected sends four empty strings for them. Something has to know these; it is here
/// rather than in the service because the service should be given settings, not a brand name.
pub fn servers_for(provider: &str) -> Option<(&'static str, u16, &'static str, u16)> {
    match provider.trim().to_lowercase().as_str() {
        "gmail" => Some(("imap.gmail.com", 993, "smtp.gmail.com", 587)),
        "outlook" => Some(("outlook.office365.com", 993, "smtp.office365.com", 587)),
        "yahoo" => Some(("imap.mail.yahoo.com", 993, "smtp.mail.yahoo.com", 587)),
        "icloud" => Some(("imap.mail.me.com", 993, "smtp.mail.me.com", 587)),
        _ => None,
    }
}

/// Turn what was typed into settings the service can act on, or say what is missing.
///
/// Every refusal here names a field. None of them contains the password — not the value, and not
/// a fragment of it — which is the rule the whole of this app's error handling is written to,
/// and which `tests/email-core` checks by building one with a sentinel and looking for it in
/// every string this module can produce.
#[allow(clippy::too_many_arguments)]
pub fn account_settings_from_form(
    email: &str,
    password: &str,
    display_name: &str,
    provider: &str,
    imap_server: &str,
    imap_port: &str,
    smtp_server: &str,
    smtp_port: &str,
) -> Result<AccountSettings, String> {
    let email = email.trim();
    if email.is_empty() {
        return Err("Enter the email address of the account.".to_string());
    }
    let (user, host) = email.split_once('@').ok_or_else(|| {
        format!("\u{201c}{email}\u{201d} is not an email address \u{2014} it has no @ in it.")
    })?;
    if user.is_empty() || host.is_empty() || !host.contains('.') {
        return Err(format!("\u{201c}{email}\u{201d} is not a complete email address."));
    }
    if password.is_empty() {
        return Err("Enter the password or app password for this account.".to_string());
    }

    let (imap_server, imap_port, smtp_server, smtp_port) = match servers_for(provider) {
        Some((imap, imap_p, smtp, smtp_p)) => {
            (imap.to_string(), imap_p, smtp.to_string(), smtp_p)
        }
        None => {
            let imap = imap_server.trim();
            let smtp = smtp_server.trim();
            if imap.is_empty() || smtp.is_empty() {
                return Err(
                    "Choose a provider, or choose Advanced and enter the IMAP and SMTP server \
                     names."
                        .to_string(),
                );
            }
            (
                imap.to_string(),
                parse_port(imap_port, "IMAP")?,
                smtp.to_string(),
                parse_port(smtp_port, "SMTP")?,
            )
        }
    };

    Ok(AccountSettings {
        email: email.to_string(),
        display_name: display_name.trim().to_string(),
        provider: provider.trim().to_string(),
        imap_server,
        imap_port,
        smtp_server,
        smtp_port,
        password: password.to_string(),
    })
}

fn parse_port(text: &str, which: &str) -> Result<u16, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err(format!("Enter the {which} port."));
    }
    text.parse::<u16>()
        .ok()
        .filter(|p| *p > 0)
        .ok_or_else(|| format!("\u{201c}{text}\u{201d} is not a {which} port number."))
}

/// What the setup screen says once both halves have been tried.
///
/// Both are reported whichever way each went: an account whose IMAP works and whose SMTP does
/// not can read mail and cannot send it, and "Connected!" for that is a smaller version of the
/// same fabrication as "Synced".
pub fn test_summary(imap_ok: bool, imap: &str, smtp_ok: bool, smtp: &str) -> String {
    match (imap_ok, smtp_ok) {
        (true, true) => format!("Signed in. {imap} \u{00b7} {smtp}"),
        (true, false) => format!("Mail can be read but not sent. {smtp}"),
        (false, true) => format!("Mail can be sent but not read. {imap}"),
        (false, false) => format!("{imap} \u{00b7} {smtp}"),
    }
}

// ── The Google sign-in, as the window sees it ────────────────────────

/// What the setup screen does with one answer from `email.oauth_status`.
///
/// The polling loop asks a question every second and most answers are "not yet". This is the
/// decision about which of them ends the wait, made once and in a place with no Slint in it —
/// because the failure mode it exists to stop is a window that sits on "Waiting for Google…"
/// forever because the answer that would have ended it was not one of the shapes the loop knew.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoogleOutcome {
    /// Still going. Ask again.
    Waiting,
    /// Signed in, and the account is in the service's file. Carries the address it signed in as
    /// — which came from Google, not from anything typed into this window.
    SignedIn(String),
    /// Over, and not signed in. The sentence goes on the screen *and* into `describe.notice`,
    /// because a sign-in that quietly stopped is the same silence this app spent September
    /// removing from everything else.
    Stopped(String),
}

/// Turn one status answer into that decision.
///
/// The `Err` arm is everything between this window and an answer: the service went away, the
/// socket closed, the method is missing on an older binary, or the service has forgotten a flow it
/// once had. It is a stop rather than a retry, because the alternative is a window polling a
/// service that cannot answer until its own deadline and then saying nothing useful about why.
///
/// The wrapper is worded for both — a refusal *is* an answer — so the sentence is true whichever
/// of the two happened, and the service's own words come after it either way.
pub fn google_outcome(answer: Result<OAuthStatus, String>) -> GoogleOutcome {
    match answer {
        Ok(OAuthStatus::Waiting) => GoogleOutcome::Waiting,
        Ok(OAuthStatus::Done { account }) => GoogleOutcome::SignedIn(account.email),
        Ok(OAuthStatus::Failed { reason }) => GoogleOutcome::Stopped(reason),
        Err(e) => GoogleOutcome::Stopped(format!(
            "The mail service could not say how the Google sign-in went: {e}"
        )),
    }
}

/// What the screen says when the app could not open a browser for the consent page.
///
/// Not a dead end and not a log line: the address is put on the screen beside this, because on a
/// machine with no `xdg-open` handler that is the only way through — and a person who can read
/// the URL can finish the sign-in from a phone.
pub fn browser_failed_note(reason: &str) -> String {
    format!(
        "This machine could not open a browser for the Google sign-in ({reason}). The address is \
         below \u{2014} open it anywhere signed in to the right Google account, and this window \
         will notice when it comes back."
    )
}

/// The sentence the setup screen carries under the password field.
///
/// It is a function rather than a line of Slint so that it changes with the service's own answer:
/// the day a secret store exists, `secrets_are_plaintext` goes false and this stops claiming
/// something that is no longer true.
pub fn password_storage_note(config_path: &str, plaintext: bool) -> String {
    if plaintext {
        format!(
            "This password is stored in clear text in {config_path}, readable only by you \
             (mode 0600). This machine has no secret store the mail service can reach yet."
        )
    } else {
        format!("This password is stored by the mail service, which reads {config_path}.")
    }
}

// ── The HTML original, as the browser gets it ────────────────────────
//
// "Open original" hands the sender's HTML to a browser, for the mail whose layout the reading
// pane's text cannot carry. The file it hands over is cleaned first, and every decision about
// that cleaning is here, where `tests/email-core` can exercise it without a window.

/// Take out of `html` every `<img>` that would send a request over the network, and say how
/// many went.
///
/// Opening tracking: notification mail is full of images that show nothing and exist so that
/// the sender learns the message was opened, when, and from which address — and a browser will
/// happily report all of it for a click that only meant "let me see the layout". Pictures the
/// mail carries inline (`cid:`, `data:`) never leave the machine and stay.
///
/// This is a scanner, not an HTML parser: it finds `<img` tags, reads their `src` and `srcset`,
/// and drops the tag when either points at the network. That is enough for the one job this
/// has — no pixel of the sender's may fetch — and a tag too malformed for the scanner to read
/// is a tag a browser will not render either.
pub fn strip_remote_images(html: &str) -> (String, usize) {
    let chars: Vec<char> = html.chars().collect();
    let mut out = String::with_capacity(html.len());
    let mut removed = 0;
    let mut i = 0;
    while i < chars.len() {
        if !is_img_tag_start(&chars, i) {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let end = img_tag_end(&chars, i);
        let tag: String = chars[i..end].iter().collect();
        if img_is_remote(&tag) {
            removed += 1;
        } else {
            out.push_str(&tag);
        }
        i = end;
    }
    (out, removed)
}

/// True at the `<` of an `<img` tag, case-insensitively, and not at `<image` or `<imgx`: what
/// follows the name has to end it — a space, the tag's own `>`, or a `/`.
fn is_img_tag_start(chars: &[char], i: usize) -> bool {
    if chars[i] != '<' {
        return false;
    }
    let name = &chars[i + 1..std::cmp::min(i + 4, chars.len())];
    if !name.iter().zip("img".chars()).all(|(a, b)| a.to_ascii_lowercase() == b) || name.len() < 3
    {
        return false;
    }
    match chars.get(i + 4) {
        None => true,
        Some(c) => c.is_ascii_whitespace() || *c == '>' || *c == '/',
    }
}

/// The index just past the `>` that ends the tag started at `i`. A `>` inside a quoted
/// attribute value does not end a tag; a tag that never ends runs to the end of the input.
fn img_tag_end(chars: &[char], i: usize) -> usize {
    let mut quote = None;
    let mut j = i + 1;
    while j < chars.len() {
        let c = chars[j];
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
            }
            None => match c {
                '"' | '\'' => quote = Some(c),
                '>' => return j + 1,
                _ => {}
            },
        }
        j += 1;
    }
    chars.len()
}

/// Whether a tag points a picture at the network, through `src` or `srcset`.
fn img_is_remote(tag: &str) -> bool {
    if attr_value(tag, "src").is_some_and(|src| fetches_from_network(&src)) {
        return true;
    }
    // A srcset is a comma-separated list of "url descriptor" candidates; each is a fetch the
    // browser may make, so the tag goes if any candidate is remote.
    attr_value(tag, "srcset").is_some_and(|set| {
        set.split(',')
            .any(|candidate| candidate.split_whitespace().next().is_some_and(fetches_from_network))
    })
}

/// Whether a URL would leave this machine: an http(s) address, or a protocol-relative `//host/…`,
/// which is http(s) too once the page has a scheme. `cid:` and `data:` would not.
fn fetches_from_network(url: &str) -> bool {
    let url = url.trim().to_lowercase();
    url.starts_with("http://") || url.starts_with("https://") || url.starts_with("//")
}

/// The value of one attribute of a tag — quoted or bare — or `None` when it does not carry one.
/// Only an attribute beginning at a boundary counts, so a lazy-load `data-src` placeholder,
/// which by itself fetches nothing, does not decide the tag's fate.
fn attr_value(tag: &str, name: &str) -> Option<String> {
    let chars: Vec<char> = tag.chars().collect();
    let needle: Vec<char> = format!("{name}=").chars().collect();
    let mut i = 0;
    while i + needle.len() <= chars.len() {
        let at_boundary = i == 0 || chars[i - 1].is_ascii_whitespace();
        let matches = chars[i..i + needle.len()]
            .iter()
            .zip(&needle)
            .all(|(a, b)| a.to_ascii_lowercase() == *b);
        if at_boundary && matches {
            let mut j = i + needle.len();
            while j < chars.len() && chars[j].is_ascii_whitespace() {
                j += 1;
            }
            if j >= chars.len() {
                return None;
            }
            return match chars[j] {
                q @ ('"' | '\'') => {
                    let end =
                        (j + 1..chars.len()).find(|k| chars[*k] == q).unwrap_or(chars.len());
                    Some(chars[j + 1..end].iter().collect())
                }
                _ => {
                    let end = (j..chars.len())
                        .find(|k| chars[*k].is_ascii_whitespace() || chars[*k] == '>')
                        .unwrap_or(chars.len());
                    Some(chars[j..end].iter().collect())
                }
            };
        }
        i += 1;
    }
    None
}

/// Where "Open original" writes its files for the browser.
///
/// The cache, not the app's state directory: these files are copies of what the mailbox already
/// holds, written so that a browser has something to open, and losing them loses nothing. The
/// override is the draft file's pattern, so `tests/email-core` can point it at a temporary
/// directory instead of a real home.
pub fn original_html_dir() -> PathBuf {
    if let Ok(explicit) = std::env::var("YANTRIK_EMAIL_HTML_DIR") {
        return PathBuf::from(explicit);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".cache/yantrik/email")
}

/// The file one message's original is written to, inside `dir`.
///
/// The id came off a wire, and what comes off a wire never becomes a filename as it stands:
/// everything but letters, digits, `-` and `_` is dropped, so no path separator and no `..`
/// segment can survive, and an id that was all punctuation leaves the word "message".
pub fn original_html_file(dir: &Path, message_id: &str) -> PathBuf {
    let name: String = message_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    let name = if name.is_empty() { "message".to_string() } else { name };
    dir.join(format!("{name}.html"))
}

/// What the notice says after the browser was handed the file, including the part that has to
/// be said: what was taken out of the mail before it went anywhere near a network.
pub fn original_opened_note(removed: usize) -> String {
    match removed {
        0 => "Opened the original HTML in the browser. It had no remote images to remove."
            .to_string(),
        1 => "Opened the original HTML in the browser with 1 remote image removed, so the \
              sender is not told this was opened."
            .to_string(),
        n => format!(
            "Opened the original HTML in the browser with {n} remote images removed, so the \
             sender is not told this was opened."
        ),
    }
}
