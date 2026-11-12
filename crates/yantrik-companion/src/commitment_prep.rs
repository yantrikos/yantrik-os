//! Carrying out a promised thing: the executor behind the `commitment_prep` instinct (#1184).
//!
//! The person said once, "I'm sending Maya the Q4 deck on Thursday. The numbers are in the budget
//! spreadsheet." When the commitment's prep window opens this finds the files they named, opens
//! them **in Mind View**, and drafts the email in Email with the deck attached. It **never sends**:
//! the only tools it calls are `glob` and `app_action`, there is no send step, and Email publishes
//! no action that sends. Email cannot send attachments yet either, and the item says so.
//!
//! It is earned, not assumed: the first time, the item only records what would be prepared and
//! asks "Shall I get these ready for promises like this?" (`commitment_store`). Until the person
//! says yes, nothing is opened or drafted. It does not run in incognito, and it has a switch.
//!
//! Every step goes through [`Hands`], which is `ToolRegistry::execute`: the same permission, taint
//! and reach gates as any other call, each step in the audit. Nothing reaches the network, so it
//! works with research off.
//!
//! A step that fails for now (an app starting, Mind View briefly down, a file not saved yet) is
//! tried again with a growing pause while the window is open; one that fails for good is said and
//! left. What the item keeps is in `commitment_progress`: chosen paths and words, never tool output.

use std::time::Duration;

use serde_json::{json, Value};
use yantrik_companion_core::tools::{ToolContext, ToolRegistry};

use crate::anticipation::{AnticipationEngine, PreparationTrigger, PreparedItem, PreparedKind};
use crate::commitment_files::{named_files, NamedFile, Role};
use crate::commitment_hands::{app_name, Failed, Hands};
use crate::commitment_locate::{locate, search_roots, Located};
use crate::commitment_progress::{DraftRecord, FileRecord, Progress};
use crate::commitment_store as store;
use crate::world_model::{Commitment, CommitmentStatus};

/// Below this a detected promise is not prepared for: a guess is not worth opening files over.
pub const CONFIDENCE_FLOOR: f64 = 0.75;

/// How long one preparation may keep starting new calls. It runs beside the chat worker.
pub const BUDGET: Duration = Duration::from_secs(20);

/// The pause before the first retry; it doubles each time.
const RETRY_BASE_SECS: f64 = 300.0;

/// How many tries a promise gets before what is missing is left said.
const MAX_ATTEMPTS: u32 = 6;

/// How long a prepared item stays useful past the deadline it was prepared for.
const KEEP_AFTER_DEADLINE_SECS: f64 = 86_400.0;

/// What the shell says `where` is for a window in Mind View.
const MIND_VIEW: &str = "Mind View";

/// What the executor reads that is not in the database: where to look, and how long it may take.
#[derive(Debug, Clone)]
pub struct Settings {
    pub roots: Vec<String>,
    pub budget: Duration,
}

impl Settings {
    /// The person's own document folders, and the usual budget.
    pub fn for_person() -> Self {
        Self { roots: search_roots().iter().map(|r| r.to_string_lossy().into_owned()).collect(), budget: BUDGET }
    }
}

