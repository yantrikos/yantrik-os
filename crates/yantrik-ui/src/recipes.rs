//! Every recipe the companion holds, as the desk shows them — the shell's copy.
//!
//! The companion worker owns the recipe store (it holds the only connection to the memory
//! database), so it publishes here: when it starts, and after anything it does that can change a
//! recipe — a turn, a tool, a task, a step, a person's answer or pause. Everything on the UI thread
//! reads this copy and never waits on the worker, which can be forty seconds into a generation:
//!
//! - the Recipes screen (`wire::recipes`), which redraws when [`Snapshot::generation`] moves;
//! - `describe shell` → `recipes` ([`for_describe`]);
//! - the mind panel's "recipes in flight" ([`in_flight`]).
//!
//! The views themselves — what a step is, where it stands, what is unbound — are
//! `yantrik_companion::recipe_view`, pure and tested there.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;

use yantrik_companion::interjection::ChatWord;
use yantrik_companion::recipe_view::{self, RecipeView};

/// How a person's press went, as the worker answered it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub recipe: String,
    pub text: String,
    pub ok: bool,
    /// Counts presses, so a screen shows each outcome once.
    pub serial: u64,
}

/// What the screen and `describe` read.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    /// Moves every time the recipes change or a press is answered.
    pub generation: u64,
    /// Whether the worker has published at all. Before it has, "no recipes" would be a guess.
    pub loaded: bool,
    pub views: Arc<Vec<RecipeView>>,
    pub outcome: Option<Outcome>,
}

static PUBLISHED: OnceLock<Mutex<Snapshot>> = OnceLock::new();

/// A refresh asked of the worker and not yet answered.
static REFRESH_ASKED: AtomicBool = AtomicBool::new(false);

/// Rung when the worker answers a press, for a caller waiting on it ([`outcome_after`]).
static ANSWERED: Condvar = Condvar::new();

fn published() -> std::sync::MutexGuard<'static, Snapshot> {
    PUBLISHED
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// From the worker: the recipes as they are now. The generation moves only when they changed.
pub fn publish(views: Vec<RecipeView>) {
    REFRESH_ASKED.store(false, Ordering::Relaxed);
    let mut p = published();
    if p.loaded && *p.views == views {
        return;
    }
    p.views = Arc::new(views);
    p.loaded = true;
    p.generation += 1;
}

/// From the worker: what came of a press.
pub fn record(recipe: &str, outcome: Result<String, String>) {
    let mut p = published();
    let serial = p.outcome.as_ref().map_or(1, |o| o.serial + 1);
    let (text, ok) = match outcome {
        Ok(text) => (text, true),
        Err(text) => (text, false),
    };
    p.outcome = Some(Outcome { recipe: recipe.to_string(), text, ok, serial });
    p.generation += 1;
    drop(p);
    ANSWERED.notify_all();
}

/// The serial of the last press the worker answered — what [`outcome_after`] waits beyond.
pub fn last_serial() -> u64 {
    published().outcome.as_ref().map_or(0, |o| o.serial)
}

/// Wait for the worker's answer to a press on `recipe` newer than `serial`. None when none came
/// in `timeout`: the worker may be forty seconds into a generation.
pub fn outcome_after(recipe: &str, serial: u64, timeout: Duration) -> Option<Outcome> {
    let answered = |s: &Snapshot| s.outcome.as_ref().is_some_and(|o| o.serial > serial && o.recipe == recipe);
    let (p, _) = ANSWERED
        .wait_timeout_while(published(), timeout, |s| !answered(s))
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    p.outcome.clone().filter(|_| answered(&p))
}

/// What a line typed to the desktop says to a recipe, if anything, read against the recipes as
/// published — so the chat can ask on the UI thread, before any mind is asked, without waiting on
/// the worker. See `interjection::from_chat`.
pub fn said_in_chat(text: &str) -> Option<ChatWord> {
    word_for(&snapshot(), text, now_secs())
}

fn word_for(snap: &Snapshot, text: &str, now: f64) -> Option<ChatWord> {
    if !snap.loaded {
        return None;
    }
    yantrik_companion::interjection::from_chat(&snap.views, text, now)
}

/// Do what the chat said to a recipe the way the Recipes screen does — `CompanionHandle::recipe`,
/// which the worker answers with `recipe_view::apply` — and stream what came of it back as the
/// reply: the worker's own sentence, its refusal, or that it is busy.
pub fn act_from_chat(handle: crate::bridge::CompanionHandle, word: ChatWord) -> crossbeam_channel::Receiver<String> {
    let (tx, rx) = crossbeam_channel::unbounded();
    std::thread::spawn(move || {
        let reply = match word {
            ChatWord::To { recipe_id, name, op } => {
                let before = last_serial();
                match handle.recipe(recipe_id.clone(), op) {
                    Err(why) => format!("I could not reach the companion to do that to '{name}': {why}."),
                    Ok(()) => match outcome_after(&recipe_id, before, Duration::from_secs(30)) {
                        Some(o) if o.ok => o.text,
                        Some(o) => format!("Not done: {}.", o.text),
                        None => format!("Sent to '{name}'. The companion is busy; the Recipes screen shows it once done."),
                    },
                }
            }
            which => which.which_text().unwrap_or_default(),
        };
        let _ = tx.send(reply);
        let _ = tx.send("__DONE__".to_string());
    });
    rx
}

