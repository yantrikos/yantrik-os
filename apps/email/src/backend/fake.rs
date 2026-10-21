//! A scripted mail service: what each method answers, what was asked of it, and which pages it
//! was asked to open. No test reaches a mail server, a socket or a browser.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::{json, Value};
use yantrik_app::Paths;

use super::MailService;

type Answer = Result<Value, String>;

#[derive(Default)]
struct Script {
    /// Answered every time.
    always: HashMap<String, Answer>,
    /// Methods whose call panics, as a parser bug would.
    panics: Vec<String>,
    /// What opening a page in the browser does.
    browser: Option<Result<(), String>>,
    asked: Vec<(String, Value)>,
    opened: Vec<String>,
}

/// The fake. Cloning shares it.
#[derive(Clone, Default)]
pub struct FakeMail(Arc<Mutex<Script>>);

/// What the transport says when nothing answers: the sentence the live client gives.
pub const UNREACHABLE: &str = "Service at /run/user/1000/email.sock is unreachable (retrying in up to 5s)";

impl FakeMail {
    /// A service that answers nothing: every method is unreachable.
    pub fn unreachable() -> FakeMail {
        FakeMail::default()
    }

    /// A service running with no account in its file.
    pub fn no_account() -> FakeMail {
        FakeMail::default().answering(
            "email.accounts",
            json!({
                "accounts": [],
                "config_path": "/home/p/.config/yantrik/email.json",
                "secrets_are_plaintext": true,
                "google_sign_in": { "available": true, "note": "Google sign-in uses this build's client." },
            }),
        )
    }

    /// One account, with INBOX, Archive and an uncounted Receipts folder, and nothing listed.
    pub fn ready() -> FakeMail {
        FakeMail::default()
            .answering(
                "email.accounts",
                json!({
                    "accounts": [{
                        "id": "acct-1", "email": "you@example.com", "display_name": "You",
                        "provider": "gmail", "imap_server": "imap.gmail.com", "imap_port": 993,
                        "smtp_server": "smtp.gmail.com", "smtp_port": 587,
                    }],
                    "config_path": "/home/p/.config/yantrik/email.json",
                    "secrets_are_plaintext": true,
                    "google_sign_in": { "available": true, "note": "Google sign-in uses this build's client." },
                }),
            )
            .answering(
                "email.list_folders",
                json!([
                    { "name": "INBOX", "counts": { "unread": 2, "total": 3 } },
                    { "name": "Archive", "counts": { "unread": 0, "total": 10 } },
                    { "name": "Receipts", "counts": null, "reason": "the mail server did not report it: STATUS timed out" },
                ]),
            )
            .answering("email.list_messages", json!([]))
    }

    /// `method` answers `value` from now on.
    pub fn answering(self, method: &str, value: Value) -> FakeMail {
        self.set(method, Ok(value));
        self
    }

    /// `method` fails with this sentence from now on.
    pub fn failing(self, method: &str, message: &str) -> FakeMail {
        self.set(method, Err(message.into()));
        self
    }

    pub fn set(&self, method: &str, answer: Answer) {
        self.0.lock().unwrap().always.insert(method.into(), answer);
    }

    /// `method` answers this when it is asked about `folder`, before what it answers for every
    /// folder: each mailbox lists its own messages.
    pub fn set_for(&self, method: &str, folder: &str, answer: Answer) {
        self.0.lock().unwrap().always.insert(format!("{method}@{folder}"), answer);
    }

    pub fn panicking(self, method: &str) -> FakeMail {
        self.0.lock().unwrap().panics.push(method.into());
        self
    }

    /// Opening a page fails with this sentence, as on a machine with no handler.
    pub fn no_browser(self, why: &str) -> FakeMail {
        self.0.lock().unwrap().browser = Some(Err(why.into()));
        self
    }

    /// Every call, in order: the method and its parameters.
    pub fn asked(&self) -> Vec<(String, Value)> {
        self.0.lock().unwrap().asked.clone()
    }

    pub fn asked_for(&self, method: &str) -> Vec<Value> {
        self.asked().into_iter().filter(|(m, _)| m == method).map(|(_, p)| p).collect()
    }

    /// Every page the browser was asked to open.
    pub fn opened(&self) -> Vec<String> {
        self.0.lock().unwrap().opened.clone()
    }
}

impl MailService for FakeMail {
    fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        let mut script = self.0.lock().unwrap();
        script.asked.push((method.into(), params));
        if script.panics.iter().any(|m| m == method) {
            drop(script);
            panic!("the fake mail service was told to panic on {method}");
        }
        let folder = script.asked.last().and_then(|(_, p)| p["folder"].as_str()).map(|f| format!("{method}@{f}"));
        folder
            .and_then(|key| script.always.get(&key))
            .or_else(|| script.always.get(method))
            .cloned()
            .unwrap_or_else(|| Err(UNREACHABLE.into()))
    }

    fn open_in_browser(&self, url: &str) -> Result<(), String> {
        let mut script = self.0.lock().unwrap();
        script.opened.push(url.into());
        script.browser.clone().unwrap_or(Ok(()))
    }
}

fn installed() -> &'static Mutex<HashMap<PathBuf, FakeMail>> {
    static INSTALLED: OnceLock<Mutex<HashMap<PathBuf, FakeMail>>> = OnceLock::new();
    INSTALLED.get_or_init(Default::default)
}

/// The fake an app started at `paths` talks to. Each test's paths are its own.
pub fn install(paths: &Paths, fake: &FakeMail) {
    installed().lock().unwrap().insert(paths.config_file().to_path_buf(), fake.clone());
}

pub fn for_app(paths: &Paths) -> Arc<dyn MailService> {
    let found = installed().lock().unwrap().get(paths.config_file()).cloned();
    Arc::new(found.unwrap_or_else(FakeMail::unreachable))
}
