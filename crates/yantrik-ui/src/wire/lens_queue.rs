//! What the Lens's composer does while a mind is busy, said before the person presses Enter.
//!
//! The box used to take any message, answer it with "Hermes Agent is working on … /stop stops it"
//! from the desktop, and then hold it with the mind until the turn ended, with nothing on screen
//! saying so. Now it has three modes, and its placeholder names the one it is in:
//!
//! - **idle**: "Message Hermes Agent…"; Enter sends.
//! - **queue** (the mind is working and asks nothing): "Queue a message for Hermes Agent…". Enter
//!   puts the message in the conversation at once as a queued bubble ("Sends when Hermes Agent
//!   finishes", with × to take it back), and it is sent as an ordinary turn when the turn ends.
//! - **answer** (the mind's run waits on a question): "Answer Hermes Agent's question…", with the
//!   question above the box. Enter answers it, through the same one path as typing in its card.
//!
//! Commands (`/stop` and the rest) are never queued or taken as an answer: they go straight on.
//!
//! The desktop's own status reply (#246, `Progress::told`) went with the old reply: nothing here
//! can tell a status question from any other message, so the queued bubble is the answer, and the
//! reply's own timeline says what the mind is doing.

use std::cell::RefCell;

use slint::{ComponentHandle, Model, SharedString, VecModel};

use crate::agents::AgentId;
use crate::{AgentsState, App, MessageData};

/// What Enter does, as the composer says it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Idle,
    Queue,
    Answer,
}

impl Mode {
    /// As the Slint side spells it.
    pub fn word(self) -> &'static str {
        match self {
            Mode::Idle => "",
            Mode::Queue => "queue",
            Mode::Answer => "answer",
        }
    }
}

/// A question the active mind's run is waiting on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asking {
    pub agent: String,
    pub request: String,
    pub prompt: String,
}

/// The active mind, as the composer needs to know it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Composer {
    pub agent: Option<AgentId>,
    pub mind: String,
    /// It has a turn open.
    pub working: bool,
    /// The newest question its run waits on.
    pub asking: Option<Asking>,
}

impl Composer {
    /// The active mind's main agent, read from the Agents store. A mind the store holds nothing for
    /// (the built-in companion) is idle here: it takes a message beside its work itself.
    pub fn of(agent: &AgentId, mind: &str) -> Composer {
        crate::agents::store().read(|s| {
            let Some(a) = s.agent(agent) else {
                return Composer { agent: Some(agent.clone()), mind: mind.to_string(), ..Composer::default() };
            };
            Composer {
                agent: Some(agent.clone()),
                mind: a.meta.mind.clone(),
                working: a.open_turn().is_some(),
                asking: a.waiting_questions().last().map(|q| Asking {
                    agent: a.meta.id.0.clone(),
                    request: q.request.clone(),
                    prompt: q.prompt.trim().to_string(),
                }),
            }
        })
    }

    /// The mind answering in the Lens now. Idle when there is no host yet.
    pub fn active() -> Composer {
        let Some(host) = crate::wire::harness::host() else { return Composer::default() };
        let active = host.active_id();
        let mind = host.list().into_iter().find(|e| e.id == active).map(|e| e.name).unwrap_or_else(|| active.clone());
        Composer::of(&crate::agents::feed::main_agent(&active), &mind)
    }

    pub fn mode(&self) -> Mode {
        if self.asking.is_some() {
            Mode::Answer
        } else if self.working {
            Mode::Queue
        } else {
            Mode::Idle
        }
    }

    /// The box's placeholder.
    pub fn placeholder(&self) -> String {
        match self.mode() {
            Mode::Idle => format!("Message {}\u{2026}", self.mind),
            Mode::Queue => format!("Queue a message for {}\u{2026}", self.mind),
            Mode::Answer => format!("Answer {}\u{2019}s question\u{2026}", self.mind),
        }
    }

