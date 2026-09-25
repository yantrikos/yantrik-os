//! The mind panel — the right edge of every screen: which mind is answering and on what, what it
//! may do, what is working, and what it did.
//!
//! "The OS holds the desk; you bring the mind." The desk is the desktop and the apps; the mind is
//! whichever one is answering, and every agent working. This panel keeps it in view: expanded on
//! the desktop, a slim strip over everything else, the person's choice kept per place. It replaces
//! the machine rail's companion section (section 3 of design/desk-and-mind-2026-09-23.md).
//!
//! Four parts, each from the one place that knows it:
//!
//! | part | source |
//! |---|---|
//! | Now — the mind, its model, the mode and the ceiling | the harness host; the mode chip's own properties |
//! | Context — project, memory store, minds | `active-project`; the companion's memory count; the host's list |
//! | Working — agents at work, recipes in flight | the Agents store; the Recipes screen's copy of the companion's recipes (`crate::recipes`) |
//! | Recent actions | the mind audit, through [`recent_acts`] — the one function the ledger (#148) replaces |
//!
//! What is not known says so. A number that has not been read is never drawn as zero.
//!
//! Everything that decides what the panel says is a plain function over plain inputs ([`build`],
//! [`working`], [`recipes_in_flight`], [`act_lines`], [`describe_of`]); [`wire`] only gathers the
//! inputs once a second and hands the result to Slint.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use slint::{ComponentHandle, Model, Timer, TimerMode};

use crate::agents::model::State;
use crate::agents::{self, Store, Tab};
use crate::{App, MindPanelAct, MindPanelAgent, MindPanelNow, MindPanelRecipe, MindPanelState};

/// How many agents the panel lists; the rest are counted and one click away in Agents.
pub const AGENT_ROWS: usize = 4;
/// How many recipes in flight it lists.
pub const RECIPE_ROWS: usize = 3;
/// How many recent actions it lists.
pub const ACT_ROWS: usize = 4;

/// How often the panel is rebuilt. Its clocks ("2m", "4m ago") move in minutes; a second is below
/// noticing and costs two short locks.
const TICK: Duration = Duration::from_secs(1);

/// How many audit lines are read back from the file after a restart.
const AUDIT_SEED: usize = 50;

// ── Where it is drawn, and how open ─────────────────────────────────

/// The two places the panel keeps a choice for: the desktop, and everything else.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Desktop,
    Elsewhere,
}

impl Place {
    pub fn of_screen(screen: i32) -> Place {
        if screen == DESKTOP_SCREEN {
            Place::Desktop
        } else {
            Place::Elsewhere
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Place::Desktop => "desktop",
            Place::Elsewhere => "elsewhere",
        }
    }

    pub fn parse(text: &str) -> Option<Place> {
        match text.trim() {
            "desktop" => Some(Place::Desktop),
            "elsewhere" => Some(Place::Elsewhere),
            _ => None,
        }
    }
}

const DESKTOP_SCREEN: i32 = 1;

/// The screens the panel is drawn on: the same ones the taskbar is on. `mind-panel-shown` in
/// app.slint says the same thing, and a test holds the two together.
pub fn shown_on(screen: i32) -> bool {
    screen == DESKTOP_SCREEN || (4..=31).contains(&screen) || screen == 33 || screen == 34 || screen == 35
}

/// What the person chose, per place. Open on the desktop and a strip everywhere else until they
/// say otherwise.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Choice {
    pub desktop: bool,
    pub elsewhere: bool,
}

impl Default for Choice {
    fn default() -> Self {
        Choice { desktop: true, elsewhere: false }
    }
}

impl Choice {
    pub fn get(self, place: Place) -> bool {
        match place {
            Place::Desktop => self.desktop,
            Place::Elsewhere => self.elsewhere,
        }
    }

    pub fn set(&mut self, place: Place, expanded: bool) {
        match place {
            Place::Desktop => self.desktop = expanded,
            Place::Elsewhere => self.elsewhere = expanded,
        }
    }
}

/// `~/.config/yantrik/mind-panel.json`, beside settings.yaml. A file of its own rather than two
/// more keys in settings.yaml: a click on the panel's chevron is not a settings save, and the
/// settings file refuses writes once it has changed on disk underneath the shell.
pub fn choice_path() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/root"));
    home.join(".config").join("yantrik").join("mind-panel.json")
}

/// The saved choice. A missing file is the defaults; so is one that cannot be read, which is
/// logged and left where it is.
pub fn load_choice(path: &Path) -> Choice {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
            tracing::warn!(path = %path.display(), error = %e, "The mind panel's saved choice could not be read; using the defaults");
            Choice::default()
        }),
        Err(_) => Choice::default(),
    }
}

/// Write the choice: a temporary file, then a rename, so a crash never leaves half of one.
pub fn save_choice(path: &Path, choice: Choice) -> Result<(), String> {
    let dir = path.parent().ok_or("the mind panel's file has no directory")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("could not make {}: {e}", dir.display()))?;
    let text = serde_json::to_string_pretty(&choice).map_err(|e| e.to_string())?;
    let temp = path.with_extension("json.new");
    std::fs::write(&temp, format!("{text}\n")).map_err(|e| format!("could not write {}: {e}", temp.display()))?;
    std::fs::rename(&temp, path).map_err(|e| format!("could not replace {}: {e}", path.display()))
}

/// Whether the panel is drawn open on `screen`: shown there, not in focus mode on the desktop,
/// and chosen open for that place. The same rule as `mind-panel-open` in app.slint.
pub fn open_on(choice: Choice, screen: i32, focus_mode: bool) -> bool {
    shown_on(screen) && !(screen == DESKTOP_SCREEN && focus_mode) && choice.get(Place::of_screen(screen))
}

// ── Now: the answering mind ──────────────────────────────────────────

/// A mind, as the harness host lists it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mind {
    pub name: String,
    pub builtin: bool,
    /// What it said about itself when it attached: its model, wherever it runs.
    pub detail: Option<String>,
}

/// What the harness host says: the answering mind, and every mind attached.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MindsSeen {
    pub answering: Option<Mind>,
    pub count: usize,
    /// Minds that say they keep memory past a turn. Not "share this store": a harness's memory is
    /// its own unless it says otherwise, and the protocol has no way to say otherwise.
    pub keep_memory: usize,
}

