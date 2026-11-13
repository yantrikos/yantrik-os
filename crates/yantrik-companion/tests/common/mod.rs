//! A rig for the commitment executor's tests (#1184): a fake disk and fake apps registered in a
//! real `ToolRegistry`, so every step goes through `ToolRegistry::execute` and is written to the
//! audit as in the shell; and a clock that is a number the test moves.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use rusqlite::Connection;
use serde_json::{json, Value};
use yantrik_companion::anticipation::{AnticipationEngine, PreparedItem};
use yantrik_companion::commitment_extractor::{extract_commitments_at, to_world_model_commitments};
use yantrik_companion::commitment_prep::{approaching_trigger, commitment_to_prepare, prepare_with, Outcome, Settings};
use yantrik_companion::{commitment_consent, commitment_store};
use yantrik_companion::world_model::{CommitmentSource, WorldModel};
use yantrik_companion_core::permission::PermissionLevel;
use yantrik_companion_core::tools::{Tool, ToolContext, ToolRegistry};
use yantrik_companion_core::types::{BondLevel, CompanionState, ModelTier, UrgeSpec};
use yantrik_companion_instincts::{CommitmentPrepInstinct, Instinct};

/// Monday 10:00 UTC, in the test's own local zone's terms: the deadline is computed by the code
/// under test (`THURSDAY` helper below), never by arithmetic here.
pub const MONDAY: f64 = 20738.0 * 86400.0 + 10.0 * 3600.0;
pub const DAY: f64 = 86400.0;
pub const HOUR: f64 = 3600.0;
pub const SAID: &str = "I'm sending Maya the Q4 deck on Thursday. The numbers are in the budget spreadsheet.";

pub const DOCS: &str = "/home/p/Documents";
pub const DECK: &str = "/home/p/Documents/Q4-deck.ydeck";
pub const SHEET: &str = "/home/p/Documents/budget-2026.csv";

/// What the fakes saw, and what the "disk" and the apps say.
pub struct World {
    pub files: Vec<String>,
    pub calls: Vec<(String, Value)>,
    /// `app.action` pairs that fail, with the sentence the app says; each is used up by a call
    /// when `once` is true.
    pub refused: Vec<(String, String, bool)>,
    /// What the shell says `where` is for a launch.
    pub lands_in: String,
    /// The most paths `glob` lists, as the real tool does.
    pub list_at_most: usize,
    pub web_calls: usize,
}

pub type Shared = Arc<Mutex<World>>;

struct FakeGlob(Shared);
struct FakeApps(Shared);
struct FakeWeb(Shared);

impl Tool for FakeGlob {
    fn name(&self) -> &'static str { "glob" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "files" }
    fn definition(&self) -> Value { json!({ "type": "function", "function": { "name": "glob" } }) }
    fn execute(&self, _: &ToolContext, args: &Value) -> String {
        let mut w = self.0.lock().unwrap();
        w.calls.push(("glob".into(), args.clone()));
        let ext = args["pattern"].as_str().unwrap_or("").rsplit('.').next().unwrap_or("").to_string();
        let root = args["path"].as_str().unwrap_or("~").to_string();
        let hits: Vec<&String> = w.files.iter().filter(|f| f.ends_with(&format!(".{ext}")) && f.starts_with(&root)).collect();
        if hits.is_empty() {
            return format!("No files matching '{}' in {root}", args["pattern"].as_str().unwrap_or(""));
        }
        let mut out: String = hits.iter().take(w.list_at_most).map(|f| format!("{f}\n")).collect();
        if hits.len() > w.list_at_most {
            out.push_str(&format!("... and {} more files\n", hits.len() - w.list_at_most));
        }
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
        let key = format!("{app}.{action}");
        if let Some(at) = w.refused.iter().position(|(k, _, _)| *k == key) {
            let said = w.refused[at].1.clone();
            if w.refused[at].2 {
                w.refused.remove(at);
            }
            return said;
        }
        if key == "shell.open_app" {
            return format!("shell.open_app \u{2192} {}", json!({ "launching": args["args"]["name"], "window": "appeared", "where": w.lands_in }));
        }
        format!("{key} \u{2192} {{\"ok\":true}}")
    }
}

impl Tool for FakeWeb {
    fn name(&self) -> &'static str { "web_fetch" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "web" }
    fn definition(&self) -> Value { json!({ "type": "function", "function": { "name": "web_fetch" } }) }
    fn execute(&self, _: &ToolContext, _: &Value) -> String {
        self.0.lock().unwrap().web_calls += 1;
        "fetched".into()
    }
}

/// The snapshot an instinct reads: a quiet companion at `now`, with the scan's triggers.
pub fn state_with(now: f64, pending_triggers: Vec<Value>) -> CompanionState {
    CompanionState {
        last_interaction_ts: now - 60.0, current_ts: now, session_active: false, conversation_turn_count: 0,
        recent_valence_avg: None, pending_triggers, active_patterns: vec![], open_conflicts_count: 0, memory_count: 0,
        config_user_name: "Pranab".into(), model_tier: ModelTier::Medium, bond_level: BondLevel::Stranger,
        bond_score: 0.0, formality: 0.5, opinions_count: 0, shared_references_count: 0, bond_level_changed: false,
        current_hour: 9, current_day_of_week: 2, idle_seconds: 60.0, interactions_last_hour: 0, workflow_hints: vec![],
        maintenance_report: vec![], recent_events: vec![], avg_user_msg_length: 0.0, daily_proactive_count: 0,
        recent_sent_messages: vec![], suppressed_urges: vec![], user_interests: vec![], user_location: String::new(),
        open_loops_count: 0, overdue_commitment_count: 0, pending_attention_count: 0,
    }
}

