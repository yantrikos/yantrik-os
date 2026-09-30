//! The chat, read in one call, for a client that is not the Lens: the Yantrik terminal.
//!
//! The Lens draws the conversation from the shell's own message list and a mind's calls from the
//! agents store. A terminal client needs the same two things, several times a second, and
//! `describe shell` is the wrong way to get them: it carries every agent, the catalogue and the
//! app list (~30 KB), and it is built on the UI thread. `chat_view` is the chat and nothing else:
//! the messages since the one the caller last had, the mind that is answering, the calls, approvals
//! and questions of its current turn, the mode, and what is open in Mind View — with a revision, so
//! a caller polling an idle chat redraws nothing.
//!
//! `safe`: it reads what the person is already looking at and changes nothing. Approvals are shown
//! as waiting, never answerable here; they are answered on the desktop's card, by a person.
//!
//! The person's, and no agent's. It gives the whole conversation in full, and the answering mind's
//! calls and approvals — far more than `describe shell`'s six clipped messages — so a caller the
//! desktop takes for an agent (a token, or the mind account's uid) is refused. An agent reads its
//! own session with `read_agent`. The lock rule on this surface already refuses it while locked.

use std::hash::{Hash, Hasher};

use slint::ComponentHandle;
use yantrik_app_runtime::control::{Action, App as ControlSurface, Param};

use crate::agents::{self, CallState};
use crate::agents::model::{Item, Turn};
use crate::App;

/// The newest messages sent when the caller names none to start from: a screenful.
const DEFAULT_TAIL: usize = 30;
/// A call's arguments as one line, for a card. The whole value is in the Agents pane.
const ARGS_CLIP: usize = 140;

pub fn actions(surface: ControlSurface, ui: &App) -> ControlSurface {
    let weak = ui.as_weak();
    let new_weak = ui.as_weak();
    surface.action(
        Action::new(
            "chat_view",
            "The chat in one read, for a client that is not the Lens: messages since `since` (in full), the mind answering, the calls, approvals and questions of its current turn, the mode, and what is in Mind View, with a revision to poll by",
        )
        .risk("safe")
        .arg(
            Param::number("since")
                .optional()
                .describe("The first message index wanted; the newest 30 when left out"),
        ),
        move |args| {
            persons_only("chat_view")?;
            let ui = weak.upgrade().ok_or_else(|| "the shell is gone".to_string())?;
            let since = args["since"].as_u64().or_else(|| args["since"].as_f64().map(|f| f as u64));
            Ok(chat_view(&ui, since.map(|s| s as usize)))
        },
    )
    .action(
        // The Lens's "New chat", for a client that is not the Lens: the conversation starts again,
        // for whichever mind is answering (#246). `standard`: nothing is lost — every earlier
        // conversation stays on the Agents screen's All tab, and can be carried on from there.
        Action::new("new_chat", "Start a new conversation with the mind that is answering, as the Lens's New chat does"),
        move |_args| {
            persons_only("new_chat")?;
            let ui = new_weak.upgrade().ok_or_else(|| "the shell is gone".to_string())?;
            ui.invoke_lens_new_chat();
            use slint::Model;
            Ok(serde_json::json!({ "messages": ui.get_messages().row_count() }))
        },
    )
}

/// Refuse a caller the desktop takes for an agent: these are the person's view of their own chat.
fn persons_only(action: &str) -> Result<(), String> {
    if yantrik_app_runtime::control::agent_is_calling() {
        return Err(format!(
            "{action} is the person's view of their own chat, and an agent is calling. An agent reads its own session with read_agent."
        ));
    }
    Ok(())
}

