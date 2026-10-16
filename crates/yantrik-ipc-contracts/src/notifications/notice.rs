//! A mind's notice: a short card a harness hands the OS to put in front of the person
//! (design/initiative-inbox-2026-10-16.md).
//!
//! ## Why this exists
//!
//! Stage 0 check 3 (16 October 2026) found that a notice a mind sent while no turn was running
//! reached nobody: the host logged `notice with no turn in flight; kept`, answered `{"kept": true}`
//! and stored nothing. A reminder the person asked for, or the result of a job they delegated, was
//! lost exactly when they were not looking.
//!
//! A notice is a [`Notification`](super::Notification) with this record beside it, in the one
//! store. Everything here is the shared half: what a mind may send ([`check`]), how every field is
//! cut ([`clamp_words`] and the `MAX_*` bounds below), what the store keeps ([`Notice`]), what the
//! person's controls are ([`Initiative`]), and the answers a mind can be given ([`answer`]). The
//! harness host checks a notice with [`check`] before it posts it; the notifications service checks
//! it again with the same function, because the store must not trust any caller to have done so.
//!
//! ## Text is data, buttons are navigation
//!
//! Every field is clamped and drawn as plain text. A button can only open something, as the
//! person, by the shell ([`ActionKind`]); an action of a kind this build does not know is dropped
//! and the card still lands, with `action` in [`Notice::clamped`].

use serde::{Deserialize, Serialize};

use super::clamp;

/// What `harness.attach` advertises when this desktop takes notices. A harness sends notices only
/// when it sees this; an older desktop sends no `features` at all, and that means no.
pub const FEATURE: &str = "notice_inbox_v1";

/// The longest `notice_id`, which is also the shortest useful bound on a key a mind invents.
pub const MAX_NOTICE_ID: usize = 96;
/// A notice's title: one line.
pub const MAX_TITLE: usize = 80;
/// A notice's body, in words, counted by whitespace. Both this and [`MAX_BODY_CHARS`] hold.
pub const MAX_BODY_WORDS: usize = 45;
/// A notice's body, in characters.
pub const MAX_BODY_CHARS: usize = 400;
/// "Why you got this": one line.
pub const MAX_WHY: usize = 140;
/// How many evidence refs one notice may carry.
pub const MAX_EVIDENCE: usize = 5;
/// The longest one evidence ref.
pub const MAX_EVIDENCE_CHARS: usize = 300;
/// The button's label.
pub const MAX_ACTION_LABEL: usize = 32;
/// What the button opens. Never cut: a cut path or URL opens something else, so a longer one is
/// dropped with the button.
pub const MAX_REF: usize = 300;

/// Notices one harness may have stored in any hour.
pub const PER_HOUR: usize = 20;
/// Notices one harness may have stored and not yet dismissed.
pub const MAX_UNDISMISSED: usize = 200;
/// Notices one account may have stored in any hour, across every harness id it attaches as. A
/// harness id is the harness's own word, so a process that attaches as `x1` … `xN` would otherwise
/// have N allowances.
pub const PER_HOUR_PER_UID: usize = 40;
/// Notices one account may have stored and not yet dismissed, across its harness ids.
pub const MAX_UNDISMISSED_PER_UID: usize = 400;
/// Notices every harness together may have stored in any hour.
pub const PER_HOUR_ALL: usize = 60;
/// Notices every harness together may have stored and not yet dismissed.
pub const MAX_UNDISMISSED_ALL: usize = 600;
/// Interrupts granted to one harness's obligations (a reminder, a delegated job) in any hour.
/// They skip the day's budget because the person asked for them; but the family is the mind's own
/// claim, so past this they wait in the inbox.
pub const OBLIGATION_INTERRUPTS_PER_HOUR: usize = 4;
/// The same, across every harness.
pub const OBLIGATION_INTERRUPTS_PER_HOUR_ALL: usize = 8;
/// Newlines a body keeps. A card has one sender line under its title, and a body of blank lines
/// could draw a second one of its own.
pub const MAX_BODY_NEWLINES: usize = 3;
/// Granted interrupts per local day, across every harness: the OS is one entity with many minds.
pub const DAILY_INTERRUPTS: u32 = 2;
/// Notices the live store keeps, counted apart from the plain notifications' 500, so a
/// `notify-send` loop cannot push out an obligation the OS acknowledged.
pub const MAX_NOTICES_STORED: usize = 1_000;
/// Feedback kept on one notice's own record. The queue a harness reads is separate.
pub const MAX_FEEDBACK_PER_NOTICE: usize = 16;
/// Feedback held for harnesses that have not acknowledged it, across all of them.
pub const MAX_FEEDBACK_QUEUED: usize = 2_000;
/// What a mind is told to wait when the store cannot be reached.
pub const RETRY_UNAVAILABLE_SECS: u64 = 30;

