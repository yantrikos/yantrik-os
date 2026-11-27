//! "Prepare for me" on the desktop: the Think arm's one call that does the research and first
//! drafts for the next item that is due (#1416, slice 5). The engine, its limits and its tool
//! boundary are `yantrik_companion::prepare`; this file reads the switches and the agenda, and
//! hands over.
//!
//! - **The switch** is Settings → Initiative → "Prepare for me", **off by default**
//!   (`opt_in::prepare_for_me_on`). Private mode or the Brain switch off: nothing runs.
//! - **Triggers** are the agenda's (`triggers`): an event within 36 h with a linked note with open
//!   items, or any open item due within 48 h. At most two preparations a day, once per item.
//! - **Its own thread, the background class.** The job runs on a thread of its own
//!   (`CompanionService::spawn_prepare_for_me`), so the companion's worker, and a person's own
//!   turn, never wait behind it. Its model calls are self-started (`asked::self_started`), so the
//!   gateway classes them `own`; it waits while the person has a turn open or the account is tight
//!   (`asked::skip_self_started_then`, which also lists it under Mind View's "Paused while you're
//!   being answered") and goes on when the turn closes. One job at a time; no second scheduler.
//! - **Agents shows it** ([`paint`]): "Preparing: <item>", each step as it happens, with Pause /
//!   Resume and Stop ([`pause`], [`resume`], [`request_stop`]). Stop always works, mid-step.
//! - **The result reaches the person** as one card per item (`notifications.notice_prepared`):
//!   "Prepared for '<item>': shot list, script, 5 sources", with Open notes, Open folder, Discard
//!   and "Why you got this". Discard moves the folder to `Documents/Prepared/.discarded/`.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use chrono::Local;
use serde_json::{json, Value};
use yantrik_companion::agenda::notes_dir::parse;
use yantrik_companion::agenda::triggers::triggers;
use yantrik_companion::agenda::Switches;
use yantrik_companion::prepare::boundary::note_title;
use yantrik_companion::prepare::hook::Start;
use yantrik_companion::prepare::{discard, ledger, live, Item, Outcome, Report, Settings};
use yantrik_companion::CompanionService;
use yantrik_ipc_contracts::notifications::opt_in::prepare_for_me_on;
use yantrik_ipc_contracts::notifications::NOTICE_PREPARED;

/// What the job is listed as while it waits for a turn (Mind View's "Paused while you're being
/// answered: …").
const WHAT: &str = "preparing for you";
const POST_TIMEOUT: Duration = Duration::from_secs(5);

static LAST: Mutex<Option<Value>> = Mutex::new(None);

/// End the job that is running. Works whatever it is doing: nothing more is called or written.
pub fn request_stop() {
    live::request_stop();
}

/// Ends the job if one is running (Private mode came on).
pub fn request_stop_if_running() {
    if live::is_active() {
        request_stop();
    }
}

/// Why a job may not run **now**, read live from the three switches: Private mode on, the Brain
/// switch off, or "Prepare for me" off (or not yet known: unknown is not "on"). The job reads it
/// at every gate, so a paused or running job does not go on into a state that is now forbidden.
pub fn forbidden() -> Option<&'static str> {
    if crate::private_mode::is_on() {
        return Some("Private mode");
    }
    if !crate::agenda_tick::brain_on() {
        return Some("the Brain switch");
    }
    if !crate::notice_desk::initiative_known().as_ref().is_some_and(prepare_for_me_on) {
        return Some("the Prepare for me switch");
    }
    None
}

/// Called when any of the three switches may have changed (Private mode on, the settings read back
/// from the store): ends a running job now, instead of waiting for its next gate.
pub fn stop_if_forbidden() {
    if live::is_active() && forbidden().is_some() {
        request_stop();
    }
}

/// The person's Pause: the job waits before its next call or write.
pub fn pause() {
    live::request_pause(true);
}

/// The person's Resume.
pub fn resume() {
    live::request_pause(false);
}