/// What came of asking for a commitment to be prepared.
#[derive(Debug)]
pub enum Outcome {
    /// Stored: the item, with its result in `data` (`state` says whether more is to be tried).
    Prepared(PreparedItem),
    /// Prepared before, by this run or an earlier one, or waiting for a yes: nothing was done again.
    AlreadyPrepared,
    /// Nothing to prepare (or not now), and why.
    Nothing(&'static str),
}

/// Prepare commitment `commitment_id` as of `now`, in the person's folders.
pub fn prepare(registry: &ToolRegistry, ctx: &ToolContext<'_>, commitment_id: i64, now: f64) -> Outcome {
    prepare_with(registry, ctx, commitment_id, now, &Settings::for_person())
}

/// [`prepare`] with the folders and the budget given. Idempotent: a commitment with a finished
/// item is not prepared again, whether the person has seen that item or not.
///
/// The database is `ctx.db`, and its connection is taken for one statement at a time: a tool call
/// writes its own audit line through the same connection, so holding it across a call would hang.
pub fn prepare_with(registry: &ToolRegistry, ctx: &ToolContext<'_>, commitment_id: i64, now: f64, settings: &Settings) -> Outcome {
    if ctx.incognito {
        return Outcome::Nothing("incognito: nothing is prepared");
    }
    if !store::enabled(&ctx.db.conn()) {
        return Outcome::Nothing("preparing is switched off");
    }
    let Some(commitment) = store::commitment(&ctx.db.conn(), commitment_id) else {
        return Outcome::Nothing("no such commitment");
    };
    if matches!(commitment.status, CommitmentStatus::Completed | CommitmentStatus::Cancelled | CommitmentStatus::Superseded) {
        return Outcome::Nothing("the commitment is already done");
    }
    if commitment.confidence < CONFIDENCE_FLOOR {
        return Outcome::Nothing("not sure enough that this was a promise");
    }
    let files = named_files(&commitment.related_entities);
    if files.is_empty() {
        return Outcome::Nothing("the commitment names no file");
    }
    let earlier = AnticipationEngine::for_commitment(&ctx.db.conn(), &commitment_id.to_string());
    let consent = store::consented(&ctx.db.conn());
    let mut progress = match earlier.as_ref().map(|i| (i, i.data["state"].as_str().unwrap_or("prepared"))) {
        Some((item, "retry")) if now < item.data["next_try"].as_f64().unwrap_or(0.0) => {
            return Outcome::Nothing("waiting to try again");
        }
        Some((item, "retry")) => Progress::from_data(&item.data).unwrap_or_else(|| Progress::start(&files)),
        Some((_, "proposed")) if consent => Progress::start(&files),
        Some(_) => return Outcome::AlreadyPrepared,
        None => Progress::start(&files),
    };
    if !consent {
        return Outcome::Prepared(propose(ctx, &commitment, &files, now));
    }

    let mut hands = Hands::new(registry, ctx, settings.budget);
    progress.attempts += 1;
    find_and_open(&mut hands, &mut progress, &settings.roots);
    let window_closed = now >= commitment.deadline;
    draft_email(&mut hands, &mut progress, &commitment, window_closed);

    let more = !progress.is_settled() && !window_closed && progress.attempts < MAX_ATTEMPTS;
    let (state, next_try) = if more {
        ("retry", Some(now + RETRY_BASE_SECS * f64::from(1u32 << (progress.attempts - 1))))
    } else {
        ("prepared", None)
    };
    let item = PreparedItem {
        id: item_id(commitment_id),
        trigger: PreparationTrigger::CommitmentDeadline { commitment_id: commitment_id.to_string() },
        kind: PreparedKind::CommitmentPrep,
        description: describe(&commitment, &progress, more),
        data: progress.to_data(commitment_id, &commitment.promisee, commitment.deadline, state, next_try),
        relevant_at: earlier.as_ref().map_or(now, |i| i.relevant_at),
        expires_at: now.max(commitment.deadline) + KEEP_AFTER_DEADLINE_SECS,
        surfaced: false,
        outcome: None,
        created_at: earlier.as_ref().map_or(now, |i| i.created_at),
    };
    AnticipationEngine::store(&ctx.db.conn(), &item);
    Outcome::Prepared(item)
}

fn item_id(commitment_id: i64) -> String {
    format!("commitment-prep-{commitment_id}")
}

/// Record what would be prepared, and ask once. Nothing is opened or drafted.
fn propose(ctx: &ToolContext<'_>, c: &Commitment, files: &[NamedFile], now: f64) -> PreparedItem {
    let first = !store::asked(&ctx.db.conn());
    let labels: Vec<&str> = files.iter().map(|f| f.label.as_str()).collect();
    let mut line = format!("{}: not prepared yet. I would open the {} in Mind View", for_promise(c), labels.join(" and the "));
    if let Some(sent) = files.iter().find(|f| f.role == Role::Attach) {
        line.push_str(&format!(" and draft the email with the {} attached", sent.label));
    }
    line.push_str(". Nothing would be sent.");
    if first {
        line.push(' ');
        line.push_str(store::QUESTION);
    }
    let item = PreparedItem {
        id: item_id(c.id),
        trigger: PreparationTrigger::CommitmentDeadline { commitment_id: c.id.to_string() },
        kind: PreparedKind::CommitmentPrep,
        description: line,
        data: json!({
            "commitment_id": c.id, "promisee": c.promisee, "deadline": c.deadline, "state": "proposed",
            "would_prepare": files.iter().map(|f| json!({ "label": f.label, "role": f.encode().split(':').next() })).collect::<Vec<_>>(),
            "asks": first.then_some(store::QUESTION), "sent": false,
        }),
        relevant_at: now,
        expires_at: now.max(c.deadline) + KEEP_AFTER_DEADLINE_SECS,
        surfaced: false,
        outcome: None,
        created_at: now,
    };
    AnticipationEngine::store(&ctx.db.conn(), &item);
    if first {
        store::mark_asked(&ctx.db.conn());
    }
    item
}

/// Find each file named, and open it in Mind View.
fn find_and_open(hands: &mut Hands<'_>, progress: &mut Progress, roots: &[String]) {
    for rec in progress.files.iter_mut().filter(|r| !r.settled) {
        let Some(kind) = rec.kind() else {
            rec.settled = true;
            continue;
        };
        if rec.path.is_none() {
            let file = NamedFile { role: if rec.is_attachment() { Role::Attach } else { Role::Reference }, label: rec.label.clone() };
            match locate(hands, &file, kind, roots) {
                Located::Found(path) => {
                    rec.path = Some(path);
                    rec.why = None;
                }
                Located::Later(words) => {
                    rec.why = Some(words);
                    continue;
                }
                // A file not saved yet may be there later; a label that fits many files will not
                // fit fewer.
                other => {
                    rec.why = other.words(&rec.label);
                    rec.settled = other != Located::Missing;
                    continue;
                }
            }
        }
        open_in_mind_view(hands, rec, kind.app(), kind.openable_extension());
    }
}

/// Where the shell says a launch landed, and if that is not Mind View, why the file is left alone.
fn landing(answer: &Value) -> Result<String, String> {
    match answer["where"].as_str() {
        Some(MIND_VIEW) => Ok(MIND_VIEW.to_string()),
        Some(_) => Err("apps open on your desktop with your current setting, so I didn't open the file".into()),
        None if answer["note"].as_str().is_some_and(|n| n.contains("already open on the person's desktop")) => {
            Err("it is already open on your desktop, so I left it alone".into())
        }
        None => Err("I couldn't tell where it opened, so I didn't open the file".into()),
    }
}

/// Open the chosen file in Mind View: the app first, read where it landed, and only if that is
/// Mind View, the file in it. Never on the person's desktop.
fn open_in_mind_view(hands: &mut Hands<'_>, rec: &mut FileRecord, app: &str, openable: &str) {
    let Some(path) = rec.path.clone() else { return };
    let readable = std::path::Path::new(&path).extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case(openable));
    if !readable {
        rec.opened = Some(false);
        rec.settled = true;
        rec.why = Some(format!("{} can't open that kind of file, so it is attached, not opened", app_name(app)));
        return;
    }
    let outcome = hands
        .act("shell", "open_app", json!({ "name": app }))
        .and_then(|answer| landing(&answer).map_err(Failed::Final))
        .and_then(|place| hands.act(app, "open", json!({ "path": path })).map(|_| place));
    match outcome {
        Ok(place) => {
            rec.opened = Some(true);
            rec.place = Some(place);
            rec.why = None;
            rec.settled = true;
        }
        Err(failed) => {
            rec.why = Some(failed.words().to_string());
            if !failed.is_later() {
                rec.opened = Some(false);
                rec.settled = true;
            }
        }
    }
}