fn chat_view(ui: &App, since: Option<usize>) -> serde_json::Value {
    use slint::Model;
    let rows = ui.get_messages();
    let total = rows.row_count();
    let from = since.unwrap_or_else(|| total.saturating_sub(DEFAULT_TAIL)).min(total);
    let messages: Vec<serde_json::Value> = (from..total)
        .filter_map(|i| rows.row_data(i).map(|m| (i, m)))
        .map(|(i, m)| {
            serde_json::json!({
                "index": i,
                "role": m.role.to_string(),
                "text": m.content.to_string(),
                "streaming": m.is_streaming,
            })
        })
        .collect();

    let host = crate::wire::harness::host();
    let active = host.map(|h| h.active_id()).unwrap_or_else(|| crate::wire::harness::BUILTIN_ID.to_string());
    let mind = host
        .and_then(|h| h.list().into_iter().find(|e| e.id == active))
        .map(|e| serde_json::json!({ "id": e.id, "name": e.name, "detail": e.detail }))
        .unwrap_or_else(|| serde_json::json!({ "id": active, "name": active, "detail": null }));
    // What else could answer, for a client's mind picker (`use_harness` switches).
    let minds: Vec<serde_json::Value> = host
        .map(|h| {
            h.list()
                .into_iter()
                .map(|e| serde_json::json!({ "id": e.id, "name": e.name, "answering": e.active }))
                .collect()
        })
        .unwrap_or_default();

    let agent = agents::feed::main_agent(&active);
    let (state, turn) = agents::store().read(|s| {
        let a = s.agent(&agent);
        (
            a.map(|a| format!("{:?}", a.state).to_lowercase()),
            a.and_then(|a| a.turns.last()).map(turn_json),
        )
    });

    let mode = crate::mind_mode::snapshot()["mode"].clone();
    let mind_view = crate::mind_view::for_describe();
    let view = serde_json::json!({
        "total": total,
        "messages": messages,
        "mind": mind,
        "minds": minds,
        "agent": agent.0,
        "state": state,
        "turn": turn,
        "mode": mode,
        // By the names a person reads, as the taskbar's Mind View entry says them.
        "mind_view": { "running": mind_view["running"], "apps": crate::mind_view::app_names() },
        "waiting_on_you": crate::control_approvals::pending_for_describe()
            .as_array()
            .map(|a| a.len())
            .unwrap_or(0),
    });
    let mut out = view;
    out["revision"] = revision_of(&out).into();
    out
}

/// The current turn as a client draws it: its prompt, how it stands, and each thing in it.
fn turn_json(t: &Turn) -> serde_json::Value {
    let items: Vec<serde_json::Value> = t.items.iter().filter_map(item_json).collect();
    serde_json::json!({
        "n": t.n,
        "prompt": t.prompt,
        "started": t.started,
        "ended": t.ended,
        "ok": t.ok,
        "items": items,
    })
}

/// One thing in a turn. The mind's own text is the message itself, and its reasoning is folded
/// away in the pane, so neither is repeated here.
fn item_json(item: &Item) -> Option<serde_json::Value> {
    match item {
        Item::Card(c) => Some(serde_json::json!({
            "kind": "call",
            "name": c.name,
            "target": c.target,
            "label": call_label(&c.name, &c.args),
            "args": args_line(inner_args(&c.args), &c.preview),
            "state": call_state(&c.state),
            "summary": c.summary,
            "repeats": c.repeats,
            "seconds": c.ended.map(|e| e.saturating_sub(c.started)),
        })),
        Item::Approval(a) => Some(serde_json::json!({
            "kind": "approval",
            "what": a.what,
            "outcome": a.outcome,
        })),
        Item::Question(q) => Some(serde_json::json!({
            "kind": "question",
            "prompt": q.prompt,
            "options": q.options,
            "answered": !q.answer.is_empty(),
        })),
        Item::Note(n) => Some(serde_json::json!({ "kind": "note", "text": n })),
        Item::Text(_) | Item::Thinking(_) => None,
    }
}

/// What a call did, as a person reads it. A desktop act arrives as a bridge tool (`os_act`,
/// `mcp.yantrik-os.os_act`) whose arguments name the app and the action; the label is those —
/// `blender.add_primitive`, `describe blender`, `apps` — and any other call keeps its own name.
fn call_label(name: &str, args: &serde_json::Value) -> String {
    let tool = name.rsplit('.').next().unwrap_or(name);
    let app = args["app"].as_str().unwrap_or_default();
    match tool {
        "os_act" if !app.is_empty() => format!("{app}.{}", args["action"].as_str().unwrap_or("?")),
        "os_describe" if !app.is_empty() => format!("describe {app}"),
        "os_apps" => "apps".to_string(),
        _ => tool.to_string(),
    }
}

