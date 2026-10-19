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
//!
//! # Why a bubble is a row in a transcript, not just a row
//!
//! A pump finds its bubble by row, and rows are only ever appended — until New chat empties the
//! conversation. New chat waits for the answering mind to finish, but not for a mind the person
//! has switched away from. On VM 541 a Hermes turn was stuck, the person switched to Yantrik Mind
//! and started afresh, and Hermes's pump went on writing to its old row: the row the Mind's reply
//! now stood in, which opened with Hermes's "⏳ Working — 3 min" line. So a bubble also names the
//! transcript it was opened in ([`clear`] starts a new one), and a pump whose transcript is gone
//! writes nothing more and stops listening. The harness host then treats that turn as one the
//! person left, the way New chat always meant: it keeps it on the Agents screen and tells the
//! late answer as a notification.

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

thread_local! {
    /// Which transcript the conversation is showing: one more each time [`clear`] empties it. Only
    /// the UI thread, which owns the transcript, reads or writes it.
    static TRANSCRIPT: Cell<u64> = const { Cell::new(0) };
}

/// Which transcript the conversation is showing now.
pub(crate) fn transcript() -> u64 {
    TRANSCRIPT.with(Cell::get)
}

/// Empty the conversation (New chat). Every answer still arriving loses its bubble with it, and
/// none of them may write into the bubbles that take their rows next.
pub fn clear(model: &VecModel<MessageData>) {
    model.set_vec(Vec::new());
    TRANSCRIPT.with(|t| t.set(t.get() + 1));
}

/// Where one answer is drawn: its row, in the transcript it was opened in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Bubble {
    row: usize,
    transcript: u64,
}

impl Bubble {
    /// A new reply bubble at the end of the conversation, signed by `speaker`.
    fn open(model: &VecModel<MessageData>, speaker: &Speaker) -> Bubble {
        model.push(reply_bubble(speaker));
        Bubble { row: model.row_count() - 1, transcript: transcript() }
    }

    /// Whether the bubble is still in the conversation. Its row may hold another reply now.
    fn here(&self) -> bool {
        self.transcript == transcript()
    }
}

/// What one token did to its answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Took {
    More,
    Done,
    /// The bubble went with the conversation it was in.
    Gone,
}

/// One answer as it arrives: what it has said, and the bubble it says it in.
struct Reply {
    bubble: Bubble,
    /// The reply as it arrived, trail lines and all. The bubble shows it without them while it
    /// streams, and gets the whole of it back at the end, for the fold.
    whole: String,
    /// `__REPLACE__` was seen: the next token is the whole reply (tool calls use it to strip raw
    /// XML from what was already streamed).
    replace_next: bool,
}

impl Reply {
    fn new(bubble: Bubble) -> Reply {
        Reply { bubble, whole: String::new(), replace_next: false }
    }

    /// Put one token where it belongs: in this answer's own bubble, and nowhere else.
    fn take(&mut self, model: &VecModel<MessageData>, token: String) -> Took {
        if !self.bubble.here() {
            return Took::Gone;
        }
        if token == "__DONE__" {
            return Took::Done;
        }
        // The run this reply was: its link, not its text. Never the desktop's own offer to leave
        // Private mode, which only the desktop puts on a bubble (`streaming::offer`).
        if token.strip_prefix(RUN_MARK) == Some(crate::private_mode::LEAVE_LINK) {
            return Took::More;
        }
        if let Some(run) = token.strip_prefix(RUN_MARK) {
            if let Some(mut bubble) = model.row_data(self.bubble.row) {
                bubble.run = run.into();
                model.set_row_data(self.bubble.row, bubble);
            }
            return Took::More;
        }
        if token == "__REPLACE__" {
            self.replace_next = true;
            return Took::More;
        }
        if let Some(mut bubble) = model.row_data(self.bubble.row) {
            if std::mem::take(&mut self.replace_next) {
                self.whole = token;
            } else {
                self.whole.push_str(&token);
            }
            bubble.content = SharedString::from(crate::trail::without_trail(&self.whole));
            model.set_row_data(self.bubble.row, bubble);
        }
        Took::More
    }

