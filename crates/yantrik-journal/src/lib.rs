//! Append-only segments, and a cursor that only moves after the bytes are on disk.
//!
//! The ordering here is the entire point of the crate. perception-service holds observations in a
//! 2048-entry ring that overwrites its oldest entry when full. That is the right structure for a
//! feed and a disqualifying one for a record: whatever the ring drops is gone, and nothing
//! upstream can be asked for it again.
//!
//! Measured on the deployed machine while an agent was working: `next_seq 5885`, ring holding
//! 2048, **`missed 3837`**. The ring wrapped nearly twice, and the only reason anyone knew was
//! that the service counts what it drops. Eight minutes of an agent's actual work was unavailable
//! by the time anybody looked — not because a component failed, but because nothing was draining.
//!
//! So: append, `fsync`, *then* advance the cursor. A crash between the append and the cursor
//! write replays the tail, which is why every record carries the source's sequence number and
//! replay is idempotent by construction. The opposite ordering loses data silently, which is the
//! one outcome this journal exists to prevent.
//!
//! The mechanism is generic over what gets written — perception-journal's observations and the
//! action ledger (#148) want the same durability with different record types. What a record *is*
//! lives in the consumer; what the journal guarantees (durable append, ordered replay, a bounded
//! budget that drops the oldest segments loudly) lives here.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

/// What the journal needs to know about a record to store it.
///
/// A record is a line of JSON; the journal never interprets it. It only needs to place it in the
/// stream: `seq` is where the record sits in the source's sequence (for a gap, one past its
/// hole, so a reader resuming at the end of the hole does not ask for it again), and `is_gap`
/// distinguishes a hole from evidence for the counters.
pub trait Entry: serde::Serialize + serde::de::DeserializeOwned {
    /// Position in the source's sequence — the idempotency key replay depends on.
    fn seq(&self) -> u64;

    /// Whether this record is a hole rather than an observation.
    fn is_gap(&self) -> bool;
}

/// The size policy, chosen by the consumer rather than the mechanism.
///
/// Different consumers write records of very different sizes; the numbers that suit one are
/// arbitrary for the other. The shape of the policy — roll at a ceiling, keep a bounded total,
/// delete oldest-first past it — is what is shared.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Bytes per segment before rolling to the next one.
    ///
    /// Small enough that the oldest data can be dropped at a useful granularity, large enough
    /// that rolling is rare.
    pub segment_bytes: u64,

    /// How much history to keep, total.
    ///
    /// The journal is short-lived infrastructure for replay and diagnosis, not an archive and
    /// not a second searchable store. A bounded consumer of a bounded producer: when this fills,
    /// the oldest segment is deleted, and that deletion is recorded as a coverage gap like any
    /// other loss.
    pub max_bytes: u64,
}

pub struct Journal<E: Entry> {
    dir: PathBuf,
    current: File,
    current_path: PathBuf,
    current_bytes: u64,
    limits: Limits,
    kind: PhantomData<E>,
    /// Highest source sequence durably written. The cursor on disk lags this only between the
    /// `fsync` and the cursor write, which is the window replay exists to cover.
    pub last_written: u64,
    pub records: u64,
    pub gaps: u64,
}

impl<E: Entry> Journal<E> {
    pub fn open(dir: impl AsRef<Path>, limits: Limits) -> std::io::Result<Self> {
        let dir = dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir)?;

        let (current_path, current_bytes) = match newest_segment(&dir)? {
            Some((path, bytes)) if bytes < limits.segment_bytes => (path, bytes),
            _ => (dir.join(segment_name(next_segment_index(&dir)?)), 0),
        };
        let current = OpenOptions::new().create(true).append(true).open(&current_path)?;

