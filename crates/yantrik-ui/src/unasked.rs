//! What the shell marks as the companion's own: the calls it starts by itself, which a free
//! account whose allowance is tight keeps for what the person asks (`yantrik_ml::asked`).
//!
//! The mark is per thread. The startup brief is sent to the companion's worker as a message, so
//! the worker takes the mark when it recognizes the brief ([`brief_mark`]); what a message
//! says is never what makes a turn the person's, only what its sender states, so the mark needs
//! all three: not from the socket, not the person's, and the brief's own opening.

use yantrik_ml::asked::SelfStarted;

/// How the startup brief's prompt begins. `wire::morning_brief` writes it; [`brief_mark`] reads it.
pub(crate) const BRIEF_OPENING: &str = "You just started up.";

/// The mark for the worker's turn on `text`, when it is the startup brief: not asked by a program
/// on the socket (`outside`), not typed or spoken by the person (`person`), and the brief's words.
pub(crate) fn brief_mark(text: &str, outside: bool, person: bool) -> Option<SelfStarted> {
    (!outside && !person && text.starts_with(BRIEF_OPENING)).then(yantrik_ml::asked::self_started)
}

/// Check if an EXECUTE urge response is a "nothing found" message that should be suppressed.
pub(crate) fn is_nothing_response(text: &str) -> bool {
    let lower = text.to_lowercase();
    let patterns = [
        "no major news",
        "nothing significant",
        "nothing to share",
        "nothing to report",
        "nothing interesting",
        "nothing noteworthy",
        "no need for a full briefing",
        "no breaking news",
        "nothing earth-shattering",
        "nothing genuinely",
        "nothing stands out",
        "no trending",
        "nothing worth",
        "no developments",
    ];
    patterns.iter().any(|p| lower.contains(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use yantrik_ml::asked::started_by_itself;

    #[test]
    fn only_the_unattended_brief_wears_the_mark() {
        let brief = format!("{BRIEF_OPENING} It is 08:00. Compose the day's first brief.");
        {
            let _own = brief_mark(&brief, false, false).expect("the brief is the companion's own");
            assert!(started_by_itself());
        }
        assert!(!started_by_itself(), "the mark ends with the turn");
        assert!(brief_mark(&brief, true, false).is_none(), "a program on the socket asked: that is asked");
        assert!(brief_mark(&brief, false, true).is_none(), "the person typed it: that is asked");
        assert!(brief_mark("You just started up, said the person", false, false).is_none(), "not the brief's opening");
        assert!(brief_mark("What is 17 times 23?", false, false).is_none(), "an ordinary message from a button is asked");
    }

    #[test]
    fn the_companion_and_the_gateway_name_the_same_harness() {
        assert_eq!(crate::wire::harness::BUILTIN_ID, yantrik_gateway::asked::COMPANION);
    }

    /// `bridge.rs` is the shell's worker; its urge and task arms are what turn a turn's reply into a
    /// notification or a state. Source checks, as the rest of that file's tests.
    #[test]
    fn the_worker_delivers_no_failed_urge_and_marks_the_paths_that_start_by_themselves() {
        let src = include_str!("bridge.rs");
        let urge = &src[src.find("Executing research urge").unwrap()..];
        let urge = &urge[..urge.find("EXECUTE urge returned no actionable content").unwrap()];
        assert!(urge.contains("self_started()"), "the urge's turn wears the mark");
        assert!(urge.contains("resp.model_failure.is_none()"), "a turn that failed is not a proactive message");
        assert!(src.contains("skip_self_started(\"an urge from the think timer\")"), "no urge is started on a tight account");
        assert!(src.contains("next_task_among(&conn, yantrik_ml::asked::keeps_for_asked())"), "a tight account takes no task the companion queued");
        assert!(src.contains("task.by_itself().then(yantrik_ml::asked::self_started)"), "a task it queued wears the mark");
        assert!(src.contains("crate::unasked::brief_mark(&text, outside, person)"), "the startup brief wears the mark");
    }
}
