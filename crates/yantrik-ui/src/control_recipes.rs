//! A recipe, operable over the shell's surface: start one, answer the question one waits on, pause
//! it, resume it, cancel it.
//!
//! `run_recipe` is the door a mind starts a recipe by — a formation among them (design/desk-and-
//! mind-2026-09-23.md, section 6), whose Agent steps hand work to roles from the agent catalog.
//! Graded **sensitive**, because it starts agents and work that runs as the person: in `ask` mode
//! the person sees a card first. Its run is given the person's leave for its agents
//! (`RecipeStore::allow_agents`); the companion's own `run_recipe` tool, graded standard, is not
//! given one and refuses a formation. An agent that another agent or a recipe started cannot start
//! one (depth one); an agent that may start agents can, and the formation's agents are its
//! children, held to its rules.
//!
//! The same four things the Recipes screen's buttons do, through the same door — the companion
//! worker applies each with `yantrik_companion::recipe_view::apply`, which answers through the
//! chat's own AskUser path and pauses, resumes and cancels through the recipe store. What a recipe
//! is doing is read from `describe shell` → `recipes`; its `can` says which of these apply.
//!
//! Deferred, all four: the worker may be in the middle of a generation, and the shell's UI thread
//! never waits on it. A request is checked here against the recipes as last published, so a caller
//! is refused at once for a recipe that does not exist or cannot take it; what the worker then
//! made of it is in `describe shell` and on the Recipes screen.

use std::time::Duration;

use yantrik_app_runtime::control::{self, Action, App as ControlSurface, Param};
use yantrik_companion::recipe::Leave;
use yantrik_companion::recipe_view::{RecipeOp, RecipeView};

use crate::agents::{self, AgentId};
use crate::bridge::CompanionHandle;
use crate::control_agents::Caller;

/// How long `run_recipe` waits for the companion to take the recipe.
const START_WAIT: Duration = Duration::from_secs(20);

/// `run_recipe`, as published.
pub(crate) fn run_recipe_spec() -> Action {
    Action::new(
        "run_recipe",
        "Start a recipe with its inputs: a built-in one by its name or id, or one a mind made. A \
         formation — Council, Red team, Build, Writers' room; `describe shell` → `recipes` → \
         `formations` lists each with its `inputs` — hands work to roles from the agent \
         catalog: each works in its own pane on the Agents screen, its row saying which recipe it \
         works for, and the recipe's stages light on the Recipes screen as they answer; its result \
         comes as the recipe's completion. Answers with the run's id; `describe shell` → `recipes` \
         shows how it goes, and `cancel_recipe` stops it and lets its agents go. An agent another \
         agent or a recipe started cannot start a formation.",
    )
    .risk("sensitive")
    .defers()
    .arg(Param::text("recipe").describe("The recipe's id from `describe shell` → `recipes`, or its name (Council, Build …)"))
    // An object, as it is described and as a caller sends it: declared as text, the dispatch's
    // type check would refuse the very object the description asks for (as it would have
    // `args_json`'s). `inputs_arg` still reads JSON text, for a caller that reaches it another way.
    .arg(Param::object("inputs").optional().describe(
        "Its inputs as a JSON object: {\"question\": \"…\"} for the Council, and a seat to change, e.g. \
         \"seat_2\": \"reviewer\". `describe shell` → `recipes` → `formations` lists what each takes",
    ))
}