        Ok(Self {
            dir,
            current,
            current_path,
            current_bytes,
            limits,
            kind: PhantomData,
            last_written: 0,
            records: 0,
            gaps: 0,
        })
    }

    /// Append one record and get it onto the disk before returning.
    ///
    /// `sync_data` rather than `sync_all`: the contents must survive, the directory metadata
    /// matters only when a segment is created, which is handled at roll time.
    pub fn append(&mut self, record: &E) -> std::io::Result<()> {
        let mut line = serde_json::to_vec(record)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        line.push(b'\n');

        self.current.write_all(&line)?;
        self.current.sync_data()?;

        self.current_bytes += line.len() as u64;
        self.records += 1;
        if record.is_gap() {
            self.gaps += 1;
        } else {
            self.last_written = record.seq().max(self.last_written);
        }

        if self.current_bytes >= self.limits.segment_bytes {
            self.roll()?;
        }
        Ok(())
    }

    fn roll(&mut self) -> std::io::Result<()> {
        let index = next_segment_index(&self.dir)?;
        self.current_path = self.dir.join(segment_name(index));
        self.current = OpenOptions::new().create(true).append(true).open(&self.current_path)?;
        self.current_bytes = 0;
        self.enforce_retention()
    }

    /// Delete the oldest segments until the journal is back inside its budget.
    ///
    /// Deliberately loud. Dropping a segment is losing history, and the whole argument of this
    /// journal is that lost history must never be silent — so it is logged with the range that
    /// went, and a reader that had not caught up will meet a `Gap` where those records were.
    fn enforce_retention(&mut self) -> std::io::Result<()> {
        let mut segments = segments(&self.dir)?;
        let mut total: u64 = segments.iter().map(|(_, bytes)| bytes).sum();

        while total > self.limits.max_bytes && segments.len() > 1 {
            let (path, bytes) = segments.remove(0);
            tracing::warn!(
                segment = %path.display(),
                bytes,
                "Journal retention reached; dropping the oldest segment. History before this \
                 point is no longer replayable."
            );
            std::fs::remove_file(&path)?;
            total -= bytes;
        }
        Ok(())
    }

    pub fn bytes_on_disk(&self) -> u64 {
        segments(&self.dir).map(|s| s.iter().map(|(_, b)| b).sum()).unwrap_or(0)
    }

    /// Every record with a sequence at or after `from`, in order.
    ///
    /// The interpretation worker's read path. It keeps its own cursor — deliberately separate
    /// from the ingestion cursor, so a slow or crashed interpreter cannot stall the drain, and a
    /// restarted interpreter can rewind without asking the source for anything.
    pub fn read_from(&self, from: u64, limit: usize) -> std::io::Result<Vec<E>> {
        let mut out = Vec::new();
        for (path, _) in segments(&self.dir)? {
            let file = File::open(&path)?;
            for line in BufReader::new(file).lines() {
                let line = line?;
                if line.trim().is_empty() {
                    continue;
                }
                // A truncated final line is expected after a crash mid-append. Skip it rather
                // than refusing to serve the rest — the whole point of fsync-before-cursor is
                // that the tail may be incomplete and the missing part will be replayed.
                let Ok(record) = serde_json::from_str::<E>(line.as_str()) else { continue };
                if record.seq() >= from {
                    out.push(record);
                    if out.len() >= limit {
                        return Ok(out);
                    }
                }
            }
        }
        Ok(out)
    }
}

fn segment_name(index: u64) -> String {
    format!("{index:08}.jsonl")
}

fn segments(dir: &Path) -> std::io::Result<Vec<(PathBuf, u64)>> {
    let mut out: Vec<(PathBuf, u64)> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
        .filter_map(|e| e.metadata().ok().map(|m| (e.path(), m.len())))
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

fn newest_segment(dir: &Path) -> std::io::Result<Option<(PathBuf, u64)>> {
    Ok(segments(dir)?.pop())
}

fn next_segment_index(dir: &Path) -> std::io::Result<u64> {
    let highest = segments(dir)?
        .iter()
        .filter_map(|(p, _)| p.file_stem()?.to_str()?.parse::<u64>().ok())
        .max();
    Ok(highest.map(|h| h + 1).unwrap_or(0))
}

// ── The ingestion cursor ────────────────────────────────────────────

/// Where the drain got to, on disk.
///
/// Written *after* the observations it refers to are durable. That ordering is the contract: on
/// restart the drain resumes from here and re-reads anything appended but not yet acknowledged,
/// which is safe precisely because every record carries the source's own sequence number.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Cursor {
    pub next_seq: u64,
    /// The perception-service run this cursor belongs to. A restart upstream resets sequence
    /// numbers, and resuming from a stale cursor would silently skip the new run's first
    /// several thousand observations while looking perfectly healthy.
    pub source: String,
}