    /// The answer has ended: the bubble stops streaming and gets its whole text, under its
    /// timeline.
    fn finish(&self, model: &VecModel<MessageData>) {
        let row = self.bubble.row;
        if !self.bubble.here() {
            return;
        }
        let Some(mut bubble) = model.row_data(row) else { return };
        bubble.is_streaming = false;
        if !self.whole.is_empty() {
            bubble.content = SharedString::from(self.whole.as_str());
        }
        let blocks = parse_content_blocks(&bubble.content);
        bubble.blocks = with_timeline(blocks, &crate::wire::lens_timeline::fold(row));
        model.set_row_data(row, bubble);
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
    let Some(bubble) = open_bubbles(&ui_weak, Some(text), &Speaker::now()) else {
        return;
    };
    pump(ui_weak, token_rx, bubble, streams);
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
    let Some(bubble) = open_bubbles(&ui_weak, None, &Speaker::now()) else {
        return;
    };
    pump(ui_weak, token_rx, bubble, streams);
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
    let Some(bubble) = open_bubbles(&ui_weak, None, &Speaker::companion()) else {
        return;
    };
    pump(ui_weak, bridge.send_message(hidden_prompt.to_string()), bubble, streams);
}

/// Add the bubbles an answer streams into. Returns the assistant's bubble.
///
/// Its own row, not "the last row": with two answers arriving at once, the last row belongs to
/// whichever started most recently, and the other one would write its words into it. Rows are
/// only appended within a transcript, so the row keeps pointing at the same message for as long
/// as the bubble is [`Bubble::here`].
fn open_bubbles(ui_weak: &slint::Weak<App>, asked: Option<&str>, speaker: &Speaker) -> Option<Bubble> {
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
    let bubble = Bubble::open(model, speaker);
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
    Some(bubble)
}

/// A token naming the run a reply was (`mind:main#n`), sent just before `__DONE__`.
pub const RUN_MARK: &str = "__RUN__:";

/// Poll one answer at 60fps and append it to its own bubble.
fn pump(
    ui_weak: slint::Weak<App>,
    token_rx: crossbeam_channel::Receiver<String>,
    bubble: Bubble,
    streams: &Streams,
) {
    let done_flag = Rc::new(Cell::new(false));
    let finished = done_flag.clone();
    let mine = streams.clone();
    // Held so that it can be let go of: an answer whose bubble is gone stops being listened to.
    let receiver = RefCell::new(Some(token_rx));
    let reply = RefCell::new(Reply::new(bubble));

    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_millis(16), move || {
        if finished.get() {
            return;
        }
        let ui = ui_weak.upgrade();
        let messages = ui.as_ref().map(|ui| ui.get_messages());
        let model = messages.as_ref().and_then(|m| m.as_any().downcast_ref::<VecModel<MessageData>>());
        let mut took = if reply.borrow().bubble.here() { Took::More } else { Took::Gone };
        if took == Took::More {
            if let Some(rx) = receiver.borrow().as_ref() {
                while let Ok(token) = rx.try_recv() {
                    took = match model {
                        Some(model) => reply.borrow_mut().take(model, token),
                        None if token == "__DONE__" => Took::Done,
                        None => Took::More,
                    };
                    if took != Took::More {
                        break;
                    }
                }
            }
        }
        match took {
            Took::More => return,
            Took::Done => {
                if let Some(model) = model {
                    reply.borrow().finish(model);
                }
            }
            // Its conversation was emptied. Whatever it says next belongs to no bubble here, and
            // listening on would only keep the host from knowing the person left it.
            Took::Gone => drop(receiver.borrow_mut().take()),
        }
        finished.set(true);
        crate::wire::notifications::note_answer_ended();
        // Still thinking, if something else is. The spinner belongs to the panel, not to one
        // answer, and turning it off here would hide work that is still running.
        if let Some(ui) = ui.as_ref().filter(|_| mine.active() == 0) {
            ui.set_is_generating(false);
            ui.set_is_thinking(false);
            ui.set_companion_status("idle".into());
        }
    });
    streams.remember(done_flag, timer);
}

/// The finished reply as its bubble draws it: its timeline first, in the order the work happened,
/// then its words, with no step left among them.
///
/// The timeline is the one the Lens followed in the Agents store, where every call is, whether a
/// harness reported it or wrote it as a trail line; the trail lines in the text are the same calls
/// told again, so they are dropped. A reply the Lens did not follow (the built-in companion keeps no
/// turn in the store) has only the calls its own text told: those are lifted out of the text to the
/// top, in the Lens's words, so the reply still says what it did before what it says.
fn with_timeline(blocks: ModelRc<ContentBlock>, entries: &[crate::wire::lens_timeline::Entry]) -> ModelRc<ContentBlock> {
    let mut all: Vec<ContentBlock> = if entries.is_empty() {
        blocks.iter().filter(|b| b.block_type == "tool").map(step_of_trail).collect()
    } else {
        crate::wire::lens_timeline::blocks(entries)
    };
    if all.is_empty() {
        return blocks;
    }
    all.extend(blocks.iter().filter(|b| b.block_type != "tool"));
    ModelRc::new(VecModel::from(all))
}

