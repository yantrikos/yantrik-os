//! Email drawn for a Mind (Mind View): `read_mind_view` reads the window as text, so the window
//! holds no sender, subject, preview, body or attachment name, in the list, the reader or the
//! thread. Nothing in it can quote the mail, show it in a browser or have the companion make
//! something of it, and a search matches nothing, since nothing it could match is shown.

use serde_json::json;
use slint::Model as _;
use yantrik_app::for_mind::{HIDDEN, READ_ONLY};
use yantrik_app::mind::fake::FakeMind;
use yantrik_app::{App as _, Paths};
use yantrik_testing::TestApp;

use super::{quiet, started};
use crate::backend::fake::{self, FakeMail};
use crate::command::{Ask, Command, Pick};
use crate::state::Draft;
use crate::Email;

const SENTINEL: &str = "SENTINEL-email-mind-view-61fa";

/// A mailbox whose one message carries the sentinel in every place the window would draw text.
fn mailbox() -> FakeMail {
    let s = SENTINEL;
    let listed = json!([{
        "id": "201", "from": format!("{s} Sender <{s}@example.com>"), "to": ["you@example.com"],
        "subject": format!("{s} subject"), "snippet": format!("{s} preview"), "date": "2026-10-20 10:35",
        "is_read": false, "is_starred": false, "has_attachments": true, "folder": "INBOX", "thread_id": null,
    }]);
    let opened = json!({
        "id": "201", "from": format!("{s} Sender <{s}@example.com>"), "to": [format!("{s}-to@example.com")],
        "cc": [format!("{s}-cc@example.com")], "bcc": [], "subject": format!("{s} subject"),
        "body_html": "", "body_text": format!("{s} body"), "date": "2026-10-20 10:35",
        "attachments": [{ "filename": format!("{s}.pdf"), "mime_type": "application/pdf", "size_bytes": 2048 }],
        "thread_messages": [{ "id": "t1", "from": format!("{s} Replier"), "date": "Mon", "snippet": format!("{s} earlier") }],
        "is_read": true, "is_starred": false,
    });
    FakeMail::ready()
        .answering("email.list_messages", listed.clone())
        .answering("email.search", listed)
        .answering("email.get_message", opened)
        .answering("email.mark_read", json!({}))
}

/// Started over `mail`, for a Mind or not, with the message opened.
fn with_message(mail: &FakeMail, for_mind: bool) -> TestApp<Email> {
    let app = if for_mind {
        let paths = Paths::temporary(Email::ID);
        fake::install(&paths, mail);
        TestApp::<Email>::start_for_mind(paths, Vec::new(), std::sync::Arc::new(FakeMind::new()))
    } else {
        started(mail)
    };
    quiet(&app);
    app.run(Command::Open(Pick::Key(0)));
    quiet(&app);
    app
}

/// Every string the window holds that a screen reader or OCR could see.
fn window_text(app: &TestApp<Email>) -> String {
    app.ui(|ui| {
        let d = ui.get_email_detail();
        let mut all: Vec<String> = [
            d.from_name, d.from_addr, d.from_initial, d.to_addr, d.cc_addr, d.subject, d.date_text, d.body,
            d.ai_summary, d.attachment_names,
        ]
        .map(|s| s.to_string())
        .into();
        let list = ui.get_email_list();
        for i in 0..list.row_count() {
            let r = list.row_data(i).unwrap();
            all.extend([r.from_name, r.from_addr, r.subject, r.preview, r.date_text, r.avatar_initial].map(|s| s.to_string()));
        }
        let files = ui.get_email_attachments();
        for i in 0..files.row_count() {
            let a = files.row_data(i).unwrap();
            all.extend([a.name, a.size_text, a.mime_type].map(|s| s.to_string()));
        }
        let thread = ui.get_email_thread_messages();
        for i in 0..thread.row_count() {
            let t = thread.row_data(i).unwrap();
            all.extend([t.from_name, t.from_addr, t.date_text, t.body].map(|s| s.to_string()));
        }
        all.extend([
            ui.get_notice(),
            ui.get_email_sync_status(),
            ui.get_email_draft_status(),
            ui.get_compose_to(),
            ui.get_compose_cc(),
            ui.get_compose_bcc(),
            ui.get_compose_subject(),
            ui.get_compose_body(),
            ui.get_email_search_query(),
        ].map(|s| s.to_string()));
        all.join("\n")
    })
}

