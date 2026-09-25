//! Recipe Engine — structured automation with tool bypass.
//!
//! Recipes are ordered lists of steps with conditional jumps. Unlike the task queue
//! (open-ended LLM work), recipes have predetermined steps where Tool steps execute
//! directly via the tool registry without LLM involvement.
//!
//! Architecture:
//! - Recipe created from natural language via `create_recipe` tool
//! - Steps stored in normalized SQLite tables (debuggable, queryable)
//! - Tool steps bypass LLM entirely for speed
//! - Think steps call LLM for decision-making
//! - Triggers: manual, time-based (cron), event-based, signal-based
//! - Self-signals via ProcessRecipeStep for continuous execution

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

// ── Step Types ──

/// Comparison operator for Filter steps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FilterOp {
    Equals,
    NotEquals,
    Contains,
    GreaterThan,
    LessThan,
}

/// Aggregation operator for Aggregate steps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AggregateOp {
    Count,
    Sum,
    Min,
    Max,
    Avg,
}

/// A single step in a recipe.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RecipeStep {
    /// Direct tool call — no LLM needed.
    Tool {
        tool_name: String,
        args: serde_json::Value,
        store_as: String,
        #[serde(default)]
        on_error: ErrorAction,
    },
    /// LLM decides what to do with context. Variable references like {{var}} are resolved.
    /// When `fallback_template` is set and no LLM is available, the template is used instead.
    Think {
        prompt: String,
        store_as: String,
        /// Template string with {{var}} substitution, used when LLM is unavailable.
        #[serde(default)]
        fallback_template: Option<String>,
    },
    /// Jump to target_step if condition is true. Evaluated in pure Rust, no LLM.
    JumpIf {
        condition: Condition,
        target_step: usize,
    },
    /// Wait for an external condition. Persists state and pauses recipe.
    WaitFor {
        condition: WaitCondition,
        #[serde(default)]
        timeout_secs: Option<u64>,
    },
    /// Notify user via proactive message. Supports {{var}} templates.
    Notify {
        message: String,
    },
    /// Pause recipe and ask user a question. Stores response in variable.
    /// Recipe transitions to Waiting until user responds.
    AskUser {
        question: String,
        store_as: String,
        /// Optional choices for multiple-choice (UI can render as buttons).
        #[serde(default)]
        choices: Option<Vec<String>>,
    },
    /// LLM synthesis with per-claim citations. Requires source variables from Tool steps.
    /// Outputs structured CitedOutput JSON stored in `store_as`.
    ThinkCited {
        prompt: String,
        store_as: String,
        /// Variable names that serve as sources (from prior Tool steps).
        source_vars: Vec<String>,
    },
    /// Deterministic validation of cited output. Strips uncited claims,
    /// computes evidence strength, produces a cleaned result. No LLM needed.
    Validate {
        /// Variable containing CitedOutput JSON (from ThinkCited).
        input_var: String,
        store_as: String,
    },
    /// Format validated data for user presentation.
    /// Supports multiple output formats.
    Render {
        /// Variable containing validated output.
        input_var: String,
        store_as: String,
        /// Output format.
        #[serde(default)]
        format: RenderFormat,
    },

    // ── Deterministic steps (no LLM needed) ──

    /// Format data using a template string with {{variable}} substitution.
    Format {
        input_vars: Vec<String>,
        template: String,
        store_as: String,
    },
    /// Filter a collection (JSON array in a variable) by a predicate.
    Filter {
        input_var: String,
        field: String,
        op: FilterOp,
        value: String,
        store_as: String,
    },
    /// Sort a collection by a field.
    Sort {
        input_var: String,
        by_field: String,
        #[serde(default)]
        descending: bool,
        store_as: String,
    },
    /// Aggregate a collection (count, sum, min, max, avg).
    Aggregate {
        input_var: String,
        op: AggregateOp,
        field: Option<String>,
        store_as: String,
    },
    /// Extract a value from structured output using a key path or regex.
    Extract {
        input_var: String,
        pattern: String,
        store_as: String,
    },
    /// Branch: if condition is true run then_steps, else run else_steps.
    Branch {
        condition: String,
        then_steps: Vec<RecipeStep>,
        else_steps: Vec<RecipeStep>,
    },

    // ── Formations (design/desk-and-mind-2026-09-23.md, section 6) ──

    /// Hand a turn to a role from the agent catalog and keep its answer in `store_as`.
    ///
    /// The shell starts the role's agent through its own `hand_off` (the executor's
    /// [`AgentHook`](crate::recipe_executor::AgentHook)); the recipe does not wait for it here.
    /// It goes on to the next step, and the first step that reads `store_as` — or the end of the
    /// recipe — waits for the answer. So Agent steps that do not read each other's answers work
    /// at the same time, and a step that needs one waits for it. `role`, `prompt` and `context`
    /// take `{{var}}`s. Only in a run the person allowed to start agents
    /// ([`RecipeStore::allow_agents`]), and only at the top of a recipe, not inside a Branch.
    Agent {
        /// A catalog role's id or name: researcher, planner, coder, reviewer, red-team, writer,
        /// chair, scribe, or the person's own.
        role: String,
        /// What it is to do: its task, after the role's own instructions.
        prompt: String,
        store_as: String,
        /// What it should read first: the answers to weigh, the change to review.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        context: Option<String>,
    },
}

/// Output format for Render steps.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub enum RenderFormat {
    /// Bullet-point summary (default).
    #[default]
    Summary,
    /// Markdown table.
    Table,
    /// Side-by-side comparison grid.
    Comparison,
    /// Numbered card layout.
    Cards,
}

// ── Citation Types ──

/// A single claim with source citations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CitedClaim {
    /// The claim text.
    pub text: String,
    /// Source variable names that back this claim.
    #[serde(default)]
    pub sources: Vec<String>,
    /// Confidence: "high", "medium", "low", "uncited".
    #[serde(default = "default_confidence")]
    pub confidence: String,
}

fn default_confidence() -> String { "uncited".to_string() }

/// Structured output from ThinkCited steps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CitedOutput {
    /// Section title.
    pub title: String,
    /// Claims with source citations.
    pub claims: Vec<CitedClaim>,
    /// Overall evidence strength.
    #[serde(default)]
    pub evidence_status: EvidenceStatus,
}

/// How well-supported the evidence is.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub enum EvidenceStatus {
    /// 3+ independent sources confirm.
    Strong,
    /// 2 sources or 1 high-quality source.
    Moderate,
    /// 1 source only.
    Thin,
    /// Sources disagree.
    Conflicting,
    /// Not enough data.
    #[default]
    Insufficient,
}

/// What to do when a Tool step fails.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action")]
pub enum ErrorAction {
    Fail,
    Skip,
    Retry { max: u8 },
    JumpTo { step: usize },
    /// Ask the LLM to diagnose the failure and replan remaining steps.
    Replan,
}

impl Default for ErrorAction {
    fn default() -> Self { Self::Fail }
}

/// Conditions evaluable in pure Rust — no LLM cost.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op")]
pub enum Condition {
    VarEquals { var: String, value: serde_json::Value },
    VarContains { var: String, substring: String },
    VarExists { var: String },
    VarGt { var: String, threshold: f64 },
    VarEmpty { var: String },
    TimeAfter { hour: u8, minute: u8 },
    TimeBefore { hour: u8, minute: u8 },
    Not { inner: Box<Condition> },
    And { conditions: Vec<Condition> },
    Or { conditions: Vec<Condition> },
}

impl Condition {
    /// Evaluate the condition against recipe variables. Pure Rust, zero LLM cost.
    pub fn evaluate(&self, vars: &std::collections::HashMap<String, serde_json::Value>) -> bool {
        match self {
            Self::VarEquals { var, value } => {
                vars.get(var).map_or(false, |v| v == value)
            }
            Self::VarContains { var, substring } => {
                vars.get(var)
                    .and_then(|v| v.as_str())
                    .map_or(false, |s| s.contains(substring.as_str()))
            }
            Self::VarExists { var } => vars.contains_key(var),
            Self::VarEmpty { var } => {
                vars.get(var).map_or(true, |v| {
                    v.is_null() || v.as_str().map_or(false, |s| s.is_empty())
                        || v.as_array().map_or(false, |a| a.is_empty())
                })
            }
            Self::VarGt { var, threshold } => {
                vars.get(var)
                    .and_then(|v| v.as_f64())
                    .map_or(false, |n| n > *threshold)
            }
            Self::TimeAfter { hour, minute } => {
                let now = chrono_now();
                now.0 > *hour || (now.0 == *hour && now.1 >= *minute)
            }
            Self::TimeBefore { hour, minute } => {
                let now = chrono_now();
                now.0 < *hour || (now.0 == *hour && now.1 < *minute)
            }
            Self::Not { inner } => !inner.evaluate(vars),
            Self::And { conditions } => conditions.iter().all(|c| c.evaluate(vars)),
            Self::Or { conditions } => conditions.iter().any(|c| c.evaluate(vars)),
        }
    }
}

/// Conditions for WaitFor steps.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WaitCondition {
    /// Wait until a specific time. Format: seconds from now.
    Duration { seconds: u64 },
    /// Wait until a cron-like time expression fires.
    Time { hour: u8, minute: u8 },
}

// ── Trigger Types ──

/// What starts a recipe.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum TriggerType {
    /// User manually runs it.
    Manual,
    /// Cron-like schedule.
    Cron { expression: String },
    /// Fired by an event (email:new, file:created, etc.)
    Event { event_type: String, filter: Option<serde_json::Value> },
    /// Fired when another recipe completes.
    RecipeComplete { recipe_id: String },
}

// ── Recipe Instance (runtime state) ──

/// Status of a recipe execution.
#[derive(Debug, Clone, PartialEq)]
pub enum RecipeStatus {
    /// Created but not started.
    Pending,
    /// Currently executing steps.
    Running,
    /// Paused waiting for a condition.
    Waiting,
    /// Held by a person (`RecipeStore::pause`). The executor does not pick it up — `get_resumable`
    /// takes `running` and `get_expired_waiting` takes `waiting` — until `RecipeStore::resume`.
    Paused,
    /// Successfully completed all steps.
    Done,
    /// Failed with an error.
    Failed,
}

impl RecipeStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Waiting => "waiting",
            Self::Paused => "paused",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }
    pub fn from_str(s: &str) -> Self {
        match s {
            "running" => Self::Running,
            "waiting" => Self::Waiting,
            "paused" => Self::Paused,
            "done" => Self::Done,
            "failed" => Self::Failed,
            _ => Self::Pending,
        }
    }
}

/// The error a cancelled recipe carries. A cancel is stored as a failure with this text — the
/// way the chat's "cancel" has always stored it — so every reader that knows `failed` keeps
/// working, and the Recipes screen can still tell a person's decision from a fault.
pub const CANCELLED: &str = "Cancelled by user";