    /// What Enter does with `text`.
    pub fn enter(&self, text: &str) -> Enter {
        if text.trim_start().starts_with('/') {
            return Enter::Send;
        }
        match (self.mode(), &self.asking) {
            (Mode::Answer, Some(asking)) => Enter::Answer(asking.clone()),
            (Mode::Queue, _) => Enter::Queue,
            _ => Enter::Send,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Enter {
    Send,
    Queue,
    Answer(Asking),
}

/// Why a command closes the questions the run waits on, when it ends the run: `/stop`, or a new
/// conversation (`/new`, `/reset`, which every harness takes as one).
pub fn closes_questions(text: &str) -> Option<&'static str> {
    use crate::agents::model::Question;
    let word = text.split_whitespace().next()?.to_lowercase();
    match word.as_str() {
        "/stop" => Some(Question::STOPPED_BY_YOU),
        "/new" | "/reset" => Some(Question::NEW_CONVERSATION),
        _ => None,
    }
}

/// Close every question `agent` waits on, saying `why`: the card folds to "Not answered: why".
pub fn close_questions(agent: &AgentId, why: &str) {
    let waiting: Vec<String> = crate::agents::store()
        .read(|s| s.agent(agent).map(|a| a.waiting_questions().map(|q| q.request.clone()).collect()))
        .unwrap_or_default();
    for request in waiting {
        crate::agents::store().question_closed(agent, &request, why);
    }
}

/// The one message waiting for a mind to finish.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Queued {
    pub text: String,
    /// Whose turn ending sends it.
    pub agent: AgentId,
    pub mind: String,
    /// Its bubble: the row, in the transcript it was put in (`streaming::transcript`).
    pub row: usize,
    pub transcript: u64,
}

/// At most one queued message. A second replaces the first's words rather than lining up behind
/// it: a person who typed twice while the mind worked usually corrected themselves, and two queued
/// turns would each start a run the mind had not seen the other's answer to. The bubble says it
/// was replaced, so nothing is dropped unseen.
#[derive(Debug, Default)]
pub struct Queue {
    held: Option<Queued>,
}

impl Queue {
    /// Hold `text` for `agent`. A message already held keeps its bubble and takes the new words;
    /// otherwise `bubble` puts one in the conversation and gives its row. True when it replaced one.
    pub fn hold(&mut self, text: &str, agent: &AgentId, mind: &str, transcript: u64, bubble: impl FnOnce() -> usize) -> bool {
        if let Some(held) = self.held.as_mut().filter(|h| h.transcript == transcript) {
            held.text = text.to_string();
            held.agent = agent.clone();
            held.mind = mind.to_string();
            return true;
        }
        self.held = Some(Queued { text: text.to_string(), agent: agent.clone(), mind: mind.to_string(), row: bubble(), transcript });
        false
    }

    /// Take it back before it is sent.
    pub fn cancel(&mut self) -> Option<Queued> {
        self.held.take()
    }

    /// The message to send now that its mind is no longer `working`, taken out so it is sent once.
    /// One whose conversation was emptied since (New chat) is let go, unsent.
    pub fn due(&mut self, working: impl Fn(&AgentId) -> bool, transcript: u64) -> Option<Queued> {
        if self.held.as_ref().is_some_and(|h| h.transcript != transcript) {
            self.held = None;
        }
        if self.held.as_ref().is_some_and(|h| !working(&h.agent)) {
            return self.held.take();
        }
        None
    }

