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
//!
//! A new conversation ends that. On 14 October the person asked Yantrik Mind six things, switched
//! to Hermes, pressed "+ new conversation" and asked about tomorrow's calendar; Hermes was handed
//! the Mind's last four exchanges, a hotel confirmation number among them, and its memory plugin
//! kept the paragraph for good. Only what was said after the last new conversation is handed over.
//! A start of the shell is one too: the fence lives in memory, the turns on disk, so a restart
//! would otherwise hand over again what the person had already put behind them.

use super::model::{Agent, AgentId, Item, Turn};
use super::progress;
use super::store::Store;

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
/// last, nobody has spoken for [`STALE_SECS`], or nobody has spoken since the person last started
/// a new conversation.
pub fn for_turn(store: &Store, active: &str, now: u64) -> Option<Handover> {
    let agents = store.agents();
    let mine = AgentId::new(active, AgentId::MAIN);
    let my_last = agents.iter().find(|a| a.meta.id == mine).map(last_spoke).unwrap_or(0);
    let (previous, since) = agents
        .iter()
        .filter(|a| a.meta.id.conversation() == AgentId::MAIN && a.meta.id.harness() != active)
        .map(|a| (a, since_new(store, a)))
        .filter(|(_, since)| !since.is_empty())
        .max_by_key(|(a, _)| last_spoke(a))?;
    let when = last_spoke(previous);
    if when <= my_last || now.saturating_sub(when) > STALE_SECS {
        return None;
    }
    Some(Handover { from: previous.meta.mind.clone(), told: told(previous, since, now) })
}

/// The turns of `agent` since the person last started a new conversation.
fn since_new<'a>(store: &Store, agent: &'a Agent) -> &'a [Turn] {
    let fence = store.fence(&agent.meta.id);
    let first = agent.turns.iter().position(|t| t.n > fence).unwrap_or(agent.turns.len());
    &agent.turns[first..]
}

/// When anything was last said in an agent's conversation.
fn last_spoke(agent: &Agent) -> u64 {
    agent.turns.last().map(|t| t.ended.unwrap_or(agent.touched).max(t.started)).unwrap_or(0)
}