/// The host's list, read now. `None` until the host exists.
pub fn minds_seen() -> Option<MindsSeen> {
    let host = crate::wire::harness::host()?;
    let entries = host.list();
    Some(MindsSeen {
        answering: entries.iter().find(|e| e.active).map(|e| Mind {
            name: e.name.clone(),
            builtin: e.builtin,
            detail: e.detail.clone(),
        }),
        count: entries.len(),
        keep_memory: entries.iter().filter(|e| e.capabilities.memory).count(),
    })
}

// ── Working: agents ─────────────────────────────────────────────────

/// One agent at work.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AgentLine {
    pub id: String,
    pub mind: String,
    pub title: String,
    pub state: String,
    pub label: String,
    pub since: String,
}

/// The agents at work: listed (needs-you first), counted, and how many did not fit.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Working {
    pub rows: Vec<AgentLine>,
    pub running: usize,
    pub needs_you: usize,
    pub more: usize,
}

/// The agents at work in `store`: thinking, running a tool, waiting for the person, or with a
/// command of theirs still running. Idle, done and failed agents are the Agents screen's business,
/// not the edge of every screen's.
///
/// The order is the store's own — newest first — with the ones waiting for the person lifted to
/// the top, which is the order the Agents screen's Active tab uses.
pub fn working(store: &Store, now: u64, cap: usize) -> Working {
    let mut at_work: Vec<&agents::model::Agent> = store
        .list(Tab::All, None)
        .iter()
        .filter_map(|id| store.agent(id))
        .filter(|a| a.busy())
        .collect();
    // Stable: newest first stays newest first within each group.
    at_work.sort_by_key(|a| a.state != State::WaitingForYou);
    let needs_you = at_work.iter().filter(|a| a.state == State::WaitingForYou).count();
    let rows: Vec<AgentLine> = at_work
        .iter()
        .take(cap)
        .map(|a| AgentLine {
            id: a.meta.id.0.clone(),
            mind: a.meta.mind.clone(),
            title: if a.meta.title.trim().is_empty() { "untitled".into() } else { a.meta.title.clone() },
            state: a.state.key().into(),
            label: a.state.label().into(),
            since: for_how_long(now.saturating_sub(a.since)),
        })
        .collect();
    Working { more: at_work.len() - rows.len(), running: at_work.len(), needs_you, rows }
}

/// "just now", "40s", "2m", "1h 5m" — the Agents screen's words for how long a state has held.
pub fn for_how_long(secs: u64) -> String {
    match secs {
        0..=4 => "just now".into(),
        5..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m", secs / 60),
        _ => format!("{}h {}m", secs / 3600, (secs % 3600) / 60),
    }
}

// ── Working: recipes in flight ──────────────────────────────────────

/// One recipe in flight, one line.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RecipeLine {
    pub id: String,
    pub name: String,
    pub step: String,
    pub status: String,
    /// Its agents need the person: a card to answer, a place to free.
    pub needs_you: bool,
}

/// Whether the companion's worker has reached its command loop. The memory count it pushes is
/// only a count once this is true; before it, the property's zero is only a default.
static WORKER_UP: AtomicBool = AtomicBool::new(false);

/// Called by the companion's worker (bridge.rs) before it waits for each command.
pub fn worker_up() {
    WORKER_UP.store(true, Ordering::Relaxed);
}

/// The recipes in flight. The one function the panel reads them through, and it reads the Recipes
/// screen's own copy (`crate::recipes`, published by the companion's worker, which owns the store),
/// so the panel, that screen and `describe shell` say the same thing about a recipe. `None` until
/// the worker has published once — which, on a machine whose companion could not start, is
/// forever, and the panel says so.
pub fn recipes() -> Option<Vec<RecipeLine>> {
    crate::recipes::in_flight().map(|views| recipes_in_flight(&views, RECIPE_ROWS * 2))
}

/// Running, waiting and paused recipes, one line each, in the desk's order: the ones waiting on
/// the person first. `pending` is left out: that is the status a recipe *definition* carries
/// before it has ever run — every built-in template sits there — and none of them is in flight.
pub fn recipes_in_flight(views: &[yantrik_companion::recipe_view::RecipeView], cap: usize) -> Vec<RecipeLine> {
    views
        .iter()
        .filter(|v| yantrik_companion::recipe_view::is_in_flight(v))
        .take(cap)
        .map(recipe_line)
        .collect()
}

/// "step 3 of 7 · running web_search", from the stage the Recipes screen lights.
pub fn recipe_line(v: &yantrik_companion::recipe_view::RecipeView) -> RecipeLine {
    let total = v.steps.len();
    let step = match (crate::recipes::focus(v), &v.needs_you) {
        (_, Some(why)) if v.status != "paused" => format!("needs you: {}", one_line(why, 80)),
        (Some(s), _) => format!("step {} of {total} · {}", s.index + 1, step_words(v, s)),
        (None, _) if total == 0 => "no steps recorded".to_string(),
        (None, _) => format!("step {} of {total}", (v.current_step + 1).min(total)),
    };
    RecipeLine { id: v.id.clone(), name: v.name.clone(), step, status: v.status.clone(), needs_you: v.needs_you.is_some() }
}

/// What the lit stage is doing, in a few words.
pub fn step_words(v: &yantrik_companion::recipe_view::RecipeView, s: &yantrik_companion::recipe_view::StepView) -> String {
    let doing = match s.kind.as_str() {
        "tool" => format!("running {}", s.label),
        "think" => "thinking".into(),
        "think_cited" => "writing, with sources".into(),
        "jump_if" | "branch" => "deciding".into(),
        "wait_for" => format!("waiting for {}", v.waiting_for.as_deref().unwrap_or("its time")),
        "notify" => "telling you something".into(),
        "ask_user" => {
            let question = v.question.as_ref().map(|q| q.text.as_str()).unwrap_or(s.summary.as_str());
            format!("asking you: {}", one_line(question, 60))
        }
        "validate" => "checking its sources".into(),
        "render" => "laying out the result".into(),
        "format" => "formatting".into(),
        "filter" => "filtering".into(),
        "sort" => "sorting".into(),
        "aggregate" => "totalling".into(),
        "extract" => "extracting".into(),
        // An Agent step: its role at work, on its mind.
        "agent" if s.state == "waiting" => format!("{} at work", s.label),
        "agent" => format!("handing to {}", s.label),
        _ => s.label.clone(),
    };
    match (v.status.as_str(), s.state.as_str()) {
        ("paused", "waiting") => format!("paused while {doing}"),
        ("paused", _) => format!("paused before {}", s.label),
        _ => doing,
    }
}