/// What `describe shell` says under `agenda.prepared`: the switch and the newest jobs.
pub fn for_describe() -> Value {
    let mut v = LAST.lock().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_else(|| json!({ "switch": "off" }));
    if let (Some(o), Some(j)) = (v.as_object_mut(), live::snapshot()) {
        o.insert("running".into(), json!({ "item": j.title, "state": j.phase.word(), "steps": j.steps }));
    }
    v
}

fn publish(v: Value) {
    *LAST.lock().unwrap_or_else(|e| e.into_inner()) = Some(v);
}

/// Draw the job on the Agents screen: "Preparing: <item>", its steps, Pause / Resume and Stop.
pub fn paint(ui: &crate::App) {
    use slint::ComponentHandle;
    let g = ui.global::<crate::AgentsState>();
    let job = if crate::private_mode::is_on() { None } else { live::snapshot() };
    match job {
        Some(j) => {
            let title: String = j.title.chars().filter(|c| !c.is_control()).take(80).collect();
            let steps = j.steps.iter().map(|s| format!("\u{2713} {s}")).collect::<Vec<_>>().join("   ");
            if !g.get_prepare_active() {
                g.set_prepare_active(true);
            }
            let (t, st, by, ss) = (slint::SharedString::from(title), slint::SharedString::from(j.phase.word()), j.by_person, slint::SharedString::from(steps));
            if g.get_prepare_title() != t {
                g.set_prepare_title(t);
            }
            if g.get_prepare_state() != st {
                g.set_prepare_state(st);
            }
            if g.get_prepare_by_person() != by {
                g.set_prepare_by_person(by);
            }
            if g.get_prepare_steps() != ss {
                g.set_prepare_steps(ss);
            }
        }
        None => {
            if g.get_prepare_active() {
                g.set_prepare_active(false);
            }
        }
    }
}

/// The Pause, Resume and Stop buttons of the Agents screen.
pub fn wire(ui: &crate::App) {
    use slint::ComponentHandle;
    let g = ui.global::<crate::AgentsState>();
    g.on_prepare_pause(pause);
    g.on_prepare_resume(resume);
    g.on_prepare_stop(request_stop);
}

/// The titles of the unticked items of the linked note (never its text, never edited).
fn open_items(note_id: Option<&str>) -> Vec<String> {
    let Some(id) = note_id.filter(|i| !i.contains("..") && !i.starts_with('/')) else { return Vec::new() };
    let path = crate::agenda_tick::notes_dir().join(id);
    let Ok(text) = std::fs::read_to_string(&path) else { return Vec::new() };
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    parse(&text, &stem).1
}

pub fn documents() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join("Documents")
}

