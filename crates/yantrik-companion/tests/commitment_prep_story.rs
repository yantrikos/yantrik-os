//! "Say it once" (#1183, #1184): said on a Monday, prepared when Thursday's prep window opens.
//!
//! The clock is a number the test moves. The tools are fakes that stand in for the disk and for the
//! apps, registered in a real `ToolRegistry`, so every step goes through `ToolRegistry::execute`
//! and is written to the audit exactly as in the shell.

use std::sync::{Arc, Mutex};

use rusqlite::Connection;
use serde_json::{json, Value};
use yantrik_companion::anticipation::{AnticipationEngine, PrepOutcome, PreparationTrigger};
use yantrik_companion::commitment_extractor::{extract_commitments_at, to_world_model_commitments};
use yantrik_companion::commitment_prep::{approaching_trigger, commitment_to_prepare, prepare, Outcome};
use yantrik_companion::world_model::{CommitmentSource, WorldModel};
use yantrik_companion_core::permission::PermissionLevel;
use yantrik_companion_core::tools::{Tool, ToolContext, ToolRegistry};
use yantrik_companion_core::types::{BondLevel, CompanionState, ModelTier, UrgeSpec};
use yantrik_companion_instincts::{CommitmentPrepInstinct, Instinct};

/// Monday 10:00 UTC. "Thursday" is then three days on, at five in the afternoon.
const MONDAY: f64 = 20738.0 * 86400.0 + 10.0 * 3600.0;
const DAY: f64 = 86400.0;
const HOUR: f64 = 3600.0;
const THURSDAY_DEADLINE: f64 = MONDAY - 10.0 * HOUR + 3.0 * DAY + 17.0 * HOUR;
const SAID: &str = "I'm sending Maya the Q4 deck on Thursday. The numbers are in the budget spreadsheet.";

const DECK: &str = "/home/p/Decks/Q4-deck.ydeck";
const SHEET: &str = "/home/p/Finance/budget-2026.csv";

/// What the fakes saw, and what the "disk" holds.
#[derive(Default)]
struct World {
    files: Vec<String>,
    calls: Vec<(String, Value)>,
    /// `app.action` pairs that are refused, with the sentence the app says.
    refused: Vec<(String, String)>,
}

type Shared = Arc<Mutex<World>>;

struct FakeGlob(Shared);
struct FakeApps(Shared);

impl Tool for FakeGlob {
    fn name(&self) -> &'static str { "glob" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "files" }
    fn definition(&self) -> Value { json!({ "type": "function", "function": { "name": "glob" } }) }
    fn execute(&self, _: &ToolContext, args: &Value) -> String {
        let mut w = self.0.lock().unwrap();
        w.calls.push(("glob".into(), args.clone()));
        let ext = args["pattern"].as_str().unwrap_or("").rsplit('.').next().unwrap_or("").to_string();
        let hits: Vec<&String> = w.files.iter().filter(|f| f.ends_with(&format!(".{ext}"))).collect();
        if hits.is_empty() {
            return format!("No files matching '{}' in ~", args["pattern"].as_str().unwrap_or(""));
        }
        let mut out: String = hits.iter().map(|f| format!("{f}\n")).collect();
        out.push_str(&format!("\n{} file(s) found", hits.len()));
        out
    }
}

impl Tool for FakeApps {
    fn name(&self) -> &'static str { "app_action" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "app" }
    fn definition(&self) -> Value { json!({ "type": "function", "function": { "name": "app_action" } }) }
    fn execute(&self, _: &ToolContext, args: &Value) -> String {
        let (app, action) = (args["app"].as_str().unwrap_or(""), args["action"].as_str().unwrap_or(""));
        let mut w = self.0.lock().unwrap();
        w.calls.push(("app_action".into(), args.clone()));
        if let Some((_, said)) = w.refused.iter().find(|(k, _)| *k == format!("{app}.{action}")) {
            return said.clone();
        }
        format!("{app}.{action} \u{2192} {{\"ok\":true}}")
    }
}

