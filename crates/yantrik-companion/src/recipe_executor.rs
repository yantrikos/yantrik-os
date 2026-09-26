//! The recipe executor — the one that runs every recipe, in the shell and anywhere else.
//!
//! One step at a time. [`step`] runs the step a recipe stands at — or the next step inside the
//! Branch it stands in — records what came of it, and says whether there is more to run now
//! ([`Advance::Next`]), whether the recipe waits on its clock or on a person
//! ([`Advance::Blocked`]), or whether it has stopped ([`Advance::Stopped`]). The shell's companion
//! worker calls it once per `ProcessRecipeStep` and signals itself again on `Next`, so a person's
//! message is taken between any two steps, and its clock asks [`due`] which waits are over.
//! [`tick`] runs the same steps in a loop, for a host with no worker (`background::run_think_cycle`).
//!
//! There were two executors. The shell's worker had its own in its command loop and never ran
//! this one, so ThinkCited, Validate, Render and the data steps were passed through, and a Branch
//! was marked taken without either side running. That one is gone (#176): the shell runs this.
//!
//! Where a recipe is, beyond its step pointer, is kept in its variables, so a restart loses
//! nothing: `_wait` (what it waits on, and when a timer wakes), `_branch` (the Branch arms it is
//! inside), `_trail` (what each step did, run by run: the way a Branch went, a loop's rounds) and
//! `_since_wait` (steps run since it last waited, which [`STEP_BUDGET`] bounds).
//!
//! What a step needs from outside — the store, the tools, the model, a way to tell the person —
//! comes through [`RecipeHost`]. `CompanionService` is the host in the shell; a test fakes one.
//!
//! # Agent steps
//!
//! An Agent step hands a turn to a role from the agent catalog, through an [`AgentHook`] the shell
//! installs (the companion does not know the catalog or the Agents screen; the shell's hook calls
//! its own `hand_off`). It never waits in the step: the agent is started, `_agents` records it,
//! and the recipe goes on. The first step that reads the agent's `store_as` — or the end of the
//! recipe — waits for its answer ([`WaitRecord::agents`]), so Agent steps that do not read each
//! other's answers work at the same time, at most [`AGENTS_AT_ONCE`] of them. An Agent step
//! inside a Branch's arm is the Branch's own (#194): `_agents` keys it under the Branch
//! ([`crate::recipe::agent_key`]), the arm goes on while it works, and the Branch joins every
//! agent it started — waits for their answers — before it closes. Every step, and the
//! clock every [`CLOCK_SECS`] while one waits, asks the hook how each agent is doing: an answer is
//! kept and the agent let go; an agent that fails, or runs past its role's minutes, fails the
//! recipe with the reason. A restart loses nothing: `_agents` is in the store, and the shell reads
//! a finished agent's answer from its saved session — or says it is gone.
//!
//! What the person agreed to is the run's leave ([`crate::recipe::Leave`]): who started it, and
//! the definition of each role it names as it was then. The hook holds each start to it — a role
//! whose definition changed since is not started. A run nobody started at the desk (a trigger, a
//! timer) has no leave, and the hook asks the person on a card before any role above `safe`. A
//! start the shell puts off — that card, or no place under a cap the recipe does not own — waits
//! at its step, needing the person, for at most [`PUT_OFF_MOST_SECS`]; so does an agent waiting
//! on the person in its own pane. None of it ever reads as done.

use std::collections::HashMap;

use rusqlite::Connection;
use serde_json::json;
use yantrik_ml::{ChatMessage, GenerationConfig};

use crate::companion::CompanionService;
use crate::recipe::{
    agent_key, agent_key_step, agent_key_sub, agent_runs, arm_list, blocked_on_agents, blocks_on_agents, branch_agents_working,
    branch_done, branch_frames, choose, clock_text, resolve_vars, resolve_vars_in_json, role_display, save_agent_runs, waited_on,
    wakes_at, AggregateOp, AgentRun, ErrorAction, FilterOp, Frame, Recipe, RecipeStatus, RecipeStep, RecipeStore,
    StoredStep, Trail, WaitRecord, Waited, BRANCH_VAR, CANCELLED, SINCE_WAIT_VAR, STEP_BUDGET_VAR, UNREADABLE,
    WAIT_VAR,
};

type Vars = HashMap<String, serde_json::Value>;

/// How many steps a recipe may run without a pause — a timer or a question — before it is
/// stopped as a loop that never ends. A recipe that needs more sets `_step_budget`.
pub const STEP_BUDGET: u64 = 100;

/// How often the worker's clock asks [`due`] for waits that are over, in seconds.
pub const CLOCK_SECS: u64 = 5;

/// Steps one recipe may take per [`tick`].
const MAX_STEPS_PER_TICK: usize = 10;

/// Maximum result size stored per step (bytes).
const MAX_RESULT_SIZE: usize = 4000;

/// The most agents one recipe has working at once. An Agent step past that waits for one of them
/// to answer. The same number as the shell's `control_agents::MAX_CHILDREN`, which holds any one
/// agent to three running children; a test in yantrik-ui keeps the two equal.
pub const AGENTS_AT_ONCE: usize = 3;

/// How much of an agent's answer a recipe keeps: three of them, labelled, fit in the 32 KiB of
/// context a hand-off carries — a Council's Chair reads all three.
pub const ANSWER_KEPT: usize = 8 * 1024;

/// How long past its role's own minutes a recipe still waits for an agent. The shell stops the
/// agent when its minutes run out and its turn ends then; this is the recipe's own backstop.
pub const ANSWER_GRACE_SECS: u64 = 120;

/// How long an Agent step waits for a start the shell put off — the person's Allow on a card, a
/// place under the desktop's cap or the asking agent's — before the recipe fails, saying why.
pub const PUT_OFF_MOST_SECS: u64 = 30 * 60;

/// Said when a step needs agents and nothing in this host can start them.
const NO_HOOK: &str = "This step hands work to an agent, and nothing here can start one: only the \
                       desktop shell runs Agent steps (it installs the hook that hands work to the \
                       catalog's roles).";

/// What a step needs from outside the store's rows.
pub trait RecipeHost {
    /// The recipe store, for one call. Not held across a step: a tool the recipe runs opens it too.
    fn with_conn<R>(&self, f: impl FnOnce(&Connection) -> R) -> R;
    /// Run a tool by name, with no model in the loop.
    fn run_tool(&mut self, name: &str, args: &serde_json::Value) -> String;
    /// One call to the model. `precise` asks for a low temperature, for output that is parsed.
    fn generate(&mut self, system: &str, prompt: &str, precise: bool) -> Result<String, String>;
    /// Tell the person something, as the companion's own message.
    fn notify(&mut self, recipe_id: &str, text: &str);
    /// Who the model speaks as.
    fn persona(&self) -> String;
    /// Now, in unix seconds.
    fn now(&self) -> f64 {
        now_ts()
    }
    /// What starts and hears a recipe's agents, when this host has one: the shell installs it.
    /// None, and an Agent step fails saying so.
    fn agent_hook(&mut self) -> Option<&mut (dyn AgentHook + 'static)> {
        None
    }
}

// ── Agents: the hook the shell installs ──

/// One Agent step's hand-off, as the executor asks the hook for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCall<'a> {
    /// The run handing the work over, and its name: what the agent's row and its approval cards
    /// say it works for ("Council recipe → Reviewer").
    pub recipe_id: &'a str,
    pub recipe_name: &'a str,
    /// The Agent step, 0-based: a card raised for it is its own. For a step inside a Branch's
    /// arm, the Branch's index — the card hangs on the Branch's row, and a recipe raises one
    /// put-off card at a time (one `_wait`), so the arm's own agents take turns on it (#194).
    pub step: usize,
    /// Someone at the desk started the run — the Recipes screen's Start, or `shell.run_recipe`,
    /// which asks the person in `ask` mode — and that start is its consent. False for a run with
    /// no leave (a trigger, a timer): the hook asks the person before any role above `safe`.
    pub attended: bool,
    /// For an attended run: the digest of this role's definition when the person agreed to it,
    /// or None when the run did not name the role then. The hook starts nothing else.
    pub consented: Option<&'a str>,
    /// The agent that asked for the run ([`crate::recipe::Leave::agent`]): the recipe's agents
    /// are its children, held to its rules. None when the person started the run.
    pub asked_by: Option<&'a str>,
    /// The role, its task and what it should read first, `{{var}}`s already filled in.
    pub role: &'a str,
    pub task: &'a str,
    pub context: &'a str,
    /// Why the previous try at this step was put off, when it was: the durable wait's own words,
    /// which a restart keeps. The hook reads it to know a re-ask — a card it raised before a
    /// desktop restart went down with the desktop, and the card it raises now should say so.
    pub put_off: Option<&'a str>,
}

/// What the hook started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentStarted {
    /// The agent, `<mind>:<conversation>`.
    pub agent: String,
    /// The role's id and its name as a person reads it.
    pub role: String,
    pub role_name: String,
    /// The mind it runs on.
    pub mind: String,
    /// The role's budget in minutes: when the recipe stops waiting for it, with
    /// [`ANSWER_GRACE_SECS`] over.
    pub minutes: u64,
}

/// Why the hook did not start an agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentRefusal {
    /// Not now: no place for it — the desktop runs as many agents as it may, or the agent that
    /// asked for the run has as many children as it may. The start queues rather than racing the
    /// person's own for a place: the step waits, needing the person, and asks again each tick.
    Wait(String),
    /// Not yet: the person has been asked, on a card naming the recipe and the role, and has not
    /// answered. The step waits, needing the person, and asks again each tick.
    Ask(String),
    /// Not at all: no such role, no mind for it attached, a role whose definition changed since
    /// the person agreed to it, a card denied or left to expire, a rule the asking agent is held
    /// to. The recipe fails with this sentence.
    Fail(String),
}

/// How an agent a recipe started is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentPoll {
    Working,
    /// Working, and waiting on the person in its own pane — an approval card, a command at a
    /// prompt — for this.
    NeedsYou(String),
    /// Its turn ended well, with this answer.
    Answered(String),
    /// It will not answer: its turn failed or was stopped, or it is gone — a restart, a harness
    /// that went away — and why.
    Failed(String),
}

/// What the shell installs so a recipe can hand work to the agent catalog. Narrow on purpose: the
/// companion knows nothing of the catalog, the Agents screen or the grades; the shell's hook goes
/// through its own `hand_off`, which holds every rule a mind's hand-off meets.
///
/// None of these may block: they run on the worker that also answers the person.
pub trait AgentHook: Send {
    /// Start the role on the task.
    fn start(&mut self, call: &AgentCall<'_>) -> Result<AgentStarted, AgentRefusal>;
    /// How `agent`, which `recipe_id` started, is doing. An agent that recipe did not start is
    /// [`AgentPoll::Failed`]: a recipe's variables are its own to write, and they are no way to
    /// read another's work.
    fn poll(&mut self, recipe_id: &str, agent: &str) -> AgentPoll;
    /// Let `agent` go — its answer is taken, or the recipe no longer wants it — saying why in its
    /// pane. Only an agent `recipe_id` started; anything else is left alone.
    fn release(&mut self, recipe_id: &str, agent: &str, why: &str);
}

/// What came of one call to [`step`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Advance {
    /// A step ran and there is more to run: signal again.
    Next,
    /// Waiting on its clock or on a person, or paused: the clock, the answer or a resume moves it.
    Blocked,
    /// Finished, failed, cancelled — or never started. Nothing to do.
    Stopped,
}

/// Run the next step of one recipe.
pub fn step<H: RecipeHost>(host: &mut H, recipe_id: &str) -> Advance {
    let loaded = host.with_conn(|c| {
        RecipeStore::get(c, recipe_id).map(|r| (r, RecipeStore::get_steps(c, recipe_id), RecipeStore::get_vars(c, recipe_id)))
    });
    let Some((recipe, steps, mut vars)) = loaded else {
        tracing::warn!(recipe_id, "Recipe not found");
        return Advance::Stopped;
    };
    let now = host.now();

    // Its agents first: an answer that has come is kept and its agent let go, an agent that will
    // not answer stops the recipe, and a recipe stopped some other way — a cancel, a failure — lets
    // the agents it still has working go.
    if agent_runs(&vars).values().any(AgentRun::working) {
        match recipe.status {
            RecipeStatus::Running | RecipeStatus::Waiting => {
                if let Some(stopped) = settle_agents(host, &recipe, now) {
                    return stopped;
                }
                vars = host.with_conn(|c| RecipeStore::get_vars(c, recipe_id));
            }
            RecipeStatus::Done | RecipeStatus::Failed => {
                let why = if recipe.error_message.as_deref() == Some(CANCELLED) {
                    format!("The {} recipe was cancelled, so its work is no longer wanted.", recipe.name)
                } else {
                    format!("The {} recipe stopped, so its work is no longer wanted.", recipe.name)
                };
                release_agents(host, recipe_id, &why);
                return Advance::Stopped;
            }
            RecipeStatus::Pending | RecipeStatus::Paused => {}
        }
    }

    match recipe.status {
        RecipeStatus::Running => {}
        // Only its clock or its answer moves a waiting recipe. A signal left over from an earlier
        // chain used to walk it on at once: past a timer, or past a question with the answer unbound.
        RecipeStatus::Waiting => {
            let waited = waited_on(&recipe, &steps, &vars);
            // Waiting on its agents: on again once what it waits for has come.
            if let Some(w) = waited.as_ref().filter(|w| w.agents) {
                // A start the shell put off: asked again — never past its time, and never read
                // as done.
                if let Some(why) = &w.put_off {
                    let here = steps.iter().find(|s| s.step_index == w.step).map(|s| s.step.clone());
                    if now - w.since >= PUT_OFF_MOST_SECS as f64 {
                        let said = format!(
                            "Its Agent step waited {} and was not started: it was waiting for {why}.",
                            crate::recipe_view::duration(PUT_OFF_MOST_SECS)
                        );
                        // A put-off start inside an arm takes the Branch down with it, as a
                        // failed arm step does (#194). Only an Agent step's start writes
                        // `put_off`, so a Branch here means the wait is one of its arm's.
                        return if matches!(here, Some(RecipeStep::Branch { .. })) {
                            fail_in_arm(host, &recipe, w.step, &said)
                        } else {
                            fail(host, &recipe, w.step, &said, &said)
                        };
                    }
                    if blocked_on_agents(&steps, w.step, &vars).is_some() {
                        return Advance::Blocked;
                    }
                    match here {
                        Some(here @ RecipeStep::Agent { .. }) => {
                            return agent_step(host, &recipe, w.step, &here, &vars, now, Some(w.since), Some(why.as_str()))
                        }
                        // The step is a Branch, so the put-off start is an Agent step inside
                        // its arm: asked again from where the Branch stands (#194).
                        Some(here @ RecipeStep::Branch { .. }) => {
                            return branch(host, &recipe, &steps, w.step, &here, &vars, now, Some((w.since, why.as_str())))
                        }
                        _ => {}
                    }
                }
                if blocked_on_agents(&steps, w.step, &vars).is_some() {
                    return Advance::Blocked;
                }
                wake(host, &recipe, waited.as_ref());
                return Advance::Next;
            }
            if waited.as_ref().is_some_and(|w| !w.is_over(now)) {
                return Advance::Blocked;
            }
            wake(host, &recipe, waited.as_ref());
            return Advance::Next;
        }
        RecipeStatus::Paused => return Advance::Blocked,
        RecipeStatus::Pending | RecipeStatus::Done | RecipeStatus::Failed => return Advance::Stopped,
    }

    let cur = recipe.current_step;
    // A step that reads an answer still coming — or an Agent step with no place for its agent, or
    // the end with answers still out — waits for its agents, here, without running.
    if let Some(block) = blocked_on_agents(&steps, cur, &vars) {
        begin_wait(
            host,
            recipe_id,
            WaitRecord { step: cur, inner: Vec::new(), since: now, until: None, agents: true, put_off: None },
            cur,
        );
        tracing::info!(recipe_id, step = cur, waits_on = ?block, "Recipe waits on its agents");
        return Advance::Blocked;
    }
    if cur >= steps.len() {
        finish(host, &recipe, &steps, &vars);
        return Advance::Stopped;
    }

    let since = vars.get(SINCE_WAIT_VAR).and_then(|v| v.as_u64()).unwrap_or(0);
    let budget = vars.get(STEP_BUDGET_VAR).and_then(|v| v.as_u64()).unwrap_or(STEP_BUDGET);
    if since >= budget {
        stop_spinning(host, &recipe, &steps, &vars, budget);
        return Advance::Stopped;
    }
    set(host, recipe_id, SINCE_WAIT_VAR, json!(since + 1));

    let here = steps[cur].step.clone();
    tracing::info!(recipe_id, step = cur, kind = crate::recipe_view::kind_of(&here), "Executing recipe step");
    if matches!(here, RecipeStep::Branch { .. }) {
        return branch(host, &recipe, &steps, cur, &here, &vars, now, None);
    }
    if matches!(here, RecipeStep::Agent { .. }) {
        return agent_step(host, &recipe, cur, &here, &vars, now, None, None);
    }

    let mut trail = Trail::read(&vars);
    let first_visit = trail.runs(cur) == 0;
    trail.ran(cur);
    let did = perform(host, recipe_id, &here, &vars, first_visit, now);
    match &did {
        Did::Went(_) if matches!(here, RecipeStep::JumpIf { .. }) => trail.went(cur, "continued"),
        Did::Jump(_) => trail.jumped(cur),
        _ => {}
    }
    save_trail(host, recipe_id, &trail);

    match did {
        Did::Went(result) => {
            host.with_conn(|c| {
                RecipeStore::complete_step(c, recipe_id, cur, &result);
                RecipeStore::update_status(c, recipe_id, &RecipeStatus::Running, cur + 1);
            });
            Advance::Next
        }
        Did::Jump(target) => {
            host.with_conn(|c| RecipeStore::complete_step(c, recipe_id, cur, "jumped"));
            go_to(host, recipe_id, cur, target);
            Advance::Next
        }
        Did::Sleep(until) => {
            host.with_conn(|c| RecipeStore::complete_step(c, recipe_id, cur, "waiting"));
            begin_wait(host, recipe_id, WaitRecord { step: cur, inner: Vec::new(), since: now, until: Some(until), agents: false, put_off: None }, cur + 1);
            Advance::Blocked
        }
        Did::Ask => {
            host.with_conn(|c| RecipeStore::complete_step(c, recipe_id, cur, "asked"));
            begin_wait(host, recipe_id, WaitRecord { step: cur, inner: Vec::new(), since: now, until: None, agents: false, put_off: None }, cur + 1);
            Advance::Blocked
        }
        Did::Failed(err) => recover(host, &recipe, &steps, cur, &err, &on_error_of(&here), &vars),
    }
}

/// Step one recipe until it waits, stops, or has taken `max` steps. Returns the steps it took.
pub fn run<H: RecipeHost>(host: &mut H, recipe_id: &str, max: usize) -> usize {
    let mut taken = 0;
    while taken < max {
        taken += 1;
        if step(host, recipe_id) != Advance::Next {
            break;
        }
    }
    taken
}

