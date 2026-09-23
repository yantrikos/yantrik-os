//! Agents on the shell's surface: `new_agent`, `send_to_agent`, `stop_agent`, `read_agent` — how a
//! mind hands work to another agent — and `show_agent`, which puts one on the person's screen.
//!
//! See `design/agents-workspace-2026-09-23.md`, decision 1 ("A mind can spin up agents too") and
//! the work table's row 4.
//!
//! # Who is asking
//!
//! Never an argument. The caller's own agent comes from the agent token that rides beside `args`
//! (`control::agent_token()`), checked against the kernel's account of the caller by the same
//! resolver the agent terminal uses (`control_agent_terminal::calling_agent`). A call with no
//! token is the person's own `yos act`, or a caller that runs as no agent — treated as the person,
//! as every door on this socket treats it. A call whose token is not believed is refused: a caller
//! that presented a token is not the person.
//!
//! # The rules a mind meets
//!
//! - `new_agent` is **sensitive**: in `ask` mode the person sees a card, in the asking agent's
//!   pane. An agent another agent started cannot start agents of its own (**depth one**); an agent
//!   holds at most [`MAX_CHILDREN`] live children; the host's global cap still applies. A child
//!   starts with **no grants**: nothing of its parent's goes with it (`launch::start_on`), and a
//!   grant asked for one agent is not spent by another (`control_approvals::grant_belongs`).
//! - `send_to_agent` and `stop_agent` are **standard**, and an agent may use them only on the
//!   agents it started. The person may use them on any agent.
//! - `read_agent` is **safe**. An agent may read itself and the agents it started.
//! - `show_agent` is **safe**: it puts a pane on screen, as `show_screen` does.
//!
//! # Handing work to a role
//!
//! `hand_off {role, task, context?, wait_seconds?}` starts a role from the agent catalog
//! (`agents::catalog`, design/desk-and-mind-2026-09-23.md section 5) on its first attached mind
//! that can give it a conversation of its own, with the role's brief, the task and the context as
//! its first turn. It is gated like `new_agent` — **sensitive**, the same depth-one and
//! three-children rules through [`may_start_child`], the child starts with nothing of its parent's
//! — and the role's **reach caps it further**: before its first turn is sent the agent is held to
//! the role's surfaces and ceiling on every door (`agents::reaches`), so an act outside them is
//! refused whoever the door is. An agent held to a reach cannot start a plain agent (which has
//! none) and cannot hand work to a role whose ceiling is above its own. With `wait_seconds` the
//! answer waits for the role's first turn to end, off the UI thread, and hands back what it said.
//!
//! # A recipe handing work to a role
//!
//! A formation's Agent steps (design section 6) come here too, through [`RecipeHands`] — the hook
//! the companion's recipe executor is given — and go through the same [`hand_off_as`]. The
//! hand-off is from the recipe, not from the person: the agent's row and its approval cards say
//! "Council recipe → Reviewer". It is still started only for a run the person allowed (the Recipes
//! screen's Start, or `shell.run_recipe`, graded sensitive); it is still held by its role's reach;
//! and it is a child — an agent a recipe started cannot start agents of its own. A recipe has at
//! most [`MAX_CHILDREN`] of its agents live at once: past that, the step waits for one to answer.
//!
//! What the person agreed to, and nothing else. A run someone started at the desk carries the
//! digest of each role's definition as it was then ([`catalog::Role::digest`]); a role whose
//! definition has changed since — a file in ~/.config/yantrik/agents replaced it — or one the run
//! did not name then is refused, with a sentence. A run nobody started at the desk (a trigger, a
//! timer) asks the person on a card naming the recipe and the role before any role above `safe`;
//! a card denied or left to expire fails the step. No place under the desktop's cap, or under the
//! asking agent's, queues the start rather than racing the person's own hand-offs: the step waits,
//! needing the person. Every agent a recipe starts without a card of its own is written to the
//! record of unasked actions, under the recipe's name.

use std::time::{Duration, Instant};

use serde_json::{json, Value};
use slint::ComponentHandle;
use yantrik_app_runtime::control::{self, Action, App as ControlSurface, Param};
use yantrik_harness::Host;
use yantrik_ipc_transport::gate;

use yantrik_companion::recipe_executor::{AgentCall, AgentHook, AgentPoll, AgentRefusal, AgentStarted};

use crate::agents::catalog::{self, Catalog};
use crate::agents::model::{Agent, Item, Turn};
use crate::agents::{self, launch, reaches, AgentId, RecipeOrigin, Store};
use crate::App;

/// How many live agents one agent may have started (design decision 1).
pub const MAX_CHILDREN: usize = 3;

/// The longest `hand_off` waits for a role's answer, as `agent_run` waits for a command.
pub const HAND_OFF_WAIT_MOST: u64 = 600;

/// The most `context` a hand-off carries into the role's first turn.
pub const CONTEXT_MOST_BYTES: usize = 32 * 1024;

/// The most of a role's answer `hand_off` hands back; `read_agent` has all of it.
const ANSWER_MOST_BYTES: usize = 32 * 1024;

/// How many turns `read_agent` gives when not told, and the most it gives.
const READ_TURNS: usize = 3;
const READ_TURNS_MOST: usize = 20;

/// Who a call is for.
#[derive(Clone, Debug, PartialEq)]
pub enum Caller {
    /// No agent token came with it: the person's own `yos act`, or a caller that runs as no agent.
    NoAgent,
    /// The agent its token named, checked against the caller's process.
    Agent(AgentId),
}

/// The caller of the call being dispatched. Read on the handler's thread, inside the dispatch.
pub(crate) fn caller() -> Result<Caller, String> {
    match crate::control_agent_terminal::calling_agent() {
        None => Ok(Caller::NoAgent),
        Some(Ok(agent)) => Ok(Caller::Agent(agent)),
        Some(Err(why)) => Err(why),
    }
}

fn host() -> Result<&'static Host, String> {
    crate::wire::harness::host().ok_or_else(|| "the harness host is not running yet".to_string())
}

fn text(args: &Value, name: &str) -> String {
    args.get(name).and_then(Value::as_str).map(str::trim).unwrap_or_default().to_string()
}

/// An agent named by a caller: `<harness>:<conversation>`, as `describe shell` lists them.
fn agent_arg(args: &Value) -> Result<AgentId, String> {
    let named = text(args, "agent");
    match named.split_once(':') {
        Some((harness, conversation)) if !harness.is_empty() && !conversation.is_empty() => Ok(AgentId(named)),
        _ => Err(format!(
            "`agent` is `{named}`; an agent is named `<mind>:<conversation>`, e.g. `pi:c-7f3a91`, \
             as new_agent answered or `describe shell` lists under `agents`."
        )),
    }
}

/// Said once, in every description, because it is the only documentation a mind reads.
const WHO: &str = " You are the agent named by the agent token your call carries beside `args` \
                   (YANTRIK_AGENT_TOKEN in yos's environment) — never an argument; with no token, \
                   the call is the person's.";

/// The six actions as published.
fn specs() -> [Action; 6] {
    [
        // Sensitive: it starts work that runs as the person, and more of it than one call.
        Action::new(
            "new_agent",
            &format!(
                "Start another agent — a new conversation with a mind — and give it a task. It \
                 works on its own, in its own pane on the Agents screen, and its row says you \
                 started it. Answers at once with its id; `read_agent` shows how it is going, \
                 `send_to_agent` says more, `stop_agent` stops it. It starts with nothing of \
                 yours: no grants, and anything it needs allowed is asked for again, in its own \
                 pane. An agent another agent started cannot start agents; one agent holds at \
                 most {MAX_CHILDREN} running at once, and the desktop caps how many run in all.{WHO}"
            ),
        )
        .risk("sensitive")
        .arg(Param::text("mind").describe(
            "Which mind: an attached one's id as `describe shell` lists under `minds` (pi, deepseek)",
        ))
        .arg(Param::text("task").describe("What it is to do: its first prompt, in full")),
        Action::new(
            "send_to_agent",
            &format!(
                "Say more to an agent you started — its next prompt. Answers once it is sent; \
                 `read_agent` shows the answer as it comes. One turn at a time: an agent still \
                 on its last turn is not sent another.{WHO}"
            ),
        )
        .arg(Param::text("agent").describe("The agent, as new_agent answered: `<mind>:<conversation>`"))
        .arg(Param::text("text").describe("What to say to it")),
        Action::new(
            "stop_agent",
            &format!(
                "Stop an agent you started: its turn is ended, every command it is running is \
                 killed (the whole process group), any approval it is waiting on is withdrawn, \
                 and the agents it started stop with it. Its pane stays readable.{WHO}"
            ),
        )
        .arg(Param::text("agent").describe("The agent, as new_agent answered: `<mind>:<conversation>`")),
        Action::new(
            "read_agent",
            &format!(
                "Read an agent's recent turns as text: what it was asked, what it said, each \
                 call it made with how it went and the end of its output, and what the person \
                 was asked for it. You may read yourself and the agents you started.{WHO}"
            ),
        )
        .risk("safe")
        .arg(Param::text("agent").describe("The agent, as new_agent answered: `<mind>:<conversation>`"))
        .arg(
            // A count of turns: whole, as the dispatch now checks (`"3"` converts; `2.5` does not).
            Param::integer("last")
                .optional()
                .describe("How many of its latest turns. Default 3, at most 20"),
        ),
        Action::new(
            "show_agent",
            "Put one agent's pane on the person's screen: the Agents screen, with that agent \
             selected. Changes nothing about the agent.",
        )
        .risk("safe")
        .arg(Param::text("agent").describe("The agent: `<mind>:<conversation>`")),
        // Sensitive, as `new_agent` is: it starts work that runs as the person.
        Action::new(
            "hand_off",
            &format!(
                "Hand a piece of work to a role from the agent catalog — Researcher, Planner, \
                 Coder, Reviewer, Red team, Writer, Chair, Scribe, or the person's own; `describe \
                 shell` lists them under `catalog` with what each is for, what it may touch and \
                 whether a mind it runs on is attached. It starts that role's agent on its first \
                 attached mind, in its own pane, with the role's standing instructions, your task \
                 and your context as its first prompt. It can act only within the role's reach: \
                 anything else it tries is refused. Without `wait_seconds` it answers at once with \
                 the agent's id; with it, it waits up to that long for the role's answer and hands \
                 it back. It starts with nothing of yours: no grants. An agent another agent \
                 started cannot hand off; one agent holds at most {MAX_CHILDREN} running at once, \
                 and the desktop caps how many run in all.{WHO}"
            ),
        )
        .risk("sensitive")
        .arg(Param::text("role").describe(
            "The role: its id as `describe shell` lists under `catalog` (reviewer, coder, red-team …), or its name",
        ))
        .arg(Param::text("task").describe("What it is to do, in full: its first prompt, after the role's own instructions"))
        .arg(Param::text("context").optional().describe(
            "What it should read first — the change to review, the answers to weigh. At most 32 KiB",
        ))
        .arg(Param::number("wait_seconds").optional().describe(
            "Seconds to wait for its answer. Left out, the answer is its id at once. At most 600",
        )),
    ]
}

