//! Starting an agent, saying more to one, and stopping one — through the harness host.
//!
//! The host issues the conversation (`Host::start_agent`), addresses each turn to it
//! (`Host::send_to`) and stops it (`Host::stop_agent`); this file records what it did in the store
//! and hands the answer to `feed`, which reads it into the agent's session. A harness that holds
//! one conversation has the one agent `<harness>:main` — the same conversation the Lens has with
//! it — and a New agent on it continues that one rather than pretending to open a second. The
//! built-in companion answers in the Lens and holds no agents; the host refuses it, and so does
//! this.
//!
//! Each act comes in two forms: the one the screen calls, on the shell's own host, and `…_on`,
//! which takes the host — what `shell.new_agent` and friends call, and what their tests drive
//! with a host of their own.

use yantrik_harness::{Host, Turn};

use super::model::{title_of, AgentId, RecipeOrigin, RoleMeta, State};

fn host() -> Result<&'static Host, String> {
    crate::wire::harness::host().ok_or_else(|| "the harness host is not running".to_string())
}

fn builtin(harness: &str) -> bool {
    harness == crate::wire::harness::BUILTIN_ID
}

fn turn(text: &str) -> Turn {
    Turn::new(text.to_string()).with_context(crate::wire::chat::desktop_context(&crate::wire::settings::place()))
}

/// Start an agent: a mind and its first prompt. Answers with the agent it became.
pub fn start(mind: &str, prompt: &str) -> Result<AgentId, String> {
    start_on(host()?, mind, prompt, None)
}

/// Start an agent on `host`. `parent` is the agent that asked for it (`shell.new_agent`), which
/// makes it a child: its row says so, Stop on the parent stops it, and it is always a new
/// conversation — never a one-conversation harness's `main`, which is the person's own
/// conversation with that mind in the Lens.
///
/// A child starts with nothing of its parent's: its first turn is `prompt` and the desktop's
/// context, as any agent's is. No grant, request id, note or token of the parent's goes with it,
/// and the host mints its own token for it (design decision 1: "a child starts with no grants").
pub fn start_on(host: &Host, mind: &str, prompt: &str, parent: Option<&AgentId>) -> Result<AgentId, String> {
    start_with(host, mind, prompt, parent, Start::default())
}

/// What more there is to starting an agent as a role from the catalog (`hand_off`).
#[derive(Default)]
pub struct Start<'a> {
    /// Its row's title, when that is not its first prompt: a role's first prompt is its brief and
    /// then the task, and its row is named for the task.
    pub title: Option<&'a str>,
    /// The role it is started as. A role is only ever a conversation of its own — never a
    /// one-conversation harness's `main`, which is the person's own.
    pub role: Option<RoleMeta>,
    /// Run once the conversation exists and before its first turn is sent: where a role's reach is
    /// published, so there is no moment in which the agent can act unheld. An `Err` ends the start,
    /// and the conversation is let go.
    pub before_first_turn: Option<&'a dyn Fn(&AgentId) -> Result<(), String>>,
    /// The recipe handing it the work (an Agent step): its row and its cards say so.
    pub recipe: Option<RecipeOrigin>,
}

/// [`start_on`], with what a role adds.
pub fn start_with(host: &Host, mind: &str, prompt: &str, parent: Option<&AgentId>, how: Start<'_>) -> Result<AgentId, String> {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Err("Say what the agent is to do.".into());
    }
    if builtin(mind) {
        return Err("The built-in companion answers in the Lens; it cannot be started as an agent yet.".into());
    }
    if let Some(role) = &how.role {
        if host.holds_conversations(mind) != Some(true) {
            return Err(format!(
                "{mind} is not attached as a mind that holds a conversation per agent, so it cannot \
                 take the {} role: a role is never started in the person's own conversation.",
                role.name
            ));
        }
    }
    let fresh = how.role.is_some() || how.recipe.is_some();
    let agent = match host.start_agent(mind) {
        Ok(agent) => agent,
        // One conversation, already open: the agent is that conversation — for the person, who
        // is continuing their own. Anything else — the cap, a harness that is gone, or a parent
        // asking for a child on a harness that cannot give it one — is said as the host said it.
        Err(why) => {
            let main = AgentId::new(mind, AgentId::MAIN);
            let continues = host.agents().iter().any(|a| a.id == main && !a.conversations);
            if !continues || parent.is_some() || fresh {
                return Err(why);
            }
            main
        }
    };
    let busy = super::store().read(|s| s.agent(&agent).is_some_and(|a| a.open_turn().is_some()));
    if busy {
        return Err(format!("`{agent}` is still on its last turn; one turn at a time."));
    }
    if let Some(before) = how.before_first_turn {
        if let Err(why) = before(&agent) {
            host.stop_agent(&agent);
            super::reaches::release(&agent);
            return Err(why);
        }
    }
    let answer = match host.send_to(&agent, turn(prompt)) {
        Ok(answer) => answer,
        Err(why) => {
            if parent.is_some() || fresh {
                // Nothing was asked of it: let the conversation go rather than hold a place
                // under the cap for a child that never started.
                host.stop_agent(&agent);
                super::reaches::release(&agent);
            }
            return Err(why);
        }
    };
    let mut meta = super::feed::meta_for(&agent);
    meta.title = title_of(how.title.unwrap_or(prompt));
    meta.parent = parent.cloned();
    meta.role = how.role;
    meta.recipe = how.recipe;
    super::store().upsert_agent(meta);
    super::store().open_turn(&agent, prompt);
    super::feed::record(agent.clone(), answer, false);
    Ok(agent)
}

