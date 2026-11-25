//! What Pi is told of each model's room (#1402).

use std::collections::HashMap;

use yantrik_gateway::Room;

use super::*;
use super::tests::{json, Home};

const GROQ: &str = "free-groq/openai/gpt-oss-120b";
const BIG: &str = "cloud/big-model";

/// The room of Groq's free tier for its 120B model: 8,000 tokens a minute, a 131,072 context.
pub(super) fn rooms() -> HashMap<String, Room> {
    [(GROQ.to_string(), Room { per_minute: Some(8_000), context: Some(131_072), ..Room::default() })].into()
}

fn model(id: &str, context: u32) -> crate::ai_accounts::models::CatalogueModel {
    let (account, name) = id.split_once('/').unwrap();
    let caps = yantrik_ml::model_caps::caps_for("openai", name, Some(context), Some(true));
    crate::ai_accounts::models::CatalogueModel { id: id.into(), account: account.into(), model: name.into(), name: name.into(), caps }
}

/// A fake token: nothing here is a real one.
fn offer(picked: &str, rooms: HashMap<String, Room>) -> Offer {
    Offer { token: Token("yantrik-test-token-not-real".into()), model: picked.into(), models: vec![model(GROQ, 131_072), model(BIG, 200_000)], rooms }
}

fn models_of(home: &Home) -> serde_json::Value {
    json(&home.0.join(".pi/agent/models.json"))["providers"]["yantrik"]["models"].clone()
}

#[test]
fn a_free_groq_model_is_told_the_eight_thousand_it_takes_and_a_large_paid_one_its_own_context() {
    let home = Home::new("pi-room");
    let plan = adapter_for("pi").unwrap().plan_gateway(&home.0, &offer(GROQ, rooms())).unwrap();
    apply(&home.0, &plan).unwrap();
    let models = models_of(&home);
    let by_id = |id: &str| models.as_array().unwrap().iter().find(|m| m["id"] == id).unwrap().clone();
    // The pick in the ask bar is the Groq model: `picked` is told its room.
    assert_eq!((by_id("picked")["contextWindow"].as_u64(), by_id("picked")["maxTokens"].as_u64()), (Some(8_000), Some(2_000)));
    assert_eq!((by_id(GROQ)["contextWindow"].as_u64(), by_id(GROQ)["maxTokens"].as_u64()), (Some(8_000), Some(2_000)));
    // A paid model whose account states no limit: its own context, and the answer Pi always asked for.
    assert_eq!((by_id(BIG)["contextWindow"].as_u64(), by_id(BIG)["maxTokens"].as_u64()), (Some(200_000), Some(32_768)));
    // Pi's own compaction numbers only where its defaults (16,384 reserved, 20,000 kept) cannot work.
    let settings = json(&home.0.join(".pi/agent/settings.json"));
    let over = &settings["compaction"]["modelOverrides"];
    assert_eq!(over["yantrik/picked"], serde_json::json!({"reserveTokens": 2_000, "keepRecentTokens": 1_000}));
    assert_eq!(over[format!("yantrik/{GROQ}")], over["yantrik/picked"]);
    assert!(over[format!("yantrik/{BIG}")].is_null(), "a room the defaults suit keeps them");
    assert!(!format!("{plan:?}").contains("yantrik-test-token-not-real"), "the token is not in Debug");
}

#[test]
fn a_model_whose_room_nothing_states_is_told_the_old_128k() {
    let home = Home::new("pi-room-unknown");
    let plan = adapter_for("pi").unwrap().plan_gateway(&home.0, &offer("picked", HashMap::new())).unwrap();
    apply(&home.0, &plan).unwrap();
    let models = models_of(&home);
    assert_eq!((models[0]["contextWindow"].as_u64(), models[0]["maxTokens"].as_u64()), (Some(128_000), Some(32_768)));
    assert!(!home.0.join(".pi/agent/settings.json").exists(), "no settings file is made for nothing");
}