/// The snapshot an instinct reads: a quiet companion at `now`, with the scan's triggers.
fn state_with(now: f64, pending_triggers: Vec<Value>) -> CompanionState {
    CompanionState {
        last_interaction_ts: now - 60.0,
        current_ts: now,
        session_active: false,
        conversation_turn_count: 0,
        recent_valence_avg: None,
        pending_triggers,
        active_patterns: vec![],
        open_conflicts_count: 0,
        memory_count: 0,
        config_user_name: "Pranab".into(),
        model_tier: ModelTier::Medium,
        bond_level: BondLevel::Stranger,
        bond_score: 0.0,
        formality: 0.5,
        opinions_count: 0,
        shared_references_count: 0,
        bond_level_changed: false,
        current_hour: 9,
        current_day_of_week: 2,
        idle_seconds: 60.0,
        interactions_last_hour: 0,
        workflow_hints: vec![],
        maintenance_report: vec![],
        recent_events: vec![],
        avg_user_msg_length: 0.0,
        daily_proactive_count: 0,
        recent_sent_messages: vec![],
        suppressed_urges: vec![],
        user_interests: vec![],
        user_location: String::new(),
        open_loops_count: 0,
        overdue_commitment_count: 0,
        pending_attention_count: 0,
    }
}

struct Rig {
    db: yantrikdb_core::YantrikDB,
    registry: ToolRegistry,
    world: Shared,
}

impl Rig {
    fn new(files: &[&str]) -> Self {
        let world: Shared = Arc::new(Mutex::new(World { files: files.iter().map(|f| f.to_string()).collect(), ..World::default() }));
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(FakeGlob(world.clone())));
        registry.register(Box::new(FakeApps(world.clone())));
        let db = yantrikdb_core::YantrikDB::new(":memory:", 64).expect("in-memory database");
        WorldModel::ensure_tables(&db.conn());
        AnticipationEngine::ensure_table(&db.conn());
        Self { db, registry, world }
    }

    fn say(&self, text: &str, now: f64) {
        let source = CommitmentSource::Conversation { turn_id: None };
        let found = extract_commitments_at(text, "Pranab", &source, now);
        for c in to_world_model_commitments(&found, "Pranab", source) {
            WorldModel::insert_commitment(&self.db.conn(), &c);
        }
    }

    /// What the shell does each think cycle at `now`: the scan's triggers, the instinct, and the urges
    /// that name the executor, each run through `prepare`.
    fn cycle(&self, now: f64) -> Vec<Outcome> {
        let triggers: Vec<Value> = WorldModel::approaching_deadlines_at(&self.db.conn(), 24.0, now)
            .iter()
            .map(approaching_trigger)
            .collect();
        let urges: Vec<UrgeSpec> = CommitmentPrepInstinct.evaluate(&state_with(now, triggers));
        urges.iter().filter_map(commitment_to_prepare).map(|id| self.prepare(id, now)).collect()
    }

    fn prepare(&self, id: i64, now: f64) -> Outcome {
        let ctx = ToolContext {
            db: &self.db,
            max_permission: PermissionLevel::Standard,
            registry_metadata: None,
            task_manager: None,
            incognito: false,
            agent_spawner: None,
        };
        prepare(&self.registry, &ctx, id, now)
    }

    fn calls(&self) -> Vec<(String, Value)> {
        self.world.lock().unwrap().calls.clone()
    }

    fn audit_lines(&self, tool: &str) -> i64 {
        let conn: &Connection = &self.db.conn();
        conn.query_row(
            "SELECT COUNT(*) FROM memories WHERE domain = 'audit/tools' AND text LIKE ?1",
            [format!("Tool: {tool}(%")],
            |r| r.get(0),
        )
        .unwrap_or(0)
    }
}

fn the_item(outcomes: &[Outcome]) -> &yantrik_companion::anticipation::PreparedItem {
    match outcomes {
        [Outcome::Prepared(item)] => item,
        other => panic!("expected one prepared item, got {other:?}"),
    }
}

fn app_calls(rig: &Rig) -> Vec<(String, String)> {
    rig.calls()
        .iter()
        .filter(|(tool, _)| tool == "app_action")
        .map(|(_, a)| (a["app"].as_str().unwrap().to_string(), a["action"].as_str().unwrap().to_string()))
        .collect()
}

