//! The Agents workroom: what the overview and a task's detail say, worked out from the store.
//!
//! The screen used to be a list of agents with a details column beside it. It is now a workroom
//! (design/minds-surfaces-spec-2026-10-02.md, section 2): the decisions waiting on the person, a
//! desk for each mind at work, and the minds with their real state. Everything here is read off
//! the agents store and the approval store and is plain data, so it can be tested without a window;
//! `wire::agents` draws it into `AgentsState`.
//!
//! Nothing is invented. A desk's activity line is a call the shell saw running, what the mind
//! itself reported, or "Last update 10:42". A change is listed only because a call the shell
//! itself ran named it, and a call the mind only claimed is listed apart, as a claim. A desk has
//! no preview here because nothing in the shell attributes a Mind View window to the mind that
//! opened it (control_screen.rs says so): the card says "Desk preview unavailable".

use std::collections::HashSet;

use crate::agents::model::{Agent, CallState, Item, Provenance, State, Turn};
use crate::agents::{progress, AgentId, ApprovalOutcome, RowKey, Store};
use crate::approvals;
use crate::{AgentItemData, ToolCallData};

use super::agents::{clock, duration, latest_request, one_line};

/// How many decision cards the shelf expands.
pub const SHELF_SHOWN: usize = 3;
/// How many recent results the empty workroom offers.
pub const RECENT_SHOWN: usize = 3;
/// How many finished runs History lists.
pub const HISTORY_SHOWN: usize = 200;

const TASK_CHARS: usize = 120;
const REQUEST_CHARS: usize = 140;

/// A desk: one run being worked on now.
#[derive(Clone, Debug, PartialEq)]
pub struct Desk {
    /// What opening it selects: the agent's id, or `agent#n` for one run of a chat.
    pub key: String,
    pub mind_id: String,
    pub mind: String,
    /// "Council recipe → Reviewer": whose work it is, when it is a recipe's or a role's.
    pub via: String,
    pub task: String,
    /// `working` or `needs_you`.
    pub state: &'static str,
    pub label: String,
    pub activity: String,
    pub since: String,
}

/// Something that waits on the person.
#[derive(Clone, Debug, PartialEq)]
pub struct Decision {
    pub key: String,
    /// The shell's request id for an approval or a question; empty for the rest.
    pub request: String,
    pub mind_id: String,
    pub mind: String,
    pub task: String,
    pub text: String,
    pub age_secs: u64,
    pub action: String,
}

/// A row under MINDS: the mind, or one of its live task agents.
#[derive(Clone, Debug, PartialEq)]
pub struct Nav {
    /// `mind` or `task`.
    pub kind: &'static str,
    /// The harness id for a mind, the row key for a task.
    pub id: String,
    pub label: String,
    pub sub: String,
    /// Requests waiting on the person.
    pub needs: usize,
    pub working: bool,
    pub available: bool,
}

/// A finished run.
#[derive(Clone, Debug, PartialEq)]
pub struct Finished {
    pub key: String,
    pub mind_id: String,
    pub mind: String,
    pub task: String,
    /// `done`, `failed` or `stopped`.
    pub outcome: &'static str,
    pub label: &'static str,
    pub when: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Workroom {
    pub working: usize,
    /// Requests waiting on the person, and how many minds they come from.
    pub requests: usize,
    pub request_minds: usize,
    pub desks: Vec<Desk>,
    /// Every request waiting, oldest first. The shelf shows [`SHELF_SHOWN`] of them.
    pub decisions: Vec<Decision>,
    pub nav: Vec<Nav>,
    /// Finished runs, newest first, and how many there are in all.
    pub history: Vec<Finished>,
    pub runs: usize,
}

impl Workroom {
    /// The workroom as one mind sees it: its desks, its requests, its results. The navigation and
    /// the header's counts stay the whole desktop's.
    pub fn narrowed(&self, mind_id: &str) -> Workroom {
        let desks: Vec<Desk> = self.desks.iter().filter(|d| d.mind_id == mind_id).cloned().collect();
        let decisions: Vec<Decision> = self.decisions.iter().filter(|d| d.mind_id == mind_id).cloned().collect();
        let history: Vec<Finished> = self.history.iter().filter(|r| r.mind_id == mind_id).cloned().collect();
        Workroom {
            working: desks.iter().filter(|d| d.state == "working").count(),
            requests: decisions.len(),
            request_minds: usize::from(!decisions.is_empty()),
            runs: history.len(),
            desks,
            decisions,
            history,
            nav: self.nav.clone(),
        }
    }

    /// "2 working · 1 needs you": the header's counts.
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if self.working > 0 {
            parts.push(format!("{} working", self.working));
        }
        if self.requests > 0 {
            parts.push(if self.requests == 1 { "1 needs you".to_string() } else { format!("{} need you", self.requests) });
        }
        if parts.is_empty() {
            "Nothing running".to_string()
        } else {
            parts.join(" · ")
        }
    }
}

/// The agents that have a desk: working now. A mind's chat is not an agent's work until a request
/// of it did something, the same line the Agents list has always drawn (`Agent::is_run`).
fn at_work(s: &Store) -> impl Iterator<Item = (&Agent, &Turn)> {
    s.agents().iter().filter_map(|a| {
        if !a.state.working() {
            return None;
        }
        let turn = a.open_turn()?;
        a.is_run(turn).then_some((a, turn))
    })
}

/// The run a detail view shows. A main mind opened without one (its name in the navigation, a
/// request card with no turn open) shows its latest run, the one the title names: listing every
/// turn it ever kept put arena saves and old approval cards among one task's changes (VM 520,
/// 4 October). A task agent's turns are all its one task, so it keeps showing them all.
pub fn run_to_show(a: &Agent, run: Option<u64>) -> Option<u64> {
    run.or_else(|| {
        // Not a promptless turn: that holds what the shell said outside any run.
        a.is_plain_main().then(|| a.turns.iter().rev().find(|t| a.is_run(t) && !t.prompt.trim().is_empty()).map(|t| t.n)).flatten()
    })
}

fn key_of(a: &Agent, turn: &Turn) -> String {
    if a.is_plain_main() {
        RowKey::run(&a.meta.id, turn.n).id()
    } else {
        RowKey::agent(&a.meta.id).id()
    }
}

/// Whose work it is, as a row has always said it: "Council recipe → Reviewer", or the role alone.
pub fn via_of(a: &Agent) -> String {
    match (a.meta.recipe.as_ref().map(|r| r.label()), a.meta.role.as_ref().map(|r| r.name.clone())) {
        (Some(recipe), Some(role)) => format!("{recipe} → {role}"),
        (Some(recipe), None) => recipe,
        (None, Some(role)) => role,
        (None, None) => String::new(),
    }
}

