//! Chat wiring — on_send_message + on_lens_submit.
//!
//! Both go through `dispatch`, which asks the harness host which mind is driving before it
//! sends anything. They used to call the builtin companion directly.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use slint::{ComponentHandle, Timer};

use crate::app_context::AppContext;
use crate::bridge::{CompanionBridge, Served};
use crate::{apps, lens, streaming, App};

/// What the desktop tells a mind about where a turn came from: facts about the machine, as JSON.
///
/// A mind keeps its own clock but has no way to know where the computer is. Asked "what is the
/// weather like right now?" on a fresh install, Yantrik Mind answered for London while this
/// desktop had already worked out it was in Bentonville. These are facts about the machine,
/// never configuration for the mind — the same things the status bar shows.
pub(crate) fn desktop_context(place: &super::settings::Place) -> String {
    machine_context(place, std::env::var("HOME").ok().as_deref())
}

/// The context from its parts. `home` is the person's home directory, what `~` means in their
/// words: a mind running as its own account (#411) has another home, and cannot see this one, so
/// "save it in ~/notes" was read as its own directory and the file "not found" there.
fn machine_context(place: &super::settings::Place, home: Option<&str>) -> String {
    let mut machine = serde_json::Map::new();
    if let Some(home) = home.filter(|h| h.starts_with('/') && *h != "/") {
        machine.insert("home".into(), home.into());
    }
    if !place.city.trim().is_empty() {
        machine.insert(
            "place".into(),
            serde_json::json!({ "city": place.city, "region": place.region, "country": place.country }),
        );
    }
    if !place.timezone.trim().is_empty() {
        machine.insert("timezone".into(), place.timezone.clone().into());
    }
    serde_json::json!({ "machine": machine }).to_string()
}

/// The desktop's context with the conversation handed over in it, for a harness that said it reads
/// it there (`Attach::handover_context`): the person's words then go in `text` alone.
pub(crate) fn context_with_handover(context: &str, handover: &crate::agents::handover::Handover) -> String {
    let mut value: serde_json::Value = serde_json::from_str(context).unwrap_or_else(|_| serde_json::json!({}));
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "handover".into(),
            serde_json::json!({ "from": handover.from, "text": handover.told }),
        );
    }
    value.to_string()
}

/// The built-in's turn: relayed to the chat panel, and counted when it ends answered.
///
/// The built-in used to score its own turns from inside its handlers — and those handlers also
/// run for the startup brief, EXECUTE urges and "Reflect naturally", prompts the machine sends
/// itself, so the store on VM 520 held `interaction` rows at times nobody was typing, each one
/// telling the proactive engine the person had just been around. The harness branch of
/// `dispatch` below already counts at the right point: after an answered turn, from the person's
/// own words. This is the same point for the built-in. Nothing that reaches the companion any
/// other way — `CompanionHandle::ask`, the proactive stream, the worker's own prompts — counts.
fn builtin_turn(
    ui_weak: &slint::Weak<App>,
    bridge: &Arc<CompanionBridge>,
    text: &str,
    handover: Option<&crate::agents::handover::Handover>,
    streams: &streaming::Streams,
    shown: bool,
) {
    let sent = match handover {
        Some(h) => crate::agents::handover::with_handover(h, text),
        None => text.to_string(),
    };
    let (answer, by_model) = bridge.send_message_reporting(sent, false);
    let (tx, rx) = crossbeam_channel::unbounded::<String>();
    let bridge = bridge.clone();
    let asked = text.to_string();
    let weak = ui_weak.clone();
    std::thread::spawn(move || {
        // Answered means the worker finished the turn (`__DONE__`) without putting its own
        // failure text in place of an answer. A `__REPLACE__` on its own is ordinary — tool
        // calls use it to strip raw XML from what was already streamed.
        let mut done = false;
        let mut failed = false;
        let mut replace_next = false;
        // What the person sees as the reply, so an empty one is not counted as an answer.
        let mut said = String::new();
        while let Ok(token) = answer.recv() {
            let end = token == "__DONE__";
            if token == "__REPLACE__" {
                replace_next = true;
            } else if !end {
                // The model thinking and giving no answer is a failed turn too: nobody was
                // talked to.
                if replace_next
                    && (token == crate::bridge::TURN_FAILED_REPLY || token == yantrik_ml::THOUGHT_WITHOUT_ANSWER)
                {
                    failed = true;
                }
                if replace_next {
                    said.clear();
                }
                said.push_str(&token);
                replace_next = false;
            }
            if tx.send(token).is_err() {
                return;
            }
            if end {
                done = true;
                break;
            }
        }
        if done && !failed {
            bridge.score_conversation_turn(asked);
        }
        // The first run's first success is a model's words, not the not-set-up reply.
        // A guard warning or a router reply is not a model's either (`without_model`), and
        // only the not-set-up reply itself means nothing is set up.
        // A model that thought and gave no answer is told as that, with what to do about it.
        let outcome = match by_model.try_recv() {
            Ok(Served::Model) if done && !failed && !said.trim().is_empty() => Ok(()),
            Ok(Served::NoModel) if said.trim() == yantrik_ml::not_set_up::NOT_SET_UP.trim() => {
                Err(super::onboarding::NOT_SET_UP_SENTENCE.to_string())
            }
            // The streamed sentence is the failure in words: no answer after thinking, or the
            // provider's own reason for refusing the request.
            Ok(Served::ModelFailed(_)) if !said.trim().is_empty() => Err(said.trim().to_string()),
            Ok(Served::ModelFailed(_)) => Err(yantrik_ml::THOUGHT_WITHOUT_ANSWER.to_string()),
            Ok(Served::Unreachable(why)) => Err(super::onboarding::unreachable_sentence(&why)),
            _ => Err(super::onboarding::NO_ANSWER_SENTENCE.to_string()),
        };
        super::onboarding::first_chat_ended(&weak, outcome);
    });
    let asked = (!shown).then_some(text);
    match handover {
        Some(h) => {
            streaming::say(ui_weak, asked, "desktop", &switched_to("Yantrik Companion", h));
            streaming::stream_answer(ui_weak.clone(), rx, streams);
        }
        None => stream_reply(ui_weak, rx, asked, streams),
    }
}