#[test]
fn said_on_monday_prepared_when_the_window_opens_and_nothing_is_sent() {
    let rig = Rig::new(&[DECK, SHEET, "/home/p/notes.md"]);

    // Day 1: said once. Stored with who it is for, when, and the files it names.
    rig.say(SAID, MONDAY);
    let stored = WorldModel::active_commitments(&rig.db.conn());
    assert_eq!(stored.len(), 1, "{stored:?}");
    assert_eq!(stored[0].promisee, "Maya");
    assert_eq!(stored[0].deadline, THURSDAY_DEADLINE);
    assert_eq!(stored[0].related_entities, vec!["attach:Q4 deck", "file:budget spreadsheet"]);

    // Monday, Tuesday, and Wednesday before the window: nothing is asked and nothing is done.
    for now in [MONDAY, MONDAY + DAY, MONDAY + 2.0 * DAY] {
        assert!(rig.cycle(now).is_empty(), "too early at {now}");
    }
    assert!(rig.calls().is_empty());

    // Wednesday 18:00: the window opens (Thursday 17:00 is 23 hours off).
    let outcomes = rig.cycle(MONDAY + 2.0 * DAY + 8.0 * HOUR);
    let item = the_item(&outcomes);

    // Both files opened in Mind View: the shell's launch (which routes a companion launch there)
    // and then the file, in the app for its kind.
    assert_eq!(
        app_calls(&rig),
        [
            ("shell", "open_app"),
            ("presentation", "open"),
            ("shell", "open_app"),
            ("spreadsheet", "open"),
            ("email", "compose"),
        ]
        .map(|(a, b)| (a.to_string(), b.to_string())),
    );
    let compose = rig.calls().into_iter().find(|(_, a)| a["action"] == "compose").unwrap().1;
    assert_eq!(compose["args"]["attach"], DECK, "the draft carries the deck");
    assert_eq!(compose["args"]["body"], "Hi Maya,\n\nHere is the Q4 deck.\n");

    // Nothing is sent: the only tools are the two, and no action sends.
    assert!(rig.calls().iter().all(|(tool, _)| tool == "glob" || tool == "app_action"));
    assert!(app_calls(&rig).iter().all(|(_, action)| !action.contains("send")));

    // The item keeps its result.
    assert_eq!(item.trigger, PreparationTrigger::CommitmentDeadline { commitment_id: stored[0].id.to_string() });
    assert_eq!(item.data["sent"], false);
    assert_eq!(item.data["attachment"]["path"], DECK);
    assert_eq!(item.data["draft"]["attached"], DECK);
    assert_eq!(item.data["draft"]["state"], "draft");
    assert_eq!(item.data["opened"][0]["result"]["in"], "mind_view");
    assert_eq!(item.data["opened"][1]["path"], SHEET);
    assert_eq!(item.data["missing"], json!([]));
    assert!(item.description.ends_with("Nothing sent."), "{}", item.description);

    // Every step is in the audit.
    assert_eq!(rig.audit_lines("app_action"), 5);
    assert!(rig.audit_lines("glob") >= 2);

    // Thursday 09:00: the brief shows it, once, and the person's use of it is recorded.
    let thursday = MONDAY + 3.0 * DAY - HOUR;
    let relevant = AnticipationEngine::get_relevant_at(&rig.db.conn(), thursday);
    assert_eq!(relevant.len(), 1);
    AnticipationEngine::mark_surfaced(&rig.db.conn(), &relevant[0].id);
    AnticipationEngine::record_outcome(&rig.db.conn(), &relevant[0].id, PrepOutcome::Useful);
    assert!(AnticipationEngine::get_relevant_at(&rig.db.conn(), thursday).is_empty());
    let kept = AnticipationEngine::for_commitment(&rig.db.conn(), &stored[0].id.to_string()).unwrap();
    assert!(kept.surfaced);
    assert_eq!(kept.outcome, Some(PrepOutcome::Useful));
    assert_eq!(kept.data["draft"]["attached"], DECK, "the result survives being surfaced");
}

#[test]
fn nothing_said_nothing_done() {
    let rig = Rig::new(&[DECK, SHEET]);
    rig.say("It is a fine morning and the coffee is hot.", MONDAY);
    assert!(rig.cycle(MONDAY + 2.0 * DAY + 8.0 * HOUR).is_empty());
    assert!(rig.calls().is_empty());
    assert!(AnticipationEngine::get_relevant_at(&rig.db.conn(), MONDAY + 3.0 * DAY).is_empty());
}

