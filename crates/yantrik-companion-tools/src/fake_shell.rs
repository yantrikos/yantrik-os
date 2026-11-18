//! A shell on a socket, for tests of everything that asks the person (#1311): the fake desk
//! `outward.rs`'s tests use, in one place so the tools' tests (in this crate and in
//! `yantrik-companion`, through the `test-support` feature) talk to the same one.
//!
//! It answers the four calls the asking makes (`request_approval`, `approval_status`,
//! `consume_approval`, `record_unasked_action`) from a script and keeps a log of every call. It
//! decides nothing the real shell decides: it is told what the person does.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::app_ui::Desk;

/// What the shell says to the first call, `request_approval`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum First {
    /// A card is up, awaiting the person.
    Pending,
    /// The mode runs it with no card (full bypass).
    NotNeeded,
    /// A card is up but the reply names no request.
    NoId,
    /// The shell would not put it up: its card cannot show it whole.
    TooLong,
}

/// What the person does with the card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Granted,
    Denied,
    /// Nobody answers: the card stays up until the wait is over.
    Silence,
    Consumed,
}

impl Answer {
    fn word(self) -> &'static str {
        match self {
            Answer::Granted => "granted",
            Answer::Denied => "denied",
            Answer::Silence => "pending",
            Answer::Consumed => "consumed",
        }
    }
}

/// The running fake: its log, and (for a session fake) the desk it put in place of the session's.
pub struct FakeShell {
    heard: Arc<Mutex<Vec<serde_json::Value>>>,
    dir: std::path::PathBuf,
    installed: bool,
}

static TAGS: AtomicUsize = AtomicUsize::new(0);

fn scratch(kind: &str) -> std::path::PathBuf {
    let n = TAGS.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("yantrik-fake-{kind}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch directory");
    dir
}

impl FakeShell {
    /// A fake on its own scratch directory, reached through [`FakeShell::desk`].
    pub fn start(first: First, answer: Answer) -> FakeShell {
        let dir = scratch("shell");
        let heard = serve(&dir.join("app-shell.sock"), first, answer);
        FakeShell { heard, dir, installed: false }
    }

    /// A fake that a tool reaches the way it reaches the shell, through the session's desk
    /// (`Desk::session()`): for the thread that calls this, until the fake is dropped, with the
    /// wait cut to a fraction of a second. Per thread, so tests run side by side.
    pub fn session(first: First, answer: Answer) -> FakeShell {
        let mut shell = FakeShell::start(first, answer);
        shell.install();
        shell
    }

    /// The session's desk with nobody behind it: the shell is not running.
    pub fn nobody() -> FakeShell {
        let dir = scratch("nobody");
        let mut shell = FakeShell { heard: Arc::default(), dir, installed: false };
        shell.install();
        shell
    }

    fn install(&mut self) {
        let desk = self.desk();
        crate::app_ask::TEST_DESK.with(|d| *d.borrow_mut() = Some(desk));
        self.installed = true;
    }

    /// A desk that reaches this fake, with a short wait.
    pub(crate) fn desk(&self) -> Desk {
        Desk { dir: Some(self.dir.clone()), wait: Duration::from_millis(300), poll: Duration::from_millis(10) }
    }

    /// Every call the shell heard, as `app.act`'s params.
    pub fn calls(&self) -> Vec<serde_json::Value> {
        self.heard.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// The `action` of every call, in order.
    pub fn actions(&self) -> Vec<String> {
        self.calls().iter().map(|p| p["action"].as_str().unwrap_or("").to_string()).collect()
    }

    /// The arguments of every card put up (`request_approval`), in order.
    pub fn cards(&self) -> Vec<serde_json::Value> {
        self.calls().into_iter().filter(|p| p["action"] == "request_approval").map(|p| p["args"].clone()).collect()
    }

    /// How many times a grant was spent.
    pub fn spent(&self) -> usize {
        self.actions().iter().filter(|a| *a == "consume_approval").count()
    }
}

impl Drop for FakeShell {
    fn drop(&mut self) {
        if self.installed {
            crate::app_ask::TEST_DESK.with(|d| *d.borrow_mut() = None);
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn serve(socket: &std::path::Path, first: First, answer: Answer) -> Arc<Mutex<Vec<serde_json::Value>>> {
    let _ = std::fs::remove_file(socket);
    let listener = UnixListener::bind(socket).expect("fake shell socket");
    let heard = Arc::new(Mutex::new(Vec::new()));
    let log = heard.clone();
    // Each card gets its own request id; its status reads "pending" once, then the answer.
    let cards = Arc::new(AtomicUsize::new(0));
    let polls = Arc::new(Mutex::new(std::collections::HashMap::<String, usize>::new()));
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { return };
            let (log, cards, polls) = (log.clone(), cards.clone(), polls.clone());
            std::thread::spawn(move || {
                let Ok(mut writer) = stream.try_clone() else { return };
                for line in BufReader::new(stream).lines() {
                    let Ok(line) = line else { return };
                    let Ok(asked) = serde_json::from_str::<serde_json::Value>(&line) else { return };
                    let params = asked["params"].clone();
                    log.lock().unwrap_or_else(|e| e.into_inner()).push(params.clone());
                    let result = match params["action"].as_str() {
                        Some("request_approval") => match first {
                            First::NotNeeded => serde_json::json!({"result": {"status": "not_needed", "mode": "bypass_all", "answered": "mode"}}),
                            First::NoId => serde_json::json!({"result": {"status": "pending"}}),
                            First::TooLong => serde_json::json!({"error": {"code": -32602,
                                "message": "this request is too long to put in front of a person in full, so nothing was asked: a value is too long"}}),
                            First::Pending => {
                                let n = cards.fetch_add(1, Ordering::SeqCst) + 1;
                                serde_json::json!({"result": {"request_id": format!("appr-{n}"), "status": "pending"}})
                            }
                        },
                        Some("approval_status") => {
                            let id = params["args"]["request_id"].as_str().unwrap_or("").to_string();
                            let mut seen = polls.lock().unwrap_or_else(|e| e.into_inner());
                            let n = seen.entry(id).or_insert(0);
                            *n += 1;
                            serde_json::json!({"result": {"status": if *n == 1 { "pending" } else { answer.word() }}})
                        }
                        Some("consume_approval") => serde_json::json!({"result": {"consumed": true, "answered": "pressed"}}),
                        Some("record_unasked_action") => serde_json::json!({"result": {"recorded": true}}),
                        _ => serde_json::json!({"error": {"code": -32602, "message": "unknown"}}),
                    };
                    let reply = match result.get("error") {
                        Some(error) => serde_json::json!({"jsonrpc": "2.0", "id": asked["id"], "error": error}),
                        None => serde_json::json!({"jsonrpc": "2.0", "id": asked["id"], "result": result}),
                    };
                    if writer.write_all(format!("{reply}\n").as_bytes()).is_err() {
                        return;
                    }
                }
            });
        }
    });
    heard
}