/// Add the recipe actions to the shell's control surface.
pub fn actions(surface: ControlSurface, companion: CompanionHandle) -> ControlSurface {
    let recipe = || Param::text("recipe").describe("The recipe's id from `describe shell` → `recipes`, or its name");
    let (pause, resume, cancel, start) = (companion.clone(), companion.clone(), companion.clone(), companion.clone());
    surface
        .action(run_recipe_spec(), move |args| {
            let want = args["recipe"].as_str().unwrap_or_default().trim().to_string();
            if want.is_empty() {
                return Err("`recipe` is empty".into());
            }
            let inputs = inputs_arg(args)?;
            let leave = leave_for(&crate::control_agents::delegating_caller()?)?;
            let handle = start.clone();
            // The worker may be in the middle of an answer: wait for it off the UI thread.
            let work = move || {
                let started = handle.start_recipe_and_wait(want, inputs, Some(leave), START_WAIT)?;
                let said = if started.formation {
                    format!(
                        "Started '{}' as `{}`. Its agents are at work, each in its own pane on the Agents \
                         screen; `describe shell` → `recipes` shows its stages, and its result comes as \
                         the recipe's completion.",
                        started.name, started.run
                    )
                } else {
                    format!("Started '{}' as `{}`; `describe shell` → `recipes` shows how it goes.", started.name, started.run)
                };
                Ok(serde_json::json!({
                    "recipe": started.run,
                    "name": started.name,
                    "formation": started.formation,
                    "said": said,
                }))
            };
            control::answer_later(work).map(|()| serde_json::json!({ "answering": "off the UI thread" })).or_else(|work| work())
        })
        .action(
            Action::new(
                "answer_recipe",
                "Answer the question a recipe is waiting on, as the person would from the Recipes \
                 screen. Its `question` in describe has the text and the choices offered; any \
                 answer is taken. The recipe goes on with it",
            )
            .defers()
            .arg(recipe())
            .arg(Param::text("text").describe("The answer: one of the choices, or your own words")),
            move |args| {
                let text = args["text"].as_str().unwrap_or_default().trim().to_string();
                if text.is_empty() {
                    return Err("`text` is empty".into());
                }
                request(&companion, args, RecipeOp::Answer(text), |v| v.can.answer, "is not waiting for an answer")
            },
        )
        .action(
            Action::new("pause_recipe", "Hold a running or waiting recipe where it is, until resume_recipe")
                .defers()
                .arg(recipe()),
            move |args| request(&pause, args, RecipeOp::Pause, |v| v.can.pause, "is not running or waiting"),
        )
        .action(
            Action::new(
                "resume_recipe",
                "Let a paused recipe go on — running, or waiting again on what it was waiting for",
            )
            .defers()
            .arg(recipe()),
            move |args| request(&resume, args, RecipeOp::Resume, |v| v.can.resume, "is not paused"),
        )
        .action(
            Action::new("cancel_recipe", "Stop a running, waiting or paused recipe for good. It cannot be resumed")
                .defers()
                .arg(recipe()),
            move |args| request(&cancel, args, RecipeOp::Cancel, |v| v.can.cancel, "has nothing to cancel"),
        )
}

/// `inputs`: absent, an object, or an object written as JSON text.
fn inputs_arg(args: &serde_json::Value) -> Result<serde_json::Map<String, serde_json::Value>, String> {
    match args.get("inputs") {
        None | Some(serde_json::Value::Null) => Ok(Default::default()),
        Some(serde_json::Value::Object(map)) => Ok(map.clone()),
        Some(serde_json::Value::String(text)) if text.trim().is_empty() => Ok(Default::default()),
        Some(serde_json::Value::String(text)) => match serde_json::from_str::<serde_json::Value>(text) {
            Ok(serde_json::Value::Object(map)) => Ok(map),
            _ => Err(format!("`inputs` is a JSON object, like {{\"question\": \"…\"}}; it was `{text}`")),
        },
        Some(other) => Err(format!("`inputs` is a JSON object, like {{\"question\": \"…\"}}; it was {other}")),
    }
}

/// The leave a run started through `run_recipe` is given for its agents: the person's — who, in
/// `ask` mode, was shown the card — or, for an agent, that agent's, if it may start agents at all:
/// not one another agent or a recipe started. Its formation's agents are then its children.
pub(crate) fn leave_for(caller: &Caller) -> Result<Leave, String> {
    match caller {
        Caller::NoAgent => Ok(Leave::new("shell.run_recipe (sensitive), by a caller that runs as no agent", None)),
        Caller::Agent(me) => {
            let live: Vec<AgentId> = crate::wire::harness::host()
                .map(|h| h.agents().into_iter().map(|a| a.id).collect())
                .unwrap_or_default();
            agents::store().read(|s| crate::control_agents::may_start_child(me, s, &live))?;
            Ok(Leave::new(format!("agent `{me}`, through shell.run_recipe (sensitive)"), Some(me.0.clone())))
        }
    }
}

