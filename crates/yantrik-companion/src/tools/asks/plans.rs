//! The cards for work the companion is about to plan (#1311): a recipe, a schedule, an
//! automation, a queued task, a recipe started. All ask `plan_work`, right before the thing is
//! stored or started, after the tool has validated its arguments. Each card says what will
//! happen and when, in words (`words.rs`): every step of a recipe and its trigger, a schedule's
//! timing and what it does when it fires, an automation's instructions. Text the model or a page
//! wrote is quoted, so it cannot pose as another field.

use yantrik_companion_tools::asks::Card;

use super::words::{cron_words, interval_words, moment, step_words, trigger_words};
use crate::recipe::{Recipe, RecipeStep, TriggerType};
use crate::scheduler::ScheduledTask;

/// `create_recipe`, or a recipe being started (`run_recipe`): its name, its trigger and every step.
pub fn recipe_card(tool: &str, name: &str, steps: &[RecipeStep], trigger: Option<&TriggerType>) -> Card {
    let mut card = Card::new(tool).text("name", name).text("trigger", &trigger_words(trigger));
    for (i, step) in steps.iter().enumerate() {
        card = card.text(&format!("step_{}", i + 1), &step_words(step));
    }
    card
}

/// `run_recipe`: the recipe as it is stored, and the variables the run starts with.
pub fn run_card(recipe: &Recipe, steps: &[RecipeStep], trigger: Option<&TriggerType>, vars: Option<&serde_json::Map<String, serde_json::Value>>) -> Card {
    let card = recipe_card("run_recipe", &recipe.name, steps, trigger).plain("change", "starts it now, and its steps run on their own");
    match vars.filter(|v| !v.is_empty()) {
        Some(v) => card.text("variables", &serde_json::Value::Object(v.clone()).to_string()),
        None => card,
    }
}

/// A schedule about to be created, as the tool validated it.
pub struct Schedule<'a> {
    pub label: &'a str,
    pub kind: &'a str,
    pub next_invoke: f64,
    pub interval_secs: Option<i64>,
    pub cron: Option<&'a str>,
    pub description: &'a str,
    pub action: Option<&'a str>,
    pub max_invocations: Option<i64>,
}

impl Schedule<'_> {
    fn timing(&self) -> String {
        match (self.kind, self.interval_secs, self.cron) {
            ("cron", _, Some(expr)) => cron_words(expr),
            ("interval", Some(secs), _) => interval_words(secs),
            _ => "once".to_string(),
        }
    }

    pub fn card(&self) -> Card {
        let does = match self.action.filter(|a| !a.trim().is_empty()) {
            Some(action) => format!("the companion does this on its own: {action}"),
            None => "only reminds you".to_string(),
        };
        let mut card = Card::new("create_schedule")
            .text("label", self.label)
            .text("schedule", &self.timing())
            .plain("next", moment(self.next_invoke))
            .text("does", &does)
            .plain("repeats", self.max_invocations.map_or("no limit".to_string(), |n| format!("at most {n} times")));
        if !self.description.is_empty() {
            card = card.text("description", self.description);
        }
        card
    }
}

/// `{before} → {after}`, each quoted.
fn change(before: &str, after: &str) -> String {
    let q = |t: &str| yantrik_companion_tools::asks::quoted(t).as_str().unwrap_or_default().to_string();
    format!("{} → {}", q(before), q(after))
}

/// `update_schedule`: the task as it is now, and each field the call changes, before and after.
pub fn update_schedule_card(before: &ScheduledTask, updates: &serde_json::Value) -> Card {
    let mut card = Card::new("update_schedule").text("task", &format!("{} [{}]", before.label, before.task_id));
    let text = |key: &str| updates.get(key).and_then(|v| v.as_str());
    let number = |key: &str| updates.get(key).and_then(|v| v.as_f64());
    for (key, was, now) in [
        ("label", Some(before.label.clone()), text("label").map(str::to_string)),
        ("description", Some(before.description.clone()), text("description").map(str::to_string)),
        ("status", Some(before.status.clone()), text("status").map(str::to_string)),
        ("cron", before.cron_expr.as_deref().map(cron_words), text("cron").map(cron_words)),
        ("next_invoke", before.next_invoke.map(moment), number("next_invoke").map(moment)),
        ("urgency", Some(before.urgency.to_string()), number("urgency").map(|u| u.to_string())),
        ("interval_seconds", before.interval_secs.map(interval_words), updates.get("interval_seconds").and_then(|v| v.as_i64()).map(interval_words)),
    ] {
        if let Some(now) = now {
            card = card.plain(key, change(&was.unwrap_or_else(|| "none".to_string()), &now));
        }
    }
    card
}