/// The reply in a bubble of its own, under the person's words: put in the conversation first when
/// `asked`, or already there (a queued message, whose bubble became the person's own).
fn stream_reply(ui_weak: &slint::Weak<App>, rx: crossbeam_channel::Receiver<String>, asked: Option<&str>, streams: &streaming::Streams) {
    match asked {
        Some(text) => streaming::stream_into(ui_weak.clone(), rx, text, streams),
        None => streaming::stream_answer(ui_weak.clone(), rx, streams),
    }
}

/// The desktop's line when the conversation changes hands: who answers now, and that it knows.
fn switched_to(mind: &str, handover: &crate::agents::handover::Handover) -> String {
    format!("Now talking to {mind}. It has been told what you and {} said, so you can carry on.", handover.from)
}

/// Send what the person typed to whichever mind is actually driving.
///
/// This is the join between the body and the mind, and until now it did not exist. `chat.rs`
/// called `bridge.send_message` directly — the builtin companion — so `use_harness` switched a
/// name in the machine rail and a chip in the status bar while every word still went to the
/// builtin. An attached harness could appear in the picker, be chosen, be shown as active, and
/// never be asked anything.
///
/// The host already knew how to do this: `Host::send` routes to the builtin or queues for the
/// attached harness and hands back a stream either way. It was simply never called.
///
/// `shown`: the person's words are already in the conversation (a queued message, now sent);
/// otherwise they are put there first (`words`).
fn dispatch(
    ui_weak: &slint::Weak<App>,
    bridge: &Arc<CompanionBridge>,
    text: &str,
    streams: &streaming::Streams,
    shown: bool,
) {
    let words = (!shown).then_some(text);
    // A person just said something — to whichever mind. The Synthesis Gate, which decides
    // whether the built-in companion may speak unprompted, used to read a timestamp bumped only
    // inside the companion's own message arm, so a conversation with a harness mind looked like
    // an idle user and unprompted messages landed in the middle of somebody else's answer.
    // This is the one join every typed message passes through, so this is where the clock goes.
    super::notifications::note_user_message();
    super::onboarding::first_chat_asked(ui_weak, text);

    // A word to a recipe — the answer to its question, or "pause the digest" — goes where the
    // Recipes screen's presses go, whichever mind is answering: the worker's `recipe_view::apply`.
    // Read from the published recipes, so nothing here waits on the worker. The companion's
    // interjection classifier was never wired to the chat, so only the screen could answer,
    // pause or cancel a recipe (#176).
    if let Some(word) = crate::recipes::said_in_chat(text) {
        let reply = crate::recipes::act_from_chat(bridge.handle(), word);
        stream_reply(ui_weak, reply, words, streams);
        return;
    }

    let Some(host) = super::harness::host() else {
        // No host yet (very early boot). The builtin is the only thing that could answer.
        builtin_turn(ui_weak, bridge, text, None, streams, shown);
        return;
    };

    // The conversation, handed over when the answering mind has changed since the last word
    // (#245): the new mind is told what was said and what the last one is still doing, in front
    // of the person's words, so the switch is seamless. The person's bubble shows what they typed.
    let active = host.active_id();
    let handover = (!text.trim_start().starts_with('/'))
        .then(|| crate::agents::store().read(|s| crate::agents::handover::for_turn(s, &active, crate::agents::model::now())))
        .flatten();

    // Private mode: no mind is talked to, the built-in companion included. Not a word goes to
    // one; the desktop says so under the person's own words, with the way out as the reply's
    // link (`private_mode::LEAVE_LINK`).
    if crate::private_mode::is_on() {
        let mind = host.list().into_iter().find(|e| e.id == active).map(|e| e.name).unwrap_or_else(|| active.clone());
        streaming::offer(ui_weak, words, &crate::private_mode::lens_offer(&mind), crate::private_mode::LEAVE_LINK);
        return;
    }

    // The builtin keeps its own path: it carries tool calls, the __REPLACE__ convention and the
    // job board, none of which the harness protocol has or needs.
    if active == super::harness::BUILTIN_ID {
        // The built-in reads the person's files itself: the ones handed over are named for it.
        // On Yantrik models it asks for `picked`, and the gateway applies the picked effort.
        let handed = crate::picker::turn_options(&active, "lens").map(|o| o.attachments).unwrap_or_default();
        let said = if handed.is_empty() {
            text.to_string()
        } else {
            let names: Vec<String> = handed.iter().map(|a| format!("{} ({})", a.path, crate::picker::attach::size_words(a.size))).collect();
            format!("{text}\n\n[Files I handed over: {}]", names.join(", "))
        };
        builtin_turn(ui_weak, bridge, &said, handover.as_ref(), streams, shown);
        crate::picker::files_sent(ui_weak);
        return;
    }

    // A message to a mind already at work no longer reaches here from the Lens: the composer
    // queues it until the turn ends (`lens_queue`), and the reply's timeline says what the mind is
    // doing. The desktop's "{mind} is working on …" reply went with that (#246's `Progress::told`,
    // which the hand-over still uses): nothing could tell a status question from any other
    // message, so every one got the status and was then held unseen.

    // An attached harness answers in Chunks. Adapt them to the token protocol the pump already
    // speaks, on a thread, because `Answer` is a blocking std channel and this is the UI thread.
    // A harness that reads the hand-over from the context gets the person's words alone; any other
    // gets it in front of them, as before.
    let context = desktop_context(&super::settings::place());
    let (sent, context) = match &handover {
        Some(h) if host.reads_handover(&active) => (text.to_string(), context_with_handover(&context, h)),
        Some(h) => (crate::agents::handover::with_handover(h, text), context),
        None => (text.to_string(), context),
    };
    // What the person picked — model, effort, the files they handed over — goes with the turn.
    let mut turn = yantrik_harness::Turn::new(sent).with_context(context).with_origin(yantrik_harness::protocol::Origin::desk());
    if let Some(options) = crate::picker::turn_options(&active, "lens") {
        turn = turn.with_options(options);
    }
    crate::picker::files_sent(ui_weak);
    let answer = host.send(turn);
    // The same turn, recorded as this mind's agent on the Agents screen; the answer passes through.
    let answer = crate::agents::feed::lens_turn(&host.active_id(), text, answer);
    let run_of = crate::agents::feed::main_agent(&host.active_id());
    let (tx, rx) = crossbeam_channel::unbounded::<String>();
    let bridge = bridge.clone();
    let asked = text.to_string();
    let weak = ui_weak.clone();
    std::thread::spawn(move || {
        // Whether the mind answered, as opposed to giving up. A turn that ended in an error is
        // not a conversation: the person spoke and was not talked to.
        let mut answered = true;
        // Whether any words came back at all: the first run's first success needs some.
        let mut said = false;
        // Whether the reply has been told which run it started. The link goes on as soon as the
        // first call is on record, not when the answer ends, so the reply names its run (in
        // `describe shell`, and for the Agents screen) while the work it did is still going.
        let mut marked = false;
        // A closed channel is the end of the turn — that is the protocol, and it is why this
        // loop ends on recv() failing rather than on a sentinel.
        while let Ok(chunk) = answer.recv() {
            let sent = match chunk {
                yantrik_harness::Chunk::Text(t) => {
                    said |= !t.trim().is_empty();
                    tx.send(t)
                }
                // Said, not swallowed. A stream that simply stopped would look identical to a
                // harness that had finished, and the person would be left with half an answer
                // and no reason.
                yantrik_harness::Chunk::Failed(why) => {
                    answered = false;
                    tx.send("__REPLACE__".to_string()).and_then(|_| tx.send(why))
                }
                // What the agent is doing — a tool call's card, its output, its thinking. The
                // chat panel draws text; the calls still show here as the trail line the harness
                // writes beside each event, and the Agents view is where the cards are drawn.
                yantrik_harness::Chunk::Event(_) => {
                    // `lens_turn` copies each chunk into the store before it forwards it, so a
                    // call that has just arrived here is already on record.
                    if !marked {
                        if let Some(run) = crate::agents::feed::chat_run(&run_of) {
                            marked = true;
                            let _ = tx.send(format!("{}{run}", crate::streaming::RUN_MARK));
                        }
                    }
                    Ok(())
                }
            };
            if sent.is_err() {
                return;
            }
        }
        // A reply that did work is told which run it was (the chat is the conversation; the run is
        // agent work), if the first call did not already say so. Named before the end so the pump
        // puts it on this reply.
        if !marked {
            if let Some(run) = crate::agents::feed::chat_run(&run_of) {
                let _ = tx.send(format!("{}{run}", crate::streaming::RUN_MARK));
            }
        }
        let _ = tx.send("__DONE__".to_string());
        // The bond is the person's relationship with the desktop, whichever mind answers — the
        // Bond screen and `describe shell` present it as such. But only the built-in ever
        // scored a turn, from inside its own handler, so a machine whose mind was Hermes said
        // "Stranger, 0.0" after forty minutes of talking. This is where a harness's answer
        // ends, so this is where its turn counts — as `builtin_turn` does for the built-in.
        // Acts on the control surface are not scored: those are the mind working, not the
        // person talking.
        if answered {
            bridge.score_conversation_turn(asked);
        }
        super::onboarding::first_chat_ended(
            &weak,
            if answered && said { Ok(()) } else { Err(super::onboarding::NO_ANSWER_SENTENCE.to_string()) },
        );
    });
    let mind = host.list().into_iter().find(|e| e.id == active).map(|e| e.name).unwrap_or_else(|| active.clone());
    match handover {
        Some(h) => {
            streaming::say(ui_weak, words, "desktop", &switched_to(&mind, &h));
            streaming::stream_answer(ui_weak.clone(), rx, streams);
        }
        None => stream_reply(ui_weak, rx, words, streams),
    }
}