/// The six actions, for the shell's surface.
pub fn actions(surface: ControlSurface, ui: &App) -> ControlSurface {
    let [new, send, stop, read, show, hand] = specs();
    let show_ui = ui.as_weak();
    // ── Agents catalog: the shell is where every agent's reach is kept. Its own dispatch reads
    // the registry in-process, as it spends grants in-process, and it answers every other door's
    // `agent.reach` from the same place — what the harness host says is live, and the reach each
    // was started with. Nothing from an earlier run is held: those tokens are gone. And a role may
    // open the apps its reach names, resolved as `open_app` opens them.
    yantrik_ipc_transport::reach::keep_reach_with(|digest| {
        reaches::standing(crate::wire::harness::host(), digest)
    });
    yantrik_ipc_transport::reach::resolve_opened_apps_with(reaches::opened_app);
    reaches::reset();
    surface
        .action(new, |args| new_agent(host()?, &caller()?, &text(args, "mind"), &text(args, "task")))
        .action(send, |args| send_to_agent(host()?, &caller()?, &agent_arg(args)?, &text(args, "text")))
        .action(stop, |args| stop_agent(host()?, &caller()?, &agent_arg(args)?))
        .action(read, |args| read_agent(&caller()?, &agent_arg(args)?, args.get("last")))
        .action(show, move |args| {
            let agent = agent_arg(args)?;
            let ui = show_ui.upgrade().ok_or("the shell is gone")?;
            let known = agents::store().read(|s| s.agent(&agent).is_some());
            if !known {
                return Err(format!("there is no agent `{agent}` on this desktop; `describe shell` lists them under `agents`."));
            }
            ui.global::<crate::AgentsState>().invoke_show_agent(agent.0.as_str().into());
            let raised = crate::windows::raise_shell().is_ok();
            Ok(json!({ "showing": agent, "raised": raised }))
        })
        .action(hand, |args| {
            let wait = wait_arg(args)?;
            let handed = hand_off(
                host()?,
                &caller()?,
                &Catalog::load(),
                &text(args, "role"),
                &text(args, "task"),
                &text(args, "context"),
            )?;
            let Some(wait) = wait else { return Ok(handed.answer(None)) };
            // The wait is for the role's answer, which may be minutes away: off the UI thread.
            let work = move || Ok(handed.answer(Some((wait, wait_for_answer(&handed.agent, wait)))));
            control::answer_later(work).map(|()| json!({ "answering": "off the UI thread" })).or_else(|work| work())
        })
}

/// `wait_seconds`: none, or a number of seconds up to [`HAND_OFF_WAIT_MOST`]. Zero is none.
fn wait_arg(args: &Value) -> Result<Option<Duration>, String> {
    let Some(given) = args.get("wait_seconds").filter(|v| !v.is_null()) else { return Ok(None) };
    let secs = given
        .as_f64()
        .or_else(|| given.as_str().and_then(|s| s.trim().parse().ok()))
        .ok_or_else(|| "`wait_seconds` is a number of seconds.".to_string())?;
    if !(0.0..=HAND_OFF_WAIT_MOST as f64).contains(&secs) {
        return Err(format!(
            "`wait_seconds` is between 0 and {HAND_OFF_WAIT_MOST}. Left out, hand_off answers at once \
             with the agent's id, and `read_agent` shows its answer when it comes."
        ));
    }
    Ok((secs > 0.0).then(|| Duration::from_secs_f64(secs)))
}

// ── The rules ─────────────────────────────────────────────────────

/// May `parent` start another agent? Depth one, and at most [`MAX_CHILDREN`] of its own still live
/// in the host. The global cap is the host's, met when the agent is started.
pub fn may_start_child(parent: &AgentId, store: &Store, live: &[AgentId]) -> Result<(), String> {
    // A recipe's agent is a child too: the recipe handed it the work.
    if let Some(origin) = store.agent(parent).and_then(|a| a.meta.recipe.clone()) {
        return Err(format!(
            "`{parent}` was started by the {}, and an agent a recipe started cannot start agents \
             of its own: one level only. Say in your answer what else needs doing.",
            origin.label()
        ));
    }
    if let Some(grand) = store.agent(parent).and_then(|a| a.meta.parent.clone()) {
        return Err(format!(
            "`{parent}` was started by `{grand}`, and an agent another agent started cannot start \
             agents of its own: one level only. Say in your answer what else needs doing, and \
             `{grand}` can start it."
        ));
    }
    let held: Vec<AgentId> = store.children_of(parent).into_iter().filter(|c| live.contains(c)).collect();
    if held.len() >= MAX_CHILDREN {
        return Err(format!(
            "`{parent}` already has {} agents running ({}), the most one agent may hold at once. \
             `stop_agent` one that is done, or wait for one to finish, then start another.",
            held.len(),
            held.iter().map(|a| a.0.as_str()).collect::<Vec<_>>().join(", ")
        ));
    }
    Ok(())
}

/// May `caller` send to or stop `target`? The person may, any agent; an agent, only one it started.
pub fn may_direct(caller: &Caller, target: &AgentId, store: &Store, verb: &str) -> Result<(), String> {
    match caller {
        Caller::NoAgent => Ok(()),
        Caller::Agent(me) if store.agent(target).and_then(|a| a.meta.parent.as_ref()) == Some(me) => Ok(()),
        Caller::Agent(_) => Err(format!(
            "`{target}` is not an agent you started, so you cannot {verb} it. An agent can {verb} \
             only the agents it started with new_agent; the person can {verb} any from the Agents \
             screen."
        )),
    }
}

/// May `caller` read `target`? The person may read any; an agent, itself and the agents it started.
pub fn may_read(caller: &Caller, target: &AgentId, store: &Store) -> Result<(), String> {
    match caller {
        Caller::Agent(me) if me == target => Ok(()),
        _ => may_direct(caller, target, store, "read"),
    }
}

/// The mind a caller named, as the host knows it: its id, or — for a caller that used the name a
/// person reads — the id of the one attached mind of that name.
fn mind_id(host: &Host, named: &str) -> Result<String, String> {
    let minds = host.list();
    if let Some(mind) = minds.iter().find(|m| m.id == named) {
        return Ok(mind.id.clone());
    }
    let lower = named.to_lowercase();
    if let Some(mind) = minds.iter().find(|m| m.name.to_lowercase() == lower) {
        return Ok(mind.id.clone());
    }
    let attached: Vec<&str> = minds.iter().filter(|m| !m.builtin).map(|m| m.id.as_str()).collect();
    Err(if attached.is_empty() {
        format!("no mind called `{named}` is attached, and none is: an agent needs an attached mind")
    } else {
        format!("no mind called `{named}` is attached; attached: {}", attached.join(", "))
    })
}

// ── The acts ──────────────────────────────────────────────────────

pub fn new_agent(host: &Host, caller: &Caller, mind: &str, task: &str) -> Result<Value, String> {
    if mind.trim().is_empty() {
        return Err("`mind` is empty: an attached mind's id, as `describe shell` lists under `minds`.".into());
    }
    if task.trim().is_empty() {
        return Err("`task` is empty: what the new agent is to do.".into());
    }
    let parent = match caller {
        Caller::NoAgent => None,
        Caller::Agent(me) => Some(me),
    };
    if let Some(parent) = parent {
        // A plain agent has no reach, so one held to a reach may not start one: that would be a
        // way out of its own.
        if let Some(mine) = reaches::of(parent) {
            return Err(format!(
                "`{parent}` is the {}, which works within a reach, and an agent started on a mind \
                 alone has none. Hand the work to a role from the catalog with hand_off instead.",
                mine.name
            ));
        }
        let live: Vec<AgentId> = host.agents().into_iter().map(|a| a.id).collect();
        agents::store().read(|s| may_start_child(parent, s, &live))?;
    }
    let mind = mind_id(host, mind.trim())?;
    let agent = launch::start_on(host, &mind, task, parent)?;
    tracing::info!(agent = %agent, parent = ?parent.map(|p| p.to_string()), "an agent was started from the socket");
    let said = format!(
        "Started `{agent}` on {mind}{}. It is working on it now, in its own pane. `read_agent` with \
         agent `{agent}` shows how it is going; `send_to_agent` says more; `stop_agent` stops it. \
         It has none of your grants: anything it needs allowed is asked for again, in its pane.",
        parent.map(|p| format!(", started by `{p}`")).unwrap_or_default(),
    );
    Ok(json!({ "agent": agent, "mind": mind, "parent": parent, "state": "thinking", "said": said }))
}

pub fn send_to_agent(host: &Host, caller: &Caller, target: &AgentId, text: &str) -> Result<Value, String> {
    if text.trim().is_empty() {
        return Err("`text` is empty: what to say to it.".into());
    }
    agents::store().read(|s| may_direct(caller, target, s, "send to"))?;
    launch::send_on(host, target, text)?;
    Ok(json!({
        "agent": target,
        "sent": true,
        "said": format!("Sent to `{target}`; it is working on it now. `read_agent` shows its answer as it comes."),
    }))
}

pub fn stop_agent(host: &Host, caller: &Caller, target: &AgentId) -> Result<Value, String> {
    agents::store().read(|s| may_direct(caller, target, s, "stop"))?;
    let stopped = launch::stop_on(host, target)?;
    let mut said = if stopped.stopped || stopped.commands > 0 {
        format!("Stopped `{target}`")
    } else {
        format!("`{target}` had nothing running; it is stopped all the same")
    };
    if stopped.commands > 0 {
        said.push_str(&format!(", and killed {} command{}", stopped.commands, if stopped.commands == 1 { "" } else { "s" }));
    }
    if !stopped.children.is_empty() {
        said.push_str(&format!(
            "; the agents it started stopped with it ({})",
            stopped.children.iter().map(|c| c.0.as_str()).collect::<Vec<_>>().join(", ")
        ));
    }
    said.push_str(". Its pane stays readable on the Agents screen.");
    Ok(json!({
        "agent": target,
        "stopped": stopped.stopped,
        "commands_killed": stopped.commands,
        "approvals_withdrawn": stopped.approvals,
        "children_stopped": stopped.children,
        "said": said,
    }))
}

pub fn read_agent(caller: &Caller, target: &AgentId, last: Option<&Value>) -> Result<Value, String> {
    let last = match last.filter(|v| !v.is_null()) {
        None => READ_TURNS,
        Some(given) => {
            let n = given
                .as_f64()
                .or_else(|| given.as_str().and_then(|s| s.trim().parse().ok()))
                .ok_or_else(|| "`last` is a number of turns.".to_string())?;
            if !(1.0..=READ_TURNS_MOST as f64).contains(&n) {
                return Err(format!("`last` is between 1 and {READ_TURNS_MOST} turns."));
            }
            n as usize
        }
    };
    agents::store().read(|s| {
        may_read(caller, target, s)?;
        let agent = s
            .agent(target)
            .ok_or_else(|| format!("there is no agent `{target}` on this desktop; `describe shell` lists them under `agents`."))?;
        let transcript = s.transcript(target, last).unwrap_or_default();
        Ok(json!({
            "agent": target,
            "mind": agent.meta.mind,
            "state": agent.state.key(),
            "needs_you": !agent.pending_approvals.is_empty() || agent.state == agents::State::WaitingForYou,
            "turns": agent.turns.len(),
            "text": transcript,
        }))
    })
}