fn one_line(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    let cut: String = flat.chars().take(max).collect();
    format!("{}…", cut.trim_end())
}

// ── Recent actions ──────────────────────────────────────────────────

/// One thing the mind did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Act {
    pub unix: u64,
    pub app: String,
    pub action: String,
    pub outcome: String,
}

/// One row of Recent actions.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ActLine {
    pub what: String,
    pub when: String,
    pub outcome: String,
}

/// The last `count` acts, newest first. THE source of Recent actions: today the mind audit — what
/// ran without anybody being asked — and, when it lands, the ledger (#148). Nothing else in the
/// panel knows where acts come from.
///
/// The audit's in-memory list starts empty with every shell, so the file is read back once and
/// merged under it: "what did it do while I was away" survives a restart.
pub fn recent_acts(count: usize) -> Vec<Act> {
    static SEED: OnceLock<Vec<Act>> = OnceLock::new();
    let seed = SEED.get_or_init(|| audit_file_tail(Path::new(&crate::mind_mode::audit_path()), AUDIT_SEED));
    let live: Vec<Act> = crate::mind_mode::recent(count)
        .into_iter()
        .map(|e| Act { unix: e.unix, app: e.app, action: e.action, outcome: e.outcome })
        .collect();
    merge_acts(seed, &live, count)
}

/// The two lists as one, each act once, newest first.
pub fn merge_acts(older: &[Act], newer: &[Act], count: usize) -> Vec<Act> {
    let mut all: Vec<Act> = Vec::with_capacity(older.len() + newer.len());
    for act in older.iter().chain(newer) {
        if !all.contains(act) {
            all.push(act.clone());
        }
    }
    all.sort_by(|a, b| b.unix.cmp(&a.unix));
    all.truncate(count);
    all
}

/// The last `count` readable lines of the audit file. A torn last line — the file is appended to
/// and fsynced, and a crash can still cut one — is skipped, not guessed at.
pub fn audit_file_tail(path: &Path, count: usize) -> Vec<Act> {
    let Ok(body) = std::fs::read_to_string(path) else { return Vec::new() };
    let mut acts: Vec<Act> = body.lines().rev().filter_map(parse_audit_line).take(count).collect();
    acts.reverse();
    acts
}

fn parse_audit_line(line: &str) -> Option<Act> {
    let v: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    Some(Act {
        unix: v.get("unix")?.as_u64()?,
        app: v.get("app")?.as_str()?.to_string(),
        action: v.get("action")?.as_str()?.to_string(),
        outcome: v.get("outcome").and_then(|o| o.as_str()).unwrap_or("").to_string(),
    })
}

/// The rows: `app.action`, how long ago, and how it went.
pub fn act_lines(acts: &[Act], now: u64, count: usize) -> Vec<ActLine> {
    let mut sorted: Vec<&Act> = acts.iter().collect();
    sorted.sort_by(|a, b| b.unix.cmp(&a.unix));
    sorted
        .into_iter()
        .take(count)
        .map(|a| ActLine {
            what: format!("{}.{}", a.app, a.action),
            when: ago(now, a.unix),
            outcome: if a.outcome.trim().is_empty() { "not reported".into() } else { a.outcome.trim().to_string() },
        })
        .collect()
}

/// "just now", "4m ago", "3h ago", "2d ago". A time ahead of the clock reads as "just now".
pub fn ago(now: u64, then: u64) -> String {
    let secs = now.saturating_sub(then);
    match secs {
        0..=59 => "just now".into(),
        60..=3599 => format!("{}m ago", secs / 60),
        3600..=86_399 => format!("{}h ago", secs / 3600),
        _ => format!("{}d ago", secs / 86_400),
    }
}

// ── The whole panel ─────────────────────────────────────────────────

/// Everything the panel is built from, read at one moment.
#[derive(Clone, Debug, Default)]
pub struct Inputs {
    /// `None` until the harness host is up.
    pub minds: Option<MindsSeen>,
    /// The built-in's configured model (`ai-active-provider-label`); empty when none is configured.
    pub provider_label: String,
    pub companion_online: bool,
    pub companion_status: String,
    /// A reply is being written right now, by whichever mind is answering.
    pub generating: bool,
    pub project: String,
    /// `None` until the companion has counted.
    pub memory_count: Option<i64>,
    pub working: Working,
    /// `None` until the companion's worker has read them.
    pub recipes: Option<Vec<RecipeLine>>,
    pub acts: Vec<ActLine>,
    /// Each service's id and status, as the shell's service manager has them.
    pub services: Vec<(String, String)>,
}

/// The single values: Now and Context, and the counts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Now {
    pub known: bool,
    pub mind: String,
    pub initial: String,
    pub builtin: bool,
    pub model: String,
    pub model_named: bool,
    pub state: String,
    pub project: String,
    pub memory: String,
    pub memory_known: bool,
    pub minds: String,
    pub running: usize,
    pub needs_you: usize,
    pub more_agents: usize,
    pub recipes_known: bool,
    pub services: String,
    pub services_trouble: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Panel {
    pub now: Now,
    pub agents: Vec<AgentLine>,
    pub recipes: Vec<RecipeLine>,
    pub acts: Vec<ActLine>,
}