    pub fn held(&self) -> Option<&Queued> {
        self.held.as_ref()
    }
}

// ── The Lens's queue, on the UI thread ──────────────────────────────────────────────────────

/// A queued message's bubble: the person's words, drawn as waiting until they are sent.
pub const QUEUED_ROLE: &str = "queued";
/// A queued message taken back while other messages stood after it: drawn as nothing, because a
/// reply still arriving finds its bubble by row and a removed row would move it.
pub const WITHDRAWN_ROLE: &str = "withdrawn";

thread_local! {
    static QUEUE: RefCell<Queue> = RefCell::new(Queue::default());
}

fn with_model<R>(ui: &App, f: impl FnOnce(&VecModel<MessageData>) -> R) -> Option<R> {
    let messages = ui.get_messages();
    let model = messages.as_any().downcast_ref::<VecModel<MessageData>>()?;
    Some(f(model))
}

fn set_role(model: &VecModel<MessageData>, row: usize, role: &str, text: Option<&str>) {
    if let Some(mut m) = model.row_data(row) {
        m.role = role.into();
        if let Some(text) = text {
            m.content = SharedString::from(text);
        }
        model.set_row_data(row, m);
    }
}

/// Put `text` in the queue for the active mind, as a queued bubble at the end of the conversation.
pub fn hold(ui: &App, text: &str, composer: &Composer) {
    let Some(agent) = composer.agent.clone() else { return };
    let transcript = crate::streaming::transcript();
    let replaced = QUEUE.with(|q| {
        let mut q = q.borrow_mut();
        let replaced = q.hold(text, &agent, &composer.mind, transcript, || {
            with_model(ui, |model| {
                model.push(MessageData { role: QUEUED_ROLE.into(), content: text.into(), ..Default::default() });
                model.row_count() - 1
            })
            .unwrap_or(0)
        });
        if replaced {
            if let Some(row) = q.held().map(|h| h.row) {
                with_model(ui, |model| set_role(model, row, QUEUED_ROLE, Some(text)));
            }
        }
        replaced
    });
    ui.set_lens_chat_mode(true);
    publish(ui, replaced);
}

/// The × on the queued bubble: it is taken back and never sent.
pub fn cancel(ui: &App) {
    let Some(held) = QUEUE.with(|q| q.borrow_mut().cancel()) else { return };
    if held.transcript == crate::streaming::transcript() {
        with_model(ui, |model| {
            if held.row + 1 == model.row_count() {
                model.remove(held.row);
            } else {
                set_role(model, held.row, WITHDRAWN_ROLE, None);
            }
        });
    }
    publish(ui, false);
}

/// The queued message, if its mind has finished: its bubble becomes the person's ordinary one,
/// and the words are handed back to be sent as a turn. Taken once.
pub fn due(ui: &App) -> Option<String> {
    let transcript = crate::streaming::transcript();
    let working = |agent: &AgentId| crate::agents::store().read(|s| s.agent(agent).is_some_and(|a| a.open_turn().is_some()));
    let held = QUEUE.with(|q| q.borrow_mut().due(working, transcript))?;
    with_model(ui, |model| set_role(model, held.row, "user", None));
    publish(ui, false);
    Some(held.text)
}

/// What the queued bubble says under the words: whose finishing sends it, and that a second
/// message replaced the first.
fn publish(ui: &App, replaced: bool) {
    let g = ui.global::<AgentsState>();
    let mind = QUEUE.with(|q| q.borrow().held().map(|h| h.mind.clone())).unwrap_or_default();
    g.set_lens_queued_mind(mind.into());
    g.set_lens_queued_note(if replaced { "Replaced your earlier queued message".into() } else { SharedString::default() });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hermes(working: bool, asking: bool) -> Composer {
        Composer {
            agent: Some(AgentId::new("hermes", AgentId::MAIN)),
            mind: "Hermes Agent".into(),
            working,
            asking: asking.then(|| Asking {
                agent: "hermes:main".into(),
                request: "c1".into(),
                prompt: "Which days is this weekend for you?".into(),
            }),
        }
    }

    #[test]
    fn the_composer_says_which_of_its_three_modes_it_is_in_and_enter_does_that() {
        let idle = hermes(false, false);
        assert_eq!((idle.mode(), idle.placeholder().as_str()), (Mode::Idle, "Message Hermes Agent\u{2026}"));
        assert_eq!(idle.enter("Dallas weather this weekend?"), Enter::Send);

        let working = hermes(true, false);
        assert_eq!((working.mode(), working.placeholder().as_str()), (Mode::Queue, "Queue a message for Hermes Agent\u{2026}"));
        assert_eq!(working.enter("and Austin?"), Enter::Queue);

        let asking = hermes(true, true);
        assert_eq!((asking.mode(), asking.placeholder().as_str()), (Mode::Answer, "Answer Hermes Agent\u{2019}s question\u{2026}"));
        assert_eq!(asking.enter("Saturday only"), Enter::Answer(asking.asking.clone().unwrap()), "the free answer, not a new turn");
    }

    #[test]
    fn stop_and_a_new_conversation_close_a_waiting_question_with_the_persons_reason() {
        use crate::agents::model::Question;
        assert_eq!(closes_questions("/stop"), Some(Question::STOPPED_BY_YOU));
        assert_eq!(closes_questions("/new"), Some(Question::NEW_CONVERSATION));
        assert_eq!(closes_questions("/approve"), None);
        assert_eq!(closes_questions("stop it"), None, "words are not a command");
    }

    #[test]
    fn a_command_goes_straight_on_whatever_the_mode() {
        for composer in [hermes(false, false), hermes(true, false), hermes(true, true)] {
            assert_eq!(composer.enter("/stop"), Enter::Send, "{:?}", composer.mode());
            assert_eq!(composer.enter("  /approve"), Enter::Send);
        }
    }

    #[test]
    fn a_queued_message_is_sent_exactly_once_when_the_turn_ends() {
        let agent = AgentId::new("hermes", AgentId::MAIN);
        let mut queue = Queue::default();
        let mut rows = 4;
        assert!(!queue.hold("and Austin?", &agent, "Hermes Agent", 7, || { rows += 1; rows }));
        assert_eq!(queue.due(|_| true, 7), None, "not while the turn runs");
        let sent = queue.due(|_| false, 7).expect("the turn ended");
        assert_eq!((sent.text.as_str(), sent.row), ("and Austin?", 5));
        assert_eq!(queue.due(|_| false, 7), None, "and only once");
    }

    #[test]
    fn a_cancelled_message_is_never_sent() {
        let agent = AgentId::new("hermes", AgentId::MAIN);
        let mut queue = Queue::default();
        queue.hold("and Austin?", &agent, "Hermes Agent", 7, || 5);
        assert_eq!(queue.cancel().map(|q| q.text), Some("and Austin?".to_string()));
        assert_eq!(queue.due(|_| false, 7), None);
    }

    #[test]
    fn a_second_message_replaces_the_first_in_the_same_bubble() {
        let agent = AgentId::new("hermes", AgentId::MAIN);
        let mut queue = Queue::default();
        queue.hold("and Austin?", &agent, "Hermes Agent", 7, || 5);
        let replaced = queue.hold("and Houston?", &agent, "Hermes Agent", 7, || panic!("no second bubble"));
        assert!(replaced);
        let sent = queue.due(|_| false, 7).unwrap();
        assert_eq!((sent.text.as_str(), sent.row), ("and Houston?", 5));
    }

    #[test]
    fn a_new_chat_lets_go_of_a_queued_message_unsent() {
        let agent = AgentId::new("hermes", AgentId::MAIN);
        let mut queue = Queue::default();
        queue.hold("and Austin?", &agent, "Hermes Agent", 7, || 5);
        assert_eq!(queue.due(|_| false, 8), None, "its conversation is gone");
        assert!(queue.held().is_none());
    }
}
