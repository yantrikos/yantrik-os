//! Shared token streaming — the chat panel's pump, used by on_send_message and on_lens_submit.
//!
//! Each answer gets a bubble and a 16ms timer that appends whatever arrives to that bubble until
//! the stream ends.
//!
//! # Why a list of streams and not one
//!
//! There used to be one slot holding one timer, and starting a second answer replaced it. Since
//! the pump's receiver lives in the timer's closure, replacing the timer dropped the receiver of
//! the answer still arriving — and the sender on the other side is the harness host's channel for
//! that turn. The host saw the send fail, dropped the turn, and refused every chunk after it:
//! "turn 18 is not one this harness was given".
//!
//! A mind working on a long task and a person typing something else meanwhile is not an edge
//! case. It is exactly what an approval is: Hermes asked to run a command, the person answered
//! `/approve` — and that answer killed the task that was waiting for it. Everything the agent said
//! for the next twenty minutes had nowhere to go, and the conversation looked dead while the work
//! carried on.
//!
//! So every stream keeps its own pump and its own bubble, and a stream ends only when its own
//! answer ends. Finished pumps are pruned when the next one starts.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use slint::{Model, ModelRc, SharedString, Timer, TimerMode, VecModel};

use crate::bridge::CompanionBridge;
use crate::markdown;
use crate::{App, ContentBlock, MessageData};

/// One running pump: whether it has finished, and whatever has to stay alive for it to run.
struct Stream {
    done: Rc<Cell<bool>>,
    /// The timer driving this pump. Held as `Any` only so the bookkeeping below can be tested
    /// without a Slint event loop to own a real `Timer`.
    _driver: Box<dyn std::any::Any>,
}

/// The answers arriving on screen right now.
///
/// Held by the callback that starts streams, so the timers outlive the call that created them.
#[derive(Clone, Default)]
pub struct Streams(Rc<RefCell<Vec<Stream>>>);

impl Streams {
    pub fn new() -> Self {
        Self::default()
    }

    /// How many answers are still arriving.
    pub fn active(&self) -> usize {
        self.0.borrow().iter().filter(|s| !s.done.get()).count()
    }

    fn remember(&self, done: Rc<Cell<bool>>, timer: Timer) {
        self.keep(done, Box::new(timer));
    }

    fn keep(&self, done: Rc<Cell<bool>>, driver: Box<dyn std::any::Any>) {
        let mut streams = self.0.borrow_mut();
        // A finished pump has nothing left to do; dropping its timer here is the cleanup that
        // used to happen by replacing the single slot. A pump still running is left alone, which
        // is the whole point.
        streams.retain(|s| !s.done.get());
        streams.push(Stream { done, _driver: driver });
    }
}

/// Who says a reply (#673): the mind answering when it begins, and the model it is on. Kept on
/// the message, so switching mind later never relabels it: `use_harness companion` used to put
/// the Mind's earlier answers under "Yantrik Companion", because every bubble was labelled with
/// whichever mind was answering now.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Speaker {
    pub mind: String,
    pub model: String,
}

impl Speaker {
    /// The mind answering now, as the picker names it.
    pub fn now() -> Speaker {
        crate::picker::speaker()
    }

    /// The built-in companion, which speaks unprompted whoever is answering the person.
    pub fn companion() -> Speaker {
        Speaker { mind: crate::wire::harness::BUILTIN_NAME.to_string(), model: String::new() }
    }
}

/// A reply's bubble, before its first word: empty, streaming, and signed by `speaker`.
pub fn reply_bubble(speaker: &Speaker) -> MessageData {
    MessageData {
        run: Default::default(),
        role: "assistant".into(),
        content: "".into(),
        is_streaming: true,
        blocks: ModelRc::default(),
        mind: speaker.mind.as_str().into(),
        model: speaker.model.as_str().into(),
    }
}

/// The same thing, from whatever is answering.
///
/// Split out because the body had the builtin companion welded into it: it called
/// `bridge.send_message` itself, so every message anyone typed went to the builtin no matter
/// which mind the picker said was driving. The harness host, the socket, the picker and the
/// status-bar chip were all built and all correct, and the conversation walked straight past
/// them. Selecting a mind changed a label.
pub fn stream_into(
    ui_weak: slint::Weak<App>,
    token_rx: crossbeam_channel::Receiver<String>,
    text: &str,
    streams: &Streams,
) {
    let row = match open_bubbles(&ui_weak, Some(text), &Speaker::now()) {
        Some(row) => row,
        None => return,
    };
    pump(ui_weak, token_rx, row, streams);
}

