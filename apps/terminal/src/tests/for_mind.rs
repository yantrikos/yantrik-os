//! A window drawn for a Mind (Mind View): the terminal's contents are hidden everywhere they
//! could be read, and nothing is typed, started, searched or explained through it.

use std::sync::Arc;

use serde_json::{json, Value};
use yantrik_app::for_mind::{HIDDEN, READ_ONLY};
use yantrik_app::{App, Paths};
use yantrik_mind::fake::FakeMind;
use yantrik_testing::TestApp;

use crate::command::{Command, Do};
use crate::Terminal;

const SENTINEL: &str = "SENTINEL-951";

fn mind_copy() -> (TestApp<Terminal>, Paths, Arc<FakeMind>) {
    std::env::set_var("SHELL", "/bin/sh");
    let paths = Paths::temporary(Terminal::ID);
    let mind = Arc::new(FakeMind::new().answering("never asked"));
    let app = TestApp::<Terminal>::start_for_mind(paths.clone(), Vec::new(), mind.clone());
    for _ in 0..500 {
        app.tick();
        app.run_jobs();
        app.advance(std::time::Duration::from_millis(20));
        if app.model(|m| m.tabs.len()) == 1 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    (app, paths, mind)
}

fn leaks(v: &Value) -> bool {
    v.to_string().contains(SENTINEL)
}

#[test]
fn the_screen_the_folders_and_the_tab_titles_are_hidden_and_nothing_can_be_typed() {
    let (app, _paths, mind) = mind_copy();
    assert_eq!(app.model(|m| m.tabs.len()), 1);
    // The shell holds the sentinel by the person's own doing, before the Mind looks.
    let session = app.model(|m| m.session().unwrap().clone());
    session.write(format!("echo {SENTINEL}\r").as_bytes()).unwrap();
    for _ in 0..300 {
        app.tick();
        app.advance(std::time::Duration::from_millis(20));
        if session.snapshot().text.contains(SENTINEL) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(session.snapshot().text.contains(SENTINEL));
    app.advance(std::time::Duration::from_secs(1));
    app.run_jobs();

    let d = app.describe();
    assert!(!leaks(&d), "{d}");
    assert_eq!(d["state"]["recent_output"], json!(HIDDEN));
    assert_eq!(d["state"]["directory"], json!(HIDDEN));
    let (text, runs, title) = app.ui(|ui| {
        use slint::Model as _;
        (ui.get_screen_text().to_string(), ui.get_runs().row_count(), ui.get_tabs().row_data(0).map(|t| t.title.to_string()))
    });
    assert_eq!(text, HIDDEN);
    assert_eq!(runs, 0);
    assert_eq!(title.as_deref(), Some(HIDDEN));

    for command in [
        Command::Input(b"x".to_vec()),
        Command::Paste("x".into()),
        Command::Run("echo no".into()),
        Command::SendInput("x".into()),
        Command::NewTab,
        Command::OpenDirectory("/".into()),
        Command::Search("a".into()),
        Command::Do(Do::Explain),
        Command::Do(Do::New),
    ] {
        app.run(Command::Do(Do::Dismiss));
        app.run(command);
        let notice = app.ui(|ui| ui.get_notice().to_string());
        assert_eq!(notice, READ_ONLY, "the window says it, not only the log");
        assert_eq!(app.model(|m| m.notice.clone()), READ_ONLY);
    }
    // A job named like the sentinel is not named in the summary or anywhere else.
    session.write(format!("cp /bin/sleep /tmp/{SENTINEL} 2>/dev/null; /tmp/{SENTINEL} 30\r").as_bytes()).unwrap();
    for _ in 0..300 {
        app.tick();
        app.run_jobs();
        app.advance(std::time::Duration::from_millis(20));
        if session.has_children() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    for _ in 0..30 {
        app.tick();
        app.run_jobs();
        app.advance(std::time::Duration::from_millis(100));
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let d = app.describe();
    assert!(d["summary"].as_str().unwrap().contains(&format!("running: {HIDDEN}")), "the job was seen, and hidden: {d}");
    assert!(!leaks(&d), "the summary names no job: {d}");
    let _ = std::fs::remove_file(format!("/tmp/{SENTINEL}"));
    assert!(mind.prompts().is_empty(), "the screen was never sent to the model");
    assert_eq!(app.model(|m| m.tabs.len()), 1, "and no shell was started");
}