#[test]
fn the_person_s_own_pi_settings_stay_beside_ours_and_revert_puts_them_back() {
    let home = Home::new("pi-room-settings");
    std::fs::create_dir_all(home.0.join(".pi/agent")).unwrap();
    let own = r#"{"theme":"dark","compaction":{"enabled":true,"modelOverrides":{"ollama/x":{"reserveTokens":1}}}}"#;
    std::fs::write(home.0.join(".pi/agent/settings.json"), own).unwrap();
    apply(&home.0, &adapter_for("pi").unwrap().plan_gateway(&home.0, &offer(GROQ, rooms())).unwrap()).unwrap();
    let now = json(&home.0.join(".pi/agent/settings.json"));
    assert_eq!((now["theme"].as_str(), now["compaction"]["enabled"].as_bool()), (Some("dark"), Some(true)));
    assert_eq!(now["compaction"]["modelOverrides"]["ollama/x"]["reserveTokens"], 1);
    assert_eq!(now["compaction"]["modelOverrides"]["yantrik/picked"]["reserveTokens"], 2_000);
    revert(&home.0, "pi").unwrap();
    assert_eq!(std::fs::read_to_string(home.0.join(".pi/agent/settings.json")).unwrap(), own);
}

#[test]
fn the_files_are_written_again_when_the_pick_changes_under_the_token_pi_holds() {
    let home = Home::new("pi-room-pick");
    apply(&home.0, &adapter_for("pi").unwrap().plan_gateway(&home.0, &offer(GROQ, rooms())).unwrap()).unwrap();
    let again = |picked: &str| pi_plan_for_new_pick(&home.0, |token| {
        assert_eq!(token.0, "yantrik-test-token-not-real", "the token Pi holds, not a new one");
        Ok(offer(picked, rooms()))
    });
    assert!(again(GROQ).unwrap().is_none(), "the same pick changes nothing Pi reads, so nothing restarts");
    let plan = again(BIG).unwrap().expect("a pick with other room is a rewrite");
    assert!(plan.restart.is_none(), "a running Pi is not restarted: it would lose its conversation");
    apply(&home.0, &plan).unwrap();
    let models = models_of(&home);
    assert_eq!((models[0]["id"].as_str(), models[0]["contextWindow"].as_u64()), (Some("picked"), Some(128_000)), "picked follows the pick");
    let settings = json(&home.0.join(".pi/agent/settings.json"));
    assert!(settings["compaction"]["modelOverrides"]["yantrik/picked"].is_null(), "and the small-room numbers go with it");
    assert!(settings["compaction"]["modelOverrides"][format!("yantrik/{GROQ}")].is_object(), "while the Groq model keeps its own");
    revert(&home.0, "pi").unwrap();
    assert!(pi_plan_for_new_pick(&home.0, |_| panic!("Pi is not on Yantrik models any more")).unwrap().is_none());
}

/// What the build before this one wrote: the room of no model known, so 131,072 and 128,000 with
/// 32,768 answers, and no settings.json (the old code wrote neither the numbers nor the file).
fn as_the_old_build_wrote_it(home: &Home) {
    let plan = adapter_for("pi").unwrap().plan_gateway(&home.0, &offer(GROQ, HashMap::new())).unwrap();
    apply(&home.0, &plan).unwrap();
    let before = models_of(home);
    assert_eq!(before[0]["contextWindow"], 128_000);
    assert_eq!(before[1]["contextWindow"], 131_072);
    assert!(!home.0.join(".pi/agent/settings.json").exists());
}

