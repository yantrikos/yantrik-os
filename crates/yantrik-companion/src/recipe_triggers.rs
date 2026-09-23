//! What starts a recipe on its own: its schedule, an event, another recipe completing (#187).
//!
//! Triggers were stored and never fired: `get_enabled_triggers` had no caller, so "every morning
//! at 8" never started. They fire from the executor's clock now — the shell's worker asks
//! [`fire_due`] every [`crate::recipe_executor::CLOCK_SECS`], a host with no worker asks it from
//! `recipe_executor::tick` — and from the two things that happen rather than come due: an event
//! the desktop records ([`fire_event`], from `CompanionService::push_event`) and a recipe
//! completing ([`fire_after`], from the executor's `finish`). `RecipeComplete` is what chains
//! formations: the Council, then whatever reads its verdict.
//!
//! A start by a trigger is **unattended**. Nobody at the desk agreed to it, so the run is given no
//! leave for agents — any it had is taken away — and an Agent step in it asks the person on a card
//! before any role above `safe`, bound to the role's definition, as the consent rules of #193 say.
//!
//! The rules the clock keeps:
//!
//! - A schedule is read on the machine's own clock ([`crate::recipe_time`]). It counts from when
//!   its recipe was made until it first fires, then from when it last fired. A time missed while
//!   the machine was off or asleep fires once on waking, if it is less than [`CATCH_UP_SECS`] old;
//!   an older one is let go.
//! - One run at a time: a trigger whose recipe — or a run of it — is still in flight does not
//!   start another. A schedule fires once that run is done, if it is still inside its catch-up;
//!   an event that finds it busy is let go.
//! - A recipe is not started by its own completion, and a chain is at most [`CHAIN_MOST`] long:
//!   two recipes that start each other would otherwise never stop.
//! - What started a run is kept on it ([`TRIGGER_VAR`]), for the Recipes screen and `describe`.

use std::collections::HashMap;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::cron_mini::CronSpec;
use crate::recipe::{Recipe, RecipeStore, Trigger, TriggerType, FROM_VAR};

/// How old a scheduled time missed while the machine was off may be and still fire on waking.
pub const CATCH_UP_SECS: f64 = 2.0 * 3600.0;

/// How many recipes long a chain of `RecipeComplete` triggers may grow.
pub const CHAIN_MOST: u64 = 5;

/// What started a run, on a run a trigger started ([`Started`]).
pub const TRIGGER_VAR: &str = "_trigger";

/// How far down a chain of `RecipeComplete` triggers a run is: 1 for the first recipe another's
/// completion started.
pub const CHAIN_VAR: &str = "_chain";

/// What started a run, kept on it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Started {
    /// The trigger's row.
    pub trigger: i64,
    /// In words: "its schedule, `0 8 * * *`".
    pub by: String,
    /// When.
    pub at: f64,
}

impl Started {
    /// What started this run, if a trigger did.
    pub fn of(vars: &HashMap<String, Value>) -> Option<Started> {
        vars.get(TRIGGER_VAR).and_then(|v| serde_json::from_value(v.clone()).ok())
    }
}

/// A trigger in words, for the Recipes screen and `describe`: "on its schedule, `0 8 * * *`, on
/// this machine's clock".
pub fn said(kind: &TriggerType) -> String {
    match kind {
        TriggerType::Manual => "by hand".to_string(),
        TriggerType::Cron { expression } => format!("on its schedule, `{expression}`, on this machine's clock"),
        TriggerType::Event { event_type, filter: None } => format!("on the event `{event_type}`"),
        TriggerType::Event { event_type, filter: Some(f) } => format!("on the event `{event_type}` matching {f}"),
        TriggerType::RecipeComplete { recipe_id } => format!("when {recipe_id} completes"),
    }
}

/// Start what the schedules say is due at `now`, on the machine's clock. Returns the runs started.
pub fn fire_due(conn: &Connection, now: f64) -> Vec<String> {
    fire_due_in(&chrono::Local, conn, now)
}

