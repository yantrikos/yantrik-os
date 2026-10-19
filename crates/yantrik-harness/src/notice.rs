//! A notice: a short card a harness hands the OS outside any turn
//! (design/initiative-inbox-2026-10-16.md).
//!
//! Sent as [`NOTICE`]: `{session, notice: {notice_id, family, title, body?, why, evidence?,
//! attention?, useful_until?, supersedes?, action?}}`, with no `turn_id`, because a notice is not
//! part of a turn. A harness sends one only when `harness.attach` answered with
//! [`FEATURE`] among its `features`.
//!
//! The host does not keep notices. It checks one with the contract's own `check`, then hands it
//! to a [`NoticeDesk`], which posts it to the one notification store on the harness's behalf,
//! naming the harness the host established at attach and nothing the request says. The answer is
//! the store's, given only after the store acknowledged; a store that cannot be reached is
//! `{"stored": false, "retry_after_secs": N}`, never `true`.
//!
//! The same desk carries the other two halves of the wire: the person's initiative settings, which
//! `attach` and every `poll` answer carry, and the person's feedback on this harness's cards,
//! which its `poll` carries as `notice_feedback` until a later poll says `feedback_ack`.

pub use yantrik_ipc_contracts::notifications::notice::{answer, check, FEATURE, OS_HARNESS, RETRY_UNAVAILABLE_SECS};

/// The method a harness calls to hand the OS a notice.
pub const NOTICE: &str = "harness.notice";

/// What the host hands the desk: the notice as the harness sent it, and who sent it as the host
/// established it at attach.
#[derive(Clone, Debug, PartialEq)]
pub struct NoticeSend {
    /// The id the harness attached under (`mind`, `hermes`): what dedupe, supersede and feedback
    /// are keyed by. Stable across the harness's restarts, so its cards and its feedback cursor
    /// are still its own after one.
    ///
    /// Shared attaches of one harness (`hermes.1`, `hermes.2`) are one sender: they post and poll
    /// as `hermes`, under one feedback cursor, so feedback acknowledged by one instance is not
    /// served again to another. That is intended — the person answered the harness, not one of
    /// its processes — and a harness that wants each instance to hear for itself attaches each
    /// under an id of its own.
    pub harness: String,
    /// The name it gave itself, shown beside what was verified and never instead of it.
    pub name: String,
    /// The process that attached and the account it ran as, as the kernel said at attach.
    pub pid: Option<u32>,
    pub uid: Option<u32>,
    pub notice: serde_json::Value,
    /// The store makes the id: a plain `{text}` notice from an older harness has none.
    pub os_id: bool,
}

/// Where the host posts notices and reads what goes back to a harness. The shell provides it,
/// over the notifications service; the host stays transport-free.
///
/// Every method is called without the host's lock held: each may do IO.
pub trait NoticeDesk: Send + Sync {
    /// Post one notice. `Ok` is the store's answer (`stored`, `duplicate`, `clamped`,
    /// `retry_after_secs`, `refused`); `Err` says the store could not be reached.
    fn post(&self, send: &NoticeSend) -> Result<serde_json::Value, String>;
    /// The person's settings, as `{quiet, off, inbox_only}`.
    fn initiative(&self) -> serde_json::Value;
    /// The feedback waiting for `harness`, after taking `ack` as read. `pid` and `uid` are the
    /// attach's, as the kernel said: the store serves a harness id's feedback only to the account
    /// and program that id is bound to.
    fn feedback(&self, harness: &str, pid: Option<u32>, uid: Option<u32>, ack: Option<u64>) -> Result<Vec<serde_json::Value>, String>;
}

/// A plain `{text}` notice sent with no turn in flight, as a notice the store can keep: the first
/// line as its title, the rest as its body, filed as a delegated job's result, waiting in the
/// inbox. The store clamps it as it clamps any other.
pub fn from_text(text: &str, harness_name: &str) -> serde_json::Value {
    let text = text.trim();
    let (title, body) = text.split_once('\n').unwrap_or((text, ""));
    serde_json::json!({
        "notice_id": "os",
        "family": "delegated",
        "title": title.trim(),
        "body": body.trim(),
        "why": format!("Sent by {harness_name} while nothing was running"),
        "attention": "inbox",
    })
}

/// The store's answer as the wire has it: only the spec's keys. The store adds its own record id,
/// which is nothing a mind needs and nothing it should start depending on.
pub fn wire_answer(answer: &serde_json::Value) -> serde_json::Value {
    const KEYS: &[&str] = &["stored", "notice_id", "duplicate", "clamped", "retry_after_secs", "why", "refused"];
    let mut out = serde_json::Map::new();
    if let Some(obj) = answer.as_object() {
        for key in KEYS {
            if let Some(v) = obj.get(*key) {
                out.insert(key.to_string(), v.clone());
            }
        }
    }
    serde_json::Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_text_notice_becomes_a_delegated_card_the_contract_accepts() {
        let raw = from_text("Backup finished\n42 files, 1.2 GB.\nNo errors.", "Hermes");
        let checked = check(&raw).expect("the store will take it");
        assert_eq!(checked.title, "Backup finished");
        assert_eq!(checked.body, "42 files, 1.2 GB.\nNo errors.");
        assert_eq!(checked.why, "Sent by Hermes while nothing was running");
        assert_eq!(checked.family.as_str(), "delegated");
        assert_eq!(checked.attention.as_str(), "inbox");
        // One line is a title and no body.
        assert_eq!(check(&from_text("done", "pi")).unwrap().body, "");
    }

    #[test]
    fn the_answer_carries_only_the_wire_keys() {
        let store = serde_json::json!({ "stored": true, "notice_id": "n", "id": "41", "clamped": ["body"] });
        assert_eq!(wire_answer(&store), serde_json::json!({ "stored": true, "notice_id": "n", "clamped": ["body"] }));
    }
}