/// Put a finished message in the conversation: `asked` as the person's bubble first when given,
/// then `said` under `role`. For words the desktop itself says (`"desktop"`), which the bubble
/// labels as the desktop's so they are never read as the mind's.
pub fn say(ui_weak: &slint::Weak<App>, asked: Option<&str>, role: &str, said: &str) {
    let Some(ui) = ui_weak.upgrade() else { return };
    let messages = ui.get_messages();
    let Some(model) = messages.as_any().downcast_ref::<VecModel<MessageData>>() else { return };
    if let Some(text) = asked {
        model.push(MessageData {
            run: Default::default(),
            role: "user".into(),
            content: SharedString::from(text),
            is_streaming: false,
            blocks: ModelRc::default(),
            mind: Default::default(),
            model: Default::default(),
        });
        ui.set_lens_chat_mode(true);
    }
    model.push(MessageData {
        run: Default::default(),
        role: role.into(),
        content: SharedString::from(said),
        is_streaming: false,
        blocks: ModelRc::default(),
        mind: Default::default(),
        model: Default::default(),
    });
}

/// `say` as the desktop, with a link under the words: `link` is what the bubble's link carries
/// (a run id, or `private_mode::LEAVE_LINK`).
pub fn offer(ui_weak: &slint::Weak<App>, asked: Option<&str>, said: &str, link: &str) {
    say(ui_weak, asked, "desktop", said);
    let Some(ui) = ui_weak.upgrade() else { return };
    let messages = ui.get_messages();
    let Some(model) = messages.as_any().downcast_ref::<VecModel<MessageData>>() else { return };
    let last = model.row_count().saturating_sub(1);
    if let Some(mut row) = model.row_data(last) {
        row.run = link.into();
        model.set_row_data(last, row);
    }
}

/// Stream an answer into a bubble of its own, under a question already in the conversation.
pub fn stream_answer(ui_weak: slint::Weak<App>, token_rx: crossbeam_channel::Receiver<String>, streams: &Streams) {
    let Some(row) = open_bubbles(&ui_weak, None, &Speaker::now()) else {
        return;
    };
    pump(ui_weak, token_rx, row, streams);
}

/// Start a proactive AI stream — only the assistant's response is shown (no user bubble).
/// Used for morning brief and other proactive messages where the AI speaks first.
pub fn start_proactive_stream(
    ui_weak: slint::Weak<App>,
    bridge: &Arc<CompanionBridge>,
    hidden_prompt: &str,
    streams: &Streams,
) {
    // The companion speaks first here, whoever is answering the person.
    let Some(row) = open_bubbles(&ui_weak, None, &Speaker::companion()) else {
        return;
    };
    pump(ui_weak, bridge.send_message(hidden_prompt.to_string()), row, streams);
}

/// Add the bubbles an answer streams into. Returns the row of the assistant's bubble.
///
/// The row, not "the last row": with two answers arriving at once, the last row belongs to
/// whichever started most recently, and the other one would write its words into it. Rows are
/// only ever appended, so an index stays pointing at the same message.
fn open_bubbles(ui_weak: &slint::Weak<App>, asked: Option<&str>, speaker: &Speaker) -> Option<usize> {
    let ui = ui_weak.upgrade()?;
    let messages = ui.get_messages();
    let model = messages.as_any().downcast_ref::<VecModel<MessageData>>()?;
    if let Some(text) = asked {
        model.push(MessageData {
            run: Default::default(),
            role: "user".into(),
            content: SharedString::from(text),
            is_streaming: false,
            blocks: ModelRc::default(),
            mind: Default::default(),
            model: Default::default(),
        });
    }
    model.push(reply_bubble(speaker));
    ui.set_is_generating(true);
    ui.set_is_thinking(true);
    ui.set_companion_status("thinking".into());
    // Readable from the companion worker thread, which cannot touch a Slint property and must
    // not put an unprompted message into a transcript that is mid-answer.
    crate::wire::notifications::note_answer_started();
    if asked.is_some() {
        // Shown, not merely recorded.
        ui.set_lens_chat_mode(true);
    }
    Some(model.row_count() - 1)
}

/// Poll one answer at 60fps and append it to its own bubble.
/// A token naming the run a reply was (`mind:main#n`), sent just before `__DONE__`.
pub const RUN_MARK: &str = "__RUN__:";