#[test]
fn a_mind_view_window_holds_no_mail_text() {
    let app = with_message(&mailbox(), true);
    let text = window_text(&app);
    assert!(!text.contains(SENTINEL), "{text}");
    assert!(text.contains(HIDDEN));
    let detail = app.ui(|ui| ui.get_email_detail());
    assert_eq!((detail.body.as_str(), detail.subject.as_str()), (HIDDEN, HIDDEN));
    assert_eq!(app.ui(|ui| ui.get_email_attachments().row_count()), 1, "the attachment is still listed");
    assert_eq!(app.ui(|ui| ui.get_email_list().row_data(0).unwrap().preview.to_string()), HIDDEN);
}

#[test]
fn a_normal_window_still_shows_the_mail() {
    let app = with_message(&mailbox(), false);
    let text = window_text(&app);
    assert!(text.contains(SENTINEL));
}

#[test]
fn a_reply_a_forward_a_send_and_the_companion_carry_nothing_from_a_message_in_a_mind_view_window() {
    let mail = mailbox().answering("email.send_message", json!({}));
    let app = with_message(&mail, true);
    let draft = Draft { to: "a@example.com".into(), subject: "s".into(), body: "b".into(), ..Draft::default() };
    for command in [
        Command::Reply { all: true },
        Command::Forward,
        Command::Send(draft.clone()),
        Command::SaveDraft(draft),
        Command::OpenOriginal,
        Command::Ask(Ask::Summarize),
        Command::Ask(Ask::SuggestReply),
        Command::Ask(Ask::Classify),
        Command::Ask(Ask::Draft("write to them".into())),
        Command::Ask(Ask::Enhance("formal".into())),
        Command::Delete(0),
        Command::Archive(0),
        Command::AddAccount,
    ] {
        app.run(command);
        quiet(&app);
        assert_eq!(app.last_refusal().unwrap().sentence, READ_ONLY);
    }
    assert!(!app.ui(|ui| ui.get_is_composing()), "no composer was opened");
    assert!(mail.asked_for("email.send_message").is_empty(), "nothing went out");
    assert!(!window_text(&app).contains(SENTINEL));
}

/// A window started over a saved draft whose every field holds the sentinel, for a Mind or not.
fn with_draft_kept(mail: &FakeMail, for_mind: bool) -> (TestApp<Email>, std::path::PathBuf, Draft) {
    let s = SENTINEL;
    let draft = Draft {
        to: format!("{s}@x"),
        cc: format!("{s}-cc@x"),
        bcc: format!("{s}-bcc@x"),
        subject: format!("{s} subj"),
        body: format!("{s} body"),
    };
    let paths = Paths::temporary(Email::ID);
    fake::install(&paths, mail);
    let file = paths.data_dir().join(crate::files::DRAFT);
    crate::state::save_draft(&file, &draft).unwrap();
    let mind = std::sync::Arc::new(FakeMind::new());
    let app = if for_mind {
        TestApp::<Email>::start_for_mind(paths, Vec::new(), mind)
    } else {
        TestApp::<Email>::start_at(paths, Vec::new())
    };
    quiet(&app);
    (app, file, draft)
}

/// A Mind's window started over a saved draft whose every field holds the sentinel.
fn with_kept_draft(mail: &FakeMail) -> (TestApp<Email>, std::path::PathBuf, Draft) {
    with_draft_kept(mail, true)
}

#[test]
fn the_composer_in_a_mind_view_window_does_not_show_the_saved_draft() {
    let (app, file, draft) = with_kept_draft(&mailbox());
    let before = std::fs::read(&file).unwrap();
    app.ui(|ui| ui.invoke_compose_new());
    quiet(&app);
    assert!(!window_text(&app).contains(SENTINEL), "{}", window_text(&app));
    // The compose call fills the composer and the kept Cc and Bcc must not stay on screen.
    app.run(Command::Compose { to: "m@x".into(), subject: "hi".into(), body: "there".into() });
    quiet(&app);
    let text = window_text(&app);
    assert!(!text.contains(SENTINEL), "{text}");
    assert_eq!(app.ui(|ui| ui.get_compose_cc().to_string()), HIDDEN, "the kept Cc is there, masked");
    assert_eq!(app.ui(|ui| ui.get_compose_to().to_string()), HIDDEN, "the Mind's own text is masked too");
    // Closing the composer neither saves over the person's draft nor clears it.
    app.run(Command::CancelCompose(Draft::default()));
    quiet(&app);
    app.run(Command::CancelCompose(Draft { body: "mind text".into(), ..Draft::default() }));
    quiet(&app);
    assert_eq!(std::fs::read(&file).unwrap(), before, "the draft file is as it was");
    assert_eq!(crate::state::load_draft(&file).unwrap(), Some(draft));
}

