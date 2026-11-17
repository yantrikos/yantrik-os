//! The planning tools ask before they store or start work (#1311): `create_recipe`,
//! `create_schedule`, `update_schedule`, `create_automation`, `toggle_automation`, `queue_task`
//! and `run_recipe`. Each is run as the model runs it, through the registry, against a database
//! that is looked at afterwards: what the card said, and what was and was not stored.

use serde_json::json;
use yantrik_companion_tools::asks_scenarios::{check_all, Scenario};
use yantrik_companion_tools::fake_shell::{Answer, FakeShell, First};

use super::tests_support::{ctx, db, q};
use crate::automation::AutomationStore;
use crate::recipe::RecipeStore;
use crate::scheduler::Scheduler;
use crate::task_queue::TaskQueue;
use crate::tools::ToolRegistry;

fn run(register: fn(&mut ToolRegistry), db: &yantrikdb_core::YantrikDB, tool: &str, args: serde_json::Value) -> String {
    let mut reg = ToolRegistry::new();
    register(&mut reg);
    reg.execute(&ctx(db), tool, &args)
}

/// What a call did: `Ok` once something was stored (the effect is counted), the tool's words if not.
fn outcome(stored: bool, said: String, effect: &dyn Fn() -> Result<(), String>) -> Result<(), String> {
    if stored {
        effect()
    } else {
        Err(said)
    }
}

#[test]
fn creating_a_recipe_asks_with_every_step_and_its_trigger() {
    check_all(&Scenario {
        action: "plan_work",
        grade: "sensitive",
        card: json!({
            "tool": "create_recipe", "name": q("Morning mail"), "trigger": q("every day at 07:00"),
            "step_1": q("tell you: hello"), "step_2": q("ask you: Archive it?"),
        }),
        call: &|variant, effect| {
            let db = db();
            let (name, message) = match variant {
                0 => ("Morning mail", "hello".to_string()),
                1 => ("Evening mail", "goodbye".to_string()),
                _ => ("Morning mail", "m".repeat(100)),
            };
            let said = run(
                crate::tools::recipe::register,
                &db,
                "create_recipe",
                json!({
                    "name": name, "trigger": {"type": "Cron", "expression": "0 7 * * *"},
                    "steps": [{"type": "Notify", "message": message}, {"type": "AskUser", "question": "Archive it?", "store_as": "a"}],
                }),
            );
            let stored = RecipeStore::list(&db.conn(), None, 10).len();
            assert_eq!(stored == 1, said.starts_with("Recipe created"), "{said}");
            outcome(stored == 1, said, effect)
        },
    });
}

#[test]
fn a_schedule_asks_in_words_and_a_denial_creates_nothing() {
    check_all(&Scenario {
        action: "plan_work",
        grade: "sensitive",
        card: json!({
            "tool": "create_schedule", "label": q("Weather"), "schedule": q("once"), "next": "2030-01-01 07:00 UTC",
            "does": q("the companion does this on its own: Tell me the weather"), "repeats": "no limit",
        }),
        call: &|variant, effect| {
            let db = db();
            let (label, action) = match variant {
                0 => ("Weather", "Tell me the weather".to_string()),
                1 => ("Traffic", "Tell me the traffic".to_string()),
                _ => ("Weather", "w".repeat(100)),
            };
            let said = run(
                crate::tools::scheduler::register,
                &db,
                "create_schedule",
                json!({"label": label, "schedule_type": "once", "at": "2030-01-01T07:00", "action": action}),
            );
            let listed = Scheduler::list(&db.conn(), Some("active")).len();
            outcome(listed == 1, said, effect)
        },
    });
}

#[test]
fn every_morning_at_seven_reads_as_a_sentence_on_the_card() {
    let shell = FakeShell::session(First::Pending, Answer::Denied);
    let db = db();
    let said = run(
        crate::tools::scheduler::register,
        &db,
        "create_schedule",
        json!({"label": "Weather", "schedule_type": "cron", "cron": "0 7 * * *", "action": "Tell me the weather"}),
    );
    assert!(said.contains("said no"), "{said}");
    assert_eq!(shell.cards()[0]["args_json"]["schedule"], q("every day at 07:00"));
    assert!(Scheduler::list(&db.conn(), None).is_empty(), "describe shows no new schedule");
}

#[test]
fn changing_a_schedule_shows_each_field_before_and_after() {
    let change = |a: &str, b: &str| format!("{} → {}", q(a).as_str().unwrap(), q(b).as_str().unwrap());
    check_all(&Scenario {
        action: "plan_work",
        grade: "sensitive",
        card: json!({ "tool": "update_schedule", "task": q("Weather [sched-1]"), "label": change("Weather", "Weather at 8"), "status": change("active", "paused") }),
        call: &|variant, effect| {
            let db = db();
            let id = Scheduler::create(&db.conn(), "Weather", "", "once", None, None, 4_000_000_000.0, None, 0.6, None, &json!({}));
            db.conn().execute("UPDATE scheduled_tasks SET task_id = 'sched-1' WHERE task_id = ?1", [&id]).unwrap();
            let label = match variant {
                0 => "Weather at 8".to_string(),
                1 => "Weather at 9".to_string(),
                _ => "w".repeat(100),
            };
            let said = run(crate::tools::scheduler::register, &db, "update_schedule", json!({"task_id": "sched-1", "label": label, "status": "paused"}));
            let after = Scheduler::get(&db.conn(), "sched-1").expect("the schedule is still there");
            assert_eq!(after.label != "Weather", after.status == "paused", "both fields change together or neither does");
            outcome(after.label == label, said, effect)
        },
    });
}

