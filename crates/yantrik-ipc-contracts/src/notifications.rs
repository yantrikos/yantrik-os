//! Notifications contract — the one store every notification on this machine lands in.
//!
//! ## What this used to be
//!
//! A `Notification` struct with `icon`, `source_app` and a `timestamp` string, plus a
//! `NotificationService` trait that nothing on this machine implemented and nothing called. The
//! struct was used by exactly one file — the service's own `main.rs` — and the trait by none, so
//! the "contract" named one side of a conversation that had no other side.
//!
//! It is a contract now because there are several senders: the shell, our own apps, the calendar
//! service's reminder thread, `yos notify`, and every ordinary Linux program through
//! `org.freedesktop.Notifications`. They all build the same request and the store parses it once.
//!
//! ## Why every field is bounded
//!
//! Anything on the machine can post here, including programs we did not write and did not test.
//! A 10 MB body from a misbehaving web page would be written to disk, read back at every start
//! and drawn into a toast. So the store truncates on the way in — see [`clamp`] and the `MAX_*`
//! constants — and the bound is part of the contract rather than a defensive check the service
//! might forget, because the freedesktop path cannot refuse: the spec has no error reply for
//! "too long", and refusing would make our desktop the one where `notify-send` mysteriously
//! fails.

use serde::{Deserialize, Serialize};

// ── Methods ─────────────────────────────────────────────────────────────────────────────────

/// Post a notification. Params: [`AddRequest`]. Answers the stored [`Notification`].
pub const ADD: &str = "notifications.add";
/// Everything the store holds that is not dismissed, newest first. Answers `Vec<Notification>`.
pub const LIST: &str = "notifications.list";
/// What changed since a revision. Params: `{ revision: u64 }`. Answers [`Since`].
pub const SINCE: &str = "notifications.since";
/// Dismiss one. Params: `{ id }`.
pub const DISMISS: &str = "notifications.dismiss";
/// Dismiss everything currently showing. Params: `{}`.
pub const DISMISS_ALL: &str = "notifications.dismiss_all";
/// Mark one — or, with no `id`, everything — as read. Params: `{ id? }`.
pub const MARK_READ: &str = "notifications.mark_read";
/// Invoke an action a notification published. Params: `{ id, action_id }`.
pub const ACTION: &str = "notifications.action";

// ── Bounds ──────────────────────────────────────────────────────────────────────────────────

/// Longest title kept. A title is one line in a toast; past this it is elided on screen anyway.
pub const MAX_TITLE: usize = 200;
/// Longest body kept. Two or three paragraphs — enough for a real message, not a log dump.
pub const MAX_BODY: usize = 2_000;
/// Longest sender name kept.
pub const MAX_APP: usize = 64;
/// Most actions kept on one notification. The toast draws them in a row; four is the row.
pub const MAX_ACTIONS: usize = 4;
/// Longest action id kept.
pub const MAX_ACTION_ID: usize = 64;
/// Longest action label kept. A button, not a sentence.
pub const MAX_ACTION_LABEL: usize = 48;
/// How many notifications the store keeps. Oldest are dropped first.
pub const MAX_STORED: usize = 500;
/// How long a dismissed notification is kept before it is pruned. A week, so "what did that
/// installer say on Tuesday" is still answerable and the file does not grow forever.
pub const DISMISSED_TTL_SECS: i64 = 7 * 24 * 60 * 60;

/// Cut a string to `max` characters — characters, not bytes, so this cannot split a UTF-8
/// sequence and write a file that will not parse. Appends nothing: an elision marker in stored
/// data is a lie about what the sender said, and the UI elides for display on its own.
pub fn clamp(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect()
}

// ── Types ───────────────────────────────────────────────────────────────────────────────────

/// How loud a notification is. The word is lowercase on the wire because that is what
/// `notify-send --urgency=` uses, what `yos notify` takes, and what the shell's describe prints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Urgency {
    Low,
    #[default]
    Normal,
    Critical,
}

impl Urgency {
    /// The short word, for a view or a log line.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Normal => "normal",
            Self::Critical => "critical",
        }
    }

    /// Parse the word, anything unrecognised being `normal`.
    ///
    /// Deliberately total. This reads user input from `yos notify --urgency`, from an app's
    /// one-liner and from a JSON-RPC caller; the alternative to a default is refusing to deliver
    /// a notification because somebody typed "high", which loses the message to protect a field.
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "low" | "0" => Self::Low,
            "critical" | "urgent" | "2" => Self::Critical,
            _ => Self::Normal,
        }
    }

    /// The freedesktop `urgency` hint byte (spec 1.2 §1.3): 0 low, 1 normal, 2 critical.
    pub fn from_hint_byte(b: u8) -> Self {
        match b {
            0 => Self::Low,
            2 => Self::Critical,
            _ => Self::Normal,
        }
    }

    /// The same byte going out, for `GetCapabilities` consumers and for the shell's toast code,
    /// which has drawn urgency as 0/1/2 since before any of this existed.
    pub fn hint_byte(self) -> u8 {
        match self {
            Self::Low => 0,
            Self::Normal => 1,
            Self::Critical => 2,
        }
    }
}

