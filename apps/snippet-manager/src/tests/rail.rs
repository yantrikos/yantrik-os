//! The agent rail, the header's AI and the pane's AI Explain: one question about the open
//! snippet, asked from a job, its answer on the card.

use std::time::Duration;

use yantrik_app::mind::fake::FakeMind;
use yantrik_app::mind::Reach;
use yantrik_app::watch::Seen;
use yantrik_app::{App, Paths};

use super::{open, start_at, Fixture};
use crate::{Command, SnippetManager};

fn ready() -> Fixture {
    let f = start_at(Paths::temporary(SnippetManager::ID), FakeMind::new().answering("It follows the log."), Vec::new());
    f.mind.set_reach(Reach::Ready);
    f.app.seen(Seen::Reach(Reach::Ready));
    f
}

#[test]
fn the_rail_offers_the_one_question_about_the_open_snippet() {
    let f = ready();
    let (journal, _) = f.seeded();
    f.app.run(Command::Select(-1));
    f.app.advance(Duration::from_millis(300));
    assert!(f.app.model(|m| m.rail.suggestions().is_empty()), "nothing open, nothing to ask about");
    f.app.run(Command::Select(journal));
    f.app.advance(Duration::from_millis(300));
    let (context, suggestions) = f.app.model(|m| (m.rail.context().to_vec(), m.rail.suggestions().to_vec()));
    assert_eq!(context[0].label, "Tail the journal");
    assert_eq!(context[0].detail, "Shell", "no measure of the code");
    assert_eq!(suggestions[0].id, "explain");
    f.app.ui(|ui| ui.invoke_agent_suggestion_activated("explain".into()));
    f.quiet();
    let prompt = f.mind.prompts()[0].clone();
    assert!(prompt.starts_with("Explain what this Shell snippet does, in at most four short lines."), "{prompt}");
    assert!(prompt.ends_with("journalctl -fu yantrik-ui"));
    let card = f.app.model(|m| m.rail.proposal().cloned().unwrap());
    assert_eq!(card.title, "About \u{201c}Tail the journal\u{201d}");
    assert_eq!(card.body, "It follows the log.");
    assert_eq!(card.verb, "Close");
    assert!(card.applies.is_none(), "an answer to read, nothing to apply");
    assert_eq!(f.app.ui(|ui| ui.get_proposal().body.to_string()), "It follows the log.");
}

/// The header's AI asks the same question through the same path, so the two cannot differ.
#[test]
fn the_ai_button_asks_the_rails_question() {
    let f = ready();
    let (journal, _) = f.seeded();
    f.app.run(Command::Select(-1));
    f.app.ui(|ui| ui.invoke_ai_explain_pressed());
    assert_eq!(f.notice(), "Open a snippet before asking about it.");
    assert!(f.mind.prompts().is_empty());
    f.app.run(Command::Select(journal));
    f.app.ui(|ui| ui.invoke_ai_explain_pressed());
    f.quiet();
    assert_eq!(f.mind.prompts().len(), 1);
    assert_eq!(f.app.model(|m| m.rail.proposal().map(|p| p.body.clone())).as_deref(), Some("It follows the log."));
}

/// With no Mind to answer, the button says why where it was pressed, and nothing is asked.
#[test]
fn with_no_mind_the_ai_button_says_why() {
    let f = open();
    let (journal, _) = f.seeded();
    f.app.run(Command::Select(journal));
    f.app.ui(|ui| ui.invoke_ai_explain_pressed());
    f.quiet();
    assert!(!f.notice().is_empty(), "the reach's sentence is said");
    assert!(f.mind.prompts().is_empty());
}

/// The rail's context row is a way back to the snippet: pressing it opens it, keeping the typing.
#[test]
fn the_context_row_opens_its_snippet() {
    let f = ready();
    let (journal, borrow) = f.seeded();
    f.app.run(Command::Select(journal));
    f.type_into("Kept by the row", "journalctl");
    f.app.ui(move |ui| ui.invoke_agent_context_activated(borrow.to_string().into()));
    f.quiet();
    assert_eq!(f.app.model(|m| m.selected), borrow);
    assert_eq!(f.on_disk().get(journal).unwrap().title, "Kept by the row");
}