// ── Handing work to a role ────────────────────────────────────────

/// What `hand_off` started.
#[derive(Clone, Debug)]
pub struct Handed {
    pub agent: AgentId,
    pub role: catalog::Role,
    pub mind: String,
}

/// How a role's first turn came out, as far as a wait saw.
#[derive(Clone, Debug, PartialEq)]
pub struct Answered {
    /// Its first turn ended within the wait.
    pub done: bool,
    /// And ended well — not failed, not stopped.
    pub ok: bool,
    /// What it said in that turn.
    pub text: String,
    /// A card it asked for in that turn that nobody answered, that was taken back, or that the
    /// person refused — said as a sentence. Its answer is then not the work asked for.
    pub unsettled: Option<String>,
}

/// Start `role` on `task`, for `caller`: the rules of `new_agent`, then the role's own — its first
/// attached mind that can give it a conversation of its own, and its reach held on every door
/// before its first turn is sent.
pub fn hand_off(host: &Host, caller: &Caller, catalog: &Catalog, role: &str, task: &str, context: &str) -> Result<Handed, String> {
    let parent = match caller {
        Caller::NoAgent => None,
        Caller::Agent(me) => Some(me),
    };
    hand_off_as(host, parent, None, catalog, role, task, context)
}

/// [`hand_off`], for whoever the work is from: `parent`, the agent handing it over — or, for a
/// recipe, the agent that asked for the run, whose children the recipe's agents are — and
/// `recipe`, the run whose Agent step it is. Every rule is the same.
pub fn hand_off_as(
    host: &Host,
    parent: Option<&AgentId>,
    recipe: Option<&RecipeOrigin>,
    catalog: &Catalog,
    role: &str,
    task: &str,
    context: &str,
) -> Result<Handed, String> {
    if role.trim().is_empty() {
        return Err(format!("`role` is empty: a role from the catalog — {}.", catalog.listing()));
    }
    let Some(role) = catalog.find(role) else {
        return Err(format!(
            "There is no role `{}` in the catalog; it has {}. `describe shell` lists them under `catalog`.",
            role.trim(),
            catalog.listing()
        ));
    };
    if task.trim().is_empty() {
        return Err(format!("`task` is empty: what the {} is to do.", role.name));
    }
    if context.len() > CONTEXT_MOST_BYTES {
        return Err(format!(
            "`context` is {} KiB; at most {} KiB goes into a first turn. Put the rest in a file and \
             name it in the task.",
            context.len() / 1024,
            CONTEXT_MOST_BYTES / 1024
        ));
    }
    if let Some(parent) = parent {
        let live: Vec<AgentId> = host.agents().into_iter().map(|a| a.id).collect();
        agents::store().read(|s| may_start_child(parent, s, &live))?;
        // A reach caps the agents it hands work to as well: never above its own ceiling.
        if let Some(mine) = reaches::of(parent) {
            if gate::grade(&role.reach.ceiling) > gate::grade(&mine.ceiling) {
                return Err(format!(
                    "`{parent}` is the {}, at most `{}`, and cannot hand work to the {}, whose reach \
                     goes up to `{}`: a role never hands work above its own ceiling.",
                    mine.name, mine.ceiling, role.name, role.reach.ceiling
                ));
            }
        }
    }
    let mind = role.pick_mind(&catalog::minds_now(host))?;
    let hold = |agent: &AgentId| reaches::hold(host, agent, role);
    let how = launch::Start {
        title: Some(task.trim()),
        role: Some(role.meta()),
        before_first_turn: Some(&hold),
        recipe: recipe.cloned(),
    };
    let agent = launch::start_with(host, &mind, &role.first_turn(task, context), parent, how)?;
    watch_budget(host.clone(), agent.clone(), role.name.clone(), role.budget.minutes);
    tracing::info!(
        agent = %agent, role = %role.id, mind = %mind, parent = ?parent.map(|p| p.to_string()),
        recipe = ?recipe.map(|r| r.id.as_str()), "work was handed to a catalog role"
    );
    Ok(Handed { agent, role: role.clone(), mind })
}

// ── A recipe's Agent steps ────────────────────────────────────────

/// What the shell gives the companion's recipe executor so a formation's Agent steps reach the
/// catalog: each one through [`hand_off_as`], never around it. Installed on the companion worker
/// (`bridge::worker_loop`) before it resumes the recipes a restart left running.
#[derive(Default)]
pub struct RecipeHands {
    /// The cards raised for runs nobody started at the desk, by recipe and step.
    asks: Asks,
}

/// Approval requests raised for a recipe's Agent steps: (recipe id, step) → request id.
pub type Asks = std::collections::HashMap<(String, usize), String>;

impl AgentHook for RecipeHands {
    fn start(&mut self, call: &AgentCall<'_>) -> Result<AgentStarted, AgentRefusal> {
        let host = host().map_err(AgentRefusal::Fail)?;
        start_for_recipe(host, &Catalog::load(), call, &mut self.asks)
    }

    fn poll(&mut self, recipe_id: &str, agent: &str) -> AgentPoll {
        poll_for_recipe(crate::wire::harness::host(), recipe_id, &AgentId(agent.to_string()))
    }

    fn release(&mut self, recipe_id: &str, agent: &str, why: &str) {
        if let Ok(host) = host() {
            release_for_recipe(host, recipe_id, &AgentId(agent.to_string()), why);
        }
    }
}

/// Start an Agent step's role for its recipe, held to what the person agreed to:
///
/// - a run started at the desk: only a role it named then, with the definition it had then;
/// - a run nobody started at the desk: a role above `safe` only on the person's Allow, asked on a
///   card naming the recipe and the role ([`AgentRefusal::Ask`] until it is answered);
/// - no place for it — the recipe's own three, the asking agent's three, the desktop's six — and
///   the start queues ([`AgentRefusal::Wait`]) rather than racing the person's own for a place;
/// - anything `hand_off` refuses — no mind for the role, a rule the asking agent is held to — and
///   a card denied or left to expire: not at all ([`AgentRefusal::Fail`]).
///
/// An agent started without a card of its own is written to the record of unasked actions under
/// the recipe's name.
pub fn start_for_recipe(host: &Host, catalog: &Catalog, call: &AgentCall<'_>, asks: &mut Asks) -> Result<AgentStarted, AgentRefusal> {
    let origin = RecipeOrigin { id: call.recipe_id.to_string(), name: call.recipe_name.to_string() };
    let Some(role) = catalog.find(call.role) else {
        return Err(AgentRefusal::Fail(format!(
            "There is no role `{}` in the catalog; it has {}.",
            call.role.trim(),
            catalog.listing()
        )));
    };
    let digest = role.digest();
    // What the person agreed to when they started the run.
    if call.attended {
        match call.consented {
            None => {
                return Err(AgentRefusal::Fail(format!(
                    "the {} was not among the roles the person agreed to when this run started — the \
                     recipe named it since — so it was not started",
                    role.name
                )))
            }
            Some(agreed) if agreed != digest => {
                return Err(AgentRefusal::Fail(format!(
                    "the {}'s definition has changed since the person started this run (a file in \
                     ~/.config/yantrik/agents replaced or edited it: its minds, brief or reach are not \
                     what they agreed to), so it was not started. Start the recipe again to agree to the \
                     role as it is now",
                    role.name
                )))
            }
            Some(_) => {}
        }
    }
    // A place for it, before anything is asked of the person: a card allowed now would run out
    // while the start waited for one.
    let live: Vec<AgentId> = host.agents().into_iter().map(|a| a.id).collect();
    let working: Vec<AgentId> =
        agents::store().read(|s| s.agents_of_recipe(&origin.id)).into_iter().filter(|a| live.contains(a)).collect();
    if working.len() >= MAX_CHILDREN {
        return Err(AgentRefusal::Wait(format!(
            "a place: the {} already has {} agents working ({}), the most one recipe holds at once",
            origin.label(),
            working.len(),
            working.iter().map(|a| a.0.as_str()).collect::<Vec<_>>().join(", ")
        )));
    }
    let asked_by = call.asked_by.map(|a| AgentId(a.to_string()));
    if let Some(parent) = &asked_by {
        let theirs: Vec<AgentId> =
            agents::store().read(|s| s.children_of(parent)).into_iter().filter(|a| live.contains(a)).collect();
        if theirs.len() >= MAX_CHILDREN {
            return Err(AgentRefusal::Wait(format!(
                "a place: `{parent}`, which asked for this run, already has {} agents running, the most one \
                 agent holds at once — stop one on the Agents screen, or it starts when one finishes",
                theirs.len()
            )));
        }
    }
    let most = yantrik_harness::protocol::MAX_LIVE_AGENTS;
    if live.len() >= most {
        return Err(AgentRefusal::Wait(format!(
            "a place: the desktop is running {} agents, the most it runs at once — stop one on the \
             Agents screen, or it starts when one finishes",
            live.len()
        )));
    }
    // Nobody started this run at the desk: a role above `safe` waits for the person's Allow.
    let above_safe = gate::grade(&role.reach.ceiling) > gate::grade("safe");
    let carded = if !call.attended && above_safe {
        allowed_on_card(&origin, call, role, &digest, asks)?;
        true
    } else {
        false
    };
    let handed = hand_off_as(host, asked_by.as_ref(), Some(&origin), catalog, call.role, call.task, call.context)
        .map_err(AgentRefusal::Fail)?;
    if !carded {
        let mode = crate::mind_mode::current();
        let verified = crate::approvals::Verified {
            line: format!("the shell's recipe executor, for the {} ({})", origin.label(), origin.id),
            agent: asked_by.as_ref().map(|a| a.0.clone()).unwrap_or_default(),
            ..Default::default()
        };
        let args = json!({ "role": role.id, "recipe": origin.id, "agent": handed.agent, "mind": handed.mind });
        let why = if call.attended { "the person's start of the recipe" } else { "a role at most safe" };
        crate::mind_mode::record(
            mode.as_str(),
            &origin.label(),
            &verified,
            "shell",
            "hand_off",
            &args,
            "sensitive",
            &format!("started {} on {} — {why}", handed.agent, handed.mind),
        );
    }
    Ok(AgentStarted {
        agent: handed.agent.0.clone(),
        role: handed.role.id.clone(),
        role_name: handed.role.name.clone(),
        mind: handed.mind.clone(),
        minutes: u64::from(handed.role.budget.minutes),
    })
}

