//! The mind panel's words: what each row says to a person, kept apart from where the numbers come
//! from (`mind_panel.rs`).
//!
//! The UI sweep of 17 October (design/ui-sweep-2026-10-17.md, root cause 1) found the panel on
//! every one of 44 screens saying `terminal.run failed 11d ago`, `YantrikDB · 1,768 memories`,
//! `6 minds · 4 keep memory` and `ceiling sensitive`. Each was true and none was plain, and because
//! the panel is on every screen, each was a finding on every screen. The rule this file keeps:
//! a row says what happened in words, the raw id goes in the row's tooltip, and a failure that is
//! more than a day old is no longer news. It stays in the audit view, which is the record.

use crate::wire::step_words;

/// A failure older than this is not shown in Recent actions. The audit file and the audit view
/// keep it; the panel is "what is going on", and an 11-day-old failure next to the clock read as
/// something broken right now.
pub const STALE_FAILURE_SECS: u64 = 86_400;

/// One act as a sentence: "Ran a command", "Changed an event in your calendar", "Rendered in
/// Blender"; a failure as what could not be done: "Couldn't run a command". The same words a
/// reply's timeline uses for the same call (`step_words`), so the two never disagree.
pub fn act_sentence(app: &str, action: &str, outcome: &str) -> String {
    let phrase = step_words::of_call("os_act", &format!("{}.{}", app.trim(), action.trim()), &serde_json::Value::Null, "");
    if outcome.trim() == "failed" {
        phrase.failed()
    } else {
        phrase.done()
    }
}

/// What the row's tooltip says: the raw id and the outcome exactly as the audit wrote them, for
/// whoever needs to look it up.
pub fn act_detail(app: &str, action: &str, outcome: &str) -> String {
    let outcome = if outcome.trim().is_empty() { "not reported" } else { outcome.trim() };
    format!("{}.{} \u{b7} {outcome}", app.trim(), action.trim())
}

/// Whether an act still belongs in the panel at `now`: everything that went through, and a
/// failure from the last day.
pub fn still_current(outcome: &str, unix: u64, now: u64) -> bool {
    outcome.trim() != "failed" || now.saturating_sub(unix) <= STALE_FAILURE_SECS
}

/// The memory row: "Remembers 1,768 things". The store's name is not something a person chose or
/// needs; how much is kept is.
pub fn memory_words(count: Option<i64>) -> String {
    match count {
        Some(1) => "Remembers 1 thing".into(),
        Some(n) => format!("Remembers {} things", thousands(n)),
        None => "Not counted yet".into(),
    }
}

/// The minds row: "6 can answer · 4 remember". One line in the panel's width. "Remember" is each
/// mind's own memory (`MindsSeen::keep_memory`): a harness's memory is its own unless it says
/// otherwise, and nothing here claims it shares yours.
pub fn minds_words(count: usize, keep_memory: usize) -> String {
    let answer = if count == 1 { "1 can answer".to_string() } else { format!("{count} can answer") };
    let keep = match keep_memory {
        0 => "none remember".to_string(),
        n => format!("{n} remember"),
    };
    format!("{answer} \u{b7} {keep}")
}

pub fn thousands(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 {
        format!("-{out}")
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_actions_are_sentences_not_ids() {
        assert_eq!(act_sentence("terminal", "run", "ok"), "Ran a command");
        assert_eq!(act_sentence("calendar", "update_event", "ok"), "Changed an event in your calendar");
        assert_eq!(act_sentence("blender", "render", "ok"), "Rendered in Blender");
        assert_eq!(act_sentence("terminal", "run", "failed"), "Couldn\u{2019}t run a command");
        for (app, action) in [("terminal", "run"), ("calendar", "update_event"), ("blender", "render"), ("arcade", "x_y_z_w")] {
            let said = act_sentence(app, action, "ok");
            assert!(!said.contains('.') && !said.contains('_'), "{app}.{action} is said as words, not as its id: {said}");
        }
        assert_eq!(act_detail("terminal", "run", "failed"), "terminal.run \u{b7} failed", "the id is kept, in the tooltip");
        assert_eq!(act_detail("files", "move", ""), "files.move \u{b7} not reported");
    }

    #[test]
    fn a_failure_older_than_a_day_leaves_the_panel_and_a_success_does_not() {
        let now = 10_000_000;
        assert!(still_current("failed", now - 3_600, now));
        assert!(!still_current("failed", now - 11 * 86_400, now), "terminal.run failed 11d ago is not news");
        assert!(still_current("ok", now - 11 * 86_400, now));
    }

    #[test]
    fn context_rows_are_plain() {
        assert_eq!(memory_words(Some(1_768)), "Remembers 1,768 things");
        assert_eq!(memory_words(Some(1)), "Remembers 1 thing");
        assert_eq!(memory_words(None), "Not counted yet", "never a zero that was not read");
        assert_eq!(minds_words(6, 4), "6 can answer \u{b7} 4 remember");
        assert_eq!(minds_words(1, 1), "1 can answer \u{b7} 1 remember");
        assert_eq!(minds_words(2, 0), "2 can answer \u{b7} none remember");
        for said in [memory_words(Some(5)), minds_words(3, 2)] {
            assert!(!said.contains("YantrikDB") && !said.contains("minds"), "{said}");
        }
    }

    #[test]
    fn the_panel_draws_no_ceiling_code_no_audit_jargon_and_no_cut_rows() {
        let slint = include_str!("../../yantrik-ui-slint/ui/components/mind_panel.slint");
        let code = slint.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
        for gone in ["\"ceiling \" +", "unasked", "mind audit\""] {
            assert!(!code.contains(gone), "the panel no longer draws {gone}");
        }
        assert!(code.contains("Asks before anything that matters"), "the mode is said as what it means");
        assert!(code.contains("text: act.detail"), "the raw id is kept, in the row's tooltip");
        let row = code.split("for act in MindPanelState.acts").nth(1).unwrap();
        let row = &row[..row.find("Machine").unwrap_or(row.len())];
        assert!(row.contains("text: act.what") && row.contains("wrap: word-wrap"), "a long sentence wraps rather than being cut");
    }
}