/// The variable `RecipeStore::pause` keeps the status it paused from in, so `resume` puts the
/// recipe back exactly where it was: running, or still waiting on its timer or its question.
pub const PAUSED_FROM_VAR: &str = "_paused_from";

/// A recipe definition + runtime state.
#[derive(Debug, Clone)]
pub struct Recipe {
    pub id: String,
    pub name: String,
    pub description: String,
    pub status: RecipeStatus,
    pub current_step: usize,
    pub created_at: f64,
    pub updated_at: f64,
    pub enabled: bool,
    pub error_message: Option<String>,
}

/// A loaded step from SQLite with its execution result.
#[derive(Debug, Clone)]
pub struct StoredStep {
    pub step_index: usize,
    pub step: RecipeStep,
    pub status: String,    // "pending", "done", "failed", "skipped"
    pub result: Option<String>,
}

/// What `get_steps` stands in for a step whose JSON it could not read: a Notify with this prefix.
pub const UNREADABLE: &str = "PARSE ERROR: ";

/// Where a waiting recipe waits ([`WaitRecord`]), kept while it waits.
pub const WAIT_VAR: &str = "_wait";

/// The Branch arms a recipe is inside, outermost first, kept while it runs them.
pub const BRANCH_VAR: &str = "_branch";

/// What each step did, run by run ([`Trail`]).
pub const TRAIL_VAR: &str = "_trail";

/// How many steps a recipe has run since it last waited — what the executor's step budget counts.
pub const SINCE_WAIT_VAR: &str = "_since_wait";

/// A recipe's own step budget, when it sets one; `recipe_executor::STEP_BUDGET` otherwise.
pub const STEP_BUDGET_VAR: &str = "_step_budget";

/// The agents a recipe's Agent steps handed work to ([`AgentRun`]), keyed by the step's index:
/// the ones still working, and — so the Recipes screen can say who answered — the last one each
/// step had.
pub const AGENTS_VAR: &str = "_agents";

/// One Agent step's agent, as the executor keeps it in [`AGENTS_VAR`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentRun {
    /// The role's id in the catalog (`chair`), as the shell resolved it.
    pub role: String,
    /// Its name as a person reads it (`Chair`).
    pub role_name: String,
    /// The mind it runs on (`deepseek`).
    pub mind: String,
    /// The agent, `<mind>:<conversation>`, as the Agents screen lists it.
    pub agent: String,
    /// Where its answer goes.
    pub store_as: String,
    /// When it was handed the work, and when the recipe stops waiting for it (its role's minutes,
    /// and a little over).
    pub since: f64,
    pub until: f64,
    /// working | answered | failed | released.
    pub state: String,
    /// While it works: it is waiting on the person in its own pane — an approval card, a command
    /// at a prompt — and what for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub needs_you: Option<String>,
}

impl AgentRun {
    pub const WORKING: &'static str = "working";
    pub const ANSWERED: &'static str = "answered";
    pub const FAILED: &'static str = "failed";
    /// Let go before it answered: the recipe failed, or was cancelled.
    pub const RELEASED: &'static str = "released";

    pub fn working(&self) -> bool {
        self.state == Self::WORKING
    }

    /// "Chair · deepseek": what the step's stage is called once it has an agent.
    pub fn stage(&self) -> String {
        format!("{} · {}", self.role_name, self.mind)
    }
}

/// Every Agent step's agent a recipe's variables hold, by step index. An entry that does not
/// read as one is left out.
pub fn agent_runs(vars: &std::collections::HashMap<String, serde_json::Value>) -> std::collections::BTreeMap<usize, AgentRun> {
    vars.get(AGENTS_VAR)
        .and_then(|v| v.as_object())
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| Some((k.parse::<usize>().ok()?, serde_json::from_value::<AgentRun>(v.clone()).ok()?)))
                .collect()
        })
        .unwrap_or_default()
}

/// Write them back.
pub fn save_agent_runs(conn: &Connection, recipe_id: &str, runs: &std::collections::BTreeMap<usize, AgentRun>) {
    let map: serde_json::Map<String, serde_json::Value> =
        runs.iter().map(|(k, v)| (k.to_string(), serde_json::to_value(v).unwrap_or_default())).collect();
    RecipeStore::set_var(conn, recipe_id, AGENTS_VAR, &serde_json::Value::Object(map));
}

/// Why the step at a recipe's pointer cannot run yet, for its agents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentBlock {
    /// It reads what these steps' agents have not answered yet — or it is the end, and they are
    /// still working. Step indexes.
    Answers(Vec<usize>),
    /// It is an Agent step, and the recipe already has as many agents working as it may
    /// (`recipe_executor::AGENTS_AT_ONCE`).
    Place,
}

/// Whether the step at `at` — `steps.len()` for the end — must wait for the recipe's agents:
/// it reads an answer still coming (a `{{name}}`, an input variable, a JumpIf's or a Branch's
/// condition, anything its arms read), or it is the end with answers still out, or it is an Agent
/// step whose own last agent is still working or for which there is no place. None: it may run.
pub fn blocked_on_agents(
    steps: &[StoredStep],
    at: usize,
    vars: &std::collections::HashMap<String, serde_json::Value>,
) -> Option<AgentBlock> {
    let runs = agent_runs(vars);
    let working: Vec<(usize, &AgentRun)> = runs.iter().filter(|(_, r)| r.working()).map(|(k, r)| (*k, r)).collect();
    if working.is_empty() {
        return None;
    }
    let Some(step) = steps.iter().find(|s| s.step_index == at).map(|s| &s.step) else {
        // The end: every answer is in before the recipe is done.
        return Some(AgentBlock::Answers(working.iter().map(|(k, _)| *k).collect()));
    };
    let is_agent = matches!(step, RecipeStep::Agent { .. });
    if is_agent && working.iter().any(|(k, _)| *k == at) {
        // Round again before its last round's agent has answered.
        return Some(AgentBlock::Answers(vec![at]));
    }
    let reads = crate::recipe_view::reads(step);
    let needed: Vec<usize> = working.iter().filter(|(_, r)| reads.contains(&r.store_as)).map(|(k, _)| *k).collect();
    if !needed.is_empty() {
        return Some(AgentBlock::Answers(needed));
    }
    if is_agent && working.len() >= crate::recipe_executor::AGENTS_AT_ONCE {
        return Some(AgentBlock::Place);
    }
    None
}

/// A catalog role's id as a person reads it, before the shell has said its name: `red-team` →
/// "Red team". The shipped roles' names are exactly this.
pub fn role_display(role: &str) -> String {
    let words = role.trim().replace(['-', '_'], " ");
    let mut chars = words.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Whether a recipe's steps hand work to agents: an Agent step anywhere, a Branch's arms included.
pub fn hands_off(steps: &[RecipeStep]) -> bool {
    steps.iter().any(step_hands_off)
}

/// Whether one step hands work to an agent, or holds one that does.
pub fn step_hands_off(step: &RecipeStep) -> bool {
    match step {
        RecipeStep::Agent { .. } => true,
        RecipeStep::Branch { then_steps, else_steps, .. } => hands_off(then_steps) || hands_off(else_steps),
        _ => false,
    }
}

/// Who let a run hand work to agents ([`RecipeStore::allow_agents`]), and to which roles.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Leave {
    /// In words, for the record: "the person, from the Recipes screen".
    pub by: String,
    /// The agent that asked for the run (`pi:c-7f3a91`), when an agent did: the run's agents are
    /// its children, held to its rules. None when the person started it.
    pub agent: Option<String>,
    /// The roles agreed to, as the run names them, each with the digest of its definition when
    /// the person started the run. A role whose definition has changed since — a file in
    /// ~/.config/yantrik/agents replaced it, with another mind, brief or reach — or one the run
    /// did not name then is not started on this leave.
    pub roles: std::collections::BTreeMap<String, String>,
}

impl Leave {
    pub fn new(by: impl Into<String>, agent: Option<String>) -> Leave {
        Leave { by: by.into(), agent, roles: Default::default() }
    }
}