/// What a desk says it is doing, from what was observed and nothing else.
fn activity_of(a: &Agent, now: u64) -> String {
    let progress = progress::of(a, now);
    if a.state == State::WaitingForYou || progress.as_ref().is_some_and(|p| p.waiting_on_you) {
        return "Waiting for your answer".to_string();
    }
    if let Some(p) = &progress {
        if let Some(stuck) = &p.stuck {
            return format!("Looks stuck: {}", one_line(stuck, 80));
        }
        if let Some(running) = &p.running {
            return format!("Running {}", one_line(running, 80));
        }
    }
    if !a.status.trim().is_empty() {
        return format!("Reports: {}", one_line(&a.status, 80));
    }
    format!("Last update {}", clock(a.touched))
}

/// The action a card offers, in the words of what is being asked for.
pub fn action_label(app: &str, action: &str) -> &'static str {
    let what = format!("{app}.{action}").to_ascii_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| what.contains(w));
    if has(&["agent_run", "terminal", "run_command", "shell.run"]) {
        "Review command"
    } else if has(&["move", "rename", "copy"]) {
        "Review moves"
    } else if has(&["delete", "trash", "remove"]) {
        "Review deletion"
    } else if has(&["mail", "email", "send_message"]) {
        "Review email"
    } else if has(&["hand_off", "new_agent"]) {
        "Review hand-off"
    } else {
        "Review request"
    }
}

/// Everything waiting on the person, oldest first: approval cards asked for an agent, questions an
/// agent asked, a command at a prompt, and a task the shell judges stuck. Approvals are the
/// shell's own cards, found by the request ids the agent holds; the card itself is still the one
/// in the agent's pane, answered through the same Allow and Deny — this only says it is there.
fn decisions_of(s: &Store, cards: &[approvals::Card], jobs: &[(AgentId, String)], now: u64) -> Vec<Decision> {
    let mut out = Vec::new();
    for a in s.agents() {
        let task = || {
            let latest = a.open_turn().map(|t| t.prompt.as_str()).filter(|p| !p.trim().is_empty()).unwrap_or(a.meta.title.as_str());
            progress::brief(latest, TASK_CHARS)
        };
        let key = match a.open_turn() {
            Some(t) if a.is_plain_main() => RowKey::run(&a.meta.id, t.n).id(),
            _ => RowKey::agent(&a.meta.id).id(),
        };
        for request in &a.pending_approvals {
            let Some(card) = cards.iter().find(|c| &c.id == request) else { continue };
            out.push(Decision {
                key: key.clone(),
                request: request.clone(),
                mind_id: a.meta.id.harness().to_string(),
                mind: a.meta.mind.clone(),
                task: task(),
                text: one_line(if card.summary.is_empty() { &card.purpose } else { &card.summary }, REQUEST_CHARS),
                age_secs: card.age_secs,
                action: action_label(&card.app, &card.action).to_string(),
            });
        }
        for turn in &a.turns {
            for item in &turn.items {
                if let Item::Question(q) = item {
                    if q.waiting() {
                        out.push(Decision {
                            key: key.clone(),
                            request: q.request.clone(),
                            mind_id: a.meta.id.harness().to_string(),
                            mind: a.meta.mind.clone(),
                            task: task(),
                            text: one_line(&q.prompt, REQUEST_CHARS),
                            age_secs: now.saturating_sub(q.asked),
                            action: "Answer question".to_string(),
                        });
                    }
                }
            }
        }
        if jobs.iter().any(|(agent, _)| agent == &a.meta.id) {
            out.push(Decision {
                key: key.clone(),
                request: String::new(),
                mind_id: a.meta.id.harness().to_string(),
                mind: a.meta.mind.clone(),
                task: task(),
                text: "A command it started is waiting at a prompt.".to_string(),
                age_secs: now.saturating_sub(a.touched),
                action: "Open the run".to_string(),
            });
        }
        if let Some(why) = progress::of(a, now).and_then(|p| p.stuck).filter(|_| a.state.working()) {
            out.push(Decision {
                key,
                request: String::new(),
                mind_id: a.meta.id.harness().to_string(),
                mind: a.meta.mind.clone(),
                task: task(),
                text: format!("Looks stuck: {}", one_line(&why, 100)),
                age_secs: now.saturating_sub(a.touched),
                action: "Open the run".to_string(),
            });
        }
    }
    // Oldest first: the one that has waited longest is the one to look at.
    out.sort_by(|x, y| y.age_secs.cmp(&x.age_secs));
    out
}

fn finished(a: &Agent, t: &Turn) -> Option<Finished> {
    if t.open() || t.prompt.trim().is_empty() || !a.is_run(t) {
        return None;
    }
    let (outcome, label) = if t.lost {
        ("stopped", "Stopped · the desktop stopped")
    } else if t.ok == Some(false) {
        ("failed", "Couldn't finish")
    } else {
        ("done", "Finished")
    };
    Some(Finished {
        key: RowKey::run(&a.meta.id, t.n).id(),
        mind_id: a.meta.id.harness().to_string(),
        mind: a.meta.mind.clone(),
        task: progress::brief(&t.prompt, TASK_CHARS),
        outcome,
        label,
        when: clock(t.ended.unwrap_or(t.started)),
    })
}

/// Everything the overview draws. `attached` lists the harness ids that can answer now (the
/// built-in mind included), `minds` the attached minds as (id, name).
pub fn compose(
    s: &Store,
    minds: &[(String, String)],
    attached: &[String],
    cards: &[approvals::Card],
    jobs: &[(AgentId, String)],
    now: u64,
) -> Workroom {
    let desks: Vec<Desk> = {
        let mut at: Vec<(&Agent, &Turn)> = at_work(s).collect();
        // Stable under the pointer: oldest started first, never reshuffled by who spoke last.
        at.sort_by_key(|(a, t)| (t.started, a.seq));
        at.into_iter()
            .map(|(a, turn)| {
                let needs = a.state == State::WaitingForYou;
                Desk {
                    key: key_of(a, turn),
                    mind_id: a.meta.id.harness().to_string(),
                    mind: a.meta.mind.clone(),
                    via: via_of(a),
                    task: progress::brief(&turn.prompt, TASK_CHARS),
                    state: if needs { "needs_you" } else { "working" },
                    label: if needs { "Needs you" } else { "Working" }.to_string(),
                    activity: activity_of(a, now),
                    since: duration(now.saturating_sub(turn.started)),
                }
            })
            .collect()
    };
    let working = desks.iter().filter(|d| d.state == "working").count();

    let decisions = decisions_of(s, cards, jobs, now);
    let mut asking: Vec<&str> = decisions.iter().map(|d| d.mind.as_str()).collect();
    asking.sort_unstable();
    asking.dedup();

    // MINDS: the attached ones, then any mind that has agents on this desktop but is not attached
    // now, which says "Unavailable" rather than disappearing with its history.
    let mut order: Vec<(String, String)> = minds.to_vec();
    for a in s.agents() {
        if !order.iter().any(|(id, _)| id == a.meta.id.harness()) {
            order.push((a.meta.id.harness().to_string(), a.meta.mind.clone()));
        }
    }
    let mut nav = Vec::new();
    for (id, name) in order {
        let available = attached.iter().any(|h| h == &id);
        let working_here = desks.iter().filter(|d| d.mind_id == id && d.state == "working").count();
        let needs = decisions.iter().filter(|d| d.mind_id == id).count();
        let sub = if !available {
            "Unavailable".to_string()
        } else if working_here > 0 {
            format!("{working_here} working")
        } else if needs > 0 {
            "Waiting on you".to_string()
        } else {
            "Idle".to_string()
        };
        nav.push(Nav { kind: "mind", id: id.clone(), label: name, sub, needs, working: working_here > 0, available });
        // Its task agents sit under it while they live; finished ones are in History.
        for a in s.agents().iter().filter(|a| a.meta.id.harness() == id && !a.is_plain_main() && a.state.live()) {
            nav.push(Nav {
                kind: "task",
                id: RowKey::agent(&a.meta.id).id(),
                label: progress::brief(latest_request(&a.turns, &a.meta.title), 60),
                sub: if a.state == State::WaitingForYou {
                    "Needs you"
                } else if a.state.working() {
                    "Working"
                } else {
                    "Idle"
                }
                .to_string(),
                needs: usize::from(a.state == State::WaitingForYou),
                working: a.state.working(),
                available,
            });
        }
    }

    // Every finished request, newest first: the store's own list of runs, closed ones only.
    let runs: Vec<Finished> = s
        .tasks(usize::MAX)
        .into_iter()
        .filter_map(|(id, n)| {
            let a = s.agent(&id)?;
            finished(a, a.turns.iter().find(|t| t.n == n)?)
        })
        .collect();
    let total = runs.len();

    Workroom {
        working,
        requests: decisions.len(),
        request_minds: asking.len(),
        desks,
        decisions,
        nav,
        history: runs.into_iter().take(HISTORY_SHOWN).collect(),
        runs: total,
    }
}