/// What the panel says, from what was read.
pub fn build(input: &Inputs) -> Panel {
    let (known, mind, initial, builtin, model, model_named, state) = match &input.minds {
        None => (false, "unknown".to_string(), "?".to_string(), false, "unknown".to_string(), false, "unknown".to_string()),
        Some(seen) => match &seen.answering {
            None => (true, "no mind".to_string(), "?".to_string(), false, "nothing is answering".to_string(), false, "none".to_string()),
            Some(m) => {
                let initial = m
                    .name
                    .chars()
                    .find(|c| c.is_alphanumeric())
                    .map(|c| c.to_uppercase().to_string())
                    .unwrap_or_else(|| "?".into());
                if m.builtin {
                    // The built-in runs on what this shell is configured with.
                    let label = input.provider_label.trim();
                    let (model, named) = if label.is_empty() {
                        ("no model configured".to_string(), false)
                    } else {
                        (label.to_string(), true)
                    };
                    let state = if !input.companion_online {
                        "offline"
                    } else if input.generating || input.companion_status == "thinking" {
                        "thinking"
                    } else {
                        "ready"
                    };
                    (true, m.name.clone(), initial, true, model, named, state.to_string())
                } else {
                    // An attached mind runs on whatever it said it runs on — and never on the
                    // shell's configured model, which is not doing the work.
                    let detail = m.detail.as_deref().map(str::trim).filter(|d| !d.is_empty());
                    let (model, named) = match detail {
                        Some(d) => (d.to_string(), true),
                        None => ("model not reported".to_string(), false),
                    };
                    let state = if input.generating { "thinking" } else { "attached" };
                    (true, m.name.clone(), initial, false, model, named, state.to_string())
                }
            }
        },
    };

    let (memory, memory_known) = match input.memory_count {
        Some(n) => (format!("YantrikDB · {} {}", thousands(n), if n == 1 { "memory" } else { "memories" }), true),
        None => ("YantrikDB · count not known yet".to_string(), false),
    };
    let minds = match &input.minds {
        None => "not known yet".to_string(),
        Some(seen) => format!(
            "{} {} · {} {} memory",
            seen.count,
            if seen.count == 1 { "mind" } else { "minds" },
            seen.keep_memory,
            if seen.keep_memory == 1 { "keeps" } else { "keep" },
        ),
    };
    let (services, services_trouble) = services_line(&input.services);

    let recipes: Vec<RecipeLine> = input.recipes.clone().unwrap_or_default().into_iter().take(RECIPE_ROWS).collect();
    Panel {
        now: Now {
            known,
            mind,
            initial,
            builtin,
            model,
            model_named,
            state,
            project: input.project.trim().to_string(),
            memory,
            memory_known,
            minds,
            running: input.working.running,
            // Agents waiting on the person, and recipes whose agents are.
            needs_you: input.working.needs_you + recipes.iter().filter(|r| r.needs_you).count(),
            more_agents: input.working.more,
            recipes_known: input.recipes.is_some(),
            services,
            services_trouble,
        },
        agents: input.working.rows.clone(),
        recipes,
        acts: input.acts.clone(),
    }
}

/// "6 running", "5 running · 1 stopped · perception failed". Failure is trouble; a stopped
/// service is not — most of them start on demand.
pub fn services_line(services: &[(String, String)]) -> (String, bool) {
    if services.is_empty() {
        return ("none registered".into(), false);
    }
    let count = |status: &str| services.iter().filter(|(_, s)| s == status).count();
    let failed: Vec<&str> = services.iter().filter(|(_, s)| s == "failed").map(|(id, _)| id.as_str()).collect();
    let mut parts = vec![format!("{} running", count("running"))];
    if count("starting") > 0 {
        parts.push(format!("{} starting", count("starting")));
    }
    if count("stopped") > 0 {
        parts.push(format!("{} stopped", count("stopped")));
    }
    if !failed.is_empty() {
        parts.push(format!("{} failed", failed.join(", ")));
    }
    (parts.join(" · "), !failed.is_empty())
}

fn thousands(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 {
        format!("-{out}")
    } else {
        out
    }
}

// ── describe shell, and set_mind_panel ─────────────────────────────

/// What `describe shell` says under `mind_panel`: where it is, whether it is open, what each place
/// is set to, and how much it is showing.
pub fn describe_of(choice: Choice, screen: i32, focus_mode: bool, now: &Now, recipes: usize, acts: usize) -> serde_json::Value {
    let shown = shown_on(screen);
    let expanded = open_on(choice, screen, focus_mode);
    serde_json::json!({
        "shown": shown,
        "where": Place::of_screen(screen).key(),
        "expanded": expanded,
        "state": if !shown { "hidden" } else if expanded { "expanded" } else { "collapsed" },
        // What `set_mind_panel` changes: one choice per place, kept across restarts.
        "choice": { "desktop": choice.desktop, "elsewhere": choice.elsewhere },
        "focus_mode_holds_it_collapsed": screen == DESKTOP_SCREEN && focus_mode && choice.desktop,
        "mind": if now.known { serde_json::json!(now.mind) } else { serde_json::Value::Null },
        "model": if now.known { serde_json::json!(now.model) } else { serde_json::Value::Null },
        "at_work": now.running,
        "needs_you": now.needs_you,
        "recipes_in_flight": if now.recipes_known { serde_json::json!(recipes) } else { serde_json::Value::Null },
        "recent_actions": acts,
        "recent_actions_source": "mind_audit",
    })
}

fn choice_of(g: &MindPanelState<'_>) -> Choice {
    Choice { desktop: g.get_desktop_expanded(), elsewhere: g.get_elsewhere_expanded() }
}

fn show_choice(g: &MindPanelState<'_>, choice: Choice) {
    if g.get_desktop_expanded() != choice.desktop {
        g.set_desktop_expanded(choice.desktop);
    }
    if g.get_elsewhere_expanded() != choice.elsewhere {
        g.set_elsewhere_expanded(choice.elsewhere);
    }
}

/// For `describe shell`.
pub fn for_describe(ui: &App) -> serde_json::Value {
    let g = ui.global::<MindPanelState>();
    let now = g.get_now();
    let now = Now {
        known: now.known,
        mind: now.mind.to_string(),
        model: now.model.to_string(),
        running: now.running.max(0) as usize,
        needs_you: now.needs_you.max(0) as usize,
        recipes_known: now.recipes_known,
        ..Now::default()
    };
    describe_of(
        choice_of(&g),
        ui.get_current_screen(),
        ui.get_focus_mode(),
        &now,
        g.get_recipes().row_count(),
        g.get_acts().row_count(),
    )
}