fn now_secs() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs_f64()
}

/// The recipes as last published.
pub fn snapshot() -> Snapshot {
    published().clone()
}

/// Ask the worker to publish again — unless it has been asked and has not answered yet, so a
/// worker busy for a minute does not come back to a queue of refreshes.
pub fn request_refresh(handle: &crate::bridge::CompanionHandle) {
    if !REFRESH_ASKED.swap(true, Ordering::Relaxed) && handle.refresh_recipes().is_err() {
        REFRESH_ASKED.store(false, Ordering::Relaxed);
    }
}

/// The recipe by id, as last published.
pub fn find(id: &str) -> Option<RecipeView> {
    published().views.iter().find(|v| v.id == id).cloned()
}

/// Recipes in flight — running, waiting or paused — in the desk's order, the ones waiting on the
/// person first. `None` until the worker has published once. The mind panel's Working section
/// reads this (`mind_panel::recipes`), and `recipe_view::one_line` says each in a line.
pub fn in_flight() -> Option<Vec<RecipeView>> {
    let p = published();
    p.loaded.then(|| p.views.iter().filter(|v| recipe_view::is_in_flight(v)).cloned().collect())
}

/// `describe shell` → `recipes`.
///
/// Every recipe but the never-run built-in definitions, which are counted: there are about fifty
/// and a caller reading what is going on does not need them spelt out. Steps are numbered from
/// one here, as a person counts them.
pub fn for_describe() -> serde_json::Value {
    describe(&snapshot())
}

fn describe(snap: &Snapshot) -> serde_json::Value {
    let views = &snap.views;
    let count = |f: &dyn Fn(&RecipeView) -> bool| views.iter().filter(|v| f(v)).count();
    let recipes: Vec<serde_json::Value> = views
        .iter()
        .filter(|v| !v.template)
        .map(|v| {
            let focus = focus(v);
            let unbound: Vec<String> = {
                let mut all: Vec<String> = Vec::new();
                for s in &v.steps {
                    for name in &s.unbound {
                        let shown = format!("{{{{{name}}}}}");
                        if !all.contains(&shown) {
                            all.push(shown);
                        }
                    }
                }
                all
            };
            serde_json::json!({
                "id": v.id,
                "name": v.name,
                "status": v.status,
                "steps": v.steps.len(),
                // The stage the row lights: running, waited on, held, or where it failed.
                "current_step": focus.map(|s| serde_json::json!({
                    "number": s.index + 1,
                    "kind": s.kind,
                    "label": s.label,
                    "summary": s.summary,
                    "state": s.state,
                })),
                "waiting_for": v.waiting_for,
                "question": v.question.as_ref().map(|q| serde_json::json!({
                    "text": q.text,
                    "choices": q.choices,
                })),
                "error": v.error,
                // Inputs a step reads that nothing has set, as the step writes them (#88).
                "unbound": unbound,
                "can": {
                    "answer": v.can.answer,
                    "pause": v.can.pause,
                    "resume": v.can.resume,
                    "cancel": v.can.cancel,
                },
                // A formation's agents: each Agent step's, working or answered.
                "agents": v.agents,
                // What its agents need the person for, if anything: a card, a place.
                "needs_you": v.needs_you,
                // A run a trigger started — with nobody at the desk — and by what; and what
                // starts this recipe on its own (#187).
                "started_by": v.started_by,
                "triggers": v.triggers,
                "updated_at": v.updated_at as i64,
            })
        })
        .collect();
    // The formations a run_recipe can start, with what each takes: its one input, and the roles
    // it seats by default (changed by giving them).
    let formations: Vec<serde_json::Value> = views
        .iter()
        .filter(|v| v.template && v.formation)
        .map(|v| {
            serde_json::json!({
                "id": v.id,
                "name": v.name,
                "description": v.description,
                "stages": v.steps.iter().map(|s| s.label.as_str()).collect::<Vec<_>>(),
                "inputs": v.inputs,
            })
        })
        .collect();
    serde_json::json!({
        "loaded": snap.loaded,
        "in_flight": count(&|v| recipe_view::is_in_flight(v)),
        "waiting_for_you": count(&|v| v.can.answer || v.needs_you.is_some()),
        "templates": count(&|v| v.template),
        "recipes": recipes,
        "formations": formations,
        "act": "run_recipe {recipe, inputs} starts one — a formation among them, whose agents it \
                hands work to (sensitive); answer_recipe {recipe, text} answers the question one \
                waits on; pause_recipe, resume_recipe and cancel_recipe {recipe} do what they say. \
                `can` says which apply.",
    })
}