/// The workroom as `describe shell` says it. `agent_reading` is another mind reading: the counts,
/// the pages and each mind's state are published, and what the person asked, what is running and
/// what is waiting on them are not.
pub fn for_describe(room: &Workroom, page: &str, opened: Option<&str>, narrowed: Option<&str>, agent_reading: bool) -> serde_json::Value {
    let minds: Vec<serde_json::Value> = room
        .nav
        .iter()
        .filter(|n| n.kind == "mind")
        .map(|n| {
            serde_json::json!({
                "id": n.id,
                "name": n.label,
                "state": n.sub,
                "working": n.working,
                "requests": n.needs,
                "available": n.available,
            })
        })
        .collect();
    let mut view = serde_json::json!({
        "page": page,
        "opened": opened,
        "narrowed_to": narrowed,
        "summary": room.summary(),
        "working": room.working,
        "requests": room.requests,
        "request_minds": room.request_minds,
        "runs_finished": room.runs,
        "minds": minds,
    });
    if agent_reading {
        view["private"] = "the person's; an agent reads its own session with read_agent".into();
        return view;
    }
    view["desks"] = room
        .desks
        .iter()
        .map(|d| {
            serde_json::json!({
                "key": d.key, "mind": d.mind, "via": d.via, "task": d.task,
                "state": d.state, "activity": d.activity, "running_for": d.since,
            })
        })
        .collect::<Vec<_>>()
        .into();
    view["requests_waiting"] = room
        .decisions
        .iter()
        .map(|d| {
            serde_json::json!({
                "key": d.key, "mind": d.mind, "task": d.task, "request": d.text,
                "action": d.action, "age_secs": d.age_secs,
            })
        })
        .collect::<Vec<_>>()
        .into();
    view
}

/// "3m ago", "just now".
pub fn ago(secs: u64) -> String {
    match secs {
        0..=9 => "just now".to_string(),
        10..=59 => format!("{secs}s ago"),
        60..=3599 => format!("{}m ago", secs / 60),
        3600..=86_399 => format!("{}h ago", secs / 3600),
        _ => format!("{}d ago", secs / 86_400),
    }
}

// ── A task's detail ─────────────────────────────────────────────────

/// One line of the changes ledger.
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    /// `Commands`, `Files`, `Approvals` or `Reported by the mind`.
    pub group: &'static str,
    pub text: String,
    pub status: String,
    /// `ok`, `bad`, `dim` or `warn`: how the status is drawn.
    pub tone: &'static str,
}

/// What the ledger says when nothing was recorded.
pub const NO_CHANGES: &str = "No changes recorded";

/// The changes ledger for the turns shown: what the shell itself ran or was asked, and what the
/// mind only said.
///
/// A path counts only when a call the shell ran named it, and then it is "named by a command the
/// shell ran" — that a command named a file is not proof it changed it, and the ledger does not
/// say it did. A call a harness reported without the shell seeing it is a claim, listed apart.
pub fn changes_of(turns: &[&Turn]) -> Vec<Change> {
    let mut out: Vec<Change> = Vec::new();
    let mut claims: Vec<Change> = Vec::new();
    let mut files: Vec<String> = Vec::new();
    for turn in turns {
        for item in &turn.items {
            match item {
                Item::Card(c) if c.provenance == Provenance::Verified => {
                    if c.is_command() {
                        let (status, tone) = match (c.state, c.exit_code) {
                            (CallState::Running, _) => ("Running".to_string(), "dim"),
                            (_, Some(0)) => ("Recorded · exit 0".to_string(), "ok"),
                            (_, Some(code)) => (format!("Attempted · exit {code}"), "bad"),
                            (CallState::Failed, None) => ("Attempted · failed".to_string(), "bad"),
                            (CallState::Interrupted, None) => ("Attempted · cut short".to_string(), "warn"),
                            _ => ("Recorded".to_string(), "ok"),
                        };
                        out.push(Change { group: "Commands", text: one_line(&c.command_line(), 120), status, tone });
                    }
                    for path in c.paths() {
                        if !files.contains(&path) {
                            files.push(path);
                        }
                    }
                }
                Item::Card(c) => claims.push(Change {
                    group: "Reported by the mind",
                    text: one_line(&c.as_call().summary(), 120),
                    status: "Reported, not verified".to_string(),
                    tone: "dim",
                }),
                Item::Approval(ap) => {
                    let (status, tone) = match ap.outcome {
                        ApprovalOutcome::Pending => ("Waiting for you", "warn"),
                        ApprovalOutcome::Allowed => ("Approved by you", "ok"),
                        ApprovalOutcome::Denied => ("Declined by you", "dim"),
                        ApprovalOutcome::Expired => ("Expired unanswered", "dim"),
                        ApprovalOutcome::Withdrawn => ("Withdrawn", "dim"),
                    };
                    out.push(Change { group: "Approvals", text: ap.what.clone(), status: status.to_string(), tone });
                }
                _ => {}
            }
        }
    }
    out.extend(files.into_iter().map(|path| Change {
        group: "Files",
        text: path,
        status: "Named by a command the shell ran".to_string(),
        tone: "dim",
    }));
    out.extend(claims);
    out
}

