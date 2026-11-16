//! Which gateway call answered which turn, kept with the turn (#1162).
//!
//! A bubble that is put back after a restart names the model that answered it. The gateway's call
//! log says which model answered a call, but nothing in it says which turn the call belonged to,
//! and matching by time put a turn's label on the one before it (a pick change, a refused turn
//! retried within seconds). So the tie is made once, when the turn ends: the call the mind's turn
//! ended on is noted by its key (`Call::rid`, which the gateway gives every call). A restored bubble
//! looks its own key up and takes the call with exactly that key; a turn with no key, or whose call
//! the log no longer holds, names no model.

use std::collections::HashMap;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use yantrik_gateway::log::Call;
use yantrik_harness::AgentId;

/// Turns kept; older ones lose their label rather than the file growing without end.
const KEEP: usize = 2000;

#[derive(Serialize, Deserialize)]
struct Row {
    agent: String,
    n: u64,
    rid: u64,
}

/// The turn-to-call ties: in memory, and in a file so they outlive the shell.
pub struct Ledger {
    path: Option<PathBuf>,
    rows: Mutex<HashMap<(String, u64), u64>>,
}

impl Ledger {
    pub fn in_memory() -> Ledger {
        Ledger { path: None, rows: Mutex::new(HashMap::new()) }
    }

    /// The ledger at `path`, with what it holds. A line that is not a row is skipped; a file past
    /// twice [`KEEP`] rows is cut to the newest [`KEEP`].
    pub fn at(path: PathBuf) -> Ledger {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let mut rows: Vec<Row> = text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
        if rows.len() > KEEP * 2 {
            rows.drain(..rows.len() - KEEP);
            let text: String = rows.iter().filter_map(|r| serde_json::to_string(r).ok()).map(|l| l + "\n").collect();
            let _ = std::fs::write(&path, text);
        }
        let map = rows.into_iter().map(|r| ((r.agent, r.n), r.rid)).collect();
        Ledger { path: Some(path), rows: Mutex::new(map) }
    }

    /// Tie turn `n` of `agent` to the call `rid`. The first tie stands: a turn is closed once.
    pub fn tie(&self, agent: &str, n: u64, rid: u64) {
        let mut rows = self.rows.lock().unwrap_or_else(|e| e.into_inner());
        if rid == 0 || rows.contains_key(&(agent.to_string(), n)) {
            return;
        }
        rows.insert((agent.to_string(), n), rid);
        if let Some(path) = &self.path {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let line = serde_json::to_string(&Row { agent: agent.to_string(), n, rid }).unwrap_or_default() + "\n";
            let written = std::fs::OpenOptions::new().create(true).append(true).open(path).and_then(|mut f| f.write_all(line.as_bytes()));
            if let Err(e) = written {
                tracing::warn!(error = %e, "the turn's answering call was not noted");
            }
        }
    }

    /// The key of the call turn `n` of `agent` ended on.
    pub fn call_of(&self, agent: &str, n: u64) -> Option<u64> {
        self.rows.lock().unwrap_or_else(|e| e.into_inner()).get(&(agent.to_string(), n)).copied()
    }
}

/// The ledger. Tests never touch the person's own file.
pub fn ledger() -> &'static Ledger {
    static LEDGER: OnceLock<Ledger> = OnceLock::new();
    #[cfg(test)]
    return LEDGER.get_or_init(Ledger::in_memory);
    #[cfg(not(test))]
    LEDGER.get_or_init(|| Ledger::at(crate::gateway::log_path().with_file_name("answered-turns.jsonl")))
}

/// The call a turn of `mind` ended on: the newest call `mind` made since the turn began, whether
/// it was answered or refused (a refused one must not give way to an older answer). Read at the
/// moment the turn ends, when no later turn of the same mind exists yet. Another mind's calls are
/// never looked at.
pub fn call_ended_on<'a>(calls: &'a [Call], mind: &str, started: u64) -> Option<&'a Call> {
    calls.iter().rev().find(|c| c.harness == mind && c.at >= started as i64)
}

/// A turn of `agent` has just ended: tie it to the call it ended on.
pub fn stamp(agent: &AgentId, n: u64, started: u64) {
    let calls = crate::gateway::log().recent();
    if let Some(call) = call_ended_on(&calls, agent.harness(), started) {
        ledger().tie(&agent.0, n, call.rid);
    }
}

