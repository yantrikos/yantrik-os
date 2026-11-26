//! The keys a person's open turn already holds, kept through a lock and no longer (#1430).
//!
//! The gateway reads an account's key from the vault at each call. A turn is many calls over
//! minutes, so a lock pressed by hand mid-turn shut the vault and the turn's next call died with
//! "the key could not be read". A key read while a turn is open is kept here, and is the answer
//! only when the vault gives none; the last open turn closing wipes them all ([`release`], wired
//! to `Now::on_close`). Nothing is kept for a call made with no turn open, so a lock still takes
//! the keys of everything but the turn already running. The idle lock never reaches here: it
//! waits for the turn (`turn_now::hold_at`).

use std::collections::HashMap;
use std::sync::Mutex;

/// A key held for the turn; its bytes are overwritten when it is dropped.
struct Held(String);

impl Drop for Held {
    fn drop(&mut self) {
        // SAFETY: every byte is overwritten with 0, which is valid UTF-8, so the String stays valid.
        for b in unsafe { self.0.as_bytes_mut() } {
            // SAFETY: `b` is a valid, aligned, exclusive reference.
            unsafe { std::ptr::write_volatile(b, 0) };
        }
    }
}

static HELD: Mutex<Option<HashMap<String, Held>>> = Mutex::new(None);

/// One vault value by id, from `vault` when it gives one, else the one this turn already held.
pub fn read(id: &str, vault: impl FnOnce(&str) -> Option<String>) -> Option<String> {
    read_with(id, vault, || yantrik_gateway::now::shared().any_open())
}

fn read_with(id: &str, vault: impl FnOnce(&str) -> Option<String>, turn_open: impl Fn() -> bool) -> Option<String> {
    // The vault is asked outside the lock: it may take its time, and `release` must not wait on it.
    let read = vault(id).filter(|v| !v.is_empty());
    let mut held = HELD.lock().unwrap_or_else(|e| e.into_inner());
    // Asked inside the lock: `release` runs after the last turn is gone, so a turn seen open here
    // is released later, and a key is never left behind by a turn that ended.
    let open = turn_open();
    if !open {
        held.take();
    }
    match read {
        Some(v) => {
            if open {
                held.get_or_insert_with(HashMap::new).insert(id.to_string(), Held(v.clone()));
            }
            Some(v)
        }
        None => held.as_ref().and_then(|m| m.get(id)).map(|k| k.0.clone()),
    }
}

/// The last open turn closed: nothing stays held.
pub fn release() {
    drop(HELD.lock().unwrap_or_else(|e| e.into_inner()).take());
}

#[cfg(test)]
mod tests {
    use super::*;

    // One test owns the static; the steps are in order.
    #[test]
    fn a_turn_keeps_its_key_through_a_lock_and_not_after_it_ends() {
        release();
        // A call with no turn open keeps nothing: after a lock it finds nothing.
        assert_eq!(read_with("k1", |_| Some("secret".into()), || false).as_deref(), Some("secret"));
        assert_eq!(read_with("k1", |_| None, || false), None);
        // A turn open: the key read is kept, and the vault shut (a manual lock) still answers it.
        assert_eq!(read_with("k1", |_| Some("secret".into()), || true).as_deref(), Some("secret"));
        assert_eq!(read_with("k1", |_| None, || true).as_deref(), Some("secret"), "the running turn keeps the key it holds");
        assert_eq!(read_with("other", |_| None, || true), None, "only what the turn read");
        // The turn ends: gone.
        release();
        assert_eq!(read_with("k1", |_| None, || true), None, "not kept after the turn");
        assert_eq!(read_with("k1", |_| None, || false), None);
    }
}
