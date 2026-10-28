//! A prompt the person typed and did not send is never lost: a close, a quit and a logout keep it
//! in the framework's draft lane, without asking, and the next start puts it back and says so.

use serde_json::json;
use yantrik_app::app::{Event, ExitReason, WindowId};
use yantrik_app::drafts::path_in;
use yantrik_testing::TestApp;

use super::window::type_into;
use super::{fake, Fixture};
use crate::update::PROMPT_DRAFT;
use crate::{Studio, StudioApp};

const TYPED: &str = "a lighthouse nobody asked for yet";

fn kept(f: &Fixture) -> Option<String> {
    f.app.run_writes();
    std::fs::read_to_string(path_in(f.world.paths.state_dir(), PROMPT_DRAFT)).ok()
}

fn restart(f: &Fixture) -> Fixture {
    let app = TestApp::<Studio>::start_at(f.world.paths.clone(), Vec::new());
    let again = Fixture { app, world: f.world.clone() };
    again.until("the grade", |g| g.app.model(|m| m.graded));
    again
}

#[test]
fn closing_with_an_unsent_prompt_keeps_it_without_asking_and_the_next_start_brings_it_back() {
    let f = fake("draft-close");
    type_into(&f, "prompt", TYPED);
    f.app.close_requested(WindowId::PRIMARY);
    assert_eq!(f.app.exit_reason(), Some(ExitReason::LastWindowClosed), "the close asked something");
    assert_eq!(kept(&f).as_deref(), Some(TYPED));

    let again = restart(&f);
    assert_eq!(again.app.ui(|ui: &StudioApp| ui.get_prompt().to_string()), TYPED);
    assert!(again.notice().contains("unsent prompt"), "{}", again.notice());
    assert_eq!(again.app.ui(|ui: &StudioApp| ui.get_notice().to_string()), again.notice(), "the window says it");
}

#[test]
fn a_quit_and_a_logout_keep_it_too() {
    for how in ["quit", "signal", "display"] {
        let f = fake(&format!("draft-{how}"));
        type_into(&f, "prompt", TYPED);
        match how {
            "quit" => f.app.event(Event::ExitRequested(ExitReason::Person)),
            "signal" => f.app.event(Event::ExitRequested(ExitReason::Logout)),
            _ => f.app.display_gone(),
        }
        assert!(f.app.exit_reason().is_some(), "{how} did not end the app");
        assert_eq!(kept(&f).as_deref(), Some(TYPED), "{how} lost the person's text");
    }
}

#[test]
fn a_prompt_that_was_sent_or_cleared_is_not_kept() {
    let f = fake("draft-sent");
    type_into(&f, "prompt", TYPED);
    f.act("generate", json!({ "prompt": TYPED, "width": 64, "height": 64 })).unwrap();
    f.settled();
    f.app.run(crate::Command::Generate(None));
    f.settled();
    f.app.close_requested(WindowId::PRIMARY);
    assert_eq!(kept(&f), None, "a sentence that was already sent came back as unsent");

    let g = fake("draft-cleared");
    type_into(&g, "prompt", TYPED);
    type_into(&g, "prompt", "");
    g.app.close_requested(WindowId::PRIMARY);
    assert_eq!(kept(&g), None);
}

#[test]
fn typing_is_kept_as_it_goes_so_a_crash_does_not_lose_it() {
    let f = fake("draft-typing");
    type_into(&f, "prompt", TYPED);
    f.app.advance(yantrik_app::drafts::DEBOUNCE + std::time::Duration::from_millis(10));
    assert_eq!(kept(&f).as_deref(), Some(TYPED));
}

/// The keeping is asked for in the command that answers the event, at once: not left to the
/// debounce a keystroke started, which a close would otherwise have to hope was flushed.
#[test]
fn a_close_a_quit_and_a_logout_checkpoint_the_prompt_in_their_own_command() {
    use yantrik_app::effects::Request;
    let (_world, mut m) = super::bare("draft-command", r#"{"backend":{"kind":"fake"}}"#);
    m.form.prompt = TYPED.to_string();
    let checkpoints = |fx: &yantrik_app::effects::Effects<Studio>| {
        fx.requests()
            .iter()
            .filter(|r| matches!(r, Request::SaveDraft { name, bytes, delay }
                if name == PROMPT_DRAFT && bytes == TYPED.as_bytes() && delay.is_zero()))
            .count()
    };
    let fx = super::apply(&mut m, crate::Command::Closing(WindowId::PRIMARY));
    assert_eq!(checkpoints(&fx), 1);
    assert!(fx.requests().iter().any(|r| r.name() == "close"));
    let fx = super::apply(&mut m, crate::Command::Ending);
    assert_eq!(checkpoints(&fx), 1);
    assert!(fx.requests().iter().any(|r| r.name() == "exit"));
}