#[test]
fn a_mind_view_window_does_not_write_the_person_s_draft_when_it_ends() {
    // The session ends with the composer open; then the person starts the app again.
    let mut at_restart = Vec::new();
    for for_mind in [false, true] {
        let (app, file, draft) = with_draft_kept(&mailbox(), for_mind);
        let before = std::fs::read(&file).unwrap();
        app.run(Command::Compose { to: "m@x".into(), subject: "hi".into(), body: "there".into() });
        quiet(&app);
        let paths = app.paths();
        app.event(yantrik_app::app::Event::ExitRequested(yantrik_app::app::ExitReason::Logout));
        let _ = app.shutdown();
        drop(app);
        if for_mind {
            assert_eq!(std::fs::read(&file).unwrap(), before, "the draft file is as it was");
        }
        let again = TestApp::<Email>::start_at(paths, Vec::new());
        quiet(&again);
        again.ui(|ui| ui.invoke_compose_new());
        at_restart.push((again.ui(|ui| ui.get_compose_body().to_string()), draft.body));
    }
    assert_eq!(at_restart[0].0, "there", "the check can see the lane: the person's composer is kept");
    assert_eq!(at_restart[1].0, at_restart[1].1, "a Mind's composer left nothing in the lane");
}

/// `compose` is an intent: in a Mind's instance it leaves the composer as it does anywhere, the
/// kept draft's Cc and Bcc included. Only the window's rendering of it is masked.
#[test]
fn compose_leaves_the_same_composer_in_a_mind_view_window_as_anywhere() {
    let mut got = Vec::new();
    for for_mind in [false, true] {
        let (app, _, _) = with_draft_kept(&mailbox(), for_mind);
        let answer = super::act(&app, "compose", json!({ "to": "m@x", "subject": "hi", "body": "there" })).unwrap();
        quiet(&app);
        got.push((answer, app.model(|m| m.composer.text.clone()), app.describe()));
    }
    assert_eq!(got[0], got[1]);
    assert!(got[0].1.cc.contains(SENTINEL), "the kept Cc is still on the draft");
}

#[test]
fn the_composer_s_own_edits_in_a_mind_view_window_are_refused() {
    let (app, _, _) = with_kept_draft(&mailbox());
    app.ui(|ui| ui.invoke_compose_new());
    quiet(&app);
    app.run(Command::Edited { text: Draft { body: HIDDEN.into(), ..Draft::default() }, query: String::new() });
    assert_eq!(app.last_refusal().unwrap().sentence, READ_ONLY);
}

/// The companion answers with the mail quoted, as the review's probe had it do.
#[test]
fn the_classify_button_in_a_mind_view_window_never_puts_the_companion_s_answer_on_the_window() {
    let paths = Paths::temporary(Email::ID);
    fake::install(&paths, &mailbox());
    let mind = std::sync::Arc::new(FakeMind::new().answering(&format!("It is about {SENTINEL} body, so action-needed")).reaching(yantrik_app::mind::Reach::Ready));
    let app = TestApp::<Email>::start_for_mind(paths, Vec::new(), mind.clone());
    quiet(&app);
    app.run(Command::Open(Pick::Key(0)));
    quiet(&app);
    app.ui(|ui| ui.invoke_ai_classify());
    quiet(&app);
    assert_eq!(app.last_refusal().unwrap().sentence, READ_ONLY);
    assert!(mind.prompts().is_empty(), "the message was not sent to the companion");
    assert!(!window_text(&app).contains(SENTINEL), "{}", window_text(&app));
}

#[test]
fn a_search_in_a_mind_view_window_matches_nothing_and_asks_the_service_nothing() {
    let mail = mailbox();
    let app = with_message(&mail, true);
    app.run(Command::Search(SENTINEL.into()));
    quiet(&app);
    assert_eq!(app.ui(|ui| ui.get_email_list().row_count()), 0);
    assert_eq!(app.ui(|ui| ui.get_email_search_count()), 0);
    assert!(mail.asked_for("email.search").is_empty(), "the service would match the text the window hides");
}

#[test]
fn masking_is_visual_only_describe_and_calls_are_as_for_a_normal_instance() {
    let mut got = Vec::new();
    for for_mind in [false, true] {
        let app = with_message(&mailbox(), for_mind);
        let d = app.describe();
        let search = super::act(&app, "search", json!({ "query": "subject" })).unwrap();
        got.push(json!([d["summary"], d["state"], search]));
    }
    assert!(got[0].to_string().contains(SENTINEL), "a Mind still reads mail through describe");
    assert_eq!(got[0], got[1]);
}