/// Where a notification came in from. Not the same thing as `app`: `app` is who is speaking,
/// this is which door they came through, and it is the difference between "Chromium said this
/// over D-Bus" and "our download manager called the service".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// Posted through `notifications.add` by something of ours.
    #[default]
    Yantrik,
    /// Arrived over `org.freedesktop.Notifications`.
    Freedesktop,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Yantrik => "yantrik",
            Self::Freedesktop => "freedesktop",
        }
    }
}

/// A button on a notification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NotificationAction {
    /// What the sender will be told was pressed. For a freedesktop notification this is the
    /// action key that goes back out in `ActionInvoked`. For one of ours it is the name of an
    /// action on the sending app's own control surface.
    pub id: String,
    /// What the button says.
    pub label: String,
    /// Arguments for that control-surface action.
    ///
    /// The freedesktop spec has `ActionInvoked` to tell a sender its button was pressed. Our own
    /// apps have no such signal — an app that posted a notification and then exited has nothing
    /// listening at all — so for a `yantrik` notification the shell presses the button *on the
    /// sender's behalf*, through the control surface the app already publishes, starting the app
    /// if it is not running. That needs the arguments, and this is where they ride: Download
    /// Manager's "Open folder" is `open_folder` with `{"id": 4}`, which is the same call a
    /// person makes by clicking the button in its window.
    ///
    /// Without this, a button on one of our notifications would be a control that does nothing,
    /// which this repo forbids.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<serde_json::Value>,
}

impl NotificationAction {
    /// A button that takes no arguments.
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            args: None,
        }
    }

    /// A button that calls the sending app's action with these arguments.
    pub fn with_args(mut self, args: serde_json::Value) -> Self {
        self.args = Some(args);
        self
    }
}

/// One notification, as stored and as handed to anyone who asks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notification {
    /// The store's own id, a decimal counter as a string: "1", "2", …
    ///
    /// A number rather than a uuid because the freedesktop spec's `Notify` has to answer with a
    /// `u32` the caller can later pass to `CloseNotification` or as `replaces_id`. With uuids
    /// the service had to keep a second id space and a map between them, and any gap between the
    /// two is a notification that cannot be closed by the program that sent it.
    pub id: String,
    /// Who is speaking, as the person would recognise them: "Downloads", "Chromium", "Calendar".
    pub app: String,
    pub title: String,
    pub body: String,
    pub urgency: Urgency,
    /// RFC 3339, UTC.
    pub created_at: String,
    pub read: bool,
    /// Dismissed notifications stay in the file for [`DISMISSED_TTL_SECS`] so that `since` can
    /// tell a client they went away, and so history survives closing a toast.
    pub dismissed: bool,
    pub actions: Vec<NotificationAction>,
    pub source: Source,
    /// The id this one replaced, when the sender asked to update rather than add.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replaces_id: Option<String>,
    /// Who this machine established sent it — beside `app`, which is only what was said.
    ///
    /// Optional on read so a store written before this existed still loads, and absent on a
    /// freedesktop notification, whose door has not asked the bus who was behind it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender: Option<Sender>,
    /// The store revision at which this notification last changed. `since` compares against it.
    pub revision: u64,
}

/// Who sent a notification, as this machine established it — kept beside `app`, which is only
/// what the sender said.
///
/// Found on 22 September 2026 (#114): a mind posted `app: "Yantrik"` with a body that was false
/// in every particular, and the stored record had nothing in it but what that caller had
/// written. The approval card had already been through this (#43) and answers with two lines it
/// refuses to merge — the name the caller gave itself, and the program the kernel says opened
/// the socket. A notification now carries the same two, and the shell draws them in the same
/// words.
///
/// None of this is set by the request. The service fills it in from `SO_PEERCRED` at the
/// moment the call arrives, which is the only moment the peer is certainly still there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Sender {
    /// `app` exactly as the caller gave it. `None` when it gave none and `app` was filled in
    /// from the verified program — so a reader can tell "it called itself Downloads" from "it
    /// was download-manager".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed: Option<String>,
    /// The program that opened the socket, as the shell's approval card prints it: the first
    /// thing in its ancestry a person would recognise, with its pid — or `could not be
    /// identified`, never a blank and never a guess.
    pub verified: String,
    /// The pid that line is about. `0` when nothing was established.
    #[serde(default)]
    pub pid: i32,
    /// Its `/proc/<pid>/exe`, empty when unknown. The line shows the command line, which says
    /// more; this is the path a person checks afterwards.
    #[serde(default)]
    pub exe: String,
    /// The process on the socket itself — not the first recognisable one `verified` and `exe`
    /// are about — was the installed desktop: the shell or this service, judged by the service
    /// at the moment of the call. The only thing that earns a card the desktop's own plain
    /// "Source process confirmed: yantrik-ui".
    ///
    /// Not `exe`, because `exe` names the first recognisable ancestor: a mind calling through a
    /// bridge the shell spawned has `yos` on the socket and `yantrik-ui` above it, and its `exe`
    /// is the shell's (security re-review of #611). `false` on a record written before this
    /// existed, which is the direction to fail in.
    ///
    /// It does not hold against a process running as the person's own user, which can exec the
    /// real binary after connecting; see docs/notifications-provenance.md.
    #[serde(default)]
    pub desktop: bool,
}