fn pump(
    ui_weak: slint::Weak<App>,
    token_rx: crossbeam_channel::Receiver<String>,
    row: usize,
    streams: &Streams,
) {
    let done_flag = Rc::new(Cell::new(false));
    let finished = done_flag.clone();
    let mine = streams.clone();
    let replace_next = Rc::new(RefCell::new(false));
    // The reply as it arrived, trail lines and all. The bubble shows it without them while it
    // streams, and gets the whole of it back at the end, for the fold.
    let whole = Rc::new(RefCell::new(String::new()));

    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_millis(16), move || {
        let mut done = false;
        while let Ok(token) = token_rx.try_recv() {
            if token == "__DONE__" {
                done = true;
                break;
            }
            // The run this reply was: its link, not its text. Never the desktop's own offer to
            // leave Private mode, which only the desktop puts on a bubble (`streaming::offer`).
            if token.strip_prefix(RUN_MARK) == Some(crate::private_mode::LEAVE_LINK) {
                continue;
            }
            if let Some(run) = token.strip_prefix(RUN_MARK) {
                if let Some(ui) = ui_weak.upgrade() {
                    let messages = ui.get_messages();
                    if let Some(model) = messages.as_any().downcast_ref::<VecModel<MessageData>>() {
                        if let Some(mut bubble) = model.row_data(row) {
                            bubble.run = run.into();
                            model.set_row_data(row, bubble);
                        }
                    }
                }
                continue;
            }
            // __REPLACE__: the next token replaces the whole message content (used when tool
            // calls are detected, to strip raw XML).
            if token == "__REPLACE__" {
                *replace_next.borrow_mut() = true;
                continue;
            }
            if let Some(ui) = ui_weak.upgrade() {
                let messages = ui.get_messages();
                let Some(model) = messages.as_any().downcast_ref::<VecModel<MessageData>>() else {
                    continue;
                };
                if let Some(mut bubble) = model.row_data(row) {
                    if *replace_next.borrow() {
                        *whole.borrow_mut() = token;
                        *replace_next.borrow_mut() = false;
                    } else {
                        whole.borrow_mut().push_str(&token);
                    }
                    bubble.content = SharedString::from(crate::trail::without_trail(&whole.borrow()));
                    model.set_row_data(row, bubble);
                }
            }
        }
        if done {
            finished.set(true);
            crate::wire::notifications::note_answer_ended();
            if let Some(ui) = ui_weak.upgrade() {
                let messages = ui.get_messages();
                if let Some(model) = messages.as_any().downcast_ref::<VecModel<MessageData>>() {
                    if let Some(mut bubble) = model.row_data(row) {
                        bubble.is_streaming = false;
                        if !whole.borrow().is_empty() {
                            bubble.content = SharedString::from(whole.borrow().as_str());
                        }
                        let blocks = parse_content_blocks(&bubble.content);
                        // A reply with no trail lines (the Mind writes none) folds the steps the
                        // Lens followed instead, so every mind's answer can say what it did.
                        let steps = crate::wire::lens_steps::fold(row);
                        bubble.blocks = with_steps(blocks, &steps);
                        model.set_row_data(row, bubble);
                    }
                }
                // Still thinking, if something else is. The spinner belongs to the panel, not to
                // one answer, and turning it off here would hide work that is still running.
                if mine.active() == 0 {
                    ui.set_is_generating(false);
                    ui.set_is_thinking(false);
                    ui.set_companion_status("idle".into());
                }
            }
        }
    });
    streams.remember(done_flag, timer);
}

/// `blocks` with the turn's steps after them as "Show actions", when the reply carried no call of
/// its own to fold. A reply whose trail lines are already folded keeps exactly those.
fn with_steps(blocks: ModelRc<ContentBlock>, steps: &[crate::wire::lens_steps::Step]) -> ModelRc<ContentBlock> {
    let mut all: Vec<ContentBlock> = blocks.iter().collect();
    if steps.is_empty() || all.iter().any(|b| b.block_type == "tool") {
        return blocks;
    }
    all.extend(steps.iter().map(|s| ContentBlock {
        block_type: "tool".into(),
        text: s.text.as_str().into(),
        call: crate::ToolCallData {
            summary: s.text.as_str().into(),
            status: s.state.word().into(),
            ..Default::default()
        },
    }));
    ModelRc::new(VecModel::from(all))
}