/// A closed set of words on the wire, read leniently from the store.
///
/// A record written by a newer build may carry a word this one does not know. Failing to parse it
/// would empty the whole store at the next start, so an unknown word is kept as [`Other`] and
/// written back as it was. What a mind SENDS is checked separately, by [`check`], which refuses an
/// unknown family rather than storing it.
macro_rules! words {
    ($(#[$m:meta])* $name:ident { $($(#[$vm:meta])* $var:ident = $word:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(from = "String", into = "String")]
        pub enum $name {
            $($(#[$vm])* $var,)+
            /// A word this build does not know, kept as it was written.
            Other(String),
        }

        impl $name {
            /// Every word this build knows, in the order the wire lists them.
            pub const ALL: &'static [$name] = &[$($name::$var),+];

            pub fn as_str(&self) -> &str {
                match self {
                    $($name::$var => $word,)+
                    $name::Other(word) => word,
                }
            }

            /// One of the words this build knows, or `None`.
            pub fn parse(word: &str) -> Option<$name> {
                match word {
                    $($word => Some($name::$var),)+
                    _ => None,
                }
            }
        }

        impl From<String> for $name {
            fn from(word: String) -> Self {
                Self::parse(&word).unwrap_or($name::Other(word))
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> String {
                value.as_str().to_string()
            }
        }
    };
}

words! {
    /// What a notice is about. Decides the budget and which of the person's controls apply.
    Family {
        /// Something the person asked to be reminded of.
        Reminder = "reminder",
        /// The result of a job the person handed a mind.
        Delegated = "delegated",
        Machine = "machine",
        Calendar = "calendar",
        Discovery = "discovery",
        StartOfDay = "start_of_day",
    }
}

impl Family {
    /// Things the person asked for. They never spend the daily budget, and the person's row for
    /// them offers On and Inbox only, never Off.
    pub fn is_obligation(&self) -> bool {
        matches!(self, Family::Reminder | Family::Delegated)
    }

    /// What Settings and the card's chip call it.
    pub fn label(&self) -> &str {
        match self {
            Family::Reminder => "Reminders",
            Family::Delegated => "Jobs you handed over",
            Family::Machine => "This machine",
            Family::Calendar => "Calendar",
            Family::Discovery => "Things a mind noticed",
            Family::StartOfDay => "Start of day",
            Family::Other(word) => word,
        }
    }
}

words! {
    /// Whether a card interrupts (a toast) or waits in the inbox.
    Attention {
        Interrupt = "interrupt",
        Inbox = "inbox",
    }
}

words! {
    /// Why the OS decided as it did, recorded on the card when it is stored.
    GrantReason {
        AskedInbox = "asked_inbox",
        BudgetSpent = "budget_spent",
        QuietInitiative = "quiet_initiative",
        FamilyInboxOnly = "family_inbox_only",
        QuietHours = "quiet_hours",
        /// An obligation over its hourly allowance of interrupts
        /// ([`OBLIGATION_INTERRUPTS_PER_HOUR`]): stored, and waiting in the inbox. A notice over
        /// the plain rate cap is not stored at all.
        Rate = "rate",
        Granted = "granted",
        /// The person set this family to Off: stored, answered as stored, and never shown.
        FamilyOff = "family_off",
    }
}

words! {
    /// What the person (or time) said about a card. Goes back to the harness that sent it.
    FeedbackKind {
        Useful = "useful",
        NotUseful = "not_useful",
        NotNow = "not_now",
        /// Sets the family to Off (Inbox only, for a family that cannot be off).
        NeverFamily = "never_family",
        /// Sets the family to Inbox only.
        InboxOnly = "inbox_only",
        Wrong = "wrong",
        /// The card was dismissed without a feedback button.
        Dismissed = "dismissed",
        /// The card passed `useful_until` unanswered.
        Expired = "expired",
    }
}

impl FeedbackKind {
    /// The kinds a person can press on a card, in the order the card draws them.
    pub const BUTTONS: &'static [FeedbackKind] = &[
        FeedbackKind::Useful,
        FeedbackKind::NotUseful,
        FeedbackKind::NotNow,
        FeedbackKind::NeverFamily,
        FeedbackKind::InboxOnly,
        FeedbackKind::Wrong,
    ];

    /// Whether pressing it also takes the card out of the inbox. "Useful" and "Inbox only" say
    /// something about the card and leave it where it is; the rest say it should go.
    pub fn takes_the_card_away(&self) -> bool {
        matches!(self, FeedbackKind::NotUseful | FeedbackKind::NotNow | FeedbackKind::NeverFamily | FeedbackKind::Wrong)
    }
}

words! {
    /// What a button may do. Each only opens something, by the shell, as the person.
    ActionKind {
        /// An absolute path, opened read-only in its default app. Only a regular file under the
        /// person's home, after symlinks.
        OpenFile = "open_file",
        /// A calendar event id, shown in Calendar.
        OpenEvent = "open_event",
        /// A Notes id.
        OpenNote = "open_note",
        /// An `http` or `https` URL, in the browser. The button shows the host.
        OpenUrl = "open_url",
    }
}

/// The one button a notice may carry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoticeAction {
    pub label: String,
    pub kind: ActionKind,
    #[serde(rename = "ref")]
    pub reference: String,
}

/// Who sent a notice, as the harness host established it: the attach's id, the account the kernel
/// named at attach, and the process line the approval card would print. Never what the request
/// said. Only a caller the store judges to be the desktop may set it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct NoticeHarness {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uid: Option<u32>,
    #[serde(default)]
    pub verified: String,
    /// The name the harness gave itself at attach, shown beside `verified` and never instead of it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// The program behind the attach, by the name of its executable (`yantrik-mind`, `python3`),
    /// as the kernel reports it. With `uid` it is what a harness id is bound to: the store refuses
    /// notices and feedback under an id from any other program or account than the one that
    /// first used it. Empty when the host could not see the process.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub program: String,
}

/// What a harness id belongs to: the account the kernel named at attach and the program behind
/// it, as the first notice stored under the id found them. The store refuses notices and feedback
/// under the id from anything else.
///
/// `program` is empty when the shell could not see the executable, which is the case for a mind
/// that runs under its own account (`yantrik-mind`); the id is then bound by the account alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarnessBinding {
    pub uid: Option<u32>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub program: String,
}