/// Check the request against the recipes as last published, then hand it to the worker.
fn request(
    companion: &CompanionHandle,
    args: &serde_json::Value,
    op: RecipeOp,
    allowed: impl Fn(&RecipeView) -> bool,
    refusal: &str,
) -> Result<serde_json::Value, String> {
    let want = args["recipe"].as_str().unwrap_or_default().trim();
    // An agent answering the person's phone answers, pauses and resumes no recipe: the recipe's
    // agents would carry on unheld.
    crate::control_agents::refuse_from_phone(&crate::control_agents::caller()?)?;
    let view = resolve(want)?;
    if !allowed(&view) {
        return Err(format!("`{}` {refusal} — it is {}", view.name, view.status));
    }
    let verb = op.verb();
    companion.recipe(view.id.clone(), op)?;
    Ok(serde_json::json!({
        "recipe": view.id,
        "name": view.name,
        "requested": verb,
        "read_back": "describe shell → recipes: its status, and `can` for what it takes now",
    }))
}

/// A recipe by id, or failing that by name, among the recipes as last published.
fn resolve(want: &str) -> Result<RecipeView, String> {
    if want.is_empty() {
        return Err("`recipe` is empty".into());
    }
    let snap = crate::recipes::snapshot();
    if !snap.loaded {
        return Err("the companion has not published its recipes yet".into());
    }
    pick(&snap.views, want).cloned().ok_or_else(|| {
        let known: Vec<&str> = snap.views.iter().filter(|v| !v.template).map(|v| v.id.as_str()).take(12).collect();
        format!(
            "no recipe `{want}`; `describe shell` lists them under `recipes`{}",
            if known.is_empty() { String::new() } else { format!(" ({})", known.join(", ")) }
        )
    })
}