/// Say something more to an agent.
pub fn send(agent: &AgentId, text: &str) -> Result<(), String> {
    if text.trim().is_empty() {
        return Ok(());
    }
    if builtin(agent.harness()) {
        return Err("The built-in companion answers in the Lens.".into());
    }
    send_on(host()?, agent, text)
}

/// The person's word to an agent, from its pane (#234). A stuck task is interrupted first: the
/// stuck step's commands are killed and its cards withdrawn, and its turn is cancelled without
/// ending the agent, so the word reaches the same mind, with its conversation, as its next turn.
/// A task waiting on the person's answer to its question is not stuck and is never interrupted:
/// the word is turned away with where to answer, and the question stays open (#25).
/// Anything else is an ordinary [`send`], which waits for the turn in flight.
pub fn tell(agent: &AgentId, text: &str) -> Result<(), String> {
    if text.trim().is_empty() {
        return Ok(());
    }
    let host = host()?;
    let now = super::model::now();
    let progress = super::store().read(|s| s.agent(agent).and_then(|a| super::progress::of(a, now)));
    if progress.as_ref().is_some_and(|p| p.question.is_some()) {
        return Err("It is waiting for your answer to its question; answer it on its card, then tell it more.".into());
    }
    let Some(why) = progress.and_then(|p| p.stuck) else { return send_on(host, agent, text) };
    let killed = crate::control_agent_terminal::jobs().kill_agent(agent);
    let withdrawn = crate::approvals::withdraw_for_agent(&agent.0).len();
    host.interrupt(agent);
    let mut note = format!("Interrupted while stuck ({why}) so you could tell it something.");
    if !killed.is_empty() || withdrawn > 0 {
        note.push_str(&format!(" {} command(s) killed, {} card(s) withdrawn.", killed.len(), withdrawn));
    }
    super::store().note(agent, &note);
    // The cancelled turn is closed in the session by its feed as soon as the host settles it,
    // which it has; give that a moment before the next turn opens.
    let until = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while super::store().read(|s| s.agent(agent).is_some_and(|a| a.open_turn().is_some())) {
        if std::time::Instant::now() >= until {
            return Err("It was interrupted, but its last turn has not closed yet; send again in a moment.".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    send_on(host, agent, text)
}

/// Say something more to an agent, on `host`.
pub fn send_on(host: &Host, agent: &AgentId, text: &str) -> Result<(), String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(());
    }
    if builtin(agent.harness()) {
        return Err("The built-in companion answers in the Lens.".into());
    }
    let (busy, gone, spent) = super::store().read(|s| {
        s.agent(agent)
            .map(|a| (a.open_turn().is_some(), a.state == State::HarnessGone, over_budget(a)))
            .unwrap_or((false, false, None))
    });
    if busy {
        return Err("It is still on its last turn; one turn at a time.".into());
    }
    if gone {
        return Err("Its harness is gone. Start it again, then ask.".into());
    }
    if let Some(spent) = spent {
        return Err(spent);
    }
    let answer = host.send_to(agent, turn(text))?;
    super::store().upsert_agent(super::feed::meta_for(agent));
    super::store().open_turn(agent, text);
    super::feed::record(agent.clone(), answer, false);
    Ok(())
}

/// Why a role's agent may not be given another turn: its budget's turns are all used. `None` for
/// an agent with turns left, or with no role.
pub fn over_budget(a: &super::model::Agent) -> Option<String> {
    let role = a.meta.role.as_ref()?;
    // Turns it was asked something in; a turn the shell opened only to say something has no prompt.
    let used = a.turns.iter().filter(|t| !t.prompt.is_empty()).count();
    (used >= role.turns as usize).then(|| {
        format!(
            "`{}` is the {}, whose budget is {} turns, and it has had them all. Start another from \
             the catalog to go on.",
            a.meta.id, role.name, role.turns
        )
    })
}

/// What a Stop came to.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stopped {
    /// Whether the host had a turn, a queued turn or a live conversation to stop.
    pub stopped: bool,
    /// The agent's commands killed, each one's whole process group.
    pub commands: usize,
    /// Approval cards taken back: nobody is waiting on their answers any more.
    pub approvals: usize,
    /// Its children, stopped with it.
    pub children: Vec<AgentId>,
}

