//! Carrying out a promised thing: the executor behind the `commitment_prep` instinct (#1184).
//!
//! The person said once, "I'm sending Maya the Q4 deck on Thursday. The numbers are in the budget
//! spreadsheet." When the commitment's prep window opens this finds the files they named, opens them
//! **in Mind View**, and drafts the email in Email with the deck attached. It **never sends**: the
//! only tools it calls are `glob` and `app_action`, there is no send step, and Email publishes no
//! action that sends. Sending stays the person's, at a card.
//!
//! Every step goes through [`ToolRegistry::execute`], so the same permission, taint and reach gates
//! run as for any other call and each step is written to the audit. Nothing here is matched against
//! words in a prompt, and nothing reaches the network, so it works with research off. Under the
//! reach-by-declaration gate (#1182) every call is local: `app_action` on the shell, on the app that
//! opens the file, and on Email's `compose`, none of which declares an address or a command line.
//!
//! The result stays on the [`PreparedItem`] (`data`): the files opened and where, the draft and its
//! attachment, and, in the person's words, what could not be found. A missing file is said, never
//! papered over: a missing *attachment* means no draft, because a draft that says "here is the deck"
//! without the deck is a lie the person finds at send time.

use rusqlite::Connection;
use serde_json::{json, Value};
use yantrik_companion_core::tools::{ToolContext, ToolRegistry};

use crate::anticipation::{AnticipationEngine, PreparationTrigger, PreparedItem, PreparedKind};
use crate::commitment_files::{named_files, FileKind, NamedFile, Role};
use crate::world_model::{Commitment, CommitmentStatus, WorldModel};

/// Where the files are looked for.
const SEARCH_ROOT: &str = "~";

/// How long a prepared item stays useful past the deadline it was prepared for.
const KEEP_AFTER_DEADLINE_SECS: f64 = 86_400.0;

/// What came of asking for a commitment to be prepared.
#[derive(Debug)]
pub enum Outcome {
    /// Prepared and stored: the item, with its result in `data`.
    Prepared(PreparedItem),
    /// This commitment was prepared before, by this run or an earlier one: nothing was done again.
    AlreadyPrepared,
    /// Nothing to prepare, and why.
    Nothing(&'static str),
}

/// One tool call the preparation made, kept on the item beside the audit's own line for it.
fn step(tool: &str, args: &Value, said: &str) -> Value {
    json!({ "tool": tool, "args": args, "ok": true, "said": said.chars().take(200).collect::<String>() })
}

/// The tools a preparation may use, and the one door they all go through.
struct Hands<'a> {
    registry: &'a ToolRegistry,
    ctx: &'a ToolContext<'a>,
    steps: Vec<Value>,
}

impl Hands<'_> {
    fn call(&mut self, tool: &str, args: Value) -> String {
        let said = self.registry.execute(self.ctx, tool, &args);
        self.steps.push(step(tool, &args, &said));
        said
    }

    /// `app_action`; `Ok` with what the app said when it took the action, `Err` with the sentence
    /// it refused with.
    fn act(&mut self, app: &str, action: &str, args: Value) -> Result<String, String> {
        let said = self.call("app_action", json!({ "app": app, "action": action, "args": args }));
        let took = said.starts_with(&format!("{app}.{action} \u{2192}")) || said.starts_with(&format!("{app}.{action} accepted"));
        if !took {
            if let Some(last) = self.steps.last_mut() {
                last["ok"] = json!(false);
            }
        }
        if took { Ok(said) } else { Err(said) }
    }
}

/// A file as found on disk.
struct Found {
    file: NamedFile,
    kind: FileKind,
    path: String,
}

