//! The watcher's own small rules.

use super::*;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

static PRIVATE_NOW: AtomicBool = AtomicBool::new(false);
static PRIVATE_READS: AtomicU64 = AtomicU64::new(0);

fn read_private() -> bool {
    PRIVATE_READS.fetch_add(1, Ordering::SeqCst);
    PRIVATE_NOW.load(Ordering::SeqCst)
}

/// A firehose of signals is not a firehose of file reads, and the answer is never older than
/// its limit.
#[test]
fn private_mode_is_read_at_most_once_per_interval() {
    let start = Instant::now();
    let mut privacy = Privacy { read: read_private, value: false, read_at: None };
    for micros in 0..1_000u64 {
        assert!(!privacy.check(start + Duration::from_micros(micros)));
    }
    assert_eq!(PRIVATE_READS.load(Ordering::SeqCst), 1, "a thousand looks inside the interval, one read");
    PRIVATE_NOW.store(true, Ordering::SeqCst);
    assert!(!privacy.check(start + PRIVATE_FRESH / 2), "still the last reading");
    assert!(privacy.check(start + PRIVATE_FRESH), "and a fresh one once it is due");
    assert_eq!(PRIVATE_READS.load(Ordering::SeqCst), 2);
}