/// The roles a recipe's Agent steps name, with `vars` — a run's inputs, with the template's
/// defaults for what they do not give — filled in: what a person starting it agrees to.
pub fn roles_named(steps: &[RecipeStep], vars: &std::collections::HashMap<String, serde_json::Value>) -> Vec<String> {
    fn walk(steps: &[RecipeStep], vars: &std::collections::HashMap<String, serde_json::Value>, out: &mut Vec<String>) {
        for step in steps {
            match step {
                RecipeStep::Agent { role, .. } => {
                    let named = resolve_vars(role, vars).trim().to_string();
                    if !out.contains(&named) {
                        out.push(named);
                    }
                }
                RecipeStep::Branch { then_steps, else_steps, .. } => {
                    walk(then_steps, vars, out);
                    walk(else_steps, vars, out);
                }
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(steps, vars, &mut out);
    out
}

/// Where a recipe waits, as the executor writes it when a WaitFor or an AskUser stops it: the
/// top-level step it waits at — the wait itself, or the Branch holding it — the arms and indexes
/// down to the waiting step when it is inside a Branch, when it began, and for a timer when it
/// wakes. The time is absolute, so a pause does not restart a timer and a restart keeps it.
///
/// `agents`: it waits on its agents rather than on a wait step — the step at `step` reads an
/// answer an Agent step's agent has not given yet, or it is an Agent step and the recipe already
/// has as many agents working as it may, or it is the end and some are still working. The
/// pointer stays at that step, and the clock looks every tick whether the answers have come.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WaitRecord {
    pub step: usize,
    #[serde(default)]
    pub inner: Vec<(String, usize)>,
    pub since: f64,
    #[serde(default)]
    pub until: Option<f64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub agents: bool,
    /// An Agent step whose start the shell put off (`AgentRefusal::Wait` or `Ask`), and why: the
    /// executor asks again each tick, from `since`, and gives up after
    /// `recipe_executor::PUT_OFF_MOST_SECS`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub put_off: Option<String>,
}

/// What a waiting recipe waits on, read from what the store holds for it ([`waited_on`]).
#[derive(Debug, Clone)]
pub struct Waited {
    /// The top-level step it waits at: the WaitFor or AskUser, or the Branch that holds it.
    pub step: usize,
    /// Inside a Branch: the arm and index at each level, down to the waiting step.
    pub inner: Vec<(String, usize)>,
    /// The WaitFor or AskUser it waits on. None: `waiting` with no wait behind it.
    pub on: Option<RecipeStep>,
    /// When it began to wait.
    pub since: f64,
    /// When a timer wakes, unix seconds.
    pub until: Option<f64>,
    /// It waits on its agents' answers ([`WaitRecord::agents`]).
    pub agents: bool,
    /// An Agent step's start was put off, and why ([`WaitRecord::put_off`]): the person can
    /// resolve it — a card to answer, a place to free.
    pub put_off: Option<String>,
}

impl Waited {
    /// Whether the wait is over at `now`. A question is over only when it is answered — the
    /// answer sets the recipe running, so a recipe still waiting on one is never over. A wait on
    /// agents is for the executor to decide each tick, by asking the shell how they are doing.
    pub fn is_over(&self, now: f64) -> bool {
        if self.agents {
            return true;
        }
        match &self.on {
            Some(RecipeStep::AskUser { .. }) => false,
            Some(RecipeStep::WaitFor { .. }) => self.until.map_or(true, |u| now >= u),
            _ => true,
        }
    }

    /// The variable the answer goes in, when what it waits on is a person's answer.
    pub fn store_as(&self) -> Option<&str> {
        match &self.on {
            Some(RecipeStep::AskUser { store_as, .. }) => Some(store_as),
            _ => None,
        }
    }
}

/// What a recipe waits on, from its row, its steps and its variables. Call it for a recipe that
/// is waiting (or paused while waiting); it does not look at the status.
///
/// The executor's `_wait` record says where, including inside a Branch. A recipe that began to
/// wait before there was one is read the old way: the step just behind the pointer, with a timer
/// counted from when the row last changed.
pub fn waited_on(
    recipe: &Recipe,
    steps: &[StoredStep],
    vars: &std::collections::HashMap<String, serde_json::Value>,
) -> Option<Waited> {
    let is_wait = |s: &&RecipeStep| matches!(s, RecipeStep::WaitFor { .. } | RecipeStep::AskUser { .. });
    if let Some(record) = vars.get(WAIT_VAR).and_then(|v| serde_json::from_value::<WaitRecord>(v.clone()).ok()) {
        let mut at = steps.iter().find(|s| s.step_index == record.step).map(|s| &s.step);
        for (arm, k) in &record.inner {
            at = match at {
                Some(RecipeStep::Branch { then_steps, else_steps, .. }) => {
                    if arm == "then" { then_steps.get(*k) } else { else_steps.get(*k) }
                }
                _ => None,
            };
        }
        return Some(Waited {
            step: record.step,
            inner: record.inner,
            on: at.filter(is_wait).cloned(),
            since: record.since,
            until: record.until,
            agents: record.agents,
            put_off: record.put_off,
        });
    }
    let before = recipe.current_step.checked_sub(1);
    let on = before
        .and_then(|i| steps.iter().find(|s| s.step_index == i))
        .map(|s| &s.step)
        .filter(is_wait)
        .cloned();
    let until = match &on {
        Some(RecipeStep::WaitFor { condition, timeout_secs }) => {
            Some(wakes_at(condition, *timeout_secs, recipe.updated_at).unwrap_or(recipe.updated_at))
        }
        _ => None,
    };
    Some(Waited { step: before.unwrap_or(0), inner: Vec::new(), on, since: recipe.updated_at, until, agents: false, put_off: None })
}

/// When a WaitFor that begins at `from` wakes: after its duration, or at the next time of day
/// it names — read on a clock `offset_secs` east of UTC, the machine's own — and no later than
/// its timeout. None when it has nothing to wait for: a zero duration, or the very minute it
/// names.
///
/// A time of day already gone by today is tomorrow's. It used to count as met, so "wait until
/// 09:00" set at 10:00 went on at once, and a daily loop around it spun.
///
/// The offset is the one in force at `from`: a wait that crosses a daylight-saving switch wakes
/// an hour off the named time, which is the price of not carrying a zone database here.
pub fn wakes_at_in(condition: &WaitCondition, timeout_secs: Option<u64>, from: f64, offset_secs: i64) -> Option<f64> {
    let due = match condition {
        WaitCondition::Duration { seconds: 0 } => return None,
        WaitCondition::Duration { seconds } => from + *seconds as f64,
        WaitCondition::Time { hour, minute } => {
            // A time of day is read on the machine's clock, so the instant is shifted into
            // local seconds, the named time is found there, and the answer is shifted back.
            let off = offset_secs as f64;
            let local = from + off;
            let today = (local / 86_400.0).floor() * 86_400.0 + f64::from(*hour) * 3600.0 + f64::from(*minute) * 60.0;
            if local < today {
                today - off
            } else if local < today + 60.0 {
                return None;
            } else {
                today + 86_400.0 - off
            }
        }
    };
    Some(match timeout_secs {
        Some(t) => due.min(from + t as f64),
        None => due,
    })
}

/// When a WaitFor that begins at `from` wakes, on this machine's clock ([`wakes_at_in`]).
///
/// It used to be UTC, as every clock in the engine was: a recipe told to wait "until 09:00"
/// waited until 09:00 UTC, and a person who meant their own morning got somebody else's (#187).
pub fn wakes_at(condition: &WaitCondition, timeout_secs: Option<u64>, from: f64) -> Option<f64> {
    wakes_at_in(condition, timeout_secs, from, local_offset_secs(from))
}

/// A unix time as a clock `offset_secs` east of UTC reads it: "08:15".
pub fn clock_text_at(ts: f64, offset_secs: i64) -> String {
    let of_day = ((ts.floor() as i64) + offset_secs as i64).rem_euclid(86_400);
    format!("{:02}:{:02}", of_day / 3600, (of_day % 3600) / 60)
}

/// A unix time as this machine's clock reads it: "08:15". It used to say "08:15 UTC" — a time
/// nobody east or west of Greenwich had set, shown as though they had (#187).
pub fn clock_text(ts: f64) -> String {
    clock_text_at(ts, local_offset_secs(ts))
}

/// How far east of UTC this machine's clock runs at `ts`, in seconds (west is negative) — the
/// same convention `tm_gmtoff` and the shell's clock object use. An instant the calendar cannot
/// read falls back to UTC rather than refusing the time.
pub fn local_offset_secs(ts: f64) -> i64 {
    chrono::DateTime::from_timestamp(ts as i64, 0)
        .map(|dt| dt.with_timezone(&chrono::Local).offset().local_minus_utc() as i64)
        .unwrap_or(0)
}

/// Whether a cron trigger is due: the first occurrence of `expression` strictly after `floor`
/// has arrived at or before `now`.
///
/// A recipe's schedule is read on the machine's clock — "0 9 * * *" means nine in the morning
/// where the machine stands — while `cron_mini` decomposes timestamps as UTC, shared as it is
/// with the scheduler, whose schedules stay UTC. So both ends are shifted by the local offset
/// going in and the answer is shifted back coming out. An expression that does not parse is
/// never due.
pub fn cron_due(expression: &str, floor: f64, now: f64, offset_secs: i64) -> bool {
    let off = offset_secs as f64;
    crate::cron_mini::next_cron(expression, floor + off).is_some_and(|next| next - off <= now)
}

/// What each step of a recipe did, run by run — `_trail`, keyed by the step's index: how many
/// times it ran (`runs`), which way a JumpIf or a Branch went (`went`), how many times a JumpIf
/// jumped (`jumps`), what each step of a Branch's arm came to (`subs`), and where a jump out of
/// an arm went (`left`). The Recipes screen reads it to say which way a Branch went and how many
/// rounds a loop has gone; the executor, whether a question is being asked for the first time.
#[derive(Debug, Clone, Default)]
pub struct Trail(serde_json::Map<String, serde_json::Value>);

impl Trail {
    pub fn read(vars: &std::collections::HashMap<String, serde_json::Value>) -> Self {
        Self(vars.get(TRAIL_VAR).and_then(|v| v.as_object()).cloned().unwrap_or_default())
    }

    pub fn save_to(&self, conn: &Connection, recipe_id: &str) {
        RecipeStore::set_var(conn, recipe_id, TRAIL_VAR, &serde_json::Value::Object(self.0.clone()));
    }

    fn get(&self, step: usize) -> Option<&serde_json::Map<String, serde_json::Value>> {
        self.0.get(&step.to_string()).and_then(|e| e.as_object())
    }

    fn entry(&mut self, step: usize) -> &mut serde_json::Map<String, serde_json::Value> {
        let e = self.0.entry(step.to_string()).or_insert_with(|| serde_json::json!({}));
        if !e.is_object() {
            *e = serde_json::json!({});
        }
        e.as_object_mut().expect("an object, just made one")
    }

    fn count(&self, step: usize, key: &str) -> u64 {
        self.get(step).and_then(|e| e.get(key)).and_then(|v| v.as_u64()).unwrap_or(0)
    }

    fn bump(&mut self, step: usize, key: &str) {
        let n = self.count(step, key) + 1;
        self.entry(step).insert(key.into(), n.into());
    }

    /// How many times the step has run.
    pub fn runs(&self, step: usize) -> u64 {
        self.count(step, "runs")
    }

    /// How many times a JumpIf has jumped.
    pub fn jumps(&self, step: usize) -> u64 {
        self.count(step, "jumps")
    }

    /// Which way a JumpIf (jumped, continued) or a Branch (then, else) went last.
    pub fn way(&self, step: usize) -> Option<&str> {
        self.get(step).and_then(|e| e.get("went")).and_then(|v| v.as_str())
    }

    /// What each step of a Branch's arm came to, on its last round: done, skipped, failed,
    /// waiting, waited, answered.
    pub fn subs(&self, step: usize) -> Vec<String> {
        self.get(step)
            .and_then(|e| e.get("subs"))
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect())
            .unwrap_or_default()
    }

    /// Where a jump out of a Branch's arm went, 0-based.
    pub fn left(&self, step: usize) -> Option<usize> {
        self.get(step).and_then(|e| e.get("left")).and_then(|v| v.as_u64()).map(|n| n as usize)
    }

    pub fn ran(&mut self, step: usize) {
        self.bump(step, "runs");
    }

    pub fn went(&mut self, step: usize, way: &str) {
        self.entry(step).insert("went".into(), way.into());
    }

    pub fn jumped(&mut self, step: usize) {
        self.bump(step, "jumps");
        self.went(step, "jumped");
    }

    /// A Branch begins a round: which arm, and nothing of it run yet.
    pub fn enter_branch(&mut self, step: usize, arm: &str) {
        self.ran(step);
        let e = self.entry(step);
        e.insert("went".into(), arm.into());
        e.insert("subs".into(), serde_json::json!([]));
        e.remove("left");
    }

    /// One more step of the arm came to `outcome`.
    pub fn sub(&mut self, step: usize, outcome: &str) {
        let e = self.entry(step);
        match e.get_mut("subs").and_then(|v| v.as_array_mut()) {
            Some(subs) => subs.push(outcome.into()),
            None => {
                e.insert("subs".into(), serde_json::json!([outcome]));
            }
        }
    }

    /// A jump out of the arm, to `target`.
    pub fn leave(&mut self, step: usize, target: usize) {
        self.entry(step).insert("left".into(), target.into());
    }

    /// The last step of the arm, `from` → `to`: a wait that is over, a question answered.
    pub fn settle_last(&mut self, step: usize, from: &str, to: &str) {
        if let Some(last) = self.entry(step).get_mut("subs").and_then(|v| v.as_array_mut()).and_then(|a| a.last_mut()) {
            if last.as_str() == Some(from) {
                *last = to.into();
            }
        }
    }

    /// The same, straight in the store.
    pub fn note(conn: &Connection, recipe_id: &str, step: usize, from: &str, to: &str) {
        let mut trail = Self::read(&RecipeStore::get_vars(conn, recipe_id));
        trail.settle_last(step, from, to);
        trail.save_to(conn, recipe_id);
    }

    /// The JumpIf that has gone back the most, and how many times.
    pub fn most_looped(&self) -> Option<(usize, u64)> {
        self.0
            .keys()
            .filter_map(|k| k.parse::<usize>().ok())
            .map(|i| (i, self.jumps(i)))
            .filter(|(_, n)| *n > 0)
            .max_by_key(|(i, n)| (*n, std::cmp::Reverse(*i)))
    }
}

// ── SQLite Persistence ──

pub struct RecipeStore;

impl RecipeStore {
    /// Create all recipe tables.
    pub fn ensure_tables(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS recipes (
                id          TEXT PRIMARY KEY,
                name        TEXT NOT NULL,
                description TEXT DEFAULT '',
                status      TEXT NOT NULL DEFAULT 'pending',
                current_step INTEGER DEFAULT 0,
                enabled     INTEGER DEFAULT 1,
                error_msg   TEXT,
                created_at  REAL NOT NULL,
                updated_at  REAL NOT NULL
            );

            CREATE TABLE IF NOT EXISTS recipe_steps (
                recipe_id   TEXT NOT NULL REFERENCES recipes(id),
                step_index  INTEGER NOT NULL,
                step_json   TEXT NOT NULL,
                status      TEXT DEFAULT 'pending',
                result      TEXT,
                PRIMARY KEY (recipe_id, step_index)
            );

            CREATE TABLE IF NOT EXISTS recipe_vars (
                recipe_id   TEXT NOT NULL REFERENCES recipes(id),
                key         TEXT NOT NULL,
                value       TEXT NOT NULL,
                PRIMARY KEY (recipe_id, key)
            );

            CREATE TABLE IF NOT EXISTS recipe_triggers (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                recipe_id   TEXT NOT NULL REFERENCES recipes(id),
                trigger_json TEXT NOT NULL,
                enabled     INTEGER DEFAULT 1,
                last_fired  REAL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_recipes_status ON recipes(status);
            CREATE INDEX IF NOT EXISTS idx_triggers_enabled ON recipe_triggers(enabled);

            CREATE TABLE IF NOT EXISTS recipe_agent_leave (
                recipe_id   TEXT PRIMARY KEY REFERENCES recipes(id),
                by_whom     TEXT NOT NULL,
                agent       TEXT,
                roles       TEXT NOT NULL DEFAULT '{}',
                at          REAL NOT NULL
            );",
        )
        .expect("failed to create recipe tables");
    }

    /// Let one run hand work to agents: its Agent steps start catalog roles through the shell.
    ///
    /// Kept in a table of its own, not in the run's variables, because a recipe's own steps and a
    /// caller's `variables` write those: a leave anything could set would be no leave. Only the
    /// doors that asked first write it — the Recipes screen's Start (the person's own press) and
    /// the shell's `run_recipe`, which is graded sensitive and so asks the person in `ask` mode.
    pub fn allow_agents(conn: &Connection, recipe_id: &str, leave: &Leave) {
        let roles = serde_json::to_string(&leave.roles).unwrap_or_else(|_| "{}".into());
        conn.execute(
            "INSERT OR REPLACE INTO recipe_agent_leave (recipe_id, by_whom, agent, roles, at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![recipe_id, leave.by, leave.agent, roles, now_ts()],
        )
        .ok();
    }

    /// Who let this run hand work to agents, if anyone did.
    pub fn agents_allowed(conn: &Connection, recipe_id: &str) -> Option<Leave> {
        conn.query_row(
            "SELECT by_whom, agent, roles FROM recipe_agent_leave WHERE recipe_id = ?1",
            params![recipe_id],
            |row| {
                let roles: String = row.get(2)?;
                Ok(Leave { by: row.get(0)?, agent: row.get(1)?, roles: serde_json::from_str(&roles).unwrap_or_default() })
            },
        )
        .ok()
    }

    /// Finished recipes that still name an agent as working: failed or cancelled from a door that
    /// does not run the executor (the chat's "cancel", `cancel_recipe`). The clock lets their
    /// agents go ([`crate::recipe_executor::step`]).
    pub fn finished_with_agents_working(conn: &Connection) -> Vec<String> {
        let mut stmt = match conn.prepare(
            "SELECT r.id FROM recipes r JOIN recipe_vars v ON v.recipe_id = r.id AND v.key = ?1
             WHERE r.status IN ('failed', 'done') AND v.value LIKE '%\"state\":\"working\"%'",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        stmt.query_map(params![AGENTS_VAR], |row| row.get::<_, String>(0))
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default()
    }

    /// Create a new recipe with steps and optional trigger.
    pub fn create(
        conn: &Connection,
        name: &str,
        description: &str,
        steps: &[RecipeStep],
        trigger: Option<&TriggerType>,
    ) -> String {
        // The UUID's last 12 hex digits: its counter's low bits and its random tail. The first 8
        // were the millisecond clock's top bits, the same for ~65 s, so a second recipe made in
        // that minute collided with the first on the primary key and panicked (#173).
        let id = format!("rcp_{}", &uuid7::uuid7().to_string()[24..]);
        let now = now_ts();

        conn.execute(
            "INSERT INTO recipes (id, name, description, status, created_at, updated_at)
             VALUES (?1, ?2, ?3, 'pending', ?4, ?4)",
            params![id, name, description, now],
        )
        .expect("insert recipe");

        for (i, step) in steps.iter().enumerate() {
            let step_json = serde_json::to_string(step).unwrap_or_default();
            conn.execute(
                "INSERT INTO recipe_steps (recipe_id, step_index, step_json)
                 VALUES (?1, ?2, ?3)",
                params![id, i as i64, step_json],
            )
            .expect("insert recipe step");
        }

        if let Some(trigger) = trigger {
            let trigger_json = serde_json::to_string(trigger).unwrap_or_default();
            conn.execute(
                "INSERT INTO recipe_triggers (recipe_id, trigger_json) VALUES (?1, ?2)",
                params![id, trigger_json],
            )
            .expect("insert recipe trigger");
        }

        tracing::info!(recipe_id = %id, name = %name, steps = steps.len(), "Recipe created");
        id
    }

    /// Register or update a built-in recipe with a fixed ID. Idempotent — called at every start.
    ///
    /// A built-in is a template: it never runs on its own id (`start_run` makes each run a recipe
    /// of its own). This used to delete and re-insert every built-in's step rows at every start, so
    /// a built-in that had run in place — as `run_recipe` used to run it — lost its steps' results
    /// and states at the next start. Now a start that changes nothing touches nothing; a built-in
    /// with a run on its own id has the run moved to an id of its own, record and all, and is
    /// made a clean template again; and only a changed definition is rewritten.
    pub fn ensure_builtin(
        conn: &Connection,
        id: &str,
        name: &str,
        description: &str,
        steps: &[RecipeStep],
    ) {
        if let Some(existing) = Self::get(conn, id) {
            let stored = Self::get_steps(conn, id);
            let ran = existing.status != RecipeStatus::Pending || stored.iter().any(|s| s.status != "pending");
            if ran {
                if let Some(kept) = Self::copy(conn, id, true) {
                    tracing::info!(recipe_id = %id, run = %kept, "Built-in's run kept as a recipe of its own");
                }
                conn.execute(
                    "UPDATE recipes SET status = 'pending', current_step = 0, error_msg = NULL WHERE id = ?1",
                    params![id],
                )
                .ok();
                conn.execute("DELETE FROM recipe_vars WHERE recipe_id = ?1", params![id]).ok();
                Self::reset_steps(conn, id, 0, usize::MAX);
            }
            let wanted: Vec<String> = steps.iter().map(|s| serde_json::to_string(s).unwrap_or_default()).collect();
            let have: Vec<String> = stored.iter().map(|s| serde_json::to_string(&s.step).unwrap_or_default()).collect();
            if wanted != have {
                // The definition changed across versions: the template takes the new steps.
                conn.execute("DELETE FROM recipe_steps WHERE recipe_id = ?1", params![id]).ok();
                for (i, step_json) in wanted.iter().enumerate() {
                    conn.execute(
                        "INSERT INTO recipe_steps (recipe_id, step_index, step_json) VALUES (?1, ?2, ?3)",
                        params![id, i as i64, step_json],
                    )
                    .ok();
                }
            }
            return;
        }
        let now = now_ts();
        conn.execute(
            "INSERT INTO recipes (id, name, description, status, created_at, updated_at)
             VALUES (?1, ?2, ?3, 'pending', ?4, ?4)",
            params![id, name, description, now],
        ).ok();
        for (i, step) in steps.iter().enumerate() {
            let step_json = serde_json::to_string(step).unwrap_or_default();
            conn.execute(
                "INSERT INTO recipe_steps (recipe_id, step_index, step_json) VALUES (?1, ?2, ?3)",
                params![id, i as i64, step_json],
            ).ok();
        }
        tracing::info!(recipe_id = %id, name = %name, steps = steps.len(), "Built-in recipe registered");
    }

    /// Get a recipe by ID.
    pub fn get(conn: &Connection, recipe_id: &str) -> Option<Recipe> {
        conn.query_row(
            "SELECT id, name, description, status, current_step, enabled, error_msg, created_at, updated_at
             FROM recipes WHERE id = ?1",
            params![recipe_id],
            |row| {
                let enabled_i: i32 = row.get(5)?;
                Ok(Recipe {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    description: row.get(2)?,
                    status: RecipeStatus::from_str(&row.get::<_, String>(3)?),
                    current_step: row.get::<_, i64>(4)? as usize,
                    enabled: enabled_i != 0,
                    error_message: row.get(6)?,
                    created_at: row.get(7)?,
                    updated_at: row.get(8)?,
                })
            },
        )
        .ok()
    }

    /// Get a recipe by name (case-insensitive).
    pub fn find_by_name(conn: &Connection, name: &str) -> Option<Recipe> {
        conn.query_row(
            "SELECT id, name, description, status, current_step, enabled, error_msg, created_at, updated_at
             FROM recipes WHERE LOWER(name) = LOWER(?1)",
            params![name],
            |row| {
                let enabled_i: i32 = row.get(5)?;
                Ok(Recipe {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    description: row.get(2)?,
                    status: RecipeStatus::from_str(&row.get::<_, String>(3)?),
                    current_step: row.get::<_, i64>(4)? as usize,
                    enabled: enabled_i != 0,
                    error_message: row.get(6)?,
                    created_at: row.get(7)?,
                    updated_at: row.get(8)?,
                })
            },
        )
        .ok()
    }

    /// Load all steps for a recipe.
    pub fn get_steps(conn: &Connection, recipe_id: &str) -> Vec<StoredStep> {
        let mut stmt = conn
            .prepare(
                "SELECT step_index, step_json, status, result
                 FROM recipe_steps WHERE recipe_id = ?1 ORDER BY step_index",
            )
            .expect("prepare get_steps");

        stmt.query_map(params![recipe_id], |row| {
            let step_json: String = row.get(1)?;
            let step: RecipeStep = serde_json::from_str(&step_json)
                .unwrap_or(RecipeStep::Notify { message: format!("{UNREADABLE}{step_json}") });
            Ok(StoredStep {
                step_index: row.get::<_, i64>(0)? as usize,
                step,
                status: row.get(2)?,
                result: row.get(3)?,
            })
        })
        .expect("query get_steps")
        .filter_map(|r| r.ok())
        .collect()
    }

    /// Get recipe variables.
    pub fn get_vars(conn: &Connection, recipe_id: &str) -> std::collections::HashMap<String, serde_json::Value> {
        let mut stmt = conn
            .prepare("SELECT key, value FROM recipe_vars WHERE recipe_id = ?1")
            .expect("prepare get_vars");

        stmt.query_map(params![recipe_id], |row| {
            let key: String = row.get(0)?;
            let val_str: String = row.get(1)?;
            let val: serde_json::Value = serde_json::from_str(&val_str).unwrap_or(serde_json::Value::String(val_str));
            Ok((key, val))
        })
        .expect("query get_vars")
        .filter_map(|r| r.ok())
        .collect()
    }

    /// Set a recipe variable.
    pub fn set_var(conn: &Connection, recipe_id: &str, key: &str, value: &serde_json::Value) {
        let val_str = serde_json::to_string(value).unwrap_or_default();
        conn.execute(
            "INSERT OR REPLACE INTO recipe_vars (recipe_id, key, value) VALUES (?1, ?2, ?3)",
            params![recipe_id, key, val_str],
        )
        .ok();
    }

    /// Forget a recipe variable.
    pub fn delete_var(conn: &Connection, recipe_id: &str, key: &str) {
        conn.execute("DELETE FROM recipe_vars WHERE recipe_id = ?1 AND key = ?2", params![recipe_id, key]).ok();
    }

    /// Mark the steps in `from..to` as not run: a loop going round them again, or a template made
    /// clean for its next run.
    pub fn reset_steps(conn: &Connection, recipe_id: &str, from: usize, to: usize) {
        conn.execute(
            "UPDATE recipe_steps SET status = 'pending', result = NULL
             WHERE recipe_id = ?1 AND step_index >= ?2 AND step_index < ?3",
            params![recipe_id, from as i64, to.min(i64::MAX as usize) as i64],
        )
        .ok();
    }

    /// A new recipe with another's name, description and steps. `with_state`: its status, step
    /// pointer, error, times, steps' records and variables too — a run moved to an id of its own.
    /// Otherwise nothing has run: a new run of the definition. Triggers stay with the original.
    fn copy(conn: &Connection, source_id: &str, with_state: bool) -> Option<String> {
        let source = Self::get(conn, source_id)?;
        let steps = Self::get_steps(conn, source_id);
        let id = format!("rcp_{}", &uuid7::uuid7().to_string()[24..]);
        let now = now_ts();
        let (status, current, error, created, updated) = if with_state {
            (source.status.as_str(), source.current_step, source.error_message.clone(), source.created_at, source.updated_at)
        } else {
            ("pending", 0, None, now, now)
        };
        conn.execute(
            "INSERT INTO recipes (id, name, description, status, current_step, error_msg, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![id, source.name, source.description, status, current as i64, error, created, updated],
        )
        .ok()?;
        for s in &steps {
            let step_json = serde_json::to_string(&s.step).unwrap_or_default();
            let (step_status, result) = if with_state { (s.status.as_str(), s.result.clone()) } else { ("pending", None) };
            conn.execute(
                "INSERT INTO recipe_steps (recipe_id, step_index, step_json, status, result) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id, s.step_index as i64, step_json, step_status, result],
            )
            .ok();
        }
        if with_state {
            for (key, value) in Self::get_vars(conn, source_id) {
                Self::set_var(conn, &id, &key, &value);
            }
            // A run moved to an id of its own keeps the leave it was given; a new run never
            // inherits one.
            if let Some(leave) = Self::agents_allowed(conn, source_id) {
                Self::allow_agents(conn, &id, &leave);
                conn.execute("DELETE FROM recipe_agent_leave WHERE recipe_id = ?1", params![source_id]).ok();
            }
        }
        Some(id)
    }

    /// Start a recipe, by id or by name, with these variables. Returns the id of the run.
    ///
    /// A run of a template — a built-in — or of a recipe that has already run is a new recipe with
    /// the same steps, so each run keeps its own record and the template stays a template. It
    /// used to be reset in place: steps to pending, pointer to 0, the last run's record gone. A
    /// recipe that has never run starts on its own id. One in flight is not started over itself.
    pub fn start_run(
        conn: &Connection,
        id_or_name: &str,
        variables: Option<&serde_json::Map<String, serde_json::Value>>,
    ) -> Result<(Recipe, String), String> {
        let recipe = match Self::get(conn, id_or_name) {
            Some(r) => r,
            None => Self::find_by_name(conn, id_or_name).ok_or_else(|| format!("Recipe not found: {id_or_name}"))?,
        };
        if matches!(recipe.status, RecipeStatus::Running | RecipeStatus::Waiting | RecipeStatus::Paused) {
            return Err(format!(
                "Recipe '{}' [{}] is already {} — cancel it first to start it over.",
                recipe.name,
                recipe.id,
                recipe.status.as_str()
            ));
        }
        let touched = Self::get_steps(conn, &recipe.id).iter().any(|s| s.status != "pending");
        let run = if recipe.id.starts_with("builtin_") || recipe.status != RecipeStatus::Pending || touched {
            Self::copy(conn, &recipe.id, false).ok_or_else(|| format!("Recipe '{}' could not be copied for a new run", recipe.name))?
        } else {
            recipe.id.clone()
        };
        for (key, value) in variables.into_iter().flatten() {
            Self::set_var(conn, &run, key, value);
        }
        // A template's inputs that have a default and were not given — a formation's seats.
        let given = |k: &str| variables.is_some_and(|v| v.get(k).is_some_and(|v| !v.is_null()));
        for (key, value, _) in crate::recipe_templates::defaults(&recipe.id) {
            if !given(key) {
                Self::set_var(conn, &run, key, &serde_json::Value::String(value.to_string()));
            }
        }
        Self::update_status(conn, &run, &RecipeStatus::Running, 0);
        Ok((recipe, run))
    }

    /// Update recipe status and current step.
    pub fn update_status(conn: &Connection, recipe_id: &str, status: &RecipeStatus, current_step: usize) {
        let now = now_ts();
        conn.execute(
            "UPDATE recipes SET status = ?1, current_step = ?2, updated_at = ?3 WHERE id = ?4",
            params![status.as_str(), current_step as i64, now, recipe_id],
        )
        .ok();
    }

    /// Set error message on a recipe.
    pub fn set_error(conn: &Connection, recipe_id: &str, error: &str) {
        let now = now_ts();
        conn.execute(
            "UPDATE recipes SET status = 'failed', error_msg = ?1, updated_at = ?2 WHERE id = ?3",
            params![error, now, recipe_id],
        )
        .ok();
    }

    /// Hold a running or waiting recipe where it is. Returns the status it was paused from.
    ///
    /// The step pointer does not move and nothing is marked, so `resume` is exact: a recipe paused
    /// while waiting on a timer or on a person's answer goes back to waiting on it.
    pub fn pause(conn: &Connection, recipe_id: &str) -> Result<RecipeStatus, String> {
        let recipe = Self::get(conn, recipe_id).ok_or_else(|| format!("no recipe `{recipe_id}`"))?;
        match recipe.status {
            RecipeStatus::Running | RecipeStatus::Waiting => {
                Self::set_var(conn, recipe_id, PAUSED_FROM_VAR, &serde_json::json!(recipe.status.as_str()));
                Self::update_status(conn, recipe_id, &RecipeStatus::Paused, recipe.current_step);
                Ok(recipe.status)
            }
            other => Err(format!("`{}` is {}, and only a running or waiting recipe can be paused", recipe.name, other.as_str())),
        }
    }

    /// Put a paused recipe back to what it was doing. Returns the status it resumes as.
    ///
    /// `running` means the caller should signal the executor; `waiting` means it waits again —
    /// for its answer, or for its timer, which keeps the time it was set to wake at (`_wait`): a
    /// timer that came due during the pause wakes at the clock's next tick.
    pub fn resume(conn: &Connection, recipe_id: &str) -> Result<RecipeStatus, String> {
        let recipe = Self::get(conn, recipe_id).ok_or_else(|| format!("no recipe `{recipe_id}`"))?;
        if recipe.status != RecipeStatus::Paused {
            return Err(format!("`{}` is {}, not paused", recipe.name, recipe.status.as_str()));
        }
        let from = Self::get_vars(conn, recipe_id)
            .get(PAUSED_FROM_VAR)
            .and_then(|v| v.as_str().map(RecipeStatus::from_str))
            .filter(|s| *s == RecipeStatus::Waiting)
            .unwrap_or(RecipeStatus::Running);
        Self::update_status(conn, recipe_id, &from, recipe.current_step);
        Ok(from)
    }

    /// Stop a recipe for good, as the chat's "cancel" does: failed, with [`CANCELLED`].
    pub fn cancel(conn: &Connection, recipe_id: &str) -> Result<(), String> {
        let recipe = Self::get(conn, recipe_id).ok_or_else(|| format!("no recipe `{recipe_id}`"))?;
        match recipe.status {
            RecipeStatus::Running | RecipeStatus::Waiting | RecipeStatus::Paused => {
                Self::set_error(conn, recipe_id, CANCELLED);
                Ok(())
            }
            other => Err(format!("`{}` is {}, and there is nothing to cancel", recipe.name, other.as_str())),
        }
    }

    /// Mark a step as done with a result.
    pub fn complete_step(conn: &Connection, recipe_id: &str, step_index: usize, result: &str) {
        conn.execute(
            "UPDATE recipe_steps SET status = 'done', result = ?1
             WHERE recipe_id = ?2 AND step_index = ?3",
            params![result, recipe_id, step_index as i64],
        )
        .ok();
    }

    /// Mark a step as failed.
    pub fn fail_step(conn: &Connection, recipe_id: &str, step_index: usize, error: &str) {
        conn.execute(
            "UPDATE recipe_steps SET status = 'failed', result = ?1
             WHERE recipe_id = ?2 AND step_index = ?3",
            params![error, recipe_id, step_index as i64],
        )
        .ok();
    }

    /// Mark a step as skipped.
    pub fn skip_step(conn: &Connection, recipe_id: &str, step_index: usize) {
        conn.execute(
            "UPDATE recipe_steps SET status = 'skipped' WHERE recipe_id = ?1 AND step_index = ?2",
            params![recipe_id, step_index as i64],
        )
        .ok();
    }

    /// Replace remaining steps from `from_step` onwards with new steps (for replanning).
    /// Keeps completed steps intact, replaces pending/failed ones.
    pub fn replace_remaining_steps(conn: &Connection, recipe_id: &str, from_step: usize, new_steps: &[RecipeStep]) {
        // Delete old steps from from_step onwards
        conn.execute(
            "DELETE FROM recipe_steps WHERE recipe_id = ?1 AND step_index >= ?2",
            params![recipe_id, from_step as i64],
        )
        .ok();
        // Insert new steps
        for (i, step) in new_steps.iter().enumerate() {
            let step_json = serde_json::to_string(step).unwrap_or_default();
            conn.execute(
                "INSERT INTO recipe_steps (recipe_id, step_index, step_json) VALUES (?1, ?2, ?3)",
                params![recipe_id, (from_step + i) as i64, step_json],
            )
            .ok();
        }
        tracing::info!(
            recipe_id = %recipe_id,
            from_step,
            new_count = new_steps.len(),
            "Replaced remaining recipe steps (replan)"
        );
    }

    /// Record a recipe failure for learning. Stores the failure context so future
    /// recipe creation can avoid the same mistakes.
    pub fn record_failure_learning(conn: &Connection, recipe_id: &str, step_index: usize,
                                    tool_name: &str, error: &str, resolution: &str) {
        // Use recipe_vars to store learning data (avoid new table)
        let learning = serde_json::json!({
            "step": step_index,
            "tool": tool_name,
            "error": error,
            "resolution": resolution,
            "timestamp": now_ts(),
        });
        let key = format!("_learning_{}", step_index);
        Self::set_var(conn, recipe_id, &key, &learning);
    }

    /// List recipes with optional status filter.
    pub fn list(conn: &Connection, status_filter: Option<&str>, limit: usize) -> Vec<Recipe> {
        let (sql, p): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match status_filter {
            Some(s) => (
                format!(
                    "SELECT id, name, description, status, current_step, enabled, error_msg, created_at, updated_at
                     FROM recipes WHERE status = ?1 ORDER BY updated_at DESC LIMIT {limit}"
                ),
                vec![Box::new(s.to_string()) as Box<dyn rusqlite::types::ToSql>],
            ),
            None => (
                format!(
                    "SELECT id, name, description, status, current_step, enabled, error_msg, created_at, updated_at
                     FROM recipes ORDER BY updated_at DESC LIMIT {limit}"
                ),
                vec![],
            ),
        };

        let mut stmt = match conn.prepare(&sql) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let refs: Vec<&dyn rusqlite::types::ToSql> = p.iter().map(|x| x.as_ref()).collect();

        stmt.query_map(refs.as_slice(), |row| {
            let enabled_i: i32 = row.get(5)?;
            Ok(Recipe {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                status: RecipeStatus::from_str(&row.get::<_, String>(3)?),
                current_step: row.get::<_, i64>(4)? as usize,
                enabled: enabled_i != 0,
                error_message: row.get(6)?,
                created_at: row.get(7)?,
                updated_at: row.get(8)?,
            })
        })
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|r| r.ok())
        .collect()
    }

    /// Get all enabled triggers.
    pub fn get_enabled_triggers(conn: &Connection) -> Vec<(String, TriggerType, f64)> {
        let mut stmt = conn
            .prepare(
                "SELECT t.recipe_id, t.trigger_json, t.last_fired
                 FROM recipe_triggers t
                 JOIN recipes r ON r.id = t.recipe_id
                 WHERE t.enabled = 1 AND r.enabled = 1 AND r.status IN ('pending', 'done')",
            )
            .expect("prepare triggers");

        stmt.query_map([], |row| {
            let recipe_id: String = row.get(0)?;
            let trigger_json: String = row.get(1)?;
            let last_fired: f64 = row.get(2)?;
            let trigger: TriggerType = serde_json::from_str(&trigger_json)
                .unwrap_or(TriggerType::Manual);
            Ok((recipe_id, trigger, last_fired))
        })
        .expect("query triggers")
        .filter_map(|r| r.ok())
        .collect()
    }

    /// Record that a trigger fired, at the moment it fired for: `now` for a schedule, the
    /// leader's completion time for a RecipeComplete — so the same event never fires it twice.
    pub fn record_trigger_fired(conn: &Connection, recipe_id: &str, at: f64) {
        conn.execute(
            "UPDATE recipe_triggers SET last_fired = ?1 WHERE recipe_id = ?2",
            params![at, recipe_id],
        )
        .ok();
    }

    /// When a leader recipe last finished, by id or by name: the newest `updated_at` among the
    /// done recipes that carry its name. A run of a template, or of a recipe that has run
    /// before, is a copy sharing the leader's name (`start_run`), so a chain formed on the first
    /// run keeps forming on every later one. A name nothing carries is a leader that never
    /// finished: None.
    pub fn completion_since(conn: &Connection, leader: &str) -> Option<f64> {
        conn.query_row(
            "SELECT MAX(updated_at) FROM recipes
             WHERE status = 'done' AND name IN (SELECT name FROM recipes WHERE id = ?1 OR name = ?1)",
            params![leader],
            |row| row.get::<_, Option<f64>>(0),
        )
        .ok()
        .flatten()
    }

    /// Start every recipe whose trigger is due at `now`, and return the runs started — the
    /// wiring the stored triggers never had (#187): `get_enabled_triggers` had no caller, so a
    /// Cron schedule or a RecipeComplete chain was written down and then ignored. The executor's
    /// clock calls this each tick ([`crate::recipe_executor::due_at`]).
    ///
    /// A trigger that has never fired waits for its first occurrence after the recipe existed
    /// (`last_fired` floors at `created_at`): a schedule made this afternoon does not fire for
    /// this morning, and a boot does not fire yesterday's. A schedule missed while the machine
    /// was off fires once at the next tick, not once per occurrence missed.
    pub fn fire_due_triggers_at(conn: &Connection, now: f64) -> Vec<String> {
        let offset = local_offset_secs(now);
        let mut started = Vec::new();
        for (recipe_id, trigger, last_fired) in Self::get_enabled_triggers(conn) {
            let Some(recipe) = Self::get(conn, &recipe_id) else { continue };
            let floor = last_fired.max(recipe.created_at);
            let due = match &trigger {
                TriggerType::Cron { expression } => cron_due(expression, floor, now, offset),
                TriggerType::RecipeComplete { recipe_id: leader } => {
                    Self::completion_since(conn, leader).is_some_and(|at| at > floor)
                }
                // Manual waits for a person; Event waits for its event
                // (`fire_event_triggers`), not for the clock.
                TriggerType::Manual | TriggerType::Event { .. } => false,
            };
            if !due {
                continue;
            }
            match Self::start_run(conn, &recipe_id, None) {
                Ok((_, run)) => {
                    // A chain fires on the completion it answers, so that same completion can
                    // never fire it again; a schedule fires on the clock it was read against.
                    let at = match &trigger {
                        TriggerType::RecipeComplete { recipe_id: leader } => {
                            Self::completion_since(conn, leader).unwrap_or(now)
                        }
                        _ => now,
                    };
                    Self::record_trigger_fired(conn, &recipe_id, at);
                    tracing::info!(recipe_id = %recipe_id, run = %run, trigger = ?trigger, "Recipe trigger fired");
                    started.push(run);
                }
                // A recipe already in flight is not started over itself (`start_run`); nothing
                // is recorded, so the trigger is asked again at the next tick.
                Err(why) => tracing::debug!(recipe_id = %recipe_id, %why, "A due trigger could not start its recipe"),
            }
        }
        started
    }

    /// Start every recipe whose Event trigger names `event_type` — and whose filter, if it has
    /// one, the event's data satisfies — and return the runs started. Called where system events
    /// arrive; the started runs are picked up by the executor's clock like any other.
    pub fn fire_event_triggers(conn: &Connection, event_type: &str, event_data: &serde_json::Value, now: f64) -> Vec<String> {
        let mut started = Vec::new();
        for (recipe_id, trigger, _) in Self::get_enabled_triggers(conn) {
            let TriggerType::Event { event_type: named, filter } = &trigger else { continue };
            if named != event_type {
                continue;
            }
            if let Some(want) = filter.as_ref().and_then(|f| f.as_object()) {
                if !want.iter().all(|(key, value)| event_data.get(key) == Some(value)) {
                    continue;
                }
            }
            match Self::start_run(conn, &recipe_id, None) {
                Ok((_, run)) => {
                    Self::record_trigger_fired(conn, &recipe_id, now);
                    tracing::info!(recipe_id = %recipe_id, run = %run, event = %event_type, "Recipe event trigger fired");
                    started.push(run);
                }
                Err(why) => tracing::debug!(recipe_id = %recipe_id, %why, "An event trigger could not start its recipe"),
            }
        }
        started
    }

    /// Count running/waiting recipes.
    pub fn active_count(conn: &Connection) -> usize {
        conn.query_row(
            "SELECT COUNT(*) FROM recipes WHERE status IN ('running', 'waiting', 'paused', 'pending')",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0) as usize
    }

    /// Get recipes that need processing (pending or running).
    pub fn get_resumable(conn: &Connection) -> Vec<String> {
        // Only genuinely in-flight recipes are resumable. 'pending' is the status
        // register_builtin() stamps on a recipe *definition* that has never run —
        // including it here made every boot execute all ~50 built-in templates with
        // their {{variables}} unsubstituted, saturating the companion worker so real
        // user queries never got serviced. 'waiting' is handled by get_expired_waiting().
        let mut stmt = conn
            .prepare("SELECT id FROM recipes WHERE status = 'running'")
            .expect("prepare resumable");

        stmt.query_map([], |row| row.get::<_, String>(0))
            .expect("query resumable")
            .filter_map(|r| r.ok())
            .collect()
    }

    /// Waiting recipes whose wait is over now: the ones the worker's clock resumes.
    pub fn get_expired_waiting(conn: &Connection) -> Vec<String> {
        Self::get_expired_waiting_at(conn, now_ts())
    }

    /// Waiting recipes whose wait is over at `now` ([`Waited::is_over`]): a timer whose time has
    /// come — counted from when it began, to the next occurrence of a time of day — or a recipe
    /// left `waiting` with no wait behind it. Never a question: its answer (the Recipes screen,
    /// `answer_recipe`, or the chat) sets the recipe running. Resuming a question here walked the
    /// recipe on at the next chat message with `{{store_as}}` unbound.
    ///
    /// And a recipe with an agent still working, whatever else it waits on: the clock hears the
    /// answer and lets the agent go even while the recipe waits on a person or a timer.
    pub fn get_expired_waiting_at(conn: &Connection, now: f64) -> Vec<String> {
        Self::list(conn, Some("waiting"), 1_000)
            .into_iter()
            .filter(|r| {
                let steps = Self::get_steps(conn, &r.id);
                let vars = Self::get_vars(conn, &r.id);
                waited_on(r, &steps, &vars).map_or(true, |w| w.is_over(now))
                    || agent_runs(&vars).values().any(AgentRun::working)
            })
            .map(|r| r.id)
            .collect()
    }

    /// Collect failure learnings across all recipes for context injection.
    /// Returns a human-readable summary of past recipe failures and how they were resolved.
    pub fn get_failure_learnings(conn: &Connection, limit: usize) -> Vec<String> {
        // Query learning vars from all recipes (keys starting with _learning_)
        let mut stmt = conn
            .prepare(
                "SELECT rv.recipe_id, r.name, rv.value
                 FROM recipe_vars rv
                 JOIN recipes r ON r.id = rv.recipe_id
                 WHERE rv.key LIKE '_learning_%'
                 ORDER BY ROWID DESC
                 LIMIT ?1"
            )
            .unwrap_or_else(|_| conn.prepare("SELECT '', '', '' FROM recipe_vars LIMIT 0").unwrap());

        stmt.query_map(params![limit as i64], |row| {
            let recipe_name: String = row.get(1)?;
            let value_str: String = row.get(2)?;
            Ok((recipe_name, value_str))
        })
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|r| r.ok())
        .map(|(name, val)| {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&val) {
                format!(
                    "Recipe '{}': tool '{}' failed with '{}'. Resolution: {}",
                    name,
                    v.get("tool").and_then(|t| t.as_str()).unwrap_or("?"),
                    v.get("error").and_then(|t| t.as_str()).unwrap_or("?"),
                    v.get("resolution").and_then(|t| t.as_str()).map(|s|
                        if s.len() > 200 { format!("{}...", &s[..s.floor_char_boundary(200)]) } else { s.to_string() }
                    ).unwrap_or_default(),
                )
            } else {
                format!("Recipe '{}': {}", name, val)
            }
        })
        .collect()
    }

    /// Format summary for system context injection.
    pub fn format_summary(conn: &Connection) -> String {
        let recipes = Self::list(conn, None, 10);
        let active: Vec<&Recipe> = recipes
            .iter()
            .filter(|r| r.status != RecipeStatus::Done && r.status != RecipeStatus::Failed)
            .collect();

        if active.is_empty() {
            return String::new();
        }

        let mut lines = vec!["Recipes:".to_string()];
        for r in active {
            let icon = match r.status {
                RecipeStatus::Running => "▶",
                RecipeStatus::Waiting => "⏸",
                RecipeStatus::Paused => "‖",
                RecipeStatus::Pending => "○",
                _ => "?",
            };
            lines.push(format!("  {} [{}] {} (step {}) — {}", icon, r.id, r.name, r.current_step, r.status.as_str()));
        }
        lines.join("\n")
    }
}