#[test]
fn a_file_that_cannot_be_found_is_said_in_the_persons_words() {
    // The deck is there; the spreadsheet is not.
    let rig = Rig::new(&[DECK]);
    rig.say(SAID, MONDAY);
    let outcomes = rig.cycle(MONDAY + 2.0 * DAY + 8.0 * HOUR);
    let item = the_item(&outcomes);
    assert_eq!(item.data["missing"], json!(["couldn't find the budget spreadsheet"]));
    assert!(item.description.contains("couldn't find the budget spreadsheet"), "{}", item.description);
    assert_eq!(item.data["opened"].as_array().unwrap().len(), 1, "only what exists was opened");
    assert_eq!(item.data["draft"]["attached"], DECK, "the deck was found, so the draft is real");

    // The deck is the thing being sent: without it there is no draft, which would pretend.
    let rig = Rig::new(&[SHEET]);
    rig.say(SAID, MONDAY);
    let outcomes = rig.cycle(MONDAY + 2.0 * DAY + 8.0 * HOUR);
    let item = the_item(&outcomes);
    assert_eq!(item.data["draft"], Value::Null);
    assert_eq!(item.data["attachment"], Value::Null);
    assert_eq!(item.data["missing"][0], "couldn't find the Q4 deck");
    assert!(app_calls(&rig).iter().all(|(app, _)| app != "email"), "no draft was made");
}

#[test]
fn a_launch_that_mind_view_refuses_is_not_retried_anywhere_else() {
    let rig = Rig::new(&[DECK, SHEET]);
    rig.world.lock().unwrap().refused.push((
        "shell.open_app".into(),
        "Mind View could not be started (no labwc); the app was not opened.".into(),
    ));
    rig.say(SAID, MONDAY);
    let outcomes = rig.cycle(MONDAY + 2.0 * DAY + 8.0 * HOUR);
    let item = the_item(&outcomes);
    assert_eq!(item.data["opened"][0]["result"]["opened"], false);
    assert!(item.data["opened"][0]["result"]["why"].as_str().unwrap().contains("Mind View could not be started"));
    // The file was not opened by driving an app that is not in Mind View.
    assert!(app_calls(&rig).iter().all(|(_, action)| action != "open"));
    assert_eq!(item.data["draft"]["state"], "draft", "the draft does not need Mind View");
}

#[test]
fn it_needs_no_research_and_no_web_tool() {
    // Research is a switch the tools' reach gate reads; this path has no tool that reaches out, so
    // it completes with only the two local tools registered: any web tool would be "Unknown tool".
    let rig = Rig::new(&[DECK, SHEET]);
    rig.say(SAID, MONDAY);
    let outcomes = rig.cycle(MONDAY + 2.0 * DAY + 8.0 * HOUR);
    let item = the_item(&outcomes);
    assert_eq!(item.data["draft"]["state"], "draft");
    let tools: std::collections::BTreeSet<_> = rig.calls().into_iter().map(|(t, _)| t).collect();
    assert_eq!(tools, ["app_action", "glob"].iter().map(|s| s.to_string()).collect());
    let apps: std::collections::BTreeSet<_> = app_calls(&rig).into_iter().map(|(a, _)| a).collect();
    for app in &apps {
        assert!(
            ["shell", "presentation", "spreadsheet", "document-editor", "email"].contains(&app.as_str()),
            "{app} is not one of the local apps this prepares with"
        );
    }
}

#[test]
fn the_same_commitment_is_not_prepared_twice() {
    let rig = Rig::new(&[DECK, SHEET]);
    rig.say(SAID, MONDAY);
    let window = MONDAY + 2.0 * DAY + 8.0 * HOUR;
    assert_eq!(rig.cycle(window).len(), 1);
    let calls_after_first = rig.calls().len();

    // The next think cycles, and a second run after the item was shown: the same answer, no work.
    for later in [window + 60.0, window + 12.0 * HOUR] {
        let outcomes = rig.cycle(later);
        assert!(matches!(outcomes.as_slice(), [Outcome::AlreadyPrepared]), "{outcomes:?}");
    }
    let id = WorldModel::active_commitments(&rig.db.conn())[0].id;
    AnticipationEngine::mark_surfaced(&rig.db.conn(), &format!("commitment-prep-{id}"));
    assert!(matches!(rig.prepare(id, window + DAY), Outcome::AlreadyPrepared));
    assert_eq!(rig.calls().len(), calls_after_first, "nothing was opened or drafted again");
    let all: i64 = rig.db.conn().query_row("SELECT COUNT(*) FROM prepared_items", [], |r| r.get(0)).unwrap();
    assert_eq!(all, 1);
}