fn pick<'a>(views: &'a [RecipeView], want: &str) -> Option<&'a RecipeView> {
    views
        .iter()
        .find(|v| v.id == want)
        .or_else(|| views.iter().find(|v| v.name.eq_ignore_ascii_case(want)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(id: &str, name: &str) -> RecipeView {
        RecipeView {
            id: id.into(),
            name: name.into(),
            description: String::new(),
            status: "running".into(),
            current_step: 0,
            created_at: 0.0,
            updated_at: 0.0,
            error: None,
            template: false,
            waiting_for: None,
            question: None,
            steps: Vec::new(),
            can: Default::default(),
            ..Default::default()
        }
    }

    #[test]
    fn a_recipe_is_found_by_its_id_or_its_name() {
        let views = vec![named("rcp_1", "Tidy downloads"), named("rcp_2", "rcp_1")];
        assert_eq!(pick(&views, "rcp_1").map(|v| v.name.as_str()), Some("Tidy downloads"), "the id wins");
        assert_eq!(pick(&views, "tidy DOWNLOADS").map(|v| v.id.as_str()), Some("rcp_1"));
        assert!(pick(&views, "nothing").is_none());
    }

    /// The four actions are on the surface, graded standard, and settle later — the worker does
    /// the work — with the arguments the design names.
    #[test]
    fn the_recipe_actions_are_published_as_designed() {
        let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/control_recipes.rs")).unwrap();
        let src = src.split("#[cfg(test)]").next().unwrap();
        for name in ["answer_recipe", "pause_recipe", "resume_recipe", "cancel_recipe"] {
            let at = src.find(&format!("\"{name}\"")).unwrap_or_else(|| panic!("{name} is published"));
            let spec = &src[at..at + src[at..].find("move |args|").expect("a handler")];
            assert!(spec.contains(".defers()"), "{name} settles later");
            assert!(!spec.contains(".risk("), "{name} is standard, the default");
            assert!(spec.contains(".arg(recipe())"), "{name} takes `recipe`");
        }
        let control = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/control.rs")).unwrap();
        assert!(control.contains("crate::control_recipes::actions(surface, ctx.bridge.handle())"), "the shell adds them");
    }

    /// `run_recipe` starts agents, so it is graded sensitive, and the gate every door meets —
    /// the one the dispatch runs before any handler — refuses it in ask mode without the person's
    /// grant, lets it through with one, runs it unasked only where the mode says sensitive runs
    /// unasked, and never above the ceiling.
    #[test]
    fn starting_a_formation_is_sensitive_and_refused_without_a_grant() {
        use yantrik_ipc_transport::gate::{decide, Authority, Mode};
        let spec = run_recipe_spec();
        assert_eq!((spec.name.as_str(), spec.permission, spec.deferred), ("run_recipe", "sensitive", true));
        let params: Vec<(&str, bool)> = spec.params.iter().map(|p| (p.name.as_str(), p.required)).collect();
        assert_eq!(params, [("recipe", true), ("inputs", false)]);
        // Published as the object it is described as, so the dispatch's type check takes one.
        assert_eq!(spec.params[1].kind, "object");
        let at = |mode: &str, granted: bool, ceiling: &str| Authority { ceiling: ceiling.into(), mode: Mode::named(mode), granted, asks_above: None };
        let err = decide(&at("ask", false, "sensitive"), "shell", "run_recipe", spec.permission, &spec.description).unwrap_err();
        assert!(err.starts_with("GRANT:") && err.contains("graded `sensitive`"), "{err}");
        assert!(decide(&at("plan", false, "sensitive"), "shell", "run_recipe", spec.permission, &spec.description).is_err());
        assert!(decide(&at("ask", true, "sensitive"), "shell", "run_recipe", spec.permission, &spec.description).is_ok(), "the person's Allow");
        assert!(decide(&at("auto", false, "sensitive"), "shell", "run_recipe", spec.permission, &spec.description).is_ok());
        assert!(decide(&at("bypass", true, "standard"), "shell", "run_recipe", spec.permission, &spec.description).is_err(), "never above the ceiling");
        assert!(decide(&at("bypass_all", true, "standard"), "shell", "run_recipe", spec.permission, &spec.description).is_err(), "not even in full bypass");

        let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/control_recipes.rs")).unwrap();
        let src = src.split("#[cfg(test)]").next().unwrap();
        assert!(src.contains(".action(run_recipe_spec(), move |args|"), "published on the shell's surface");
    }

    /// The leave a run gets for its agents: the person's, or a top-level agent's — never one a
    /// child or a recipe's agent could give.
    #[test]
    fn only_the_person_or_an_agent_that_may_start_agents_gives_a_run_its_leave() {
        use crate::agents::{AgentMeta, RecipeOrigin};
        let person = leave_for(&Caller::NoAgent).unwrap();
        assert_eq!((person.by.as_str(), person.agent), ("shell.run_recipe (sensitive), by a caller that runs as no agent", None));
        let top = AgentId("pi:c-leave-top".into());
        agents::store().upsert_agent(AgentMeta::new(top.clone(), "pi"));
        let leave = leave_for(&Caller::Agent(top.clone())).unwrap();
        assert_eq!(leave.agent.as_deref(), Some("pi:c-leave-top"));
        let child = AgentId("pi:c-leave-kid".into());
        let mut meta = AgentMeta::new(child.clone(), "pi");
        meta.parent = Some(top);
        agents::store().upsert_agent(meta);
        assert!(leave_for(&Caller::Agent(child)).unwrap_err().contains("one level only"));
        let recipes = AgentId("pi:c-leave-recipe".into());
        let mut meta = AgentMeta::new(recipes.clone(), "pi");
        meta.recipe = Some(RecipeOrigin { id: "rcp_x".into(), name: "Build".into() });
        agents::store().upsert_agent(meta);
        assert!(leave_for(&Caller::Agent(recipes)).unwrap_err().contains("a recipe started cannot start agents"));
    }

    #[test]
    fn inputs_come_as_an_object_or_as_json_text() {
        assert!(inputs_arg(&serde_json::json!({})).unwrap().is_empty());
        let given = inputs_arg(&serde_json::json!({"inputs": {"question": "Ship Friday?"}})).unwrap();
        assert_eq!(given["question"], "Ship Friday?");
        let text = inputs_arg(&serde_json::json!({"inputs": "{\"goal\": \"a flag\"}"})).unwrap();
        assert_eq!(text["goal"], "a flag");
        assert!(inputs_arg(&serde_json::json!({"inputs": "  "})).unwrap().is_empty());
        assert!(inputs_arg(&serde_json::json!({"inputs": "a flag"})).unwrap_err().contains("JSON object"));
        assert!(inputs_arg(&serde_json::json!({"inputs": [1]})).is_err());
    }
}
