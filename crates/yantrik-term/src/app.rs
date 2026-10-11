//! The client's state and what the keys do. The shell is polled on a thread of its own and sends
//! go on another, so the screen never waits on the socket.

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::json;

use crate::client::Shell;
use crate::view::ChatView;

/// How often the chat is read. A revision that has not moved costs one small reply.
const POLL: Duration = Duration::from_millis(350);

pub enum Update {
    View(ChatView),
    Offline(String),
    Said(String),
}

pub struct App {
    pub view: ChatView,
    pub input: String,
    pub scroll: usize,
    pub tick: u64,
    pub connected: bool,
    /// One line for the footer: a command's answer or an error, until the next key.
    pub note: Option<String>,
    pub quit: bool,
    history: Vec<String>,
    recall: Option<usize>,
    tx: Sender<Update>,
    pub rx: Receiver<Update>,
}

impl App {
    pub fn new() -> App {
        let (tx, rx) = mpsc::channel();
        App {
            view: ChatView::default(),
            input: String::new(),
            scroll: 0,
            tick: 0,
            connected: false,
            note: None,
            quit: false,
            history: Vec::new(),
            recall: None,
            tx,
            rx,
        }
    }

    /// Start reading the chat: the whole visible tail, then again whenever it changes.
    pub fn start_polling(&self) {
        let tx = self.tx.clone();
        thread::spawn(move || {
            let shell = Shell::connect();
            let mut last = String::new();
            loop {
                match shell.chat_view(None) {
                    Ok(v) => {
                        let view = ChatView::read(&v);
                        if view.revision != last {
                            last = view.revision.clone();
                            if tx.send(Update::View(view)).is_err() {
                                return;
                            }
                        }
                    }
                    Err(e) => {
                        last.clear();
                        if tx.send(Update::Offline(e)).is_err() {
                            return;
                        }
                    }
                }
                thread::sleep(POLL);
            }
        });
    }

    pub fn apply(&mut self, update: Update) {
        match update {
            Update::View(v) => {
                self.connected = true;
                self.view = v;
            }
            Update::Offline(why) => {
                self.connected = false;
                self.note = Some(format!("the desktop is not answering: {why}"));
            }
            // An act with nothing to say (a message sent) leaves the status line as it was.
            Update::Said(line) if line.is_empty() => {}
            Update::Said(line) => self.note = Some(line),
        }
    }

    /// Run a shell act on a thread; its outcome comes back as a footer line.
    fn act(&self, action: &'static str, args: serde_json::Value, done: impl Fn(serde_json::Value) -> String + Send + 'static) {
        let tx = self.tx.clone();
        thread::spawn(move || {
            let line = match Shell::connect().act(action, args) {
                Ok(v) => done(v),
                Err(e) => format!("{action}: {e}"),
            };
            let _ = tx.send(Update::Said(line));
        });
    }

    pub fn key(&mut self, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let alt = k.modifiers.contains(KeyModifiers::ALT);
        if !matches!(k.code, KeyCode::PageUp | KeyCode::PageDown) {
            self.note = None;
        }
        match k.code {
            KeyCode::Char('c') | KeyCode::Char('d') if ctrl => self.quit = true,
            KeyCode::Char('u') if ctrl => self.input.clear(),
            KeyCode::Enter if alt || k.modifiers.contains(KeyModifiers::SHIFT) => self.input.push('\n'),
            KeyCode::Enter => self.submit(),
            KeyCode::Char(c) => self.input.push(c),
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Esc => self.input.clear(),
            KeyCode::PageUp => self.scroll += 10,
            KeyCode::PageDown => self.scroll = self.scroll.saturating_sub(10),
            KeyCode::End => self.scroll = 0,
            KeyCode::Up if !self.input.contains('\n') => self.recall_older(),
            KeyCode::Down if !self.input.contains('\n') => self.recall_newer(),
            _ => {}
        }
    }