/// What the person sent from the Lens, done as the composer said it would be (`lens_queue`): sent
/// to the mind, queued while it works, or given as the answer to the question its run waits on.
/// A command is always sent at once; one that ends the run closes its waiting question with the
/// person's reason, before the run's end would close it as merely "stopped asking".
fn submit(ui_weak: &slint::Weak<App>, bridge: &Arc<CompanionBridge>, text: &str, streams: &streaming::Streams) {
    let composer = super::lens_queue::Composer::active();
    match composer.enter(text) {
        super::lens_queue::Enter::Send => {
            if let (Some(why), Some(agent)) = (super::lens_queue::closes_questions(text), composer.agent.as_ref()) {
                super::lens_queue::close_questions(agent, why);
            }
            dispatch(ui_weak, bridge, text, streams, false);
        }
        super::lens_queue::Enter::Queue => {
            if let Some(ui) = ui_weak.upgrade() {
                super::lens_queue::hold(&ui, text, &composer);
            }
        }
        super::lens_queue::Enter::Answer(asking) => {
            super::agents::answer_typed(&asking.agent, &asking.request, text);
        }
    }
}

/// An answer the Lens was waiting for when the shell restarted, picked back up after it (#246):
/// the desktop says what is being continued, and the rest of the answer streams in under it.
pub fn resume_in_lens(
    ui_weak: &slint::Weak<App>,
    mind: &str,
    prompt: &str,
    answer: yantrik_harness::Answer,
    streams: &streaming::Streams,
) {
    let (tx, rx) = crossbeam_channel::unbounded::<String>();
    std::thread::spawn(move || {
        while let Ok(chunk) = answer.recv() {
            let sent = match chunk {
                yantrik_harness::Chunk::Text(t) => tx.send(t),
                yantrik_harness::Chunk::Failed(why) => tx.send("__REPLACE__".to_string()).and_then(|_| tx.send(why)),
                yantrik_harness::Chunk::Event(_) => Ok(()),
            };
            if sent.is_err() {
                return;
            }
        }
        let _ = tx.send("__DONE__".to_string());
    });
    streaming::say(
        ui_weak,
        None,
        "desktop",
        &format!(
            "The desktop restarted while {mind} was answering \u{201c}{}\u{201d}. It kept working; the rest of its answer follows.",
            crate::agents::progress::brief(prompt, 120)
        ),
    );
    streaming::stream_answer(ui_weak.clone(), rx, streams);
}

