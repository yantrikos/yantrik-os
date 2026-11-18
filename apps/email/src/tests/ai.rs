//! The companion's five actions: the prompts it is given, where each answer lands, and an
//! answer about a message that is no longer open, dropped.

use serde_json::json;
use yantrik_app::mind::fake::FakeMind;
use yantrik_app::mind::AskError;

use super::{act, notice, quiet, start_with, with_inbox};
use crate::command::{Ask, Command};

fn started_with(mind: FakeMind) -> (yantrik_testing::TestApp<crate::Email>, std::sync::Arc<FakeMind>) {
    let (app, mind) = start_with(&with_inbox(), mind);
    quiet(&app);
    (app, mind)
}

#[test]
fn the_summary_lands_on_the_message_it_is_about() {
    let (app, mind) = started_with(FakeMind::new().answering("Lunch is moved to one."));
    app.ui(|ui| ui.invoke_summarize_email());
    assert_eq!(notice(&app), "There is no message open to read.");
    act(&app, "open_message", json!({ "which": 1 })).unwrap();
    app.ui(|ui| ui.invoke_summarize_email());
    assert!(app.ui(|ui| ui.get_ai_is_working()));
    quiet(&app);
    assert!(!app.ui(|ui| ui.get_ai_is_working()));
    assert_eq!(app.ui(|ui| ui.get_email_detail().ai_summary), "Lunch is moved to one.");
    assert_eq!(
        mind.prompts()[0],
        "Summarise this email in at most three short lines. Say what is being asked of me, if anything. \
         Use only what it says.\n\nFrom: Priya R.\nSubject: Lunch on Friday\n\nThe text of Lunch on Friday.\nSecond line."
    );
}

#[test]
fn an_answer_about_a_message_no_longer_open_is_dropped() {
    let (app, _) = started_with(FakeMind::new().answering("About lunch."));
    act(&app, "open_message", json!({ "which": 1 })).unwrap();
    app.run(Command::Ask(Ask::Summarize));
    // An answer about another message than the one open is not drawn.
    app.run(Command::Answered {
        ask: Ask::Summarize,
        about: Some(("999".into(), "INBOX".into())),
        outcome: Ok("About something else.".into()),
    });
    assert_eq!(app.ui(|ui| ui.get_email_detail().ai_summary), "", "not drawn on the wrong message");
    quiet(&app);
    assert_eq!(app.ui(|ui| ui.get_email_detail().ai_summary), "About lunch.");
}

#[test]
fn a_suggested_reply_and_a_draft_go_into_the_composer_unsent() {
    let mind = FakeMind::new().answers([Ok("Noon works.".to_string()), Ok("Dear team, ...".to_string()), Ok("Shorter.".to_string())]);
    let (app, mind) = started_with(mind);
    act(&app, "open_message", json!({ "which": 1 })).unwrap();
    app.ui(|ui| ui.invoke_ai_reply_suggest());
    quiet(&app);
    assert!(app.ui(|ui| ui.get_is_composing()));
    assert_eq!(app.ui(|ui| ui.get_compose_subject()), "Re: Lunch on Friday");
    assert_eq!(app.ui(|ui| ui.get_compose_body()), "Noon works.");

    app.ui(|ui| ui.invoke_ai_draft("  ".into()));
    assert_eq!(notice(&app), "Say what the message should be about.");
    app.ui(|ui| ui.invoke_ai_draft("tell the team I am late".into()));
    quiet(&app);
    assert_eq!(app.ui(|ui| ui.get_compose_body()), "Dear team, ...");
    assert!(mind.prompts()[1].ends_with("no subject line and no commentary.\n\ntell the team I am late"));

    // Enhance names a tone; the body is the composer's, once.
    app.ui(|ui| ui.invoke_edited());
    app.ui(|ui| ui.invoke_enhance_text("shorter and more concise".into()));
    quiet(&app);
    assert_eq!(
        mind.prompts()[2],
        "Rewrite this email to be shorter and more concise. Keep every fact and every commitment. Reply with the body only.\n\nDear team, ..."
    );
    assert_eq!(app.ui(|ui| ui.get_compose_body()), "Shorter.");
}

#[test]
fn a_category_off_the_list_is_reported_and_a_companion_that_is_not_there_is_said() {
    let mind = FakeMind::new().answers([Ok("Meeting.".to_string()), Ok("urgent".to_string()), Err(AskError::NoModel)]);
    let (app, _) = started_with(mind);
    act(&app, "open_message", json!({ "which": 1 })).unwrap();
    app.ui(|ui| ui.invoke_ai_classify());
    quiet(&app);
    assert_eq!(app.ui(|ui| ui.get_email_ai_intent()), "meeting");
    app.ui(|ui| ui.invoke_ai_classify());
    quiet(&app);
    assert_eq!(app.ui(|ui| ui.get_email_ai_intent()), "");
    assert_eq!(
        notice(&app),
        "The companion answered \u{201c}urgent\u{201d}, which is not one of the four categories this screen knows."
    );
    app.ui(|ui| ui.invoke_ai_classify());
    quiet(&app);
    assert_eq!(notice(&app), AskError::NoModel.to_string());
    assert!(!app.ui(|ui| ui.get_ai_is_working()));
}