// ── Step Executor ──

/// Result of executing a single step.
pub enum StepResult {
    /// Step completed, advance to next step.
    Continue,
    /// Jump to a specific step index.
    JumpTo(usize),
    /// Recipe is waiting for a condition. Persist and pause.
    Waiting,
    /// Recipe completed (reached end or Notify was the last step).
    Done,
    /// Step failed with error message.
    Failed(String),
    /// Step produced a notification to deliver.
    Notify(String),
}

/// Resolve {{variable}} references in a string using recipe variables.
pub fn resolve_vars(template: &str, vars: &std::collections::HashMap<String, serde_json::Value>) -> String {
    let mut result = template.to_string();
    for (key, value) in vars {
        let placeholder = format!("{{{{{}}}}}", key);
        let replacement = match value {
            serde_json::Value::String(s) => s.clone(),
            other => serde_json::to_string(other).unwrap_or_default(),
        };
        result = result.replace(&placeholder, &replacement);
    }
    result
}

/// Resolve {{variable}} references in JSON args.
pub fn resolve_vars_in_json(
    args: &serde_json::Value,
    vars: &std::collections::HashMap<String, serde_json::Value>,
) -> serde_json::Value {
    match args {
        serde_json::Value::String(s) => {
            // Check if it's a pure variable reference like "{{emails}}"
            let trimmed = s.trim();
            if trimmed.starts_with("{{") && trimmed.ends_with("}}") {
                let var_name = &trimmed[2..trimmed.len() - 2];
                if let Some(val) = vars.get(var_name) {
                    return val.clone();
                }
            }
            serde_json::Value::String(resolve_vars(s, vars))
        }
        serde_json::Value::Object(map) => {
            let mut new_map = serde_json::Map::new();
            for (k, v) in map {
                new_map.insert(k.clone(), resolve_vars_in_json(v, vars));
            }
            serde_json::Value::Object(new_map)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(|v| resolve_vars_in_json(v, vars)).collect())
        }
        other => other.clone(),
    }
}