impl Cursor {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        // Write-and-rename: a cursor half-written by a crash would be worse than an old one,
        // because an old cursor replays and a corrupt one resets to zero.
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_vec(self)?)?;
        std::fs::rename(&tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in with the two shapes every consumer's records have: a thing that happened,
    /// and a hole where things could not.
    #[derive(Debug, serde::Serialize, serde::Deserialize)]
    #[serde(tag = "shape", rename_all = "snake_case")]
    enum TestEntry {
        Note { seq: u64, text: String },
        Gap { from: u64, to: u64 },
    }

    impl Entry for TestEntry {
        fn seq(&self) -> u64 {
            match self {
                TestEntry::Note { seq, .. } => *seq,
                TestEntry::Gap { to, .. } => *to,
            }
        }

        fn is_gap(&self) -> bool {
            matches!(self, TestEntry::Gap { .. })
        }
    }

    fn lab(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("journal-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn note(seq: u64) -> TestEntry {
        TestEntry::Note { seq, text: format!("event {seq}") }
    }

    fn limits() -> Limits {
        Limits { segment_bytes: 2 * 1024 * 1024, max_bytes: 64 * 1024 * 1024 }
    }

    #[test]
    fn what_is_appended_can_be_read_back_in_order() {
        let dir = lab("roundtrip");
        let mut j = Journal::<TestEntry>::open(&dir, limits()).unwrap();
        for seq in 0..5 {
            j.append(&note(seq)).unwrap();
        }
        let back = j.read_from(0, 100).unwrap();
        assert_eq!(back.len(), 5);
        assert_eq!(j.last_written, 4);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_reader_can_resume_from_where_it_stopped() {
        // The interpretation worker's restart. It must be able to rewind without asking the eye,
        // because the eye's ring will not have it any more.
        let dir = lab("resume");
        let mut j = Journal::<TestEntry>::open(&dir, limits()).unwrap();
        for seq in 0..10 {
            j.append(&note(seq)).unwrap();
        }
        let tail = j.read_from(7, 100).unwrap();
        assert_eq!(tail.len(), 3, "everything from 7 onwards, and nothing before it");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_gap_is_a_record_in_the_stream_not_a_footnote() {
        // A reader walking the journal must meet the hole where it happened. If coverage lived
        // only in a health endpoint, a reader could process straight across 3837 missing
        // observations and never know its account of that period was fiction.
        let dir = lab("gap");
        let mut j = Journal::<TestEntry>::open(&dir, limits()).unwrap();
        j.append(&note(0)).unwrap();
        j.append(&TestEntry::Gap { from: 1, to: 3838 }).unwrap();
        j.append(&note(3838)).unwrap();

        let all = j.read_from(0, 100).unwrap();
        assert_eq!(all.len(), 3);
        assert!(matches!(all[1], TestEntry::Gap { from: 1, to: 3838 }));
        assert_eq!(j.gaps, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_cursor_survives_a_restart_and_carries_which_run_it_belongs_to() {
        // Without `source`, a perception-service restart resets seq to 0, and a cursor at 5000
        // would skip the new run's first 5000 observations while reporting perfect health.
        let dir = lab("cursor");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cursor.json");

        Cursor { next_seq: 5885, source: "run-1".into() }.save(&path).unwrap();
        let loaded = Cursor::load(&path);
        assert_eq!(loaded.next_seq, 5885);
        assert_eq!(loaded.source, "run-1");

        // A missing cursor is a fresh start, not a crash.
        assert_eq!(Cursor::load(&dir.join("absent.json")).next_seq, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_truncated_final_line_does_not_poison_the_rest() {
        // Expected after a crash between write and fsync. The tail will be replayed; refusing to
        // serve the good records before it would turn a recoverable partial write into an outage.
        let dir = lab("torn");
        let mut j = Journal::<TestEntry>::open(&dir, limits()).unwrap();
        j.append(&note(0)).unwrap();
        j.append(&note(1)).unwrap();
        drop(j);

        let seg = segments(&dir).unwrap().pop().unwrap().0;
        let mut f = OpenOptions::new().append(true).open(&seg).unwrap();
        f.write_all(br#"{"shape":"note","seq":2,"te"#).unwrap();

        let j = Journal::<TestEntry>::open(&dir, limits()).unwrap();
        let back = j.read_from(0, 100).unwrap();
        assert_eq!(back.len(), 2, "the two complete records survive the torn third");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn past_the_budget_the_oldest_segments_are_dropped_and_the_reader_meets_a_gap() {
        // Retention is the journal's own coverage loss, and it has to behave like any other:
        // the surviving tail stays readable and in order, and what is gone is simply not
        // there — a reader that expected it finds its sequence starts above zero.
        let dir = lab("retention");
        let tight = Limits { segment_bytes: 200, max_bytes: 600 };
        let mut j = Journal::<TestEntry>::open(&dir, tight).unwrap();
        for seq in 0..40 {
            j.append(&note(seq)).unwrap();
        }

        assert!(j.bytes_on_disk() <= 600 + 200, "one segment of slack, not an unbounded budget");
        assert!(segments(&dir).unwrap().len() > 1, "the budget must not delete down to nothing");

        let back = j.read_from(0, 1000).unwrap();
        assert!(back.len() < 40, "the oldest segments are gone, so not everything survives");
        let first = back.first().expect("the newest segment always survives");
        assert!(first.seq() > 0, "the reader meets the loss as a hole at the start, not silence");
        let seqs: Vec<u64> = back.iter().map(|e| e.seq()).collect();
        for pair in seqs.windows(2) {
            assert_eq!(pair[1], pair[0] + 1, "what survives is contiguous and in order");
        }
        assert_eq!(*seqs.last().unwrap(), 39);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