/// Wire on_send_message and on_lens_submit callbacks.
pub fn wire(ui: &App, ctx: &AppContext) {
    wire_send_message(ui, ctx);
    wire_lens_submit(ui, ctx);
    wire_new_chat(ui, ctx);
    wire_queue(ui, ctx);
}

/// The Lens's queued message (`lens_queue`): sent as an ordinary turn once the mind it waits for
/// has finished, looked for four times a second, and taken back by its ×.
fn wire_queue(ui: &App, ctx: &AppContext) {
    let ui_weak = ui.as_weak();
    ui.on_lens_cancel_queued({
        let ui_weak = ui_weak.clone();
        move || {
            if let Some(ui) = ui_weak.upgrade() {
                super::lens_queue::cancel(&ui);
            }
        }
    });
    let bridge = ctx.bridge.clone();
    let streams = streaming::Streams::new();
    let timer = Timer::default();
    timer.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(250), move || {
        let Some(ui) = ui_weak.upgrade() else { return };
        if let Some(text) = super::lens_queue::due(&ui) {
            dispatch(&ui_weak, &bridge, &text, &streams, true);
        }
    });
    // The timer lives as long as the shell, the idiom every wire module uses.
    std::mem::forget(timer);
}

/// New chat (#246): the Lens empties, and whichever mind is answering starts a new conversation.
/// The built-in drops its conversation history; an attached mind is sent `/new`, which every
/// harness answers itself (harnesses/lib/yantrik_harness.py `_command`, Hermes's own `/new`).
/// Nothing is forgotten: the old conversation stays in Agents → History, and memory is not the
/// conversation.
fn wire_new_chat(ui: &App, ctx: &AppContext) {
    let bridge = ctx.bridge.clone();
    let ui_weak = ui.as_weak();
    ui.on_lens_new_chat(move || {
        use slint::Model as _;
        let Some(ui) = ui_weak.upgrade() else { return };
        // `/new` ends the work in progress as well as the conversation, in every harness. A mind
        // at work is left at work: the desktop says so rather than stopping it on a press that
        // looked like housekeeping.
        if let Some(host) = super::harness::host() {
            let agent = crate::agents::feed::main_agent(&host.active_id());
            // A run that is only waiting on the person's answer is not at work: a new chat drops
            // the question with it, and the card says so.
            let busy = crate::agents::store().read(|s| {
                let a = s.agent(&agent)?;
                crate::agents::progress::of(a, crate::agents::model::now())
                    .filter(|p| p.question.is_none() || p.running.is_some())
                    .map(|p| (a.meta.mind.clone(), p.task))
            });
            if let Some((mind, task)) = busy {
                streaming::say(
                    &ui_weak,
                    None,
                    "desktop",
                    &format!(
                        "{mind} is still working on \u{201c}{}\u{201d}. A new chat would stop it, so this one \
                         carries on; send /stop first if stopping it is what you want.",
                        crate::agents::progress::brief(&task, 100)
                    ),
                );
                return;
            }
        }
        if let Some(host) = super::harness::host() {
            let agent = crate::agents::feed::main_agent(&host.active_id());
            super::lens_queue::close_questions(&agent, crate::agents::model::Question::NEW_CONVERSATION);
        }
        // A message queued for the old conversation goes with it, unsent.
        super::lens_queue::cancel(&ui);
        let messages = ui.get_messages();
        if let Some(model) = messages.as_any().downcast_ref::<slint::VecModel<crate::MessageData>>() {
            // Through `streaming`, so an answer still arriving from a mind the person switched
            // away from cannot write into the bubbles that take its row next.
            streaming::clear(model);
        }
        // Stay in the conversation: an empty one says what a desk is and offers three ways to
        // begin, which the search row it used to fall back to did not.
        ui.set_lens_chat_mode(true);
        // From here the conversation is new: the next word to any mind goes without a handover
        // of what was said before (agents/handover.rs). Set only now, past the refusal above.
        crate::agents::store().new_conversation();
        let active = super::harness::host().map(|h| h.active_id());
        // And, when this is the person's own press of +, a taint scope that has read nothing for
        // the mind that is sent /new (`taint_scope`). Not for `new_chat` on the control surface,
        // which invokes this same callback: a caller there may be a mind that passes for the
        // person, and only the person may untaint a conversation.
        if crate::taint_scope::is_press() {
            crate::taint_scope::begin_by_person(active.as_deref().unwrap_or(super::harness::BUILTIN_ID));
        }
        match active.as_deref() {
            None | Some(super::harness::BUILTIN_ID) => {
                if let Err(e) = bridge.handle().new_conversation() {
                    tracing::warn!(error = %e, "New chat: the companion was not told");
                }
            }
            Some(_) => {
                if let Some(host) = super::harness::host() {
                    // The harness answers `/new` itself; its one-line reply is not news.
                    let answer = host.send(yantrik_harness::Turn::new("/new".to_string()));
                    std::thread::spawn(move || while answer.recv().is_ok() {});
                }
            }
        }
        tracing::info!(mind = ?active, "New chat");
    });
}

