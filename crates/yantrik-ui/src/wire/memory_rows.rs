//! What the Memory screen shows of a recall: statements a person can read and correct, not rows
//! of a database (the art direction of 20 Oct 2026, design/art-direction-2026-10-20.md, F).
//!
//! On VM 520 the screen showed "episodic" and "semantic" pills, an importance bar per row, the
//! machine's own records ("Security event [information_leak]: llm_response — …") among facts about
//! the person, and "User lives in Bentonville, Arkansas" twice. Here:
//!
//! - the machine's own records stay off a list about the person: an `audit/` or `system/` domain
//!   is what a tool or a watcher wrote, not something learned about anyone;
//! - a safety event is said plainly, not hidden: a firewall stopping a reply is something the
//!   person may want to know about, but not as an enum;
//! - the same statement twice is one row, which says how many times it was saved;
//! - each row says where it came from in words: when it was saved, and what kind of thing it is.

use crate::bridge::MemoryResult;

/// A row as the screen draws it.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub rid: String,
    pub text: String,
    /// "A fact · saved 2d ago", "Something that happened · saved 4m ago", "Saved 3 times · 1h ago".
    pub origin: String,
    pub score: f64,
}

/// The rows for `results` (newest or best first, as recalled), at `now` (unix seconds).
pub fn rows(results: &[MemoryResult], now: f64, ago: impl Fn(f64) -> String) -> Vec<Row> {
    let mut out: Vec<(Row, usize, String)> = Vec::new();
    for r in results {
        if is_machine_record(&r.domain) {
            continue;
        }
        let text = plain(&r.text);
        if text.is_empty() {
            continue;
        }
        let key = key_of(&text);
        if let Some((_, count, _)) = out.iter_mut().find(|(_, _, k)| *k == key) {
            *count += 1;
            continue;
        }
        let row = Row { rid: r.rid.clone(), text, origin: String::new(), score: r.score };
        out.push((row, 1, key));
    }
    // The origin is written once the count is known.
    out.into_iter()
        .map(|(mut row, count, _)| {
            let first = results.iter().find(|r| r.rid == row.rid).expect("the row's own result");
            let when = ago(now - first.created_at);
            row.origin = if count > 1 {
                format!("Saved {count} times \u{b7} {when}")
            } else {
                format!("{} \u{b7} saved {when}", kind(&first.memory_type, &row.text))
            };
            row
        })
        .collect()
}

/// A tool's record or a watcher's: not something learned about a person.
fn is_machine_record(domain: &str) -> bool {
    let d = domain.trim();
    d.starts_with("audit/") || d.starts_with("system/") || d == "audit" || d == "system"
}

/// The statement in words: a safety event said as one, everything else as saved.
fn plain(text: &str) -> String {
    let t = text.trim();
    // "Security event [information_leak]: llm_response — Response contained sensitive internal
    // information": what the safety check did, in its own last clause.
    if let Some(rest) = t.strip_prefix("Security event [") {
        let what = rest
            .rsplit_once(" \u{2014} ")
            .or_else(|| rest.rsplit_once(" - "))
            .map(|(_, w)| w)
            .unwrap_or(rest)
            .trim();
        let what = what.trim_end_matches('.');
        if !what.is_empty() && !what.contains(']') {
            let mut chars = what.chars();
            let first = chars.next().map(|c| c.to_lowercase().collect::<String>()).unwrap_or_default();
            return format!("Safety check: {first}{}.", chars.as_str());
        }
    }
    t.to_string()
}

/// What makes two statements the same: case, spaces and a final full stop aside.
fn key_of(text: &str) -> String {
    text.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ").trim_end_matches('.').to_string()
}

/// What kind of thing it is, in a word or two.
fn kind(memory_type: &str, text: &str) -> &'static str {
    if text.starts_with("Safety check:") {
        return "A safety check";
    }
    match memory_type.trim() {
        "episodic" => "Something that happened",
        "semantic" => "A fact",
        "procedural" => "How to do something",
        _ => "Saved",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(rid: &str, text: &str, kind: &str, domain: &str, at: f64) -> MemoryResult {
        MemoryResult {
            rid: rid.into(),
            text: text.into(),
            memory_type: kind.into(),
            importance: 0.5,
            valence: 0.0,
            score: 0.0,
            created_at: at,
            domain: domain.into(),
        }
    }

    fn ago(secs: f64) -> String {
        format!("{}m ago", (secs / 60.0) as i64)
    }

    #[test]
    fn what_520_showed_reads_as_statements() {
        let now = 10_000.0;
        let results = vec![
            r("a", "Security event [information_leak]: llm_response \u{2014} Response contained sensitive internal information", "episodic", "security", now - 240.0),
            r("b", "User lives in Bentonville, Arkansas", "semantic", "people", now - 600.0),
            r("c", "user lives in Bentonville,  Arkansas.", "semantic", "people", now - 6000.0),
            r("d", "terminal.run ok", "episodic", "audit/tools", now - 60.0),
            r("e", "Watched 14 processes", "episodic", "system/watch", now - 60.0),
            r("f", "User engages in WebGL debugging sessions", "semantic", "work", now - 1200.0),
        ];
        let rows = rows(&results, now, ago);
        let texts: Vec<&str> = rows.iter().map(|x| x.text.as_str()).collect();
        assert_eq!(
            texts,
            vec![
                "Safety check: response contained sensitive internal information.",
                "User lives in Bentonville, Arkansas",
                "User engages in WebGL debugging sessions",
            ],
            "the machine's records are off the list, the duplicate is one row"
        );
        assert_eq!(rows[0].origin, "A safety check \u{b7} saved 4m ago");
        assert_eq!(rows[1].origin, "Saved 2 times \u{b7} 10m ago");
        assert_eq!(rows[2].origin, "A fact \u{b7} saved 20m ago");
        assert!(rows.iter().all(|x| !x.origin.contains("semantic") && !x.origin.contains("episodic")));
    }

    #[test]
    fn nothing_saved_is_nothing_shown() {
        assert!(rows(&[], 0.0, ago).is_empty());
        assert!(rows(&[r("x", "   ", "semantic", "", 0.0)], 0.0, ago).is_empty());
    }
}