/// `create_automation`: its name, when it runs, and the instructions it will follow.
pub fn automation_card(name: &str, description: &str, when: &str, condition: Option<&str>, instructions: &str) -> Card {
    let mut card = Card::new("create_automation").text("name", name).text("trigger", when).text("instructions", instructions);
    if let Some(c) = condition.filter(|c| !c.is_empty()) {
        card = card.text("condition", c);
    }
    if !description.is_empty() {
        card = card.text("description", description);
    }
    card
}

/// An automation's trigger in words, from the arguments the tool validated.
pub fn automation_trigger(trigger_type: &str, cron: Option<&str>, interval: Option<i64>, event_type: Option<&str>, event_match: &serde_json::Value) -> String {
    match (trigger_type, cron, interval, event_type) {
        ("schedule", Some(expr), _, _) => cron_words(expr),
        ("schedule", None, Some(secs), _) => interval_words(secs),
        ("event", _, _, Some(event)) if event_match.as_object().is_some_and(|m| !m.is_empty()) => format!("when `{event}` happens and matches {event_match}"),
        ("event", _, _, Some(event)) => format!("when `{event}` happens"),
        _ => "only when it is started by hand".to_string(),
    }
}

/// `toggle_automation`: which one, and whether it is turned on or off.
pub fn toggle_card(name: &str, id: &str, was_on: bool, on: bool) -> Card {
    let word = |b: bool| if b { "on" } else { "off" };
    Card::new("toggle_automation").text("automation", &format!("{name} [{id}]")).plain("change", format!("turns it {} (it is {} now)", word(on), word(was_on)))
}

/// `queue_task`: the title, the instructions the companion will work from, and the priority.
pub fn task_card(title: &str, instructions: &str, priority: i32) -> Card {
    Card::new("queue_task").text("title", title).text("instructions", instructions).plain("priority", priority)
}

/// `run_recipe` when the model starts it: ask, then start the run. A recipe that is not there,
/// is already running or hands work to agents is refused by `start` itself with its own
/// sentence and nobody is asked about it: the card is for a run that would really start.
///
/// The database is read, let go, and taken again inside the effect: the card waits up to two
/// minutes and nothing else that uses the database may wait with it (#1313, M2).
pub fn start_asking(
    db: &yantrikdb_core::YantrikDB,
    id_or_name: &str,
    vars: Option<&serde_json::Map<String, serde_json::Value>>,
) -> Result<(Recipe, String), String> {
    use crate::recipe::{RecipeStatus, RecipeStore};
    let looked = {
        let conn = db.conn();
        let found = RecipeStore::get(&conn, id_or_name).or_else(|| RecipeStore::find_by_name(&conn, id_or_name));
        let steps = |recipe: &Recipe| -> Vec<RecipeStep> { RecipeStore::get_steps(&conn, &recipe.id).into_iter().map(|s| s.step).collect() };
        let askable = found
            .filter(|r| !matches!(r.status, RecipeStatus::Running | RecipeStatus::Waiting | RecipeStatus::Paused))
            .map(|r| (steps(&r), r))
            .filter(|(steps, _)| !crate::recipe::hands_off(steps));
        askable.map(|(steps, recipe)| {
            let trigger = RecipeStore::get_enabled_triggers(&conn).into_iter().find(|(id, _, _)| *id == recipe.id).map(|(_, t, _)| t);
            (steps, recipe, trigger)
        })
    };
    let Some((steps, recipe, trigger)) = looked else {
        return crate::recipe_templates::start(&db.conn(), id_or_name, vars, None);
    };
    super::plan(&run_card(&recipe, &steps, trigger.as_ref(), vars), || crate::recipe_templates::start(&db.conn(), id_or_name, vars, None))
}
