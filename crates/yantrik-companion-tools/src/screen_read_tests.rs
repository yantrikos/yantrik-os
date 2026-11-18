//! #1247, review R11-M1: the companion's whole-screen reads are the shell's `read_screen`, asked of the
//! person on every call, and no tool here starts `grim`. A fake shell answers on a socket, as the
//! window tools' tests do, since a CI machine has no compositor.

use super::*;
use std::sync::{Arc, Mutex};

const FAKE_LINE: &str = "FAKE-SECRET-1247 tokens.json - gedit";

const SHOT_PATH: &str = "/home/ann/Pictures/Screenshots/shot-1.png";

/// A shell whose `read_screen` and `screenshot_screen` need a card, answered `answer`: the first reads one
/// line, the second saves a picture at `SHOT_PATH`.
fn desk(tag: &str, answer: &'static str) -> (Desk, Arc<Mutex<Vec<serde_json::Value>>>) {
    let dir = std::env::temp_dir().join(format!("yantrik-companion-screen-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let heard = crate::app_ask::tests::fake(dir.join("app-shell.sock"), move |params| match params["action"].as_str() {
        Some(action @ ("read_screen" | "screenshot_screen")) if params["grant"].is_null() => {
            serde_json::json!({"error": {"code": -32602, "message": format!("GRANT: shell.{action} is graded `sensitive` and this machine is in ask mode")}})
        }
        Some("read_screen") => serde_json::json!({"result": {"accepted": true, "action_id": "shell#1", "settled": true, "result": {"lines": [{"text": FAKE_LINE, "box": [0, 0, 1, 1]}]}}}),
        Some("screenshot_screen") => serde_json::json!({"result": {"accepted": true, "action_id": "shell#2", "settled": true, "result": {"saved": SHOT_PATH, "display": "desktop"}}}),
        Some("request_approval") => serde_json::json!({"result": {"result": {"request_id": "appr-1", "status": "pending"}}}),
        _ => serde_json::json!({"result": {"result": {"request_id": "appr-1", "status": answer}}}),
    });
    (Desk { dir: Some(dir), wait: std::time::Duration::from_millis(600), poll: std::time::Duration::from_millis(20) }, heard)
}

fn cards(heard: &Arc<Mutex<Vec<serde_json::Value>>>) -> Vec<String> {
    let asked = heard.lock().unwrap().clone();
    asked.iter().filter(|p| p["action"] == "request_approval").filter_map(|p| p["args"]["action"].as_str().map(String::from)).collect()
}

/// The tools themselves: `analyze_screen` asks for `read_screen` and answers its text; `screenshot` asks
/// for `screenshot_screen` and answers only the path the shell saved the picture at.
#[cfg(unix)]
#[test]
fn analyze_screen_reads_text_and_screenshot_takes_a_picture_each_after_its_own_card() {
    let (read_desk, heard) = desk("granted", "granted");
    let said = crate::vision::AnalyzeScreenTool.run(&read_desk, &serde_json::json!({"question": "what is open?"}));
    assert!(said.contains("what is open?") && said.contains(FAKE_LINE), "allowed, so the text comes back: {said}");
    assert_eq!(cards(&heard), ["read_screen"]);

    let (shot_desk, heard) = desk("granted-shot", "granted");
    let said = crate::media::ScreenshotTool.run(&shot_desk, &serde_json::json!({}));
    assert!(said.contains(SHOT_PATH), "the reply carries the path: {said}");
    assert!(!said.contains("FAKE") && !said.contains("tokens"), "and no text of the screen: {said}");
    assert_eq!(cards(&heard), ["screenshot_screen"], "the card is for the picture, not for a read of the text");
    assert!(heard.lock().unwrap().iter().all(|p| p["action"] != "read_screen"), "the text was never read");
}

#[cfg(unix)]
#[test]
fn a_denied_card_returns_nothing_of_the_screen_and_takes_no_picture() {
    let (denied, heard) = desk("denied", "denied");
    for said in [crate::vision::AnalyzeScreenTool.run(&denied, &serde_json::json!({})), crate::media::ScreenshotTool.run(&denied, &serde_json::json!({}))] {
        assert!(said.contains("said no") && !said.contains("FAKE") && !said.contains("tokens") && !said.contains("Screenshots"), "{said}");
    }
    let asked = heard.lock().unwrap().clone();
    assert!(asked.iter().all(|p| p["grant"].is_null()), "no action ran with a grant: {asked:?}");
}

/// With no shell to ask, the tools refuse: they never fall back to reading the screen themselves.
#[test]
fn with_no_shell_the_tools_refuse() {
    let dir = std::env::temp_dir().join(format!("yantrik-companion-screen-none-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let desk = Desk { dir: Some(dir), wait: std::time::Duration::from_millis(100), poll: std::time::Duration::from_millis(20) };
    let said = analyze(&desk, "q");
    assert!(said.contains("failed") && !said.contains("Screenshot"), "{said}");
}

/// No tool of this crate starts `grim`, the program that reads the whole desktop with no one asked: not as
/// `Command::new("grim")`, and not as a `"grim"` string handed to anything that starts programs. Test files
/// and everything after a `#[cfg(test)]` are left out.
#[test]
fn no_tool_here_starts_grim() {
    let needle = format!("\"{}{}\"", "gr", "im");
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut found = Vec::new();
    for entry in std::fs::read_dir(src).unwrap().flatten() {
        let path = entry.path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if !name.ends_with(".rs") || name.ends_with("tests.rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let outside_tests = text.split("#[cfg(test)]").next().unwrap_or_default();
        if outside_tests.contains(&needle) {
            found.push(path);
        }
    }
    assert!(found.is_empty(), "{found:?}");
}