/// `set_mind_panel`: open or fold the panel at `place`, and keep it that way. The file first and
/// the screen after, so an error means nothing changed.
pub fn set(ui: &App, place: Place, expanded: bool) -> Result<(), String> {
    let g = ui.global::<MindPanelState>();
    let mut choice = choice_of(&g);
    choice.set(place, expanded);
    save_choice(&choice_path(), choice)?;
    show_choice(&g, choice);
    Ok(())
}

// ── Wiring ──────────────────────────────────────────────────────────

pub fn wire(ui: &App) {
    let g = ui.global::<MindPanelState>();
    show_choice(&g, load_choice(&choice_path()));

    // The chevron and the strip. A click shows at once; the file follows, and a file that cannot
    // be written is logged rather than undoing what the person just did.
    g.on_set_expanded({
        let weak = ui.as_weak();
        move |place, expanded| {
            let Some(ui) = weak.upgrade() else { return };
            let Some(place) = Place::parse(&place) else { return };
            let g = ui.global::<MindPanelState>();
            let mut choice = choice_of(&g);
            choice.set(place, expanded);
            show_choice(&g, choice);
            if let Err(e) = save_choice(&choice_path(), choice) {
                tracing::warn!(error = %e, "The mind panel's choice was not saved");
            }
        }
    });

    refresh(ui);
    let timer = Timer::default();
    {
        let weak = ui.as_weak();
        timer.start(TimerMode::Repeated, TICK, move || {
            if let Some(ui) = weak.upgrade() {
                refresh(&ui);
            }
        });
    }
    // The timer lives as long as the shell, the idiom every wire module uses.
    std::mem::forget(timer);
}

/// Read everything once and hand the panel what changed.
fn refresh(ui: &App) {
    let now = agents::model::now();
    let services: Vec<(String, String)> = {
        let model = ui.get_services();
        (0..model.row_count())
            .filter_map(|i| model.row_data(i))
            .map(|s| (s.id.to_string(), s.status.to_string()))
            .collect()
    };
    let input = Inputs {
        minds: minds_seen(),
        provider_label: ui.get_ai_active_provider_label().to_string(),
        companion_online: ui.get_companion_online(),
        companion_status: ui.get_companion_status().to_string(),
        generating: ui.get_is_generating(),
        project: ui.get_active_project().to_string(),
        memory_count: WORKER_UP.load(Ordering::Relaxed).then(|| ui.get_memory_count() as i64),
        working: agents::store().read(|s| working(s, now, AGENT_ROWS)),
        recipes: recipes(),
        acts: act_lines(&recent_acts(ACT_ROWS), now, ACT_ROWS),
        services,
    };
    publish(ui, &build(&input));
}

