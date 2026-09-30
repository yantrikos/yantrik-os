//! Agents — one pane per agent, with its work inside it.
//!
//! An agent is one conversation with one mind (`<harness>:<conversation>`). This module keeps every
//! agent the desktop knows of: its state, its session — the person's prompts, the mind's text and
//! thinking, one card per tool call with that call's own output inside it — and the details the
//! Agents screen counts. The screen (`wire::agents`, `agents.slint`) and every popped-out agent
//! window draw from this one store, so they cannot disagree. See
//! `design/agents-workspace-2026-09-23.md`.
//!
//! # Feeding it
//!
//! The store is fed through [`Agents`], from any thread:
//!
//! | call | who calls it |
//! |---|---|
//! | `upsert_agent(AgentMeta)` | the host, when a conversation starts |
//! | `open_turn(&id, prompt)` / `close_turn(&id, ok)` | the host, around each turn |
//! | `text(&id, delta)` | the host, for each `harness.chunk` |
//! | `event(&id, &Event, Provenance)` | the host's `Chunk::Event` (`Reported`); anything the shell saw itself (`Verified`) |
//! | `command_started / command_output / command_finished` | the agent terminal: `Jobs::on_output(agent, job, bytes)` and `Jobs::on_finish` (`Verified`, bytes as read off the PTY) |
//! | `set_state(&id, State)` | the host (`HarnessGone`) and the approval card (`WaitingForYou`) |
//! | `jobs_waiting(&[AgentId])` | the shell's tick: which agents have a terminal job at a prompt (`WaitingForYou`, #182) |
//! | `approval_asked / approval_answered / approval_settled` | `control_approvals`: a request carrying this agent's token, and how it came out ([`settle_approvals`]) |
//! | `remove_agent(&id)` | Close |
//!
//! Every turn the shell sends a mind passes through `feed`, which does the host's part: the
//! prompt, the text, the events, and — for a harness that writes only text — its trail lines.

// The agent catalog — roles work can be handed to — and the reach each role's agent is held to
// (design/desk-and-mind-2026-09-23.md, section 5).
pub mod catalog;
pub mod feed;
pub mod handover;
pub mod launch;
pub mod model;
pub mod progress;
pub mod reaches;
pub mod route;
pub mod store;

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

pub use model::{AgentId, AgentMeta, ApprovalOutcome, CallState, Event, Provenance, RecipeOrigin, RoleMeta, State, Stream, Tab};
pub use store::{RowKey, Store};

/// The title every popped-out agent window starts with, so the window list can tell an agent's
/// window from anything else a task might be named after.
pub const WINDOW_TITLE_PREFIX: &str = "Agent · ";

/// Where sessions are kept: `$XDG_DATA_HOME/yantrik/agents`, or `~/.local/share/yantrik/agents`.
pub fn dir() -> PathBuf {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| {
            std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/")).join(".local").join("share")
        });
    data.join("yantrik").join("agents")
}

/// How often changed sessions are written out.
const SAVE_EVERY: Duration = Duration::from_secs(2);

/// The store, shared by everything in the shell that feeds it or draws it.
pub struct Agents {
    store: Mutex<Store>,
    dir: PathBuf,
    saved: Mutex<Instant>,
}

static AGENTS: OnceLock<Agents> = OnceLock::new();

/// The shell's agents, loaded from disk the first time anything asks.
///
/// Under test, a directory of the test run's own: the agent terminal feeds this store from every
/// command a test runs, and a test must neither read the person's saved sessions nor write into
/// them.
pub fn store() -> &'static Agents {
    AGENTS.get_or_init(|| {
        #[cfg(not(test))]
        let dir = dir();
        #[cfg(test)]
        let dir = std::env::temp_dir().join(format!("yantrik-agents-under-test-{}", std::process::id()));
        let store = Store::load(&dir, Box::new(model::now));
        // Every test in the binary shares this one store, many at once: under test it keeps them
        // all, so no test's agents are let go to make room for another's.
        #[cfg(test)]
        let store = store.keeping(usize::MAX);
        tracing::info!(agents = store.agents().len(), dir = %dir.display(), "Agents loaded");
        Agents { store: Mutex::new(store), dir, saved: Mutex::new(Instant::now()) }
    })
}