/// What the clock should move at `now`: first the recipes whose triggers are due — a schedule
/// whose time has come, or a leader's completion that chains another recipe (#187) — then every
/// recipe running (a signal lost to a restart is not a recipe lost), every one whose wait is over
/// or that has an agent working, and every finished one that still names an agent as working, so
/// the agent is let go.
pub fn due_at(conn: &Connection, now: f64) -> Vec<String> {
    let mut ids = RecipeStore::fire_due_triggers_at(conn, now);
    for id in RecipeStore::get_resumable(conn)
        .into_iter()
        .chain(RecipeStore::get_expired_waiting_at(conn, now))
        .chain(RecipeStore::finished_with_agents_working(conn))
    {
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    ids
}

/// What the clock should move now ([`due_at`] on the machine's clock).
pub fn due(conn: &Connection) -> Vec<String> {
    due_at(conn, now_ts())
}

/// Run what is due, a few steps each, for a host with no worker to signal
/// (`background::run_think_cycle`). Returns the steps taken.
pub fn tick(service: &mut CompanionService) -> usize {
    let due = due(&service.db.conn());
    let mut taken = 0;
    for id in due {
        taken += run(service, &id, MAX_STEPS_PER_TICK);
    }
    if taken > 0 {
        tracing::info!(taken, "Recipe executor tick complete");
    }
    taken
}

// ── What a step did ──

enum Did {
    /// Done, with what to record as its result.
    Went(String),
    /// Go to this step.
    Jump(usize),
    /// Wait for the clock, until this unix time.
    Sleep(f64),
    /// Wait for a person's answer.
    Ask,
    Failed(String),
}

/// Run one step that is not a Branch.
fn perform<H: RecipeHost>(host: &mut H, id: &str, step: &RecipeStep, vars: &Vars, first_visit: bool, now: f64) -> Did {
    match step {
        RecipeStep::Tool { tool_name, args, store_as, .. } => {
            let out = host.run_tool(tool_name, &resolve_vars_in_json(args, vars));
            if is_tool_error(&out) {
                return Did::Failed(out);
            }
            let kept = truncate(&out);
            set(host, id, store_as, json!(kept));
            Did::Went(kept)
        }
        RecipeStep::Think { prompt, store_as, fallback_template } => {
            let system = format!(
                "You are {}, a personal AI companion. Answer based ONLY on the provided data. Never invent \
                 prices, ratings, or availability. If data is missing, say so. Be concise.",
                host.persona()
            );
            let answer = host
                .generate(&system, &resolve_vars(prompt, vars), false)
                .map(|t| strip_think_tags(&t))
                .and_then(|t| if t.is_empty() { Err("the model gave an empty answer".to_string()) } else { Ok(t) });
            let text = match (answer, fallback_template) {
                (Ok(text), _) => text,
                (Err(_), Some(template)) => resolve_vars(template, vars),
                (Err(e), None) => return Did::Failed(format!("The model did not answer: {e}")),
            };
            let kept = truncate(&text);
            set(host, id, store_as, json!(kept));
            Did::Went(kept)
        }
        RecipeStep::JumpIf { condition, target_step } => {
            if condition.evaluate(vars) {
                Did::Jump(*target_step)
            } else {
                Did::Went("continued".into())
            }
        }
        RecipeStep::WaitFor { condition, timeout_secs } => match wakes_at(condition, *timeout_secs, now) {
            Some(until) => Did::Sleep(until),
            None => Did::Went("its time had come".into()),
        },
        RecipeStep::Notify { message } if message.starts_with(UNREADABLE) => {
            Did::Failed(format!("This step's definition could not be read: {}", &message[UNREADABLE.len()..]))
        }
        RecipeStep::Notify { message } => {
            let said = resolve_vars(message, vars);
            host.notify(id, &said);
            Did::Went(truncate(&said))
        }
        RecipeStep::AskUser { question, store_as, choices } => {
            // Answered before it was reached — a caller's variables — on its first visit only: a
            // loop back to a question asks it again.
            if first_visit {
                if let Some(given) = vars.get(store_as).filter(|v| is_answer(v)) {
                    return Did::Went(value_text(given));
                }
            }
            let mut text = resolve_vars(question, vars);
            for (i, c) in choices.as_deref().unwrap_or_default().iter().enumerate() {
                text.push_str(&format!("\n{}. {}", i + 1, resolve_vars(c, vars)));
            }
            host.notify(id, &text);
            Did::Ask
        }
        RecipeStep::ThinkCited { prompt, store_as, source_vars } => think_cited(host, id, prompt, store_as, source_vars, vars),
        RecipeStep::Validate { input_var, store_as } => validate(host, id, input_var, store_as, vars),
        RecipeStep::Render { input_var, store_as, format } => render(host, id, input_var, store_as, format, vars),
        RecipeStep::Format { template, store_as, .. } => {
            let kept = truncate(&resolve_vars(template, vars));
            set(host, id, store_as, json!(kept));
            Did::Went(kept)
        }
        RecipeStep::Filter { input_var, field, op, value, store_as } => filter(host, id, input_var, field, op, value, store_as, vars),
        RecipeStep::Sort { input_var, by_field, descending, store_as } => sort(host, id, input_var, by_field, *descending, store_as, vars),
        RecipeStep::Aggregate { input_var, op, field, store_as } => aggregate(host, id, input_var, op, field.as_deref(), store_as, vars),
        RecipeStep::Extract { input_var, pattern, store_as } => extract(host, id, input_var, pattern, store_as, vars),
        RecipeStep::Branch { .. } => Did::Failed("a Branch is run by its arms, not as one step".into()),
        // At the top it is `agent_step`'s; inside an arm, `agent_in_arm`'s — the executor
        // starts the agent, `perform` never runs one.
        RecipeStep::Agent { .. } => Did::Failed("an Agent step is started by the executor, not run as one step".into()),
    }
}

// ── Agent: a turn handed to a catalog role ──

/// Start the Agent step's agent through the hook and go on; its answer is waited for where it is
/// read ([`blocked_on_agents`]). `put_off_since` and `put_off_why`: asked again from a wait the
/// shell put it in — since then, and for the reason it gave, which outlives a restart.
fn agent_step<H: RecipeHost>(
    host: &mut H,
    recipe: &Recipe,
    cur: usize,
    here: &RecipeStep,
    vars: &Vars,
    now: f64,
    put_off_since: Option<f64>,
    put_off_why: Option<&str>,
) -> Advance {
    let RecipeStep::Agent { role, prompt, store_as, context } = here else {
        return fail(host, recipe, cur, "not an Agent step", "not an Agent step");
    };
    let id = recipe.id.as_str();
    // The person's leave, if someone at the desk started this run: whose it is, and the roles
    // they agreed to. None: a run nobody started at the desk, and the hook asks.
    let leave = host.with_conn(|c| RecipeStore::agents_allowed(c, id));
    let role = resolve_vars(role, vars).trim().to_string();
    if role.is_empty() || role.contains("{{") {
        let why = format!("This step names no role (`{role}`): give it one of the catalog's, or the variable that holds one.");
        return fail(host, recipe, cur, &why, &why);
    }
    let task = resolve_vars(prompt, vars);
    let context = context.as_deref().map(|c| resolve_vars(c, vars)).unwrap_or_default();
    let call = AgentCall {
        recipe_id: id,
        recipe_name: &recipe.name,
        step: cur,
        attended: leave.is_some(),
        consented: leave.as_ref().and_then(|l| l.roles.get(&role)).map(String::as_str),
        asked_by: leave.as_ref().and_then(|l| l.agent.as_deref()),
        role: &role,
        task: &task,
        context: &context,
        put_off: put_off_why,
    };
    let started = match host.agent_hook() {
        Some(hook) => hook.start(&call),
        None => return fail(host, recipe, cur, NO_HOOK, NO_HOOK),
    };
    match started {
        Ok(s) => {
            let mut trail = Trail::read(vars);
            trail.ran(cur);
            save_trail(host, id, &trail);
            let until = now + (s.minutes * 60 + ANSWER_GRACE_SECS) as f64;
            tracing::info!(recipe_id = id, step = cur, agent = %s.agent, role = %s.role, "Recipe handed work to an agent");
            let run = AgentRun {
                role: s.role,
                role_name: s.role_name,
                mind: s.mind,
                agent: s.agent,
                store_as: store_as.clone(),
                since: now,
                until,
                state: AgentRun::WORKING.into(),
                needs_you: None,
            };
            host.with_conn(|c| {
                let mut runs = agent_runs(&RecipeStore::get_vars(c, id));
                runs.insert(cur.to_string(), run);
                save_agent_runs(c, id, &runs);
                RecipeStore::delete_var(c, id, WAIT_VAR);
                RecipeStore::update_status(c, id, &RecipeStatus::Running, cur + 1);
            });
            Advance::Next
        }
        Err(AgentRefusal::Wait(why) | AgentRefusal::Ask(why)) => {
            tracing::info!(recipe_id = id, step = cur, why = %why, "An Agent step's start was put off");
            let record = WaitRecord {
                step: cur,
                inner: Vec::new(),
                since: put_off_since.unwrap_or(now),
                until: None,
                agents: true,
                put_off: Some(why),
            };
            begin_wait(host, id, record, cur);
            Advance::Blocked
        }
        Err(AgentRefusal::Fail(why)) => {
            let said = format!("The {} could not be started: {why}", role_display(&role));
            fail(host, recipe, cur, &said, &said)
        }
    }
}

/// Ask the hook about every agent the recipe has working: keep each answer that has come (in its
/// step's `store_as`, the step ticked with its head) and let its agent go. An agent that failed,
/// is gone, or has run past its role's minutes fails the recipe: returned as `Some(Stopped)`.
fn settle_agents<H: RecipeHost>(host: &mut H, recipe: &Recipe, now: f64) -> Option<Advance> {
    let id = recipe.id.as_str();
    let mut runs = host.with_conn(|c| agent_runs(&RecipeStore::get_vars(c, id)));
    let mut changed = false;
    let mut failed: Option<(String, String, String)> = None;
    for (step, run) in runs.iter_mut().filter(|(_, r)| r.working()) {
        let heard = match host.agent_hook() {
            Some(hook) => hook.poll(id, &run.agent),
            None => AgentPoll::Failed("nothing here can hear its answer: only the desktop shell runs Agent steps".into()),
        };
        // Waiting on the person in its own pane: still working, and the recipe says so.
        let needs_you = match &heard {
            AgentPoll::NeedsYou(why) => Some(why.clone()),
            _ => None,
        };
        if matches!(heard, AgentPoll::Working | AgentPoll::NeedsYou(_)) && run.needs_you != needs_you {
            run.needs_you = needs_you;
            changed = true;
        }
        match heard {
            AgentPoll::Working | AgentPoll::NeedsYou(_) if now < run.until => {}
            AgentPoll::Working | AgentPoll::NeedsYou(_) => {
                let minutes = ((run.until - run.since).max(0.0) as u64).saturating_sub(ANSWER_GRACE_SECS) / 60;
                run.state = AgentRun::FAILED.into();
                let waiting_on_you = run.needs_you.as_deref().map(|why| format!(" It was waiting for you: {why}.")).unwrap_or_default();
                failed = Some((step.clone(), run.agent.clone(), format!(
                    "The {} ({}) did not answer within its {minutes} minutes.{waiting_on_you}",
                    run.role_name, run.agent
                )));
                changed = true;
                break;
            }
            AgentPoll::Answered(text) => {
                let kept = keep_answer(&text);
                let head = crate::recipe_view::head(&kept, 200);
                host.with_conn(|c| {
                    RecipeStore::set_var(c, id, &run.store_as, &json!(kept));
                    // At the top of the recipe, the step's row gets the answer's head, as
                    // before. An arm Agent step's row is the Branch's, and it closes with the
                    // arm's name: the step's own outcome goes to the trail, where its
                    // "waiting" was recorded when it started (#194). An agent deeper in
                    // nested Branches has no `subs` entry of its own — the nested Branch's
                    // one "done" covers it.
                    if let Some(index) = agent_key_sub(step) {
                        let mut trail = Trail::read(&RecipeStore::get_vars(c, id));
                        trail.settle_sub(agent_key_step(step).unwrap_or_default(), index, "waiting", "answered");
                        trail.save_to(c, id);
                    } else if !step.contains(':') {
                        if let Some(top) = agent_key_step(step) {
                            RecipeStore::complete_step(c, id, top, &head);
                        }
                    }
                });
                if let Some(hook) = host.agent_hook() {
                    hook.release(id, &run.agent, &format!("Its answer went to the {} recipe; it was let go.", recipe.name));
                }
                run.state = AgentRun::ANSWERED.into();
                run.needs_you = None;
                changed = true;
                tracing::info!(recipe_id = id, step = %step, agent = %run.agent, "An agent answered its recipe");
            }
            AgentPoll::Failed(why) => {
                run.state = AgentRun::FAILED.into();
                failed = Some((step.clone(), run.agent.clone(), format!("The {} ({}) did not answer: {why}", run.role_name, run.agent)));
                changed = true;
                break;
            }
        }
    }
    if changed {
        host.with_conn(|c| save_agent_runs(c, id, &runs));
    }
    let (step, agent, why) = failed?;
    if let Some(hook) = host.agent_hook() {
        hook.release(id, &agent, &format!("The {} recipe stopped waiting for it.", recipe.name));
    }
    // An agent inside a Branch's arm: the recipe fails at the Branch, its arm step's outcome
    // is recorded, and the frames go with the failure — a failed recipe leaves no `_branch`
    // behind (#194).
    if step.contains(':') {
        host.with_conn(|c| {
            if let Some(index) = agent_key_sub(&step) {
                let mut trail = Trail::read(&RecipeStore::get_vars(c, id));
                trail.settle_sub(agent_key_step(&step).unwrap_or_default(), index, "waiting", "failed");
                trail.save_to(c, id);
            }
            RecipeStore::delete_var(c, id, BRANCH_VAR);
        });
    }
    let top = agent_key_step(&step).unwrap_or(recipe.current_step);
    Some(fail(host, recipe, top, &why, &why))
}

/// Let every agent the recipe still has working go, saying why in each one's pane. Returns how
/// many there were. The recipe failed, was cancelled, or stopped for its step budget.
pub fn release_agents<H: RecipeHost>(host: &mut H, recipe_id: &str, why: &str) -> usize {
    let mut runs = host.with_conn(|c| agent_runs(&RecipeStore::get_vars(c, recipe_id)));
    let mut let_go = 0;
    for run in runs.values_mut().filter(|r| r.working()) {
        if let Some(hook) = host.agent_hook() {
            hook.release(recipe_id, &run.agent, why);
        }
        run.state = AgentRun::RELEASED.into();
        let_go += 1;
    }
    if let_go > 0 {
        host.with_conn(|c| save_agent_runs(c, recipe_id, &runs));
        tracing::info!(recipe_id, let_go, "A recipe's agents were let go");
    }
    let_go
}

/// An answer as the recipe keeps it: whole up to [`ANSWER_KEPT`], cut there saying so.
fn keep_answer(text: &str) -> String {
    let text = text.trim();
    if text.len() <= ANSWER_KEPT {
        return text.to_string();
    }
    format!(
        "{}\n…(cut at {} KiB; the whole answer is in the agent's pane)",
        &text[..text.floor_char_boundary(ANSWER_KEPT)],
        ANSWER_KEPT / 1024
    )
}

// ── Branch: its arms, one step at a time ──

/// Run the next step inside the Branch at `cur` ([`Frame`] says where it stands). `put_off`:
/// an Agent step in the arm had its start put off and is asked again — since when, and why
/// ([`agent_in_arm`]).
fn branch<H: RecipeHost>(
    host: &mut H,
    recipe: &Recipe,
    steps: &[StoredStep],
    cur: usize,
    top: &RecipeStep,
    vars: &Vars,
    now: f64,
    put_off: Option<(f64, &str)>,
) -> Advance {
    let id = recipe.id.as_str();
    let mut trail = Trail::read(vars);
    let mut frames: Vec<Frame> =
        vars.get(BRANCH_VAR).and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
    if frames.first().map(|f| f.step) != Some(cur) {
        let arm = choose(top, vars);
        trail.enter_branch(cur, arm);
        frames = vec![Frame { step: cur, arm: arm.to_string(), next: 0 }];
    }
    let arm = frames[0].arm.clone();

    let Some(sub) = descend(top, &mut frames, vars, &mut trail, cur) else {
        save_trail(host, id, &trail);
        close_branch(host, id, cur, &arm);
        return Advance::Next;
    };
    // An Agent step in the arm: its agent is the Branch's own — tracked under the Branch's key
    // and joined before the Branch closes (#194).
    if matches!(sub, RecipeStep::Agent { .. }) {
        return agent_in_arm(host, recipe, cur, &sub, &mut frames, &mut trail, vars, now, put_off);
    }
    let depth = frames.len();
    let first_visit = trail.runs(cur) <= 1;
    let did = perform(host, id, &sub, vars, first_visit, now);

    match did {
        Did::Went(_) => {
            if let Some(f) = frames.last_mut() {
                f.next += 1;
            }
            if depth == 1 {
                trail.sub(cur, "done");
            }
            settle_frames(host, id, cur, top, &mut frames, &mut trail, &arm, vars)
        }
        Did::Jump(target) => {
            trail.leave(cur, target);
            save_trail(host, id, &trail);
            host.with_conn(|c| {
                RecipeStore::delete_var(c, id, BRANCH_VAR);
                RecipeStore::complete_step(c, id, cur, &arm);
            });
            go_to(host, id, cur, target);
            Advance::Next
        }
        Did::Sleep(until) => wait_in_arm(host, id, cur, &mut frames, &mut trail, now, Some(until)),
        Did::Ask => wait_in_arm(host, id, cur, &mut frames, &mut trail, now, None),
        Did::Failed(err) => match on_error_of(&sub) {
            ErrorAction::Skip => {
                if let Some(f) = frames.last_mut() {
                    f.next += 1;
                }
                if depth == 1 {
                    trail.sub(cur, "skipped");
                }
                settle_frames(host, id, cur, top, &mut frames, &mut trail, &arm, vars)
            }
            ErrorAction::Retry { max } => {
                let key = format!(
                    "_retry_{cur}_{}",
                    frames.iter().map(|f| f.next.to_string()).collect::<Vec<_>>().join("_")
                );
                let tried = vars.get(&key).and_then(|v| v.as_u64()).unwrap_or(0);
                if tried < max as u64 {
                    set(host, id, &key, json!(tried + 1));
                    set(host, id, BRANCH_VAR, json!(frames));
                    save_trail(host, id, &trail);
                    return Advance::Next;
                }
                if depth == 1 {
                    trail.sub(cur, "failed");
                }
                save_trail(host, id, &trail);
                host.with_conn(|c| RecipeStore::delete_var(c, id, BRANCH_VAR));
                fail(host, recipe, cur, &err, &format!("Failed after {max} retries: {err}"))
            }
            ErrorAction::JumpTo { step: target } => {
                if depth == 1 {
                    trail.sub(cur, "failed");
                }
                save_trail(host, id, &trail);
                host.with_conn(|c| {
                    RecipeStore::delete_var(c, id, BRANCH_VAR);
                    RecipeStore::fail_step(c, id, cur, &err);
                    RecipeStore::update_status(c, id, &RecipeStatus::Running, target);
                });
                Advance::Next
            }
            other => {
                if depth == 1 {
                    trail.sub(cur, "failed");
                }
                save_trail(host, id, &trail);
                host.with_conn(|c| RecipeStore::delete_var(c, id, BRANCH_VAR));
                recover(host, recipe, steps, cur, &err, &other, vars)
            }
        },
    }
}

/// Start an Agent step inside a Branch's arm (#194): its agent is the Branch's own — tracked in
/// `_agents` under the Branch's key ([`agent_key`]) — the arm goes on past it while it works, and
/// the Branch joins the agents it started before it closes ([`settle_frames`]). A start the shell
/// puts off is asked again from here, as a top-level one is ([`agent_step`]).
#[allow(clippy::too_many_arguments)]
fn agent_in_arm<H: RecipeHost>(
    host: &mut H,
    recipe: &Recipe,
    cur: usize,
    here: &RecipeStep,
    frames: &mut Vec<Frame>,
    trail: &mut Trail,
    vars: &Vars,
    now: f64,
    put_off: Option<(f64, &str)>,
) -> Advance {
    let RecipeStep::Agent { role, prompt, store_as, context } = here else {
        return fail(host, recipe, cur, "not an Agent step", "not an Agent step");
    };
    let id = recipe.id.as_str();
    // The check `advance` makes before every step, made here too. On a Branch's first entry no
    // frames are saved yet, so that check saw only the Branch, never the Agent step in its arm:
    // a fourth agent started past AGENTS_AT_ONCE, and a loop back into the arm started the step
    // again over its last round's agent, still working, which was then never polled or released.
    if let Some(block) = blocks_on_agents(here, &agent_key(cur, frames), &agent_runs(vars)) {
        tracing::info!(recipe_id = id, step = cur, waits_on = ?block, "An Agent step inside a Branch's arm waits on the recipe's agents");
        // The frames keep pointing at the step, as for a start the shell put off: it is asked
        // again from there once the agents it waits on have answered.
        let at = frames.last().map(|f| f.next).unwrap_or(0);
        let inner: Vec<(String, usize)> = frames
            .iter()
            .enumerate()
            .map(|(d, f)| (f.arm.clone(), frames.get(d + 1).map_or(at, |inner| inner.step)))
            .collect();
        save_trail(host, id, trail);
        set(host, id, BRANCH_VAR, json!(frames));
        let record = WaitRecord { step: cur, inner, since: now, until: None, agents: true, put_off: None };
        begin_wait(host, id, record, cur);
        return Advance::Blocked;
    }
    let leave = host.with_conn(|c| RecipeStore::agents_allowed(c, id));
    let role = resolve_vars(role, vars).trim().to_string();
    if role.is_empty() || role.contains("{{") {
        let why = format!("This step names no role (`{role}`): give it one of the catalog's, or the variable that holds one.");
        return failed_start_in_arm(host, recipe, cur, frames, trail, &why);
    }
    let task = resolve_vars(prompt, vars);
    let context = context.as_deref().map(|c| resolve_vars(c, vars)).unwrap_or_default();
    let call = AgentCall {
        recipe_id: id,
        recipe_name: &recipe.name,
        step: cur,
        attended: leave.is_some(),
        consented: leave.as_ref().and_then(|l| l.roles.get(&role)).map(String::as_str),
        asked_by: leave.as_ref().and_then(|l| l.agent.as_deref()),
        role: &role,
        task: &task,
        context: &context,
        put_off: put_off.map(|(_, why)| why),
    };
    let started = match host.agent_hook() {
        Some(hook) => hook.start(&call),
        None => return failed_start_in_arm(host, recipe, cur, frames, trail, NO_HOOK),
    };
    match started {
        Ok(s) => {
            let key = agent_key(cur, frames);
            // Its arm step waits for the answer, as a top-level one's row does; the answer
            // settles it (`settle_agents`), and the arm has already gone on.
            if frames.len() == 1 {
                trail.sub(cur, "waiting");
            }
            save_trail(host, id, trail);
            let until = now + (s.minutes * 60 + ANSWER_GRACE_SECS) as f64;
            tracing::info!(recipe_id = id, step = cur, key = %key, agent = %s.agent, role = %s.role, "A Branch handed work to an agent");
            let run = AgentRun {
                role: s.role,
                role_name: s.role_name,
                mind: s.mind,
                agent: s.agent,
                store_as: store_as.clone(),
                since: now,
                until,
                state: AgentRun::WORKING.into(),
                needs_you: None,
            };
            if let Some(f) = frames.last_mut() {
                f.next += 1;
            }
            // The pointer stays at the Branch: it is not done until its agents are joined.
            host.with_conn(|c| {
                let mut runs = agent_runs(&RecipeStore::get_vars(c, id));
                runs.insert(key, run);
                save_agent_runs(c, id, &runs);
                RecipeStore::delete_var(c, id, WAIT_VAR);
                RecipeStore::set_var(c, id, BRANCH_VAR, &json!(frames));
                RecipeStore::update_status(c, id, &RecipeStatus::Running, cur);
            });
            Advance::Next
        }
        Err(AgentRefusal::Wait(why) | AgentRefusal::Ask(why)) => {
            tracing::info!(recipe_id = id, step = cur, why = %why, "An Agent step's start was put off inside a Branch's arm");
            // The frames keep pointing at the step: the start is asked again from there. They
            // are saved even on a first entry, where nothing has saved them yet — `enter_branch`
            // has counted the round in the trail, and re-entering must not count it again.
            let at = frames.last().map(|f| f.next).unwrap_or(0);
            let inner: Vec<(String, usize)> = frames
                .iter()
                .enumerate()
                .map(|(d, f)| (f.arm.clone(), frames.get(d + 1).map_or(at, |inner| inner.step)))
                .collect();
            save_trail(host, id, trail);
            set(host, id, BRANCH_VAR, json!(frames));
            let record = WaitRecord {
                step: cur,
                inner,
                since: put_off.map(|(since, _)| since).unwrap_or(now),
                until: None,
                agents: true,
                put_off: Some(why),
            };
            begin_wait(host, id, record, cur);
            Advance::Blocked
        }
        Err(AgentRefusal::Fail(why)) => {
            let said = format!("The {} could not be started: {why}", role_display(&role));
            failed_start_in_arm(host, recipe, cur, frames, trail, &said)
        }
    }
}

/// An Agent step whose start never happened: where the Branch stands goes to the store first —
/// on a first entry nothing has saved it yet, and the round the trail counted must survive —
/// then the failure lands as this arm step's outcome ([`fail_in_arm`], #194).
fn failed_start_in_arm<H: RecipeHost>(
    host: &mut H,
    recipe: &Recipe,
    cur: usize,
    frames: &[Frame],
    trail: &Trail,
    why: &str,
) -> Advance {
    let id = recipe.id.as_str();
    save_trail(host, id, trail);
    set(host, id, BRANCH_VAR, json!(frames));
    fail_in_arm(host, recipe, cur, why)
}

/// Fail a recipe at a Branch whose arm stood mid-way: the arm step's outcome is recorded, and
/// the frames go with the failure — a failed recipe leaves no `_branch` behind (#194).
fn fail_in_arm<H: RecipeHost>(host: &mut H, recipe: &Recipe, cur: usize, why: &str) -> Advance {
    let id = recipe.id.as_str();
    let vars = host.with_conn(|c| RecipeStore::get_vars(c, id));
    let mut trail = Trail::read(&vars);
    if branch_frames(&vars, cur).is_some_and(|f| f.len() == 1) {
        trail.sub(cur, "failed");
    }
    save_trail(host, id, &trail);
    host.with_conn(|c| RecipeStore::delete_var(c, id, BRANCH_VAR));
    fail(host, recipe, cur, why, why)
}

/// A step inside an arm waits — a timer or a question. The frames move past it, as the pointer
/// moves past a wait at the top, and `_wait` says where it is: the Branch, and the arms and
/// indexes down to the step.
fn wait_in_arm<H: RecipeHost>(
    host: &mut H,
    id: &str,
    cur: usize,
    frames: &mut [Frame],
    trail: &mut Trail,
    now: f64,
    until: Option<f64>,
) -> Advance {
    let at = frames.last().map(|f| f.next).unwrap_or(0);
    let inner: Vec<(String, usize)> = frames
        .iter()
        .enumerate()
        .map(|(d, f)| (f.arm.clone(), frames.get(d + 1).map_or(at, |inner| inner.step)))
        .collect();
    if let Some(f) = frames.last_mut() {
        f.next += 1;
    }
    if frames.len() == 1 {
        trail.sub(cur, "waiting");
    }
    save_trail(host, id, trail);
    set(host, id, BRANCH_VAR, json!(frames));
    begin_wait(host, id, WaitRecord { step: cur, inner, since: now, until, agents: false, put_off: None }, cur);
    Advance::Blocked
}

/// After a step in an arm: close the arms that are done, and the Branch with them when it is —
/// unless the Branch still has agents of its own working: then it holds at the closing position
/// and `step` waits for their answers before it closes (#194). `close_finished` runs once, on
/// the pass after the join, so every arm step's outcome lands in the trail exactly once.
fn settle_frames<H: RecipeHost>(
    host: &mut H,
    id: &str,
    cur: usize,
    top: &RecipeStep,
    frames: &mut Vec<Frame>,
    trail: &mut Trail,
    arm: &str,
    vars: &Vars,
) -> Advance {
    if branch_done(top, frames) && !branch_agents_working(vars, cur).is_empty() {
        save_trail(host, id, trail);
        set(host, id, BRANCH_VAR, json!(frames));
        return Advance::Next;
    }
    let closed = close_finished(top, frames, trail, cur);
    save_trail(host, id, &trail);
    if closed {
        close_branch(host, id, cur, arm);
    } else {
        set(host, id, BRANCH_VAR, json!(frames));
    }
    Advance::Next
}

/// The step to run next inside the Branch: arms that are done are closed, and a Branch reached
/// inside an arm is entered. None when the whole Branch is done.
fn descend(top: &RecipeStep, frames: &mut Vec<Frame>, vars: &Vars, trail: &mut Trail, cur: usize) -> Option<RecipeStep> {
    loop {
        if close_finished(top, frames, trail, cur) {
            return None;
        }
        let at = frames.last()?.next;
        let sub = arm_list(top, frames).get(at)?.clone();
        if matches!(sub, RecipeStep::Branch { .. }) {
            frames.push(Frame { step: at, arm: choose(&sub, vars).to_string(), next: 0 });
            continue;
        }
        return Some(sub);
    }
}

/// Pop every arm whose steps have all run. True when the outermost one is done too.
fn close_finished(top: &RecipeStep, frames: &mut Vec<Frame>, trail: &mut Trail, cur: usize) -> bool {
    loop {
        let Some(last) = frames.last() else { return true };
        if last.next < arm_list(top, frames).len() {
            return false;
        }
        frames.pop();
        match frames.last_mut() {
            None => return true,
            Some(parent) => {
                parent.next += 1;
                if frames.len() == 1 {
                    // A Branch inside the arm is one of the arm's steps, and it is done.
                    trail.sub(cur, "done");
                }
            }
        }
    }
}

fn close_branch<H: RecipeHost>(host: &mut H, id: &str, cur: usize, arm: &str) {
    host.with_conn(|c| {
        RecipeStore::delete_var(c, id, BRANCH_VAR);
        RecipeStore::complete_step(c, id, cur, arm);
        RecipeStore::update_status(c, id, &RecipeStatus::Running, cur + 1);
    });
}

// ── Moving on, waiting, and stopping ──

/// Move the pointer to `target`. A jump back is a loop: the steps it goes round again are marked
/// to come, so the row shows this round, not the last one's ticks. `_trail` keeps the count.
fn go_to<H: RecipeHost>(host: &mut H, id: &str, from: usize, target: usize) {
    host.with_conn(|c| {
        if target <= from {
            RecipeStore::reset_steps(c, id, target, from);
        }
        RecipeStore::update_status(c, id, &RecipeStatus::Running, target);
    });
}

fn begin_wait<H: RecipeHost>(host: &mut H, id: &str, record: WaitRecord, pointer: usize) {
    host.with_conn(|c| {
        RecipeStore::set_var(c, id, WAIT_VAR, &json!(record));
        RecipeStore::set_var(c, id, SINCE_WAIT_VAR, &json!(0));
        RecipeStore::update_status(c, id, &RecipeStatus::Waiting, pointer);
    });
}

/// A timer is over — or a recipe was left `waiting` with no wait behind it: running again, from
/// where it stands.
fn wake<H: RecipeHost>(host: &mut H, recipe: &Recipe, waited: Option<&Waited>) {
    let id = recipe.id.as_str();
    if let Some(w) = waited.filter(|w| matches!(w.on, Some(RecipeStep::WaitFor { .. }))) {
        if w.inner.is_empty() {
            let note = match w.until {
                Some(until) => format!("waited until {}", clock_text(until)),
                None => "waited".to_string(),
            };
            host.with_conn(|c| RecipeStore::complete_step(c, id, w.step, &note));
        } else {
            host.with_conn(|c| Trail::note(c, id, w.step, "waiting", "waited"));
        }
    }
    host.with_conn(|c| {
        RecipeStore::delete_var(c, id, WAIT_VAR);
        RecipeStore::update_status(c, id, &RecipeStatus::Running, recipe.current_step);
    });
    tracing::info!(recipe_id = id, "Recipe resumed: its wait is over");
}

/// Past the last step: done, and the person told, with what it came to.
fn finish<H: RecipeHost>(host: &mut H, recipe: &Recipe, steps: &[StoredStep], vars: &Vars) {
    let id = recipe.id.as_str();
    host.with_conn(|c| RecipeStore::update_status(c, id, &RecipeStatus::Done, recipe.current_step));
    let last = steps.last().and_then(|s| outcome_of(&s.step, vars)).unwrap_or_default();
    let text = if last.is_empty() {
        format!("Recipe completed: {}", recipe.name)
    } else {
        format!("Recipe completed: {}\n\nResult: {}", recipe.name, last)
    };
    host.notify(id, &text);
    tracing::info!(recipe_id = id, name = %recipe.name, steps = steps.len(), "Recipe completed");
}

/// What a last step came to, for the completion message.
fn outcome_of(step: &RecipeStep, vars: &Vars) -> Option<String> {
    match step {
        RecipeStep::Notify { message } => Some(resolve_vars(message, vars)),
        other => {
            let key = crate::recipe_view::store_as(other)?;
            let value = vars.get(key)?.as_str()?;
            Some(if value.len() > 500 { format!("{}...", &value[..value.floor_char_boundary(500)]) } else { value.to_string() })
        }
    }
}

/// The spin guard: too many steps without a pause.
fn stop_spinning<H: RecipeHost>(host: &mut H, recipe: &Recipe, steps: &[StoredStep], vars: &Vars, budget: u64) {
    let cur = recipe.current_step;
    let label = steps.get(cur).map(|s| crate::recipe_view::stage_label(&s.step)).unwrap_or_default();
    let looped = Trail::read(vars)
        .most_looped()
        .map(|(i, n)| format!("; step {} had gone back {n} times", i + 1))
        .unwrap_or_default();
    let why = format!(
        "{budget} steps ran without a pause (a timer or a question), and it was stopped before step {} ({label}){looped}. \
         A loop that never waits would run forever: give it a WaitFor, or set `{STEP_BUDGET_VAR}` if it needs more steps.",
        cur + 1
    );
    release_agents(host, &recipe.id, &format!("The {} recipe was stopped, so its work is no longer wanted.", recipe.name));
    host.with_conn(|c| RecipeStore::set_error(c, &recipe.id, &format!("Stopped: {why}")));
    host.notify(&recipe.id, &format!("Recipe '{}' stopped: {why}", recipe.name));
    tracing::warn!(recipe_id = %recipe.id, budget, "Recipe stopped by its step budget");
}

/// A step failed: what its `on_error` says to do.
fn recover<H: RecipeHost>(
    host: &mut H,
    recipe: &Recipe,
    steps: &[StoredStep],
    cur: usize,
    err: &str,
    on_error: &ErrorAction,
    vars: &Vars,
) -> Advance {
    let id = recipe.id.as_str();
    match on_error {
        ErrorAction::Fail => fail(host, recipe, cur, err, err),
        ErrorAction::Skip => {
            host.with_conn(|c| {
                RecipeStore::skip_step(c, id, cur);
                RecipeStore::update_status(c, id, &RecipeStatus::Running, cur + 1);
            });
            Advance::Next
        }
        ErrorAction::Retry { max } => {
            let key = format!("_retry_{cur}");
            let tried = vars.get(&key).and_then(|v| v.as_u64()).unwrap_or(0);
            if tried < *max as u64 {
                set(host, id, &key, json!(tried + 1));
                tracing::info!(recipe_id = id, step = cur, retry = tried + 1, max = *max, "Recipe step retry");
                Advance::Next
            } else {
                fail(host, recipe, cur, err, &format!("Failed after {max} retries: {err}"))
            }
        }
        ErrorAction::JumpTo { step: target } => {
            host.with_conn(|c| {
                RecipeStore::fail_step(c, id, cur, err);
                RecipeStore::update_status(c, id, &RecipeStatus::Running, *target);
            });
            Advance::Next
        }
        ErrorAction::Replan => match replan(host, recipe, steps, cur, err) {
            Ok(n) => {
                host.notify(
                    id,
                    &format!("Recipe '{}' step {} failed ({}). Replanned with {} new steps.", recipe.name, cur + 1, err, n),
                );
                Advance::Next
            }
            Err(why) => fail(host, recipe, cur, err, &format!("Step {} failed and could not be replanned ({why}): {err}", cur + 1)),
        },
    }
}

fn fail<H: RecipeHost>(host: &mut H, recipe: &Recipe, cur: usize, step_error: &str, recipe_error: &str) -> Advance {
    release_agents(host, &recipe.id, &format!("The {} recipe failed, so its work is no longer wanted.", recipe.name));
    host.with_conn(|c| {
        RecipeStore::fail_step(c, &recipe.id, cur, step_error);
        RecipeStore::set_error(c, &recipe.id, recipe_error);
    });
    host.notify(&recipe.id, &format!("Recipe '{}' failed at step {}: {}", recipe.name, cur + 1, recipe_error));
    tracing::warn!(recipe_id = %recipe.id, step = cur, error = %recipe_error, "Recipe failed");
    Advance::Stopped
}

/// Ask the model for steps to replace the ones after a failed step. The failed step keeps its
/// record; the new steps start after it. Returns how many there are.
fn replan<H: RecipeHost>(host: &mut H, recipe: &Recipe, steps: &[StoredStep], cur: usize, err: &str) -> Result<usize, String> {
    let done: Vec<String> = steps
        .iter()
        .take(cur)
        .map(|s| format!("Step {}: {} → {}", s.step_index + 1, crate::recipe_view::kind_of(&s.step), s.result.as_deref().unwrap_or("(no result)")))
        .collect();
    let failed = steps.get(cur).map(|s| serde_json::to_string(&s.step).unwrap_or_default()).unwrap_or_default();
    let remaining: Vec<String> = steps
        .iter()
        .skip(cur + 1)
        .map(|s| format!("Step {}: {}", s.step_index + 1, serde_json::to_string(&s.step).unwrap_or_default()))
        .collect();
    let prompt = format!(
        "Recipe '{}' failed at step {}.\nError: {}\n\nCompleted steps:\n{}\n\nFailed step: {}\n\n\
         Remaining planned steps:\n{}\n\nRecipe goal: {}\n\n\
         Analyze the failure and provide replacement steps as a JSON array. Each step must be one of:\n\
         - {{\"type\":\"Tool\",\"tool_name\":\"...\",\"args\":{{...}},\"store_as\":\"...\",\"on_error\":{{\"action\":\"Replan\"}}}}\n\
         - {{\"type\":\"Think\",\"prompt\":\"...\",\"store_as\":\"...\"}}\n\
         - {{\"type\":\"Notify\",\"message\":\"...\"}}\n\n\
         Reply with ONLY the JSON array of replacement steps. Fix the root cause, don't just retry the same thing. \
         If the failure is unrecoverable, reply with [].",
        recipe.name,
        cur + 1,
        err,
        if done.is_empty() { "(none)".to_string() } else { done.join("\n") },
        failed,
        if remaining.is_empty() { "(none)".to_string() } else { remaining.join("\n") },
        recipe.description,
    );
    let reply = host.generate("You are a recipe debugger. Output ONLY a JSON array of replacement steps.", &prompt, true)?;
    let new_steps: Vec<RecipeStep> = serde_json::from_str(&extract_json_array(&strip_think_tags(&reply)))
        .map_err(|e| format!("its steps could not be read: {e}"))?;
    if new_steps.is_empty() {
        return Err("the model offered no steps".into());
    }
    let tool = match steps.get(cur).map(|s| &s.step) {
        Some(RecipeStep::Tool { tool_name, .. }) => tool_name.clone(),
        Some(other) => crate::recipe_view::kind_of(other).to_string(),
        None => String::new(),
    };
    let id = recipe.id.as_str();
    host.with_conn(|c| {
        RecipeStore::fail_step(c, id, cur, err);
        RecipeStore::record_failure_learning(c, id, cur, &tool, err, &format!("Replanned with {} new steps", new_steps.len()));
        RecipeStore::replace_remaining_steps(c, id, cur + 1, &new_steps);
        RecipeStore::update_status(c, id, &RecipeStatus::Running, cur + 1);
    });
    tracing::info!(recipe_id = id, step = cur, new_steps = new_steps.len(), "Recipe replanned after failure");
    Ok(new_steps.len())
}

fn on_error_of(step: &RecipeStep) -> ErrorAction {
    match step {
        RecipeStep::Tool { on_error, .. } => on_error.clone(),
        _ => ErrorAction::Fail,
    }
}

/// How the tools say they failed. A tool returns a string either way.
fn is_tool_error(out: &str) -> bool {
    out.starts_with("Unknown tool:") || out.starts_with("Permission denied:") || out.starts_with("Failed") || out.starts_with("Error")
}

fn is_answer(v: &serde_json::Value) -> bool {
    !v.is_null() && v.as_str().map_or(true, |s| !s.trim().is_empty())
}

fn value_text(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn set<H: RecipeHost>(host: &H, id: &str, key: &str, value: serde_json::Value) {
    host.with_conn(|c| RecipeStore::set_var(c, id, key, &value));
}

fn save_trail<H: RecipeHost>(host: &H, id: &str, trail: &Trail) {
    host.with_conn(|c| trail.save_to(c, id));
}

// ── The shell's host ──

impl RecipeHost for CompanionService {
    fn with_conn<R>(&self, f: impl FnOnce(&Connection) -> R) -> R {
        f(&self.db.conn())
    }

    fn run_tool(&mut self, name: &str, args: &serde_json::Value) -> String {
        self.execute_tool_direct(name, args)
    }

    fn generate(&mut self, system: &str, prompt: &str, precise: bool) -> Result<String, String> {
        let messages = vec![ChatMessage::system(system), ChatMessage::user(prompt)];
        let config = GenerationConfig {
            max_tokens: self.config.llm.max_tokens,
            temperature: if precise { 0.2 } else { self.config.llm.temperature },
            ..Default::default()
        };
        self.llm.chat(&messages, &config, None).map(|r| r.text).map_err(|e| e.to_string())
    }

    fn notify(&mut self, recipe_id: &str, text: &str) {
        let at = now_ts();
        self.set_proactive_message(crate::types::ProactiveMessage {
            text: text.to_string(),
            // Every message carries a key of its own: the shell remembers deliveries per key,
            // and one key per recipe used to hold back that recipe's next message for the
            // whole delivery cooldown (#187). The first segment stays "recipe" — that is what
            // names the instinct the message came from.
            urge_ids: vec![format!("recipe:{recipe_id}:{at}")],
            generated_at: at,
        });
    }

    fn persona(&self) -> String {
        self.config.personality.name.clone()
    }

    fn agent_hook(&mut self) -> Option<&mut (dyn AgentHook + 'static)> {
        self.agent_hook.as_deref_mut()
    }
}

// ── ThinkCited: LLM synthesis with per-claim citations ──

fn think_cited<H: RecipeHost>(host: &mut H, id: &str, prompt: &str, store_as: &str, source_vars: &[String], vars: &Vars) -> Did {
    use crate::recipe::{CitedClaim, CitedOutput, EvidenceStatus};

    let mut source_context = String::new();
    for (i, name) in source_vars.iter().enumerate() {
        let content = vars.get(name).and_then(|v| v.as_str()).unwrap_or("(no data)");
        source_context.push_str(&format!("\n[SOURCE:{}] (from step '{}'): {}\n", i + 1, name, content));
    }
    let instruction = format!(
        "You have the following sources:\n{}\n\n{}\n\n\
         IMPORTANT: Output JSON with this exact structure:\n\
         {{\n  \"title\": \"<section title>\",\n  \"claims\": [\n    {{\"text\": \"<claim>\", \"sources\": [\"<source_var_name>\", ...]}},\n    ...\n  ]\n}}\n\
         Each claim MUST reference which source(s) support it by variable name.\n\
         If a fact has no source, do NOT include it.\nOutput ONLY the JSON, no other text.",
        source_context,
        resolve_vars(prompt, vars)
    );
    let system = format!(
        "You are {}. You produce citation-backed analysis. Every claim must reference its source. Never invent facts.",
        host.persona()
    );
    match host.generate(&system, &instruction, true) {
        Ok(reply) => {
            let text = strip_think_tags(&reply);
            let output = match serde_json::from_str::<CitedOutput>(&extract_json_object(&text)) {
                Ok(mut output) => {
                    for claim in &mut output.claims {
                        claim.confidence = match claim.sources.len() {
                            0 => "uncited",
                            1 => "low",
                            2 => "medium",
                            _ => "high",
                        }
                        .to_string();
                    }
                    output.evidence_status = compute_evidence_status(&output.claims, source_vars);
                    output
                }
                Err(_) => CitedOutput {
                    title: "Analysis".to_string(),
                    claims: vec![CitedClaim { text: truncate(&text), sources: vec![], confidence: "uncited".to_string() }],
                    evidence_status: EvidenceStatus::Insufficient,
                },
            };
            set(host, id, store_as, serde_json::to_value(&output).unwrap_or_default());
            Did::Went("done".into())
        }
        Err(e) => Did::Failed(format!("The model did not answer (ThinkCited): {e}")),
    }
}

fn compute_evidence_status(claims: &[crate::recipe::CitedClaim], source_vars: &[String]) -> crate::recipe::EvidenceStatus {
    use crate::recipe::EvidenceStatus;
    if claims.is_empty() {
        return EvidenceStatus::Insufficient;
    }
    let cited = claims.iter().filter(|c| !c.sources.is_empty()).count();
    if cited == 0 {
        return EvidenceStatus::Insufficient;
    }
    let unique: std::collections::HashSet<&str> = claims.iter().flat_map(|c| c.sources.iter().map(|s| s.as_str())).collect();
    let coverage = if source_vars.is_empty() { 0.0 } else { unique.len() as f64 / source_vars.len() as f64 };
    let cite_ratio = cited as f64 / claims.len() as f64;
    if cite_ratio >= 0.8 && coverage >= 0.6 {
        EvidenceStatus::Strong
    } else if cite_ratio >= 0.5 {
        EvidenceStatus::Moderate
    } else {
        EvidenceStatus::Thin
    }
}

// ── Validate: deterministic claim verification ──

fn validate<H: RecipeHost>(host: &mut H, id: &str, input_var: &str, store_as: &str, vars: &Vars) -> Did {
    use crate::recipe::{CitedClaim, CitedOutput, EvidenceStatus};

    let Some(input) = vars.get(input_var).cloned() else {
        return Did::Failed(format!("Validate: variable '{input_var}' not found"));
    };
    let mut output: CitedOutput = serde_json::from_value(input.clone()).unwrap_or_else(|_| CitedOutput {
        title: "Validation".to_string(),
        claims: vec![CitedClaim { text: input.as_str().unwrap_or("").to_string(), sources: vec![], confidence: "uncited".to_string() }],
        evidence_status: EvidenceStatus::Insufficient,
    });
    let before = output.claims.len();
    output.claims.retain(|c| !c.sources.is_empty());
    let cited = output.claims.len();
    output.evidence_status = match cited {
        0 => EvidenceStatus::Insufficient,
        1 => EvidenceStatus::Thin,
        2 => EvidenceStatus::Moderate,
        _ => EvidenceStatus::Strong,
    };
    let report = json!({
        "total_claims": before,
        "cited_claims": cited,
        "stripped_uncited": before - cited,
        "evidence_status": format!("{:?}", output.evidence_status),
    });
    set(host, id, &format!("{store_as}_report"), report);
    set(host, id, store_as, serde_json::to_value(&output).unwrap_or_default());
    Did::Went("done".into())
}

// ── Render: format validated data for presentation ──

fn render<H: RecipeHost>(host: &mut H, id: &str, input_var: &str, store_as: &str, format: &crate::recipe::RenderFormat, vars: &Vars) -> Did {
    let Some(input) = vars.get(input_var).cloned() else {
        return Did::Failed(format!("Render: variable '{input_var}' not found"));
    };
    let rendered = match serde_json::from_value::<crate::recipe::CitedOutput>(input.clone()) {
        Ok(output) => render_cited_output(&output, format),
        Err(_) => input.as_str().unwrap_or("(no data)").to_string(),
    };
    set(host, id, store_as, json!(rendered));
    Did::Went("done".into())
}

fn render_cited_output(output: &crate::recipe::CitedOutput, format: &crate::recipe::RenderFormat) -> String {
    use crate::recipe::{EvidenceStatus, RenderFormat};

    if output.claims.is_empty() {
        return format!("**{}**\n\nNo verified information available.", output.title);
    }
    let evidence = match &output.evidence_status {
        EvidenceStatus::Strong => "Well-supported",
        EvidenceStatus::Moderate => "Moderately supported",
        EvidenceStatus::Thin => "Limited evidence",
        EvidenceStatus::Conflicting => "Conflicting sources",
        EvidenceStatus::Insufficient => "Insufficient evidence",
    };
    let mut out = format!("**{}** _({})_\n\n", output.title, evidence);
    match format {
        RenderFormat::Summary => {
            for claim in &output.claims {
                let marker = match claim.confidence.as_str() {
                    "high" | "medium" => "",
                    "low" => " _(limited source)_",
                    _ => " _(unverified)_",
                };
                out.push_str(&format!("- {}{}\n", claim.text, marker));
            }
        }
        RenderFormat::Table => {
            out.push_str("| Finding | Confidence | Sources |\n|---|---|---|\n");
            for claim in &output.claims {
                out.push_str(&format!("| {} | {} | {} |\n", claim.text, claim.confidence, claim.sources.join(", ")));
            }
        }
        RenderFormat::Comparison => {
            for (i, claim) in output.claims.iter().enumerate() {
                let sources = if claim.sources.is_empty() { "none".to_string() } else { claim.sources.join(", ") };
                out.push_str(&format!("**{}. {}**\n  Sources: {}\n  Confidence: {}\n\n", i + 1, claim.text, sources, claim.confidence));
            }
        }
        RenderFormat::Cards => {
            for (i, claim) in output.claims.iter().enumerate() {
                out.push_str(&format!(
                    "┌─ {} ─────────────────────\n│ {}\n│ Sources: {} | Confidence: {}\n└─────────────────────────────\n\n",
                    i + 1,
                    claim.text,
                    claim.sources.join(", "),
                    claim.confidence
                ));
            }
        }
    }
    out
}

// ── The data steps: Filter, Sort, Aggregate, Extract ──

#[allow(clippy::too_many_arguments)]
fn filter<H: RecipeHost>(host: &mut H, id: &str, input_var: &str, field: &str, op: &FilterOp, value: &str, store_as: &str, vars: &Vars) -> Did {
    let Some(input) = vars.get(input_var) else {
        return Did::Failed(format!("Filter: variable '{input_var}' not found"));
    };
    let Some(rows) = parse_json_array(input) else {
        return Did::Failed(format!("Filter: '{input_var}' is not a JSON array"));
    };
    let kept: Vec<serde_json::Value> = rows
        .into_iter()
        .filter(|row| {
            let v = row.get(field);
            match op {
                FilterOp::Equals => v.is_some_and(|v| value_matches_str(v, value)),
                FilterOp::NotEquals => v.map_or(true, |v| !value_matches_str(v, value)),
                FilterOp::Contains => v.and_then(|v| v.as_str()).is_some_and(|s| s.contains(value)),
                FilterOp::GreaterThan => compare_field_value(v, value) == Some(std::cmp::Ordering::Greater),
                FilterOp::LessThan => compare_field_value(v, value) == Some(std::cmp::Ordering::Less),
            }
        })
        .collect();
    set(host, id, store_as, json!(truncate(&serde_json::to_string(&kept).unwrap_or_else(|_| "[]".into()))));
    Did::Went("done".into())
}

fn sort<H: RecipeHost>(host: &mut H, id: &str, input_var: &str, by_field: &str, descending: bool, store_as: &str, vars: &Vars) -> Did {
    let Some(input) = vars.get(input_var) else {
        return Did::Failed(format!("Sort: variable '{input_var}' not found"));
    };
    let Some(mut rows) = parse_json_array(input) else {
        return Did::Failed(format!("Sort: '{input_var}' is not a JSON array"));
    };
    rows.sort_by(|a, b| {
        let ord = compare_json_values(a.get(by_field), b.get(by_field));
        if descending { ord.reverse() } else { ord }
    });
    set(host, id, store_as, json!(truncate(&serde_json::to_string(&rows).unwrap_or_else(|_| "[]".into()))));
    Did::Went("done".into())
}

fn aggregate<H: RecipeHost>(host: &mut H, id: &str, input_var: &str, op: &AggregateOp, field: Option<&str>, store_as: &str, vars: &Vars) -> Did {
    let Some(input) = vars.get(input_var) else {
        return Did::Failed(format!("Aggregate: variable '{input_var}' not found"));
    };
    let Some(rows) = parse_json_array(input) else {
        return Did::Failed(format!("Aggregate: '{input_var}' is not a JSON array"));
    };
    let result = match op {
        AggregateOp::Count => rows.len().to_string(),
        _ => {
            let values: Vec<f64> = rows
                .iter()
                .filter_map(|row| match field {
                    Some(f) => row.get(f).and_then(json_to_f64),
                    None => json_to_f64(row),
                })
                .collect();
            if values.is_empty() {
                "0".to_string()
            } else {
                match op {
                    AggregateOp::Sum => values.iter().sum::<f64>().to_string(),
                    AggregateOp::Min => values.iter().cloned().fold(f64::INFINITY, f64::min).to_string(),
                    AggregateOp::Max => values.iter().cloned().fold(f64::NEG_INFINITY, f64::max).to_string(),
                    AggregateOp::Avg => (values.iter().sum::<f64>() / values.len() as f64).to_string(),
                    AggregateOp::Count => unreachable!("counted above"),
                }
            }
        }
    };
    set(host, id, store_as, json!(result));
    Did::Went("done".into())
}

fn extract<H: RecipeHost>(host: &mut H, id: &str, input_var: &str, pattern: &str, store_as: &str, vars: &Vars) -> Did {
    let Some(input) = vars.get(input_var).cloned() else {
        return Did::Failed(format!("Extract: variable '{input_var}' not found"));
    };
    if pattern.starts_with('/') {
        let source = pattern.trim_start_matches('/').trim_end_matches('/');
        return match regex::Regex::new(source) {
            Ok(re) => {
                let text = value_text(&input);
                let found = re.find(&text).map(|m| m.as_str().to_string()).unwrap_or_default();
                set(host, id, store_as, json!(found));
                Did::Went("done".into())
            }
            Err(e) => Did::Failed(format!("Extract: invalid regex '{source}': {e}")),
        };
    }
    // Dot-notation key path ("data.name", "items.0.title").
    let parsed = match &input {
        serde_json::Value::String(s) => serde_json::from_str::<serde_json::Value>(s).unwrap_or(input.clone()),
        other => other.clone(),
    };
    let mut at = &parsed;
    for key in pattern.split('.') {
        let next = match key.parse::<usize>() {
            Ok(i) => at.get(i),
            Err(_) => at.get(key),
        };
        match next {
            Some(v) => at = v,
            None => {
                set(host, id, store_as, json!(""));
                return Did::Went("done".into());
            }
        }
    }
    set(host, id, store_as, json!(truncate(&value_text(at))));
    Did::Went("done".into())
}

fn parse_json_array(val: &serde_json::Value) -> Option<Vec<serde_json::Value>> {
    match val {
        serde_json::Value::Array(a) => Some(a.clone()),
        serde_json::Value::String(s) => serde_json::from_str::<Vec<serde_json::Value>>(s).ok(),
        _ => None,
    }
}

fn value_matches_str(v: &serde_json::Value, s: &str) -> bool {
    match v {
        serde_json::Value::String(vs) => vs == s,
        serde_json::Value::Number(n) => n.to_string() == s,
        serde_json::Value::Bool(b) => b.to_string() == s,
        serde_json::Value::Null => s.is_empty() || s == "null",
        _ => false,
    }
}

fn compare_field_value(field: Option<&serde_json::Value>, threshold: &str) -> Option<std::cmp::Ordering> {
    json_to_f64(field?)?.partial_cmp(&threshold.parse::<f64>().ok()?)
}

fn compare_json_values(a: Option<&serde_json::Value>, b: Option<&serde_json::Value>) -> std::cmp::Ordering {
    match (a, b) {
        (None, None) => std::cmp::Ordering::Equal,
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
        (Some(va), Some(vb)) => {
            if let (Some(na), Some(nb)) = (json_to_f64(va), json_to_f64(vb)) {
                return na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal);
            }
            value_text(va).cmp(&value_text(vb))
        }
    }
}

fn json_to_f64(v: &serde_json::Value) -> Option<f64> {
    match v {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

// ── Helpers ──

fn truncate(s: &str) -> String {
    if s.len() > MAX_RESULT_SIZE {
        format!("{}...(truncated)", &s[..s.floor_char_boundary(MAX_RESULT_SIZE)])
    } else {
        s.to_string()
    }
}

fn strip_think_tags(text: &str) -> String {
    let mut result = String::new();
    let mut remaining = text;
    while let Some(start) = remaining.find("<think>") {
        result.push_str(&remaining[..start]);
        let after = &remaining[start + "<think>".len()..];
        match after.find("</think>") {
            Some(end) => remaining = &after[end + "</think>".len()..],
            None => {
                remaining = "";
                break;
            }
        }
    }
    result.push_str(remaining);
    result.trim().to_string()
}

fn extract_json_array(text: &str) -> String {
    match (text.find('['), text.rfind(']')) {
        (Some(start), Some(end)) if end > start => text[start..=end].to_string(),
        _ => "[]".to_string(),
    }
}

fn extract_json_object(text: &str) -> String {
    match (text.find('{'), text.rfind('}')) {
        (Some(start), Some(end)) if end > start => text[start..=end].to_string(),
        _ => "{}".to_string(),
    }
}

fn now_ts() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs_f64()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recipe::{Condition, TriggerType, WaitCondition};
    use crate::recipe_view::{self, RecipeOp};
    use std::collections::VecDeque;

    /// 2026-09-23 08:00:00 UTC.
    const EIGHT_AM: f64 = 1_790_150_400.0;

    /// A desk to run recipes on: the store in memory, tools and a model that answer from a
    /// script, what the recipe told the person, and a clock that moves when the test moves it.
    struct Desk {
        conn: Connection,
        clock: f64,
        tools: HashMap<String, VecDeque<String>>,
        called: Vec<String>,
        model: VecDeque<Result<String, String>>,
        said: Vec<String>,
        /// The shell's hook, faked: None is a host with no shell.
        hands: Option<Hands>,
    }

    impl Desk {
        fn new() -> Self {
            let conn = Connection::open_in_memory().expect("in-memory sqlite");
            RecipeStore::ensure_tables(&conn);
            Self {
                conn,
                clock: EIGHT_AM,
                tools: HashMap::new(),
                called: Vec::new(),
                model: VecDeque::new(),
                said: Vec::new(),
                hands: Some(Hands::default()),
            }
        }

        fn hands(&mut self) -> &mut Hands {
            self.hands.as_mut().expect("a hook")
        }

        /// A recipe of these steps, started with the person's leave to hand work to agents.
        fn start_allowed(&self, steps: &[RecipeStep]) -> String {
            let id = self.start(steps);
            RecipeStore::allow_agents(&self.conn, &id, &crate::recipe::Leave::new("the person, in a test", None));
            id
        }

        /// Move every recipe the clock would move, once — what the worker's clock does each tick.
        fn tick(&mut self) {
            for id in due(&self.conn) {
                run(self, &id, 50);
            }
        }

        /// A tool answers these in turn, and the last one from then on.
        fn tool_says(&mut self, name: &str, replies: &[&str]) {
            self.tools.insert(name.into(), replies.iter().map(|s| s.to_string()).collect());
        }

        /// A recipe of these steps, started.
        fn start(&self, steps: &[RecipeStep]) -> String {
            let id = RecipeStore::create(&self.conn, "Digest", "", steps, None);
            RecipeStore::update_status(&self.conn, &id, &RecipeStatus::Running, 0);
            id
        }

        fn status(&self, id: &str) -> (RecipeStatus, usize) {
            let r = RecipeStore::get(&self.conn, id).expect("the recipe");
            (r.status, r.current_step)
        }

        fn view(&self, id: &str) -> recipe_view::RecipeView {
            recipe_view::list(&self.conn).into_iter().find(|v| v.id == id).expect("the recipe's view")
        }

        fn var(&self, id: &str, key: &str) -> Option<serde_json::Value> {
            RecipeStore::get_vars(&self.conn, id).get(key).cloned()
        }
    }

    impl RecipeHost for Desk {
        fn with_conn<R>(&self, f: impl FnOnce(&Connection) -> R) -> R {
            f(&self.conn)
        }
        fn run_tool(&mut self, name: &str, _args: &serde_json::Value) -> String {
            self.called.push(name.to_string());
            match self.tools.get_mut(name) {
                Some(q) if q.len() > 1 => q.pop_front().unwrap_or_default(),
                Some(q) => q.front().cloned().unwrap_or_default(),
                None => format!("ok:{name}"),
            }
        }
        fn generate(&mut self, _system: &str, _prompt: &str, _precise: bool) -> Result<String, String> {
            self.model.pop_front().unwrap_or_else(|| Err("no model here".into()))
        }
        fn notify(&mut self, _recipe_id: &str, text: &str) {
            self.said.push(text.to_string());
        }
        fn persona(&self) -> String {
            "Yantrik".into()
        }
        fn now(&self) -> f64 {
            self.clock
        }
        fn agent_hook(&mut self) -> Option<&mut (dyn AgentHook + 'static)> {
            self.hands.as_mut().map(|h| h as &mut (dyn AgentHook + 'static))
        }
    }

    /// The shell's hook, faked: each role answers from a script — or keeps working until the test
    /// says it answered — and every start, poll and release is recorded.
    #[derive(Default)]
    struct Hands {
        /// Each hand-off, in order: role, task, context.
        started: Vec<(String, String, String)>,
        /// What each start was told the person agreed to, in order: attended, and the role's
        /// digest as agreed — refused ones included.
        told: Vec<(bool, Option<String>)>,
        /// What each start heard the previous try was put off for, in order — refused ones
        /// included.
        heard_put_off: Vec<Option<String>>,
        /// What a role's agents come to, in turn, and the last one from then on. A role with no
        /// script answers "<role> answered: <the task's first line>".
        script: HashMap<String, VecDeque<AgentPoll>>,
        /// Every agent started: the recipe that started it, and how it is doing.
        agents: HashMap<String, (String, AgentPoll)>,
        /// Each agent let go, and why.
        released: Vec<(String, String)>,
        /// Refuse every start with this.
        refuse: Option<AgentRefusal>,
        polls: usize,
    }

    impl Hands {
        fn says(&mut self, role: &str, answers: Vec<AgentPoll>) {
            self.script.insert(role.into(), answers.into());
        }
        /// The roles handed work, in order.
        fn roles(&self) -> Vec<&str> {
            self.started.iter().map(|(r, _, _)| r.as_str()).collect()
        }
        /// An agent still working answers now.
        fn answer(&mut self, agent: &str, text: &str) {
            if let Some((_, heard)) = self.agents.get_mut(agent) {
                *heard = AgentPoll::Answered(text.into());
            }
        }
        fn working(&self) -> Vec<String> {
            let mut w: Vec<String> =
                self.agents.iter().filter(|(_, (_, h))| *h == AgentPoll::Working).map(|(a, _)| a.clone()).collect();
            w.sort();
            w
        }
    }

    impl AgentHook for Hands {
        fn start(&mut self, call: &AgentCall<'_>) -> Result<AgentStarted, AgentRefusal> {
            self.told.push((call.attended, call.consented.map(str::to_string)));
            self.heard_put_off.push(call.put_off.map(str::to_string));
            if let Some(refusal) = self.refuse.clone() {
                return Err(refusal);
            }
            let heard = match self.script.get_mut(call.role) {
                Some(q) if q.len() > 1 => q.pop_front(),
                Some(q) => q.front().cloned(),
                None => None,
            }
            .unwrap_or_else(|| AgentPoll::Answered(format!("{} answered: {}", call.role, call.task.lines().next().unwrap_or(""))));
            self.started.push((call.role.into(), call.task.into(), call.context.into()));
            let agent = format!("pi:c-{:04}", self.started.len());
            self.agents.insert(agent.clone(), (call.recipe_id.into(), heard));
            Ok(AgentStarted {
                agent,
                role: call.role.into(),
                role_name: role_display(call.role),
                mind: "pi".into(),
                minutes: 10,
            })
        }
        fn poll(&mut self, recipe_id: &str, agent: &str) -> AgentPoll {
            self.polls += 1;
            match self.agents.get(agent) {
                Some((by, heard)) if by == recipe_id => heard.clone(),
                Some(_) => AgentPoll::Failed(format!("`{agent}` was not started by this recipe")),
                None => AgentPoll::Failed(format!("the desktop no longer knows `{agent}`")),
            }
        }
        fn release(&mut self, recipe_id: &str, agent: &str, why: &str) {
            if self.agents.get(agent).is_some_and(|(by, _)| by == recipe_id) {
                self.released.push((agent.into(), why.into()));
            }
        }
    }

    fn agent(role: &str, prompt: &str, store_as: &str) -> RecipeStep {
        RecipeStep::Agent { role: role.into(), prompt: prompt.into(), store_as: store_as.into(), context: None }
    }

    fn format(template: &str, store_as: &str) -> RecipeStep {
        RecipeStep::Format { input_vars: vec![], template: template.into(), store_as: store_as.into() }
    }

    fn tool(name: &str, store_as: &str) -> RecipeStep {
        RecipeStep::Tool { tool_name: name.into(), args: json!({}), store_as: store_as.into(), on_error: ErrorAction::Fail }
    }

    fn notify(message: &str) -> RecipeStep {
        RecipeStep::Notify { message: message.into() }
    }

    fn ask(question: &str, choices: &[&str], store_as: &str) -> RecipeStep {
        RecipeStep::AskUser {
            question: question.into(),
            store_as: store_as.into(),
            choices: Some(choices.iter().map(|c| c.to_string()).collect()),
        }
    }

    /// A Branch runs the steps of the side it takes, and only those (#176). The shell's own
    /// executor marked the Branch `then` or `else` and went on, running neither side.
    #[test]
    fn a_branch_runs_the_side_it_takes() {
        let steps = [
            tool("check", "urgent"),
            RecipeStep::Branch {
                condition: "urgent".into(),
                then_steps: vec![tool("page", "paged"), notify("Paged: {{paged}}")],
                else_steps: vec![tool("file", "filed")],
            },
            notify("Done."),
        ];
        let mut desk = Desk::new();
        desk.tool_says("check", &["yes"]);
        let id = desk.start(&steps);
        run(&mut desk, &id, 20);
        assert_eq!(desk.status(&id).0, RecipeStatus::Done);
        assert_eq!(desk.called, ["check", "page"], "the then side ran, and the else side did not");
        assert!(desk.said.iter().any(|s| s == "Paged: ok:page"), "{:?}", desk.said);
        assert_eq!(desk.var(&id, "filed"), None);
        let v = desk.view(&id);
        assert_eq!(v.steps[1].state, "done");
        assert_eq!(v.steps[1].path.as_deref(), Some("took then: page → Notify"));

        // The other way.
        let mut desk = Desk::new();
        desk.tool_says("check", &[""]);
        let id = desk.start(&steps);
        run(&mut desk, &id, 20);
        assert_eq!(desk.called, ["check", "file"]);
        assert_eq!(desk.var(&id, "filed"), Some(json!("ok:file")));
        assert_eq!(desk.view(&id).steps[1].path.as_deref(), Some("took else: file"));
    }

    /// A question inside a Branch waits for its answer like any other — the clock and a stray
    /// signal leave it alone — and the answer, by the screen's path, carries the arm on.
    #[test]
    fn a_question_inside_a_branch_waits_for_its_answer() {
        let steps = [
            RecipeStep::Branch {
                condition: "draft".into(),
                then_steps: vec![ask("Send {{draft}}?", &["yes", "no"], "send"), notify("send={{send}}")],
                else_steps: vec![],
            },
            notify("End."),
        ];
        let mut desk = Desk::new();
        let id = desk.start(&steps);
        RecipeStore::set_var(&desk.conn, &id, "draft", &json!("the memo"));
        assert_eq!(run(&mut desk, &id, 20), 1, "it asks, and waits");
        assert_eq!(desk.status(&id), (RecipeStatus::Waiting, 0), "at the Branch, which is not done");
        assert_eq!(desk.said.last().map(String::as_str), Some("Send the memo?\n1. yes\n2. no"));
        assert!(!RecipeStore::get_expired_waiting_at(&desk.conn, EIGHT_AM + 86_400.0).contains(&id), "no clock ends a question");
        assert_eq!(step(&mut desk, &id), Advance::Blocked, "nor does a stray signal");

        let v = desk.view(&id);
        assert!(v.can.answer);
        assert_eq!(v.question.as_ref().map(|q| q.text.as_str()), Some("Send the memo?"));
        assert!(recipe_view::apply(&desk.conn, &id, &RecipeOp::Answer("yes".into())).expect("answered").run_now);
        run(&mut desk, &id, 20);
        assert_eq!(desk.status(&id).0, RecipeStatus::Done);
        assert!(desk.said.iter().any(|s| s == "send=yes"), "{:?}", desk.said);
        assert_eq!(desk.view(&id).steps[0].path.as_deref(), Some("took then: Ask you (answered) → Notify"));
    }

    /// A JumpIf back is a loop: it goes round until its condition says stop, and the view counts
    /// the rounds.
    #[test]
    fn a_loop_goes_round_until_its_condition_and_counts_its_rounds() {
        let steps = [
            tool("count", "n"),
            RecipeStep::JumpIf {
                condition: Condition::Not { inner: Box::new(Condition::VarEquals { var: "n".into(), value: json!("3") }) },
                target_step: 0,
            },
            notify("n={{n}}"),
        ];
        let mut desk = Desk::new();
        desk.tool_says("count", &["1", "2", "3"]);
        let id = desk.start(&steps);
        run(&mut desk, &id, 50);
        assert_eq!(desk.status(&id).0, RecipeStatus::Done);
        assert_eq!(desk.called.len(), 3, "three rounds");
        assert!(desk.said.iter().any(|s| s == "n=3"), "{:?}", desk.said);
        let v = desk.view(&id);
        assert_eq!(v.steps[1].path.as_deref(), Some("went on to step 3 after looping back 2 times"));
        assert!(v.steps.iter().all(|s| s.state == "done"), "{:?}", v.steps.iter().map(|s| &s.state).collect::<Vec<_>>());
    }

    /// A loop that never waits is stopped, with a reason a person can act on — not left to spin
    /// the worker for ever.
    #[test]
    fn a_loop_that_never_waits_is_stopped_and_says_why() {
        let steps = [tool("poll", "x"), RecipeStep::JumpIf { condition: Condition::VarExists { var: "x".into() }, target_step: 0 }];
        let mut desk = Desk::new();
        let id = desk.start(&steps);
        run(&mut desk, &id, 10_000);
        let r = RecipeStore::get(&desk.conn, &id).expect("the recipe");
        assert_eq!(r.status, RecipeStatus::Failed);
        let why = r.error_message.expect("a reason");
        assert!(why.starts_with("Stopped: 100 steps ran without a pause"), "{why}");
        assert!(why.contains("step 2 had gone back 50 times"), "{why}");
        assert_eq!(desk.called.len(), 50);
        assert!(desk.said.last().is_some_and(|s| s.starts_with("Recipe 'Digest' stopped: 100 steps")), "{:?}", desk.said.last());
        assert_eq!(desk.view(&id).status, "failed");

        // A recipe that needs more, or fewer, says so.
        let mut desk = Desk::new();
        let id = desk.start(&steps);
        RecipeStore::set_var(&desk.conn, &id, STEP_BUDGET_VAR, &json!(6));
        run(&mut desk, &id, 10_000);
        assert_eq!(desk.called.len(), 3);
        assert_eq!(desk.status(&id).0, RecipeStatus::Failed);
    }

    /// A loop that waits each round is not spinning, however many rounds it goes.
    #[test]
    fn a_loop_that_waits_each_round_runs_on() {
        let steps = [
            tool("poll", "x"),
            RecipeStep::WaitFor { condition: WaitCondition::Duration { seconds: 60 }, timeout_secs: None },
            RecipeStep::JumpIf { condition: Condition::VarExists { var: "x".into() }, target_step: 0 },
        ];
        let mut desk = Desk::new();
        let id = desk.start(&steps);
        run(&mut desk, &id, 10);
        for _ in 0..60 {
            desk.clock += 60.0;
            assert_eq!(RecipeStore::get_expired_waiting_at(&desk.conn, desk.clock), vec![id.clone()]);
            run(&mut desk, &id, 10);
        }
        assert_eq!(desk.status(&id), (RecipeStatus::Waiting, 2));
        assert_eq!(desk.called.len(), 61);
    }

    /// A timer wakes on the clock — with nobody talking to the companion — and not before, and a
    /// stray signal does not walk the recipe past it (#176).
    #[test]
    fn a_timer_wakes_on_the_clock_and_not_before() {
        let steps = [
            notify("Starting."),
            RecipeStep::WaitFor { condition: WaitCondition::Duration { seconds: 900 }, timeout_secs: None },
            notify("Later."),
        ];
        let mut desk = Desk::new();
        let id = desk.start(&steps);
        run(&mut desk, &id, 20);
        assert_eq!(desk.status(&id), (RecipeStatus::Waiting, 2));
        // The engine's own clock reading: the test does not hard-code the machine's zone.
        let wake = clock_text(EIGHT_AM + 900.0);
        assert_eq!(desk.view(&id).waiting_for, Some(format!("15m to pass, until {wake}")));
        // A second chain's signal, or a chat turn's sweep, finds it still waiting.
        assert_eq!(step(&mut desk, &id), Advance::Blocked);
        assert_eq!(desk.status(&id), (RecipeStatus::Waiting, 2));
        assert!(!desk.said.iter().any(|s| s == "Later."));

        // Not a second early.
        assert!(RecipeStore::get_expired_waiting_at(&desk.conn, EIGHT_AM + 899.0).is_empty());
        // Paused past its time and resumed: due at once. Its time stands; it does not start over.
        recipe_view::apply(&desk.conn, &id, &RecipeOp::Pause).expect("paused");
        desk.clock = EIGHT_AM + 1_000.0;
        assert!(RecipeStore::get_expired_waiting_at(&desk.conn, desk.clock).is_empty(), "paused");
        assert!(!recipe_view::apply(&desk.conn, &id, &RecipeOp::Resume).expect("resumed").run_now);
        assert_eq!(RecipeStore::get_expired_waiting_at(&desk.conn, desk.clock), vec![id.clone()]);

        run(&mut desk, &id, 20);
        assert_eq!(desk.status(&id).0, RecipeStatus::Done);
        assert!(desk.said.iter().any(|s| s == "Later."));
        assert_eq!(desk.view(&id).steps[1].result, Some(format!("waited until {wake}")));
    }

    /// What the worker's clock moves: the recipes running, and the waits that are over — not a
    /// question, and not a timer still counting.
    #[test]
    fn the_clock_moves_what_is_due() {
        let desk = Desk::new();
        let running = desk.start(&[notify("a")]);
        let asking = desk.start(&[ask("Which?", &["a", "b"], "which"), notify("b")]);
        RecipeStore::complete_step(&desk.conn, &asking, 0, "asked");
        RecipeStore::update_status(&desk.conn, &asking, &RecipeStatus::Waiting, 1);
        let timed = desk.start(&[RecipeStep::WaitFor { condition: WaitCondition::Duration { seconds: 0 }, timeout_secs: None }]);
        let far = json!({"step": 0, "since": now_ts(), "until": now_ts() + 3_600.0});
        RecipeStore::set_var(&desk.conn, &timed, WAIT_VAR, &far);
        RecipeStore::update_status(&desk.conn, &timed, &RecipeStatus::Waiting, 1);
        let over = desk.start(&[RecipeStep::WaitFor { condition: WaitCondition::Duration { seconds: 0 }, timeout_secs: None }]);
        RecipeStore::set_var(&desk.conn, &over, WAIT_VAR, &json!({"step": 0, "since": 1.0, "until": 2.0}));
        RecipeStore::update_status(&desk.conn, &over, &RecipeStatus::Waiting, 1);
        let mut moved = due(&desk.conn);
        moved.sort();
        let mut expected = vec![running, over];
        expected.sort();
        assert_eq!(moved, expected);
    }

    /// A tool's failure goes where its `on_error` says: skipped here and the recipe runs on;
    /// failed there, and the recipe stops with the tool's words.
    #[test]
    fn a_failed_tool_does_what_its_on_error_says() {
        let skip = RecipeStep::Tool { tool_name: "flaky".into(), args: json!({}), store_as: "f".into(), on_error: ErrorAction::Skip };
        let mut desk = Desk::new();
        desk.tool_says("flaky", &["Error: no network"]);
        let id = desk.start(&[skip, notify("on")]);
        run(&mut desk, &id, 20);
        assert_eq!(desk.status(&id).0, RecipeStatus::Done);
        assert_eq!(desk.view(&id).steps[0].state, "skipped");

        let mut desk = Desk::new();
        desk.tool_says("flaky", &["Error: no network"]);
        let id = desk.start(&[tool("flaky", "f"), notify("never")]);
        run(&mut desk, &id, 20);
        let r = RecipeStore::get(&desk.conn, &id).expect("the recipe");
        assert_eq!((r.status, r.error_message.as_deref()), (RecipeStatus::Failed, Some("Error: no network")));
        assert!(!desk.said.iter().any(|s| s == "never"));
    }

    /// The shell's host: the companion runs a recipe's tools from its registry, its Think step
    /// through its model, the data steps for real, and says what the recipe came to as its own
    /// message — the executor the shell's worker calls, end to end.
    #[test]
    fn the_companion_runs_a_recipe_with_its_own_tools_and_model() {
        use yantrik_ml::{LLMBackend, LLMResponse};

        struct Scripted;
        impl LLMBackend for Scripted {
            fn chat(&self, _m: &[ChatMessage], _c: &GenerationConfig, _t: Option<&[serde_json::Value]>) -> anyhow::Result<LLMResponse> {
                Ok(LLMResponse {
                    text: "<think>hm</think>Two recipes, both fine.".into(),
                    prompt_tokens: 0,
                    completion_tokens: 1,
                    tool_calls: vec![],
                    api_tool_calls: vec![],
                    stop_reason: "stop".into(),
                })
            }
            fn chat_streaming(
                &self,
                m: &[ChatMessage],
                c: &GenerationConfig,
                t: Option<&[serde_json::Value]>,
                _on_token: &mut dyn FnMut(&str),
            ) -> anyhow::Result<LLMResponse> {
                self.chat(m, c, t)
            }
            fn count_tokens(&self, text: &str) -> anyhow::Result<usize> {
                Ok(text.len())
            }
            fn backend_name(&self) -> &str {
                "scripted"
            }
        }

        let db = yantrikdb_core::YantrikDB::new(":memory:", 384).expect("in-memory database");
        let mut config = crate::config::CompanionConfig::default();
        config.tools.enabled = false;
        let mut companion = CompanionService::new(db, std::sync::Arc::new(Scripted), config);
        let steps = [
            RecipeStep::Tool { tool_name: "list_recipes".into(), args: json!({}), store_as: "listed".into(), on_error: ErrorAction::Fail },
            RecipeStep::Branch {
                condition: "listed".into(),
                then_steps: vec![RecipeStep::Think { prompt: "Summarise {{listed}}".into(), store_as: "summary".into(), fallback_template: None }],
                else_steps: vec![],
            },
            RecipeStep::Extract { input_var: "summary".into(), pattern: "/[A-Z][a-z]+ recipes/".into(), store_as: "head".into() },
            notify("{{head}}: {{summary}}"),
        ];
        let id = RecipeStore::create(&companion.db.conn(), "Recipe check", "", &steps, None);
        RecipeStore::update_status(&companion.db.conn(), &id, &RecipeStatus::Running, 0);
        run(&mut companion, &id, 20);

        let (status, vars) = {
            let conn = companion.db.conn();
            (RecipeStore::get(&conn, &id).map(|r| r.status), RecipeStore::get_vars(&conn, &id))
        };
        assert_eq!(status, Some(RecipeStatus::Done));
        assert!(vars["listed"].as_str().is_some_and(|s| s.starts_with("Recipes (")), "{:?}", vars["listed"]);
        assert_eq!(vars["summary"], json!("Two recipes, both fine."), "the model's answer, its thinking stripped");
        assert_eq!(vars["head"], json!("Two recipes"), "Extract ran for real, not passed through");
        let said = companion.take_proactive_message().expect("the companion tells the person");
        assert!(
            said.urge_ids.len() == 1 && said.urge_ids[0].starts_with(&format!("recipe:{id}:")),
            "the message keys on its own delivery, still named for its recipe: {:?}",
            said.urge_ids
        );
        assert!(said.text.contains("Two recipes: Two recipes, both fine."), "{}", said.text);
    }

    // ── Agent steps ──

    /// An Agent step hands its role the task — its `{{vars}}` filled in — and keeps the answer in
    /// `store_as`, where the next step reads it; the step is ticked with the answer's head, its
    /// stage says who answered on what mind, and the agent is let go once its answer is taken.
    #[test]
    fn an_agent_step_keeps_the_roles_answer_where_its_store_as_says() {
        let mut desk = Desk::new();
        desk.hands().says("reviewer", vec![AgentPoll::Answered("Verdict — ship.\nFindings — none.".into())]);
        let id = desk.start_allowed(&[agent("reviewer", "Review {{change}}", "review"), format("Got: {{review}}", "out")]);
        RecipeStore::set_var(&desk.conn, &id, "change", &json!("the patch"));
        run(&mut desk, &id, 20);

        assert_eq!(desk.status(&id).0, RecipeStatus::Done, "{:?}", desk.said);
        assert_eq!(desk.hands().started, [("reviewer".to_string(), "Review the patch".to_string(), String::new())]);
        assert_eq!(desk.var(&id, "review"), Some(json!("Verdict — ship.\nFindings — none.")), "the answer, where store_as says");
        assert_eq!(desk.var(&id, "out"), Some(json!("Got: Verdict — ship.\nFindings — none.")));
        let v = desk.view(&id);
        assert_eq!((v.steps[0].kind.as_str(), v.steps[0].label.as_str(), v.steps[0].state.as_str()), ("agent", "Reviewer · pi", "done"));
        assert_eq!(v.steps[0].result.as_deref(), Some("Verdict — ship. Findings — none."), "ticked with the answer's head");
        assert_eq!(v.steps[0].agent.as_deref(), Some("the Reviewer on pi (pi:c-0001)"));
        assert_eq!(v.agents.len(), 1);
        assert_eq!(v.agents[0].state, "answered");
        let released = &desk.hands().released;
        assert_eq!(released.len(), 1);
        assert!(released[0].1.contains("Its answer went to the Digest recipe"), "{released:?}");
        assert!(desk.said.last().is_some_and(|s| s.contains("Got: Verdict")), "the completion carries it: {:?}", desk.said);
    }

    /// An agent that fails fails the recipe, with its reason, and nothing after it runs. So does a
    /// start the shell refuses, a run nobody allowed agents, a host with no shell, and an agent
    /// that runs past its role's minutes — each saying which.
    #[test]
    fn an_agent_that_does_not_answer_fails_the_recipe_saying_why() {
        let steps = [agent("reviewer", "Review it", "review"), tool("ship", "shipped")];

        let mut desk = Desk::new();
        desk.hands().says("reviewer", vec![AgentPoll::Failed("its turn was stopped: its budget of 15 minutes ran out".into())]);
        let id = desk.start_allowed(&steps);
        run(&mut desk, &id, 20);
        let r = RecipeStore::get(&desk.conn, &id).unwrap();
        assert_eq!(r.status, RecipeStatus::Failed);
        assert_eq!(
            r.error_message.as_deref(),
            Some("The Reviewer (pi:c-0001) did not answer: its turn was stopped: its budget of 15 minutes ran out")
        );
        assert!(desk.called.is_empty(), "nothing after it ran");
        assert_eq!(desk.view(&id).steps[0].state, "failed");
        assert!(desk.said.last().is_some_and(|s| s.starts_with("Recipe 'Digest' failed at step 1: The Reviewer")), "{:?}", desk.said);

        let error_of = |desk: &mut Desk, id: &str| {
            run(desk, id, 20);
            RecipeStore::get(&desk.conn, id).and_then(|r| r.error_message).unwrap_or_default()
        };
        // The shell refuses the start: no mind for the role, the desktop's cap.
        let mut desk = Desk::new();
        desk.hands().refuse = Some(AgentRefusal::Fail("6 agents are running already; the most is 6.".into()));
        let id = desk.start_allowed(&steps);
        assert_eq!(error_of(&mut desk, &id), "The Reviewer could not be started: 6 agents are running already; the most is 6.");
        // A host with no shell.
        let mut desk = Desk::new();
        desk.hands = None;
        let id = desk.start_allowed(&steps);
        assert!(error_of(&mut desk, &id).contains("only the desktop shell runs Agent steps"));

        // Past its role's minutes (10, and two over), still working: failed, and let go.
        let mut desk = Desk::new();
        desk.hands().says("reviewer", vec![AgentPoll::Working]);
        let id = desk.start_allowed(&steps);
        run(&mut desk, &id, 20);
        assert_eq!(desk.status(&id).0, RecipeStatus::Waiting, "waiting, at the step that follows, for the answer");
        desk.clock += 11.0 * 60.0;
        desk.tick();
        assert_eq!(desk.status(&id).0, RecipeStatus::Waiting, "still inside its minutes");
        desk.clock += 60.0;
        desk.tick();
        let r = RecipeStore::get(&desk.conn, &id).unwrap();
        assert_eq!(r.status, RecipeStatus::Failed);
        assert_eq!(r.error_message.as_deref(), Some("The Reviewer (pi:c-0001) did not answer within its 10 minutes."));
        assert_eq!(desk.hands().released.len(), 1, "the agent is let go");
    }

    /// A recipe waiting on its agent holds up nobody: the step that needs the answer waits, the
    /// worker goes on to other recipes, and the clock only asks how the agent is doing.
    #[test]
    fn waiting_on_an_agent_does_not_hold_up_other_recipes() {
        let mut desk = Desk::new();
        desk.hands().says("researcher", vec![AgentPoll::Working]);
        let waiting = desk.start_allowed(&[agent("researcher", "Find it", "found"), format("Found: {{found}}", "out")]);
        let other = desk.start(&[tool("a", "x"), tool("b", "y"), notify("{{x}} {{y}}")]);

        desk.tick();
        assert_eq!(desk.status(&other).0, RecipeStatus::Done, "the other recipe ran to its end");
        assert_eq!(desk.status(&waiting), (RecipeStatus::Waiting, 1), "at the step that reads the answer");
        assert_eq!(desk.var(&waiting, "out"), None, "which did not run");
        let v = desk.view(&waiting);
        assert_eq!(v.waiting_for.as_deref(), Some("the Researcher's answer (pi)"));
        assert!(v.waits_on_agents);
        assert_eq!((v.steps[0].state.as_str(), v.steps[0].label.as_str()), ("waiting", "Researcher · pi"));
        assert_eq!(v.steps[1].state, "pending");
        assert!(v.steps[1].unbound.is_empty(), "the answer is coming: {:?}", v.steps[1].unbound);
        assert_eq!(recipe_view::one_line(&v), "Digest — step 2 of 2, Format, waiting for the Researcher's answer (pi)");

        // Each tick asks once and returns at once.
        let polls = desk.hands().polls;
        assert_eq!(step(&mut desk, &waiting), Advance::Blocked);
        assert_eq!(desk.hands().polls, polls + 1);
        assert!(due(&desk.conn).contains(&waiting), "the clock keeps asking");

        desk.hands().answer("pi:c-0001", "It is in the attic.");
        desk.tick();
        assert_eq!(desk.status(&waiting).0, RecipeStatus::Done);
        assert_eq!(desk.var(&waiting, "out"), Some(json!("Found: It is in the attic.")));
    }

    /// A restart while an agent works leaves the recipe honest: it waits on in the store, the
    /// clock picks it up, and the shell's answer decides — the answer the agent's saved session
    /// kept, or the reason it will never come. Never "running" for ever.
    #[test]
    fn a_restart_while_an_agent_works_leaves_the_recipe_honest() {
        let steps = [agent("coder", "Fix it", "change"), format("{{change}}", "out")];

        // The shell came back, and the agent's turn had been cut off with it.
        let mut desk = Desk::new();
        desk.hands().says("coder", vec![AgentPoll::Working]);
        let id = desk.start_allowed(&steps);
        desk.tick();
        assert_eq!(desk.status(&id).0, RecipeStatus::Waiting);
        let mut after = Hands::default();
        after.agents.insert("pi:c-0001".into(), (id.clone(), AgentPoll::Failed("[The shell stopped while this turn was running.]".into())));
        desk.hands = Some(after);
        assert!(due(&desk.conn).contains(&id), "the clock picks it up after the restart");
        desk.tick();
        let r = RecipeStore::get(&desk.conn, &id).unwrap();
        assert_eq!(r.status, RecipeStatus::Failed);
        assert_eq!(r.error_message.as_deref(), Some("The Coder (pi:c-0001) did not answer: [The shell stopped while this turn was running.]"));

        // It had answered just before: the saved session has the answer, and the recipe goes on.
        let mut desk = Desk::new();
        desk.hands().says("coder", vec![AgentPoll::Working]);
        let id = desk.start_allowed(&steps);
        desk.tick();
        let mut after = Hands::default();
        after.agents.insert("pi:c-0001".into(), (id.clone(), AgentPoll::Answered("Changed — src/a.rs".into())));
        desk.hands = Some(after);
        desk.tick();
        assert_eq!(desk.status(&id).0, RecipeStatus::Done);
        assert_eq!(desk.var(&id, "out"), Some(json!("Changed — src/a.rs")));

        // The desktop no longer knows the agent at all.
        let mut desk = Desk::new();
        desk.hands().says("coder", vec![AgentPoll::Working]);
        let id = desk.start_allowed(&steps);
        desk.tick();
        desk.hands = Some(Hands::default());
        desk.tick();
        let err = RecipeStore::get(&desk.conn, &id).and_then(|r| r.error_message).unwrap_or_default();
        assert_eq!(err, "The Coder (pi:c-0001) did not answer: the desktop no longer knows `pi:c-0001`");

        // Back on a host with no shell to ask: said so, not left waiting.
        let mut desk = Desk::new();
        desk.hands().says("coder", vec![AgentPoll::Working]);
        let id = desk.start_allowed(&steps);
        desk.tick();
        desk.hands = None;
        desk.tick();
        let err = RecipeStore::get(&desk.conn, &id).and_then(|r| r.error_message).unwrap_or_default();
        assert!(err.contains("nothing here can hear its answer"), "{err}");
    }

    /// At most three agents at once: a fourth Agent step waits for a place — it does not fail —
    /// and starts when one of the three answers. The shell's own refusal (the desktop's cap) is a
    /// failure, with the shell's sentence; the recipe's own cap is a wait.
    #[test]
    fn a_fourth_agent_waits_for_a_place_and_starts_when_one_answers() {
        let mut desk = Desk::new();
        for role in ["researcher", "planner", "writer", "scribe"] {
            desk.hands().says(role, vec![AgentPoll::Working]);
        }
        let id = desk.start_allowed(&[
            agent("researcher", "a", "a"),
            agent("planner", "b", "b"),
            agent("writer", "c", "c"),
            agent("scribe", "d", "d"),
            format("{{a}} {{b}} {{c}} {{d}}", "all"),
        ]);
        desk.tick();
        assert_eq!(desk.hands().roles(), ["researcher", "planner", "writer"], "three at once, and no more");
        assert_eq!(desk.status(&id), (RecipeStatus::Waiting, 3), "the fourth waits at its step");
        let v = desk.view(&id);
        assert_eq!(v.waiting_for.as_deref(), Some("a place for its next agent: 3 of its agents are working, the most one recipe has at once"));
        assert_eq!(v.steps.iter().map(|s| s.state.as_str()).collect::<Vec<_>>(), ["waiting", "waiting", "waiting", "pending", "pending"]);
        desk.tick();
        assert_eq!(desk.hands().started.len(), 3, "still waiting, still three");

        desk.hands().answer("pi:c-0002", "the plan");
        desk.tick();
        assert_eq!(desk.hands().roles(), ["researcher", "planner", "writer", "scribe"], "a place came free");
        assert_eq!(desk.hands().working(), ["pi:c-0001", "pi:c-0003", "pi:c-0004"]);
        assert_eq!(desk.view(&id).waiting_for.as_deref(), Some("answers from the Researcher (pi), the Writer (pi) and the Scribe (pi)"));
        for agent in ["pi:c-0001", "pi:c-0003", "pi:c-0004"] {
            desk.hands().answer(agent, agent);
        }
        desk.tick();
        assert_eq!(desk.status(&id).0, RecipeStatus::Done);
        assert_eq!(desk.var(&id, "all"), Some(json!("pi:c-0001 the plan pi:c-0003 pi:c-0004")));
    }

    /// A recipe cancelled while its agents work — from the screen, the chat or `cancel_recipe`,
    /// none of which runs a step — lets them go at the clock's next tick.
    #[test]
    fn a_cancelled_recipe_lets_its_agents_go() {
        let mut desk = Desk::new();
        desk.hands().says("researcher", vec![AgentPoll::Working]);
        let id = desk.start_allowed(&[agent("researcher", "a", "a"), agent("researcher", "b", "b"), format("{{a}}{{b}}", "x")]);
        desk.tick();
        assert_eq!(desk.hands().working().len(), 2);
        recipe_view::apply(&desk.conn, &id, &RecipeOp::Cancel).expect("cancelled");
        assert!(due(&desk.conn).contains(&id), "the clock still has its agents to let go");
        desk.tick();
        let released = &desk.hands().released;
        assert_eq!(released.len(), 2, "{released:?}");
        assert!(released.iter().all(|(_, why)| why == "The Digest recipe was cancelled, so its work is no longer wanted."), "{released:?}");
        assert!(!due(&desk.conn).contains(&id), "and then there is nothing left to do");
        assert!(recipe_view::list(&desk.conn).iter().find(|v| v.id == id).unwrap().agents.iter().all(|a| a.state == "released"));
    }

    /// The hook is told what the person agreed to: for a run started at the desk, that it was, and
    /// the digest of each role's definition as it was then — or none, for a role the run named
    /// only later; for a run nobody started at the desk (a trigger, a timer), that nobody did, so
    /// it asks before any role above `safe`. While it asks, the recipe waits at the step, needing
    /// the person, and nothing reads as done.
    #[test]
    fn the_hook_is_told_what_the_person_agreed_to() {
        let steps = [agent("reviewer", "Review it", "review"), agent("{{later}}", "More", "more"), format("{{review}} {{more}}", "out")];
        let mut desk = Desk::new();
        let id = desk.start(&steps);
        let mut leave = Leave::new("the person, from the Recipes screen", None);
        leave.roles.insert("reviewer".into(), "digest-of-the-reviewer-then".into());
        RecipeStore::allow_agents(&desk.conn, &id, &leave);
        assert_eq!(RecipeStore::agents_allowed(&desk.conn, &id), Some(leave), "the leave reads back, roles and all");
        RecipeStore::set_var(&desk.conn, &id, "later", &json!("coder"));
        desk.tick();
        assert_eq!(
            desk.hands().told,
            [(true, Some("digest-of-the-reviewer-then".to_string())), (true, None)],
            "an agreed role with its digest; a role named later with none, which the shell refuses"
        );

        // A run nobody started at the desk: the hook asks, and the step waits for the person.
        let mut desk = Desk::new();
        desk.hands().refuse = Some(AgentRefusal::Ask("your Allow on the card: Digest recipe → Reviewer".into()));
        let id = desk.start(&steps);
        RecipeStore::set_var(&desk.conn, &id, "later", &json!("coder"));
        desk.tick();
        assert_eq!(desk.hands().told, [(false, None)], "told that nobody started it at the desk");
        assert_eq!(desk.status(&id), (RecipeStatus::Waiting, 0), "at the step, not past it");
        let v = desk.view(&id);
        assert_eq!(v.needs_you.as_deref(), Some("your Allow on the card: Digest recipe → Reviewer"));
        assert_eq!(v.waiting_for, v.needs_you);
        assert_eq!(v.steps[0].state, "waiting");
        assert_eq!(recipe_view::one_line(&v), "Digest — needs you: your Allow on the card: Digest recipe → Reviewer");
        // Allowed: the next tick starts it and the recipe goes on.
        desk.hands().refuse = None;
        desk.tick();
        assert_eq!(desk.hands().roles(), ["reviewer", "coder"]);
        assert!(desk.view(&id).needs_you.is_none());
    }

    /// A start the shell put off — no place for it under a cap the recipe does not own, or a card
    /// not yet answered — waits at its step needing the person, asked again every tick, and is
    /// never done; past its time it fails, saying what it waited for. It does not race: nothing is
    /// started until the shell says there is a place.
    #[test]
    fn a_start_put_off_needs_you_and_is_never_done() {
        let place = "a place: the desktop is running 6 agents, the most it runs at once — stop one on the Agents screen, or it starts when one finishes";
        let mut desk = Desk::new();
        desk.hands().refuse = Some(AgentRefusal::Wait(place.into()));
        let id = desk.start_allowed(&[agent("reviewer", "Review it", "review")]);
        desk.tick();
        for _ in 0..5 {
            desk.clock += CLOCK_SECS as f64;
            desk.tick();
        }
        assert_eq!(desk.status(&id), (RecipeStatus::Waiting, 0));
        assert!(desk.hands().started.is_empty(), "nothing started while there is no place");
        assert_eq!(desk.hands().told.len(), 6, "asked again each tick");
        let v = desk.view(&id);
        assert_eq!(v.status, "waiting");
        assert_eq!(v.needs_you.as_deref(), Some(place));
        assert_eq!(v.steps[0].state, "waiting");
        assert!(desk.var(&id, "review").is_none(), "no answer, and no empty one");

        // A place comes free: it starts, and the recipe no longer needs the person.
        desk.hands().refuse = None;
        desk.clock += CLOCK_SECS as f64;
        desk.tick();
        assert_eq!(desk.status(&id).0, RecipeStatus::Done);
        assert!(desk.view(&id).needs_you.is_none());

        // Put off past its time: failed, with what it waited for — not done.
        let mut desk = Desk::new();
        desk.hands().refuse = Some(AgentRefusal::Ask("your Allow on the card: Digest recipe → Reviewer".into()));
        let id = desk.start_allowed(&[agent("reviewer", "Review it", "review"), format("{{review}}", "out")]);
        desk.tick();
        desk.clock += PUT_OFF_MOST_SECS as f64 - 1.0;
        desk.tick();
        assert_eq!(desk.status(&id).0, RecipeStatus::Waiting, "still inside its time");
        desk.clock += 1.0;
        desk.tick();
        let r = RecipeStore::get(&desk.conn, &id).unwrap();
        assert_eq!(r.status, RecipeStatus::Failed);
        assert_eq!(
            r.error_message.as_deref(),
            Some("Its Agent step waited 30m and was not started: it was waiting for your Allow on the card: Digest recipe → Reviewer.")
        );
        assert!(desk.var(&id, "out").is_none());
    }

    /// A start asked again from the wait the shell put it in hears why it was put off — all that
    /// survives a desktop restart of the card that was on screen, and what lets the shell know
    /// the card it raises now must say it asks again because of the restart (#194). A first ask
    /// hears nothing behind it.
    #[test]
    fn a_start_asked_again_tells_the_hook_what_it_waited_for() {
        let card = "your Allow on the card: Digest recipe → Reviewer";
        let mut desk = Desk::new();
        desk.hands().refuse = Some(AgentRefusal::Ask(card.into()));
        let id = desk.start(&[agent("reviewer", "Review it", "review")]);
        desk.tick();
        assert_eq!(desk.hands().heard_put_off, vec![None], "a first ask has nothing behind it");
        // The tick after the put-off — as after a desktop restart, when the durable wait is all
        // that survives — asks again, with the wait's own reason.
        desk.tick();
        assert_eq!(desk.hands().heard_put_off, vec![None, Some(card.to_string())]);
        // Allowed at last: the step starts on the reason it waited for, and the recipe goes on.
        desk.hands().refuse = None;
        desk.tick();
        assert_eq!(desk.hands().heard_put_off[2], Some(card.to_string()));
        assert_eq!(desk.status(&id).0, RecipeStatus::Done);
    }

    /// An agent waiting on the person in its own pane — a card it asked for — makes its recipe
    /// need the person, with what for; the card left to expire is no answer, and the recipe fails
    /// saying so rather than going on with whatever the agent said after.
    #[test]
    fn an_agent_waiting_on_you_makes_its_recipe_need_you() {
        let card = "the Coder (pi:c-0001) is waiting for your Allow on a card in its pane: shell.agent_run";
        let mut desk = Desk::new();
        desk.hands().says("coder", vec![AgentPoll::NeedsYou(card.into())]);
        let id = desk.start_allowed(&[agent("coder", "Fix it", "change"), format("{{change}}", "out")]);
        desk.tick();
        let v = desk.view(&id);
        assert_eq!(v.needs_you.as_deref(), Some(card));
        assert_eq!(v.agents[0].needs_you.as_deref(), Some(card));
        assert_eq!(recipe_view::one_line(&v), format!("Digest — needs you: {card}"));
        let expired = "its card for shell.agent_run went unanswered and expired, so its work is not what was asked";
        desk.hands().agents.insert("pi:c-0001".into(), (id.clone(), AgentPoll::Failed(expired.into())));
        desk.tick();
        let r = RecipeStore::get(&desk.conn, &id).unwrap();
        assert_eq!(r.status, RecipeStatus::Failed, "never done on an unanswered card");
        assert_eq!(r.error_message.as_deref(), Some(format!("The Coder (pi:c-0001) did not answer: {expired}").as_str()));
        assert!(desk.var(&id, "out").is_none());
    }

    // ── The four formations ──

    use crate::recipe::Leave;
    use crate::recipe_templates::{self, formations};

    fn person() -> Leave {
        Leave::new("the person, from the Recipes screen", None)
    }

    /// A formation, registered as the shell registers the built-ins and started as the Recipes
    /// screen starts it.
    fn formation(desk: &Desk, id: &str, inputs: serde_json::Value) -> String {
        recipe_templates::register_all(&desk.conn);
        let vars = inputs.as_object().cloned().unwrap_or_default();
        let (_, run) = recipe_templates::start(&desk.conn, id, Some(&vars), Some(&person())).expect("started");
        run
    }

    /// Every formation's steps read back as written (they are stored as JSON), take one input a
    /// run must be given, hand work only to catalog roles, and tell the person one thing: no
    /// Notify and no question on the way (#187), and a last step that keeps what it came to.
    #[test]
    fn every_formation_parses_and_says_one_thing_once() {
        let desk = Desk::new();
        recipe_templates::register_all(&desk.conn);
        let ids = [formations::COUNCIL, formations::RED_TEAM, formations::BUILD, formations::WRITERS_ROOM];
        let catalog = ["researcher", "planner", "coder", "reviewer", "red-team", "writer", "chair", "scribe"];
        for id in ids {
            let template = recipe_templates::get_template(id).expect("registered");
            let written = (template.steps)();
            let stored: Vec<RecipeStep> = RecipeStore::get_steps(&desk.conn, id).into_iter().map(|s| s.step).collect();
            assert_eq!(serde_json::to_value(&stored).unwrap(), serde_json::to_value(&written).unwrap(), "{id} reads back as written");
            assert!(stored.iter().all(|s| !matches!(s, RecipeStep::Notify { .. } | RecipeStep::AskUser { .. })), "{id}");
            assert!(crate::recipe_view::store_as(stored.last().unwrap()).is_some(), "{id} ends on a step that keeps its result");
            assert_eq!(template.required_vars.len(), 1, "{id} takes one input a run must give it");
            assert_eq!(template.category, "formations");
            let defaults: Vars = recipe_templates::defaults(id).iter().map(|(k, v, _)| (k.to_string(), json!(v))).collect();
            for step in &stored {
                if let RecipeStep::Agent { role, .. } = step {
                    let role = crate::recipe::resolve_vars(role, &defaults);
                    assert!(catalog.contains(&role.as_str()), "{id} hands work to `{role}`, which the catalog has");
                }
            }
            let v = recipe_view::list(&desk.conn).into_iter().find(|v| v.id == id).unwrap();
            assert!(v.template && v.formation, "{id}");
            let unbound: Vec<&String> = v.steps.iter().flat_map(|s| &s.unbound).collect();
            assert!(
                unbound.iter().all(|u| template.required_vars.iter().any(|(n, _)| n == u)),
                "{id}: only its input is unbound on the template: {unbound:?}"
            );
        }
        let council = recipe_view::list(&desk.conn).into_iter().find(|v| v.id == formations::COUNCIL).unwrap();
        assert_eq!(council.steps.iter().map(|s| s.label.as_str()).collect::<Vec<_>>(), ["Researcher", "Red team", "Planner", "Chair"]);
        assert_eq!(council.inputs[0].name, "question");
        assert_eq!(council.inputs[0].default, None);
        assert_eq!(council.inputs[2].default.as_deref(), Some("red-team"));

        // Started without the person's leave — the companion's own run_recipe — or without its
        // input: refused, and nothing is started.
        let vars = json!({"question": "x"}).as_object().cloned().unwrap();
        let err = recipe_templates::start(&desk.conn, "Council", Some(&vars), None).unwrap_err();
        assert!(err.contains("needs the person's leave") && err.contains("run_recipe"), "{err}");
        let err = recipe_templates::start(&desk.conn, formations::COUNCIL, None, Some(&person())).unwrap_err();
        assert_eq!(err, "'Council' needs `question` (The question the council is to answer) to start.");
        let blank = json!({"question": "  "}).as_object().cloned().unwrap();
        assert!(recipe_templates::start(&desk.conn, formations::COUNCIL, Some(&blank), Some(&person())).is_err());
        assert!(RecipeStore::list(&desk.conn, Some("running"), 10).is_empty());

        // The roles a start agrees to: the seats as given, the defaults for the rest — and the
        // leave that carries their digests is the run's.
        let seated = json!({"question": "q", "seat_2": "reviewer"}).as_object().cloned().unwrap();
        assert_eq!(recipe_templates::roles_for(&desk.conn, "Council", Some(&seated)).unwrap(), ["researcher", "reviewer", "planner", "chair"]);
        assert_eq!(recipe_templates::roles_for(&desk.conn, formations::BUILD, None).unwrap(), ["planner", "coder", "reviewer"]);
        let mut leave = person();
        leave.roles.insert("researcher".into(), "d1".into());
        let (_, run) = recipe_templates::start(&desk.conn, "Council", Some(&seated), Some(&leave)).unwrap();
        assert_eq!(RecipeStore::agents_allowed(&desk.conn, &run).map(|l| l.roles), Some(leave.roles));
    }

    /// Council: the three seats are handed the same question at once — none waits for another —
    /// and the Chair, only when all three have answered, with the three answers to weigh. Four
    /// hand-offs, in that order. A run can seat other roles.
    #[test]
    fn a_council_seats_three_at_once_and_the_chair_weighs_them() {
        let mut desk = Desk::new();
        for role in ["researcher", "red-team", "planner"] {
            desk.hands().says(role, vec![AgentPoll::Working]);
        }
        let id = formation(&desk, formations::COUNCIL, json!({"question": "Should we ship on Friday?"}));
        desk.tick();
        assert_eq!(desk.hands().roles(), ["researcher", "red-team", "planner"], "all three seats at once");
        assert!(desk.hands().started.iter().all(|(_, task, _)| task.ends_with("The question: Should we ship on Friday?")));
        let v = desk.view(&id);
        let stages: Vec<(&str, &str)> = v.steps.iter().map(|s| (s.label.as_str(), s.state.as_str())).collect();
        assert_eq!(stages, [("Researcher · pi", "waiting"), ("Red team · pi", "waiting"), ("Planner · pi", "waiting"), ("Chair", "pending")]);
        assert_eq!(v.waiting_for.as_deref(), Some("answers from the Researcher (pi), the Red team (pi) and the Planner (pi)"));

        desk.hands().answer("pi:c-0001", "Yes: the tests pass.");
        desk.hands().answer("pi:c-0002", "No: nobody is on call Saturday.");
        desk.tick();
        assert_eq!(desk.hands().started.len(), 3, "the Chair waits for the third");
        desk.hands().answer("pi:c-0003", "Monday: plan the on-call first.");
        desk.tick();
        assert_eq!(desk.hands().roles(), ["researcher", "red-team", "planner", "chair"]);
        let (_, task, context) = &desk.hands().started[3];
        assert!(task.contains("Should we ship on Friday?") && task.contains("verdict"), "{task}");
        assert_eq!(
            context,
            "From the researcher:\nYes: the tests pass.\n\nFrom the red-team:\nNo: nobody is on call Saturday.\n\nFrom the planner:\nMonday: plan the on-call first."
        );
        assert_eq!(desk.status(&id).0, RecipeStatus::Done);
        assert!(desk.var(&id, "verdict").is_some_and(|v| v.as_str().unwrap().starts_with("chair answered")));
        assert_eq!(desk.said.len(), 1, "one message, the completion: {:?}", desk.said);
        assert!(desk.said[0].starts_with("Recipe completed: Council\n\nResult: chair answered"), "{:?}", desk.said);

        // Other seats.
        let mut desk = Desk::new();
        let id = formation(&desk, formations::COUNCIL, json!({"question": "q", "seat_2": "reviewer", "chair": "scribe"}));
        desk.tick();
        assert_eq!(desk.status(&id).0, RecipeStatus::Done);
        assert_eq!(desk.hands().roles(), ["researcher", "reviewer", "planner", "scribe"]);
    }

    /// Red team: author, attacker, author, attacker, author — two rounds, each reading the last —
    /// and the final proposal is the result.
    #[test]
    fn a_red_team_goes_two_rounds() {
        let mut desk = Desk::new();
        let id = formation(&desk, formations::RED_TEAM, json!({"brief": "a way to back up the photos"}));
        desk.tick();
        assert_eq!(desk.status(&id).0, RecipeStatus::Done);
        assert_eq!(desk.hands().roles(), ["planner", "red-team", "planner", "red-team", "planner"]);
        let started = &desk.hands().started;
        assert_eq!(started[1].2, "The proposal:\nplanner answered: Propose: a way to back up the photos");
        assert!(started[3].2.contains("The revised proposal:\nplanner answered: Revise your proposal"), "{}", started[3].2);
        assert!(desk.var(&id, "proposal").is_some_and(|p| p.as_str().unwrap().contains("one last time")));
        // Another author.
        let mut desk = Desk::new();
        formation(&desk, formations::RED_TEAM, json!({"brief": "b", "author": "writer"}));
        desk.tick();
        assert_eq!(desk.hands().roles(), ["writer", "red-team", "writer", "red-team", "writer"]);
    }

    /// Build: Planner → Coder → Reviewer; a Reviewer that asks for changes sends its findings back
    /// to the Coder — once — by a JumpIf on its answer, and then it stops whatever it says.
    #[test]
    fn a_build_loops_back_to_the_coder_once_on_the_reviewers_answer() {
        // Shipped the first time: no loop.
        let mut desk = Desk::new();
        desk.hands().says("reviewer", vec![AgentPoll::Answered("Verdict — ship.\nROUND: ship".into())]);
        let id = formation(&desk, formations::BUILD, json!({"goal": "add a --dry-run flag"}));
        desk.tick();
        assert_eq!(desk.status(&id).0, RecipeStatus::Done);
        assert_eq!(desk.hands().roles(), ["planner", "coder", "reviewer"]);
        assert!(desk.hands().started[1].2.ends_with("findings to address:\nNone yet: this is the first version."));

        // Changes asked for, every time: back to the Coder once, with the findings, and then done.
        let mut desk = Desk::new();
        desk.hands().says(
            "reviewer",
            vec![
                AgentPoll::Answered("Verdict — fix first.\nFindings — flag is ignored.\nROUND: fix".into()),
                AgentPoll::Answered("Verdict — fix first.\nFindings — help text.\nROUND: fix".into()),
            ],
        );
        let id = formation(&desk, formations::BUILD, json!({"goal": "add a --dry-run flag"}));
        desk.tick();
        assert_eq!(desk.status(&id).0, RecipeStatus::Done, "{:?}", RecipeStore::get(&desk.conn, &id));
        assert_eq!(desk.hands().roles(), ["planner", "coder", "reviewer", "coder", "reviewer"], "one loop, then it stops");
        let second = &desk.hands().started[3].2;
        assert!(second.contains("findings to address:\nVerdict — fix first.\nFindings — flag is ignored.\nROUND: fix"), "{second}");
        let result = desk.var(&id, "result").unwrap();
        assert!(result.as_str().unwrap().ends_with("Findings — help text.\nROUND: fix"), "the last review is in the result: {result}");
        let v = desk.view(&id);
        assert_eq!(v.steps[6].path.as_deref(), Some("looped back to step 3 (1 time so far)"), "the way back, taken once");
        assert_eq!(v.steps[4].path.as_deref(), Some("jumped to step 8"), "and on the second round, out");
        assert_eq!(v.steps[2].label, "Coder · pi");
        assert_eq!(v.agents.iter().filter(|a| a.state == "answered").count(), 3, "one per Agent step: the last round's");
        assert_eq!(desk.hands().released.len(), 5, "every agent let go once it answered");
    }

    /// Writers' room: three writers, one voice each, at once, on the showrunner's beats; the
    /// Scribe assembles the scene from their lines.
    #[test]
    fn a_writers_room_writes_three_voices_at_once_and_the_scribe_assembles() {
        let mut desk = Desk::new();
        desk.hands().says("writer", vec![AgentPoll::Working]);
        let id = formation(&desk, formations::WRITERS_ROOM, json!({"beats": "They meet; the door will not open; one of them has the key"}));
        desk.tick();
        assert_eq!(desk.hands().roles(), ["writer", "writer", "writer"], "three at once");
        let voices: Vec<bool> = ["first", "second", "third"]
            .iter()
            .zip(&desk.hands().started)
            .map(|(which, (_, task, context))| {
                task.contains(&format!("only for the {which} of them"))
                    && task.contains("Voice A, Voice B, Narrator")
                    && context.contains("the door will not open")
            })
            .collect();
        assert_eq!(voices, [true, true, true]);
        for (n, agent) in ["pi:c-0001", "pi:c-0002", "pi:c-0003"].iter().enumerate() {
            desk.hands().answer(agent, &format!("lines {}", n + 1));
        }
        desk.tick();
        assert_eq!(desk.hands().roles(), ["writer", "writer", "writer", "scribe"]);
        let (_, task, context) = &desk.hands().started[3];
        assert!(task.starts_with("Do not summarise this time: assemble the scene"), "{task}");
        assert!(
            context.ends_with("The first voice's lines:\nlines 1\n\nThe second voice's lines:\nlines 2\n\nThe third voice's lines:\nlines 3"),
            "{context}"
        );
        assert_eq!(desk.status(&id).0, RecipeStatus::Done);
    }

    /// What the executor's clock moves now includes the triggers whose time has come (#187):
    /// they were stored and never fired — `get_enabled_triggers` had no caller — so a recipe
    /// with a schedule never started by itself.
    #[test]
    fn the_clock_starts_a_recipe_whose_trigger_is_due() {
        let mut desk = Desk::new();
        // Every minute: a schedule that reads the same whatever zone the test machine is in.
        let id = RecipeStore::create(
            &desk.conn,
            "Digest",
            "",
            &[notify("digest ready")],
            Some(&TriggerType::Cron { expression: "* * * * *".into() }),
        );
        // create() stamps with the real clock; this recipe lives on the test's calendar.
        desk.conn.execute("UPDATE recipes SET created_at = ?1 WHERE id = ?2", rusqlite::params![desk.clock - 120.0, id]).unwrap();

        assert!(due_at(&desk.conn, desk.clock - 61.0).is_empty(), "before the first occurrence since it existed: nothing to move");
        assert_eq!(due_at(&desk.conn, desk.clock), vec![id.clone()], "the due occurrence starts its recipe, on the clock");
        // And the clock runs it the way it runs anything else due.
        for rid in due_at(&desk.conn, desk.clock) {
            run(&mut desk, &rid, 50);
        }
        assert_eq!(desk.status(&id).0, RecipeStatus::Done);
        assert!(desk.said.iter().any(|s| s == "digest ready"), "{:?}", desk.said);
        assert!(due_at(&desk.conn, desk.clock).is_empty(), "the same occurrence fires once");
        desk.clock += 30.0;
        assert!(due_at(&desk.conn, desk.clock).is_empty(), "and not again before the next one");
        desk.clock += 30.0;
        let again = due_at(&desk.conn, desk.clock);
        assert_eq!(again.len(), 1, "the next occurrence fires");
        assert_ne!(again[0], id, "as a run of its own, the last one's record kept");
    }

    /// A formation chained on another: the clock sees the leader's completion and starts the
    /// follower — the wiring that was to run the chair after the council (#187).
    #[test]
    fn the_clock_starts_the_follower_when_its_leader_completes() {
        let mut desk = Desk::new();
        let council = RecipeStore::create(&desk.conn, "Council", "", &[notify("council sat")], None);
        RecipeStore::update_status(&desk.conn, &council, &RecipeStatus::Running, 0);
        let chair = RecipeStore::create(
            &desk.conn,
            "Chair",
            "",
            &[notify("chair sat")],
            Some(&TriggerType::RecipeComplete { recipe_id: council.clone() }),
        );

        assert_eq!(due_at(&desk.conn, desk.clock), vec![council.clone()], "the follower waits on its leader");
        run(&mut desk, &council, 50);
        assert_eq!(desk.status(&council).0, RecipeStatus::Done);
        let moved = due_at(&desk.conn, desk.clock);
        assert!(moved.contains(&chair), "the clock sees the completion and starts the follower: {moved:?}");
        for rid in moved {
            run(&mut desk, &rid, 50);
        }
        assert!(desk.said.iter().any(|s| s == "chair sat"), "{:?}", desk.said);
        assert_eq!(desk.status(&chair).0, RecipeStatus::Done);
        assert!(due_at(&desk.conn, desk.clock).is_empty(), "one completion chains once");
    }

    /// An Agent step inside a Branch's arm runs (#194): its agent is the Branch's own — keyed
    /// under the Branch in `_agents` — the arm goes on while it works, and the Branch joins the
    /// agents it started before it closes, so their answers are in the run's records when the
    /// recipe is done, as a top-level Agent step's are.
    #[test]
    fn an_agent_step_inside_a_branch_is_tracked_and_joined_at_the_branch_s_end() {
        let steps = [
            tool("check", "urgent"),
            RecipeStep::Branch {
                condition: "urgent".into(),
                then_steps: vec![agent("researcher", "Find it", "found"), agent("writer", "Draft it", "draft")],
                else_steps: vec![notify("Nothing urgent.")],
            },
            format("Found {{found}}; drafted {{draft}}", "out"),
        ];
        let mut desk = Desk::new();
        desk.tool_says("check", &["yes"]);
        desk.hands().says("researcher", vec![AgentPoll::Working]);
        desk.hands().says("writer", vec![AgentPoll::Working]);
        let id = desk.start_allowed(&steps);
        run(&mut desk, &id, 20);

        // Both started — the arm went on past the first — and the recipe waits at the Branch
        // for its own join rather than closing it with the answers still out.
        assert_eq!(desk.hands().roles(), ["researcher", "writer"], "the arm ran both Agent steps");
        assert_eq!(desk.status(&id), (RecipeStatus::Waiting, 1), "at the Branch, waiting on its agents");
        let vars = RecipeStore::get_vars(&desk.conn, &id);
        let runs = crate::recipe::agent_runs(&vars);
        assert_eq!(runs.keys().map(String::as_str).collect::<Vec<_>>(), ["1:t0", "1:t1"], "the Branch's agents, under its key");
        assert!(runs.values().all(AgentRun::working));
        assert_eq!(Trail::read(&vars).subs(1), ["waiting", "waiting"], "both arm steps recorded waiting");
        assert!(desk.var(&id, "found").is_none() && desk.var(&id, "draft").is_none(), "no answer yet");
        let v = desk.view(&id);
        assert_eq!(v.steps[1].state, "waiting", "the Branch's row is drawn waiting");
        assert_eq!(v.agents.iter().map(|a| a.step).collect::<Vec<_>>(), [1, 1], "both drawn under the Branch");

        // The answers come: the join is over, the Branch closes, and the recipe finishes with
        // them in its records.
        desk.hands().answer("pi:c-0001", "It is in the attic.");
        desk.hands().answer("pi:c-0002", "A draft.");
        desk.tick();
        assert_eq!(desk.status(&id).0, RecipeStatus::Done, "{:?}", desk.said);
        assert_eq!(desk.var(&id, "found"), Some(json!("It is in the attic.")), "the answer, where store_as says");
        assert_eq!(desk.var(&id, "draft"), Some(json!("A draft.")));
        assert_eq!(desk.var(&id, "out"), Some(json!("Found It is in the attic.; drafted A draft.")), "the step after the Branch read both");
        assert_eq!(desk.var(&id, "_branch"), None, "the Branch closed");
        let vars = RecipeStore::get_vars(&desk.conn, &id);
        assert!(crate::recipe::agent_runs(&vars).values().all(|r| r.state == AgentRun::ANSWERED), "recorded as answered");
        assert_eq!(Trail::read(&vars).subs(1), ["answered", "answered"], "both arm steps settled");
        assert_eq!(desk.hands().released.len(), 2, "both agents let go");
        assert_eq!(desk.view(&id).steps[1].state, "done");
    }

    /// A step inside the arm that reads an arm agent's answer waits for it — mid-arm, with the
    /// Branch drawn waiting — and runs with the answer once it has come (#194).
    #[test]
    fn a_step_inside_the_arm_waits_for_the_answer_it_reads() {
        let steps = [RecipeStep::Branch {
            condition: "x".into(),
            then_steps: vec![],
            else_steps: vec![agent("researcher", "Find it", "found"), notify("Found: {{found}}")],
        }];
        let mut desk = Desk::new();
        desk.hands().says("researcher", vec![AgentPoll::Working]);
        let id = desk.start_allowed(&steps);
        run(&mut desk, &id, 20);
        assert_eq!(desk.status(&id), (RecipeStatus::Waiting, 0), "at the Branch, for the answer its arm reads");
        assert!(desk.said.is_empty(), "the Notify did not run without the answer");
        let v = desk.view(&id);
        assert_eq!(v.steps[0].state, "waiting");
        assert_eq!(v.waiting_for.as_deref(), Some("the Researcher's answer (pi)"));

        desk.hands().answer("pi:c-0001", "It is in the attic.");
        desk.tick();
        assert_eq!(desk.status(&id).0, RecipeStatus::Done, "{:?}", desk.said);
        assert!(desk.said.iter().any(|s| s == "Found: It is in the attic."), "{:?}", desk.said);
        assert_eq!(Trail::read(&RecipeStore::get_vars(&desk.conn, &id)).subs(0), ["answered", "done"]);
        assert_eq!(desk.view(&id).steps[0].path.as_deref(), Some("took else: Researcher (answered) → Notify"), "the arm's steps, as they came to");
    }

    /// The cap on agents at once holds inside an arm too, on the Branch's first entry — before
    /// anything has saved where the Branch stands, which is when the check made before every
    /// step could not see the Agent step in its arm and a fourth agent started (#194 review).
    #[test]
    fn an_agent_step_first_in_an_arm_waits_for_a_place_like_any_other() {
        let steps = [
            agent("researcher", "Find it", "a1"),
            agent("writer", "Draft it", "b1"),
            agent("reviewer", "Check it", "c1"),
            RecipeStep::Branch {
                condition: "x".into(),
                then_steps: vec![],
                else_steps: vec![agent("editor", "Edit it", "d1")],
            },
            format("{{a1}} {{b1}} {{c1}} {{d1}}", "out"),
        ];
        let mut desk = Desk::new();
        for role in ["researcher", "writer", "reviewer", "editor"] {
            desk.hands().says(role, vec![AgentPoll::Working]);
        }
        let id = desk.start_allowed(&steps);
        run(&mut desk, &id, 20);
        assert_eq!(desk.hands().roles(), ["researcher", "writer", "reviewer"], "no fourth agent while three work");
        assert_eq!(desk.status(&id), (RecipeStatus::Waiting, 3), "at the Branch, for a place");

        // One answers: its place goes to the arm's agent, and the recipe runs to the end once
        // every answer is in.
        desk.hands().answer("pi:c-0001", "one");
        desk.tick();
        assert_eq!(desk.hands().roles(), ["researcher", "writer", "reviewer", "editor"], "the arm's agent took the place");
        for (agent, said) in [("pi:c-0002", "two"), ("pi:c-0003", "three"), ("pi:c-0004", "four")] {
            desk.hands().answer(agent, said);
        }
        desk.tick();
        assert_eq!(desk.status(&id).0, RecipeStatus::Done, "{:?}", desk.said);
        assert_eq!(desk.var(&id, "out"), Some(json!("one two three four")));
    }

    /// A Branch inside an arm whose condition is an answer an arm agent has not given yet waits
    /// for it, then takes the arm the answer picks. It used to be entered at once with the
    /// condition unset, commit to `else` for good, and run it while the answer was still coming
    /// (#194 review).
    #[test]
    fn a_nested_branch_on_an_arm_agent_s_answer_waits_for_it_to_choose() {
        let steps = [RecipeStep::Branch {
            condition: "x".into(),
            then_steps: vec![],
            else_steps: vec![
                agent("researcher", "Is it there?", "found"),
                RecipeStep::Branch {
                    condition: "found".into(),
                    then_steps: vec![notify("Went then.")],
                    else_steps: vec![notify("Went else.")],
                },
            ],
        }];
        let mut desk = Desk::new();
        desk.hands().says("researcher", vec![AgentPoll::Working]);
        let id = desk.start_allowed(&steps);
        run(&mut desk, &id, 20);
        assert_eq!(desk.status(&id), (RecipeStatus::Waiting, 0), "at the Branch, for the answer the inner one reads");
        assert!(desk.said.is_empty(), "no arm of the inner Branch ran: {:?}", desk.said);

        desk.hands().answer("pi:c-0001", "Yes, in the attic.");
        desk.tick();
        assert_eq!(desk.status(&id).0, RecipeStatus::Done, "{:?}", desk.said);
        assert!(desk.said.iter().any(|s| s == "Went then."), "{:?}", desk.said);
        assert!(!desk.said.iter().any(|s| s == "Went else."), "{:?}", desk.said);
    }

    /// A Branch whose Agent step fails does not hang the run: the recipe fails with the reason,
    /// the arm step's outcome is recorded, the frames go with the failure, the agent is let go,
    /// and nothing after the Branch runs (#194).
    #[test]
    fn a_branch_whose_agent_step_fails_does_not_hang_the_run() {
        let steps = [
            RecipeStep::Branch {
                condition: "x".into(),
                then_steps: vec![agent("reviewer", "Review it", "review")],
                else_steps: vec![agent("reviewer", "Review it", "review")],
            },
            notify("Shipped."),
        ];

        // The agent fails while it works.
        let mut desk = Desk::new();
        desk.hands().says("reviewer", vec![AgentPoll::Failed("its turn was stopped".into())]);
        let id = desk.start_allowed(&steps);
        run(&mut desk, &id, 20);
        let r = RecipeStore::get(&desk.conn, &id).unwrap();
        assert_eq!(r.status, RecipeStatus::Failed);
        assert_eq!(r.error_message.as_deref(), Some("The Reviewer (pi:c-0001) did not answer: its turn was stopped"));
        assert!(desk.said.iter().all(|s| !s.contains("Shipped")), "nothing after the Branch ran: {:?}", desk.said);
        assert_eq!(desk.var(&id, "_branch"), None, "the frames went with the failure");
        assert_eq!(Trail::read(&RecipeStore::get_vars(&desk.conn, &id)).subs(0), ["failed"]);
        assert_eq!(desk.hands().released.len(), 1, "the agent was let go");
        assert_eq!(step(&mut desk, &id), Advance::Stopped, "a failed recipe stays stopped: the run cannot hang on it");

        // The shell refuses the start itself.
        let mut desk = Desk::new();
        desk.hands().refuse = Some(AgentRefusal::Fail("no mind is attached to the role".into()));
        let id = desk.start_allowed(&steps);
        run(&mut desk, &id, 20);
        let r = RecipeStore::get(&desk.conn, &id).unwrap();
        assert_eq!(r.status, RecipeStatus::Failed);
        assert_eq!(r.error_message.as_deref(), Some("The Reviewer could not be started: no mind is attached to the role"));
        assert_eq!(desk.var(&id, "_branch"), None);
        assert_eq!(Trail::read(&RecipeStore::get_vars(&desk.conn, &id)).subs(0), ["failed"], "the refused step's outcome, in the arm");
        assert_eq!(step(&mut desk, &id), Advance::Stopped);
    }

    /// A start the shell puts off inside a Branch's arm — the person's Allow on a card — waits at
    /// the Branch needing the person, is asked again from where the Branch stands hearing why it
    /// was put off, and the card path is the top-level one: the call carries the Branch's step.
    /// Past its time it fails the recipe saying so, and the frames go with it (#194).
    #[test]
    fn a_start_put_off_inside_a_branch_is_asked_again_from_where_it_stands() {
        let card = "your Allow on the card: Digest recipe → Reviewer";
        let steps = [
            RecipeStep::Branch {
                condition: "x".into(),
                then_steps: vec![],
                else_steps: vec![agent("reviewer", "Review it", "review")],
            },
            notify("Shipped: {{review}}"),
        ];
        let mut desk = Desk::new();
        desk.hands().refuse = Some(AgentRefusal::Ask(card.into()));
        let id = desk.start_allowed(&steps);
        run(&mut desk, &id, 20);
        assert_eq!(desk.status(&id), (RecipeStatus::Waiting, 0), "at the Branch, needing the person");
        assert_eq!(desk.hands().heard_put_off, vec![None], "a first ask has nothing behind it");
        assert_eq!(desk.hands().started.len(), 0, "nothing started while the card is out");
        let v = desk.view(&id);
        assert_eq!(v.steps[0].state, "waiting");
        assert_eq!(v.needs_you.as_deref(), Some(card));

        // Allowed: the start is asked again from where the Branch stands, hearing the reason it
        // waited, and the recipe goes on to its end.
        desk.hands().refuse = None;
        desk.tick();
        assert_eq!(desk.hands().heard_put_off, vec![None, Some(card.to_string())]);
        assert_eq!(desk.status(&id).0, RecipeStatus::Done, "{:?}", desk.said);
        assert_eq!(desk.var(&id, "review"), Some(json!("reviewer answered: Review it")));
        assert!(desk.said.iter().any(|s| s == "Shipped: reviewer answered: Review it"), "{:?}", desk.said);
        assert_eq!(desk.var(&id, "_branch"), None);
        assert_eq!(Trail::read(&RecipeStore::get_vars(&desk.conn, &id)).subs(0), ["answered"]);
        assert_eq!(Trail::read(&RecipeStore::get_vars(&desk.conn, &id)).runs(0), 1, "the put-off round was counted once");

        // Left past its time: the recipe fails saying what it waited for, not hangs.
        let mut desk = Desk::new();
        desk.hands().refuse = Some(AgentRefusal::Ask(card.into()));
        let id = desk.start_allowed(&steps);
        desk.tick();
        desk.clock += PUT_OFF_MOST_SECS as f64 - 1.0;
        desk.tick();
        assert_eq!(desk.status(&id).0, RecipeStatus::Waiting, "still inside its time");
        desk.clock += 1.0;
        desk.tick();
        let r = RecipeStore::get(&desk.conn, &id).unwrap();
        assert_eq!(r.status, RecipeStatus::Failed);
        assert_eq!(
            r.error_message.as_deref(),
            Some("Its Agent step waited 30m and was not started: it was waiting for your Allow on the card: Digest recipe → Reviewer.")
        );
        assert_eq!(desk.var(&id, "_branch"), None);
        assert_eq!(Trail::read(&RecipeStore::get_vars(&desk.conn, &id)).subs(0), ["failed"]);
        assert!(desk.var(&id, "review").is_none(), "no answer, and no empty one");
    }
}