/// A run nobody started at the desk asks the person before a role above `safe`: a card naming the
/// recipe and the role — its reach, its minds — bound to the role's definition, so an Allow for
/// one definition starts no other. `Ok` once the Allow is spent; [`AgentRefusal::Ask`] while the
/// card waits; [`AgentRefusal::Fail`] once it is denied or left to expire.
fn allowed_on_card(origin: &RecipeOrigin, call: &AgentCall<'_>, role: &catalog::Role, digest: &str, asks: &mut Asks) -> Result<(), AgentRefusal> {
    use crate::approvals::{self, Outcome};
    let key = (origin.id.clone(), call.step);
    let task: String = call.task.chars().take(200).collect();
    let args = json!({ "role": role.id, "recipe": origin.id, "task": task, "definition": digest });
    let waiting = format!("your Allow on the card: {} → {} (it may touch {})", origin.label(), role.name, role.reach.text());
    if let Some(id) = asks.get(&key).cloned() {
        return match approvals::outcome(&id) {
            None => Err(AgentRefusal::Ask(waiting)),
            Some((Outcome::Allowed, _)) => {
                asks.remove(&key);
                approvals::consume(&id, "shell", "hand_off", &args).map_err(AgentRefusal::Fail)
            }
            Some((Outcome::Denied, _)) => {
                asks.remove(&key);
                Err(AgentRefusal::Fail(format!("the person denied handing work to the {} on its card", role.name)))
            }
            Some((Outcome::Unanswered | Outcome::Withdrawn, _)) => {
                asks.remove(&key);
                Err(AgentRefusal::Fail(format!(
                    "the card asking to hand work to the {} went unanswered and expired, so it was not started",
                    role.name
                )))
            }
        };
    }
    let verified = approvals::Verified {
        line: format!(
            "the shell's recipe executor, for the {} ({}), which started with nobody at the desk",
            origin.label(),
            origin.id
        ),
        ..Default::default()
    };
    let purpose = format!(
        "Hand work to the {} from the agent catalog, for the {} — which started with nobody at the \
         desk (a trigger or a timer), so nobody has agreed to this yet. The {} may touch {}, for up to \
         {} minutes, on {}.",
        role.name,
        origin.label(),
        role.name,
        role.reach.text(),
        role.budget.minutes,
        role.mind.join(" or ")
    );
    match approvals::request(&origin.label(), verified, "shell", "hand_off", args, "sensitive", &purpose) {
        Ok(asked) => {
            asks.insert(key, asked.id);
            Err(AgentRefusal::Ask(waiting))
        }
        // The cards on screen are all the person can take at once: ask again next time.
        Err(why) if why.contains("already waiting") => Err(AgentRefusal::Ask(format!("{waiting} — {why}"))),
        Err(why) => Err(AgentRefusal::Fail(why)),
    }
}

/// How an agent `recipe_id` started is doing, from its session: its first turn's answer once the
/// turn has ended — which a restart keeps, the saved session has it — or working while its
/// conversation is live. An agent the recipe did not start, or one the desktop no longer knows, is
/// no answer at all.
pub fn poll_for_recipe(host: Option<&Host>, recipe_id: &str, agent: &AgentId) -> AgentPoll {
    let seen = agents::store().read(|s| {
        s.agent(agent).map(|a| (a.meta.recipe.as_ref().is_some_and(|r| r.id == recipe_id), first_answer(a)))
    });
    match seen {
        None => AgentPoll::Failed(format!("the desktop no longer knows `{agent}`")),
        Some((false, _)) => AgentPoll::Failed(format!("`{agent}` was not started by this recipe")),
        // A card of its own that nobody answered, or that the person refused: its work is not what
        // was asked, whatever it said after. Never read as done.
        Some((true, Some(answered))) if answered.unsettled.is_some() => {
            AgentPoll::Failed(answered.unsettled.unwrap_or_default())
        }
        Some((true, Some(answered))) if answered.ok && !answered.text.trim().is_empty() => AgentPoll::Answered(answered.text),
        Some((true, Some(answered))) if answered.ok => AgentPoll::Failed("it ended its turn without an answer".to_string()),
        Some((true, Some(answered))) => AgentPoll::Failed(if answered.text.is_empty() {
            "its turn ended without an answer".to_string()
        } else {
            answered.text
        }),
        // Its turn is still open: working while its conversation is live — waiting on the person
        // when it asked for something in its pane. Once the conversation is gone the answer will
        // never come. With no host yet — the shell still starting — it cannot be told.
        Some((true, None)) => match host {
            Some(host) if !host.agents().iter().any(|a| &a.id == agent) => AgentPoll::Failed(format!(
                "its conversation is gone (`{agent}` is no longer running), so its answer will not come"
            )),
            _ => match agents::store().read(|s| s.agent(agent).and_then(waiting_on_you)) {
                Some(why) => AgentPoll::NeedsYou(why),
                None => AgentPoll::Working,
            },
        },
    }
}

/// What an agent is waiting on the person for in its own pane, if anything: a card it asked for,
/// or a command of its at a prompt.
fn waiting_on_you(agent: &Agent) -> Option<String> {
    let who = match &agent.meta.role {
        Some(role) => format!("the {} ({})", role.name, agent.meta.id),
        None => format!("`{}`", agent.meta.id),
    };
    let asked = agent.turns.iter().rev().flat_map(|t| t.items.iter().rev()).find_map(|item| match item {
        Item::Approval(a) if a.outcome == agents::ApprovalOutcome::Pending => Some(a.what.clone()),
        _ => None,
    });
    match asked {
        Some(what) => Some(format!("{who} is waiting for your Allow on a card in its pane: {what}")),
        None if !agent.pending_approvals.is_empty() || agent.state == agents::State::WaitingForYou => {
            Some(format!("{who} is waiting for you in its pane"))
        }
        None => None,
    }
}

/// Let an agent `recipe_id` started go, saying why in its pane. Any other agent is left alone.
pub fn release_for_recipe(host: &Host, recipe_id: &str, agent: &AgentId, why: &str) {
    let ours = agents::store().read(|s| s.agent(agent).and_then(|a| a.meta.recipe.as_ref()).is_some_and(|r| r.id == recipe_id));
    if ours {
        launch::let_go(host, agent, why);
    }
}

/// The Agents screen's New agent → from the catalog: the person hands `task` to `role`.
pub fn hand_off_from_screen(role: &str, task: &str) -> Result<AgentId, String> {
    hand_off(host()?, &Caller::NoAgent, &Catalog::load(), role, task, "").map(|handed| handed.agent)
}

/// A role's budget in minutes, held: when it runs out the agent is stopped — its conversation let
/// go, its place under the desktop's cap freed — and its pane says why. Ends early once it is
/// stopped some other way.
fn watch_budget(host: Host, agent: AgentId, name: String, minutes: u32) {
    let budget = Duration::from_secs(u64::from(minutes) * 60);
    let spawned = std::thread::Builder::new().name("agent-budget".into()).spawn(move || {
        let started = Instant::now();
        let live = |host: &Host| host.agents().iter().any(|a| a.id == agent);
        loop {
            let left = budget.saturating_sub(started.elapsed());
            if left.is_zero() {
                break;
            }
            std::thread::sleep(left.min(Duration::from_secs(5)));
            if !live(&host) {
                return;
            }
        }
        if live(&host) {
            let _ = launch::stop_on(&host, &agent);
            agents::store().note(&agent, &format!("Its budget of {minutes} minutes as the {name} ran out, so it was stopped."));
            tracing::info!(agent = %agent, minutes, "a role's budget ran out; it was stopped");
        }
    });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "could not watch a role's budget");
    }
}

