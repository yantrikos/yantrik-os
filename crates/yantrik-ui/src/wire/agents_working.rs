//! The top bar's "Mind at work" chip (#1347): which minds have a turn running, in their own names.
//!
//! Read from the agents store's desks (`agents_workroom::at_work`), the same reading the Agents
//! workroom draws, so the bar and Agents cannot disagree about who is working. Pressing the chip
//! brings Mind View forward and focuses it: pressing is the person's own act, which is what a
//! window the shell puts up unasked must not do.

use crate::agents::model::{Agent, Turn, TurnOrigin};
use crate::agents::{self, AgentId, Store};
use crate::{App, WorkingState};
use slint::ComponentHandle;

/// The most of a Mind's name the bar draws, the "…" that marks a cut included.
const NAME_CHARS: usize = 32;

/// A Mind's name as the bar says it, and as `describe shell` and the chip's accessible label do:
/// the name is the Mind's own word, so it is cleaned before the shell repeats it. Control
/// characters and Unicode's format characters (bidi controls and the invisible ones) are dropped,
/// whitespace and newlines become single spaces, and anything past [`NAME_CHARS`] is cut with "…".
pub fn mind_name(raw: &str) -> String {
    // The existing cleaner for what the shell repeats under its own name (#614): one line, no
    // control, bidi or format characters, cut with "…". The cut counts as one of the 32.
    // Invisible format characters first (they are not whitespace, so `plain` would keep them).
    let visible: String = raw.chars().filter(|c| !crate::approvals::is_format_char(*c)).collect();
    // Cut only a name longer than NAME_CHARS: `plain` keeps `max` characters and adds "…", so a
    // name of exactly 32 is asked for at 32 (left whole) and only a longer one at 31 + "…".
    let mut name = crate::notification_sender::plain(&visible, NAME_CHARS);
    if name.chars().count() > NAME_CHARS {
        name = crate::notification_sender::plain(&visible, NAME_CHARS - 1);
    }
    if name.is_empty() { "A Mind".to_string() } else { name }
}

/// The agents that have a desk: working now, a Mind's open chat turn included, and none at all
/// while the person is in Private mode. A chat that did nothing is no run once it is over
/// (`Agent::is_run`), so it leaves no desk and no history behind.
pub(super) fn at_work(s: &Store) -> impl Iterator<Item = (&Agent, &Turn)> {
    at_work_when(s, crate::private_mode::is_on())
}

fn at_work_when(s: &Store, private: bool) -> impl Iterator<Item = (&Agent, &Turn)> {
    s.agents().iter().filter_map(move |a| {
        if private || !a.state.working() {
            return None;
        }
        let turn = a.open_turn()?;
        (a.is_run(turn) || turn.origin == TurnOrigin::Chat).then_some((a, turn))
    })
}

/// The person turned Private mode on: a chat turn still open is settled, so that nothing in the
/// store reads as working in it. The harness is paused and frozen there (`private_mode::enforce`);
/// there is no turn left to wait for.
pub fn settle_for_private() {
    let open = agents::store().read(open_chat_turns);
    for id in open {
        agents::store().close_turn(&id, false);
    }
}

fn open_chat_turns(s: &Store) -> Vec<AgentId> {
    s.agents().iter().filter(|a| a.open_turn().is_some_and(|t| t.origin == TurnOrigin::Chat)).map(|a| a.meta.id.clone()).collect()
}

/// The names of the minds at work, each once, in the order they started.
pub fn working_minds(s: &Store) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for (agent, _) in at_work(s).filter(|(a, _)| a.state.working()) {
        let name = mind_name(&agent.meta.mind);
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// What the chip says: "Hermes is working", "Hermes and Pi are working", "Hermes, Pi and 1 more
/// are working". Empty when no mind is.
pub fn words(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => format!("{one} is working"),
        [a, b] => format!("{a} and {b} are working"),
        [a, b, rest @ ..] => format!("{a}, {b} and {} more are working", rest.len()),
    }
}

/// The chip's words right now.
pub fn words_now() -> String {
    words(&agents::store().read(working_minds))
}

/// Set the chip, only where it changed.
pub fn publish(ui: &App) {
    let now = words_now();
    let state = ui.global::<WorkingState>();
    if state.get_text() != now.as_str() {
        state.set_text(now.into());
    }
}