/// What a sender posts.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AddRequest {
    pub app: String,
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub urgency: Urgency,
    #[serde(default)]
    pub actions: Vec<NotificationAction>,
    #[serde(default)]
    pub source: Source,
    /// Replace this notification instead of adding another. Used by progress-style senders and
    /// by the freedesktop `replaces_id` argument.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replaces_id: Option<String>,
}

/// The answer to [`SINCE`]: where the store is now, and everything that changed to get there.
///
/// `changed` includes notifications that were dismissed or marked read, not only new ones —
/// a client that only heard about additions would keep drawing a toast the person just closed
/// from the notification centre.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Since {
    pub revision: u64,
    pub changed: Vec<Notification>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urgency_words_round_trip() {
        for u in [Urgency::Low, Urgency::Normal, Urgency::Critical] {
            assert_eq!(Urgency::parse(u.as_str()), u);
            assert_eq!(Urgency::from_hint_byte(u.hint_byte()), u);
        }
    }

    #[test]
    fn unknown_urgency_is_normal_not_an_error() {
        // A notification with a typo in one field still has to be delivered.
        assert_eq!(Urgency::parse("high"), Urgency::Normal);
        assert_eq!(Urgency::parse(""), Urgency::Normal);
        assert_eq!(Urgency::from_hint_byte(200), Urgency::Normal);
    }

    #[test]
    fn urgency_is_lowercase_on_the_wire() {
        assert_eq!(
            serde_json::to_string(&Urgency::Critical).unwrap(),
            "\"critical\""
        );
        assert_eq!(
            serde_json::from_str::<Urgency>("\"low\"").unwrap(),
            Urgency::Low
        );
    }

    #[test]
    fn a_record_written_before_the_sender_existed_still_loads() {
        // `~/.local/share/yantrik/notifications.json` on every machine that took the update
        // holds records with exactly these fields and nothing about who sent them. A field that
        // failed to parse would empty the whole store at the next start.
        let old = r#"{"id":"134","app":"Yantrik","title":"Studio finished","body":"","urgency":"normal","created_at":"2026-09-23T00:43:53Z","read":true,"dismissed":true,"actions":[],"source":"yantrik","revision":237}"#;
        let n: Notification = serde_json::from_str(old).expect("an old record parses");
        assert_eq!(n.sender, None);
        // And it is written back without inventing one.
        assert!(!serde_json::to_string(&n).unwrap().contains("sender"));

        let mut n = n;
        n.sender = Some(Sender {
            claimed: Some("Yantrik".into()),
            verified: "python -m hermes_cli.main gateway run (pid 689)".into(),
            pid: 689,
            exe: "/home/yantrik/.hermes/hermes-agent/venv/bin/python".into(),
            desktop: false,
        });
        let text = serde_json::to_string(&n).unwrap();
        let back: Notification = serde_json::from_str(&text).unwrap();
        assert_eq!(back.sender, n.sender);

        // A sender record written before `desktop` existed reads as not the desktop.
        let old_sender = r#"{"claimed":"Yantrik","verified":"yantrik-ui config.yaml (pid 7)","pid":7,"exe":"/opt/yantrik/bin/yantrik-ui"}"#;
        let s: Sender = serde_json::from_str(old_sender).expect("an old sender parses");
        assert!(!s.desktop);
    }

    #[test]
    fn clamp_counts_characters_not_bytes() {
        // Four characters, twelve bytes. Cutting at a byte offset here would write invalid
        // UTF-8 into the store and the next start would fail to parse the whole file.
        let devanagari = "नमस्ते";
        let cut = clamp(devanagari, 3);
        assert_eq!(cut.chars().count(), 3);
        assert!(devanagari.starts_with(&cut));
        assert_eq!(clamp("short", 100), "short");
    }
}
