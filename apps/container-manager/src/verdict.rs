//! What an action the Mind asked for answers, read from the listing its own command caused to be
//! re-read, so an action answers what the runtime shows rather than what a zero exit status
//! implied.
//!
//! The one that mattered most: `remove` is graded `dangerous`, and it used to answer
//! `{"removed": name}` without ever looking at whether the container was gone. A grade on an
//! action that fabricates its outcome approves nothing.

use serde_json::{json, Value};

use crate::model::{Model, Named, Verb};

/// The answer to a named action whose command exited zero: `Ok` with what the list shows now,
/// or `Err` with the sentence for a runtime that said yes and did not do it.
pub fn of(m: &Model, named: &Named) -> Result<Value, String> {
    let Named { verb, id, name } = named;
    let state = m.observed_state(id);
    match verb {
        // What the re-read list shows, not what the exit status implied: `docker start` succeeds
        // for a container whose process ends a moment later, and answering "started" without the
        // state would hide that it is not running.
        Verb::Start => Ok(json!({ "started": name, "status": state.unwrap_or_else(gone) })),
        Verb::Stop => match state {
            Some(status) if status == "running" => {
                Err(format!("\u{201C}{name}\u{201D} is still running after stop"))
            }
            Some(status) => Ok(json!({ "stopped": name, "status": status })),
            None => Ok(json!({ "stopped": name, "status": "gone" })),
        },
        Verb::Restart => Ok(json!({ "restarted": name, "status": state.unwrap_or_else(gone) })),
        Verb::Remove => match state {
            Some(_) => Err(format!("\u{201C}{name}\u{201D} is still listed after rm -f")),
            None => Ok(json!({ "removed": name })),
        },
        // The log's answer is its tail, read by the caller from the text it read.
        Verb::Logs => Ok(json!({ "container": name })),
    }
}

fn gone() -> String {
    "gone".to_string()
}

/// The last 40 lines of a log, as `show_logs` answers and the AI's question reads them.
pub fn last_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

/// The last 4000 characters of a log, as `describe` carries the open pane.
pub fn last_chars(text: &str, n: usize) -> String {
    let count = text.chars().count();
    text.chars().skip(count.saturating_sub(n)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tails_are_what_the_old_iterators_took() {
        let text = (1..=50).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
        let tail = last_lines(&text, 40);
        assert!(tail.starts_with("line 11\n") && tail.ends_with("line 50"));
        assert_eq!(last_lines("a\nb", 40), "a\nb");
        assert_eq!(last_lines("", 40), "");
        assert_eq!(last_chars("héllo", 3), "llo");
        assert_eq!(last_chars("hi", 4000), "hi");
    }
}