/// Prepare commitment `commitment_id` as of `now`. Idempotent: a commitment with an item is not
/// prepared again, whether the person has seen that item or not.
pub fn prepare(
    conn: &Connection,
    registry: &ToolRegistry,
    ctx: &ToolContext<'_>,
    commitment_id: i64,
    now: f64,
) -> Outcome {
    let Some(commitment) = WorldModel::commitment(conn, commitment_id) else {
        return Outcome::Nothing("no such commitment");
    };
    if matches!(commitment.status, CommitmentStatus::Completed | CommitmentStatus::Cancelled | CommitmentStatus::Superseded) {
        return Outcome::Nothing("the commitment is already done");
    }
    if AnticipationEngine::for_commitment(conn, &commitment_id.to_string()).is_some() {
        return Outcome::AlreadyPrepared;
    }
    let files = named_files(&commitment.related_entities);
    if files.is_empty() {
        return Outcome::Nothing("the commitment names no file");
    }
    let sends_something = files.iter().any(|f| f.role == Role::Attach);

    let mut hands = Hands { registry, ctx, steps: Vec::new() };
    let mut missing: Vec<String> = Vec::new();
    let mut found: Vec<Found> = Vec::new();
    for file in files {
        let Some(kind) = file.kind() else { continue };
        match locate(&mut hands, &file, kind) {
            Some(path) => found.push(Found { file, kind, path }),
            None => missing.push(format!("couldn't find the {}", file.label)),
        }
    }

    let opened: Vec<Value> = found.iter().map(|f| open_in_mind_view(&mut hands, f)).collect();
    let attachment = found.iter().find(|f| f.file.role == Role::Attach);
    let draft = draft_email(&mut hands, &commitment, attachment, sends_something, &mut missing);

    let item = PreparedItem {
        id: format!("commitment-prep-{commitment_id}"),
        trigger: PreparationTrigger::CommitmentDeadline { commitment_id: commitment_id.to_string() },
        kind: PreparedKind::CommitmentPrep,
        description: describe(&commitment, &opened, &draft, &missing),
        data: json!({
            "commitment_id": commitment_id,
            "promisee": commitment.promisee,
            "deadline": commitment.deadline,
            "opened": opened,
            "attachment": attachment.map(|a| json!({ "label": a.file.label, "path": a.path })),
            "draft": draft,
            "missing": missing,
            "sent": false,
            "steps": hands.steps,
        }),
        relevant_at: now,
        expires_at: now.max(commitment.deadline) + KEEP_AFTER_DEADLINE_SECS,
        surfaced: false,
        outcome: None,
        created_at: now,
    };
    AnticipationEngine::store(conn, &item);
    Outcome::Prepared(item)
}

/// Where the file is: the newest file of its kind whose name holds every distinguishing word of its
/// label ("Q4" for "Q4 deck"), trying the kind's extensions best first.
fn locate(hands: &mut Hands<'_>, file: &NamedFile, kind: FileKind) -> Option<String> {
    let words = file.distinguishing_words();
    for ext in kind.extensions() {
        let said = hands.call("glob", json!({ "pattern": format!("**/*.{ext}"), "path": SEARCH_ROOT }));
        // The tool lists newest first, one path to a line, then a blank line and a count.
        let hit = said.lines().take_while(|l| !l.trim().is_empty()).find(|path| {
            let stem = std::path::Path::new(path).file_stem().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
            path.starts_with('/') && words.iter().all(|w| stem.contains(w.as_str()))
        });
        if let Some(path) = hit {
            return Some(path.to_string());
        }
    }
    None
}

/// Open a found file in Mind View: the app first (the shell routes a launch the companion makes into
/// Mind View, and fails it, saying why, if Mind View is down — it never lands on the person's
/// desktop), then the file in that app. A file the app cannot read, or an app the person already has
/// open on their desktop, is said, and not driven.
fn open_in_mind_view(hands: &mut Hands<'_>, f: &Found) -> Value {
    let app = f.kind.app();
    let with = |extra: Value| json!({ "label": f.file.label, "path": f.path, "app": app, "result": extra });
    let readable = std::path::Path::new(&f.path)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case(f.kind.openable_extension()));
    if !readable {
        return with(json!({ "opened": false, "why": format!("{app} cannot open this kind of file; it is attached, not opened") }));
    }
    let launched = match hands.act("shell", "open_app", json!({ "name": app })) {
        Ok(said) => said,
        Err(why) => return with(json!({ "opened": false, "why": why })),
    };
    if launched.contains("already open on the person's desktop") {
        return with(json!({ "opened": false, "why": "already open on your desktop, so it was left alone" }));
    }
    match hands.act(app, "open", json!({ "path": f.path })) {
        Ok(_) => with(json!({ "opened": true, "in": "mind_view" })),
        Err(why) => with(json!({ "opened": false, "why": why })),
    }
}