impl HarnessBinding {
    /// Whether `uid` running `program` is what this id belongs to.
    pub fn admits(&self, uid: Option<u32>, program: &str) -> bool {
        self.uid == uid && (self.program.is_empty() || self.program == program)
    }

    /// One line for Settings: "account 999" or "account 1000 · python3".
    pub fn line(&self) -> String {
        let account = self.uid.map_or_else(|| "an unknown account".to_string(), |u| format!("account {u}"));
        if self.program.is_empty() { account } else { format!("{account} · {}", self.program) }
    }
}

/// One response to a card, as the store keeps it and as the sending harness reads it on its poll.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Feedback {
    pub seq: u64,
    pub notice_id: String,
    pub kind: FeedbackKind,
    pub seen_at: Option<String>,
    pub opened_at: Option<String>,
    /// RFC 3339, UTC.
    pub at: String,
}

/// The notice half of a stored notification. Title and body are the notification's own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notice {
    pub notice_id: String,
    pub family: Family,
    pub why: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<String>,
    /// What the mind asked for.
    pub attention: Attention,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub useful_until: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<NoticeAction>,
    pub harness: NoticeHarness,
    /// What the OS decided, once, when it stored the card.
    pub granted: Attention,
    pub grant_reason: GrantReason,
    /// Quiet hours held an interrupt: the card is toasted at this time if it is still useful and
    /// not yet seen. RFC 3339.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub held_until: Option<String>,
    /// `toast` or `inbox`, once the shell has shown it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shown_as: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shown_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opened_at: Option<String>,
    /// The fields that were cut on the way in: `body`, `evidence[2]`, `action`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clamped: Vec<String>,
    /// The person's responses, newest last, at most [`MAX_FEEDBACK_PER_NOTICE`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub feedback: Vec<Feedback>,
}

impl Notice {
    /// Whether the card is drawn at all. A family the person switched off is stored and never shown.
    pub fn hidden(&self) -> bool {
        self.grant_reason == GrantReason::FamilyOff
    }

    /// Whether the person has already pressed one of the card's feedback buttons.
    pub fn answered(&self) -> bool {
        self.feedback.iter().any(|f| FeedbackKind::BUTTONS.contains(&f.kind))
    }
}

/// What the harness host posts to `notifications.add` beside a notification, on a harness's behalf.
///
/// `notice` is the mind's request as it sent it; the store reads it with [`check`]. `harness` is
/// the host's own account of who sent it. Both are honoured only from the desktop.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NoticePost {
    pub notice: serde_json::Value,
    pub harness: NoticeHarness,
    /// The store makes the id, `os-<revision>`: a plain `{text}` notice from an older harness
    /// has none of its own.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub os_id: bool,
}

/// A notice as it passed [`check`]: every field bounded, and what was cut, named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checked {
    pub notice_id: String,
    pub family: Family,
    pub title: String,
    pub body: String,
    pub why: String,
    pub evidence: Vec<String>,
    pub attention: Attention,
    pub useful_until: Option<String>,
    pub supersedes: Option<String>,
    pub action: Option<NoticeAction>,
    pub clamped: Vec<String>,
}

/// Whether a `notice_id` is one a mind may use: 1–96 of `[A-Za-z0-9._:-]`.
pub fn valid_notice_id(id: &str) -> bool {
    (1..=MAX_NOTICE_ID).contains(&id.len())
        && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-'))
}

/// Cut `s` after `max` words, counted by whitespace, keeping the text as it was written up to the
/// cut. Answers the text and whether anything was cut.
///
/// The cut lands on a character boundary because it is taken where a word starts, so it can
/// never split a UTF-8 sequence; the character bound is applied after it with [`clamp`].
pub fn clamp_words(s: &str, max: usize) -> (String, bool) {
    let mut words = 0;
    let mut in_word = false;
    for (at, c) in s.char_indices() {
        if c.is_whitespace() {
            in_word = false;
        } else if !in_word {
            in_word = true;
            words += 1;
            if words > max {
                return (s[..at].trim_end().to_string(), true);
            }
        }
    }
    (s.to_string(), false)
}