impl Agents {
    fn lock(&self) -> MutexGuard<'_, Store> {
        self.store.lock().unwrap_or_else(|e| e.into_inner())
    }

    // The input API. Each is the store's own, under the lock; see `store.rs` for the rules.

    pub fn upsert_agent(&self, meta: AgentMeta) {
        self.lock().upsert_agent(meta)
    }

    pub fn remove_agent(&self, id: &AgentId) -> bool {
        self.lock().remove_agent(id)
    }

    pub fn open_turn(&self, id: &AgentId, prompt: &str) {
        self.lock().open_turn(id, prompt)
    }

    /// A turn of the person's chat with a mind (the Lens): listed in Agents only if it does work.
    pub fn open_chat_turn(&self, id: &AgentId, prompt: &str) {
        self.lock().open_chat_turn(id, prompt)
    }

    pub fn text(&self, id: &AgentId, delta: &str) {
        self.lock().text(id, delta)
    }

    pub fn event(&self, id: &AgentId, event: &Event, provenance: Provenance) {
        self.lock().event(id, event, provenance)
    }

    pub fn close_turn(&self, id: &AgentId, ok: bool) {
        self.lock().close_turn(id, ok)
    }

    pub fn set_state(&self, id: &AgentId, state: State) {
        self.lock().set_state(id, state)
    }

    pub fn jobs_waiting(&self, waiting: &[AgentId]) {
        self.lock().jobs_waiting(waiting)
    }

    pub fn command_started(&self, id: &AgentId, job: &str, command: &str, cwd: &str) {
        self.lock().command_started(id, job, command, cwd)
    }

    pub fn command_output(&self, id: &AgentId, job: &str, bytes: &[u8]) {
        self.lock().command_output(id, job, bytes)
    }

    pub fn command_finished(&self, id: &AgentId, job: &str, command: &str, exit_code: Option<i32>, killed: bool) {
        self.lock().command_finished(id, job, command, exit_code, killed)
    }

    pub fn trail_call(&self, id: &AgentId, call: &crate::trail::ToolCall) {
        self.lock().trail_call(id, call)
    }

    pub fn note(&self, id: &AgentId, text: &str) {
        self.lock().note(id, text)
    }

    pub fn replace_text(&self, id: &AgentId, text: &str) {
        self.lock().replace_text(id, text)
    }

    pub fn approval_asked(&self, id: &AgentId, request: &str, what: &str) {
        self.lock().approval_asked(id, request, what)
    }

    pub fn approval_answered(&self, id: &AgentId, request: &str, allowed: bool) {
        self.lock().approval_answered(id, request, allowed)
    }

    pub fn approval_settled(&self, id: &AgentId, request: &str, outcome: ApprovalOutcome, record: &str) {
        self.lock().approval_settled(id, request, outcome, record)
    }

    pub fn question_answered(&self, id: &AgentId, request: &str, answer: &str) -> bool {
        self.lock().question_answered(id, request, answer)
    }

    pub fn question_closed(&self, id: &AgentId, request: &str, why: &str) {
        self.lock().question_closed(id, request, why)
    }

    /// Read the store. Keep it short: every feeder waits on the same lock.
    pub fn read<R>(&self, f: impl FnOnce(&Store) -> R) -> R {
        f(&self.lock())
    }

    pub fn revision(&self) -> u64 {
        self.lock().revision()
    }

    /// Write out what changed, at most every [`SAVE_EVERY`]. The files are written off the lock and
    /// off the UI thread.
    pub fn save_if_due(&self) {
        {
            let mut saved = self.saved.lock().unwrap_or_else(|e| e.into_inner());
            if saved.elapsed() < SAVE_EVERY {
                return;
            }
            *saved = Instant::now();
        }
        let (writes, deletes) = self.lock().take_dirty(&self.dir);
        if writes.is_empty() && deletes.is_empty() {
            return;
        }
        let dir = self.dir.clone();
        let _ = std::thread::Builder::new().name("agents-save".into()).spawn(move || {
            if let Err(e) = store::write_all(&dir, &writes, &deletes) {
                tracing::warn!(error = %e, dir = %dir.display(), "Could not save the agents' sessions");
            }
        });
    }
}

/// Bring every agent's waiting approvals up to date with the shell's approval store — the one
/// place a request is answered, whichever card the person pressed, the Lens's or the pane's.
/// `status_of` says where one request stands: `None` while it is still waiting, or how it came
/// out and the line it leaves. `control_approvals` calls this each time it redraws the cards, so
/// an answer, an expiry and a withdrawal all reach the pane on the same tick they reach the Lens.
///
/// The store's lock is not held while `status_of` runs: that reads the approval store, and
/// nothing holds both locks at once.
pub fn settle_approvals(status_of: impl Fn(&str) -> Option<(ApprovalOutcome, String)>) {
    let waiting: Vec<(AgentId, String)> = store().read(|s| {
        s.agents()
            .iter()
            .flat_map(|a| a.pending_approvals.iter().map(move |r| (a.meta.id.clone(), r.clone())))
            .collect()
    });
    for (agent, request) in waiting {
        if let Some((outcome, record)) = status_of(&request) {
            store().approval_settled(&agent, &request, outcome, &record);
        }
    }
}

