//! Private mode at a service's door (#924): `Surface::answer`, the dispatch weather-service,
//! system-monitor-service, notifications-service and calendar-service answer `app.describe` and
//! `app.act` through, reads `privacy.json` for every call, as the shell publishes it.
//!
//! Its own test binary, so it can own `HOME` without moving it under any other test.

use std::sync::{Mutex, MutexGuard, OnceLock};

use serde_json::{json, Value};
use yantrik_ipc_transport::privacy;
use std::sync::atomic::{AtomicBool, Ordering};

use yantrik_surface::{
    answer_later, Action, LocalRegistry, Param, Surface, View, PRIVATE_SUMMARY, WITHHELD, WITHHELD_REFUSAL,
};

/// Set by a test: the next describe turns Private mode on while it runs.
static FLIP_IN_DESCRIBE: AtomicBool = AtomicBool::new(false);

const SENTINEL: &str = "Reminder: call Priya SENTINEL-924";

fn leaks(answer: &Value) -> bool {
    answer.to_string().contains("SENTINEL-924")
}

/// This binary's home, set once, and the lock that keeps one test at a time on the switch.
fn machine() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    static HOME: OnceLock<()> = OnceLock::new();
    let held = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    HOME.get_or_init(|| {
        let home = std::env::temp_dir().join(format!("yantrik-surface-private-{}", std::process::id()));
        std::fs::create_dir_all(&home).expect("a home of our own");
        std::env::set_var("HOME", home);
    });
    privacy::publish(false, 1).expect("privacy.json written");
    held
}

fn reminders() -> Surface {
    Surface::new("reminders")
        .describe(|| {
            if FLIP_IN_DESCRIBE.swap(false, Ordering::SeqCst) {
                privacy::publish(true, 1).expect("privacy.json written");
            }
            View::new(format!("Reminders — next: {SENTINEL}")).with("next", SENTINEL)
        })
        .action(Action::new("read_next", "Answer with the next reminder").risk("safe"), |_| {
            Ok(json!({ "next": SENTINEL }))
        })
        .action(
            Action::new("open_reminder", "Open a reminder by id").risk("safe").arg(Param::text("id")),
            |_| Err(format!("no reminder like that; the next is {SENTINEL}")),
        )
        .action(Action::new("go_private", "Turn Private mode on mid-call").risk("safe"), |_| {
            privacy::publish(true, 1).map_err(|e| e.to_string())?;
            Ok(json!({ "ok": true }))
        })
        .action(Action::new("snooze", "Snooze the next reminder").risk("safe"), |_| Ok(json!({ "snoozed": true })))
        .action(Action::new("sync", "Fetch reminders, off the worker").risk("safe"), |_| {
            // The person turns Private mode on while the work runs.
            let work = || {
                privacy::publish(true, 1).map_err(|e| e.to_string())?;
                Ok(json!({ "fetched": 1 }))
            };
            answer_later(work).map(|()| json!("replaced")).or_else(|work| work())
        })
}

fn answer(surface: &Surface, method: &str, params: Value) -> Value {
    surface.answer(method, &params, None).expect("a surface method").expect("answered")
}

#[test]
fn a_services_describe_and_act_follow_private_mode_from_the_next_call() {
    let _machine = machine();
    let surface = reminders();
    assert_eq!(answer(&surface, "app.describe", json!({}))["state"]["next"], SENTINEL);

    privacy::publish(true, 1).unwrap();
    let hidden = answer(&surface, "app.describe", json!({}));
    assert_eq!(hidden["private"], true);
    assert_eq!(hidden["summary"], PRIVATE_SUMMARY);
    assert_eq!(hidden["state"], json!({ "private": true }), "a service has no window and keeps nothing");
    assert!(!leaks(&hidden), "{hidden}");
    let acted = answer(&surface, "app.act", json!({ "action": "snooze", "args": {} }));
    assert_eq!(acted["accepted"], true, "the person's own call still runs");
    assert_eq!(acted["result"], Value::Null, "and what it answered is withheld from an unvouched caller");
    assert_eq!(acted["private"], true);
    assert!(!leaks(&acted), "{acted}");

    privacy::publish(false, 1).unwrap();
    assert_eq!(answer(&surface, "app.describe", json!({}))["state"]["next"], SENTINEL);
}

#[test]
fn a_services_work_finished_later_is_read_back_under_private_mode_as_it_is_then() {
    let _machine = machine();
    let surface = reminders();
    let reply = answer(&surface, "app.act", json!({ "action": "sync", "args": {} }));
    assert_eq!(reply["result"], Value::Null, "{reply}");
    assert_eq!(reply["withheld"], WITHHELD);
    assert_eq!(reply["private"], true, "Private mode came on while the work ran: {reply}");
    assert!(!leaks(&reply), "{reply}");
    privacy::publish(false, 1).unwrap();
}

/// #926 review, L1: Private mode turned on while a service builds its answer holds for it.
#[test]
fn private_mode_turned_on_during_a_services_call_holds_for_that_answer() {
    let _machine = machine();
    let surface = reminders();
    let acted = answer(&surface, "app.act", json!({ "action": "go_private", "args": {} }));
    assert_eq!(acted["private"], true, "{acted}");
    assert!(!leaks(&acted), "{acted}");

    privacy::publish(false, 1).unwrap();
    FLIP_IN_DESCRIBE.store(true, Ordering::SeqCst);
    let described = answer(&surface, "app.describe", json!({}));
    assert_eq!(described["private"], true, "{described}");
    assert_eq!(described["summary"], PRIVATE_SUMMARY);
    assert!(!leaks(&described), "{described}");
    privacy::publish(false, 1).unwrap();
}

/// #926 review, L2: a privacy source can only add. One that says "not private" does not make a
/// registry's own reads ignore the file the person's Private mode is in.
#[test]
fn a_privacy_source_cannot_make_a_surface_less_private_than_the_file() {
    let _machine = machine();
    let mut reg = LocalRegistry::new("reminders");
    reg.set_describe(Box::new(|| View::new(SENTINEL).with("next", SENTINEL)));
    reg.set_privacy_source(Box::new(|| false));
    privacy::publish(true, 1).unwrap();
    let described = reg.describe();
    assert_eq!(described["private"], true, "{described}");
    assert!(!leaks(&described), "{described}");
    privacy::publish(false, 1).unwrap();
    assert_eq!(reg.describe()["state"]["next"], SENTINEL);
}

/// #926 review, M1: at a service's door, in Private mode, a caller the kernel does not vouch for
/// (no peer, here) gets the action run and nothing it said.
#[test]
fn an_unvouched_caller_at_a_services_door_gets_neither_the_result_nor_the_refusal_text() {
    let _machine = machine();
    let surface = reminders();
    assert_eq!(answer(&surface, "app.act", json!({ "action": "read_next", "args": {} }))["result"]["next"], SENTINEL);
    privacy::publish(true, 1).unwrap();
    let read = answer(&surface, "app.act", json!({ "action": "read_next", "args": {} }));
    assert_eq!((read["accepted"].clone(), read["result"].clone()), (json!(true), Value::Null), "{read}");
    assert!(!leaks(&read), "{read}");
    let refused = surface
        .answer("app.act", &json!({ "action": "open_reminder", "args": { "id": "r1" } }), None)
        .expect("a surface method")
        .unwrap_err();
    assert_eq!(refused.message, WITHHELD_REFUSAL);
    privacy::publish(false, 1).unwrap();
}