/// Characters that change how the text around them is drawn rather than being text: the bidi
/// embeddings, overrides, isolates and marks. `a\u{202E}gpj.exe` draws as `aexe.jpg`.
fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// A mind's text, made plain before it is stored: every bidi control and every control character
/// becomes a space, and so do the line and paragraph separators, except `\n`, which survives
/// only where `newlines` allows it — the first `newlines` of them; later ones become spaces too.
///
/// The shell draws these words as plain text, but plain text still obeys U+202E and still breaks
/// at CR: a body could reverse its own tail, or put a fake "Sent by … · verified" line under the
/// real one. And the store keeps the record and `describe` hands it out, so it is cleaned here,
/// once, rather than wherever it is drawn.
pub fn plain(s: &str, newlines: usize) -> String {
    let mut kept = 0;
    s.chars()
        .map(|c| {
            if c == '\n' && kept < newlines {
                kept += 1;
                '\n'
            } else if c.is_control() || is_bidi_control(c) || matches!(c, '\u{2028}' | '\u{2029}') {
                ' '
            } else {
                c
            }
        })
        .collect()
}

/// One line: the first, cut to `max` characters. Answers the line and whether anything was lost.
pub fn one_line(s: &str, max: usize) -> (String, bool) {
    let s = plain(s, usize::MAX);
    let trimmed = s.trim();
    let first = trimmed.lines().next().unwrap_or_default().trim();
    let line = clamp(first, max);
    let cut = line.len() != trimmed.len();
    (line, cut)
}

/// A body: 45 words and 400 characters, whichever is shorter.
pub fn clamp_body(s: &str) -> (String, bool) {
    let cleaned = plain(s, MAX_BODY_NEWLINES);
    let trimmed = cleaned.trim();
    let (words, cut_words) = clamp_words(trimmed, MAX_BODY_WORDS);
    let body = clamp(&words, MAX_BODY_CHARS);
    let cut = cut_words || body.len() != words.len();
    (body, cut)
}

/// Read a notice a mind sent, as the spec's table bounds it.
///
/// `Err` is a refusal the mind should not retry: malformed, an unknown family, a missing `title`
/// or `why`. Everything else lands, cut where it had to be, with what was cut named in `clamped`.
/// An action that is not one of [`ActionKind`], or whose `ref` fails its shape, is dropped and
/// named `action`; whether an `open_file` path really is a file under the person's home is for
/// whoever can look ([`file_under`]).
pub fn check(raw: &serde_json::Value) -> Result<Checked, String> {
    let Some(obj) = raw.as_object() else {
        return Err("a notice is an object".into());
    };
    let text = |key: &str| -> Result<Option<String>, String> {
        match obj.get(key) {
            None | Some(serde_json::Value::Null) => Ok(None),
            Some(serde_json::Value::String(s)) => Ok(Some(s.clone())),
            Some(_) => Err(format!("`{key}` must be a string")),
        }
    };

    let notice_id = text("notice_id")?.unwrap_or_default();
    if !valid_notice_id(&notice_id) {
        return Err("`notice_id` must be 1 to 96 characters of A-Z, a-z, 0-9, `.`, `_`, `:` or `-`".into());
    }
    let family_word = text("family")?.unwrap_or_default();
    let Some(family) = Family::parse(&family_word) else {
        let known: Vec<&str> = Family::ALL.iter().map(Family::as_str).collect();
        return Err(format!(
            "`family` must be one of {}; the OS cannot apply the person's settings to `{family_word}`",
            known.join(", ")
        ));
    };
    let title_raw = text("title")?.unwrap_or_default();
    if title_raw.trim().is_empty() {
        return Err("a notice needs a `title`".into());
    }
    let why_raw = text("why")?.unwrap_or_default();
    if why_raw.trim().is_empty() {
        return Err("a notice needs a `why`, shown as \"Why you got this\"".into());
    }
    let attention = match text("attention")?.as_deref() {
        None => Attention::Inbox,
        Some(word) => Attention::parse(word).ok_or("`attention` must be `interrupt` or `inbox`")?,
    };
    let useful_until = text("useful_until")?;
    if let Some(when) = &useful_until {
        if chrono::DateTime::parse_from_rfc3339(when).is_err() {
            return Err("`useful_until` must be an RFC 3339 time".into());
        }
    }
    let supersedes = text("supersedes")?.filter(|s| !s.is_empty());
    if supersedes.as_deref().is_some_and(|s| !valid_notice_id(s)) {
        return Err("`supersedes` must be a `notice_id`".into());
    }

    let mut clamped = Vec::new();
    let (title, cut) = one_line(&title_raw, MAX_TITLE);
    if cut {
        clamped.push("title".to_string());
    }
    let (body, cut) = clamp_body(&text("body")?.unwrap_or_default());
    if cut {
        clamped.push("body".to_string());
    }
    let (why, cut) = one_line(&why_raw, MAX_WHY);
    if cut {
        clamped.push("why".to_string());
    }

    let evidence = match obj.get("evidence") {
        None | Some(serde_json::Value::Null) => Vec::new(),
        Some(serde_json::Value::Array(items)) => {
            if items.len() > MAX_EVIDENCE {
                clamped.push("evidence".to_string());
            }
            let mut kept = Vec::new();
            for (i, item) in items.iter().take(MAX_EVIDENCE).enumerate() {
                let Some(s) = item.as_str() else {
                    return Err(format!("`evidence[{i}]` must be a string"));
                };
                let cleaned = plain(s, 0);
                let cut = clamp(&cleaned, MAX_EVIDENCE_CHARS);
                if cut.len() != cleaned.len() {
                    clamped.push(format!("evidence[{i}]"));
                }
                kept.push(cut);
            }
            kept
        }
        Some(_) => return Err("`evidence` must be a list of strings".into()),
    };

    let action = match obj.get("action") {
        None | Some(serde_json::Value::Null) => None,
        Some(raw) => match read_action(raw) {
            Some((action, label_cut)) => {
                if label_cut {
                    clamped.push("action.label".to_string());
                }
                Some(action)
            }
            None => {
                clamped.push("action".to_string());
                None
            }
        },
    };

    Ok(Checked { notice_id, family, title, body, why, evidence, attention, useful_until, supersedes, action, clamped })
}

