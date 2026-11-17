//! The card for forgetting one memory (#1311): `forget_memory` asks `change_item` immediately
//! before it tombstones the row. The card shows the memory's own text, quoted, so what is about
//! to be forgotten is what the person reads and not the model's description of it. A text the
//! card cannot show whole is not asked about, and the memory stays
//! ([`crate::asks::Question::fits`]).

use crate::asks::{act, Card, Question};

/// Ask, then tombstone memory `rid`. `Err` is a sentence for the model: the person's no, or the
/// reason nobody was asked, or that the row could not be changed; in each case it says what was
/// left as it was.
pub fn forget_one(conn: &rusqlite::Connection, rid: &str) -> Result<usize, String> {
    let text: String = conn
        .query_row("SELECT text FROM memories WHERE rid = ?1", [rid], |r| r.get(0))
        .map_err(|e| format!("Failed to read memory '{rid}', so nothing was forgotten: {e}"))?;
    let card = Card::new("forget_memory").plain("memory", rid.chars().take(12).collect::<String>()).text("text", &text);
    act(Question::ChangeItem, &card, || {
        conn.execute(
            "UPDATE memories SET half_life = 1.0, consolidation_status = 'tombstoned' WHERE rid = ?1",
            [rid],
        )
        .map_err(|e| format!("Failed to forget memory '{rid}': {e}"))
    })
}
