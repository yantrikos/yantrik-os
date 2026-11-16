//! #1247, review R11-M1: the companion's whole-screen reads are the shell's `read_screen`, asked of the
//! person on every call, and no tool here starts `grim`. A fake shell answers on a socket, as the
//! window tools' tests do, since a CI machine has no compositor.

use super::*;
use std::sync::{Arc, Mutex};

const FAKE_LINE: &str = "FAKE-SECRET-1247 tokens.json - gedit";

/// A shell whose `read_screen` needs a card, answered `answer`, and then reads one line.
fn desk(tag: &str, answer: &'static str) -> (Desk, Arc<Mutex<Vec<serde_json::Value>>>) {
    let dir = std::env::temp_dir().join(format!("yantrik-companion-screen-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let heard = crate::app_ui::tests::fake(dir.join("app-shell.sock"), move |params| match params["action"].as_str() {
        Some("read_screen") if params["grant"].is_null() => {
            serde_json::json!({"error": {"code": -32602, "message": "GRANT: shell.read_screen is graded `sensitive` and this machine is in ask mode"}})
        }
        Some("read_screen") => serde_json::json!({"result": {"accepted": true, "action_id": "shell#1", "settled": true, "result": {"lines": [{"text": FAKE_LINE, "box": [0, 0, 1, 1]}]}}}),
        Some("request_approval") => serde_json::json!({"result": {"result": {"request_id": "appr-1", "status": "pending"}}}),
        _ => serde_json::json!({"result": {"result": {"request_id": "appr-1", "status": answer}}}),
    });
    (Desk { dir: Some(dir), wait: std::time::Duration::from_millis(600), poll: std::time::Duration::from_millis(20) }, heard)
}

#[cfg(unix)]
#[test]
fn analyze_screen_and_screenshot_ask_the_person_through_the_shell() {
    let (desk, heard) = desk("granted", "granted");
    let said = analyze(&desk, "what is open?");
    assert!(said.contains("what is open?") && said.contains(FAKE_LINE), "allowed, so the text comes back: {said}");
    let asked = heard.lock().unwrap().clone();
    assert!(asked.iter().any(|p| p["action"] == "request_approval" && p["args"]["action"] == "read_screen"), "a card named read_screen: {asked:?}");

    let out = std::env::temp_dir().join(format!("yantrik-companion-screen-save-{}.txt", std::process::id()));
    let saved = save_text(&desk, &out.to_string_lossy(), "~/s.txt");
    assert!(saved.contains("no picture was taken"), "{saved}");
    assert_eq!(std::fs::read_to_string(&out).unwrap_or_default(), FAKE_LINE);
    let _ = std::fs::remove_file(out);
}

#[cfg(unix)]
#[test]
fn a_denied_card_returns_nothing_of_the_screen_and_saves_nothing() {
    let (desk, _) = desk("denied", "denied");
    let out = std::env::temp_dir().join(format!("yantrik-companion-screen-denied-{}.txt", std::process::id()));
    let _ = std::fs::remove_file(&out);
    for said in [analyze(&desk, "q"), save_text(&desk, &out.to_string_lossy(), "~/s.txt")] {
        assert!(said.contains("said no") && !said.contains("FAKE") && !said.contains("tokens"), "{said}");
    }
    assert!(!out.exists(), "nothing was written");
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

/// No tool of this crate starts `grim`, the program that reads the whole desktop with no one asked.
#[test]
fn no_tool_here_starts_grim() {
    let needle = format!("Command::new(\"{}{}\")", "gr", "im");
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut found = Vec::new();
    for entry in std::fs::read_dir(src).unwrap().flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "rs") && std::fs::read_to_string(&path).is_ok_and(|text| text.contains(&needle)) {
            found.push(path);
        }
    }
    assert!(found.is_empty(), "{found:?}");
}