/// Draft the email in Email, with the thing being sent attached. `None`, and a line in `missing`,
/// when there is nothing to attach: no draft is better than one that pretends.
fn draft_email(
    hands: &mut Hands<'_>,
    c: &Commitment,
    attachment: Option<&Found>,
    sends_something: bool,
    missing: &mut Vec<String>,
) -> Value {
    let Some(attachment) = attachment else {
        if sends_something {
            missing.push("no email was drafted, because there is nothing to attach".into());
        }
        return Value::Null;
    };
    let to = if c.promisee.contains('@') { c.promisee.clone() } else { String::new() };
    let who = if c.promisee.is_empty() || c.promisee == "unknown" { "there".to_string() } else { c.promisee.clone() };
    let label = &attachment.file.label;
    let args = json!({
        "to": to,
        "subject": label,
        "body": format!("Hi {who},\n\nHere is the {label}.\n"),
        "attach": attachment.path,
    });
    match hands.act("email", "compose", args) {
        Ok(_) => json!({
            "app": "email", "state": "draft", "to": to, "for": c.promisee,
            "subject": label, "attached": attachment.path,
        }),
        Err(why) => {
            missing.push(format!("couldn't draft the email: {why}"));
            Value::Null
        }
    }
}

/// One line for the brief to read from: what was done, and what was not.
fn describe(c: &Commitment, opened: &[Value], draft: &Value, missing: &[String]) -> String {
    let n = opened.iter().filter(|o| o["result"]["opened"] == true).count();
    let mut line = format!("For {}: {n} file(s) opened in Mind View", c.promisee);
    line.push_str(if draft.is_null() { ", no draft." } else { ", draft ready with the file attached. Nothing sent." });
    if !missing.is_empty() {
        line.push(' ');
        line.push_str(&missing.join("; "));
        line.push('.');
    }
    line
}

/// The trigger the open-loops scan hands the instincts for a commitment inside its prep window.
pub fn approaching_trigger(c: &Commitment) -> Value {
    json!({
        "trigger_type": "commitment_approaching",
        "commitment_id": c.id,
        "action": c.action,
        "promisor": c.promisor,
        "promisee": c.promisee,
        "deadline": c.deadline,
        "file_count": named_files(&c.related_entities).len(),
        "urgency": 0.6,
    })
}

/// The commitment an urge asks to have prepared, if it is a `commitment_prep` urge: the shell sends
/// those here instead of to the model.
pub fn commitment_to_prepare(urge: &crate::types::UrgeSpec) -> Option<i64> {
    (urge.context.get("executor")?.as_str()? == yantrik_companion_instincts::commitment_prep::EXECUTOR)
        .then(|| urge.context.get("commitment_id")?.as_i64())
        .flatten()
}

impl crate::companion::CompanionService {
    /// Prepare a commitment with this companion's own tools and database, as of now. The ceiling is
    /// Standard, which is all `glob` and `app_action` need; nothing here is above it.
    pub fn prepare_commitment(&self, commitment_id: i64) -> Outcome {
        let ctx = ToolContext {
            db: &self.db,
            max_permission: self.max_permission_now().min(yantrik_companion_core::permission::PermissionLevel::Standard),
            registry_metadata: None,
            task_manager: Some(&self.task_manager),
            incognito: self.incognito,
            agent_spawner: None,
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        prepare(&self.db.conn(), &self.registry, &ctx, commitment_id, now)
    }
}