#[test]
fn a_machine_updated_from_the_old_numbers_gets_the_new_ones_when_the_shell_starts() {
    let home = Home::new("pi-room-update");
    as_the_old_build_wrote_it(&home);
    // The installer added `memory` to pi.json right after the handoff wrote it (every install with
    // memory on): that file differs from the marker's record and is not this refresh's to write.
    let pi_json = home.0.join(".config/yantrik/pi.json");
    let mut pi = json(&pi_json);
    pi["memory"] = "yantrikdb".into();
    std::fs::write(&pi_json, serde_json::to_string_pretty(&pi).unwrap()).unwrap();
    let pi_bytes = std::fs::read(&pi_json).unwrap();
    // The shell starts: the same call the pick makes, with what this build knows of each room.
    let plan = pi_plan_for_new_pick(&home.0, |_| Ok(offer(GROQ, rooms()))).unwrap().expect("old numbers are rewritten");
    assert!(plan.restart.is_none(), "Pi keeps its conversation; its harness makes it read the files again");
    apply(&home.0, &plan).unwrap();
    let models = models_of(&home);
    assert_eq!((models[0]["contextWindow"].as_u64(), models[0]["maxTokens"].as_u64()), (Some(8_000), Some(2_000)));
    assert_eq!(models[1]["contextWindow"], 8_000);
    let settings = json(&home.0.join(".pi/agent/settings.json"));
    assert_eq!(settings["compaction"]["modelOverrides"]["yantrik/picked"], serde_json::json!({"reserveTokens": 2_000, "keepRecentTokens": 1_000}));
    assert_eq!(std::fs::read(&pi_json).unwrap(), pi_bytes, "pi.json is not written: its provider and model do not change with the room");
    assert!(pi_plan_for_new_pick(&home.0, |_| Ok(offer(GROQ, rooms()))).unwrap().is_none(), "the next start changes nothing");
    revert(&home.0, "pi").unwrap();
    assert!(!home.0.join(".pi/agent/settings.json").exists(), "Revert removes the file the handoff made");
    assert!(!pi_json.exists(), "and the record of pi.json was kept through the refresh, so Revert still removes it");
}

#[test]
fn a_file_the_person_edited_by_hand_is_left_alone_at_start_and_at_a_pick() {
    let home = Home::new("pi-room-hand");
    as_the_old_build_wrote_it(&home);
    let path = home.0.join(".pi/agent/models.json");
    let mut text = std::fs::read_to_string(&path).unwrap();
    text.push_str("\n");
    std::fs::write(&path, &text).unwrap();
    assert!(pi_plan_for_new_pick(&home.0, |_| Ok(offer(GROQ, rooms()))).unwrap().is_none(), "theirs now");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text, "and not a byte of it written");
    assert!(!home.0.join(".pi/agent/settings.json").exists());
}

#[test]
fn an_override_of_ours_that_the_person_changed_in_settings_is_left_alone_but_their_other_keys_are_not_a_reason() {
    let home = Home::new("pi-room-settings-hand");
    apply(&home.0, &adapter_for("pi").unwrap().plan_gateway(&home.0, &offer(GROQ, rooms())).unwrap()).unwrap();
    let path = home.0.join(".pi/agent/settings.json");
    // Another key added by the person: the file differs from the marker's record, but no override of ours does.
    let mut settings = json(&path);
    settings["theme"] = "dark".into();
    std::fs::write(&path, serde_json::to_string_pretty(&settings).unwrap()).unwrap();
    let plan = pi_plan_for_new_pick(&home.0, |_| Ok(offer(BIG, rooms()))).unwrap().expect("their other keys are merged, not a reason to stop");
    apply(&home.0, &plan).unwrap();
    assert_eq!(json(&path)["theme"], "dark");
    // An override of ours with a key of their own in it is theirs.
    let mut settings = json(&path);
    settings["compaction"]["modelOverrides"][format!("yantrik/{GROQ}")]["reserveTokens"] = 1.5.into();
    std::fs::write(&path, serde_json::to_string_pretty(&settings).unwrap()).unwrap();
    let before = std::fs::read(&path).unwrap();
    assert!(pi_plan_for_new_pick(&home.0, |_| Ok(offer(GROQ, rooms()))).unwrap().is_none());
    assert_eq!(std::fs::read(&path).unwrap(), before);
}
