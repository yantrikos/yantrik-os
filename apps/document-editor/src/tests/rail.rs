//! The agent layer: the rail's two suggestions, the AI panel's three controls and the card they
//! share. A summary is read and never applied; a rewrite is applied through `set_content`, as an
//! agent, and left unsaved.

use std::sync::Arc;

use serde_json::json;
use slint::Model as _;
use yantrik_app::app::Event;
use yantrik_app::mind::fake::FakeMind;
use yantrik_app::mind::{AskError, Reach, Recalled};
use yantrik_app::paths::Paths;
use yantrik_app::rail::RailCommand;
use yantrik_app::services::Services;
use yantrik_testing::TestApp;

use super::{act, modified, settle, text};
use crate::{Command, YDoc};

const DOC: &str = "# Plan\n\nWe ship on Friday. Marketing is ready.\n\n## Risks\nNone known.\n";

fn start(mind: FakeMind) -> (TestApp<YDoc>, Arc<FakeMind>) {
    let mind = Arc::new(mind);
    let app = TestApp::<YDoc>::start_at_with(Paths::temporary("documents"), Vec::new(), mind.clone(), Services::fakes());
    settle(&app);
    (app, mind)
}

fn ready(app: &TestApp<YDoc>) {
    app.event(Event::ReachChanged(Reach::Ready));
    settle(app);
}

fn card(app: &TestApp<YDoc>) -> (String, String, String) {
    app.ui(|ui| {
        let p = ui.get_proposal();
        (p.title.to_string(), p.verb.to_string(), p.impact.to_string())
    })
}

#[test]
fn the_rail_lists_the_headings_and_offers_two_suggestions_when_there_is_something_to_ask_about() {
    let (app, _) = start(FakeMind::new());
    ready(&app);
    assert!(app.ui(|ui| ui.get_agent_suggestions().row_count()) == 0, "nothing to ask about in an empty document");
    act(&app, "set_content", json!({ "text": DOC })).unwrap();
    app.advance(std::time::Duration::from_secs(1));
    settle(&app);
    let context: Vec<(String, String, String)> = app.ui(|ui| {
        ui.get_agent_context().iter().map(|c| (c.id.to_string(), c.label.to_string(), c.detail.to_string())).collect()
    });
    assert_eq!(
        context,
        vec![("heading:0".into(), "Plan".into(), "H1".into()), ("heading:48".into(), "Risks".into(), "H2".into())]
    );
    let suggestions: Vec<(String, String)> =
        app.ui(|ui| ui.get_agent_suggestions().iter().map(|s| (s.id.to_string(), s.label.to_string())).collect());
    assert_eq!(
        suggestions,
        vec![("summarize".into(), "Summarise it".into()), ("tighten".into(), "Make it shorter".into())]
    );
}

#[test]
fn a_summary_is_shown_beside_the_document_and_never_swaps_into_it() {
    let (app, mind) = start(FakeMind::new().answering("- ships Friday\n- marketing ready"));
    ready(&app);
    act(&app, "set_content", json!({ "text": DOC })).unwrap();
    app.run(Command::Ask("summarize".into()));
    settle(&app);
    let (title, verb, impact) = card(&app);
    assert_eq!((title.as_str(), verb.as_str(), impact.as_str()), ("Summary", "Close", "The document itself is left alone"));
    assert!(app.model(|m| m.rail.proposal().unwrap().applies.is_none()), "a summary leaves nothing to apply");
    let asked = mind.prompts()[0].clone();
    assert!(asked.contains("Reply with the summary only.") && !asked.contains("rewritten document"), "{asked}");
    assert!(asked.ends_with(DOC), "{asked}");
    assert_eq!(text(&app), DOC);
}

#[test]
fn a_rewrite_says_what_it_replaces_and_applies_through_set_content_unsaved() {
    let (app, mind) = start(FakeMind::new().answering("# Plan\n\nShip Friday.\n"));
    ready(&app);
    act(&app, "set_content", json!({ "text": DOC })).unwrap();
    app.run(Command::Window(crate::command::WindowAction::Cancel));
    app.run(Command::Rail(RailCommand::Activate("tighten".into())));
    settle(&app);
    let (title, verb, impact) = card(&app);
    assert_eq!((title.as_str(), verb.as_str()), ("Rewritten document", "Replace"));
    assert_eq!(impact, "Replaces the document with 4 words, unsaved");
    assert!(mind.prompts()[0].contains("Reply with the rewritten document only."));
    assert_eq!(text(&app), DOC, "nothing changes until Replace is pressed");
    app.run(Command::Rail(RailCommand::Apply));
    settle(&app);
    assert_eq!(text(&app), "# Plan\n\nShip Friday.\n");
    assert!(modified(&app), "generated text is looked at before it is kept");
    assert_eq!(card(&app).0, "", "the card went away");
}