/// Parse message content into styled blocks for rich rendering.
fn parse_content_blocks(content: &SharedString) -> ModelRc<ContentBlock> {
    let text = content.to_string();
    if text.trim().is_empty() {
        return ModelRc::default();
    }

    let parsed = markdown::parse_blocks(&text);
    let blocks: Vec<ContentBlock> = parsed
        .into_iter()
        .map(|b| ContentBlock {
            block_type: SharedString::from(b.block_type),
            text: SharedString::from(b.text.as_str()),
            // The card the bubble draws for a tool call; empty for every other block.
            call: b.call.as_ref().map(crate::trail::ToolCall::to_card).unwrap_or_default(),
        })
        .collect();

    ModelRc::new(VecModel::from(blocks))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the bubble is handed when an answer finishes: a tool call arrives as a `tool` block
    /// whose card holds the one line and the whole arguments — the exact thing the person clicks
    /// open. On the version #125 fixed, the same text reached the bubble as a prose block reading
    /// `⚙️ os_act studio.generate …` with nowhere to put the arguments.
    #[test]
    fn a_finished_answer_hands_the_bubble_each_call_with_its_arguments() {
        let blocks = parse_content_blocks(&SharedString::from(
            "On it.\n⚙️ os_act studio.generate {\"args\":{\"prompt\":\"a red kite over the sea\"}}\n\nDone.",
        ));
        let tool = blocks.row_data(1).expect("the call is the second block");
        assert_eq!(tool.block_type, "tool");
        assert_eq!(tool.call.summary, "os_act studio.generate prompt=\"a red kite over the sea\"");
        assert!(tool.call.arguments.contains("\"prompt\": \"a red kite over the sea\""));
        let prose = blocks.row_data(2).expect("the answer goes on after the call");
        assert_eq!(prose.text, "Done.");
        assert_eq!(prose.call.summary, "", "a paragraph carries no call");
    }

    /// The trail lines of a reply still arriving are steps above it, not its words; the whole text
    /// keeps them, so the finished reply folds them under "Show actions" as before.
    #[test]
    fn trail_lines_are_left_out_while_streaming_and_kept_for_the_fold() {
        let whole = "Saving that.\n⚙️ memory_remember {\"text\": \"tea, no sugar\"}\nDone, I\u{2019}ll remember.";
        let shown = crate::trail::without_trail(whole);
        assert_eq!(shown, "Saving that.\nDone, I\u{2019}ll remember.");
        assert!(!shown.contains('\u{2699}') && !shown.contains('{'));
        assert_eq!(crate::trail::without_trail("⚙️ os_ac"), "", "a call still arriving is not drawn either");
        assert_eq!(crate::trail::without_trail("⚙️ os_act(['app'])\n{\"app\": \"notes\"}\nOk."), "Ok.", "Hermes' arguments line goes with its call");

        let blocks = parse_content_blocks(&SharedString::from(whole));
        let tool = blocks.iter().find(|b| b.block_type == "tool").expect("the call is folded at the end");
        assert_eq!(tool.call.name, "memory_remember");
    }

    /// The Mind writes no trail lines: its steps become the fold. A reply that carried its own
    /// calls keeps them, and is not given a second list.
    #[test]
    fn a_reply_with_no_trail_lines_folds_the_steps_the_lens_followed() {
        use crate::wire::lens_steps::{Step, StepState};
        let steps = vec![
            Step { text: "Looked at your calendar".into(), state: StepState::Done, action: true },
            Step { text: "Couldn\u{2019}t open the web page".into(), state: StepState::Failed, action: true },
        ];
        let blocks = with_steps(parse_content_blocks(&SharedString::from("You have a dentist at 10.")), &steps);
        let tools: Vec<ContentBlock> = blocks.iter().filter(|b| b.block_type == "tool").collect();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].call.summary, "Looked at your calendar");
        assert_eq!(tools[1].call.status, "failed");
        assert_eq!(tools[0].call.arguments, "", "nothing to open: the sentence is everything");

        let own = parse_content_blocks(&SharedString::from("On it.\n⚙️ os_apps\nDone."));
        let kept = with_steps(own, &steps);
        assert_eq!(kept.iter().filter(|b| b.block_type == "tool").count(), 1);
    }

    /// Starting an answer must not end one that is still arriving.
    ///
    /// The single slot did exactly that, and the cost was not a visual glitch: the pump's
    /// receiver died with it, the harness host saw its send fail and dropped the turn, and the
    /// mind's next twenty minutes of work had nowhere to go.
    #[test]
    fn a_new_answer_does_not_end_one_still_arriving() {
        let streams = Streams::new();
        let first = Rc::new(Cell::new(false));
        let second = Rc::new(Cell::new(false));
        streams.keep(first.clone(), Box::new(()));
        streams.keep(second.clone(), Box::new(()));
        assert_eq!(streams.active(), 2, "both answers are still arriving");

        first.set(true);
        let third = Rc::new(Cell::new(false));
        streams.keep(third, Box::new(()));
        assert_eq!(streams.active(), 2, "the finished one is gone; the live one is not");
        assert_eq!(streams.0.borrow().len(), 2, "a finished pump is dropped, not kept forever");
    }

    /// The spinner belongs to the panel: it stops when nothing is arriving, not when any one
    /// answer finishes.
    #[test]
    fn the_panel_is_still_thinking_while_anything_is_arriving() {
        let streams = Streams::new();
        let slow = Rc::new(Cell::new(false));
        let quick = Rc::new(Cell::new(false));
        streams.keep(slow.clone(), Box::new(()));
        streams.keep(quick.clone(), Box::new(()));
        quick.set(true);
        assert_eq!(streams.active(), 1);
        slow.set(true);
        assert_eq!(streams.active(), 0);
    }
}
