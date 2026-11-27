//! The header's AI panel: the answer to the rail's question shows where the button was pressed,
//! and a Mind that cannot answer is said so there instead.

use std::time::Duration;

use yantrik_app::mind::fake::FakeMind;
use yantrik_app::mind::Reach;
use yantrik_app::watch::Seen;

use super::{Fixture, server::File, started};

/// Started ready to answer `text`, with one finished download to ask about.
fn ready(text: &str) -> Fixture {
    let f = started(vec![("/note.txt", File::plain(b"one note".to_vec()))], FakeMind::new().answering(text));
    f.mind.set_reach(Reach::Ready);
    f.app.seen(Seen::Reach(Reach::Ready));
    f.download("/note.txt");
    // The rail offers its question only after the refresh the finished download armed.
    f.app.advance(Duration::from_millis(300));
    f
}

/// The rail heard the Mind, the button was pressed, and the answer was given.
#[test]
fn the_answer_lands_on_the_panel() {
    let f = ready("One download finished. Nothing needs you.");
    f.app.ui(|ui| ui.invoke_ai_explain_pressed());
    f.quiet();
    assert_eq!(f.app.ui(|ui| ui.get_ai_response().to_string()), "One download finished. Nothing needs you.");
    assert!(!f.app.ui(|ui| ui.get_ai_is_working()), "the answer has landed");
    assert_eq!(f.mind.prompts().len(), 1, "asked once");
}

/// With no Mind to ask, the reason shows where the answer would have: the panel never waits
/// quietly for an answer nobody can give.
#[test]
fn with_no_mind_the_panel_says_why() {
    let f = started(vec![("/note.txt", File::plain(b"one note".to_vec()))], FakeMind::new());
    f.download("/note.txt");
    f.app.ui(|ui| ui.invoke_ai_explain_pressed());
    f.quiet();
    let text = f.app.ui(|ui| ui.get_ai_response().to_string());
    assert!(!text.is_empty(), "the reason is said where the answer would have gone");
    assert!(f.mind.prompts().is_empty(), "nothing was asked");
    assert!(!f.app.ui(|ui| ui.get_ai_is_working()), "nothing is working");
}