fn told(previous: &Agent, turns: &[Turn], now: u64) -> String {
    let mind = &previous.meta.mind;
    let mut out = format!(
        "[From the desktop: you are taking this conversation over from {mind}. The person expects \
         you to know what was said; it was:"
    );
    let skip = turns.len().saturating_sub(TURNS_HANDED);
    for turn in &turns[skip..] {
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
fn reply_of(turn: &Turn) -> String {
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

    /// The shell stops and starts again: everything saved to `dir` is read back by the store's own
    /// load path, on the same clock.
    fn restart(mut store: Store, dir: &std::path::Path, clock: Arc<AtomicU64>) -> Store {
        store.save(dir).unwrap();
        drop(store);
        Store::load(dir, Box::new(move || clock.load(Ordering::SeqCst)))
    }

    fn scratch_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "yantrik-handover-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
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

        let handed = for_turn(&store, "mind", 2_100).expect("Hermes spoke after Mind did");
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
        assert_eq!(for_turn(&store, "mind", 2_000), None, "Mind spoke last");
        assert!(for_turn(&store, "hermes", 2_000).is_some(), "Hermes has missed Mind's turn");
        assert_eq!(for_turn(&store, "hermes", 2_000 + STALE_SECS + 1), None, "too long ago");
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
        let handed = for_turn(&store, "hermes", 1_100).unwrap();
        assert!(!handed.told.contains("question 1\n") && !handed.told.contains("question 0"), "{}", handed.told);
        assert!(handed.told.contains("question 2") && handed.told.contains("question 5"), "{}", handed.told);
    }

    /// A switch of minds in the middle of a conversation hands it over, as #245 intends; the
    /// person's "+ new conversation" afterwards means the next word carries nothing from before.
    /// The 14 October leg: Mind, a switch to Hermes, a new conversation, a calendar question.
    #[test]
    fn a_new_conversation_after_a_switch_hands_nothing_over() {
        let clock = Arc::new(AtomicU64::new(1_000));
        let mut store = store_at(clock.clone());
        speak(&mut store, "mind", "book the hotel", "Booked: confirmation 77-QX.", false);
        clock.store(1_010, Ordering::SeqCst);
        speak(&mut store, "mind", "and the weather?", "Sunny, 24°C.", false);
        clock.store(1_020, Ordering::SeqCst);
        assert!(for_turn(&store, "hermes", 1_020).is_some(), "a switch with no new conversation hands over");

        store.new_conversation();
        assert_eq!(for_turn(&store, "hermes", 1_020), None, "a new conversation carries nothing");
        // In the same second as the press, and a minute later: the fence is not a clock.
        assert_eq!(for_turn(&store, "hermes", 1_080), None);
    }

    /// After a new conversation, only what was said since it is handed over, even by the mind
    /// that spoke before it.
    #[test]
    fn only_what_came_after_a_new_conversation_is_handed_over() {
        let clock = Arc::new(AtomicU64::new(1_000));
        let mut store = store_at(clock.clone());
        speak(&mut store, "mind", "book the hotel", "Booked: confirmation 77-QX.", false);
        store.new_conversation();
        clock.store(1_100, Ordering::SeqCst);
        speak(&mut store, "mind", "what's the time in Tokyo?", "Half past nine at night.", false);
        clock.store(1_200, Ordering::SeqCst);

        let handed = for_turn(&store, "hermes", 1_200).expect("Mind spoke after the new conversation");
        assert!(handed.told.contains("- The person: what's the time in Tokyo?"), "{}", handed.told);
        assert!(!handed.told.contains("hotel") && !handed.told.contains("77-QX"), "{}", handed.told);
    }

    /// The fence is kept in memory and the turns on disk, so a restart used to read it as 0:
    /// the person presses "+ new conversation", the shell restarts, and the next mind was handed
    /// the hotel booking again. A start of the shell is a new conversation.
    #[test]
    fn a_restart_hands_nothing_over_from_before_it() {
        let dir = scratch_dir("restart");
        let clock = Arc::new(AtomicU64::new(1_000));
        let mut store = store_at(clock.clone());
        speak(&mut store, "mind", "book the hotel", "Booked: confirmation 77-QX.", false);
        clock.store(1_010, Ordering::SeqCst);
        speak(&mut store, "mind", "and the weather?", "Sunny, 24°C.", false);
        clock.store(1_020, Ordering::SeqCst);

        let store = restart(store, &dir, clock.clone());
        assert_eq!(for_turn(&store, "hermes", 1_020), None, "nothing from before the restart");
        assert_eq!(for_turn(&store, "hermes", 1_080), None, "nor a minute later");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A turn still streaming when the shell stopped is saved open (`ended: null`) and closed by
    /// the load as lost, at the time of the start. That makes it the newest thing Mind "said", but
    /// it is behind the fence like every other turn from before the restart: none of it goes.
    #[test]
    fn a_turn_cut_off_by_the_restart_is_not_handed_over() {
        let dir = scratch_dir("restart-open");
        let clock = Arc::new(AtomicU64::new(1_000));
        let mut store = store_at(clock.clone());
        speak(&mut store, "mind", "book the hotel", "Booked: confirmation 77-QX.", false);
        clock.store(1_010, Ordering::SeqCst);
        speak(&mut store, "mind", "email the confirmation to Asha", "Sending 77-QX to asha@", true);
        store.save(&dir).unwrap();
        let saved = std::fs::read_to_string(crate::agents::store::file_for(&dir, &AgentId::new("mind", AgentId::MAIN))).unwrap();
        assert!(saved.contains("\"ended\":null"), "the turn is saved unfinished: {saved}");

        clock.store(1_100, Ordering::SeqCst);
        let store = restart(store, &dir, clock.clone());
        let mind = store.agent(&AgentId::new("mind", AgentId::MAIN)).unwrap();
        assert!(mind.turns.last().unwrap().lost, "the load closed it as cut off");
        assert_eq!(for_turn(&store, "hermes", 1_100), None, "the cut-off turn is not handed over");
        assert_eq!(for_turn(&store, "hermes", 1_200), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// After a restart, a switch of minds still hands over what was said since it, as #245
    /// intends, and only that.
    #[test]
    fn a_switch_after_a_restart_hands_over_only_what_came_after_it() {
        let dir = scratch_dir("restart-switch");
        let clock = Arc::new(AtomicU64::new(1_000));
        let mut store = store_at(clock.clone());
        speak(&mut store, "mind", "book the hotel", "Booked: confirmation 77-QX.", false);
        clock.store(1_010, Ordering::SeqCst);
        speak(&mut store, "mind", "email it to Asha", "Sending it", true);

        clock.store(1_100, Ordering::SeqCst);
        let mut store = restart(store, &dir, clock.clone());
        clock.store(1_200, Ordering::SeqCst);
        speak(&mut store, "mind", "what's the time in Tokyo?", "Half past nine at night.", false);
        clock.store(1_300, Ordering::SeqCst);

        let handed = for_turn(&store, "hermes", 1_300).expect("Mind spoke after the restart");
        assert!(handed.told.contains("- The person: what's the time in Tokyo?"), "{}", handed.told);
        assert!(handed.told.contains("  mind: Half past nine at night."), "{}", handed.told);
        for before in ["hotel", "77-QX", "Asha", "Sending it"] {
            assert!(!handed.told.contains(before), "{before} is from before the restart: {}", handed.told);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