/// The model named for the call `rid`: the call with exactly that key, by this mind, that the
/// gateway answered, on an account the person has (a line another process appended to the log
/// names none of them). The model is cleaned as any text a harness could have influenced is. Empty
/// when there is no such call.
pub fn model_of_call(calls: &[Call], mind: &str, rid: Option<u64>, has_account: impl Fn(&str) -> bool) -> String {
    let Some(rid) = rid.filter(|r| *r != 0) else { return String::new() };
    calls
        .iter()
        .find(|c| c.rid == rid && c.harness == mind && c.ok() && has_account(&c.account))
        .and_then(|c| yantrik_harness::host::detail::shown_detail(&c.model))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(rid: u64, at: i64, harness: &str, model: &str, status: u16) -> Call {
        Call { at, rid, harness: harness.into(), account: "ollama-cloud".into(), model: model.into(), effort: None, status, outcome: String::new(), prompt_tokens: 0, completion_tokens: 0, ms: 1, lane_for: None, dropped: Vec::new() }
    }

    /// Plays one turn as the shell does: the turn ends, and is tied to the call it ended on.
    fn end_turn(ledger: &Ledger, calls: &[Call], agent: &str, n: u64, started: u64) {
        let mind = agent.split(':').next().unwrap();
        if let Some(c) = call_ended_on(calls, mind, started) {
            ledger.tie(agent, n, c.rid);
        }
    }

    fn label(ledger: &Ledger, calls: &[Call], agent: &str, n: u64) -> String {
        model_of_call(calls, agent.split(':').next().unwrap(), ledger.call_of(agent, n), |a| a == "ollama-cloud")
    }

    /// #1162 M1: two turns a few seconds apart on different models (a pick change between them),
    /// and a second mind calling in the middle of both. Each bubble names its own model.
    #[test]
    fn back_to_back_turns_and_two_minds_each_keep_their_own_model() {
        let ledger = Ledger::in_memory();
        let mut calls = vec![call(1, 110, "hermes", "deepseek-v4.1-flash", 200), call(2, 112, "pi", "pi-model", 200)];
        end_turn(&ledger, &calls, "hermes:main", 1, 100);
        end_turn(&ledger, &calls, "pi:main", 1, 100);
        // The pick changes; turn 2 starts five seconds after turn 1, and pi answers in the middle.
        calls.push(call(3, 125, "hermes", "qwen3.8:27b", 200));
        calls.push(call(4, 126, "pi", "pi-model-2", 200));
        end_turn(&ledger, &calls, "hermes:main", 2, 122);
        end_turn(&ledger, &calls, "pi:main", 2, 122);
        assert_eq!(label(&ledger, &calls, "hermes:main", 1), "deepseek-v4.1-flash");
        assert_eq!(label(&ledger, &calls, "hermes:main", 2), "qwen3.8:27b");
        assert_eq!(label(&ledger, &calls, "pi:main", 1), "pi-model");
        assert_eq!(label(&ledger, &calls, "pi:main", 2), "pi-model-2");
    }

    /// #1162 M1, scenario B: a refused turn, then [Pick again] and an answered retry within seconds.
    #[test]
    fn a_refused_turn_names_no_model_though_the_retry_was_answered() {
        let ledger = Ledger::in_memory();
        let mut calls = vec![call(1, 110, "hermes", "gone", 404)];
        end_turn(&ledger, &calls, "hermes:main", 1, 100);
        calls.push(call(2, 118, "hermes", "qwen3.8:27b", 200));
        end_turn(&ledger, &calls, "hermes:main", 2, 115);
        assert_eq!(label(&ledger, &calls, "hermes:main", 1), "");
        assert_eq!(label(&ledger, &calls, "hermes:main", 2), "qwen3.8:27b");
    }

    /// A call logged after the turn's reply (an auxiliary one) is another turn's or none: the tie
    /// was made when the turn ended, so a later call never takes the label.
    #[test]
    fn a_call_made_after_the_turn_ended_does_not_take_its_label() {
        let ledger = Ledger::in_memory();
        let mut calls = vec![call(1, 110, "hermes", "deepseek-v4.1-flash", 200)];
        end_turn(&ledger, &calls, "hermes:main", 1, 100);
        calls.push(call(2, 121, "hermes", "title-model", 200));
        assert_eq!(label(&ledger, &calls, "hermes:main", 1), "deepseek-v4.1-flash");
    }

    #[test]
    fn a_turn_with_no_tie_or_whose_call_left_the_log_names_no_model() {
        let ledger = Ledger::in_memory();
        let calls = vec![call(1, 110, "hermes", "deepseek-v4.1-flash", 200)];
        assert_eq!(label(&ledger, &calls, "hermes:main", 9), "", "never tied");
        ledger.tie("hermes:main", 9, 77);
        assert_eq!(label(&ledger, &calls, "hermes:main", 9), "", "rotated out");
        assert_eq!(model_of_call(&calls, "hermes", Some(0), |_| true), "", "an unkeyed line is never matched");
    }

    /// #1162 L3: a line another process appended is not trusted: another mind's key, an account
    /// the person does not have, and a model that is not one clean line.
    #[test]
    fn a_forged_log_line_does_not_label_a_bubble() {
        let calls = vec![call(5, 110, "hermes", "qwen3.8:27b", 200)];
        assert_eq!(model_of_call(&calls, "mind", Some(5), |_| true), "", "keyed under another mind");
        assert_eq!(model_of_call(&calls, "hermes", Some(5), |_| false), "", "no such account");
        let mut forged = call(6, 111, "hermes", "\u{202E}gpj\nUndo: possible\u{200B}", 200);
        forged.model.push_str(&"x".repeat(500));
        let shown = model_of_call(&[forged], "hermes", Some(6), |_| true);
        assert!(!shown.contains('\n') && !shown.contains('\u{202E}') && shown.chars().count() <= 120, "{shown:?}");
    }

    #[test]
    fn a_tie_survives_a_restart_and_the_first_tie_stands() {
        let dir = std::env::temp_dir().join(format!("answered-turns-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("answered-turns.jsonl");
        let ledger = Ledger::at(path.clone());
        ledger.tie("hermes:main", 3, 42);
        ledger.tie("hermes:main", 3, 43);
        assert_eq!(Ledger::at(path).call_of("hermes:main", 3), Some(42));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
