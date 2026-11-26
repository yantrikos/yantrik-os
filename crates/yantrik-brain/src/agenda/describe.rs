//! `describe`: what the agenda looks like, for people and for the machine test.
//!
//! Private things stay private: the output carries counts, and for the top entries kind, state,
//! next step, score, terms, when and a reason in words (never a note's or a message's text). A title appears only when the entry's source is the person's own
//! promise or task (`title_public`), and a memory's text is never in the table to begin with.

use rusqlite::Connection;
use serde_json::{json, Map, Value};

use super::salience::ranked;
use super::senses::in_words;
use super::store::reason_of;

const TOP: usize = 5;

pub fn describe(conn: &Connection, now: f64) -> rusqlite::Result<Value> {
    let mut by_kind = Map::new();
    let mut by_state = Map::new();
    let mut by_next = Map::new();
    let mut count = 0i64;
    {
        let mut stmt = conn.prepare(
            "SELECT kind, state, next_step, COUNT(*) FROM agenda \
             WHERE state NOT IN ('done','dropped') GROUP BY kind, state, next_step",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)?))
        })?;
        for row in rows.flatten() {
            let (k, s, n, c) = row;
            count += c;
            bump(&mut by_kind, &k, c);
            bump(&mut by_state, &s, c);
            bump(&mut by_next, &n, c);
        }
    }
    let updated_at: Option<f64> = conn.query_row("SELECT MAX(updated_at) FROM agenda", [], |r| r.get(0))?;
    let top: Vec<Value> = ranked(conn, now, TOP)?
        .into_iter()
        .map(|(e, s)| {
            let terms: Map<String, Value> =
                s.terms.iter().map(|(n, v)| ((*n).to_string(), json!(v))).collect();
            let mut o = Map::new();
            o.insert("kind".into(), json!(e.kind.as_str()));
            o.insert("state".into(), json!(e.state.as_str()));
            o.insert("next_step".into(), json!(e.next_step.as_str()));
            o.insert("score".into(), json!(s.total));
            o.insert("terms".into(), Value::Object(terms));
            if e.title_public {
                o.insert("title".into(), json!(e.title));
            }
            if let Some(due) = e.due_at {
                o.insert("due_at".into(), json!(due));
                let when = if due >= now { format!("in {}", in_words(due - now)) } else { format!("{} ago", in_words(now - due)) };
                o.insert("when".into(), json!(when));
            }
            // What a sense said in words, or else the largest term of the score.
            let reason = reason_of(conn, e.id).unwrap_or_else(|| {
                let (name, _) = s.terms.iter().fold(("deadline", 0.0), |b, (n, v)| if *v > b.1 { (*n, *v) } else { b });
                format!("Ranked mostly by its {name}.")
            });
            o.insert("reason".into(), json!(reason));
            Value::Object(o)
        })
        .collect();
    Ok(json!({
        "count": count,
        "by_kind": by_kind,
        "by_state": by_state,
        "by_next_step": by_next,
        "updated_at": updated_at,
        "top": top,
        "moments": super::prepare::describe(conn).unwrap_or_default(),
    }))
}

fn bump(m: &mut Map<String, Value>, key: &str, by: i64) {
    let cur = m.get(key).and_then(Value::as_i64).unwrap_or(0);
    m.insert(key.to_string(), json!(cur + by));
}