/// The stage a recipe's row lights: the one running, waited on, held, or where it stopped.
pub fn focus(v: &RecipeView) -> Option<&recipe_view::StepView> {
    v.steps.iter().find(|s| matches!(s.state.as_str(), "current" | "waiting" | "paused" | "failed" | "stopped"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use yantrik_companion::recipe::{Recipe, RecipeStatus, RecipeStep, StoredStep};

    fn waiting_recipe() -> RecipeView {
        let recipe = Recipe {
            id: "rcp_1".into(),
            name: "Tidy downloads".into(),
            description: String::new(),
            status: RecipeStatus::Waiting,
            current_step: 2,
            created_at: 0.0,
            updated_at: 10.0,
            enabled: true,
            error_message: None,
        };
        let steps = vec![
            StoredStep {
                step_index: 0,
                step: RecipeStep::Tool {
                    tool_name: "recall".into(),
                    args: serde_json::json!({"query": "{{topic}}"}),
                    store_as: "found".into(),
                    on_error: Default::default(),
                },
                status: "done".into(),
                result: None,
            },
            StoredStep {
                step_index: 1,
                step: RecipeStep::AskUser { question: "Which folder?".into(), store_as: "folder".into(), choices: Some(vec!["Archive".into()]) },
                status: "done".into(),
                result: Some("asked".into()),
            },
        ];
        recipe_view::view(&recipe, &steps, &Default::default())
    }

    #[test]
    fn describe_lists_each_recipe_with_where_it_is_and_what_it_waits_for() {
        let mut template = waiting_recipe();
        template.id = "builtin_x".into();
        template.template = true;
        let snap = Snapshot { generation: 1, loaded: true, views: Arc::new(vec![waiting_recipe(), template]), outcome: None };
        let d = describe(&snap);
        assert_eq!(d["loaded"], true);
        assert_eq!(d["templates"], 1);
        assert_eq!(d["waiting_for_you"], 2);
        let r = &d["recipes"];
        assert_eq!(r.as_array().map(Vec::len), Some(1), "templates are counted, not listed: {r}");
        let r = &r[0];
        assert_eq!(r["id"], "rcp_1");
        assert_eq!(r["status"], "waiting");
        assert_eq!(r["current_step"]["number"], 2);
        assert_eq!(r["current_step"]["kind"], "ask_user");
        assert_eq!(r["waiting_for"], "your answer");
        assert_eq!(r["question"]["text"], "Which folder?");
        assert_eq!(r["unbound"], serde_json::json!(["{{topic}}"]));
        assert_eq!(r["can"]["answer"], true);
    }

    #[test]
    fn nothing_published_says_so_rather_than_no_recipes() {
        let d = describe(&Snapshot::default());
        assert_eq!(d["loaded"], false);
        assert_eq!(d["recipes"], serde_json::json!([]));
    }

    #[test]
    fn publishing_the_same_recipes_does_not_move_the_generation() {
        // One test owns the global: the others use `describe` on a snapshot of their own.
        publish(vec![waiting_recipe()]);
        let first = snapshot().generation;
        assert!(snapshot().loaded);
        // The mind panel's line.
        let flying = in_flight().expect("published");
        assert_eq!(flying.iter().map(recipe_view::one_line).collect::<Vec<_>>(), ["Tidy downloads — waiting for your answer: Which folder?"]);
        assert_eq!(find("rcp_1").map(|v| v.status), Some("waiting".to_string()));
        publish(vec![waiting_recipe()]);
        assert_eq!(snapshot().generation, first, "unchanged recipes are not a change");
        let mut changed = waiting_recipe();
        changed.status = "done".into();
        publish(vec![changed]);
        assert_eq!(snapshot().generation, first + 1);
        assert_eq!(in_flight().map(|v| v.len()), Some(0));
        let before = last_serial();
        record("rcp_1", Err("`Tidy downloads` is not waiting for an answer".into()));
        let s = snapshot();
        assert_eq!(s.generation, first + 2);
        let o = s.outcome.expect("the outcome");
        assert!(!o.ok && o.recipe == "rcp_1");
        // What the chat waits for after a word to a recipe: the answer to that press, not an older one.
        assert_eq!(outcome_after("rcp_1", before, Duration::ZERO).map(|o| o.serial), Some(before + 1));
        assert_eq!(outcome_after("rcp_1", before + 1, Duration::from_millis(10)), None, "nothing newer came");
        assert_eq!(outcome_after("rcp_2", before, Duration::from_millis(10)), None, "that one was another recipe's");
    }

    /// The chat reads the published recipes: a word to one is found there, and before the worker
    /// has published nothing is taken as one.
    #[test]
    fn the_chat_reads_the_published_recipes() {
        let snap = Snapshot { generation: 1, loaded: true, views: Arc::new(vec![waiting_recipe()]), outcome: None };
        assert_eq!(
            word_for(&snap, "archive", 20.0),
            Some(ChatWord::To {
                recipe_id: "rcp_1".into(),
                name: "Tidy downloads".into(),
                op: yantrik_companion::recipe_view::RecipeOp::Answer("Archive".into()),
            })
        );
        assert!(matches!(word_for(&snap, "pause it", 20.0), Some(ChatWord::To { .. })));
        assert_eq!(word_for(&snap, "what's the time", 20.0), None);
        assert_eq!(word_for(&Snapshot::default(), "archive", 20.0), None, "nothing published yet");
    }
}