/// Direct chat: send message → stream response.
fn wire_send_message(ui: &App, ctx: &AppContext) {
    let bridge = ctx.bridge.clone();
    let ui_weak = ui.as_weak();
    // One per entry point, holding the pumps for answers still arriving. Two at once is ordinary:
    // a person answers an approval while the task waiting on it keeps talking.
    let streams = streaming::Streams::new();

    ui.on_send_message(move |text| {
        let text = text.to_string();
        if text.is_empty() {
            return;
        }
        // V22: No offline guard — companion handles offline mode internally
        // via OfflineResponder (memory recall + pattern matching + templates)
        submit(&ui_weak, &bridge, &text, &streams);
    });
}

/// Lens submit: try app launch first, fall back to AI streaming.
fn wire_lens_submit(ui: &App, ctx: &AppContext) {
    let bridge = ctx.bridge.clone();
    let ui_weak = ui.as_weak();
    let catalogue = ctx.installed_apps.clone();
    // One per entry point, holding the pumps for answers still arriving. Two at once is ordinary:
    // a person answers an approval while the task waiting on it keeps talking.
    let streams = streaming::Streams::new();

    ui.on_lens_submit(move |query| {
        let query = query.to_string();
        if query.is_empty() {
            return;
        }

        tracing::info!(query = %query, "Lens submit");

        let lower = query.to_lowercase();

        // Check installed .desktop apps first
        // Bound, not inlined: `get()` hands back an Arc snapshot, and the search borrows
        // from it, so it has to outlive the call.
        let installed = catalogue.get();
        let app_matches = apps::search(&lower, &installed);
        if let Some(entry) = app_matches.first() {
            // The Exec line read per the spec: field codes survive parsing now, and a launch
            // with no file removes them where they stand instead of passing them as words (#304).
            let argv = apps::exec_argv(&entry.exec, None);
            if let Some((bin, args)) = argv.split_first() {
                tracing::info!(exec = %entry.exec, name = %entry.name, "Launching app from Lens");
                match std::process::Command::new(bin).args(args).spawn() {
                    Ok(_) => tracing::info!(name = %entry.name, "App started"),
                    Err(e) => tracing::error!(name = %entry.name, error = %e, "Failed to launch"),
                }
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_lens_open(false);
                }
                return;
            }
        }

        // Fallback: hardcoded KNOWN_APPS
        for (app_id, cmd, _) in lens::KNOWN_APPS {
            if lower.contains(&format!("open {}", app_id))
                || lower.contains(app_id)
                || lower.contains(cmd)
            {
                tracing::info!(cmd, "Launching app from Lens (fallback)");
                match std::process::Command::new(cmd).spawn() {
                    Ok(_) => tracing::info!(cmd, "App started"),
                    Err(e) => tracing::error!(cmd, error = %e, "Failed to launch app"),
                }
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_lens_open(false);
                }
                return;
            }
        }

        // Not a launch and not a known app, so it is a question — and a question goes to
        // whichever mind is answering, the same as one typed into chat. This line called the
        // builtin directly, which meant the Lens (the primary way anyone talks to this desktop)
        // ignored the mind picker even after chat stopped doing so.
        submit(&ui_weak, &bridge, &query, &streams);
    });
}