/// A call read off the reply's text, as a step of its timeline: the Lens's words for it, and what
/// it opens to.
fn step_of_trail(block: ContentBlock) -> ContentBlock {
    let args: serde_json::Value = serde_json::from_str(&block.call.arguments).unwrap_or(serde_json::Value::Null);
    let phrase = crate::wire::step_words::of_call(&block.call.name, &block.call.target, &args, "");
    let (text, state) = if block.call.status == "failed" { (phrase.failed(), "failed") } else { (phrase.done(), "done") };
    ContentBlock {
        block_type: "step".into(),
        text: text.into(),
        call: crate::ToolCallData { status: state.into(), ..block.call },
    }
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
    /// keeps them, so the finished reply can still find its calls.
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

    fn kinds(blocks: &ModelRc<ContentBlock>) -> Vec<(String, String)> {
        blocks.iter().map(|b| (b.block_type.to_string(), b.text.to_string())).collect()
    }

    /// A finished reply is its timeline, then its words: the calls its text told are the same
    /// ones the store has, so they are not drawn a second time in the middle of what it says.
    #[test]
    fn a_finished_reply_draws_its_timeline_first_and_its_words_with_no_step_among_them() {
        use crate::wire::lens_steps::{Step, StepState};
        use crate::wire::lens_timeline::Entry;
        let timeline = vec![
            Entry::Thinking { secs: 3, text: "tea, then".into() },
            Entry::Step { step: Step { text: "Saved it to memory".into(), state: StepState::Done, action: true }, detail: None },
        ];
        let text = "Saving that.\n⚙️ memory_remember {\"text\": \"tea, no sugar\"}\nDone, I\u{2019}ll remember.";
        let blocks = with_timeline(parse_content_blocks(&SharedString::from(text)), &timeline);
        assert_eq!(
            kinds(&blocks),
            [
                ("thinking".into(), "Thought for 3 s".into()),
                ("step".into(), "Saved it to memory".into()),
                ("text".into(), "Saving that.".into()),
                ("text".into(), "Done, I\u{2019}ll remember.".into()),
            ]
        );
    }

    /// The built-in keeps no turn in the store, so the Lens follows nothing for it: the calls its
    /// own text told are its timeline, lifted to the top and said in the Lens's words.
    #[test]
    fn a_reply_the_lens_did_not_follow_lifts_its_own_calls_to_the_top_in_plain_words() {
        let blocks = with_timeline(parse_content_blocks(&SharedString::from("On it.\n⚙️ os_apps\nDone.")), &[]);
        let kinds = kinds(&blocks);
        assert_eq!(kinds[0].0, "step");
        assert!(!kinds[0].1.contains("os_apps"), "never the call's name: {:?}", kinds[0]);
        assert_eq!(&kinds[1..], [("text".to_string(), "On it.".to_string()), ("text".into(), "Done.".into())]);
        assert_eq!(blocks.row_data(0).unwrap().call.name, "os_apps", "what it opens to is the call itself");

        let plain = parse_content_blocks(&SharedString::from("Hello."));
        assert_eq!(kinds_of_plain(&with_timeline(plain, &[])), ["text"], "talk with no work is just its words");
    }

    fn kinds_of_plain(blocks: &ModelRc<ContentBlock>) -> Vec<String> {
        blocks.iter().map(|b| b.block_type.to_string()).collect()
    }

    fn speaker(mind: &str) -> Speaker {
        Speaker { mind: mind.into(), model: String::new() }
    }

    fn asked(model: &VecModel<MessageData>, text: &str) {
        model.push(MessageData { role: "user".into(), content: text.into(), ..reply_bubble(&Speaker::default()) });
    }

    fn said(model: &VecModel<MessageData>, bubble: Bubble) -> String {
        model.row_data(bubble.row).map(|m| m.content.to_string()).unwrap_or_default()
    }

    /// Two minds answering at once, their chunks interleaved: each bubble holds its own reply, and
    /// the newer one being the last row does not draw the older one's words into it.
    #[test]
    fn two_minds_answering_at_once_each_fill_only_their_own_bubble() {
        let transcript = VecModel::<MessageData>::default();
        asked(&transcript, "compare flights to Denver");
        let mut hermes = Reply::new(Bubble::open(&transcript, &speaker("Hermes Agent")));
        asked(&transcript, "what's on my calendar?");
        let mut mind = Reply::new(Bubble::open(&transcript, &speaker("Yantrik Mind")));

        assert_eq!(hermes.take(&transcript, "\u{23f3} Working \u{2014} 3 min".into()), Took::More);
        assert_eq!(mind.take(&transcript, "You have ".into()), Took::More);
        assert_eq!(hermes.take(&transcript, " \u{2014} iteration 1/500, clarify".into()), Took::More);
        assert_eq!(mind.take(&transcript, "a dentist at 10.".into()), Took::More);
        assert_eq!(mind.take(&transcript, "__DONE__".into()), Took::Done);
        assert_eq!(hermes.take(&transcript, "\n\nThe cheapest is Tuesday.".into()), Took::More);

        assert_eq!(said(&transcript, mind.bubble), "You have a dentist at 10.");
        assert_eq!(
            said(&transcript, hermes.bubble),
            "\u{23f3} Working \u{2014} 3 min \u{2014} iteration 1/500, clarify\n\nThe cheapest is Tuesday."
        );
        assert_eq!(transcript.row_data(mind.bubble.row).unwrap().mind, "Yantrik Mind");
    }

    /// VM 541: Hermes stuck mid-turn, the person switched to Yantrik Mind and started a new chat,
    /// and the Mind's reply opened in the row Hermes's pump was still writing to. The row is the
    /// same; the transcript is not, so Hermes's late words land nowhere and the Mind's bubble has
    /// only its own.
    #[test]
    fn an_answer_whose_conversation_was_emptied_never_writes_into_the_next_one() {
        let transcript = VecModel::<MessageData>::default();
        asked(&transcript, "book the flight");
        let mut hermes = Reply::new(Bubble::open(&transcript, &speaker("Hermes Agent")));
        assert_eq!(hermes.take(&transcript, "\u{23f3} Working \u{2014} 3 min".into()), Took::More);

        clear(&transcript);
        asked(&transcript, "what's on my calendar?");
        let mut mind = Reply::new(Bubble::open(&transcript, &speaker("Yantrik Mind")));
        assert_eq!(mind.bubble.row, hermes.bubble.row, "the new reply stands in the old one's row");

        assert_eq!(hermes.take(&transcript, " \u{2014} iteration 1/500, clarify".into()), Took::Gone);
        assert_eq!(mind.take(&transcript, "You have a dentist at 10.".into()), Took::More);
        assert_eq!(hermes.take(&transcript, "__REPLACE__".into()), Took::Gone);
        assert_eq!(hermes.take(&transcript, "Booked.".into()), Took::Gone);
        assert_eq!(hermes.take(&transcript, "__DONE__".into()), Took::Gone, "its end is not the new reply's end");

        assert_eq!(said(&transcript, mind.bubble), "You have a dentist at 10.");
        let bubble = transcript.row_data(mind.bubble.row).unwrap();
        assert!(bubble.is_streaming, "still the Mind's, still arriving");
        assert_eq!(bubble.mind, "Yantrik Mind");
        assert!(!hermes.bubble.here() && mind.bubble.here());
    }

    /// Within one transcript a reply's run link and `__REPLACE__` still reach its own bubble.
    #[test]
    fn a_reply_keeps_its_run_link_and_replacements_in_its_own_bubble() {
        let transcript = VecModel::<MessageData>::default();
        let mut first = Reply::new(Bubble::open(&transcript, &speaker("Hermes Agent")));
        let second = Reply::new(Bubble::open(&transcript, &speaker("Yantrik Mind")));
        first.take(&transcript, "<tool_call>".into());
        first.take(&transcript, "__REPLACE__".into());
        first.take(&transcript, "Done.".into());
        first.take(&transcript, format!("{RUN_MARK}hermes:main#4"));
        let row = transcript.row_data(first.bubble.row).unwrap();
        assert_eq!((row.content.as_str(), row.run.as_str()), ("Done.", "hermes:main#4"));
        let other = transcript.row_data(second.bubble.row).unwrap();
        assert_eq!((other.content.as_str(), other.run.as_str()), ("", ""));
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