    fn recall_older(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let i = match self.recall {
            Some(0) => 0,
            Some(i) => i - 1,
            None => self.history.len() - 1,
        };
        self.recall = Some(i);
        self.input = self.history[i].clone();
    }

    fn recall_newer(&mut self) {
        match self.recall {
            Some(i) if i + 1 < self.history.len() => {
                self.recall = Some(i + 1);
                self.input = self.history[i + 1].clone();
            }
            _ => {
                self.recall = None;
                self.input.clear();
            }
        }
    }

    fn submit(&mut self) {
        let text = self.input.trim().to_string();
        self.input.clear();
        self.recall = None;
        if text.is_empty() {
            return;
        }
        self.history.push(text.clone());
        self.scroll = 0;
        if let Some(cmd) = text.strip_prefix('/') {
            self.command(cmd);
            return;
        }
        self.act("send_message", json!({ "text": text }), |_| String::new());
    }

    fn command(&mut self, cmd: &str) {
        let (name, arg) = cmd.split_once(' ').map(|(a, b)| (a, b.trim())).unwrap_or((cmd, ""));
        match name {
            "quit" | "exit" | "q" => self.quit = true,
            "help" | "?" => {
                self.note = Some(
                    "/mind [id] switch mind · /new new chat · /view show Mind View · /agents · /mode [plan|ask|auto] · /quit".into(),
                )
            }
            "new" | "clear" => self.act("new_chat", json!({}), |_| "a new chat".into()),
            "view" => self.act("focus_window", json!({ "title": "Mind View" }), |_| "Mind View is in front on the desktop".into()),
            "agents" => self.act("show_screen", json!({ "screen": "agents" }), |_| "the Agents screen is open on the desktop".into()),
            "mind" if arg.is_empty() => {
                let list: Vec<String> = self
                    .view
                    .minds
                    .iter()
                    .map(|m| if m.answering { format!("[{}]", m.id) } else { m.id.clone() })
                    .collect();
                self.note = Some(format!("minds: {} · /mind <id> to switch", list.join("  ")));
            }
            "mind" => {
                let id = arg.to_string();
                self.act("use_harness", json!({ "id": id }), move |_| format!("{id} answers now"));
            }
            "mode" if arg.is_empty() => self.note = Some(format!("mode: {} · /mode plan|ask|auto", self.view.mode)),
            "mode" => {
                let mode = arg.to_string();
                self.act("set_mind_mode", json!({ "mode": mode }), move |_| format!("mode: {mode}"));
            }
            other => self.note = Some(format!("no command /{other} · /help")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEventKind;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent { code, modifiers: KeyModifiers::NONE, kind: KeyEventKind::Press, state: crossterm::event::KeyEventState::NONE }
    }

    #[test]
    fn history_walks_back_and_forth() {
        let mut app = App::new();
        app.history = vec!["one".into(), "two".into()];
        app.key(key(KeyCode::Up));
        assert_eq!(app.input, "two");
        app.key(key(KeyCode::Up));
        assert_eq!(app.input, "one");
        app.key(key(KeyCode::Up));
        assert_eq!(app.input, "one", "stays at the oldest");
        app.key(key(KeyCode::Down));
        assert_eq!(app.input, "two");
        app.key(key(KeyCode::Down));
        assert_eq!(app.input, "", "past the newest is an empty line");
    }

    #[test]
    fn commands_that_need_no_desktop_answer_at_once() {
        let mut app = App::new();
        app.input = "/help".into();
        app.key(key(KeyCode::Enter));
        assert!(app.note.as_deref().unwrap().contains("/mind"));
        app.input = "/nope".into();
        app.key(key(KeyCode::Enter));
        assert_eq!(app.note.as_deref(), Some("no command /nope · /help"));
        app.input = "/quit".into();
        app.key(key(KeyCode::Enter));
        assert!(app.quit);
    }
}
