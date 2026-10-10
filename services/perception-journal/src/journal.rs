//! Perception's records, on the shared segment journal.
//!
//! The mechanism — append, `fsync`, *then* advance the cursor, with a bounded budget that drops
//! the oldest segments loudly — lives in `yantrik-journal`, because the action ledger wants the
//! same guarantees over different records. What lives here is what only perception knows: what
//! an observation is, what a hole in perception's stream is, and how much of each to keep.

use yantrik_journal::{Entry, Limits};

/// Bytes per segment before rolling to the next one.
///
/// Small enough that the oldest data can be dropped at a useful granularity, large enough that
/// rolling is rare. Observations run a few hundred bytes, so this is a few thousand of them.
const SEGMENT_BYTES: u64 = 2 * 1024 * 1024;

/// How much history to keep, total.
///
/// The journal is short-lived infrastructure for replay and diagnosis, not an archive and not a
/// second searchable store. A bounded consumer of a bounded producer: when this fills, the oldest
/// segment is deleted, and that deletion is recorded as a coverage gap like any other loss.
const MAX_BYTES: u64 = 64 * 1024 * 1024;

/// One line in a segment.
///
/// Either an observation exactly as the eye reported it, or a note about something that could not
/// be observed. Both are records; only one is evidence.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(tag = "record", rename_all = "snake_case")]
pub enum Record {
    /// An observation, carried through unmodified. The journal does not interpret.
    Observation {
        /// The eye's sequence number. This is the idempotency key: a replayed record has the same
        /// `seq` and must not become a second memory.
        seq: u64,
        /// Which run of perception-service produced it. A restart resets `seq` to zero, so
        /// without this a replay after a restart would look like a rewind rather than a new
        /// stream, and the two would interleave into nonsense.
        source: String,
        at: f64,
        observation: serde_json::Value,
    },
    /// A hole. Recorded in-band, in sequence, so a reader walking the journal encounters the
    /// absence where it happened rather than having to ask a separate health endpoint whether
    /// what it just read was complete.
    Gap {
        /// First sequence known lost, and one past the last.
        from: u64,
        to: u64,
        at: f64,
        /// Why it is missing. The two causes need different responses: `ring_overrun` means the
        /// reader was too slow or absent, `retention` means the journal itself aged it out.
        ///
        /// Owned rather than `&'static str`: this type is deserialized when the journal is read
        /// back, and a borrowed field would demand the input outlive the record.
        cause: String,
    },
}

impl Entry for Record {
    fn seq(&self) -> u64 {
        match self {
            Record::Observation { seq, .. } => *seq,
            Record::Gap { to, .. } => *to,
        }
    }

    fn is_gap(&self) -> bool {
        matches!(self, Record::Gap { .. })
    }
}

/// Perception's numbers for the shared mechanism.
pub const LIMITS: Limits = Limits { segment_bytes: SEGMENT_BYTES, max_bytes: MAX_BYTES };

pub type Journal = yantrik_journal::Journal<Record>;
pub use yantrik_journal::Cursor;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_is_one_json_line_exactly_as_the_journal_has_always_written_it() {
        // The on-disk format is the interface this service can never change without stranding
        // every journal already deployed. The mechanism moved to a shared crate; this asserts
        // that writing through it still produces the byte-identical line. If the tag, the field
        // names or the variant names ever move, this fails and the argument has to be had on
        // purpose.
        let dir = std::env::temp_dir().join(format!("journal-format-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let mut j = Journal::open(&dir, LIMITS).unwrap();
        j.append(&Record::Observation {
            seq: 1,
            source: "run-1".into(),
            at: 2.5,
            observation: serde_json::json!({ "path": "/x" }),
        })
        .unwrap();

        let segment = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .find(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
            .expect("appending created a segment");
        let on_disk = std::fs::read_to_string(segment.path()).unwrap();
        assert_eq!(
            on_disk,
            "{\"record\":\"observation\",\"seq\":1,\"source\":\"run-1\",\"at\":2.5,\"observation\":{\"path\":\"/x\"}}\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
