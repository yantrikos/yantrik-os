//! The top bar's "Mind at work" chip (#1347): which minds have a turn running, in their own names.
//!
//! Read from the agents store's desks (`agents_workroom::at_work`), the same reading the Agents
//! workroom draws, so the bar and Agents cannot disagree about who is working. Pressing the chip
//! brings Mind View forward and focuses it: pressing is the person's own act, which is what a
//! window the shell puts up unasked must not do.

use crate::agents;
use crate::App;
use slint::ComponentHandle;

/// The names of the minds at work, each once, in the order they started.
pub fn working_minds(s: &agents::Store) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for (agent, _) in super::agents_workroom::at_work(s).filter(|(a, _)| a.state.working()) {
        if !names.contains(&agent.meta.mind) {
            names.push(agent.meta.mind.clone());
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
    if ui.get_minds_working() != now.as_str() {
        ui.set_minds_working(now.into());
    }
}

/// Pressing the chip.
pub fn wire(ui: &App) {
    ui.on_minds_working_clicked(crate::mind_view_show::bring_forward);
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

    #[test]
    fn two_minds_at_work_are_both_named_once() {
        let mut s = Store::new();
        chat(&mut s, "hermes:main", "Hermes", "one");
        chat(&mut s, "companion:main", "Yantrik Mind", "two");
        chat(&mut s, "hermes:c-1", "Hermes", "three");
        assert_eq!(working_minds(&s).len(), 2);
    }
}