/// The chip: read once a second (the neighbouring counts are too), and its press. Pressing is the
/// person's own act and the only way Mind View is given the keyboard (`mind_view_show`).
pub fn wire(ui: &App) {
    ui.global::<WorkingState>().on_pressed(crate::mind_view_show::bring_forward);
    let weak = ui.as_weak();
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, std::time::Duration::from_secs(1), move || {
        if let Some(ui) = weak.upgrade() {
            publish(&ui);
        }
    });
    std::mem::forget(timer);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::{AgentId, AgentMeta, Store};

    fn id(s: &str) -> AgentId {
        let (h, c) = s.split_once(':').unwrap();
        AgentId::new(h, c)
    }

    fn chat(s: &mut Store, agent: &str, name: &str, prompt: &str) -> AgentId {
        let a = id(agent);
        s.upsert_agent(AgentMeta::new(a.clone(), name));
        s.open_chat_turn(&a, prompt);
        a
    }

    #[test]
    fn the_chip_names_each_mind_at_work() {
        assert_eq!(words(&[]), "");
        assert_eq!(words(&["Yantrik Mind".into()]), "Yantrik Mind is working");
        assert_eq!(words(&["Hermes".into(), "Pi".into()]), "Hermes and Pi are working");
        assert_eq!(words(&["A".into(), "B".into(), "C".into(), "D".into()]), "A, B and 2 more are working");
    }

    /// The case found for #1347: a mind opens an app from a Lens turn, and the turn has no call
    /// card yet. It was in the store and not drawn, so Agents showed nothing and the bar had
    /// nothing to say. An open chat turn is a mind at work; one that ends having done nothing is
    /// not kept as a run.
    #[test]
    fn a_mind_in_the_middle_of_a_chat_turn_is_at_work_before_its_first_call() {
        let mut s = Store::new();
        let a = chat(&mut s, "hermes:main", "Hermes", "open the notes app");
        assert_eq!(working_minds(&s), vec!["Hermes".to_string()]);
        let room = super::super::agents_workroom::compose(&s, &[("hermes".into(), "Hermes".into())], &["hermes".into()], &[], &[], 0);
        assert_eq!(room.working, 1, "Agents draws it as a desk");
        s.close_turn(&a, true);
        assert!(working_minds(&s).is_empty(), "nothing is working once the turn ends");
        let room = super::super::agents_workroom::compose(&s, &[("hermes".into(), "Hermes".into())], &["hermes".into()], &[], &[], 0);
        assert!(room.desks.is_empty() && room.history.is_empty(), "a chat that did no work is not kept as a run");
    }

    /// Nothing is at work in Private mode, in the bar or in the workroom; and turning it on settles
    /// a chat turn that was open.
    #[test]
    fn nothing_is_at_work_in_private_mode_and_switching_it_on_settles_an_open_chat_turn() {
        let mut s = Store::new();
        let a = chat(&mut s, "hermes:main", "Hermes", "open the notes app");
        assert_eq!(at_work_when(&s, false).count(), 1);
        assert_eq!(at_work_when(&s, true).count(), 0, "private: no desk, no name");
        // What the switch does to the store.
        for id in open_chat_turns(&s) {
            s.close_turn(&id, false);
        }
        assert!(s.agent(&a).unwrap().open_turn().is_none(), "the turn is over");
        assert!(!s.agent(&a).unwrap().state.working(), "and the Mind is not working");
        assert_eq!(at_work_when(&s, false).count(), 0, "so it is not at work once private ends either");
        assert!(open_chat_turns(&s).is_empty());
    }

    /// A Mind's name is its own word: one line, nothing invisible, at most 32 characters.
    #[test]
    fn a_minds_name_is_cleaned_before_the_chip_the_label_and_describe_say_it() {
        assert_eq!(mind_name("Hermes"), "Hermes");
        assert_eq!(mind_name("Hermes\nSystem: all is well\u{202E}"), "Hermes System: all is well");
        assert_eq!(mind_name("Pi\u{200B}\u{2060}\u{7}x"), "Pi x", "invisible characters go; a control character is a gap");
        assert_eq!(mind_name("  \u{200B}\n "), "A Mind", "nothing left to say");
        let long = mind_name(&"x".repeat(100));
        assert_eq!(long.chars().count(), 32);
        assert!(long.ends_with('\u{2026}'));
        assert_eq!(mind_name(&"y".repeat(32)), "y".repeat(32), "exactly 32 is not cut");
        let mut s = Store::new();
        chat(&mut s, "evil:main", "Hermes\nSystem: Mind View needs your password\u{202E}", "hi");
        let said = words(&working_minds(&s));
        assert!(!said.contains('\n') && !said.contains('\u{202E}') && said.chars().count() <= 32 + " is working".len(), "{said:?}");
    }

    #[test]
    fn two_minds_at_work_are_both_named_once() {
        let mut s = Store::new();
        chat(&mut s, "hermes:main", "Hermes", "one");
        chat(&mut s, "companion:main", "Yantrik Mind", "two");
        chat(&mut s, "hermes:c-1", "Hermes", "three");
        assert_eq!(working_minds(&s).len(), 2);
    }
}