/// [`fire_due`], with the zone schedules are read in.
pub fn fire_due_in<Tz: chrono::TimeZone>(tz: &Tz, conn: &Connection, now: f64) -> Vec<String> {
    let mut started = Vec::new();
    for t in RecipeStore::triggers(conn) {
        let TriggerType::Cron { expression } = &t.kind else { continue };
        let Some(spec) = CronSpec::parse(expression) else {
            tracing::debug!(trigger = t.id, expression = %expression, "A recipe's schedule cannot be read");
            continue;
        };
        let from = if t.last_fired > 0.0 { t.last_fired } else { t.since };
        let Some(at) = crate::recipe_time::cron_first_in(tz, &spec, from.max(now - CATCH_UP_SECS), now) else { continue };
        if busy(conn, &t.recipe_id) {
            tracing::info!(trigger = t.id, recipe_id = %t.recipe_id, "A schedule came due while its recipe still runs: it waits");
            continue;
        }
        let by = format!("its schedule, `{expression}`, at {}", crate::recipe_time::clock_text_in(tz, at));
        if let Some(run) = start(conn, &t, Map::new(), &by, now) {
            started.push(run);
        }
    }
    started
}

/// Start what an event the desktop recorded triggers: its type (`system/network`) and what it
/// carried. The run is given `event_type`, `event` and — when the event has one — `event_text`.
pub fn fire_event(conn: &Connection, event_type: &str, data: &Value, now: f64) -> Vec<String> {
    let mut started = Vec::new();
    for t in RecipeStore::triggers(conn) {
        let TriggerType::Event { event_type: want, filter } = &t.kind else { continue };
        if !event_matches(want, filter.as_ref(), event_type, data) {
            continue;
        }
        if busy(conn, &t.recipe_id) {
            tracing::info!(trigger = t.id, recipe_id = %t.recipe_id, event = event_type, "An event came while its recipe still runs: let go");
            continue;
        }
        let mut given = Map::new();
        given.insert("event_type".into(), json!(event_type));
        given.insert("event".into(), data.clone());
        if let Some(text) = data.get("text").and_then(|t| t.as_str()) {
            given.insert("event_text".into(), json!(text));
        }
        if let Some(run) = start(conn, &t, given, &format!("the event `{event_type}`"), now) {
            started.push(run);
        }
    }
    started
}

/// Start what waits on `done` completing: any trigger naming it, or the recipe it is a run of, by
/// id or by name. The run is given each of `done`'s own variables as `after_<name>` — the
/// Council's verdict as `{{after_verdict}}` — and `after_recipe`, `after_run` and `after_result`
/// (what its last step kept, as its completion message says it).
pub fn fire_after(conn: &Connection, done: &Recipe, vars: &HashMap<String, Value>, result: &str, now: f64) -> Vec<String> {
    let source = vars.get(FROM_VAR).and_then(|v| v.as_str()).unwrap_or(&done.id).to_string();
    let source_name = RecipeStore::get(conn, &source).map(|r| r.name).unwrap_or_else(|| done.name.clone());
    let depth = vars.get(CHAIN_VAR).and_then(|v| v.as_u64()).unwrap_or(0);
    let mut started = Vec::new();
    for t in RecipeStore::triggers(conn) {
        let TriggerType::RecipeComplete { recipe_id: named } = &t.kind else { continue };
        let named = named.trim();
        let hears = [done.id.as_str(), source.as_str()].contains(&named)
            || named.eq_ignore_ascii_case(&done.name)
            || named.eq_ignore_ascii_case(&source_name);
        if !hears {
            continue;
        }
        if t.recipe_id == source || t.recipe_id == done.id {
            tracing::warn!(trigger = t.id, recipe_id = %t.recipe_id, "A recipe is not started by its own completion");
            continue;
        }
        if depth + 1 > CHAIN_MOST {
            tracing::warn!(
                trigger = t.id, recipe_id = %t.recipe_id, depth,
                "A chain of recipes starting each other stopped at {CHAIN_MOST}"
            );
            continue;
        }
        if busy(conn, &t.recipe_id) {
            tracing::info!(trigger = t.id, recipe_id = %t.recipe_id, "A recipe it chains to still runs: not started again");
            continue;
        }
        let mut given: Map<String, Value> = vars
            .iter()
            .filter(|(k, _)| !k.starts_with('_'))
            .map(|(k, v)| (format!("after_{k}"), v.clone()))
            .collect();
        given.insert("after_recipe".into(), json!(done.name));
        given.insert("after_run".into(), json!(done.id));
        given.insert("after_result".into(), json!(result));
        if let Some(run) = start(conn, &t, given, &format!("{} completing ({})", done.name, done.id), now) {
            RecipeStore::set_var(conn, &run, CHAIN_VAR, &json!(depth + 1));
            started.push(run);
        }
    }
    started
}