/// Stop an agent's work: the host fails the turns waiting for it, settles the one in flight and
/// tells the harness to stop; and every command the shell is running for it is killed, each one's
/// whole process group (`Jobs::kill_agent`).
pub fn stop(agent: &AgentId) -> Result<(), String> {
    if builtin(agent.harness()) {
        return Err("The built-in companion answers in the Lens and cannot be stopped from here.".into());
    }
    stop_on(host()?, agent).map(|_| ())
}

/// Stop an agent on `host`, and the agents it started: Stop on a parent stops its children
/// (design decision 1). One level, because a child cannot start agents of its own.
pub fn stop_on(host: &Host, agent: &AgentId) -> Result<Stopped, String> {
    if builtin(agent.harness()) {
        return Err("The built-in companion answers in the Lens and cannot be stopped from here.".into());
    }
    let mut stopped = stop_one(host, agent);
    let children = super::store().read(|s| s.children_of(agent));
    let live: Vec<AgentId> = host.agents().into_iter().map(|a| a.id).collect();
    for child in children {
        let working = super::store().read(|s| s.agent(&child).is_some_and(|a| a.busy()));
        if !working && !live.contains(&child) {
            continue;
        }
        let theirs = stop_one(host, &child);
        super::store().note(&child, &format!("Stopped with `{agent}`, the agent that started it."));
        stopped.commands += theirs.commands;
        stopped.approvals += theirs.approvals;
        stopped.children.push(child);
    }
    Ok(stopped)
}

/// Let an agent go once its work is taken — a recipe has its answer, or no longer wants it: its
/// conversation ended, its place under the cap freed, its reach released, anything it was waiting
/// on withdrawn — with `why` in its pane rather than "Stop asked". Its pane stays readable.
pub fn let_go(host: &Host, agent: &AgentId, why: &str) -> Stopped {
    let stopped = halt(host, agent);
    super::store().note(agent, why);
    stopped
}

fn stop_one(host: &Host, agent: &AgentId) -> Stopped {
    let stopped = halt(host, agent);
    let note = match (stopped.stopped, stopped.commands) {
        (false, 0) => "Stop asked; nothing was running.".to_string(),
        (_, 0) => "Stop asked.".to_string(),
        (_, 1) => "Stop asked, and its one running command killed.".to_string(),
        (_, n) => format!("Stop asked, and its {n} running commands killed."),
    };
    super::store().note(agent, &note);
    stopped
}

/// Everything a stop does, but the note.
fn halt(host: &Host, agent: &AgentId) -> Stopped {
    let killed = crate::control_agent_terminal::jobs().kill_agent(agent);
    let stopped = host.stop_agent(agent);
    // Stopped, it acts no more; its token names nothing now either.
    super::reaches::release(agent);
    // A card for work that is no longer happening is refused, never granted. The approval store's
    // own tick redraws the Lens, and the pane with it.
    let approvals = crate::approvals::withdraw_for_agent(&agent.0).len();
    Stopped { stopped, commands: killed.len(), approvals, children: Vec::new() }
}

/// Whether an agent can still be spoken to: its harness attached, and — for a harness with
/// conversations — the conversation still live. `<harness>:main` is always there while its harness
/// is attached.
pub fn reachable(agent: &AgentId, live: &[AgentId], attached: &[String]) -> bool {
    let harness = agent.harness();
    !builtin(harness)
        && attached.iter().any(|a| a == harness)
        && (agent.conversation() == AgentId::MAIN || live.contains(agent))
}