// ── Helpers ──

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

/// Get current hour and minute on this machine's clock. It said "local time" and computed UTC,
/// so a JumpIf on "after 17:00" answered against somebody else's evening (#187).
fn chrono_now() -> (u8, u8) {
    let now = now_ts();
    let secs = now as i64 + local_offset_secs(now) as i64;
    let hour = (secs.rem_euclid(86_400) / 3_600) as u8;
    let minute = (secs.rem_euclid(3_600) / 60) as u8;
    (hour, minute)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn store() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory sqlite");
        RecipeStore::ensure_tables(&conn);
        conn
    }

    fn tool(name: &str, store_as: &str) -> RecipeStep {
        RecipeStep::Tool { tool_name: name.into(), args: json!({}), store_as: store_as.into(), on_error: ErrorAction::Fail }
    }

    /// A built-in keeps the record of what it did across a restart (#176). `ensure_builtin` runs
    /// at every start, and it deleted and re-inserted every built-in's step rows — so a built-in
    /// that had run (in place, on its own id, as `run_recipe` used to run it) lost its results and
    /// states at the next start, and the Recipes screen could only say "not recorded".
    #[test]
    fn a_built_in_keeps_what_it_did_across_a_restart() {
        let conn = store();
        let steps = [tool("get_weather", "weather"), RecipeStep::Notify { message: "{{weather}}".into() }];
        RecipeStore::ensure_builtin(&conn, "builtin_weather", "Weather", "", &steps);
        // A run as the old `run_recipe` made one: in place, on the built-in's own id.
        RecipeStore::set_var(&conn, "builtin_weather", "weather", &json!("Sunny, 21°C"));
        RecipeStore::complete_step(&conn, "builtin_weather", 0, "Sunny, 21°C");
        RecipeStore::complete_step(&conn, "builtin_weather", 1, "Sunny, 21°C");
        RecipeStore::update_status(&conn, "builtin_weather", &RecipeStatus::Done, 2);

        // The shell starts again.
        RecipeStore::ensure_builtin(&conn, "builtin_weather", "Weather", "", &steps);

        let all = RecipeStore::list(&conn, None, 10);
        let run = all.iter().find(|r| r.status == RecipeStatus::Done).expect("the run is still in the store, done");
        let kept = RecipeStore::get_steps(&conn, &run.id);
        assert_eq!(kept.iter().map(|s| s.status.as_str()).collect::<Vec<_>>(), ["done", "done"], "its steps' states survive the start");
        assert_eq!(kept[0].result.as_deref(), Some("Sunny, 21°C"), "and their results");
        assert_eq!(RecipeStore::get_vars(&conn, &run.id).get("weather"), Some(&json!("Sunny, 21°C")));
        assert_ne!(run.id, "builtin_weather", "the run is kept as a recipe of its own");
        // The built-in is a clean definition again, for its next run.
        assert_eq!(RecipeStore::get(&conn, "builtin_weather").map(|r| r.status), Some(RecipeStatus::Pending));
        assert!(RecipeStore::get_steps(&conn, "builtin_weather").iter().all(|s| s.status == "pending" && s.result.is_none()));
        assert!(RecipeStore::get_vars(&conn, "builtin_weather").is_empty());

        // A start that changes nothing touches nothing.
        RecipeStore::ensure_builtin(&conn, "builtin_weather", "Weather", "", &steps);
        assert_eq!(RecipeStore::list(&conn, None, 10).len(), 2, "no second copy of the run");
        assert_eq!(RecipeStore::get_steps(&conn, &run.id)[0].result.as_deref(), Some("Sunny, 21°C"));
    }

    /// A wait for a time of day that has already gone by today waits for tomorrow's. It counted as
    /// met — `h > hour || (h == hour && m >= minute)` — so "wait until 09:00" set at 10:00 went on
    /// at once, and a daily loop around it would spin.
    #[test]
    fn a_time_of_day_already_gone_waits_for_tomorrow() {
        let conn = store();
        let now = now_ts();
        let of_day = (now as i64 + local_offset_secs(now)).rem_euclid(86_400);
        if of_day < 180 {
            // The first minutes after midnight on the machine's clock: nothing today has gone by yet.
            return;
        }
        let gone = of_day - 120;
        let (hour, minute) = ((gone / 3600) as u8, ((gone % 3600) / 60) as u8);
        let id = RecipeStore::create(
            &conn,
            "Daily digest",
            "",
            &[
                RecipeStep::WaitFor { condition: WaitCondition::Time { hour, minute }, timeout_secs: None },
                RecipeStep::Notify { message: "digest".into() },
            ],
            None,
        );
        RecipeStore::complete_step(&conn, &id, 0, "waiting");
        RecipeStore::update_status(&conn, &id, &RecipeStatus::Waiting, 1);
        assert!(
            RecipeStore::get_expired_waiting(&conn).is_empty(),
            "{hour:02}:{minute:02} has gone by the machine's today, so it waits for tomorrow's"
        );
    }

    /// When a wait begun at a moment wakes, to the second — on a clock at UTC with no offset,
    /// and on a machine's clock east of it, where the time of day it names is the machine's
    /// (#187).
    #[test]
    fn a_wait_wakes_at_its_next_time() {
        let day = 20_719.0 * 86_400.0; // 2026-09-23 00:00 UTC
        let at = |h: f64, m: f64, s: f64| day + h * 3600.0 + m * 60.0 + s;
        let nine = WaitCondition::Time { hour: 9, minute: 0 };
        assert_eq!(wakes_at_in(&nine, None, at(8.0, 0.0, 0.0), 0), Some(at(9.0, 0.0, 0.0)), "later today");
        assert_eq!(wakes_at_in(&nine, None, at(10.0, 0.0, 0.0), 0), Some(at(33.0, 0.0, 0.0)), "gone by: tomorrow's");
        assert_eq!(wakes_at_in(&nine, None, at(9.0, 0.0, 30.0), 0), None, "this very minute: no wait");
        assert_eq!(wakes_at_in(&nine, Some(600), at(8.0, 0.0, 0.0), 0), Some(at(8.0, 10.0, 0.0)), "no later than its timeout");
        let quarter = WaitCondition::Duration { seconds: 900 };
        assert_eq!(wakes_at_in(&quarter, None, at(8.0, 0.0, 0.0), 0), Some(at(8.0, 15.0, 0.0)));
        assert_eq!(wakes_at_in(&WaitCondition::Duration { seconds: 0 }, None, 5.0, 0), None);
        assert_eq!(clock_text_at(at(8.0, 15.0, 0.0), 0), "08:15");
        assert_eq!(clock_text_at(at(33.0, 0.0, 0.0), 0), "09:00");

        // Five and a half hours east, 08:00 UTC is already 13:30 in the afternoon: today's
        // 09:00 has gone by, and tomorrow's is 03:30 UTC. The clock reads 13:45, not 08:15.
        let ist = 19_800;
        assert_eq!(wakes_at_in(&nine, None, at(8.0, 0.0, 0.0), ist), Some(at(27.0, 30.0, 0.0)), "the next local 09:00");
        assert_eq!(clock_text_at(at(8.0, 15.0, 0.0), ist), "13:45");
    }

    /// A schedule is read on the machine's clock (#187): 09:00 where the machine stands, which
    /// east of Greenwich is earlier in the UTC day — not 09:00 UTC, a time nobody there set.
    #[test]
    fn a_cron_schedule_is_read_on_the_machines_clock() {
        let day = 20_719.0 * 86_400.0; // 2026-09-23 00:00 UTC
        let ist = 19_800;
        // 09:00 local east of Greenwich is 03:30 UTC.
        let half_past_three_utc = day + 3.5 * 3_600.0;
        assert!(!cron_due("0 9 * * *", day, half_past_three_utc - 61.0, ist), "a minute early is not due");
        assert!(cron_due("0 9 * * *", day, half_past_three_utc, ist), "on the local nine o'clock");
        // The same expression on a clock at UTC waits for the UTC nine o'clock.
        assert!(!cron_due("0 9 * * *", day, half_past_three_utc, 0));
        assert!(cron_due("0 9 * * *", day, day + 9.0 * 3_600.0, 0));
        // The occurrence at the floor itself has been answered; an expression that does not
        // parse is never due.
        assert!(!cron_due("0 9 * * *", day + 9.0 * 3_600.0, day + 9.0 * 3_600.0, 0));
        assert!(!cron_due("nonsense", day, day + 86_400.0, 0));
    }

    /// Triggers were stored and never fired (#187): `get_enabled_triggers` had no caller. A due
    /// schedule starts its recipe once, and not again before its next occurrence; a trigger that
    /// has never fired waits for the first occurrence after the recipe existed.
    #[test]
    fn a_due_schedule_starts_its_recipe_once() {
        let conn = store();
        let day = 20_719.0 * 86_400.0; // 2026-09-23 00:00 UTC
        let steps = [RecipeStep::Notify { message: "digest".into() }];
        // Every minute: a schedule that reads the same whatever zone the test machine is in.
        let id = RecipeStore::create(&conn, "Digest", "", &steps, Some(&TriggerType::Cron { expression: "* * * * *".into() }));
        // create() stamps with the real clock; this recipe lives on the test's calendar.
        conn.execute("UPDATE recipes SET created_at = ?1 WHERE id = ?2", params![day - 120.0, id]).unwrap();

        assert!(RecipeStore::fire_due_triggers_at(&conn, day - 61.0).is_empty(), "not due before the first occurrence after it existed");
        assert_eq!(RecipeStore::fire_due_triggers_at(&conn, day), vec![id.clone()], "the due occurrence starts the recipe");
        assert_eq!(RecipeStore::get(&conn, &id).map(|r| r.status), Some(RecipeStatus::Running));
        assert!(RecipeStore::fire_due_triggers_at(&conn, day).is_empty(), "the same occurrence does not fire twice");
        assert!(RecipeStore::fire_due_triggers_at(&conn, day + 59.0).is_empty(), "nor before the next one");

        // The next occurrence, once the first run has finished, is a run of its own — a copy
        // sharing the name, as every re-run is (`start_run`).
        RecipeStore::complete_step(&conn, &id, 0, "digest");
        RecipeStore::update_status(&conn, &id, &RecipeStatus::Done, 1);
        let next = RecipeStore::fire_due_triggers_at(&conn, day + 60.0);
        assert_eq!(next.len(), 1, "the schedule fires again at its next occurrence");
        assert_ne!(next[0], id, "a re-run is a recipe of its own");
        assert_eq!(RecipeStore::get(&conn, &next[0]).map(|r| r.name), Some("Digest".to_string()));
    }

    /// A leader's completion starts the recipe chained behind it — including when the leader's
    /// run is a copy sharing its name, which is how every run after the first happens.
    #[test]
    fn a_recipe_complete_trigger_starts_the_recipe_it_chains() {
        let conn = store();
        let steps = [RecipeStep::Notify { message: "sat".into() }];
        let leader = RecipeStore::create(&conn, "Council", "", &steps, None);
        let follower = RecipeStore::create(&conn, "Chair", "", &steps, Some(&TriggerType::RecipeComplete { recipe_id: leader.clone() }));
        assert_eq!(RecipeStore::completion_since(&conn, "no such recipe"), None);

        // The leader in flight, nothing finished: nothing fires.
        RecipeStore::update_status(&conn, &leader, &RecipeStatus::Running, 0);
        assert!(RecipeStore::fire_due_triggers_at(&conn, now_ts()).is_empty());

        // The leader finishes; the clock sees it and starts the follower.
        RecipeStore::complete_step(&conn, &leader, 0, "sat");
        RecipeStore::update_status(&conn, &leader, &RecipeStatus::Done, 1);
        assert_eq!(RecipeStore::fire_due_triggers_at(&conn, now_ts()), vec![follower.clone()], "the completion starts the chained recipe");
        assert!(RecipeStore::fire_due_triggers_at(&conn, now_ts()).is_empty(), "one completion fires once");

        // The follower finishes, the leader runs again as a copy of itself, and its completion
        // chains again — found by the name the copies share.
        RecipeStore::complete_step(&conn, &follower, 0, "sat");
        RecipeStore::update_status(&conn, &follower, &RecipeStatus::Done, 1);
        let first_done = RecipeStore::completion_since(&conn, &leader).expect("the leader finished");
        let (_, rerun) = RecipeStore::start_run(&conn, &leader, None).expect("a re-run");
        assert_ne!(rerun, leader);
        RecipeStore::complete_step(&conn, &rerun, 0, "sat");
        RecipeStore::update_status(&conn, &rerun, &RecipeStatus::Done, 1);
        // Stamp the copy's completion after the first one, however fast the store was.
        conn.execute("UPDATE recipes SET updated_at = ?1 WHERE id = ?2", params![first_done + 60.0, rerun]).unwrap();
        let started = RecipeStore::fire_due_triggers_at(&conn, now_ts());
        assert_eq!(started.len(), 1, "the copy's completion chains too");
        assert_ne!(started[0], follower, "and the follower's second run is a recipe of its own");
    }

    /// An event trigger fires on the event it names, and only when its filter — if it has one —
    /// is satisfied by the event's data.
    #[test]
    fn an_event_trigger_fires_on_its_event_and_its_filter() {
        let conn = store();
        let steps = [RecipeStep::Notify { message: "read".into() }];
        let id = RecipeStore::create(&conn, "On mail", "", &steps, Some(&TriggerType::Event { event_type: "system/mail".into(), filter: Some(json!({"importance": 0.9})) }));
        assert!(RecipeStore::fire_event_triggers(&conn, "system/other", &json!({}), now_ts()).is_empty(), "another event passes by");
        assert!(RecipeStore::fire_event_triggers(&conn, "system/mail", &json!({"importance": 0.5}), now_ts()).is_empty(), "the filter has a say");
        let started = RecipeStore::fire_event_triggers(&conn, "system/mail", &json!({"importance": 0.9, "text": "hello"}), now_ts());
        assert_eq!(started, vec![id.clone()], "its event, with data the filter accepts");
        assert_eq!(RecipeStore::get(&conn, &id).map(|r| r.status), Some(RecipeStatus::Running));
    }

    /// A new version's definition still reaches a built-in that has never run, and a copy of a
    /// run started from a template begins with nothing run.
    #[test]
    fn a_built_in_takes_a_changed_definition() {
        let conn = store();
        RecipeStore::ensure_builtin(&conn, "builtin_x", "X", "", &[tool("a", "a")]);
        RecipeStore::ensure_builtin(&conn, "builtin_x", "X", "", &[tool("a", "a"), tool("b", "b")]);
        let steps = RecipeStore::get_steps(&conn, "builtin_x");
        assert_eq!(steps.len(), 2);
        assert!(matches!(&steps[1].step, RecipeStep::Tool { tool_name, .. } if tool_name == "b"));
        assert_eq!(RecipeStore::list(&conn, None, 10).len(), 1, "nothing had run: nothing to keep");

        let vars = serde_json::Map::from_iter([("topic".to_string(), json!("rust"))]);
        let (template, run) = RecipeStore::start_run(&conn, "x", Some(&vars)).expect("started by name");
        assert_eq!(template.id, "builtin_x");
        assert_ne!(run, "builtin_x");
        let started = RecipeStore::get(&conn, &run).expect("the run");
        assert_eq!((started.status, started.current_step, started.name.as_str()), (RecipeStatus::Running, 0, "X"));
        assert_eq!(RecipeStore::get_vars(&conn, &run).get("topic"), Some(&json!("rust")));
        assert!(RecipeStore::start_run(&conn, "no such recipe", None).is_err());
    }
}