#[test]
fn an_automation_asks_and_a_denial_creates_neither_it_nor_its_schedule() {
    check_all(&Scenario {
        action: "plan_work",
        grade: "sensitive",
        card: json!({
            "tool": "create_automation", "name": q("Digest"), "trigger": q("every Monday at 09:00"),
            "instructions": q("Summarise my mail"),
        }),
        call: &|variant, effect| {
            let db = db();
            let (name, steps) = match variant {
                0 => ("Digest", "Summarise my mail".to_string()),
                1 => ("Digest 2", "Summarise my calendar".to_string()),
                _ => ("Digest", "s".repeat(100)),
            };
            let said = run(
                crate::tools::automation::register,
                &db,
                "create_automation",
                json!({"name": name, "trigger_type": "schedule", "schedule_cron": "0 9 * * 1", "steps": steps}),
            );
            let (automations, schedules) = (AutomationStore::list(&db.conn(), None, None).len(), Scheduler::list(&db.conn(), None).len());
            assert_eq!(automations, schedules, "the automation and its schedule are made together or not at all: {said}");
            outcome(automations == 1, said, effect)
        },
    });
}

#[test]
fn switching_an_automation_off_asks_which_one_and_the_change() {
    check_all(&Scenario {
        action: "plan_work",
        grade: "sensitive",
        card: json!({ "tool": "toggle_automation", "automation": q("Digest [auto-1]"), "change": "turns it off (it is on now)" }),
        call: &|variant, effect| {
            let db = db();
            let name = match variant {
                0 => "Digest".to_string(),
                1 => "Notes".to_string(),
                _ => "D".repeat(100),
            };
            let id = AutomationStore::create(&db.conn(), &name, "", "manual", &json!({}), None, "Summarise");
            db.conn().execute("UPDATE automations SET automation_id = 'auto-1' WHERE automation_id = ?1", [&id]).unwrap();
            let said = run(crate::tools::automation::register, &db, "toggle_automation", json!({"automation_id": "auto-1", "enabled": false}));
            let enabled = AutomationStore::get(&db.conn(), "auto-1").map(|a| a.enabled);
            outcome(enabled == Some(false), said, effect)
        },
    });
}

#[test]
fn queueing_a_task_asks_with_the_instructions_it_will_work_from() {
    check_all(&Scenario {
        action: "plan_work",
        grade: "sensitive",
        card: json!({ "tool": "queue_task", "title": q("Tidy notes"), "instructions": q("Merge the two notes files"), "priority": "2" }),
        call: &|variant, effect| {
            let db = db();
            let (title, description) = match variant {
                0 => ("Tidy notes", "Merge the two notes files".to_string()),
                1 => ("Sort photos", "Group them by month".to_string()),
                _ => ("Tidy notes", "n".repeat(100)),
            };
            let said = run(crate::tools::task_queue::register, &db, "queue_task", json!({"title": title, "description": description}));
            let queued = TaskQueue::list(&db.conn(), None, 10).len();
            outcome(queued == 1, said, effect)
        },
    });
}

#[test]
fn starting_a_recipe_asks_with_its_steps_and_a_denial_starts_nothing() {
    check_all(&Scenario {
        action: "plan_work",
        grade: "sensitive",
        card: json!({
            "tool": "run_recipe", "name": q("Digest"), "trigger": q("only when it is started by hand"),
            "step_1": q("tell you: hi"), "change": "starts it now, and its steps run on their own",
        }),
        call: &|variant, effect| {
            let db = db();
            let message = match variant {
                0 => "hi".to_string(),
                1 => "hello again".to_string(),
                _ => "h".repeat(100),
            };
            let steps = [crate::recipe::RecipeStep::Notify { message }];
            RecipeStore::create(&db.conn(), "Digest", "", &steps, None);
            let said = run(crate::tools::recipe::register, &db, "run_recipe", json!({"recipe_id": "Digest"}));
            let running = RecipeStore::list(&db.conn(), Some("running"), 10).len();
            outcome(running == 1, said, effect)
        },
    });
}

#[test]
fn a_recipe_that_is_already_running_is_refused_without_a_card() {
    let shell = FakeShell::session(First::Pending, Answer::Granted);
    let db = db();
    RecipeStore::create(&db.conn(), "Digest", "", &[crate::recipe::RecipeStep::Notify { message: "hi".into() }], None);
    assert!(run(crate::tools::recipe::register, &db, "run_recipe", json!({"recipe_id": "Digest"})).contains("queued"));
    let again = run(crate::tools::recipe::register, &db, "run_recipe", json!({"recipe_id": "Digest"}));
    assert!(again.contains("already"), "{again}");
    assert_eq!(shell.cards().len(), 1, "only the run that started was asked about");
}