/// The arguments worth showing: an act's own `args`, not the envelope that named the app and action.
fn inner_args(args: &serde_json::Value) -> &serde_json::Value {
    if args.get("app").is_some() && args.get("action").is_some() {
        &args["args"]
    } else if args.get("app").is_some() && args.as_object().is_some_and(|m| m.len() == 1) {
        &serde_json::Value::Null
    } else {
        args
    }
}

fn call_state(s: &CallState) -> &'static str {
    match s {
        CallState::Running => "running",
        CallState::Ok => "ok",
        CallState::Failed => "failed",
        CallState::Interrupted => "interrupted",
        CallState::Untold => "untold",
    }
}

/// A call's arguments on one line: the harness's own preview when it gave one, otherwise the
/// arguments as `key=value`, clipped.
fn args_line(args: &serde_json::Value, preview: &str) -> String {
    let line = if !preview.trim().is_empty() {
        preview.trim().to_string()
    } else if let Some(map) = args.as_object() {
        map.iter()
            .map(|(k, v)| match v {
                serde_json::Value::String(s) => format!("{k}={s}"),
                other => format!("{k}={other}"),
            })
            .collect::<Vec<_>>()
            .join(" ")
    } else if args.is_null() {
        String::new()
    } else {
        args.to_string()
    };
    let flat = line.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > ARGS_CLIP {
        format!("{}…", flat.chars().take(ARGS_CLIP).collect::<String>())
    } else {
        flat
    }
}

/// A hash of everything a client draws, so an idle chat polled twice a second answers the same
/// revision and nothing is redrawn.
fn revision_of(view: &serde_json::Value) -> String {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    view.to_string().hash(&mut h);
    format!("{:016x}", h.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_calls_arguments_are_one_clipped_line() {
        let args = serde_json::json!({"kind": "monkey", "location": "0,0,2", "name": "Suzanne"});
        assert_eq!(args_line(&args, ""), "kind=monkey location=0,0,2 name=Suzanne");
        assert_eq!(args_line(&args, "  add a monkey \n"), "add a monkey");
        assert_eq!(args_line(&serde_json::Value::Null, ""), "");
        let long = serde_json::json!({ "text": "x".repeat(400) });
        let line = args_line(&long, "");
        assert!(line.ends_with('…') && line.chars().count() == ARGS_CLIP + 1, "{line}");
        let multiline = serde_json::json!({ "code": "a\n  b\n\nc" });
        assert_eq!(args_line(&multiline, ""), "code=a b c");
    }

    #[test]
    fn a_desktop_act_is_labelled_by_its_app_and_action() {
        let act = serde_json::json!({"app": "blender", "action": "add_primitive", "args": {"kind": "monkey"}});
        assert_eq!(call_label("mcp.yantrik-os.os_act", &act), "blender.add_primitive");
        assert_eq!(args_line(inner_args(&act), ""), "kind=monkey");
        let look = serde_json::json!({"app": "blender"});
        assert_eq!(call_label("os_describe", &look), "describe blender");
        assert_eq!(args_line(inner_args(&look), ""), "");
        assert_eq!(call_label("os_apps", &serde_json::json!({})), "apps");
        assert_eq!(call_label("terminal", &serde_json::json!({"command": "ls"})), "terminal");
        assert_eq!(args_line(inner_args(&serde_json::json!({"command": "ls"})), ""), "command=ls");
    }

    #[test]
    fn the_same_view_has_the_same_revision() {
        let a = serde_json::json!({"messages": [], "state": "idle"});
        assert_eq!(revision_of(&a), revision_of(&a.clone()));
        assert_ne!(revision_of(&a), revision_of(&serde_json::json!({"messages": [], "state": "done"})));
    }
}