/// Whether an event is one a trigger waits on: its type — equal, the last part of it, or a prefix
/// ending in `*`, with `:` and `/` the same — and every key of its filter.
pub fn event_matches(want: &str, filter: Option<&Value>, got: &str, data: &Value) -> bool {
    let norm = |s: &str| s.trim().to_lowercase().replace(':', "/");
    let (want, got) = (norm(want), norm(got));
    let kind = !want.is_empty()
        && (want == got
            || got.ends_with(&format!("/{want}"))
            || want == "*"
            || want.strip_suffix('*').is_some_and(|prefix| got.starts_with(prefix)));
    let fields = match filter.and_then(|f| f.as_object()) {
        None => true,
        Some(keys) => keys.iter().all(|(key, wanted)| match (data.get(key), wanted) {
            (Some(Value::String(have)), Value::String(wanted)) => have.to_lowercase().contains(&wanted.to_lowercase()),
            (Some(have), wanted) => have == wanted,
            (None, _) => false,
        }),
    };
    kind && fields
}

/// Whether the recipe a trigger starts, or a run of it, is in flight.
fn busy(conn: &Connection, recipe_id: &str) -> bool {
    let from = serde_json::to_string(recipe_id).unwrap_or_default();
    conn.query_row(
        "SELECT COUNT(*) FROM recipes r
         WHERE r.status IN ('running', 'waiting', 'paused')
           AND (r.id = ?1 OR EXISTS (SELECT 1 FROM recipe_vars v WHERE v.recipe_id = r.id AND v.key = ?2 AND v.value = ?3))",
        params![recipe_id, FROM_VAR, from],
        |row| row.get::<_, i64>(0),
    )
    .map(|n| n > 0)
    .unwrap_or(false)
}