fn publish(ui: &App, panel: &Panel) {
    let g = ui.global::<MindPanelState>();
    let n = &panel.now;
    let now = MindPanelNow {
        known: n.known,
        mind: n.mind.as_str().into(),
        initial: n.initial.as_str().into(),
        builtin: n.builtin,
        model: n.model.as_str().into(),
        model_named: n.model_named,
        state: n.state.as_str().into(),
        project: n.project.as_str().into(),
        memory: n.memory.as_str().into(),
        memory_known: n.memory_known,
        minds: n.minds.as_str().into(),
        running: n.running as i32,
        needs_you: n.needs_you as i32,
        more_agents: n.more_agents as i32,
        recipes_known: n.recipes_known,
        services: n.services.as_str().into(),
        services_trouble: n.services_trouble,
    };
    if g.get_now() != now {
        g.set_now(now);
    }
    let agents: Vec<MindPanelAgent> = panel
        .agents
        .iter()
        .map(|a| MindPanelAgent {
            id: a.id.as_str().into(),
            mind: a.mind.as_str().into(),
            title: a.title.as_str().into(),
            state: a.state.as_str().into(),
            label: a.label.as_str().into(),
            since: a.since.as_str().into(),
        })
        .collect();
    if let Some(model) = crate::models::changed(g.get_agents(), agents) {
        g.set_agents(model);
    }
    let recipes: Vec<MindPanelRecipe> = panel
        .recipes
        .iter()
        .map(|r| MindPanelRecipe {
            id: r.id.as_str().into(),
            name: r.name.as_str().into(),
            step: r.step.as_str().into(),
            status: r.status.as_str().into(),
        })
        .collect();
    if let Some(model) = crate::models::changed(g.get_recipes(), recipes) {
        g.set_recipes(model);
    }
    let acts: Vec<MindPanelAct> = panel
        .acts
        .iter()
        .map(|a| MindPanelAct { what: a.what.as_str().into(), when: a.when.as_str().into(), outcome: a.outcome.as_str().into() })
        .collect();
    if let Some(model) = crate::models::changed(g.get_acts(), acts) {
        g.set_acts(model);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::{AgentId, AgentMeta};
    use std::sync::Arc;
    use std::sync::atomic::AtomicU64;

    /// A store whose clock the test moves.
    fn store_at(clock: &Arc<AtomicU64>) -> Store {
        let c = clock.clone();
        Store::with_clock(Box::new(move || c.load(Ordering::Relaxed)))
    }

    fn agent(store: &mut Store, id: &str, mind: &str, prompt: &str) -> AgentId {
        let id = AgentId(id.to_string());
        store.upsert_agent(AgentMeta::new(id.clone(), mind));
        store.open_turn(&id, prompt);
        id
    }

    fn input() -> Inputs {
        Inputs {
            minds: Some(MindsSeen {
                answering: Some(Mind { name: "Yantrik Companion".into(), builtin: true, detail: None }),
                count: 3,
                keep_memory: 2,
            }),
            provider_label: "qwen3.5:9b".into(),
            companion_online: true,
            companion_status: "idle".into(),
            memory_count: Some(1234),
            recipes: Some(Vec::new()),
            ..Inputs::default()
        }
    }

    #[test]
    fn now_says_who_is_answering_and_on_what() {
        let panel = build(&input());
        assert_eq!(panel.now.mind, "Yantrik Companion");
        assert!(panel.now.builtin);
        assert_eq!(panel.now.model, "qwen3.5:9b", "the built-in runs on the shell's configured model");
        assert!(panel.now.model_named);
        assert_eq!(panel.now.state, "ready");
        assert_eq!(panel.now.initial, "Y");

        // An attached mind runs on what it said it runs on, never on the shell's own setting.
        let mut hermes = input();
        hermes.minds.as_mut().unwrap().answering =
            Some(Mind { name: "hermes".into(), builtin: false, detail: Some("gpt-6 on node1".into()) });
        let panel = build(&hermes);
        assert_eq!(panel.now.model, "gpt-6 on node1");
        assert_eq!(panel.now.state, "attached");
        assert_eq!(panel.now.initial, "H");

        // One that said nothing about its model is not given the shell's.
        hermes.minds.as_mut().unwrap().answering.as_mut().unwrap().detail = None;
        let panel = build(&hermes);
        assert_eq!(panel.now.model, "model not reported");
        assert!(!panel.now.model_named);
        assert!(!panel.now.model.contains("qwen"), "the shell's setting is not the attached mind's model");

        // Writing a reply, whoever is answering.
        hermes.generating = true;
        assert_eq!(build(&hermes).now.state, "thinking");

        let mut offline = input();
        offline.companion_online = false;
        assert_eq!(build(&offline).now.state, "offline");
    }

    #[test]
    fn what_is_not_known_says_so_and_is_never_a_zero() {
        let unknown = Inputs::default();
        let panel = build(&unknown);
        assert!(!panel.now.known, "no host yet: the mind is not known");
        assert_eq!(panel.now.mind, "unknown");
        assert_eq!(panel.now.minds, "not known yet");
        assert!(!panel.now.memory_known);
        assert_eq!(panel.now.memory, "YantrikDB · count not known yet", "not \"0 memories\"");
        assert!(!panel.now.recipes_known, "no read from the companion yet: recipes are not known, not none");
        assert_eq!(panel.now.project, "", "no project detected is drawn as such by the panel, not invented");

        let known = build(&input());
        assert_eq!(known.now.memory, "YantrikDB · 1,234 memories");
        assert_eq!(known.now.minds, "3 minds · 2 keep memory");
        assert!(known.now.recipes_known);

        let mut one = input();
        one.memory_count = Some(1);
        one.minds = Some(MindsSeen { answering: None, count: 1, keep_memory: 1 });
        let panel = build(&one);
        assert_eq!(panel.now.memory, "YantrikDB · 1 memory");
        assert_eq!(panel.now.minds, "1 mind · 1 keeps memory");
        assert_eq!(panel.now.mind, "no mind");
        assert_eq!(panel.now.model, "nothing is answering");

        let mut unconfigured = input();
        unconfigured.provider_label = "  ".into();
        let panel = build(&unconfigured);
        assert_eq!(panel.now.model, "no model configured");
        assert!(!panel.now.model_named);
    }

    #[test]
    fn agents_waiting_for_the_person_come_first_and_idle_ones_are_not_at_work() {
        let clock = Arc::new(AtomicU64::new(1_000));
        let mut s = store_at(&clock);
        let waiting = agent(&mut s, "deepseek:main", "DeepSeek", "release notes for 0.4");
        clock.store(1_010, Ordering::Relaxed);
        let running = agent(&mut s, "pi:main", "pi", "tidy the photos folder");
        clock.store(1_020, Ordering::Relaxed);
        let idle = AgentId("hermes:c-1".into());
        s.upsert_agent(AgentMeta::new(idle.clone(), "hermes"));
        clock.store(1_030, Ordering::Relaxed);
        let done = agent(&mut s, "pi:c-2", "pi", "count the files");
        s.close_turn(&done, true);
        clock.store(1_040, Ordering::Relaxed);
        s.set_state(&waiting, State::WaitingForYou);

        let w = working(&s, 1_160, AGENT_ROWS);
        let ids: Vec<&str> = w.rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["deepseek:main", "pi:main"], "needs-you first, then the rest newest first; idle and done are not at work");
        assert_eq!(w.running, 2);
        assert_eq!(w.needs_you, 1);
        assert_eq!(w.more, 0);
        assert_eq!(w.rows[0].state, "waiting_for_you");
        assert_eq!(w.rows[0].label, "waiting for you");
        assert_eq!(w.rows[0].since, "2m", "since its state changed, in the Agents screen's words");
        assert_eq!(w.rows[1].title, "tidy the photos folder");
        let _ = running;

        // More at work than fit: listed up to the cap, the rest counted.
        for n in 0..5 {
            agent(&mut s, &format!("pi:c-{}", 10 + n), "pi", "another");
        }
        let w = working(&s, 1_160, AGENT_ROWS);
        assert_eq!(w.rows.len(), AGENT_ROWS);
        assert_eq!(w.running, 7);
        assert_eq!(w.more, 3);
        assert_eq!(w.rows[0].id, "deepseek:main", "the one waiting on the person is never pushed off the list");
    }

    #[test]
    fn recipes_in_flight_are_the_recipes_screen_s_own_one_line_each() {
        use yantrik_companion::recipe::{RecipeStatus, RecipeStep, RecipeStore, WaitCondition};
        use yantrik_companion::recipe_view;
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        RecipeStore::ensure_tables(&conn);
        let tool = |name: &str| RecipeStep::Tool {
            tool_name: name.into(),
            args: serde_json::json!({}),
            store_as: "x".into(),
            on_error: Default::default(),
        };
        // Each where the executors leave it: a wait or a question marked, and the pointer past it.
        let recipe = |name: &str, steps: &[RecipeStep], status: RecipeStatus, at: usize| {
            let id = RecipeStore::create(&conn, name, "", steps, None);
            for done in 0..at.min(steps.len()) {
                RecipeStore::complete_step(&conn, &id, done, "asked");
            }
            if status != RecipeStatus::Pending {
                RecipeStore::update_status(&conn, &id, &status, at);
            }
            id
        };
        let think = RecipeStep::Think { prompt: "sum up".into(), store_as: "y".into(), fallback_template: None };
        recipe("Morning digest", &[tool("calendar_today"), tool("web_search"), think], RecipeStatus::Running, 1);
        let ask = RecipeStep::AskUser { question: "Which days are you travelling?".into(), store_as: "days".into(), choices: None };
        recipe("Plan the week", &[ask, tool("book")], RecipeStatus::Waiting, 1);
        let wait = RecipeStep::WaitFor { condition: WaitCondition::Time { hour: 17, minute: 5 }, timeout_secs: None };
        recipe("Remind me", &[wait, tool("notify")], RecipeStatus::Waiting, 1);
        let held = recipe("Weekly backup", &[tool("disk_usage"), tool("run_command")], RecipeStatus::Running, 1);
        RecipeStore::pause(&conn, &held).unwrap();
        // A definition that has never run, and one that finished: neither is in flight.
        recipe("Built-in template", &[tool("noop")], RecipeStatus::Pending, 0);
        recipe("Done already", &[tool("noop")], RecipeStatus::Done, 1);

        let lines = recipes_in_flight(&recipe_view::list(&conn), 10);
        let names: Vec<&str> = lines.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(lines.len(), 4, "running, waiting and paused only: {names:?}");
        assert_eq!(names[0], "Plan the week", "the one waiting on the person first: {names:?}");
        let line = |name: &str| lines.iter().find(|l| l.name == name).unwrap().clone();
        assert_eq!(line("Morning digest").step, "step 2 of 3 · running web_search");
        assert_eq!(line("Morning digest").status, "running");
        // The question it waits on, not the step after it that the pointer names.
        assert_eq!(line("Plan the week").step, "step 1 of 2 · asking you: Which days are you travelling?");
        assert_eq!(line("Plan the week").status, "waiting");
        assert_eq!(line("Remind me").step, "step 1 of 2 · waiting for 17:05");
        assert_eq!(line("Weekly backup").step, "step 2 of 2 · paused before run_command");
        assert_eq!(line("Weekly backup").status, "paused");
        assert_eq!(recipes_in_flight(&recipe_view::list(&conn), 2).len(), 2, "capped");

        // A store without the tables is an empty list, never a panic on the companion's worker.
        let bare = rusqlite::Connection::open_in_memory().unwrap();
        assert!(recipe_view::list(&bare).is_empty());
    }

    #[test]
    fn recent_actions_are_newest_first_with_how_long_ago_and_how_it_went() {
        let act = |unix: u64, app: &str, action: &str, outcome: &str| Act {
            unix,
            app: app.into(),
            action: action.into(),
            outcome: outcome.into(),
        };
        let now = 1_000_000;
        let lines = act_lines(
            &[
                act(now - 7_300, "notes", "create", "ok"),
                act(now - 240, "files", "move", "failed"),
                act(now - 20, "calendar", "add_event", ""),
                act(now - 200_000, "email", "send", "ok"),
                act(now - 60, "files", "copy", "ok"),
            ],
            now,
            ACT_ROWS,
        );
        let rows: Vec<(&str, &str, &str)> = lines.iter().map(|l| (l.what.as_str(), l.when.as_str(), l.outcome.as_str())).collect();
        assert_eq!(
            rows,
            [
                ("calendar.add_event", "just now", "not reported"),
                ("files.copy", "1m ago", "ok"),
                ("files.move", "4m ago", "failed"),
                ("notes.create", "2h ago", "ok"),
            ]
        );
        assert_eq!(ago(now, now + 30), "just now", "a clock that disagrees is not a negative age");
        assert_eq!(ago(now, now - 200_000), "2d ago");
    }

    #[test]
    fn the_audit_file_is_read_back_after_a_restart_and_merged_once() {
        let dir = std::env::temp_dir().join(format!("mind-panel-audit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mind-audit.jsonl");
        let line = |unix: u64, app: &str, action: &str, outcome: &str| {
            serde_json::json!({"at": "10:00", "unix": unix, "mode": "auto", "requester": "pi", "verified": {},
                               "app": app, "action": action, "args": [], "grade": "standard", "outcome": outcome})
            .to_string()
        };
        std::fs::write(
            &path,
            format!(
                "{}\n{}\n{}\n{{\"unix\": 9, \"app\": \"torn",
                line(1, "notes", "create", "ok"),
                line(2, "files", "move", "ok"),
                line(3, "files", "copy", "failed"),
            ),
        )
        .unwrap();
        let seed = audit_file_tail(&path, 2);
        assert_eq!(seed.len(), 2, "the last two readable lines; the torn one is skipped: {seed:?}");
        assert_eq!((seed[0].unix, seed[1].unix), (2, 3));

        // What the shell recorded since starting is merged in, and an act in both is listed once.
        let live = vec![seed[1].clone(), Act { unix: 4, app: "email".into(), action: "draft".into(), outcome: "ok".into() }];
        let merged = merge_acts(&seed, &live, 10);
        let order: Vec<u64> = merged.iter().map(|a| a.unix).collect();
        assert_eq!(order, [4, 3, 2]);

        assert!(audit_file_tail(&dir.join("absent.jsonl"), 5).is_empty(), "no file is no acts, not an error");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn services_are_one_line_and_only_a_failure_is_trouble() {
        let s = |id: &str, status: &str| (id.to_string(), status.to_string());
        assert_eq!(services_line(&[]), ("none registered".into(), false));
        assert_eq!(services_line(&[s("network", "running"), s("notes", "stopped")]), ("1 running · 1 stopped".into(), false));
        assert_eq!(
            services_line(&[s("network", "running"), s("perception", "failed"), s("audio", "starting")]),
            ("1 running · 1 starting · perception failed".into(), true)
        );
    }

    #[test]
    fn the_choice_is_kept_per_place_and_survives_a_restart() {
        let dir = std::env::temp_dir().join(format!("mind-panel-choice-{}", std::process::id()));
        let path = dir.join("mind-panel.json");
        let _ = std::fs::remove_dir_all(&dir);

        // Nothing saved: open on the desktop, the strip everywhere else.
        let choice = load_choice(&path);
        assert_eq!(choice, Choice { desktop: true, elsewhere: false });
        assert!(open_on(choice, 1, false), "the desktop opens it");
        assert!(!open_on(choice, 8, false), "Files gets the strip");
        assert!(!open_on(choice, 34, false), "so does Agents");
        assert!(!open_on(choice, 1, true), "focus mode keeps it to the strip");
        assert!(!open_on(choice, 3, false) && !shown_on(3), "not on the lock screen at all");

        let mut chosen = choice;
        chosen.set(Place::Elsewhere, true);
        chosen.set(Place::Desktop, false);
        save_choice(&path, chosen).unwrap();
        let back = load_choice(&path);
        assert_eq!(back, Choice { desktop: false, elsewhere: true });
        assert!(open_on(back, 8, false) && !open_on(back, 1, false));

        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(load_choice(&path), Choice::default(), "a file that cannot be read is the defaults, not a crash");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn describe_says_where_it_is_how_open_and_what_each_place_is_set_to() {
        let panel = build(&input());
        let d = describe_of(Choice::default(), 1, false, &panel.now, 0, 2);
        assert_eq!(d["where"], "desktop");
        assert_eq!(d["expanded"], true);
        assert_eq!(d["state"], "expanded");
        assert_eq!(d["choice"]["elsewhere"], false);
        assert_eq!(d["mind"], "Yantrik Companion");
        assert_eq!(d["recipes_in_flight"], 0);
        assert_eq!(d["recent_actions"], 2);

        let d = describe_of(Choice::default(), 8, false, &panel.now, 0, 0);
        assert_eq!((d["where"].as_str(), d["state"].as_str()), (Some("elsewhere"), Some("collapsed")));
        let d = describe_of(Choice::default(), 1, true, &panel.now, 0, 0);
        assert_eq!(d["expanded"], false);
        assert_eq!(d["focus_mode_holds_it_collapsed"], true);
        let d = describe_of(Choice::default(), 0, false, &build(&Inputs::default()).now, 0, 0);
        assert_eq!(d["state"], "hidden", "boot has no panel");
        assert!(d["mind"].is_null() && d["recipes_in_flight"].is_null(), "unknown is null, not a guess");

        assert_eq!(Place::parse("desktop"), Some(Place::Desktop));
        assert_eq!(Place::parse(" elsewhere "), Some(Place::Elsewhere));
        assert_eq!(Place::parse("everywhere"), None);
    }

    fn read(relative: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
    }

    /// The recurring bug: furniture declared in desktop.slint exists on one screen while its room is
    /// reserved on all of them. The panel is declared in app.slint, on the taskbar's screens, and
    /// every screen it is drawn over gives it room.
    #[test]
    fn the_panel_is_shell_furniture_declared_in_app_slint() {
        let app = read("../yantrik-ui-slint/ui/app.slint");
        let desktop = read("../yantrik-ui-slint/ui/desktop.slint");
        assert!(app.contains("if root.mind-panel-shown : MindPanel {"), "app.slint instantiates the panel");
        assert!(!desktop.contains("MindPanel {") && !desktop.contains("MachineRail"), "and desktop.slint does not");
        assert!(
            !Path::new(env!("CARGO_MANIFEST_DIR")).join("../yantrik-ui-slint/ui/components/machine_rail.slint").exists(),
            "the rail the panel replaced is gone, not left defined and unused"
        );

        // The panel is drawn on the taskbar's screens, and Rust agrees about which those are.
        let shown = app.split("property <bool> mind-panel-shown:").nth(1).expect("mind-panel-shown").split(';').next().unwrap().to_string();
        for part in ["current-screen == 1", "current-screen >= 4", "current-screen <= 31", "current-screen == 33", "current-screen == 34", "current-screen == 35"] {
            assert!(shown.contains(part), "mind-panel-shown covers `{part}`: {shown}");
        }
        assert!(app.contains("current-screen == 1 || (current-screen >= 4 && current-screen <= 31) || current-screen == 33 || current-screen == 34 || current-screen == 35 : Rectangle"), "the taskbar's own condition is the one mirrored");
        for screen in [1, 4, 8, 31, 33, 34, 35] {
            assert!(shown_on(screen), "screen {screen}");
        }
        for screen in [0, 2, 3, 32] {
            assert!(!shown_on(screen), "screen {screen}");
        }

        // Its room: every maximized shell window stops short of it, and the desktop is inset by it.
        // As many as there are framed screens, counted rather than typed: this said `>= 16` and
        // failed the day three screens became windows of their own (#253).
        let framed = app.lines().filter(|l| l.trim_start().starts_with("if current-screen == ") && l.contains(": WindowFrame {")).count();
        let maximized: Vec<&str> = app.lines().filter(|l| l.contains("width: root.window-maximized ?")).collect();
        assert!(framed >= 10, "found only {framed} framed screens; the reader is broken");
        assert!(maximized.len() >= framed, "found {} maximized widths for {framed} framed screens", maximized.len());
        for line in &maximized {
            assert!(line.contains("parent.width - root.mind-panel-reserve"), "a maximized window runs under the panel: {line}");
        }
        assert!(app.contains("right-inset: root.mind-panel-reserve;"), "the desktop is told the panel's room");
        assert!(desktop.contains("in property <length> right-inset"), "and the desktop takes it");

        // Drawn after the taskbar (over the screens) and before the approval card (under it).
        let panel_at = app.find("if root.mind-panel-shown : MindPanel {").unwrap();
        assert!(app.find("Taskbar {").unwrap() < panel_at);
        assert!(panel_at < app.find("for item in root.pending-approvals : ApprovalCard").unwrap());
    }

    /// `describe shell` reads it and `set_mind_panel` sets it, graded safe: it only changes how much
    /// of a panel is drawn.
    #[test]
    fn describe_shell_reads_the_panel_and_a_safe_action_sets_it() {
        let src = read("src/control.rs");
        let src = src.split("#[cfg(test)]").next().unwrap();
        assert!(src.contains(".with(\"mind_panel\", crate::mind_panel::for_describe(&ui))"), "describe shell has `mind_panel`");
        let from = src.find("\"set_mind_panel\"").expect("the shell publishes set_mind_panel");
        let rest = &src[from..];
        let action = &rest[..rest.find(".action(").unwrap_or(rest.len())];
        assert!(action.contains(".risk(\"safe\")"), "graded safe:\n{action}");
        assert!(action.contains("Param::flag(\"expanded\")"), "takes `expanded`:\n{action}");
        assert!(action.contains("crate::mind_panel::set("), "sets through the same path as the chevron:\n{action}");
        assert!(action.contains("crate::mind_panel::for_describe(&ui)"), "answers with what it now is:\n{action}");
    }
}
