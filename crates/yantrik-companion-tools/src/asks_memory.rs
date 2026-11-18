//! The card for forgetting one memory (#1311): `forget_memory` asks `change_item` immediately
//! before it tombstones the row. The card shows the memory's own text, quoted, so what is about
//! to be forgotten is what the person reads and not the model's description of it. A text the
//! card cannot show whole is not asked about, and the memory stays
//! ([`crate::asks::Question::fits`]).

use crate::asks::{act, Card, Question};

/// Ask, then tombstone memory `rid`. `Err` is a sentence for the model: the person's no, or the
/// left as it was. `lock` hands out the database; it is taken to read, let go while the card is
/// up (a wait of up to two minutes must not stop everything else that uses the database, #1313 M2)
/// and taken again inside the effect.
pub fn forget_one<G: std::ops::Deref<Target = rusqlite::Connection>>(lock: impl Fn() -> G, rid: &str) -> Result<usize, String> {
    let text: String = lock()
        .query_row("SELECT text FROM memories WHERE rid = ?1", [rid], |r| r.get(0))
        .map_err(|e| format!("Failed to read memory '{rid}', so nothing was forgotten: {e}"))?;
    let card = Card::new("forget_memory").plain("memory", rid.chars().take(12).collect::<String>()).text("text", &text);
    act(Question::ChangeItem, &card, || {
        lock().execute(
            "UPDATE memories SET half_life = 1.0, consolidation_status = 'tombstoned' WHERE rid = ?1",
            [rid],
        )
        .map_err(|e| format!("Failed to forget memory '{rid}': {e}"))
    })
}