#[cfg(test)]
mod handover_context_tests {
    use super::*;

    /// A harness that reads the hand-over from the context gets it there, beside what the desktop
    /// already says about itself, with the contract strings a mind may still key on unchanged.
    #[test]
    fn the_handover_rides_in_the_context_beside_the_machine() {
        let handover = crate::agents::handover::Handover {
            from: "Hermes Agent".into(),
            told: "[From the desktop: you are taking this conversation over from Hermes Agent. The person \
                   expects you to know what was said; it was:\n- The person: build the town model\nCarry on from here.]"
                .into(),
        };
        let context = r#"{"machine":{"timezone":"America/Chicago"}}"#;
        let with: serde_json::Value = serde_json::from_str(&context_with_handover(context, &handover)).unwrap();
        assert_eq!(with["machine"]["timezone"], "America/Chicago", "what was there stays");
        assert_eq!(with["handover"]["from"], "Hermes Agent");
        let text = with["handover"]["text"].as_str().unwrap();
        assert!(text.starts_with("[From the desktop:") && text.ends_with("Carry on from here.]"), "{text}");

        let broken: serde_json::Value = serde_json::from_str(&context_with_handover("not json", &handover)).unwrap();
        assert_eq!(broken["handover"]["from"], "Hermes Agent", "a context that did not parse still carries it");
    }
}

