//! The conversation, handed to the mind that answers next (#245).
//!
//! Switching the answering mind lost the conversation. On 23 September Hermes was building a town
//! model when the Lens changed hands to Yantrik Mind, which had never heard of it and answered
//! "Can you please continue with the town model" by asking about the person's wedding anniversary.
//! Pranab's reframe: "The switch should be seamless. Even if we switch harness, memory is the same."
//! Long-term memory is shared at best; the conversation in progress and the work in flight never
//! were. The shell holds both: every mind's Lens conversation is its `<harness>:main` agent in
//! this store. So on the first word to a mind after another one spoke, the desktop tells it what
//! was said and what is still running.

use super::model::{Agent, AgentId, Item};
use super::progress;

/// How many of the previous mind's exchanges are handed over.
pub const TURNS_HANDED: usize = 4;
/// A conversation quiet for longer than this is over; nothing is handed from it.
pub const STALE_SECS: u64 = 6 * 3600;

/// What the desktop tells a mind taking a conversation over.
#[derive(Clone, Debug, PartialEq)]
pub struct Handover {
    /// The mind the conversation comes from, as a person reads its name.
    pub from: String,
    /// The bracketed paragraph put in front of the person's words for the new mind.
    pub told: String,
}

/// The handover for a word about to go to `active`'s conversation, or None when `active` spoke
/// last (or nobody has spoken for [`STALE_SECS`]).
pub fn for_turn(agents: &[Agent], active: &str, now: u64) -> Option<Handover> {
    let mine = AgentId::new(active, AgentId::MAIN);
    let my_last = agents.iter().find(|a| a.meta.id == mine).map(last_spoke).unwrap_or(0);
    let previous = agents
        .iter()
        .filter(|a| a.meta.id.conversation() == AgentId::MAIN && a.meta.id.harness() != active)
        .filter(|a| !a.turns.is_empty())
        .max_by_key(|a| last_spoke(a))?;
    let when = last_spoke(previous);
    if when <= my_last || now.saturating_sub(when) > STALE_SECS {
        return None;
    }
    Some(Handover { from: previous.meta.mind.clone(), told: told(previous, now) })
}

/// When anything was last said in an agent's conversation.
fn last_spoke(agent: &Agent) -> u64 {
    agent.turns.last().map(|t| t.ended.unwrap_or(agent.touched).max(t.started)).unwrap_or(0)
}

fn told(previous: &Agent, now: u64) -> String {
    let mind = &previous.meta.mind;
    let mut out = format!(
        "[From the desktop: you are taking this conversation over from {mind}. The person expects \
         you to know what was said; it was:"
    );
    let skip = previous.turns.len().saturating_sub(TURNS_HANDED);
    for turn in &previous.turns[skip..] {
        out.push_str(&format!("\n- The person: {}", progress::brief(&turn.prompt, 300)));
        let said = reply_of(turn);
        if !said.is_empty() {
            out.push_str(&format!("\n  {mind}: {}", progress::brief(&said, 400)));
        }
    }
    if let Some(working) = progress::of(previous, now) {
        out.push_str(&format!(
            "\n{mind} is still at work on the last of these. The desktop's own account: {}",
            working.told(mind).replace('\n', " ")
        ));
    }
    out.push_str("\nCarry on from here.]");
    out
}

/// What the mind said in a turn: its text, without the calls.
fn reply_of(turn: &super::model::Turn) -> String {
    turn.items
        .iter()
        .filter_map(|i| match i {
            Item::Text(t) => Some(t.text()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
        .trim()
        .to_string()
}

/// The person's words with the handover in front, for the mind only.
pub fn with_handover(handover: &Handover, text: &str) -> String {
    format!("{}\n\n{text}", handover.told)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::store::Store;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    /// A store whose clock the test moves.
    fn store_at(clock: Arc<AtomicU64>) -> Store {
        Store::with_clock(Box::new(move || clock.load(Ordering::SeqCst)))
    }

    fn speak(store: &mut Store, harness: &str, prompt: &str, reply: &str, open: bool) {
        let id = AgentId::new(harness, AgentId::MAIN);
        store.open_turn(&id, prompt);
        store.text(&id, reply);
        if !open {
            store.close_turn(&id, true);
        }
    }

    /// The 23 September switch: Hermes is building the town model when Yantrik Mind takes the
    /// Lens. Mind is told what was asked, what Hermes said, and that the build is still going.
    #[test]
    fn the_next_mind_is_told_the_conversation_and_the_work_in_flight() {
        let clock = Arc::new(AtomicU64::new(1_000));
        let mut store = store_at(clock.clone());
        speak(&mut store, "mind", "good morning", "Morning! What's on today?", false);
        clock.store(2_000, Ordering::SeqCst);
        speak(&mut store, "hermes", "build a small town model: homes, roads, cars", "Starting with the roads.", true);
        clock.store(2_100, Ordering::SeqCst);

        let handed = for_turn(store.agents(), "mind", 2_100).expect("Hermes spoke after Mind did");
        assert_eq!(handed.from, "hermes");
        assert!(handed.told.starts_with("[From the desktop: you are taking this conversation over from hermes."), "{}", handed.told);
        assert!(handed.told.contains("- The person: build a small town model: homes, roads, cars"), "{}", handed.told);
        assert!(handed.told.contains("  hermes: Starting with the roads."), "{}", handed.told);
        assert!(handed.told.contains("hermes is still at work on the last of these."), "{}", handed.told);
        assert!(handed.told.ends_with("Carry on from here.]"), "{}", handed.told);
        assert_eq!(
            with_handover(&handed, "can you continue the town model?"),
            format!("{}\n\ncan you continue the town model?", handed.told)
        );
    }

    /// Nothing is handed when the mind being spoken to said the last word itself, or when the
    /// other conversation is long over.
    #[test]
    fn nothing_is_handed_to_the_mind_that_spoke_last_or_from_a_stale_conversation() {
        let clock = Arc::new(AtomicU64::new(1_000));
        let mut store = store_at(clock.clone());
        speak(&mut store, "hermes", "tidy my notes", "Done.", false);
        clock.store(2_000, Ordering::SeqCst);
        speak(&mut store, "mind", "thanks", "Any time.", false);
        assert_eq!(for_turn(store.agents(), "mind", 2_000), None, "Mind spoke last");
        assert!(for_turn(store.agents(), "hermes", 2_000).is_some(), "Hermes has missed Mind's turn");
        assert_eq!(for_turn(store.agents(), "hermes", 2_000 + STALE_SECS + 1), None, "too long ago");
    }

    /// Only the last few exchanges go, oldest first.
    #[test]
    fn only_the_last_exchanges_are_handed_over() {
        let clock = Arc::new(AtomicU64::new(1_000));
        let mut store = store_at(clock.clone());
        for n in 0..(TURNS_HANDED + 2) {
            clock.store(1_000 + n as u64, Ordering::SeqCst);
            speak(&mut store, "pi", &format!("question {n}"), &format!("answer {n}"), false);
        }
        let handed = for_turn(store.agents(), "hermes", 1_100).unwrap();
        assert!(!handed.told.contains("question 1\n") && !handed.told.contains("question 0"), "{}", handed.told);
        assert!(handed.told.contains("question 2") && handed.told.contains("question 5"), "{}", handed.told);
    }
}