/// An action's shape, or `None` to drop it. The label is cut; the `ref` never is.
fn read_action(raw: &serde_json::Value) -> Option<(NoticeAction, bool)> {
    let kind = ActionKind::parse(raw.get("kind")?.as_str()?)?;
    let reference = raw.get("ref")?.as_str()?.trim().to_string();
    if reference.is_empty() || reference.chars().count() > MAX_REF || reference.chars().any(char::is_control) {
        return None;
    }
    let ok = match kind {
        ActionKind::OpenFile => reference.starts_with('/'),
        ActionKind::OpenUrl => url_host(&reference).is_some(),
        ActionKind::OpenEvent | ActionKind::OpenNote => {
            reference.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-' | '@'))
        }
        ActionKind::Other(_) => false,
    };
    if !ok {
        return None;
    }
    let label_raw = raw.get("label").and_then(|l| l.as_str()).unwrap_or_default();
    let (label, cut) = one_line(label_raw, MAX_ACTION_LABEL);
    let label = if label.is_empty() { default_label(&kind, &reference) } else { label };
    Some((NoticeAction { label, kind, reference }, cut))
}

/// What a button says when the mind gave it no words.
fn default_label(kind: &ActionKind, reference: &str) -> String {
    match kind {
        ActionKind::OpenFile => "Open file".into(),
        ActionKind::OpenEvent => "Open event".into(),
        ActionKind::OpenNote => "Open note".into(),
        ActionKind::OpenUrl => clamp(&format!("Open {}", url_host(reference).unwrap_or_default()), MAX_ACTION_LABEL),
        ActionKind::Other(_) => "Open".into(),
    }
}

/// The host of an `http` or `https` URL, lowercased, or `None` for anything else. The button
/// shows it ("Open example.com"), so a label cannot dress one site up as another.
pub fn url_host(url: &str) -> Option<String> {
    let lower = url.to_ascii_lowercase();
    let rest = lower.strip_prefix("https://").or_else(|| lower.strip_prefix("http://"))?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    // Credentials in a URL are how `https://bank.example@evil.example` reads as the bank.
    if authority.contains('@') {
        return None;
    }
    let host = authority.rsplit_once(':').map_or(authority, |(h, port)| {
        if port.chars().all(|c| c.is_ascii_digit()) { h } else { authority }
    });
    let ok = !host.is_empty()
        && host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '[' | ']' | ':'));
    ok.then(|| host.to_string())
}

/// The path an `open_file` ref resolves to, after symlinks, when it is a regular file under
/// `home`. `None` drops the button.
///
/// Read when the notice is stored and again when the button is pressed: a file can be replaced by
/// a symlink to `/etc/shadow` in between, and only the second read is what opens.
///
/// Nothing hidden: a path with any component that starts with `.`, where it was written or where
/// it resolves, is refused. That is where a home keeps what a card must never put on the screen
/// a mind can read — `~/.ssh`, `~/.gnupg`, `~/.config/*/credentials`, the keyrings, browser
/// profiles, and this desktop's own `~/.local/share/yantrik`.
///
/// And through the rule every agent read shares (`home_paths::may_read_file`, with its
/// `PROTECTED` list), as the review asked: outside the protected places, and a file with one name,
/// because a hard link `~/Documents/key.txt` to `~/.ssh/id_ed25519` has no hidden component and
/// no link to follow, and is the key.
pub fn file_under(reference: &str, home: &std::path::Path) -> Option<std::path::PathBuf> {
    crate::home_paths::may_read_file(reference, home).ok()?;
    let home = std::fs::canonicalize(home).ok()?;
    let hidden = |p: &std::path::Path| p.components().any(|c| c.as_os_str().to_string_lossy().starts_with('.'));
    let written = std::path::Path::new(reference);
    if written.strip_prefix(&home).map_or_else(|_| hidden(written), hidden) {
        return None;
    }
    let real = std::fs::canonicalize(reference).ok()?;
    let inside = real.strip_prefix(&home).ok()?;
    if inside.as_os_str().is_empty() || hidden(inside) {
        return None;
    }
    if crate::home_paths::is_protected(&real) {
        return None;
    }
    std::fs::metadata(&real).ok()?.is_file().then_some(real)
}