/// Draft the email in Email with the thing being sent attached. No draft, and the reason, when
/// there is nothing to attach: no draft is better than one that pretends. While the window is
/// open a file not found yet may still turn up, so the draft waits for it.
fn draft_email(hands: &mut Hands<'_>, progress: &mut Progress, c: &Commitment, window_closed: bool) {
    if progress.draft.is_some() || progress.draft_settled {
        return;
    }
    let Some(rec) = progress.attachment() else { return };
    let Some(path) = rec.path.clone() else {
        // Nothing to attach for good: the file is settled as not found, or the window has closed.
        if rec.settled || window_closed {
            progress.draft_why = Some("no email was drafted, because there is nothing to attach".into());
            progress.draft_settled = true;
        }
        return;
    };
    let label = rec.label.clone();
    let to = if c.promisee.contains('@') { c.promisee.clone() } else { String::new() };
    let greeting = named(c).map(|who| format!("Hi {who},\n\n")).unwrap_or_default();
    let args = json!({ "to": to, "subject": label, "body": format!("{greeting}Here is the {label}.\n"), "attach": path });
    match hands.act("email", "compose", args) {
        Ok(_) => {
            progress.draft = Some(DraftRecord { to, subject: label, attached: path });
            progress.draft_why = None;
            progress.draft_settled = true;
        }
        Err(failed) => {
            progress.draft_why = Some(if failed.is_later() {
                format!("the email isn't drafted yet: {}", failed.words())
            } else {
                format!("couldn't draft the email: {}", failed.words())
            });
            progress.draft_settled = !failed.is_later();
        }
    }
}

/// The promisee, when it is a name.
fn named(c: &Commitment) -> Option<&str> {
    let who = c.promisee.trim();
    (!who.is_empty() && !["unknown", "you", "me"].contains(&who.to_lowercase().as_str())).then_some(who)
}

/// "For your promise to Maya on Thursday": the weekday is the person's own.
fn for_promise(c: &Commitment) -> String {
    use chrono::TimeZone;
    let day = chrono::Local.timestamp_opt(c.deadline as i64, 0).single().map(|t| t.format("%A").to_string());
    let to = named(c).map(|w| format!(" to {w}")).unwrap_or_default();
    let on = day.map(|d| format!(" on {d}")).unwrap_or_default();
    format!("For your promise{to}{on}")
}

fn files_word(n: usize) -> String {
    format!("{n} {}", if n == 1 { "file" } else { "files" })
}

/// One line for the brief to read from: what was done, and what was not, and that the draft cannot
/// be sent from Email while Email cannot send attachments.
fn describe(c: &Commitment, progress: &Progress, more_to_try: bool) -> String {
    let opened = progress.files.iter().filter(|f| f.opened == Some(true)).count();
    let mut line = format!("{}: ", for_promise(c));
    line.push_str(&if opened == 0 { "no files opened".to_string() } else { format!("{} opened in Mind View", files_word(opened)) });
    match &progress.draft {
        Some(d) => line.push_str(&format!(
            ". The draft is in Email with the {} attached, but Email can't send attachments yet, so it can't be sent from there. Nothing sent.",
            d.subject
        )),
        None => line.push_str(". No draft made. Nothing sent."),
    }
    let missing = progress.missing();
    if !missing.is_empty() {
        line.push_str(&format!(" Not done: {}.", missing.join("; ")));
    }
    if more_to_try {
        line.push_str(" I'll try again.");
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