/// One pass from the Think arm, after the agenda's. Cheap when the switch is off, and it returns
/// at once: a job it starts runs on a thread of its own.
pub fn tick(companion: &CompanionService) {
    let settings = crate::notice_desk::initiative_known();
    let on = settings.as_ref().is_some_and(prepare_for_me_on);
    if !on {
        publish(json!({ "switch": "off" }));
        return;
    }
    let private = crate::private_mode::is_on();
    let brain = crate::agenda_tick::brain_on();
    let now = Local::now();
    let secs = now.timestamp() as f64;
    let day = now.format("%Y-%m-%d").to_string();
    let shown = |state: &str| json!({ "switch": "on", "state": state, "jobs": ledger::describe(&companion.db.conn(), secs) });
    if live::is_active() {
        publish(shown("running"));
        return;
    }
    let next = triggers(&companion.db.conn(), secs, Switches { private, enabled: brain }, 5)
        .ok()
        .and_then(|all| all.into_iter().find(|t| !ledger::exists(&companion.db.conn(), &t.id)));
    let Some(t) = next else {
        publish(shown(if private { "paused: private mode" } else if !brain { "off: the Brain switch" } else { "nothing to prepare" }));
        return;
    };
    // A person's turn is open (or the account is tight): not started now, and listed in Mind View
    // as held until the turn closes.
    if yantrik_ml::asked::skip_self_started_then(WHAT, || {}) {
        publish(shown("waiting: a turn is open"));
        return;
    }
    let item = Item { id: t.id.clone(), title: t.title.clone(), note_title: t.note_title.clone(), open_items: open_items(t.note_id.as_deref()) };
    let start = Start { private, brain, paused: std::sync::Arc::new(|| yantrik_ml::asked::skip_self_started_then(WHAT, || {})), allowed: std::sync::Arc::new(forbidden) };
    let run_settings = Settings { documents: documents(), day, now: secs, budget: Duration::from_secs(90) };
    let title = item.title.clone();
    let id = item.id.clone();
    let done = Box::new(move |outcome: Outcome| match outcome {
        Outcome::Ran(r) => {
            tracing::info!(item = %title, state = r.state, tokens = r.tokens, refused = r.refused.len(), "prepared");
            publish(json!({ "switch": "on", "state": r.line.clone(), "last": describe_report(&r) }));
            post_result(&id, &title, &r);
        }
        Outcome::Off(why) => publish(json!({ "switch": "on", "state": format!("off: {why}") })),
        Outcome::Waiting => publish(json!({ "switch": "on", "state": "waiting: not started" })),
        Outcome::NotAgain(why) => publish(json!({ "switch": "on", "state": why })),
    });
    match companion.spawn_prepare_for_me(item, start, run_settings, done) {
        Ok(()) => publish(shown("started")),
        Err(why) => publish(shown(&format!("not started: {why}"))),
    }
}

fn describe_report(r: &Report) -> Value {
    json!({ "line": r.line, "state": r.state, "files": r.files, "sources": r.sources.len(), "missing": r.missing })
}

/// What the card says about why it came.
fn why_words(title: &str) -> String {
    let shown: String = title.chars().filter(|c| !c.is_control()).take(80).collect();
    format!("You turned on \u{201c}Prepare for me\u{201d}, and \u{2018}{shown}\u{2019} is coming up. It only researched and wrote drafts: nothing was sent, posted or changed.")
}

/// The card for what a job left: one per item. Nothing is posted when it left nothing.
fn post_result(item_id: &str, title: &str, r: &Report) {
    if r.files.is_empty() && !r.note {
        return;
    }
    let params = json!({
        "item_id": item_id,
        "line": r.line,
        "folder": r.folder.to_string_lossy(),
        "note_title": if r.note { note_title(title) } else { String::new() },
        "why": why_words(title),
    });
    let answer = crate::notice_desk::call(NOTICE_PREPARED, params, POST_TIMEOUT);
    tracing::debug!(?answer, "the prepared card was posted");
}

/// A folder a card's button names, if it is one a job made: directly inside `Documents/Prepared`,
/// not hidden, not a link, still there.
fn prepared_folder(reference: &str) -> Result<PathBuf, String> {
    let folder = PathBuf::from(reference);
    let root = discard::prepared_root(&documents());
    let name = folder.file_name().and_then(|n| n.to_str()).filter(|n| !n.starts_with('.'));
    if folder.parent() != Some(root.as_path()) || name.is_none() || reference.contains("..") {
        return Err("that is not a folder Prepare for me made".into());
    }
    match std::fs::symlink_metadata(&folder) {
        Ok(m) if m.is_dir() && !m.file_type().is_symlink() => Ok(folder),
        _ => Err("that folder is not there any more".into()),
    }
}

/// Open folder: the folder to show in Files.
pub fn folder_to_open(reference: &str) -> Result<PathBuf, String> {
    prepared_folder(reference)
}

/// Discard: move the folder to `Prepared/.discarded/`. Never deletes.
pub fn discard_folder(reference: &str) -> Result<PathBuf, String> {
    let folder = prepared_folder(reference)?;
    discard::discard(&documents(), &folder)
}
