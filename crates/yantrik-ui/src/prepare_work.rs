//! "Prepare for me" on the desktop: the Think arm's one call that does the research and first
//! drafts for the next item that is due (#1416, slice 5). The engine, its limits and its tool
//! boundary are `yantrik_companion::prepare`; this file reads the switches and the agenda, and
//! hands over.
//!
//! - **The switch** is Settings → Initiative → "Prepare for me", **off by default**
//!   (`opt_in::prepare_for_me_on`). Private mode or the Brain switch off: nothing runs.
//! - **Triggers** are the agenda's (`triggers`): an event within 36 h with a linked note with open
//!   items, or any open item due within 48 h. At most two preparations a day, once per item.
//! - **Background class.** The job's model calls are self-started (`asked::self_started`), so the
//!   gateway classes them `own`, and it waits while the person has a turn open, or the account is
//!   tight (`asked::keeps_for_asked`, the shell's check of the gateway's "now").
//! - **Stop.** [`request_stop`] ends a running job between steps.
//!
//! It runs on the companion's worker, one job at a time, for at most
//! `yantrik_companion::prepare::BUDGET`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use chrono::Local;
use serde_json::{json, Value};
use yantrik_companion::agenda::notes_dir::parse;
use yantrik_companion::agenda::triggers::triggers;
use yantrik_companion::agenda::Switches;
use yantrik_companion::prepare::{ledger, Controls, Item, Outcome, Settings};
use yantrik_companion::CompanionService;
use yantrik_ipc_contracts::notifications::opt_in::prepare_for_me_on;

static STOP: AtomicBool = AtomicBool::new(false);
static LAST: Mutex<Option<Value>> = Mutex::new(None);

/// End the job that is running, between its steps. (Mind View's Stop reaches it here.)
pub fn request_stop() {
    STOP.store(true, Ordering::SeqCst);
}

/// What `describe shell` says under `agenda.prepared`: the switch and the newest jobs.
pub fn for_describe() -> Value {
    LAST.lock().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_else(|| json!({ "switch": "off" }))
}

fn publish(v: Value) {
    *LAST.lock().unwrap_or_else(|e| e.into_inner()) = Some(v);
}

/// The titles of the unticked items of the linked note (never its text, never edited).
fn open_items(note_id: Option<&str>) -> Vec<String> {
    let Some(id) = note_id.filter(|i| !i.contains("..") && !i.starts_with('/')) else { return Vec::new() };
    let path = crate::agenda_tick::notes_dir().join(id);
    let Ok(text) = std::fs::read_to_string(&path) else { return Vec::new() };
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    parse(&text, &stem).1
}

/// One pass from the Think arm, after the agenda's. Cheap when the switch is off.
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
    let next = triggers(&companion.db.conn(), secs, Switches { private, enabled: brain }, 5)
        .ok()
        .and_then(|all| all.into_iter().find(|t| !ledger::exists(&companion.db.conn(), &t.id)));
    let shown = |state: &str| json!({ "switch": "on", "state": state, "jobs": ledger::describe(&companion.db.conn(), secs) });
    let Some(t) = next else {
        publish(shown(if private { "paused: private mode" } else if !brain { "off: the Brain switch" } else { "nothing to prepare" }));
        return;
    };
    let item = Item { id: t.id.clone(), title: t.title.clone(), note_title: t.note_title.clone(), open_items: open_items(t.note_id.as_deref()) };
    let paused = || yantrik_ml::asked::keeps_for_asked();
    let stopped = || STOP.load(Ordering::SeqCst);
    let controls = Controls { enabled: true, private, brain, paused: &paused, stopped: &stopped };
    let documents = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_default().join("Documents");
    let run_settings = Settings { documents, day, now: secs, budget: Duration::from_secs(90) };
    STOP.store(false, Ordering::SeqCst);
    let outcome = companion.prepare_for_me(&item, &controls, &run_settings);
    let state = match &outcome {
        Outcome::Off(why) => format!("off: {why}"),
        Outcome::Waiting => "waiting: a turn is open".to_string(),
        Outcome::NotAgain(why) => (*why).to_string(),
        Outcome::Ran(r) => {
            tracing::info!(item = %t.title, state = r.state, tokens = r.tokens, refused = r.refused.len(), "prepared");
            r.line.clone()
        }
    };
    publish(shown(&state));
}