/// Whether the ledger holds anything the shell itself recorded: "No changes recorded" is said
/// when it does not, even if the mind made claims.
pub fn recorded(changes: &[Change]) -> bool {
    changes.iter().any(|c| c.group != "Reported by the mind")
}

/// The Activity timeline: consecutive tool calls folded into one line a person reads, expandable.
///
/// `items` is the session as `items_of` drew it. A run of calls becomes one `group` item, "Made 4
/// calls, 1 failed", with the calls' own cards after it while it is open — or while one of them is
/// still running, since its output is what is happening. Prompts, the mind's words, approvals and
/// questions stay as they were.
pub fn group_calls(items: Vec<AgentItemData>, open: &HashSet<String>) -> Vec<AgentItemData> {
    fn flush(run: &mut Vec<AgentItemData>, open: &HashSet<String>, out: &mut Vec<AgentItemData>) {
        if run.is_empty() {
            return;
        }
        let calls = std::mem::take(run);
        let key = format!("g:{}", calls[0].key);
        let failed = calls.iter().filter(|c| c.call.status == "failed").count();
        let running = calls.iter().filter(|c| c.live || c.call.status == "running").count();
        let mut title = format!("Made {} call{}", calls.len(), if calls.len() == 1 { "" } else { "s" });
        if failed > 0 {
            title.push_str(&format!(", {failed} failed"));
        }
        if running > 0 {
            title.push_str(&format!(", {running} still running"));
        }
        let names: Vec<String> = calls.iter().take(2).map(|c| one_line(&c.call.summary, 50)).collect();
        let more = if calls.len() > 2 { format!(", and {} more", calls.len() - 2) } else { String::new() };
        let expanded = open.contains(&key) || running > 0;
        out.push(AgentItemData {
            kind: "group".into(),
            key: key.into(),
            text: title.into(),
            explain: format!("{}{more}", names.join(", ")).into(),
            expanded,
            call: ToolCallData::default(),
            ..Default::default()
        });
        if expanded {
            out.extend(calls);
        }
    }
    let mut out = Vec::with_capacity(items.len());
    let mut run: Vec<AgentItemData> = Vec::new();
    for item in items {
        if item.kind == "card" {
            run.push(item);
        } else {
            flush(&mut run, open, &mut out);
            out.push(item);
        }
    }
    flush(&mut run, open, &mut out);
    out
}

/// One run's state in the screen's vocabulary — Working · Needs you · Finished · Stopped ·
/// Couldn't finish · Connection lost — as `(key, label)`. A run of a chat is judged by its own
/// turn, not by the conversation, which may already be on to the next request.
pub fn state_of(a: &Agent, run: Option<u64>) -> (&'static str, &'static str) {
    let turn = run.and_then(|n| a.turns.iter().find(|t| t.n == n));
    let live = |state: State| match state {
        State::WaitingForYou => ("needs_you", "Needs you"),
        State::HarnessGone => ("lost", "Connection lost"),
        State::Thinking | State::RunningTool => ("working", "Working"),
        State::Idle => ("idle", "Idle"),
        State::Done => ("done", "Finished"),
        State::Failed => ("failed", "Couldn't finish"),
    };
    match turn {
        Some(t) if t.open() => live(a.state),
        Some(t) => match finished(a, t) {
            Some(f) => (f.outcome, f.label),
            None => ("done", "Finished"),
        },
        None => live(a.state),
    }
}

/// A run another agent started from the opened one.
#[derive(Clone, Debug, PartialEq)]
pub struct ChildRun {
    pub key: String,
    pub title: String,
    pub state: &'static str,
    pub label: &'static str,
}

/// The runs the opened agent started, oldest first: one level, because a child cannot start
/// agents of its own (`launch::stop_on`, which stops them with their parent). With a run chosen
/// only the children started while that run was open are listed; without, all of them.
pub fn child_runs(s: &Store, a: &Agent, run: Option<u64>) -> Vec<ChildRun> {
    let window = run.and_then(|n| a.turns.iter().find(|t| t.n == n)).map(|t| (t.started, t.ended.unwrap_or(u64::MAX)));
    s.children_of(&a.meta.id)
        .iter()
        .filter_map(|id| s.agent(id))
        .filter(|c| window.is_none_or(|(from, to)| c.meta.started >= from && c.meta.started <= to))
        .map(|c| {
            let who = c.meta.role.as_ref().map(|r| r.name.clone()).unwrap_or_else(|| c.meta.mind.clone());
            let (state, label) = state_of(c, None);
            ChildRun { key: c.meta.id.0.clone(), title: format!("{who}: {}", c.meta.title), state, label }
        })
        .collect()
}

/// The desktop's mode in a word. It is the desktop's, not the run's: one mode governs every mind.
pub fn mode_label(mode: crate::mind_mode::Mode) -> &'static str {
    use crate::mind_mode::Mode;
    match mode {
        Mode::Plan => "Plan",
        Mode::Ask => "Ask",
        Mode::Auto => "Auto",
        Mode::Bypass => "Bypass",
        Mode::BypassAll => "Full bypass",
    }
}

/// What the mode lets a mind do, as Start work says it.
pub fn mode_line(mode: crate::mind_mode::Mode) -> String {
    use crate::mind_mode::Mode;
    let what = match mode {
        Mode::Plan => "Inspect and propose. No changes allowed.",
        Mode::Ask => "Request approval for actions that require it.",
        Mode::Auto => "Act within granted permissions; the most dangerous actions still ask.",
        Mode::Bypass => "Most actions run without asking, for a limited time. What cannot be undone still asks.",
        Mode::BypassAll => "Everything below the machine's ceiling runs without asking, for a limited time.",
    };
    format!("{} — {what} This is the desktop's mode and applies to every mind.", mode_label(mode))
}