/// The fields of an agent's `describe shell` entry that are the person's: what they asked it
/// (`title`), what it says it is doing, the commands it ran and the files it touched, and what is
/// waiting on the person for it. Another mind reading `describe` is told none of them — only that
/// the agent exists, which mind it is, how it stands and the counts — the same boundary as
/// `read_agent`, which lets an agent read itself and the agents it started, and nothing else.
pub const PERSONS_FIELDS: [&str; 6] = ["title", "status", "commands", "files", "pending_approvals", "running_jobs"];

/// `for_describe`'s answer as an agent may read it: each entry with [`PERSONS_FIELDS`] taken out
/// and marked `private`.
pub fn for_describe_by_an_agent() -> serde_json::Value {
    let mut view = for_describe();
    if let Some(agents) = view["agents"].as_array_mut() {
        for entry in agents.iter_mut() {
            if let Some(map) = entry.as_object_mut() {
                for field in PERSONS_FIELDS {
                    map.remove(field);
                }
                map.insert("private".into(), "the person's; an agent reads its own session with read_agent".into());
            }
        }
    }
    view
}

/// What `describe shell` says under `agents`: the counts per tab, and one entry per agent with its
/// state, whether it is waiting on the person, the commands the shell is running for it now, when
/// it last did anything, and what it has done.
pub fn for_describe() -> serde_json::Value {
    // Read before the store's lock is taken: the terminal has its own.
    let running = crate::control_agent_terminal::running_jobs();
    let now = model::now();
    store().read(|s| {
        let counts = s.counts();
        let agents: Vec<serde_json::Value> = s
            .list(Tab::All, None)
            .iter()
            .filter_map(|id| s.agent(id).map(|a| (a, s.details(id).unwrap_or_default())))
            .map(|(a, d)| {
                let jobs: Vec<&serde_json::Value> =
                    running.iter().filter(|(agent, _)| agent == &a.meta.id).map(|(_, job)| job).collect();
                let waiting_for_input = jobs.iter().any(|j| j["waiting_for_input"] == true);
                serde_json::json!({
                    "id": a.meta.id,
                    "mind": a.meta.mind,
                    "title": a.meta.title,
                    "state": a.state.key(),
                    "since": a.since,
                    "status": a.status,
                    // Waiting on the person: an approval card up for it, or one of its commands
                    // sitting at a prompt only the person can answer.
                    "needs_you": a.state == State::WaitingForYou || !a.pending_approvals.is_empty() || waiting_for_input,
                    "pending_approvals": a.pending_approvals,
                    // The commands the shell is running for it right now — its own terminal's.
                    "running_jobs": jobs,
                    // When anything last happened to it, and how long ago.
                    "last_activity": a.touched,
                    "last_activity_secs_ago": now.saturating_sub(a.touched),
                    "parent": a.meta.parent,
                    // The catalog role it was started as (`hand_off`), what it may touch and its
                    // budget — or null for an agent started on a mind alone.
                    "role": a.meta.role.as_ref().map(|r| serde_json::json!({
                        "id": r.id, "name": r.name, "reach": r.reach,
                        "budget": { "turns": r.turns, "minutes": r.minutes },
                    })),
                    // The recipe that handed it the work (an Agent step), or null.
                    "recipe": a.meta.recipe.as_ref().map(|r| serde_json::json!({ "id": r.id, "name": r.name })),
                    // Who it works for, as its row and its approval cards say it.
                    "on_behalf": a.meta.on_behalf(),
                    "children": s.children_of(&a.meta.id),
                    "turns": d.turns,
                    "calls": d.calls,
                    "failed_calls": d.failed_calls,
                    // What the shell itself ran, never what a harness says it ran.
                    "commands": d.commands.iter().map(|(line, exit, state)| serde_json::json!({
                        "command": line, "exit_code": exit, "state": state.key(),
                    })).collect::<Vec<_>>(),
                    "files": d.files,
                    "approvals": { "asked": d.approvals_asked, "answered": d.approvals_answered },
                    "one_conversation": !a.meta.conversations,
                })
            })
            .collect();
        serde_json::json!({
            "active": counts[0],
            "needs_you": counts[1],
            "complete": counts[2],
            "all": counts[3],
            "agents": agents,
        })
    })
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use serde_json::json;

    /// A chat reply links to its run only when the turn did work: talk stays with the chat.
    #[test]
    fn a_chat_reply_links_to_its_run_only_when_it_did_work() {
        let mind = AgentId::new("chatlink", AgentId::MAIN);
        store().open_chat_turn(&mind, "thanks!");
        store().text(&mind, "Any time.");
        assert_eq!(feed::chat_run(&mind), None, "talk: no run to link to");
        store().close_turn(&mind, true);
        store().open_chat_turn(&mind, "build a small game");
        store().event(
            &mind,
            &Event::ToolStart { call: "c1".into(), name: "os_act".into(), target: "editor".into(), args: json!({}) },
            Provenance::Reported,
        );
        assert_eq!(feed::chat_run(&mind).as_deref(), Some("chatlink:main#2"), "work: its run");
    }

    /// Another mind reading `describe` learns that an agent exists and how it stands — never what
    /// the person asked it, the commands it ran, the files it touched or what waits on the person.
    #[test]
    fn an_agent_reading_describe_is_told_none_of_the_persons_fields() {
        let pi = AgentId::new("pi", "c-private");
        store().open_turn(&pi, "Lunch with Sam, 12:30");
        store().approval_asked(&pi, "appr-private", "calendar.new_event");
        let described = for_describe_by_an_agent();
        let entry = described["agents"]
            .as_array()
            .and_then(|all| all.iter().find(|a| a["id"] == "pi:c-private"))
            .unwrap_or_else(|| panic!("pi is still listed: {described}"))
            .clone();
        for field in PERSONS_FIELDS {
            assert!(entry.get(field).is_none(), "{field} reached an agent: {entry}");
        }
        assert!(!entry.to_string().contains("Lunch with Sam"), "{entry}");
        assert_eq!(entry["mind"], "pi");
        assert_eq!(entry["needs_you"], true, "how it stands is not private");
        assert!(entry["private"].as_str().unwrap().contains("read_agent"));
        // The person's own describe is unchanged.
        let theirs = for_describe();
        assert!(theirs.to_string().contains("Lunch with Sam, 12:30"));
    }

    /// `describe shell` → `agents`: each agent's id, mind, title and state, whether it needs the
    /// person, the commands the shell is running for it now, when it last did anything, and the
    /// agents it started — and never a token.
    #[test]
    fn describe_says_who_needs_you_what_runs_and_when_it_last_did_anything() {
        let pi = AgentId::new("pi", "c-describe");
        let kid = AgentId::new("pi", "c-describe-kid");
        store().open_turn(&pi, "tidy the photos folder");
        let mut meta = AgentMeta::new(kid.clone(), "pi");
        meta.parent = Some(pi.clone());
        meta.role = catalog::Catalog::from_layers(&catalog::SHIPPED, &[]).find("reviewer").map(|r| r.meta());
        store().upsert_agent(meta);
        store().approval_asked(&pi, "appr-describe", "files.move");
        let jobs = crate::control_agent_terminal::jobs();
        let job = jobs.start(&pi, "sleep 30", None).unwrap();

        let described = for_describe();
        let entry = described["agents"]
            .as_array()
            .and_then(|all| all.iter().find(|a| a["id"] == "pi:c-describe"))
            .unwrap_or_else(|| panic!("pi is not listed: {described}"))
            .clone();
        assert_eq!((entry["mind"].clone(), entry["title"].clone()), (json!("pi"), json!("tidy the photos folder")));
        assert_eq!(entry["state"], "waiting_for_you", "{entry}");
        assert_eq!(entry["needs_you"], true, "{entry}");
        assert_eq!(entry["pending_approvals"], json!(["appr-describe"]));
        assert!(
            entry["running_jobs"].as_array().unwrap().iter().any(|j| j["job"] == job.0.as_str() && j["command"] == "sleep 30"),
            "{entry}"
        );
        assert!(entry["last_activity"].as_u64().is_some_and(|t| t > 1_700_000_000), "{entry}");
        assert!(entry["last_activity_secs_ago"].as_u64().is_some_and(|s| s < 60), "{entry}");
        assert_eq!(entry["children"], json!(["pi:c-describe-kid"]));
        assert!(entry["role"].is_null(), "an agent started on a mind alone has no role: {entry}");
        let kid_entry = described["agents"].as_array().unwrap().iter().find(|a| a["id"] == "pi:c-describe-kid").unwrap();
        assert_eq!((kid_entry["parent"].clone(), kid_entry["needs_you"].clone()), (json!("pi:c-describe"), json!(false)));
        assert_eq!(kid_entry["role"]["name"], "Reviewer", "each agent says its role: {kid_entry}");
        assert_eq!(kid_entry["role"]["reach"], "editor, documents and notes · at most safe");
        assert_eq!(kid_entry["role"]["budget"], json!({"turns": 4, "minutes": 15}));
        assert!(!described.to_string().contains("agent_token"), "{described}");
        jobs.kill(&pi, &job).unwrap();
    }
}
