//! The keep-the-person's-config rule, run from the table both sides share
//! (harnesses/tests/fixtures/keep_live_cases.json: THE TABLE IS THE RULE; the Python side,
//! hermes_model.py, runs every row of it too, in harnesses/tests/test_keep_live_table.py).
//! Real apply and revert in a temp home; fake keys only.

use super::*;
use serde_json::Value;

const TABLE: &str = include_str!("../../../../harnesses/tests/fixtures/keep_live_cases.json");
const FAKE_TOKEN_NOTE: &str = "ygw-";

fn offer() -> Offer {
    Offer { token: Token(yantrik_gateway::tokens::mint().unwrap()), model: "picked".into(), models: Vec::new() }
}

fn gateway_plan(home: &Path) -> Plan {
    adapter_for("deepseek").unwrap().plan_gateway(home, &offer()).unwrap()
}

fn files_named(dir: &Path, infix: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.file_name().unwrap().to_string_lossy().contains(infix))
        .collect();
    v.sort();
    v
}

fn value_of(p: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

fn sorted(mut v: Vec<Value>) -> Vec<String> {
    let mut s: Vec<String> = v.drain(..).map(|x| x.to_string()).collect();
    s.sort();
    s
}

/// An expected value is equal to what is on disk, or `{"has": {k: v}}`: an object holding those keys
/// (a copy of an edited applied file also holds the tool's own keys, which differ on each side).
fn matches(expected: &Value, actual: &Value) -> bool {
    match expected.get("has").and_then(Value::as_object) {
        Some(h) => h.iter().all(|(k, v)| actual.get(k) == Some(v)),
        None => expected == actual,
    }
}

/// Every expected value is held by a different actual one, and there is no other.
fn same_set(expected: &[Value], actual: &[Value]) -> bool {
    let mut free: Vec<&Value> = actual.iter().collect();
    expected.len() == actual.len()
        && expected.iter().all(|e| free.iter().position(|a| matches(e, a)).map(|i| free.remove(i)).is_some())
}

/// The copies a result names hold exactly `expected`, and it names no other file.
fn check_names(said: Option<String>, dir: &Path, expected: &Value, what: &str) {
    let text = said.unwrap_or_default();
    assert!(!text.contains(FAKE_TOKEN_NOTE), "{what}: no token in a sentence: {text}");
    let named: Vec<Value> = files_named(dir, ".yantrik-replaced-").iter().filter(|p| text.contains(&p.display().to_string())).map(|p| value_of(p)).collect();
    assert!(same_set(expected.as_array().unwrap(), &named), "{what}: the copies it names: {named:?}; it said {text:?}");
}

fn run_case(case: &Value) {
    let name = case["name"].as_str().unwrap();
    let home = std::env::temp_dir().join(format!("handoff-table-{}-{}", std::process::id(), name.len() * 31 + name.bytes().map(usize::from).sum::<usize>()));
    let _ = std::fs::remove_dir_all(&home);
    let dir = home.join(".config/yantrik");
    std::fs::create_dir_all(&dir).unwrap();
    let live = dir.join("deepseek.json");
    let mut ever: Vec<Value> = Vec::new();
    // "plan_output": the person's file already holds exactly what the first Apply writes (#1205 round 3).
    let mut first_plan = (case["live"] == "plan_output").then(|| gateway_plan(&home));
    let plan_value = first_plan.as_ref().map(|p| serde_json::from_str::<Value>(&p.writes[0].content).unwrap());
    for (key, path) in [("live", live.clone()), ("stray", backup_path(&live))] {
        let v = if case[key] == "plan_output" { plan_value.clone().unwrap() } else { case[key].clone() };
        if !v.is_null() {
            std::fs::write(path, v.to_string()).unwrap();
            ever.push(v);
        }
    }
    for (i, step) in case["steps"].as_array().unwrap().iter().enumerate() {
        let what = format!("{name}, step {i} ({})", step["do"]);
        match step["do"].as_str().unwrap() {
            "apply" => {
                let plan = first_plan.take().unwrap_or_else(|| gateway_plan(&home));
                let marker = apply(&home, &plan).unwrap_or_else(|e| panic!("{what}: {e}"));
                check_names(marker.replaced_note(), &dir, &step["names"], &what);
            }
            "revert" => {
                let said = revert(&home, "deepseek").unwrap_or_else(|e| panic!("{what}: {e}"));
                if step["found_named"] == true {
                    let set_aside = files_named(&dir, ".before-yantrik.found-");
                    assert!(!set_aside.is_empty(), "{what}: something was set aside");
                    for p in set_aside {
                        assert!(said.clone().unwrap_or_default().contains(&p.display().to_string()), "{what}: names {}", p.display());
                    }
                }
                check_names(said, &dir, &step["names"], &what)
            }
            "edit_applied" => {
                // One key of the applied file changes; the gateway address stays.
                let mut v = value_of(&live);
                v[step["key"].as_str().unwrap()] = step["value"].clone();
                std::fs::write(&live, v.to_string()).unwrap();
                ever.push(serde_json::json!({"has": {step["key"].as_str().unwrap(): step["value"].clone()}}));
            }
            "edit" => {
                std::fs::write(&live, step["value"].to_string()).unwrap();
                ever.push(step["value"].clone());
            }
            "apply_fails" => {
                // Fails after the live write: the Mind it is told to post to is not there.
                let mut plan = gateway_plan(&home);
                plan.mind_post = Some(Token("x".into()));
                assert!(apply(&home, &plan).is_err(), "{what}");
            }
            "revert_stops_after_restore" => std::fs::rename(backup_path(&live), &live).unwrap(),
            other => panic!("unknown step {other}"),
        }
    }
    let on_disk = |p: &Path| if p.exists() { value_of(p) } else { Value::Null };
    let expect = &case["expect"];
    if expect["live"] == "plan_output" {
        assert_eq!(on_disk(&live), plan_value.clone().unwrap(), "{name}: live is the person's own bytes");
    } else if expect["live"] == "applied" {
        assert!(on_disk(&live)["base_url"].as_str().unwrap_or("").starts_with("http://127.0.0.1"), "{name}: live is the tool's own");
    } else {
        assert_eq!(on_disk(&live), expect["live"], "{name}: live");
    }
    let copies: Vec<Value> = files_named(&dir, ".yantrik-replaced-").iter().map(|p| value_of(p)).collect();
    let found: Vec<Value> = files_named(&dir, ".before-yantrik.found-").iter().map(|p| value_of(p)).collect();
    assert!(same_set(expect["copies"].as_array().unwrap(), &copies), "{name}: copies: {copies:?}");
    assert_eq!(sorted(found.clone()), sorted(expect["found"].as_array().unwrap().clone()), "{name}: found");
    assert_eq!(on_disk(&backup_path(&live)), expect["stray"], "{name}: what is left at the backup's name");
    // Nothing of the person's is gone.
    let mut kept: Vec<Value> = copies.into_iter().chain(found).collect();
    kept.push(on_disk(&live));
    kept.push(on_disk(&backup_path(&live)));
    for v in &ever {
        assert!(kept.iter().any(|k| matches(v, k)), "{name}: {v} was lost; on disk: {kept:?}");
    }
}

#[test]
fn every_row_of_the_shared_table() {
    let table: Value = serde_json::from_str(TABLE).unwrap();
    let (mut ran, mut left_out) = (0, 0);
    for case in table["cases"].as_array().unwrap() {
        let only = case["only"].as_array();
        if only.is_some_and(|o| !o.iter().any(|s| s == "rust")) {
            assert!(!case["why"].as_str().unwrap_or("").is_empty(), "a row left out of this side says why: {}", case["name"]);
            left_out += 1;
            continue;
        }
        run_case(case);
        ran += 1;
    }
    assert_eq!(ran + left_out, table["cases"].as_array().unwrap().len());
    assert!(ran >= 10, "the table has rows for this side: {ran}");
}