// ── The person's controls ──────────────────────────────────────────────────────────────────

/// One family's setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FamilySetting {
    On,
    InboxOnly,
    Off,
}

impl FamilySetting {
    pub fn as_str(self) -> &'static str {
        match self {
            FamilySetting::On => "on",
            FamilySetting::InboxOnly => "inbox_only",
            FamilySetting::Off => "off",
        }
    }

    pub fn parse(word: &str) -> Option<FamilySetting> {
        match word {
            "on" => Some(FamilySetting::On),
            "inbox_only" => Some(FamilySetting::InboxOnly),
            "off" => Some(FamilySetting::Off),
            _ => None,
        }
    }
}

/// Quiet hours, in the machine's local time: from `start` to `end`, each an hour of the day.
/// `22` to `7` crosses midnight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuietHours {
    pub start: u8,
    pub end: u8,
}

impl QuietHours {
    /// Whether `hour` (0–23) falls inside.
    pub fn contains(&self, hour: u32) -> bool {
        let (start, end) = (u32::from(self.start % 24), u32::from(self.end % 24));
        if start == end {
            return false;
        }
        if start < end {
            (start..end).contains(&hour)
        } else {
            hour >= start || hour < end
        }
    }

    /// When the quiet hours that contain `now` end, or `None` when `now` is not inside them.
    pub fn ends_after(&self, now: chrono::DateTime<chrono::Local>) -> Option<chrono::DateTime<chrono::Local>> {
        use chrono::{Duration, TimeZone, Timelike};
        if !self.contains(now.hour()) {
            return None;
        }
        let end = u32::from(self.end % 24);
        let today = now.date_naive().and_hms_opt(end, 0, 0)?;
        let at = if now.hour() < end { today } else { today + Duration::days(1) };
        chrono::Local.from_local_datetime(&at).earliest()
    }

    /// "22:00–07:00".
    pub fn label(&self) -> String {
        format!("{:02}:00–{:02}:00", self.start % 24, self.end % 24)
    }
}

/// The person's initiative settings: Settings → Minds → Initiative.
///
/// Kept in the notifications store, so the service that makes the grant decision reads the same
/// settings the person set, and only the desktop can change them.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Initiative {
    /// Quiet initiative: every initiative family goes to the inbox. Obligations still arrive.
    pub quiet: bool,
    pub off: Vec<Family>,
    pub inbox_only: Vec<Family>,
    /// `None` is no quiet hours.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quiet_hours: Option<QuietHours>,
}

impl Initiative {
    /// What a harness is told on attach and on every poll: `{quiet, off, inbox_only}`, the
    /// spec's shape exactly.
    pub fn wire(&self) -> serde_json::Value {
        serde_json::json!({
            "quiet": self.quiet,
            "off": self.off.iter().map(Family::as_str).collect::<Vec<_>>(),
            "inbox_only": self.inbox_only.iter().map(Family::as_str).collect::<Vec<_>>(),
        })
    }

    pub fn setting(&self, family: &Family) -> FamilySetting {
        if self.off.contains(family) {
            FamilySetting::Off
        } else if self.inbox_only.contains(family) {
            FamilySetting::InboxOnly
        } else {
            FamilySetting::On
        }
    }

    /// Set one family. Off is refused for a family the person asked for.
    pub fn set(&mut self, family: &Family, setting: FamilySetting) -> Result<(), String> {
        if setting == FamilySetting::Off && family.is_obligation() {
            return Err(format!("{} are things you asked for; they can be Inbox only, never Off", family.label()));
        }
        self.off.retain(|f| f != family);
        self.inbox_only.retain(|f| f != family);
        match setting {
            FamilySetting::On => {}
            FamilySetting::InboxOnly => self.inbox_only.push(family.clone()),
            FamilySetting::Off => self.off.push(family.clone()),
        }
        Ok(())
    }
}

// ── Answers ────────────────────────────────────────────────────────────────────────────────

/// The answers to `harness.notice`. Every one is a result, never a transport error, so a mind's
/// outbox can tell "retry" from "stop".
pub mod answer {
    /// On disk, after fsync. `duplicate` when it was already held from this harness.
    pub fn stored(notice_id: &str, duplicate: bool, clamped: &[String]) -> serde_json::Value {
        let mut answer = serde_json::json!({ "stored": true, "notice_id": notice_id });
        if duplicate {
            answer["duplicate"] = true.into();
        }
        if !clamped.is_empty() {
            answer["clamped"] = clamped.into();
        }
        answer
    }

    /// Try again later.
    pub fn retry(secs: u64, why: &str) -> serde_json::Value {
        serde_json::json!({ "stored": false, "retry_after_secs": secs, "why": why })
    }

