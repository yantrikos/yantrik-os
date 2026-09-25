//! Chat wiring — on_send_message + on_lens_submit.
//!
//! Both go through `dispatch`, which asks the harness host which mind is driving before it
//! sends anything. They used to call the builtin companion directly.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use slint::{ComponentHandle, Timer};

use crate::app_context::AppContext;
use crate::bridge::CompanionBridge;
use crate::{apps, lens, streaming, App};

/// What the desktop tells a mind about where a turn came from: facts about the machine, as JSON.
///
/// A mind keeps its own clock but has no way to know where the computer is. Asked "what is the
/// weather like right now?" on a fresh install, Yantrik Mind answered for London while this
/// desktop had already worked out it was in Bentonville. These are facts about the machine,
/// never configuration for the mind — the same things the status bar shows.
pub(crate) fn desktop_context(place: &super::settings::Place) -> String {
    let mut machine = serde_json::Map::new();
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
    streams: &streaming::Streams,
) {
    let answer = bridge.send_message(text.to_string());
    let (tx, rx) = crossbeam_channel::unbounded::<String>();
    let bridge = bridge.clone();
    let asked = text.to_string();
    std::thread::spawn(move || {
        // Answered means the worker finished the turn (`__DONE__`) without putting its own
        // failure text in place of an answer. A `__REPLACE__` on its own is ordinary — tool
        // calls use it to strip raw XML from what was already streamed.
        let mut done = false;
        let mut failed = false;
        let mut replace_next = false;
        while let Ok(token) = answer.recv() {
            let end = token == "__DONE__";
            if token == "__REPLACE__" {
                replace_next = true;
            } else if !end {
                if replace_next && token == crate::bridge::TURN_FAILED_REPLY {
                    failed = true;
                }
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
    });
    streaming::stream_into(ui_weak.clone(), rx, text, streams);
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
fn dispatch(
    ui_weak: &slint::Weak<App>,
    bridge: &Arc<CompanionBridge>,
    text: &str,
    streams: &streaming::Streams,
) {
    // A person just said something — to whichever mind. The Synthesis Gate, which decides
    // whether the built-in companion may speak unprompted, used to read a timestamp bumped only
    // inside the companion's own message arm, so a conversation with a harness mind looked like
    // an idle user and unprompted messages landed in the middle of somebody else's answer.
    // This is the one join every typed message passes through, so this is where the clock goes.
    super::notifications::note_user_message();

    // A word to a recipe — the answer to its question, or "pause the digest" — goes where the
    // Recipes screen's presses go, whichever mind is answering: the worker's `recipe_view::apply`.
    // Read from the published recipes, so nothing here waits on the worker. The companion's
    // interjection classifier was never wired to the chat, so only the screen could answer,
    // pause or cancel a recipe (#176).
    if let Some(word) = crate::recipes::said_in_chat(text) {
        let reply = crate::recipes::act_from_chat(bridge.handle(), word);
        streaming::stream_into(ui_weak.clone(), reply, text, streams);
        return;
    }

    let Some(host) = super::harness::host() else {
        // No host yet (very early boot). The builtin is the only thing that could answer.
        builtin_turn(ui_weak, bridge, text, streams);
        return;
    };

    // The builtin keeps its own path: it carries tool calls, the __REPLACE__ convention and the
    // job board, none of which the harness protocol has or needs.
    if host.active_id() == super::harness::BUILTIN_ID {
        builtin_turn(ui_weak, bridge, text, streams);
        return;
    }

    // An attached harness answers in Chunks. Adapt them to the token protocol the pump already
    // speaks, on a thread, because `Answer` is a blocking std channel and this is the UI thread.
    let answer = host.send(
        yantrik_harness::Turn::new(text.to_string()).with_context(desktop_context(&super::settings::place())),
    );
    // The same turn, recorded as this mind's agent on the Agents screen; the answer passes through.
    let answer = crate::agents::feed::lens_turn(&host.active_id(), text, answer);
    let (tx, rx) = crossbeam_channel::unbounded::<String>();
    let bridge = bridge.clone();
    let asked = text.to_string();
    std::thread::spawn(move || {
        // Whether the mind answered, as opposed to giving up. A turn that ended in an error is
        // not a conversation: the person spoke and was not talked to.
        let mut answered = true;
        // A closed channel is the end of the turn — that is the protocol, and it is why this
        // loop ends on recv() failing rather than on a sentinel.
        while let Ok(chunk) = answer.recv() {
            let sent = match chunk {
                yantrik_harness::Chunk::Text(t) => tx.send(t),
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
                yantrik_harness::Chunk::Event(_) => Ok(()),
            };
            if sent.is_err() {
                return;
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
    });
    streaming::stream_into(ui_weak.clone(), rx, text, streams);
}

/// Wire on_send_message and on_lens_submit callbacks.
pub fn wire(ui: &App, ctx: &AppContext) {
    wire_send_message(ui, ctx);
    wire_lens_submit(ui, ctx);
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
        dispatch(&ui_weak, &bridge, &text, &streams);
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
        dispatch(&ui_weak, &bridge, &query, &streams);
    });
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
            body.matches("dispatch(").count(),
            2,
            "there are two entry points — on_send_message and the Lens fallback — and both should reach the chosen mind through `dispatch`"
        );
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
        let v: serde_json::Value = serde_json::from_str(&super::desktop_context(&place)).unwrap();
        assert_eq!(v["machine"]["place"]["city"], "Bentonville");
        assert_eq!(v["machine"]["timezone"], "America/Chicago");
        // Coordinates and how the place was found stay on the machine.
        assert!(v["machine"].get("lat").is_none() && v["machine"]["place"].get("lat").is_none());
        assert!(v["machine"].get("source").is_none());

        let unknown: serde_json::Value =
            serde_json::from_str(&super::desktop_context(&Default::default())).unwrap();
        assert_eq!(unknown, serde_json::json!({ "machine": {} }));
    }

}