pub struct Rig {
    pub db: yantrikdb_core::YantrikDB,
    pub registry: ToolRegistry,
    pub world: Shared,
    pub settings: Settings,
    pub incognito: bool,
}

impl Rig {
    /// A rig whose person has said yes to having promises prepared.
    pub fn new(files: &[&str]) -> Self {
        let rig = Self::asked(files);
        rig.person_turns_it_on();
        rig
    }

    /// A rig whose person has not been asked yet.
    pub fn asked(files: &[&str]) -> Self {
        let world: Shared = Arc::new(Mutex::new(World {
            files: files.iter().map(|f| f.to_string()).collect(),
            calls: Vec::new(),
            refused: Vec::new(),
            lands_in: "Mind View".into(),
            list_at_most: 50,
            web_calls: 0,
        }));
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(FakeGlob(world.clone())));
        registry.register(Box::new(FakeApps(world.clone())));
        registry.register(Box::new(FakeWeb(world.clone())));
        let db = yantrikdb_core::YantrikDB::new(":memory:", 64).expect("in-memory database");
        WorldModel::ensure_tables(&db.conn());
        AnticipationEngine::ensure_table(&db.conn());
        let settings = Settings { roots: vec![DOCS.into(), "/home/p/Desktop".into(), "/home/p/Downloads".into()], budget: Duration::from_secs(60) };
        Self { db, registry, world, settings, incognito: false }
    }

    /// The person's own button: preparing off, then back on, which is consent by the person-only act.
    pub fn person_turns_it_on(&self) {
        commitment_store::refuse(&self.db.conn());
        assert!(commitment_consent::person_answer(&self.db.conn(), commitment_consent::TURN_ON).is_some());
    }

    /// The person is shown the question on their brief at `now`, and presses the button `which`
    /// (`YES` or `NO`) on it.
    pub fn person_answers(&self, which: &str, now: f64) -> Option<String> {
        let shown = commitment_consent::brief_actions(&self.db.conn(), now);
        let yes = &shown.first()?.action_id;
        let button = if which == commitment_consent::NO { commitment_consent::no_for(yes)? } else { yes.clone() };
        commitment_consent::person_answer(&self.db.conn(), &button)
    }

    pub fn say(&self, text: &str, now: f64) -> usize {
        let source = CommitmentSource::Conversation { turn_id: None };
        let found = extract_commitments_at(text, "Pranab", &source, now);
        let all = to_world_model_commitments(&found, "Pranab", source);
        for c in &all {
            WorldModel::insert_commitment(&self.db.conn(), c);
        }
        all.len()
    }

    /// What the shell does each think cycle at `now`: the scan's triggers, the instinct, and the
    /// urges that name the executor, each run through `prepare_with`.
    pub fn cycle(&self, now: f64) -> Vec<Outcome> {
        let triggers: Vec<Value> = commitment_store::approaching_at(&self.db.conn(), 24.0, now).iter().map(approaching_trigger).collect();
        let urges: Vec<UrgeSpec> = CommitmentPrepInstinct.evaluate(&state_with(now, triggers));
        urges.iter().filter_map(commitment_to_prepare).map(|id| self.prepare(id, now)).collect()
    }

    pub fn prepare(&self, id: i64, now: f64) -> Outcome {
        let ctx = ToolContext {
            db: &self.db,
            max_permission: PermissionLevel::Standard,
            registry_metadata: None,
            task_manager: None,
            incognito: self.incognito,
            agent_spawner: None,
        };
        prepare_with(&self.registry, &ctx, id, now, &self.settings)
    }

    pub fn calls(&self) -> Vec<(String, Value)> {
        self.world.lock().unwrap().calls.clone()
    }

    pub fn audit_lines(&self, tool: &str) -> i64 {
        let conn: &Connection = &self.db.conn();
        conn.query_row("SELECT COUNT(*) FROM memories WHERE domain = 'audit/tools' AND text LIKE ?1", [format!("Tool: {tool}(%")], |r| r.get(0))
            .unwrap_or(0)
    }

    pub fn app_calls(&self) -> Vec<(String, String)> {
        self.calls()
            .iter()
            .filter(|(tool, _)| tool == "app_action")
            .map(|(_, a)| (a["app"].as_str().unwrap().to_string(), a["action"].as_str().unwrap().to_string()))
            .collect()
    }

    pub fn items(&self) -> i64 {
        self.db.conn().query_row("SELECT COUNT(*) FROM prepared_items", [], |r| r.get(0)).unwrap()
    }

    pub fn only_commitment(&self) -> yantrik_companion::world_model::Commitment {
        let all = WorldModel::active_commitments(&self.db.conn());
        assert_eq!(all.len(), 1, "{all:?}");
        all.into_iter().next().unwrap()
    }

    pub fn item_for(&self, id: i64) -> PreparedItem {
        AnticipationEngine::for_commitment(&self.db.conn(), &id.to_string()).expect("an item")
    }
}

pub fn the_item(outcomes: &[Outcome]) -> &PreparedItem {
    match outcomes {
        [Outcome::Prepared(item)] => item,
        other => panic!("expected one prepared item, got {other:?}"),
    }
}

/// Thursday's deadline for a promise said at `MONDAY`, as the extractor works it out.
pub fn thursday() -> f64 {
    yantrik_companion::commitment_when::next_weekday_at_five(MONDAY, 3, &chrono::Local).unwrap()
}

/// A moment inside Thursday's prep window: 23 hours before the deadline.
pub fn window() -> f64 {
    thursday() - 23.0 * HOUR
}