/// Wait up to `wait` for a role's first turn to end, and read what it said.
pub fn wait_for_answer(agent: &AgentId, wait: Duration) -> Answered {
    let deadline = Instant::now() + wait;
    loop {
        if let Some(answered) = agents::store().read(|s| s.agent(agent).and_then(first_answer)) {
            return answered;
        }
        if Instant::now() >= deadline {
            return Answered { done: false, ok: false, text: String::new(), unsettled: None };
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// An agent's first turn — the work it was handed — once it has ended: how it came out, what it
/// said, and any card of its that did not come out as allowed. None while it is still open.
fn first_answer(agent: &Agent) -> Option<Answered> {
    let first = agent.turns.iter().find(|t| !t.prompt.is_empty())?;
    first.ended?;
    let unsettled = first.items.iter().find_map(|item| match item {
        Item::Approval(a) => match a.outcome {
            agents::ApprovalOutcome::Expired => Some(format!(
                "its card for {} went unanswered and expired, so its work is not what was asked",
                a.what
            )),
            agents::ApprovalOutcome::Withdrawn => {
                Some(format!("its card for {} was taken back, so its work is not what was asked", a.what))
            }
            agents::ApprovalOutcome::Denied => {
                Some(format!("the person denied its {}, so its work is not what was asked", a.what))
            }
            _ => None,
        },
        _ => None,
    });
    Some(Answered { done: true, ok: first.ok == Some(true), text: turn_text(first), unsettled })
}

/// What a turn said, in words: its text, and — for a turn that did not end well — the shell's
/// notes on why. Cut at [`ANSWER_MOST_BYTES`], saying so.
fn turn_text(turn: &Turn) -> String {
    let mut text = String::new();
    for item in &turn.items {
        match item {
            Item::Text(t) => text.push_str(&t.text()),
            Item::Note(note) if turn.ok != Some(true) => text.push_str(&format!("\n[{note}]\n")),
            _ => {}
        }
    }
    let text = text.trim();
    if text.len() <= ANSWER_MOST_BYTES {
        return text.to_string();
    }
    let mut cut = ANSWER_MOST_BYTES;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}\n… {} more bytes; `read_agent` has the rest.", &text[..cut], text.len() - cut)
}

impl Handed {
    /// What `hand_off` answers: the agent, the role, where it runs and what it may touch — and,
    /// after a wait, what it said or that it is still working.
    pub fn answer(&self, waited: Option<(Duration, Answered)>) -> Value {
        let role = &self.role;
        let who = format!("the {} (`{}`, on {})", role.name, self.agent, self.mind);
        let mut out = json!({
            "agent": self.agent,
            "role": role.id,
            "role_name": role.name,
            "mind": self.mind,
            "reach": role.reach.text(),
            "budget": { "turns": role.budget.turns, "minutes": role.budget.minutes },
        });
        let follow = format!(
            "`read_agent` with agent `{agent}` shows how it is going; `send_to_agent` says more; \
             `stop_agent` stops it.",
            agent = self.agent
        );
        let said = match waited {
            None => format!(
                "Handed to {who}. It works on its own, in its own pane, within its reach ({reach}), \
                 for up to {turns} turns and {minutes} minutes, and it has none of your grants. {follow}",
                reach = role.reach.text(),
                turns = role.budget.turns,
                minutes = role.budget.minutes,
            ),
            Some((_, answered)) if answered.done => {
                out["done"] = true.into();
                out["ok"] = answered.ok.into();
                out["answer"] = answered.text.clone().into();
                let how = if answered.ok { "answered" } else { "could not finish; what it said" };
                let text = if answered.text.is_empty() { "(it said nothing)".to_string() } else { answered.text };
                format!("{} {how}:\n\n{text}", capitalised(&who))
            }
            Some((wait, _)) => {
                out["done"] = false.into();
                format!(
                    "Handed to {who}; it is still working after {} s. {follow}",
                    wait.as_secs()
                )
            }
        };
        out["said"] = said.into();
        out
    }
}

/// Whether `agent` may be shown asking for `app.action(args)`, graded `grade`: an agent held to a
/// role's reach is refused, in the reach's words, before a card for an act its reach refuses
/// would reach the person — the act itself would be refused on its door whatever they pressed.
/// With the arguments, because an opening act is within a reach by the app it names.
pub fn within_reach(agent: &AgentId, app: &str, action: &str, grade: &str, args: &Value) -> Result<(), String> {
    match reaches::of(agent) {
        Some(reach) => yantrik_ipc_transport::reach::within_call(&reach, app, action, grade, args),
        None => Ok(()),
    }
}

fn capitalised(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::{AgentMeta, Store};
    use yantrik_harness::protocol;

    fn id(s: &str) -> AgentId {
        AgentId(s.to_string())
    }

    /// Nothing that shows or keeps arguments — `describe shell`, a card, an audit line — is handed a
    /// token by these actions: none takes one, and none takes the caller's own agent either. The
    /// one `agent` argument some of them take is the agent acted ON.
    #[test]
    fn no_agents_action_takes_the_token_or_the_callers_own_agent() {
        for spec in specs() {
            let schema = spec.schema();
            let params = schema["parameters"]["properties"].as_object().unwrap();
            for banned in ["agent_token", "token", "parent", "caller", "grant"] {
                assert!(!params.contains_key(banned), "{} takes `{banned}`: {schema}", spec.name);
            }
        }
        let grade = |name: &str| specs().into_iter().find(|s| s.name == name).unwrap().permission;
        assert_eq!(
            [grade("new_agent"), grade("send_to_agent"), grade("stop_agent"), grade("read_agent"), grade("show_agent"), grade("hand_off")],
            ["sensitive", "standard", "standard", "safe", "safe", "sensitive"],
            "the grades the design settled: hand_off is gated like new_agent"
        );
    }

    // ── Handing work to a role ──

    fn attach(host: &Host, id: &str, conversations: bool) -> String {
        let attach = json!({ "id": id, "name": id, "conversations": conversations });
        host.handle_from(protocol::ATTACH, &attach, Some(std::process::id())).unwrap()["session"]
            .as_str()
            .unwrap()
            .to_string()
    }

    fn shipped() -> Catalog {
        Catalog::from_layers(&catalog::SHIPPED, &[])
    }

    /// Every turn waiting for a harness, by conversation.
    fn handed_out(host: &Host, session: &str) -> std::collections::HashMap<String, Value> {
        let mut out = std::collections::HashMap::new();
        for _ in 0..16 {
            let turn = host.handle(protocol::POLL, &json!({ "session": session })).unwrap();
            let Some(conversation) = turn["conversation"].as_str() else { break };
            out.insert(conversation.to_string(), turn.clone());
        }
        out
    }

    fn settled(agent: &AgentId) -> bool {
        (0..100).any(|_| {
            let done = agents::store().read(|s| s.agent(agent).is_some_and(|a| a.open_turn().is_none()));
            if !done {
                std::thread::sleep(Duration::from_millis(50));
            }
            done
        })
    }

    /// The whole of hand_off against a real host: the role's preferred attached mind, a first turn
    /// of brief, task and context with a token of its own, a row named for the task with its role
    /// — and the role's reach held before that first turn, on the shell's registry and in the file
    /// a door in another process reads, which never holds the token. Then in reach, off its
    /// surfaces and above its ceiling, as every door decides them; and a stop lets it go.
    #[test]
    fn hand_off_starts_the_role_on_its_first_attached_mind_held_to_its_reach() {
        let host = Host::new(vec![]);
        let pi = attach(&host, "pi", true);
        let deepseek = attach(&host, "deepseek", true);
        let handed = hand_off(&host, &Caller::NoAgent, &shipped(), "Reviewer", "review the change in ~/src/app", "diff --git a/x b/x").unwrap();
        assert_eq!((handed.mind.as_str(), handed.agent.harness()), ("deepseek", "deepseek"), "the Reviewer runs on deepseek first");

        let first = host.handle(protocol::POLL, &json!({ "session": deepseek })).unwrap();
        let text = first["text"].as_str().unwrap();
        for says in ["You are the Reviewer", "Find what is wrong with a change", "The task:\nreview the change in ~/src/app", "Read this first:\ndiff --git a/x b/x"] {
            assert!(text.contains(says), "{says:?} missing:\n{text}");
        }
        let token = first["agent_token"].as_str().unwrap().to_string();
        assert!(host.handle(protocol::POLL, &json!({ "session": pi })).unwrap()["turn_id"].is_null(), "nothing went to pi");

        let (title, role) = agents::store().read(|s| s.agent(&handed.agent).map(|a| (a.meta.title.clone(), a.meta.role.clone()))).unwrap();
        assert_eq!(title, "review the change in ~/src/app", "its row is named for the task, not the brief");
        assert_eq!(role.map(|r| (r.name, r.reach)), Some(("Reviewer".to_string(), "editor, documents and notes · at most safe".to_string())));

        let held = reaches::lookup(&token).expect("the shell's own dispatch holds it");
        assert_eq!(held.agent, handed.agent.0);
        // Every other door asks the shell by the token's digest, and is told it is held.
        use yantrik_ipc_transport::reach::{token_digest, Standing};
        let digest = token_digest(&token);
        assert_eq!(reaches::standing(Some(&host), &digest), Standing::Held(held.clone()), "and so does every other door");
        assert!(!Standing::Held(held.clone()).to_json().to_string().contains(&token), "what a door is told never holds the token");

        use yantrik_ipc_transport::reach::within;
        assert!(within(&held, "notes", "list_notes", "safe").is_ok(), "in reach");
        let err = within(&held, "files", "move", "safe").unwrap_err();
        assert!(err.starts_with("REACH: files.move is outside the Reviewer's reach") && err.contains(&handed.agent.0), "{err}");
        let err = within(&held, "notes", "new_note", "standard").unwrap_err();
        assert!(err.contains("above the Reviewer's `safe` ceiling"), "{err}");
        let err = within(&held, "shell", "agent_run", "sensitive").unwrap_err();
        assert!(err.starts_with("REACH: shell.agent_run is outside"), "a reviewer runs no commands: {err}");
        // Asking the person about an act its reach refuses is refused too, in the same words.
        let err = within_reach(&handed.agent, "files", "move", "sensitive", &json!({})).unwrap_err();
        assert!(err.starts_with("REACH:"), "{err}");
        assert!(within_reach(&AgentId("pi:c-noreach".into()), "files", "move", "sensitive", &json!({})).is_ok(), "an agent with no role has no reach");

        let answer = handed.answer(None);
        let said = answer["said"].as_str().unwrap();
        assert!(said.starts_with(&format!("Handed to the Reviewer (`{}`, on deepseek)", handed.agent)), "{said}");
        assert!(said.contains("editor, documents and notes · at most safe") && said.contains("4 turns and 15 minutes"), "{said}");
        assert!(!answer.to_string().contains(&token), "{answer}");

        stop_agent(&host, &Caller::NoAgent, &handed.agent).unwrap();
        assert_eq!(reaches::lookup(&token), None, "stopped, it is let go");
        assert_eq!(reaches::standing(Some(&host), &digest), Standing::Unknown, "and its token is refused on every door");
    }

    /// Down its list to the first mind attached that can give it a conversation of its own; a
    /// one-conversation mind is never used, and none at all is said plainly with nothing started.
    #[test]
    fn a_role_falls_back_down_its_list_and_is_refused_plainly_when_none_of_its_minds_is_attached() {
        let host = Host::new(vec![]);
        attach(&host, "hermes", false);
        let err = hand_off(&host, &Caller::NoAgent, &shipped(), "reviewer", "review it", "").unwrap_err();
        assert!(err.starts_with("No mind the Reviewer runs on is attached: it runs on deepseek, pi, openclaw"), "{err}");
        assert!(err.contains("hermes holds one conversation at a time — the person's own"), "{err}");
        assert!(host.agents().is_empty(), "nothing was started, and the person's own conversation with hermes is untouched");

        attach(&host, "pi", true);
        let handed = hand_off(&host, &Caller::NoAgent, &shipped(), "reviewer", "review it", "").unwrap();
        assert_eq!(handed.mind, "pi", "past deepseek, which is not attached, to pi");

        let err = hand_off(&host, &Caller::NoAgent, &shipped(), "janitor", "x", "").unwrap_err();
        assert!(err.contains("no role `janitor`") && err.contains("reviewer (Reviewer)"), "{err}");
        assert!(hand_off(&host, &Caller::NoAgent, &shipped(), "", "x", "").unwrap_err().contains("`role` is empty"));
        assert!(hand_off(&host, &Caller::NoAgent, &shipped(), "reviewer", "  ", "").unwrap_err().contains("`task` is empty"));
        let err = hand_off(&host, &Caller::NoAgent, &shipped(), "reviewer", "x", &"a".repeat(40 * 1024)).unwrap_err();
        assert!(err.contains("at most 32 KiB"), "{err}");
        stop_agent(&host, &Caller::NoAgent, &handed.agent).unwrap();
    }

    /// new_agent's rules, word for word — depth one, three children, nothing of the parent's — and
    /// the reach capping further: an agent held to a reach cannot hand work above its own ceiling,
    /// nor start a plain agent that would have no reach at all.
    #[test]
    fn hand_off_meets_new_agents_rules_and_a_reach_caps_what_it_hands_on() {
        let host = Host::new(vec![]);
        let session = attach(&host, "pi", true);
        let started = new_agent(&host, &Caller::NoAgent, "pi", "plan the release").unwrap();
        let parent = AgentId(started["agent"].as_str().unwrap().to_string());
        let parent_token = handed_out(&host, &session)[parent.conversation()]["agent_token"].as_str().unwrap().to_string();
        let as_parent = Caller::Agent(parent.clone());
        // A plain agent — started on a mind, with no role — is live and held to nothing: every
        // door is told so, and only the gate decides for it.
        use yantrik_ipc_transport::reach::{token_digest, Standing};
        assert_eq!(reaches::standing(Some(&host), &token_digest(&parent_token)), Standing::Plain);
        assert_eq!(reaches::standing(Some(&host), &token_digest("0123456789abcdef0123456789abcdef")), Standing::Unknown);
        assert_eq!(reaches::standing(None, &token_digest(&parent_token)), Standing::Unknown, "no host, no live agent");

        let kids: Vec<Handed> = ["researcher", "writer", "scribe"]
            .iter()
            .map(|role| hand_off(&host, &as_parent, &shipped(), role, &format!("{role}'s part"), "").unwrap())
            .collect();
        let err = hand_off(&host, &as_parent, &shipped(), "planner", "a fourth", "").unwrap_err();
        assert!(err.contains("already has 3 agents running"), "{err}");
        let err = hand_off(&host, &Caller::Agent(kids[0].agent.clone()), &shipped(), "chair", "weigh them", "").unwrap_err();
        assert!(err.contains("one level only"), "{err}");

        let turns = handed_out(&host, &session);
        for kid in &kids {
            let turn = &turns[kid.agent.conversation()];
            assert!(!turn.to_string().contains(&parent_token), "nothing of the parent's token");
            assert_ne!(turn["agent_token"].as_str().unwrap(), parent_token, "a token of its own");
            let parent_of = agents::store().read(|s| s.agent(&kid.agent).and_then(|a| a.meta.parent.clone()));
            assert_eq!(parent_of, Some(parent.clone()), "its row says who handed it the work");
        }
        let stopped = stop_agent(&host, &Caller::NoAgent, &parent).unwrap();
        assert_eq!(stopped["children_stopped"].as_array().map(Vec::len), Some(3), "{stopped}");
        assert_eq!(
            reaches::standing(Some(&host), &token_digest(&parent_token)),
            Standing::Unknown,
            "stopped, its token is no live agent's, and every door refuses it"
        );

        // A Reviewer (safe) the person started: it may hand work to the Red team (safe), not to the
        // Coder (sensitive), and it may not start a plain agent.
        let reviewer = hand_off(&host, &Caller::NoAgent, &shipped(), "reviewer", "review it", "").unwrap();
        let as_reviewer = Caller::Agent(reviewer.agent.clone());
        let err = hand_off(&host, &as_reviewer, &shipped(), "coder", "fix it", "").unwrap_err();
        assert!(err.contains("is the Reviewer, at most `safe`, and cannot hand work to the Coder"), "{err}");
        let err = new_agent(&host, &as_reviewer, "pi", "do anything").unwrap_err();
        assert!(err.contains("works within a reach"), "{err}");
        let red = hand_off(&host, &as_reviewer, &shipped(), "red-team", "attack it", "").unwrap();
        assert_eq!(red.role.id, "red-team");
        stop_agent(&host, &Caller::NoAgent, &reviewer.agent).unwrap();
    }

    /// With a wait, hand_off hands back what the role said once its first turn ends; a wait that
    /// runs out says it is still working. And a role's turns are its budget.
    #[test]
    fn hand_off_with_a_wait_hands_back_the_roles_answer_and_its_turns_are_a_budget() {
        let host = Host::new(vec![]);
        let session = attach(&host, "pi", true);
        let handed = hand_off(&host, &Caller::NoAgent, &shipped(), "chair", "weigh the three answers", "A says ship; B says wait").unwrap();
        let harness = {
            let (host, session) = (host.clone(), session.clone());
            std::thread::spawn(move || {
                let turn = host.handle(protocol::POLL, &json!({ "session": session })).unwrap();
                let id = turn["turn_id"].clone();
                host.handle(protocol::CHUNK, &json!({ "session": session, "turn_id": id, "delta": "Verdict — ship on Friday." })).unwrap();
                host.handle(protocol::COMPLETE, &json!({ "session": session, "turn_id": id })).unwrap();
            })
        };
        let answered = wait_for_answer(&handed.agent, Duration::from_secs(10));
        harness.join().unwrap();
        assert_eq!(answered, Answered { done: true, ok: true, text: "Verdict — ship on Friday.".into(), unsettled: None });
        let answer = handed.answer(Some((Duration::from_secs(10), answered)));
        assert_eq!((answer["done"].clone(), answer["answer"].clone()), (json!(true), json!("Verdict — ship on Friday.")));
        let said = answer["said"].as_str().unwrap();
        assert!(said.starts_with(&format!("The Chair (`{}`, on pi) answered:\n\nVerdict", handed.agent)), "{said}");

        // The Chair has two turns: a second is sent, a third is refused.
        assert!(settled(&handed.agent));
        send_to_agent(&host, &Caller::NoAgent, &handed.agent, "and C says never").unwrap();
        let turn = host.handle(protocol::POLL, &json!({ "session": session })).unwrap();
        host.handle(protocol::COMPLETE, &json!({ "session": session, "turn_id": turn["turn_id"] })).unwrap();
        assert!(settled(&handed.agent));
        let err = send_to_agent(&host, &Caller::NoAgent, &handed.agent, "one more").unwrap_err();
        assert!(err.contains("is the Chair, whose budget is 2 turns, and it has had them all"), "{err}");

        // A wait that runs out.
        let other = hand_off(&host, &Caller::NoAgent, &shipped(), "scribe", "summarise it", "").unwrap();
        let answered = wait_for_answer(&other.agent, Duration::from_millis(300));
        assert!(!answered.done);
        let said = other.answer(Some((Duration::from_millis(300), answered)))["said"].as_str().unwrap().to_string();
        assert!(said.contains("it is still working after 0 s") && said.contains("`read_agent` with agent"), "{said}");

        assert_eq!(wait_arg(&json!({})).unwrap(), None);
        assert_eq!(wait_arg(&json!({ "wait_seconds": 0 })).unwrap(), None, "zero is not waiting");
        assert_eq!(wait_arg(&json!({ "wait_seconds": 90 })).unwrap(), Some(Duration::from_secs(90)));
        assert!(wait_arg(&json!({ "wait_seconds": 601 })).is_err());
        assert!(wait_arg(&json!({ "wait_seconds": "soon" })).is_err());
        for agent in [&handed.agent, &other.agent] {
            stop_agent(&host, &Caller::NoAgent, agent).unwrap();
        }
    }

    /// Depth one, three children, and the host's own cap — checked without a host.
    #[test]
    fn a_child_cannot_start_agents_and_a_parent_holds_at_most_three() {
        let mut s = Store::new();
        let parent = id("pi:c-par001");
        s.upsert_agent(AgentMeta::new(parent.clone(), "pi"));
        let mut live = vec![parent.clone()];
        for n in 0..3 {
            let child = id(&format!("pi:c-kid00{n}"));
            let mut meta = AgentMeta::new(child.clone(), "pi");
            meta.parent = Some(parent.clone());
            s.upsert_agent(meta);
            live.push(child);
        }
        let err = may_start_child(&parent, &s, &live).unwrap_err();
        assert!(err.contains("already has 3 agents running"), "{err}");
        // One of them stopped: its place is free again.
        live.retain(|a| a.0 != "pi:c-kid000");
        assert!(may_start_child(&parent, &s, &live).is_ok());
        // A child asking for a child of its own.
        let err = may_start_child(&id("pi:c-kid001"), &s, &live).unwrap_err();
        assert!(err.contains("one level only") && err.contains("pi:c-par001"), "{err}");
    }

    #[test]
    fn an_agent_directs_only_the_agents_it_started_and_the_person_directs_any() {
        let mut s = Store::new();
        let mut meta = AgentMeta::new(id("pi:c-kid001"), "pi");
        meta.parent = Some(id("pi:c-par001"));
        s.upsert_agent(meta);
        s.upsert_agent(AgentMeta::new(id("deepseek:c-other1"), "deepseek"));
        let parent = Caller::Agent(id("pi:c-par001"));
        assert!(may_direct(&parent, &id("pi:c-kid001"), &s, "stop").is_ok());
        let err = may_direct(&parent, &id("deepseek:c-other1"), &s, "send to").unwrap_err();
        assert!(err.contains("not an agent you started"), "{err}");
        assert!(may_direct(&Caller::Agent(id("pi:c-kid001")), &id("pi:c-par001"), &s, "stop").is_err(), "a child does not direct its parent");
        assert!(may_direct(&Caller::NoAgent, &id("deepseek:c-other1"), &s, "stop").is_ok(), "the person directs any");
        // Reading: itself and its children, never a stranger.
        assert!(may_read(&parent, &id("pi:c-par001"), &s).is_ok());
        assert!(may_read(&parent, &id("pi:c-kid001"), &s).is_ok());
        assert!(may_read(&parent, &id("deepseek:c-other1"), &s).is_err());
    }

    /// The whole of `new_agent` against a real host: depth, children, the global cap, and the
    /// child's first turn — which carries its task and nothing of its parent's.
    #[test]
    fn new_agent_starts_a_child_with_nothing_of_its_parents_and_meets_every_cap() {
        let host = Host::new(vec![]);
        let attach = json!({ "id": "glue", "name": "Glue", "conversations": true });
        let session = host.handle_from(protocol::ATTACH, &attach, Some(std::process::id())).unwrap()["session"]
            .as_str()
            .unwrap()
            .to_string();
        let poll = || host.handle(protocol::POLL, &json!({ "session": session })).unwrap();
        let finish = |handed: &Value| {
            host.handle(protocol::COMPLETE, &json!({ "session": session, "turn_id": handed["turn_id"] })).unwrap();
        };

        // The parent: an agent of its own, holding a token, with a note waiting for its next turn
        // — the kind of thing a child must not be handed.
        let parent = host.start_agent("glue").unwrap();
        let _answer = host.send_to(&parent, yantrik_harness::Turn::new("plan the release")).unwrap();
        let handed = poll();
        let parent_token = handed["agent_token"].as_str().unwrap().to_string();
        finish(&handed);
        assert!(host.note_for(&parent, "your command `make` finished: exit code 0".into()));
        agents::store().upsert_agent(AgentMeta::new(parent.clone(), "Glue"));

        let caller = Caller::Agent(parent.clone());
        let started = new_agent(&host, &caller, "glue", "write the changelog").unwrap();
        let child = AgentId(started["agent"].as_str().unwrap().to_string());
        assert_eq!(started["parent"], json!(parent.0), "{started}");
        assert!(started["said"].as_str().unwrap().contains("none of your grants"), "{started}");
        let recorded = agents::store().read(|s| s.agent(&child).map(|a| (a.meta.parent.clone(), a.meta.title.clone())));
        assert_eq!(recorded, Some((Some(parent.clone()), "write the changelog".to_string())), "its row says who started it");

        // What the child's harness is handed: its task, the desktop's context and its own token.
        let first = poll();
        assert_eq!(first["conversation"], json!(child.conversation()));
        assert_eq!(first["text"], "write the changelog");
        let child_token = first["agent_token"].as_str().unwrap();
        assert_ne!(child_token, parent_token, "a token of its own");
        let whole = first.to_string();
        assert!(!whole.contains(&parent_token), "nothing of the parent's token");
        assert!(!whole.contains("finished: exit code"), "nor the parent's notes");
        assert!(first["context"].as_str().map_or(true, |c| !c.contains("notes")), "{first}");
        finish(&first);

        // The name a person reads works too; a mind that is not attached is said plainly.
        assert!(new_agent(&host, &caller, "Glue", "second").is_ok());
        let err = new_agent(&host, &caller, "hermes", "x").unwrap_err();
        assert!(err.contains("no mind called `hermes`") && err.contains("glue"), "{err}");
        // A third, then a fourth refused.
        assert!(new_agent(&host, &caller, "glue", "third").is_ok());
        let err = new_agent(&host, &caller, "glue", "fourth").unwrap_err();
        assert!(err.contains("already has 3 agents running"), "{err}");
        // A child cannot start one.
        let err = new_agent(&host, &Caller::Agent(child.clone()), "glue", "grandchild").unwrap_err();
        assert!(err.contains("one level only"), "{err}");

        // The host's own cap: parent + 3 children live, then the person starts two more.
        assert!(new_agent(&host, &Caller::NoAgent, "glue", "fifth").is_ok());
        assert!(new_agent(&host, &Caller::NoAgent, "glue", "sixth").is_ok());
        let err = new_agent(&host, &Caller::NoAgent, "glue", "seventh").unwrap_err();
        assert!(err.contains(&format!("the most is {}", protocol::MAX_LIVE_AGENTS)), "{err}");

        // Stop on the parent stops its children, and frees their places.
        let stopped = stop_agent(&host, &Caller::NoAgent, &parent).unwrap();
        assert_eq!(stopped["children_stopped"].as_array().map(Vec::len), Some(3), "{stopped}");
        let live: Vec<AgentId> = host.agents().into_iter().map(|a| a.id).collect();
        assert!(!live.contains(&parent) && !live.contains(&child), "{live:?}");

        // Nothing anyone was answered carried either token.
        for answer in [started.to_string(), stopped.to_string()] {
            assert!(!answer.contains(&parent_token) && !answer.contains(child_token), "{answer}");
        }
    }

    /// `read_agent` answers with the session as text — a verified card with its exit code, what
    /// the person was asked — and refuses a stranger's session to an agent.
    #[test]
    fn read_agent_gives_the_session_as_text_and_only_to_those_who_may_read_it() {
        let me = id("pi:c-read01");
        let store = agents::store();
        store.upsert_agent(AgentMeta::new(me.clone(), "pi"));
        store.open_turn(&me, "count the photos");
        store.text(&me, "Counting them now.");
        store.command_started(&me, "job-read01", "ls ~/Pictures | wc -l", "/home/me");
        store.command_output(&me, "job-read01", b"4127\r\n");
        store.command_finished(&me, "job-read01", "ls ~/Pictures | wc -l", Some(0), false);
        store.approval_asked(&me, "appr-read01", "files.move");
        store.close_turn(&me, true);

        let read = read_agent(&Caller::Agent(me.clone()), &me, None).unwrap();
        let text = read["text"].as_str().unwrap();
        for said in ["count the photos", "Counting them now.", "[verified call]", "exit 0", "4127", "[asked the person] files.move"] {
            assert!(text.contains(said), "{said:?} missing:\n{text}");
        }
        assert_eq!(read["needs_you"], true, "{read}");
        let err = read_agent(&Caller::Agent(id("deepseek:c-other2")), &me, None).unwrap_err();
        assert!(err.contains("not an agent you started"), "{err}");
        assert!(read_agent(&Caller::NoAgent, &me, Some(&json!(21))).unwrap_err().contains("between 1 and 20"));
        assert!(read_agent(&Caller::NoAgent, &id("pi:c-nobody"), None).unwrap_err().contains("no agent"));
    }

    // ── A recipe's Agent steps ──

    /// The digest of a shipped role's definition, as a run the person started records it.
    fn agreed(role: &str) -> &'static str {
        Box::leak(shipped().find(role).unwrap().digest().into_boxed_str())
    }

    /// An Agent step of a run the person started, agreeing to the role as shipped.
    fn call<'a>(recipe_id: &'a str, role: &'a str, task: &'a str) -> AgentCall<'a> {
        AgentCall {
            recipe_id,
            recipe_name: "Council",
            step: 0,
            attended: true,
            consented: Some(agreed(role)),
            asked_by: None,
            role,
            task,
            context: "",
        }
    }

    /// Finish an agent's open turn the way its harness would: what it said, and done.
    fn answer_as_harness(host: &Host, session: &str, said: &str) {
        let turn = host.handle(protocol::POLL, &json!({ "session": session })).unwrap();
        let id = turn["turn_id"].clone();
        host.handle(protocol::CHUNK, &json!({ "session": session, "turn_id": id, "delta": said })).unwrap();
        host.handle(protocol::COMPLETE, &json!({ "session": session, "turn_id": id })).unwrap();
    }

    fn answered_poll(host: &Host, recipe: &str, agent: &AgentId) -> AgentPoll {
        for _ in 0..100 {
            let heard = poll_for_recipe(Some(host), recipe, agent);
            if heard != AgentPoll::Working {
                return heard;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        AgentPoll::Working
    }

    /// A recipe's Agent step goes through hand_off — the role's first attached mind, its reach
    /// held — and its agent says whose it is: its row, `describe` and its approval cards say
    /// "Council recipe → Reviewer". The step hears the answer from the agent's own session once
    /// the turn ends, and lets the agent go with a line in its pane that says why. Another
    /// recipe can neither hear it nor let it go.
    #[test]
    fn a_recipe_hands_work_through_hand_off_and_hears_its_answer() {
        let host = Host::new(vec![]);
        attach(&host, "pi", true);
        let deepseek = attach(&host, "deepseek", true);
        let started = start_for_recipe(&host, &shipped(), &call("rcp_council_hand", "reviewer", "review the plan"), &mut Asks::default()).unwrap();
        assert_eq!(
            (started.role.as_str(), started.role_name.as_str(), started.mind.as_str(), started.minutes),
            ("reviewer", "Reviewer", "deepseek", 15)
        );
        let agent = AgentId(started.agent.clone());
        let meta = agents::store().read(|s| s.agent(&agent).map(|a| a.meta.clone())).unwrap();
        assert_eq!(meta.recipe, Some(RecipeOrigin { id: "rcp_council_hand".into(), name: "Council".into() }));
        assert_eq!(meta.parent, None, "the person's run: no agent parent");
        assert_eq!(meta.on_behalf(), "Council recipe → Reviewer", "what its row and its cards say");
        assert!(reaches::of(&agent).is_some(), "held to the Reviewer's reach like any hand-off");
        assert_eq!(poll_for_recipe(Some(&host), "rcp_council_hand", &agent), AgentPoll::Working);
        let err = match poll_for_recipe(Some(&host), "rcp_other", &agent) {
            AgentPoll::Failed(why) => why,
            other => panic!("{other:?}"),
        };
        assert!(err.contains("was not started by this recipe"), "{err}");

        answer_as_harness(&host, &deepseek, "Verdict — ship.");
        assert_eq!(answered_poll(&host, "rcp_council_hand", &agent), AgentPoll::Answered("Verdict — ship.".into()));
        release_for_recipe(&host, "rcp_other", &agent, "not yours");
        assert!(host.agents().iter().any(|a| a.id == agent), "another recipe cannot let it go");
        release_for_recipe(&host, "rcp_council_hand", &agent, "Its answer went to the Council recipe; it was let go.");
        assert!(!host.agents().iter().any(|a| a.id == agent), "let go: its place under the cap is free");
        assert_eq!(reaches::of(&agent), None, "and its reach released");
        let pane = agents::store().read(|s| s.transcript(&agent, 3)).unwrap_or_default();
        assert!(pane.contains("Its answer went to the Council recipe; it was let go."), "{pane}");
        assert!(!pane.contains("Stop asked"), "{pane}");
        assert_eq!(
            poll_for_recipe(Some(&host), "rcp_council_hand", &agent),
            AgentPoll::Answered("Verdict — ship.".into()),
            "its answer is still its answer once it is let go"
        );
    }

    /// Depth one holds: an agent a recipe started cannot hand work on, nor start an agent — and a
    /// recipe an agent asked for makes its agents that agent's children.
    #[test]
    fn an_agent_a_recipe_started_cannot_start_agents_of_its_own() {
        let host = Host::new(vec![]);
        attach(&host, "pi", true);
        let started = start_for_recipe(&host, &shipped(), &call("rcp_depth", "coder", "fix it"), &mut Asks::default()).unwrap();
        let coder = Caller::Agent(AgentId(started.agent.clone()));
        let err = hand_off(&host, &coder, &shipped(), "reviewer", "review my fix", "").unwrap_err();
        assert!(err.contains("was started by the Council recipe") && err.contains("one level only"), "{err}");
        let live: Vec<AgentId> = host.agents().into_iter().map(|a| a.id).collect();
        let err = agents::store().read(|s| may_start_child(&AgentId(started.agent.clone()), s, &live)).unwrap_err();
        assert!(err.contains("an agent a recipe started cannot start agents of its own"), "{err}");

        // A run a top-level agent asked for: its agents are that agent's children.
        let asker = new_agent(&host, &Caller::NoAgent, "pi", "plan the week").unwrap();
        let asker = asker["agent"].as_str().unwrap().to_string();
        let mut asked = call("rcp_asked", "planner", "plan it");
        asked.asked_by = Some(asker.as_str());
        let kid = start_for_recipe(&host, &shipped(), &asked, &mut Asks::default()).unwrap();
        let parent = agents::store().read(|s| s.agent(&AgentId(kid.agent.clone())).and_then(|a| a.meta.parent.clone()));
        assert_eq!(parent, Some(AgentId(asker.clone())), "held to its asker's rules: a child");
        // One that is itself a child cannot ask for a run's agents.
        let mut from_child = call("rcp_from_child", "planner", "plan it");
        from_child.asked_by = Some(started.agent.as_str());
        match start_for_recipe(&host, &shipped(), &from_child, &mut Asks::default()) {
            Err(AgentRefusal::Fail(why)) => assert!(why.contains("one level only"), "{why}"),
            other => panic!("{other:?}"),
        }
        for agent in [&started.agent, &kid.agent, &asker] {
            let _ = stop_agent(&host, &Caller::NoAgent, &AgentId(agent.clone()));
        }
    }

    /// The caps. A recipe has at most MAX_CHILDREN agents live: the next waits for a place (its
    /// step waits, and the executor starts it when one answers). The desktop's own cap is a
    /// refusal the recipe fails with, in the host's words. And the recipe executor's own count is
    /// the same number.
    #[test]
    fn a_recipe_holds_three_agents_at_most_and_the_desktops_cap_still_applies() {
        assert_eq!(MAX_CHILDREN, yantrik_companion::recipe_executor::AGENTS_AT_ONCE, "one number for one rule");
        let host = Host::new(vec![]);
        attach(&host, "pi", true);
        let three: Vec<AgentStarted> =
            ["researcher", "red-team", "planner"].iter().map(|r| start_for_recipe(&host, &shipped(), &call("rcp_cap", r, "q"), &mut Asks::default()).unwrap()).collect();
        match start_for_recipe(&host, &shipped(), &call("rcp_cap", "chair", "weigh them"), &mut Asks::default()) {
            Err(AgentRefusal::Wait(why)) => {
                assert!(why.starts_with("a place: the Council recipe already has 3 agents working"), "{why}")
            }
            other => panic!("{other:?}"),
        }
        // Another recipe is not held by this one's cap.
        let other = start_for_recipe(&host, &shipped(), &call("rcp_cap_other", "chair", "weigh"), &mut Asks::default()).unwrap();
        // One of the three let go: a place is free.
        release_for_recipe(&host, "rcp_cap", &AgentId(three[0].agent.clone()), "answered");
        let fourth = start_for_recipe(&host, &shipped(), &call("rcp_cap", "chair", "weigh them"), &mut Asks::default()).unwrap();

        // The desktop's cap: as many live as it runs, and the start fails with its sentence.
        let mut extra = Vec::new();
        while host.agents().len() < protocol::MAX_LIVE_AGENTS {
            extra.push(new_agent(&host, &Caller::NoAgent, "pi", "busy").unwrap()["agent"].as_str().unwrap().to_string());
        }
        // Queued, not raced for: the step waits for a place, needing the person, and starts nothing.
        let before = host.agents().len();
        match start_for_recipe(&host, &shipped(), &call("rcp_cap_full", "scribe", "sum up"), &mut Asks::default()) {
            Err(AgentRefusal::Wait(why)) => {
                assert!(why.contains(&format!("the desktop is running {} agents", protocol::MAX_LIVE_AGENTS)), "{why}")
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(host.agents().len(), before, "nothing was started");
        let mut all: Vec<String> = three.iter().map(|s| s.agent.clone()).collect();
        all.extend([other.agent, fourth.agent]);
        all.extend(extra);
        for agent in all {
            let _ = stop_agent(&host, &Caller::NoAgent, &AgentId(agent));
        }
    }

    /// What the person agreed to when they started the run, and nothing else: a role whose
    /// definition has changed since (a file in ~/.config/yantrik/agents replacing the shipped one,
    /// here with a wider reach) is refused with a sentence, and so is a role the run did not name
    /// when it started. Nothing is started either way.
    #[test]
    fn a_role_whose_definition_changed_since_the_start_is_refused() {
        let host = Host::new(vec![]);
        attach(&host, "pi", true);
        attach(&host, "deepseek", true);
        let agreed_to = agreed("reviewer");
        let wider = Catalog::from_layers(
            &catalog::SHIPPED,
            &[],
        );
        let mut changed = wider.find("reviewer").unwrap().clone();
        changed.reach.ceiling = "sensitive".into();
        changed.reach.surfaces.push("files".into());
        assert_ne!(changed.digest(), agreed_to, "a wider reach is a different definition");
        assert_eq!(wider.find("reviewer").unwrap().digest(), agreed_to, "the same file, the same digest");

        let dir = std::env::temp_dir().join(format!("yantrik-roles-changed-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("reviewer.toml"),
            "id = \"reviewer\"\nname = \"Reviewer\"\npurpose = \"Reviews, and now moves files too.\"\n\
             mind = [\"deepseek\", \"pi\"]\nreturns = \"A verdict.\"\nbrief = \"Review it. Answer in this shape: Verdict.\"\n\
             [reach]\nsurfaces = [\"files\", \"notes\"]\nceiling = \"sensitive\"\n[budget]\nturns = 4\nminutes = 15\n",
        )
        .unwrap();
        let replaced = Catalog::from_layers(&catalog::SHIPPED, &[(dir.clone(), catalog::Source::Person)]);
        assert_ne!(replaced.find("reviewer").unwrap().digest(), agreed_to, "the person's file replaced the role");
        let live_before = host.agents().len();
        match start_for_recipe(&host, &replaced, &call("rcp_digest", "reviewer", "review it"), &mut Asks::default()) {
            Err(AgentRefusal::Fail(why)) => {
                assert!(why.contains("the Reviewer's definition has changed since the person started this run"), "{why}")
            }
            other => panic!("a changed role must not start: {other:?}"),
        }
        let mut unnamed = call("rcp_digest", "coder", "fix it");
        unnamed.consented = None;
        match start_for_recipe(&host, &shipped(), &unnamed, &mut Asks::default()) {
            Err(AgentRefusal::Fail(why)) => assert!(why.contains("not among the roles the person agreed to"), "{why}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(host.agents().len(), live_before, "nothing was started");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A run nobody started at the desk — a trigger, a timer — asks the person before a role above
    /// `safe`: a card naming the recipe and the role, bound to its definition. Allowed, the step
    /// starts it; denied, the step fails. A role at most `safe` starts without a card, and is
    /// written to the record of unasked actions under the recipe's name.
    #[test]
    fn a_run_nobody_started_asks_before_a_role_above_safe() {
        use crate::approvals;
        let host = Host::new(vec![]);
        attach(&host, "pi", true);
        let unattended = |role: &'static str, step: usize| AgentCall {
            recipe_id: "rcp_nightly",
            recipe_name: "Nightly",
            step,
            attended: false,
            consented: None,
            asked_by: None,
            role,
            task: "tidy the build cache",
            context: "",
        };
        let mut asks = Asks::default();
        let before = host.agents().len();
        let mut asked = None;
        for _ in 0..50 {
            match start_for_recipe(&host, &shipped(), &unattended("coder", 1), &mut asks) {
                Err(AgentRefusal::Ask(why)) if asks.contains_key(&("rcp_nightly".to_string(), 1)) => {
                    asked = Some(why);
                    break;
                }
                // The cards on screen are full for a moment: other tests ask too.
                Err(AgentRefusal::Ask(_)) => std::thread::sleep(Duration::from_millis(100)),
                other => panic!("an above-safe role must be asked for: {other:?}"),
            }
        }
        let why = asked.expect("a card was raised");
        assert!(why.starts_with("your Allow on the card: Nightly recipe → Coder"), "{why}");
        assert_eq!(host.agents().len(), before, "nothing started while it waits");
        let id = asks[&("rcp_nightly".to_string(), 1)].clone();
        let card = approvals::card(&id).expect("the card");
        assert_eq!((card.requester.as_str(), card.app.as_str(), card.action.as_str()), ("Nightly recipe", "shell", "hand_off"));
        assert!(card.args.iter().any(|a| a.contains("coder")), "{:?}", card.args);
        assert!(card.purpose.contains("the Coder") && card.purpose.contains("nobody at the desk"), "{}", card.purpose);
        // Still waiting: asked again, the same card.
        assert!(matches!(start_for_recipe(&host, &shipped(), &unattended("coder", 1), &mut asks), Err(AgentRefusal::Ask(_))));
        assert_eq!(asks[&("rcp_nightly".to_string(), 1)], id);
        approvals::grant(&id).unwrap();
        let started = start_for_recipe(&host, &shipped(), &unattended("coder", 1), &mut asks).expect("allowed on its card");
        assert_eq!(started.role, "coder");
        assert!(asks.is_empty(), "its card is spent");

        // Denied: the step fails, saying so, and nothing starts.
        let mut asks = Asks::default();
        for _ in 0..50 {
            if matches!(start_for_recipe(&host, &shipped(), &unattended("researcher", 2), &mut asks), Err(AgentRefusal::Ask(_)))
                && !asks.is_empty()
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        approvals::deny(&asks[&("rcp_nightly".to_string(), 2)].clone()).unwrap();
        match start_for_recipe(&host, &shipped(), &unattended("researcher", 2), &mut asks) {
            Err(AgentRefusal::Fail(why)) => assert!(why.contains("the person denied handing work to the Researcher"), "{why}"),
            other => panic!("{other:?}"),
        }

        // At most safe: no card, and on the record under the recipe's name.
        let chair = start_for_recipe(&host, &shipped(), &unattended("chair", 3), &mut Asks::default()).expect("a safe role starts");
        let recorded = crate::mind_mode::recent(200)
            .into_iter()
            .any(|e| e.requester == "Nightly recipe" && e.action == "hand_off" && e.outcome.contains(&chair.agent));
        assert!(recorded, "the start is on the record of unasked actions, under the recipe's name");
        for agent in [started.agent, chair.agent] {
            let _ = stop_agent(&host, &Caller::NoAgent, &AgentId(agent));
        }
    }

    /// An agent waiting on the person in its own pane makes its recipe need the person; a card of
    /// its that nobody answered is not an answer, whatever it said after.
    #[test]
    fn an_agent_waiting_on_you_needs_you_and_an_unanswered_card_is_no_answer() {
        let agent = AgentId::new("pi", "c-recipe-card");
        let mut meta = AgentMeta::new(agent.clone(), "pi");
        meta.recipe = Some(RecipeOrigin { id: "rcp_card".into(), name: "Build".into() });
        meta.role = shipped().find("coder").map(|r| r.meta());
        agents::store().upsert_agent(meta);
        agents::store().open_turn(&agent, "make the change");
        agents::store().approval_asked(&agent, "appr-recipe-card", "shell.agent_run");
        match poll_for_recipe(None, "rcp_card", &agent) {
            AgentPoll::NeedsYou(why) => {
                assert_eq!(why, "the Coder (pi:c-recipe-card) is waiting for your Allow on a card in its pane: shell.agent_run")
            }
            other => panic!("{other:?}"),
        }
        agents::store().approval_settled(&agent, "appr-recipe-card", agents::ApprovalOutcome::Expired, "Nobody answered — 21:04");
        agents::store().text(&agent, "Changed — nothing; the build needed a card nobody answered.");
        agents::store().close_turn(&agent, true);
        match poll_for_recipe(None, "rcp_card", &agent) {
            AgentPoll::Failed(why) => assert!(why.contains("its card for shell.agent_run went unanswered and expired"), "{why}"),
            other => panic!("an unanswered card is not an answer: {other:?}"),
        }
        // And a turn that ended well with nothing said is no answer either.
        let quiet = AgentId::new("pi", "c-recipe-quiet");
        let mut meta = AgentMeta::new(quiet.clone(), "pi");
        meta.recipe = Some(RecipeOrigin { id: "rcp_card".into(), name: "Build".into() });
        agents::store().upsert_agent(meta);
        agents::store().open_turn(&quiet, "plan it");
        agents::store().close_turn(&quiet, true);
        assert_eq!(poll_for_recipe(None, "rcp_card", &quiet), AgentPoll::Failed("it ended its turn without an answer".into()));
    }

    /// After a restart the store is what is left: a turn cut off by the shell's stop is an answer
    /// that will not come, said with the store's own note; an open turn whose conversation the
    /// host no longer has is gone; an agent the store never had is unknown.
    #[test]
    fn a_recipe_hears_honestly_what_a_restart_left() {
        let host = Host::new(vec![]);
        let cut = AgentId::new("pi", "c-recipe-cut");
        let mut meta = AgentMeta::new(cut.clone(), "pi");
        meta.recipe = Some(RecipeOrigin { id: "rcp_restart".into(), name: "Build".into() });
        agents::store().upsert_agent(meta.clone());
        agents::store().open_turn(&cut, "fix it");
        agents::store().note(&cut, "The shell stopped while this turn was running.");
        agents::store().close_turn(&cut, false);
        match poll_for_recipe(Some(&host), "rcp_restart", &cut) {
            AgentPoll::Failed(why) => assert!(why.contains("The shell stopped while this turn was running."), "{why}"),
            other => panic!("{other:?}"),
        }
        let open = AgentId::new("pi", "c-recipe-open");
        meta.id = open.clone();
        agents::store().upsert_agent(meta);
        agents::store().open_turn(&open, "fix it");
        match poll_for_recipe(Some(&host), "rcp_restart", &open) {
            AgentPoll::Failed(why) => assert!(why.contains("its conversation is gone"), "{why}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(poll_for_recipe(None, "rcp_restart", &open), AgentPoll::Working, "no host yet: it cannot be told");
        match poll_for_recipe(Some(&host), "rcp_restart", &AgentId::new("pi", "c-never")) {
            AgentPoll::Failed(why) => assert!(why.contains("no longer knows"), "{why}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_agent_is_named_the_way_describe_lists_it() {
        assert_eq!(agent_arg(&json!({"agent": " pi:c-7f3a91 "})).unwrap(), id("pi:c-7f3a91"));
        for bad in ["pi", ":c-1", "pi:", ""] {
            assert!(agent_arg(&json!({"agent": bad})).is_err(), "{bad}");
        }
    }
}