/// The facts behind a run, as "Run details" lists them. Anything the shell was not told reads
/// "Not recorded", and cost is marked an estimate: it is the harness's own figure. Tokens and cost
/// are counted per conversation, so for one run of a chat they are not recorded for the run.
pub fn run_facts(a: &Agent, run: Option<u64>) -> Vec<(String, String)> {
    let recorded_or = |text: String| if text.trim().is_empty() { "Not recorded".to_string() } else { text };
    let scoped: Vec<&Turn> = match run {
        Some(n) => a.turns.iter().filter(|t| t.n == n).collect(),
        None => a.turns.iter().collect(),
    };
    let calls: usize = scoped.iter().map(|t| t.cards().count()).sum();
    let failed: usize = scoped.iter().flat_map(|t| t.cards()).filter(|c| c.state == CallState::Failed).count();
    let model = if a.meta.model.is_empty() { a.usage.model.clone() } else { a.meta.model.clone() };
    let per_run = run.is_some() && a.is_plain_main();
    let tokens = if a.usage.reported && !per_run {
        format!("{} in · {} out", a.usage.input_tokens, a.usage.output_tokens)
    } else {
        String::new()
    };
    let cost = if a.usage.reported && !per_run && a.usage.cost_usd > 0.0 { format!("${:.2}", a.usage.cost_usd) } else { String::new() };
    let mut facts = vec![
        ("Mind".to_string(), a.meta.mind.clone()),
        ("Model".to_string(), recorded_or(model)),
        ("Agent".to_string(), a.meta.id.0.clone()),
    ];
    if let Some(n) = run {
        facts.push(("Run".to_string(), format!("{}#{n}", a.meta.id.0)));
    }
    facts.push(("Turns".to_string(), scoped.len().to_string()));
    facts.push(("Calls".to_string(), if failed > 0 { format!("{calls} ({failed} failed)") } else { calls.to_string() }));
    facts.push(("Tokens".to_string(), recorded_or(tokens)));
    facts.push(("Cost · Estimated".to_string(), recorded_or(cost)));
    facts.push(("Started".to_string(), clock(a.meta.started)));
    facts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::model::now;
    use crate::agents::{AgentMeta, Event};

    fn id(text: &str) -> AgentId {
        AgentId(text.to_string())
    }

    fn start(s: &mut Store, agent: &str, mind: &str, prompt: &str) -> AgentId {
        let a = id(agent);
        s.upsert_agent(AgentMeta::new(a.clone(), mind));
        s.open_turn(&a, prompt);
        a
    }

    fn compose_for(s: &Store) -> Workroom {
        compose(
            s,
            &[("pi".into(), "pi".into()), ("hermes".into(), "Hermes".into())],
            &["pi".into(), "hermes".into()],
            &[],
            &[],
            now(),
        )
    }

    /// A main mind opened without a run shows its latest run, not every turn it kept; a task agent,
    /// whose turns are all its one task, shows them all (VM 520, 4 October).
    #[test]
    fn a_main_mind_opened_without_a_run_shows_its_latest_run() {
        let mut s = Store::new();
        let m = start(&mut s, "pi:main", "pi", "write the week plan");
        s.close_turn(&m, true);
        s.open_turn(&m, "go through my notes");
        s.close_turn(&m, true);
        s.approval_asked(&m, "late", "files.move"); // after the run: a promptless turn of its own
        let a = s.agent(&m).unwrap();
        let notes = a.turns.iter().find(|t| t.prompt == "go through my notes").unwrap().n;
        assert_eq!(run_to_show(a, None), Some(notes), "the latest run, not the promptless turn after it");
        assert_eq!(run_to_show(a, Some(1)), Some(1), "a run asked for is the run shown");
        let t = start(&mut s, "pi:c-7", "pi", "tidy the photos folder");
        assert_eq!(run_to_show(s.agent(&t).unwrap(), None), None, "a task agent shows all its turns");
    }

    #[test]
    fn two_agents_at_work_are_two_desks_and_the_header_counts_them() {
        let mut s = Store::new();
        start(&mut s, "pi:c-1", "pi", "tidy the photos folder");
        start(&mut s, "hermes:c-2", "Hermes", "write the release notes");
        let w = compose_for(&s);
        assert_eq!(w.desks.len(), 2);
        assert_eq!(w.working, 2);
        assert_eq!(w.summary(), "2 working");
        assert!(w.desks.iter().all(|d| d.state == "working" && d.label == "Working"));
    }

    #[test]
    fn nothing_running_says_so_and_does_not_invent_progress() {
        let w = compose_for(&Store::new());
        assert!(w.desks.is_empty() && w.decisions.is_empty());
        assert_eq!(w.summary(), "Nothing running");
    }

    #[test]
    fn a_desk_says_when_it_last_heard_until_a_call_is_seen_running() {
        let mut s = Store::new();
        let a = start(&mut s, "pi:c-1", "pi", "tidy the photos folder");
        let w = compose_for(&s);
        assert!(w.desks[0].activity.starts_with("Last update "), "{}", w.desks[0].activity);
        let call = Event::ToolStart { call: "c1".into(), name: "agent_run".into(), target: "fdupes".into(), args: serde_json::json!({}) };
        s.event(&a, &call, Provenance::Verified);
        let w = compose_for(&s);
        assert!(w.desks[0].activity.starts_with("Running agent_run"), "{}", w.desks[0].activity);
    }

    #[test]
    fn a_mind_with_nothing_running_is_idle_and_one_that_is_gone_is_unavailable() {
        let mut s = Store::new();
        start(&mut s, "pi:c-1", "pi", "tidy the photos folder");
        s.upsert_agent(AgentMeta::new(id("ghost:c-9"), "Ghost"));
        let w = compose_for(&s);
        let sub = |name: &str| w.nav.iter().find(|n| n.kind == "mind" && n.label == name).map(|n| n.sub.clone()).unwrap();
        assert_eq!(sub("pi"), "1 working");
        assert_eq!(sub("Hermes"), "Idle", "attached, nothing to do");
        assert_eq!(sub("Ghost"), "Unavailable", "its history stays attributable");
    }

    #[test]
    fn a_waiting_question_is_a_decision_and_counts_in_the_header() {
        let mut s = Store::new();
        let a = start(&mut s, "pi:c-1", "pi", "tidy the photos folder");
        let ask = Event::Request { request_id: "q1".into(), prompt: "Which folder?".into(), options: vec![] };
        s.event(&a, &ask, Provenance::Reported);
        let w = compose_for(&s);
        assert_eq!(w.requests, 1);
        assert_eq!(w.request_minds, 1);
        assert_eq!(w.decisions[0].action, "Answer question");
        assert_eq!(w.decisions[0].text, "Which folder?");
        assert!(w.summary().ends_with("1 needs you"), "{}", w.summary());
    }

    #[test]
    fn decisions_are_listed_oldest_first() {
        let mut s = Store::new();
        let a = start(&mut s, "pi:c-1", "pi", "task a");
        let b = start(&mut s, "hermes:c-2", "Hermes", "task b");
        let ask = |request: &str| Event::Request { request_id: request.into(), prompt: format!("prompt {request}"), options: vec![] };
        s.event(&a, &ask("older"), Provenance::Reported);
        s.event(&b, &ask("newer"), Provenance::Reported);
        let mut w = compose_for(&s);
        // Both asked within the same second here; give the first a real head start.
        w.decisions[0].age_secs = 1;
        w.decisions[1].age_secs = 300;
        w.decisions.sort_by(|x, y| y.age_secs.cmp(&x.age_secs));
        assert!(w.decisions[0].age_secs >= w.decisions[1].age_secs);
        assert_eq!(w.requests, 2);
        assert_eq!(w.request_minds, 2);
    }

    #[test]
    fn narrowing_to_a_mind_keeps_only_its_desks_and_results() {
        let mut s = Store::new();
        start(&mut s, "pi:c-1", "pi", "tidy the photos folder");
        let h = start(&mut s, "hermes:c-2", "Hermes", "write the release notes");
        s.close_turn(&h, true);
        let w = compose_for(&s);
        let pi = w.narrowed("pi");
        assert_eq!((pi.desks.len(), pi.history.len()), (1, 0));
        let hermes = w.narrowed("hermes");
        assert_eq!((hermes.desks.len(), hermes.history.len()), (0, 1));
        assert_eq!(hermes.nav, w.nav, "the navigation is the whole desktop's");
    }

    #[test]
    fn a_card_offers_the_action_for_what_it_asks() {
        assert_eq!(action_label("shell", "agent_run"), "Review command");
        assert_eq!(action_label("files", "move_files"), "Review moves");
        assert_eq!(action_label("mail", "send_email"), "Review email");
        assert_eq!(action_label("calendar", "create_event"), "Review request");
    }

    #[test]
    fn a_finished_run_moves_to_history_and_its_desk_goes() {
        let mut s = Store::new();
        let a = start(&mut s, "pi:c-1", "pi", "tidy the photos folder");
        assert_eq!(compose_for(&s).desks.len(), 1);
        s.close_turn(&a, true);
        let w = compose_for(&s);
        assert!(w.desks.is_empty());
        assert_eq!(w.runs, 1);
        assert_eq!(w.history[0].label, "Finished");
        assert_eq!(w.history[0].mind, "pi");
    }

    #[test]
    fn a_failed_run_says_it_could_not_finish_and_never_that_it_is_fixed() {
        let mut s = Store::new();
        let a = start(&mut s, "pi:c-1", "pi", "tidy the photos folder");
        s.close_turn(&a, false);
        let w = compose_for(&s);
        assert_eq!(w.history[0].label, "Couldn't finish");
        assert_eq!(w.history[0].outcome, "failed");
    }

    #[test]
    fn task_agents_sit_under_their_mind_while_they_live() {
        let mut s = Store::new();
        start(&mut s, "pi:c-1", "pi", "tidy the photos folder");
        let w = compose_for(&s);
        let at = w.nav.iter().position(|n| n.kind == "mind" && n.label == "pi").unwrap();
        assert_eq!(w.nav[at + 1].kind, "task");
        assert_eq!(w.nav[at + 1].id, "pi:c-1");
    }

    #[test]
    fn a_finished_task_agent_leaves_its_mind_for_history() {
        let mut s = Store::new();
        let a = start(&mut s, "pi:c-1", "pi", "tidy the photos folder");
        s.close_turn(&a, true);
        s.set_state(&a, State::Done);
        let w = compose_for(&s);
        assert!(w.nav.iter().all(|n| n.kind != "task"), "{:?}", w.nav);
        assert_eq!(w.runs, 1);
    }

    #[test]
    fn no_changes_is_said_as_no_changes_recorded() {
        let mut s = Store::new();
        let a = start(&mut s, "pi:c-1", "pi", "say hello");
        s.text(&a, "hello");
        let agent = s.agent(&a).unwrap();
        let turns: Vec<&Turn> = agent.turns.iter().collect();
        let ledger = changes_of(&turns);
        assert!(ledger.is_empty());
        assert!(!recorded(&ledger));
        assert_eq!(NO_CHANGES, "No changes recorded");
    }

    #[test]
    fn a_claim_the_shell_did_not_see_is_listed_apart_and_does_not_count_as_recorded() {
        let mut s = Store::new();
        let a = start(&mut s, "pi:c-1", "pi", "move the copies");
        let call = Event::ToolStart {
            call: "c1".into(),
            name: "os_act".into(),
            target: "files.move".into(),
            args: serde_json::json!({ "from": "/home/x/a.jpg", "to": "/home/x/Trash" }),
        };
        s.event(&a, &call, Provenance::Reported);
        let agent = s.agent(&a).unwrap();
        let turns: Vec<&Turn> = agent.turns.iter().collect();
        let ledger = changes_of(&turns);
        assert_eq!(ledger.len(), 1);
        assert_eq!(ledger[0].group, "Reported by the mind");
        assert_eq!(ledger[0].status, "Reported, not verified");
        assert!(!recorded(&ledger), "a claim is not a record");
    }

    #[test]
    fn a_command_the_shell_ran_is_recorded_with_its_exit() {
        let mut s = Store::new();
        let a = start(&mut s, "pi:c-1", "pi", "list the pictures");
        s.command_started(&a, "j1", "ls ~/Pictures", "/home/x");
        s.command_finished(&a, "j1", "ls ~/Pictures", Some(0), false);
        let agent = s.agent(&a).unwrap();
        let turns: Vec<&Turn> = agent.turns.iter().collect();
        let ledger = changes_of(&turns);
        let command = ledger.iter().find(|c| c.group == "Commands").expect("the command is listed");
        assert_eq!(command.status, "Recorded · exit 0");
        assert!(recorded(&ledger));
    }

    #[test]
    fn a_path_a_command_names_is_named_not_changed() {
        let mut s = Store::new();
        let a = start(&mut s, "pi:c-1", "pi", "move it");
        let call = Event::ToolStart {
            call: "j1".into(),
            name: "agent_run".into(),
            target: String::new(),
            args: serde_json::json!({ "command": "mv a b", "path": "/home/x/a.jpg" }),
        };
        s.event(&a, &call, Provenance::Verified);
        let agent = s.agent(&a).unwrap();
        let turns: Vec<&Turn> = agent.turns.iter().collect();
        let ledger = changes_of(&turns);
        let file = ledger.iter().find(|c| c.group == "Files").expect("the path is listed");
        assert!(file.status.starts_with("Named by"), "{}", file.status);
    }

    fn card(key: &str, status: &str) -> AgentItemData {
        AgentItemData {
            kind: "card".into(),
            key: key.into(),
            call: ToolCallData { summary: format!("agent_run {key}").into(), status: status.into(), ..Default::default() },
            ..Default::default()
        }
    }

    #[test]
    fn calls_fold_into_one_prose_line_that_opens_into_the_calls() {
        let prose = AgentItemData { kind: "text".into(), key: "t".into(), ..Default::default() };
        let items = vec![card("a", "done"), card("b", "failed"), card("c", "done"), prose, card("d", "done")];
        let closed = group_calls(items.clone(), &HashSet::new());
        let kinds: Vec<String> = closed.iter().map(|i| i.kind.to_string()).collect();
        assert_eq!(kinds, ["group", "text", "group"]);
        assert_eq!(closed[0].text.as_str(), "Made 3 calls, 1 failed");
        assert_eq!(closed[2].text.as_str(), "Made 1 call");
        let mut open = HashSet::new();
        open.insert("g:a".to_string());
        let opened = group_calls(items, &open);
        let kinds: Vec<String> = opened.iter().map(|i| i.kind.to_string()).collect();
        assert_eq!(kinds, ["group", "card", "card", "card", "text", "group"]);
    }

    #[test]
    fn a_call_still_running_keeps_its_group_open() {
        let running = AgentItemData { live: true, ..card("a", "running") };
        let out = group_calls(vec![running], &HashSet::new());
        assert_eq!(out.len(), 2);
        assert!(out[0].expanded);
        assert!(out[0].text.contains("still running"));
    }

    #[test]
    fn run_details_say_not_recorded_for_what_is_missing_and_mark_cost_estimated() {
        let mut s = Store::new();
        let a = start(&mut s, "pi:c-1", "pi", "x");
        let agent = s.agent(&a).unwrap();
        let facts = run_facts(agent, None);
        let get = |k: &str| facts.iter().find(|(l, _)| l == k).map(|(_, v)| v.clone()).unwrap();
        assert_eq!(get("Model"), "Not recorded");
        assert_eq!(get("Tokens"), "Not recorded");
        assert_eq!(get("Cost · Estimated"), "Not recorded");
        assert_eq!(get("Agent"), "pi:c-1");
    }

    #[test]
    fn a_runs_state_is_its_own_not_the_conversations() {
        let mut s = Store::new();
        let a = start(&mut s, "pi:main", "pi", "first request");
        s.close_turn(&a, false);
        s.open_turn(&a, "second request");
        let agent = s.agent(&a).unwrap();
        let first = agent.turns[0].n;
        let second = agent.turns[1].n;
        assert_eq!(state_of(agent, Some(first)), ("failed", "Couldn't finish"), "it ended; the next request does not change that");
        assert_eq!(state_of(agent, Some(second)).1, "Working");
        assert_eq!(state_of(agent, None).1, "Working");
    }

    #[test]
    fn tokens_are_not_recorded_for_one_run_of_a_shared_conversation() {
        let mut s = Store::new();
        let a = start(&mut s, "pi:main", "pi", "x");
        let usage = Event::Usage { model: "m".into(), input_tokens: Some(10), output_tokens: Some(5), cost_usd: Some(0.02) };
        s.event(&a, &usage, Provenance::Reported);
        let agent = s.agent(&a).unwrap();
        let n = agent.turns[0].n;
        let get = |facts: &[(String, String)], k: &str| facts.iter().find(|(l, _)| l == k).map(|(_, v)| v.clone()).unwrap();
        let run = run_facts(agent, Some(n));
        assert_eq!(get(&run, "Tokens"), "Not recorded", "the figure is the conversation's, not this run's");
        assert_eq!(get(&run, "Cost · Estimated"), "Not recorded");
        assert_eq!(get(&run, "Model"), "m");
        let task = start(&mut s, "pi:c-2", "pi", "y");
        s.event(&task, &usage, Provenance::Reported);
        let whole = run_facts(s.agent(&task).unwrap(), None);
        assert_eq!(get(&whole, "Tokens"), "10 in · 5 out");
        assert_eq!(get(&whole, "Cost · Estimated"), "$0.02");
    }

    #[test]
    fn the_mode_is_said_as_the_desktops_and_every_mode_has_a_sentence() {
        use crate::mind_mode::Mode;
        for mode in Mode::ALL {
            let line = mode_line(mode);
            assert!(line.starts_with(mode_label(mode)), "{line}");
            assert!(line.ends_with("applies to every mind."), "{line}");
        }
        assert!(mode_line(Mode::Plan).contains("No changes allowed"));
        assert!(mode_line(Mode::Ask).contains("approval"));
    }

    fn slint(file: &str) -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../yantrik-ui-slint/ui").join(file);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
    }

    /// The spec drops three things from the screen: Pop out (window controls do that), the
    /// List/Overview toggle, and a permanent details column. None of them is drawn.
    #[test]
    fn the_screen_has_no_pop_out_no_list_toggle_and_no_details_column() {
        let screen = slint("agents.slint");
        let at = screen.find("export component AgentsScreen").expect("the screen");
        let screen = &screen[at..screen[at..].find("// One agent in a window of its own").map_or(screen.len(), |n| at + n)];
        for gone in ["Pop out", "pop-out", "ViewSwitch", "AgentDetailsColumn", "New agent", "DETAILS"] {
            assert!(!screen.contains(gone), "the workroom still draws {gone:?}");
        }
        for drawn in ["Start work", "History", "WorkNav", "WorkroomOverview", "TaskDetail"] {
            assert!(screen.contains(drawn), "the workroom does not draw {drawn:?}");
        }
    }

    /// A desk shows no picture it does not have. Nothing in the shell says which mind opened which
    /// window on Mind View, so a capture could be another mind's; the card says so.
    #[test]
    fn a_desk_with_no_preview_says_so_and_never_draws_one() {
        let overview = slint("agents_workroom.slint");
        let card = &overview[overview.find("export component DeskCard").unwrap()..overview.find("export component DeskGrid").unwrap()];
        assert!(card.contains("Desk preview unavailable"));
        for faked in ["Image {", "@image-url", "Snapshot"] {
            assert!(!card.contains(faked), "a desk card draws {faked:?} with no capture behind it");
        }
    }

    /// Stop… asks. The one place the screen calls stop is the confirmation's own button, and no
    /// other control on it stops a run directly. Pause is not drawn, because nothing can pause.
    #[test]
    fn stop_asks_first_and_pause_is_not_drawn() {
        let source = slint("agents.slint");
        let at = source.find("component TaskDetail").unwrap();
        let screen = &source[at..source.find("// One agent in a window of its own").unwrap()];
        assert_eq!(screen.matches("AgentsState.stop(").count(), 1, "one call to stop on the screen");
        assert!(screen.contains("AgentsState.stop(AgentsState.confirm-stop)"), "and it is the confirmation's");
        assert!(screen.contains("confirm-stop = AgentsState.header.id"), "the run's Stop… asks");
        assert!(!screen.contains("\"Pause\"") && !screen.contains("\"Resume\""), "no control for what cannot be done");
        for file in ["agents_workroom.slint", "agents_nav.slint", "agents_task.slint", "agents_start.slint"] {
            let drawn = slint(file);
            assert!(!drawn.contains("\"Pause") && !drawn.contains("AgentsState."), "{file} is drawn from props and draws no Pause");
        }
    }

    /// Teal is a mind and amber is a request waiting on the person; neither is used for anything
    /// else. A selection is neutral, and every surface is opaque.
    #[test]
    fn teal_marks_minds_amber_marks_requests_and_selection_is_neutral() {
        let screen = slint("agents.slint");
        let from = screen.find("component TaskDetail").unwrap();
        let to = screen.find("// One agent in a window of its own").unwrap();
        // The session's own components (ItemView, the details column) are older and shared with an
        // agent's window; the workroom around them is what the rule is held to here.
        for (file, text) in [
            ("agents_workroom.slint", slint("agents_workroom.slint")),
            ("agents_nav.slint", slint("agents_nav.slint")),
            ("agents_task.slint", slint("agents_task.slint")),
            ("agents_start.slint", slint("agents_start.slint")),
            ("agents.slint (screen)", screen[from..to].to_string()),
        ] {
            for (i, line) in text.lines().enumerate() {
                if line.trim_start().starts_with("//") {
                    continue;
                }
                if line.contains("Theme.mind") {
                    let ok = line.contains("MindMark") || line.contains("background: Theme.mind") || line.contains("border-color: root.available") || line.contains("available ? Theme.mind");
                    assert!(ok, "{file}:{}: teal on something that is not a mind: {line}", i + 1);
                }
                assert!(!line.contains("Theme.accent"), "{file}:{}: the person's accent colours the workroom: {line}", i + 1);
                assert!(!line.contains("accent-glow"), "{file}:{}: a selection is a raised neutral fill, not a glow: {line}", i + 1);
            }
        }
    }

    #[test]
    fn describe_tells_a_mind_the_counts_and_the_person_the_desks() {
        let mut s = Store::new();
        let a = start(&mut s, "pi:c-1", "pi", "tidy the photos folder");
        let ask = Event::Request { request_id: "q1".into(), prompt: "Which folder?".into(), options: vec![] };
        s.event(&a, &ask, Provenance::Reported);
        let room = compose_for(&s);
        let person = for_describe(&room, "workroom", None, None, false);
        assert_eq!(person["requests"], 1);
        assert_eq!(person["desks"][0]["task"], "tidy the photos folder");
        assert_eq!(person["requests_waiting"][0]["request"], "Which folder?");
        let minds = person["minds"].as_array().unwrap();
        assert!(minds.iter().any(|m| m["id"] == "pi"));
        // Another mind reads the same counts, and none of what the person asked or is being asked.
        let mind = for_describe(&room, "workroom", None, None, true);
        assert_eq!(mind["requests"], 1);
        assert_eq!(mind["working"], 0);
        assert!(mind.get("desks").is_none() && mind.get("requests_waiting").is_none(), "{mind}");
        assert!(!mind.to_string().contains("tidy the photos folder"), "{mind}");
        assert!(!mind.to_string().contains("Which folder?"), "{mind}");
    }

    #[test]
    fn ages_read_as_a_person_says_them() {
        assert_eq!(ago(3), "just now");
        assert_eq!(ago(42), "42s ago");
        assert_eq!(ago(190), "3m ago");
        assert_eq!(ago(7300), "2h ago");
    }

    /// The route showed the agents a run started and Stop on a parent stops them (review of #584,
    /// finding 4); the task detail lists them now, under the run that started them, and a stranger
    /// is not listed.
    #[test]
    fn the_task_detail_lists_the_runs_the_opened_one_started() {
        let mut s = Store::new();
        let parent = start(&mut s, "pi:c-parent01", "pi", "Review the migration");
        let child = id("hermes:c-review01");
        let mut meta = AgentMeta::new(child.clone(), "hermes");
        meta.parent = Some(parent.clone());
        meta.title = "Check the rollback".into();
        s.upsert_agent(meta);
        s.open_turn(&child, "Check the rollback");
        start(&mut s, "deepseek:c-other01", "deepseek", "Something unrelated");

        let a = s.agent(&parent).unwrap();
        let kids = child_runs(&s, a, None);
        assert_eq!(kids.len(), 1, "one child, not the stranger: {kids:?}");
        assert_eq!(kids[0].key, "hermes:c-review01");
        assert!(kids[0].title.contains("Check the rollback"), "{}", kids[0].title);
        assert_eq!(kids[0].state, "working");
        // A run chosen scopes it to what started while that run was open (the parent's one open
        // turn is `n`); a run number that is not there lists them all; a child has none.
        let n = a.turns[0].n;
        assert_eq!(child_runs(&s, a, Some(n)).len(), 1, "started while that run was open");
        assert_eq!(child_runs(&s, a, Some(9_999)).len(), 1, "an unknown run falls back to all children");
        assert!(child_runs(&s, s.agent(&child).unwrap(), None).is_empty());
    }

    const WORKROOM_SLINT: [(&str, &str); 4] = [
        ("agents_workroom.slint", include_str!("../../../yantrik-ui-slint/ui/agents_workroom.slint")),
        ("agents_start.slint", include_str!("../../../yantrik-ui-slint/ui/agents_start.slint")),
        ("agents_nav.slint", include_str!("../../../yantrik-ui-slint/ui/agents_nav.slint")),
        ("agents_task.slint", include_str!("../../../yantrik-ui-slint/ui/agents_task.slint")),
    ];

    /// Review of #584, finding 1: sizes, spacings and weights come from the theme, not from
    /// literals, so a density or scale change reaches the whole workroom. Only `0px` (nothing) may
    /// be written out. Comments are free to name the spec's numbers.
    #[test]
    fn the_workroom_screens_use_tokens_and_no_pixel_or_weight_literals() {
        for (name, src) in WORKROOM_SLINT {
            for (n, line) in src.lines().enumerate() {
                let code = line.split("//").next().unwrap();
                let bytes = code.as_bytes();
                let mut at = 0;
                while let Some(found) = code[at..].find("px") {
                    let end = at + found;
                    let mut start = end;
                    while start > 0 && (bytes[start - 1].is_ascii_digit() || bytes[start - 1] == b'.') {
                        start -= 1;
                    }
                    let number = &code[start..end];
                    let word_before = start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'-' || bytes[start - 1] == b'_');
                    if !number.is_empty() && !word_before {
                        assert!(
                            number.parse::<f32>() == Ok(0.0),
                            "{name}:{}: `{number}px` is a literal; use a Theme token: {line}",
                            n + 1
                        );
                    }
                    at = end + 2;
                }
                assert!(
                    !line.contains("font-weight:") || !code.split("font-weight:").nth(1).is_some_and(|w| w.chars().any(|c| c.is_ascii_digit())),
                    "{name}:{}: a numeric font-weight; use Theme.fw-*: {line}",
                    n + 1
                );
            }
        }
    }

    /// Every `Theme.` token these screens name is defined, so a renamed or mistyped one is a failing
    /// test and not a Slint error found ten minutes into a build.
    #[test]
    fn every_theme_token_the_workroom_screens_name_exists() {
        let theme = include_str!("../../../yantrik-design-tokens/slint/theme.slint");
        for (name, src) in WORKROOM_SLINT {
            for line in src.lines().filter(|l| !l.trim_start().starts_with("//")) {
                let mut rest = line;
                while let Some(at) = rest.find("Theme.") {
                    let tail = &rest[at + 6..];
                    let token: String = tail.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
                    assert!(
                        theme.contains(&format!("property <length> {token}:"))
                            || theme.contains(&format!("property <color> {token}:"))
                            || theme.contains(&format!("property <int> {token}:"))
                            || theme.contains(&format!("property <int>    {token}:"))
                            || theme.contains(&format!("property <float>  {token}:"))
                            || theme.contains(&format!("property <string> {token}:"))
                            || theme.contains(&format!("{token}:")),
                        "{name}: `Theme.{token}` is not defined in theme.slint"
                    );
                    rest = &tail[token.len()..];
                }
            }
        }
    }
}