    /// Do not retry.
    pub fn refused(why: &str) -> serde_json::Value {
        serde_json::json!({ "stored": false, "refused": why })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn reminder() -> serde_json::Value {
        json!({
            "notice_id": "rem-2026-10-16-0001",
            "family": "reminder",
            "title": "Stretch",
            "body": "You asked me at 14:02 to remind you to stretch.",
            "why": "You asked for this reminder.",
            "evidence": ["task:41"],
            "attention": "interrupt",
            "useful_until": "2026-10-16T19:30:00Z",
            "supersedes": null,
            "action": { "label": "Open note", "kind": "open_note", "ref": "n-7f3a" }
        })
    }

    #[test]
    fn the_spec_example_reads_whole_and_cuts_nothing() {
        let c = check(&reminder()).expect("the spec's own example is a notice");
        assert_eq!(c.family, Family::Reminder);
        assert_eq!(c.attention, Attention::Interrupt);
        assert_eq!(c.action.as_ref().map(|a| a.kind.clone()), Some(ActionKind::OpenNote));
        assert!(c.clamped.is_empty(), "{:?}", c.clamped);
    }

    #[test]
    fn a_46_word_body_is_cut_to_45_and_says_so() {
        let mut n = reminder();
        let words: Vec<String> = (1..=46).map(|i| format!("w{i}")).collect();
        n["body"] = words.join(" ").into();
        let c = check(&n).unwrap();
        assert_eq!(c.body.split_whitespace().count(), 45);
        assert!(c.body.ends_with("w45"), "{}", c.body);
        assert_eq!(c.clamped, vec!["body"]);
        // Exactly 45 is not a cut.
        n["body"] = words[..45].join(" ").into();
        assert!(check(&n).unwrap().clamped.is_empty());
    }

    #[test]
    fn a_body_is_also_held_to_400_characters_without_splitting_a_character() {
        let mut n = reminder();
        // Ten words, each 60 characters of Devanagari: well under 45 words, well over 400 chars.
        n["body"] = vec!["न".repeat(60); 10].join(" ").into();
        let c = check(&n).unwrap();
        assert_eq!(c.body.chars().count(), MAX_BODY_CHARS);
        assert_eq!(c.clamped, vec!["body"]);
        // Whitespace that is not a space still separates words, and the text up to the cut is
        // kept as written.
        let (cut, was) = clamp_words("one\ttwo\nthree four", 3);
        assert_eq!((cut.as_str(), was), ("one\ttwo\nthree", true));
    }

    #[test]
    fn missing_title_or_why_and_an_unknown_family_are_refused() {
        for (key, value) in [("title", json!("  ")), ("why", json!(null)), ("family", json!("gossip")), ("notice_id", json!("has space"))] {
            let mut n = reminder();
            n[key] = value;
            assert!(check(&n).is_err(), "{key}");
        }
        let mut n = reminder();
        n["attention"] = "shout".into();
        assert!(check(&n).is_err());
        n["attention"] = json!(null);
        assert_eq!(check(&n).unwrap().attention, Attention::Inbox, "inbox is the default");
        assert!(valid_notice_id(&"a".repeat(96)) && !valid_notice_id(&"a".repeat(97)));
    }

    #[test]
    fn an_unknown_action_kind_is_dropped_and_the_card_still_lands() {
        let mut n = reminder();
        n["action"] = json!({ "label": "Delete all", "kind": "run_command", "ref": "rm -rf ~" });
        let c = check(&n).expect("the card lands");
        assert_eq!(c.action, None);
        assert_eq!(c.clamped, vec!["action"]);
        // A URL that is not http(s), or that hides its host behind credentials, goes the same way.
        for bad in ["javascript:alert(1)", "file:///etc/passwd", "https://bank.example@evil.example/"] {
            n["action"] = json!({ "kind": "open_url", "ref": bad });
            assert_eq!(check(&n).unwrap().action, None, "{bad}");
        }
        n["action"] = json!({ "kind": "open_url", "ref": "https://Example.com:8443/a?b" });
        assert_eq!(check(&n).unwrap().action.unwrap().label, "Open example.com");
    }

    #[test]
    fn evidence_past_five_or_past_300_characters_is_named() {
        let mut n = reminder();
        n["evidence"] = json!(["a", "b", "x".repeat(301), "d", "e", "f"]);
        let c = check(&n).unwrap();
        assert_eq!(c.evidence.len(), 5);
        assert_eq!(c.clamped, vec!["evidence", "evidence[2]"]);
    }

    #[test]
    fn an_old_store_word_survives_a_build_that_does_not_know_it() {
        let f: Family = serde_json::from_str("\"weather\"").unwrap();
        assert_eq!(f, Family::Other("weather".into()));
        assert_eq!(serde_json::to_string(&f).unwrap(), "\"weather\"");
        // But a mind cannot send one.
        let mut n = reminder();
        n["family"] = "weather".into();
        assert!(check(&n).is_err());
    }

    #[test]
    fn obligations_cannot_be_switched_off() {
        let mut settings = Initiative::default();
        assert!(settings.set(&Family::Reminder, FamilySetting::Off).is_err());
        settings.set(&Family::Reminder, FamilySetting::InboxOnly).unwrap();
        settings.set(&Family::Discovery, FamilySetting::Off).unwrap();
        assert_eq!(settings.setting(&Family::Discovery), FamilySetting::Off);
        assert_eq!(
            settings.wire(),
            json!({ "quiet": false, "off": ["discovery"], "inbox_only": ["reminder"] })
        );
    }

    #[test]
    fn quiet_hours_cross_midnight() {
        let q = QuietHours { start: 22, end: 7 };
        assert!(q.contains(23) && q.contains(0) && q.contains(6));
        assert!(!q.contains(7) && !q.contains(21));
        let day = QuietHours { start: 13, end: 14 };
        assert!(day.contains(13) && !day.contains(14));
        assert!(!QuietHours { start: 5, end: 5 }.contains(5), "an empty window is no window");
    }

    /// Security review of #831, finding 3, through the shared rule: a protected place and a second
    /// name for a credential are refused, though neither has a hidden component.
    #[cfg(unix)]
    #[test]
    fn a_file_ref_goes_through_the_shared_protected_paths_rule() {
        let root = std::env::temp_dir().join(format!("notice-protected-{}", std::process::id()));
        let home = root.join("home");
        std::fs::create_dir_all(home.join(".ssh")).unwrap();
        std::fs::create_dir_all(home.join("Documents")).unwrap();
        std::fs::write(home.join(".ssh/id_ed25519"), "key").unwrap();
        std::fs::write(home.join("memory.db"), "x").unwrap();
        std::fs::write(home.join("Documents/report.pdf"), "x").unwrap();
        std::fs::hard_link(home.join(".ssh/id_ed25519"), home.join("Documents/key.txt")).unwrap();
        let at = |rel: &str| home.join(rel).to_string_lossy().to_string();
        assert!(file_under(&at("Documents/key.txt"), &home).is_none(), "a second name for a key");
        assert!(file_under(&at("memory.db"), &home).is_none(), "on the PROTECTED list, not hidden");
        assert!(file_under(&at("Documents/report.pdf"), &home).is_some());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_file_ref_must_be_a_regular_file_under_home_after_symlinks() {
        let root = std::env::temp_dir().join(format!("notice-file-{}", std::process::id()));
        let home = root.join("home");
        std::fs::create_dir_all(home.join("docs")).unwrap();
        std::fs::write(home.join("docs/plan.md"), "x").unwrap();
        std::fs::write(root.join("outside.txt"), "x").unwrap();
        assert!(file_under(home.join("docs/plan.md").to_str().unwrap(), &home).is_some());
        assert!(file_under(home.join("docs").to_str().unwrap(), &home).is_none(), "a directory");
        assert!(file_under(root.join("outside.txt").to_str().unwrap(), &home).is_none());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.join("outside.txt"), home.join("link.txt")).unwrap();
            assert!(file_under(home.join("link.txt").to_str().unwrap(), &home).is_none(), "a symlink out");
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn nothing_hidden_under_home_can_be_a_cards_file() {
        let root = std::env::temp_dir().join(format!("notice-hidden-{}", std::process::id()));
        let home = root.join("home");
        std::fs::create_dir_all(home.join(".ssh")).unwrap();
        std::fs::create_dir_all(home.join("docs")).unwrap();
        std::fs::write(home.join(".ssh/id_ed25519"), "secret").unwrap();
        std::fs::write(home.join(".bashrc"), "x").unwrap();
        std::fs::write(home.join("docs/plan.md"), "x").unwrap();
        assert!(file_under(home.join(".ssh/id_ed25519").to_str().unwrap(), &home).is_none());
        assert!(file_under(home.join(".bashrc").to_str().unwrap(), &home).is_none());
        assert!(file_under(home.join("docs/../.bashrc").to_str().unwrap(), &home).is_none());
        #[cfg(unix)]
        {
            // A plain name that resolves into a hidden directory is judged where it resolves.
            std::os::unix::fs::symlink(home.join(".ssh/id_ed25519"), home.join("docs/key.txt")).unwrap();
            assert!(file_under(home.join("docs/key.txt").to_str().unwrap(), &home).is_none());
        }
        assert!(file_under(home.join("docs/plan.md").to_str().unwrap(), &home).is_some());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn bidi_and_control_characters_never_reach_the_store() {
        let mut n = reminder();
        n["title"] = json!("a\u{202E}gpj.exe\r\u{2028}x");
        n["why"] = json!("why\u{2066}not\u{0}");
        n["body"] = json!("x\r\n\u{2028}y\n\n\n\n\nSent by Yantrik \u{00B7} verified\u{000B}");
        n["evidence"] = json!(["line\none\u{202D}"]);
        let checked = check(&n).unwrap();
        for text in [&checked.title, &checked.why, &checked.body, &checked.evidence[0]] {
            assert!(!text.chars().any(|c| (c.is_control() && c != '\n') || is_bidi_control(c) || c == '\u{2028}'), "{text:?}");
        }
        assert_eq!(checked.title, "a gpj.exe  x");
        assert!(!checked.evidence[0].contains('\n'));
        assert!(checked.body.matches('\n').count() <= MAX_BODY_NEWLINES, "{:?}", checked.body);
        assert!(checked.body.ends_with("verified"), "{:?}", checked.body);
    }
}