/// Start one run for a trigger: unattended, with what started it kept on it.
fn start(conn: &Connection, t: &Trigger, given: Map<String, Value>, by: &str, now: f64) -> Option<String> {
    match RecipeStore::start_run(conn, &t.recipe_id, Some(&given)) {
        Ok((recipe, run)) => {
            // Nobody at the desk started this: whatever leave its recipe had is not this run's.
            RecipeStore::forget_leave(conn, &run);
            let started = Started { trigger: t.id, by: by.to_string(), at: now };
            RecipeStore::set_var(conn, &run, TRIGGER_VAR, &serde_json::to_value(&started).unwrap_or_default());
            RecipeStore::record_trigger_fired(conn, t.id, now);
            tracing::info!(trigger = t.id, recipe = %recipe.name, run = %run, by, "A trigger started a recipe");
            Some(run)
        }
        Err(why) => {
            // Recorded as fired all the same: a start that cannot be made now would otherwise be
            // tried again every tick.
            RecipeStore::record_trigger_fired(conn, t.id, now);
            tracing::warn!(trigger = t.id, recipe_id = %t.recipe_id, why = %why, "A trigger could not start its recipe");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recipe::{RecipeStatus, RecipeStep};
    use crate::recipe_time::tests::{central, Central};

    fn store() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory sqlite");
        RecipeStore::ensure_tables(&conn);
        conn
    }

    fn notify(text: &str) -> RecipeStep {
        RecipeStep::Notify { message: text.into() }
    }

    /// A recipe with a trigger, made at `made`.
    fn with_trigger(conn: &Connection, name: &str, kind: TriggerType, made: f64) -> String {
        let id = RecipeStore::create(conn, name, "", &[notify("{{after_result}}")], Some(&kind));
        conn.execute("UPDATE recipes SET created_at = ?1, updated_at = ?1 WHERE id = ?2", params![made, id]).unwrap();
        id
    }

    fn runs_of(conn: &Connection, id: &str) -> Vec<String> {
        RecipeStore::list(conn, None, 100)
            .into_iter()
            .filter(|r| RecipeStore::get_vars(conn, &r.id).get(FROM_VAR).and_then(|v| v.as_str()) == Some(id))
            .map(|r| r.id)
            .collect()
    }

    /// "Every morning at 8" starts at 08:00 on the machine's clock — not before, once, and at
    /// 08:00 on both sides of a daylight-saving change. What started it is kept on the run.
    #[test]
    fn a_schedule_starts_its_recipe_at_its_local_time_once() {
        let conn = store();
        let made = central(2026, 3, 6, 12, 0);
        let id = with_trigger(&conn, "Morning digest", TriggerType::Cron { expression: "0 8 * * *".into() }, made);

        assert!(fire_due_in(&Central, &conn, central(2026, 3, 7, 7, 59)).is_empty(), "not early");
        let run = fire_due_in(&Central, &conn, central(2026, 3, 7, 8, 0) + 3.0);
        assert_eq!(run.len(), 1, "08:00 CST");
        let first = &run[0];
        assert_eq!(RecipeStore::get(&conn, first).map(|r| r.status), Some(RecipeStatus::Running));
        let vars = RecipeStore::get_vars(&conn, first);
        let started = Started::of(&vars).expect("what started it");
        assert_eq!(started.by, "its schedule, `0 8 * * *`, at 08:00");
        assert!(fire_due_in(&Central, &conn, central(2026, 3, 7, 8, 0) + 8.0).is_empty(), "once");
        RecipeStore::update_status(&conn, first, &RecipeStatus::Done, 1);

        // The morning after the clocks went forward: 08:00 CDT, an hour sooner in UTC's count.
        assert!(fire_due_in(&Central, &conn, central(2026, 3, 8, 7, 59)).is_empty());
        let second = fire_due_in(&Central, &conn, central(2026, 3, 8, 8, 0) + 3.0);
        assert_eq!(second.len(), 1);
        assert_eq!(central(2026, 3, 8, 8, 0) - central(2026, 3, 7, 8, 0), 23.0 * 3600.0, "a day of 23 hours");
        assert_eq!(runs_of(&conn, &id).len(), 2);
    }

    /// Missed while the machine was off: fired once on waking inside the catch-up, let go after
    /// it. Due while its last run still works: it waits, and fires when that run is done.
    #[test]
    fn a_missed_schedule_catches_up_once_and_a_busy_one_waits() {
        let conn = store();
        let id = with_trigger(&conn, "Digest", TriggerType::Cron { expression: "0 8 * * *".into() }, central(2026, 7, 1, 0, 0));
        // Woken at 09:30: 08:00 is 90 minutes old.
        let woke = fire_due_in(&Central, &conn, central(2026, 7, 1, 9, 30));
        assert_eq!(woke.len(), 1, "caught up once");
        assert!(fire_due_in(&Central, &conn, central(2026, 7, 1, 9, 31)).is_empty());
        RecipeStore::update_status(&conn, &woke[0], &RecipeStatus::Done, 1);
        // Asleep all the next day until 11:00: 08:00 is three hours old, and let go.
        assert!(fire_due_in(&Central, &conn, central(2026, 7, 2, 11, 0)).is_empty(), "too old to catch up");

        // Due while the last run still works: waits, then fires once it is done.
        let run = fire_due_in(&Central, &conn, central(2026, 7, 3, 8, 0) + 3.0);
        assert_eq!(run.len(), 1);
        assert!(fire_due_in(&Central, &conn, central(2026, 7, 4, 8, 0) + 3.0).is_empty(), "the 3rd's run is still in flight");
        RecipeStore::update_status(&conn, &run[0], &RecipeStatus::Done, 1);
        assert_eq!(fire_due_in(&Central, &conn, central(2026, 7, 4, 8, 20)).len(), 1, "fired once it was free");
        assert_eq!(runs_of(&conn, &id).len(), 3);
    }

    /// `RecipeComplete` chains: any run of the Council completing starts the recipe waiting on it,
    /// by the Council's id or its name, with the Council's variables as `after_*`. Never a recipe
    /// by its own completion, and never a chain longer than CHAIN_MOST.
    #[test]
    fn a_completion_starts_the_recipe_waiting_on_it_with_what_it_came_to() {
        let conn = store();
        crate::recipe_templates::register_all(&conn);
        let council = crate::recipe_templates::formations::COUNCIL;
        let by_name = with_trigger(&conn, "Act on the verdict", TriggerType::RecipeComplete { recipe_id: "council".into() }, 0.0);
        let by_id = with_trigger(&conn, "File the verdict", TriggerType::RecipeComplete { recipe_id: council.into() }, 0.0);
        let q = serde_json::Map::from_iter([("question".to_string(), json!("Ship on Friday?"))]);
        let (_, run) = RecipeStore::start_run(&conn, council, Some(&q)).unwrap();
        RecipeStore::set_var(&conn, &run, "verdict", &json!("Ship on Monday."));
        RecipeStore::update_status(&conn, &run, &RecipeStatus::Done, 4);
        let done = RecipeStore::get(&conn, &run).unwrap();
        let started = fire_after(&conn, &done, &RecipeStore::get_vars(&conn, &run), "Ship on Monday.", 1_000.0);
        assert_eq!(started.len(), 2, "by its name and by its id");
        for (next, owner) in started.iter().zip([&by_name, &by_id]) {
            let vars = RecipeStore::get_vars(&conn, next);
            assert_eq!(vars.get(FROM_VAR).and_then(|v| v.as_str()), Some(owner.as_str()));
            assert_eq!(vars.get("after_verdict"), Some(&json!("Ship on Monday.")));
            assert_eq!(vars.get("after_question"), Some(&json!("Ship on Friday?")));
            assert_eq!(vars.get("after_result"), Some(&json!("Ship on Monday.")));
            assert_eq!(vars.get("after_recipe"), Some(&json!("Council")));
            assert_eq!(vars.get(CHAIN_VAR), Some(&json!(1)));
            assert!(!vars.contains_key("after__from"), "the run's own bookkeeping is not handed on");
            assert!(Started::of(&vars).unwrap().by.starts_with("Council completing"));
        }

        // Its own completion does not start it again.
        let looping = with_trigger(&conn, "Loop", TriggerType::RecipeComplete { recipe_id: "Loop".into() }, 0.0);
        RecipeStore::update_status(&conn, &looping, &RecipeStatus::Done, 1);
        let me = RecipeStore::get(&conn, &looping).unwrap();
        assert!(fire_after(&conn, &me, &RecipeStore::get_vars(&conn, &looping), "", 1.0).is_empty());
        // A chain is at most CHAIN_MOST long.
        let mut deep = RecipeStore::get_vars(&conn, &run);
        deep.insert(CHAIN_VAR.into(), json!(CHAIN_MOST));
        for next in &started {
            RecipeStore::update_status(&conn, next, &RecipeStatus::Done, 1);
        }
        assert!(fire_after(&conn, &done, &deep, "", 2.0).is_empty(), "stopped at the longest chain");
    }

    /// An event starts what waits on it: by its type, its last part or a prefix, and its filter.
    #[test]
    fn an_event_starts_what_waits_on_it() {
        let conn = store();
        let filter = json!({"text": "disk"});
        let id = with_trigger(&conn, "Tidy disk", TriggerType::Event { event_type: "system:storage".into(), filter: Some(filter.clone()) }, 0.0);
        assert!(event_matches("storage", None, "system/storage", &json!({})));
        assert!(event_matches("system/*", None, "system/storage", &json!({})));
        assert!(!event_matches("network", None, "system/storage", &json!({})));
        assert!(!event_matches("system/storage", Some(&filter), "system/storage", &json!({"text": "battery low"})));
        assert!(fire_event(&conn, "system/storage", &json!({"text": "Battery low"}), 1.0).is_empty(), "the filter holds");
        let run = fire_event(&conn, "system/storage", &json!({"text": "Disk 95% full", "importance": 0.8}), 2.0);
        assert_eq!(run.len(), 1);
        let vars = RecipeStore::get_vars(&conn, &run[0]);
        assert_eq!(vars.get("event_text"), Some(&json!("Disk 95% full")));
        assert_eq!(vars.get("event_type"), Some(&json!("system/storage")));
        assert!(fire_event(&conn, "system/storage", &json!({"text": "Disk 96% full"}), 3.0).is_empty(), "one at a time");
        assert_eq!(runs_of(&conn, &id).len(), 1);
    }

    /// A triggered start is unattended: a leave its recipe had — from an earlier start at the
    /// desk, say — is not the run's.
    #[test]
    fn a_triggered_start_is_unattended() {
        let conn = store();
        let id = with_trigger(&conn, "Nightly", TriggerType::Cron { expression: "0 2 * * *".into() }, central(2026, 7, 1, 0, 0));
        RecipeStore::allow_agents(&conn, &id, &crate::recipe::Leave::new("the person, from the Recipes screen", None));
        let run = fire_due_in(&Central, &conn, central(2026, 7, 1, 2, 0) + 3.0);
        assert_eq!(run, vec![id.clone()], "never run: it runs in place");
        assert_eq!(RecipeStore::agents_allowed(&conn, &run[0]), None, "no leave: nobody at the desk started it");
        assert_eq!(said(&TriggerType::Cron { expression: "0 2 * * *".into() }), "on its schedule, `0 2 * * *`, on this machine's clock");
    }
}