#[cfg(test)]
mod tests {
    /// This file, read as text.
    ///
    /// The bug this guards was not a wrong line — it was a MISSING one, and no type, signature or
    /// call graph could have noticed. `on_send_message` called the builtin companion directly and
    /// compiled perfectly; the harness host, the socket, the picker and the status-bar chip were
    /// all built, all correct, and simply never consulted. Selecting a mind changed a label.
    ///
    /// Nothing observable was broken either: the desktop answered every question, because the
    /// builtin always answers. It took attaching a harness, watching it be chosen, and then
    /// watching its log stay empty to see it. So the property is asserted where it lives.
    const SELF_SRC: &str = include_str!("chat.rs");

    /// Everything below `dispatch` is the part of the file that decides where a message goes.
    /// Cut at the test module, or this test would read itself.
    fn callbacks() -> &'static str {
        let after = SELF_SRC
            .split_once("/// Wire on_send_message and on_lens_submit callbacks.")
            .expect("the wiring doc comment marks the end of dispatch")
            .1;
        &after[..after.find("#[cfg(test)]").unwrap_or(after.len())]
    }

    #[test]
    fn every_way_of_talking_to_this_desktop_asks_who_is_answering() {
        let body = callbacks();
        assert!(
            !body.contains("start_ai_stream") && !body.contains("builtin_turn("),
            "a callback sends straight to the builtin companion. Both ways of talking to this desktop — the chat panel and the Lens — must go through `dispatch`, which reads the harness host; otherwise choosing a mind changes a label and nothing else."
        );
        assert_eq!(
            body.matches(" submit(&ui_weak").count(),
            2,
            "there are two entry points — on_send_message and the Lens fallback — and both should reach the chosen mind through `submit`, then `dispatch`"
        );
        assert_eq!(body.matches("dispatch(").count(), 1, "and the queued message, sent when its mind finishes, goes through `dispatch` too");
        let submit = SELF_SRC.split_once("fn submit(").expect("submit").1;
        assert!(submit[..submit.find("\n}\n").unwrap()].contains("dispatch(ui_weak, bridge, text, streams, false)"), "what is sent goes through `dispatch`");
    }

    /// The builtin's own path still has to exist. `dispatch` sends to it by id rather than by
    /// being the default, so this is the one place the name may appear.
    #[test]
    fn the_builtin_is_chosen_by_name_not_by_default() {
        let before = SELF_SRC
            .split_once("/// Wire on_send_message and on_lens_submit callbacks.")
            .expect("the wiring doc comment marks the end of dispatch")
            .0;
        assert!(
            before.contains("harness::BUILTIN_ID"),
            "`dispatch` decides between the builtin and an attached harness by comparing the active id; without that comparison the builtin is simply whatever happens to run"
        );
    }
    /// The bond counts conversation with the desktop, whichever mind answered.
    ///
    /// The built-in scores its own turns inside the companion. A harness's turn is relayed
    /// from the host to the panel by the thread below `host.send(`, and nothing else in the
    /// shell sees it end — so if this branch does not score it, nothing does, and a machine
    /// whose mind is Hermes stays "Stranger, 0.0" however long the person talks.
    #[test]
    fn a_turn_a_harness_answered_counts_toward_the_bond() {
        let dispatch = SELF_SRC
            .split_once("/// Wire on_send_message and on_lens_submit callbacks.")
            .expect("the wiring doc comment marks the end of dispatch")
            .0;
        let harness_branch = dispatch
            .split_once("host.send(")
            .expect("dispatch sends a harness its turn through the host")
            .1;
        assert!(
            harness_branch.contains("bridge.score_conversation_turn("),
            "the harness branch of `dispatch` relays the answer and must also score the turn; the built-in scores its own, and nothing else sees a harness's answer end"
        );
        assert!(
            harness_branch.contains("if answered"),
            "a turn the harness failed is not a conversation and must not count"
        );
    }

    /// The built-in's turn counts at the same point, from the same text — not inside its own
    /// handlers, which also run for the startup brief, EXECUTE urges and "Reflect naturally".
    /// The store on VM 520 held `interaction` rows at times nobody was typing.
    #[test]
    fn a_turn_the_builtin_answered_counts_at_the_same_point() {
        let above_wiring = SELF_SRC
            .split_once("/// Wire on_send_message and on_lens_submit callbacks.")
            .expect("the wiring doc comment marks the end of dispatch")
            .0;
        let relay = above_wiring
            .split_once("fn builtin_turn(")
            .expect("the built-in's turn has its own relay, `builtin_turn`, beside dispatch")
            .1;
        let relay = &relay[..relay.find("fn dispatch(").unwrap_or(relay.len())];
        assert!(
            relay.contains("bridge.score_conversation_turn(asked)"),
            "the built-in's relay must score the turn when it ends, from the text the person typed; the companion's own handlers no longer do, because they also run for prompts the machine sends itself"
        );
        assert!(
            relay.contains("TURN_FAILED_REPLY") && relay.contains("if done && !failed"),
            "a turn the worker failed is not a conversation and must not count — the rule the harness path applies to `Chunk::Failed`"
        );
        let dispatch = above_wiring
            .split_once("fn dispatch(")
            .expect("dispatch is below the relay")
            .1;
        assert_eq!(
            dispatch.matches("builtin_turn(").count(),
            2,
            "both roads to the built-in — no host yet, and the built-in chosen by id — go through the relay that counts the turn"
        );
        assert!(
            !dispatch.contains("start_ai_stream"),
            "a road to the built-in that bypasses the relay is a turn that never counts"
        );
    }

    /// A word to a recipe — its answer, or "pause the digest" — goes where the Recipes screen's
    /// presses go, whichever mind is answering, and before any mind is asked (#176). The
    /// companion's interjection classifier was never wired to the chat, so only the screen could
    /// answer, pause or cancel a recipe.
    #[test]
    fn a_word_to_a_recipe_goes_where_the_recipes_screen_s_presses_go() {
        let dispatch = SELF_SRC
            .split_once("fn dispatch(")
            .expect("dispatch")
            .1
            .split_once("/// Wire on_send_message and on_lens_submit callbacks.")
            .expect("the wiring doc comment marks the end of dispatch")
            .0;
        let recipes = dispatch
            .find("crate::recipes::said_in_chat(")
            .expect("dispatch asks the published recipes whether the line speaks to one");
        let mind = dispatch.find("harness::host()").expect("dispatch reads the harness host");
        assert!(recipes < mind, "a word to a recipe is taken before any mind is asked");
        assert!(dispatch.contains("crate::recipes::act_from_chat("), "and it acts through the screen's own path");
    }

    #[test]
    fn a_turn_tells_the_mind_where_the_machine_is_and_nothing_more() {
        let place = crate::wire::settings::Place {
            city: "Bentonville".into(),
            region: "Arkansas".into(),
            country: "US".into(),
            lat: 36.37,
            lon: -94.2,
            timezone: "America/Chicago".into(),
            source: "detected".into(),
        };
        let v: serde_json::Value =
            serde_json::from_str(&super::machine_context(&place, Some("/home/ann"))).unwrap();
        assert_eq!(v["machine"]["place"]["city"], "Bentonville");
        assert_eq!(v["machine"]["timezone"], "America/Chicago");
        assert_eq!(v["machine"]["home"], "/home/ann", "what ~ means in the person's words");
        // Coordinates and how the place was found stay on the machine.
        assert!(v["machine"].get("lat").is_none() && v["machine"]["place"].get("lat").is_none());
        assert!(v["machine"].get("source").is_none());

        let unknown: serde_json::Value =
            serde_json::from_str(&super::machine_context(&Default::default(), None)).unwrap();
        assert_eq!(unknown, serde_json::json!({ "machine": {} }));
        for not_a_home in ["", "relative/dir", "/"] {
            let v: serde_json::Value =
                serde_json::from_str(&super::machine_context(&Default::default(), Some(not_a_home))).unwrap();
            assert!(v["machine"].get("home").is_none(), "{not_a_home:?}");
        }
    }

}