#[test]
fn the_panels_own_words_are_asked_as_a_rewrite_of_this_document() {
    let (app, mind) = start(FakeMind::new().answering("Ship vendredi."));
    ready(&app);
    act(&app, "set_content", json!({ "text": "Ship Friday." })).unwrap();
    app.run(Command::Ask("ask:Translate it to French".into()));
    settle(&app);
    let asked = mind.prompts()[0].clone();
    assert!(asked.starts_with("Translate it to French\n\nHere is the document. Use only what it says; invent nothing."), "{asked}");
    assert_eq!(card(&app).1, "Replace");
}

#[test]
fn an_empty_document_and_a_mind_that_cannot_answer_are_said_on_the_card() {
    let (app, mind) = start(FakeMind::new());
    ready(&app);
    app.run(Command::Ask("summarize".into()));
    settle(&app);
    assert_eq!(card(&app).0, "Nothing to work on");
    assert!(mind.prompts().is_empty(), "nothing was sent");
    act(&app, "set_content", json!({ "text": DOC })).unwrap();
    app.event(Event::ReachChanged(Reach::NoShell));
    settle(&app);
    app.run(Command::Ask("tighten".into()));
    settle(&app);
    let (title, verb, _) = card(&app);
    assert_eq!((title.as_str(), verb.as_str()), ("The companion did not answer", "Close"));
    assert!(mind.prompts().is_empty());
}

#[test]
fn a_failed_ask_leaves_the_document_alone() {
    let (app, _) = start(FakeMind::new().answers([Err(AskError::Failed("model fell over".into()))]));
    ready(&app);
    act(&app, "set_content", json!({ "text": DOC })).unwrap();
    app.run(Command::Ask("tighten".into()));
    settle(&app);
    assert_eq!(card(&app).0, "The companion did not answer");
    assert_eq!(text(&app), DOC);
}

#[test]
fn what_the_mind_remembers_about_the_title_is_added_to_the_rail_once_the_title_stands_still() {
    let mind = FakeMind::new().remembering([Recalled { rid: "r1".into(), text: "Friday launch\nmore".into(), score: 0.9 }]);
    let (app, _) = start(mind);
    ready(&app);
    act(&app, "set_content", json!({ "text": DOC })).unwrap();
    app.advance(std::time::Duration::from_secs(1));
    settle(&app);
    app.advance(std::time::Duration::from_secs(1));
    let rows: Vec<(String, String, String)> = app.ui(|ui| {
        ui.get_agent_context().iter().map(|c| (c.id.to_string(), c.label.to_string(), c.detail.to_string())).collect()
    });
    assert!(rows.contains(&("memory:r1".into(), "Friday launch".into(), "90% match".into())), "{rows:?}");
}

#[test]
fn the_persons_replace_works_on_a_document_agents_are_not_shown_and_an_agent_still_is_refused() {
    let (app, _) = start(FakeMind::new().answering("# Plan

Ship Friday.
"));
    ready(&app);
    let path = super::scratch("rail-hidden").join("outside.md");
    super::opened(&app, &path, DOC);
    if !app.model(|m| m.doc.hidden) {
        return; // A temp folder inside the home is shown to agents and proves nothing here.
    }
    app.run(Command::Rail(RailCommand::Activate("tighten".into())));
    settle(&app);
    assert_eq!(card(&app).1, "Replace");
    app.run(Command::Rail(RailCommand::Apply));
    settle(&app);
    assert_eq!(text(&app), "# Plan

Ship Friday.
", "the person's press replaced it: {}", super::notice(&app));
    assert!(modified(&app));
    // Presenting the rail's own token over the socket is an agent's call, and is refused.
    let err = super::act_as_agent(&app, "rail", "set_content", json!({ "text": "x" })).unwrap_err();
    assert!(err.ends_with(crate::agent_rule::HIDDEN_REFUSAL), "{err}");
    assert_eq!(text(&app), "# Plan

Ship Friday.
");
}
