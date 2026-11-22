//! #1357 round 4: an app's gate has the shell write the line.

use super::*;
use serde_json::json;

fn lines() -> Vec<serde_json::Value> {
    std::fs::read_to_string(crate::mind_mode::audit_path()).unwrap_or_default().lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

#[test]
fn the_shell_writes_the_line_with_its_own_mode_and_the_whole_arguments() {
    let command = format!("echo {}", "q".repeat(1500));
    write("an app's gate (test)", "gate-test-a", "run", "sensitive", "bypass_all", &json!({ "command": command }), false).unwrap();
    let line = lines().into_iter().rev().find(|v| v["app"] == "gate-test-a").expect("the line is in the file");
    assert!(line["args_full"]["text"].as_str().unwrap().contains(&command), "whole");
    assert!(line["mode"].as_str().unwrap().starts_with(crate::mind_mode::current().as_str()), "the shell's own mode: {}", line["mode"]);
    assert!(line["mode"].as_str().unwrap().contains("the gate read bypass_all") || crate::mind_mode::current().as_str() == "bypass_all");
    assert!(line["requester"].as_str().unwrap().starts_with("an app's gate"));
}

#[test]
fn arguments_past_a_megabyte_are_not_written_and_so_not_run() {
    let huge = json!({ "command": "x".repeat(crate::audit_store::SIDE_FILE_MAX + 10) });
    let err = write("an app's gate (test)", "gate-test-b", "run", "sensitive", "bypass", &huge, false).unwrap_err();
    assert!(err.contains("written down in full"), "{err}");
    assert!(!lines().iter().any(|v| v["app"] == "gate-test-b"), "no line says it ran");
}

/// One test at a time changes the shell's mode, and puts it back.
static MODE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn call(app: &str) -> Result<serde_json::Value, String> {
    handle(&json!({ "app": app, "action": "run", "grade": "sensitive", "mode": "bypass_all", "args_json": json!({ "command": "ls" }).to_string() }))
}

fn caller(uid: u32, the_mind: bool) -> yantrik_app_runtime::control::CallerScope {
    yantrik_app_runtime::control::CallerScope::enter(Some(yantrik_app_runtime::control::Caller { pid: 1, uid, gid: 0, the_mind }))
}

fn full_bypass() -> (impl Drop, std::sync::MutexGuard<'static, ()>) {
    struct Back;
    impl Drop for Back {
        fn drop(&mut self) {
            crate::mind_mode::person_set_mode(crate::mind_mode::Mode::Ask, crate::mind_mode::Bypass::Hour);
        }
    }
    let one = MODE.lock().unwrap_or_else(|e| e.into_inner());
    crate::mind_mode::person_set_mode(crate::mind_mode::Mode::BypassAll, crate::mind_mode::Bypass::Hour);
    (Back, one)
}

/// A Mind is refused through the handler, whether it comes as the mind account or with a token, and
/// nothing is written: it cannot forge the record or push real entries out of what the person reads.
#[test]
fn the_handler_refuses_a_mind_by_account_and_by_token_even_in_full_bypass() {
    let _mode = full_bypass();
    {
        let _who = caller(4242, true);
        let err = call("gate-test-mind").unwrap_err();
        assert!(err.contains("not one"), "{err}");
    }
    {
        let _token = yantrik_app_runtime::control::AgentTokenScope::enter(Some("an-agent-token".into()));
        let err = call("gate-test-token").unwrap_err();
        assert!(err.contains("not one"), "{err}");
    }
    assert!(!lines().iter().any(|v| v["app"] == "gate-test-mind" || v["app"] == "gate-test-token"), "no line was written");
}

#[test]
fn the_handler_refuses_every_mode_that_is_not_a_bypass() {
    let _one = MODE.lock().unwrap_or_else(|e| e.into_inner());
    for mode in [crate::mind_mode::Mode::Plan, crate::mind_mode::Mode::Ask, crate::mind_mode::Mode::Auto] {
        crate::mind_mode::person_set_mode(mode, crate::mind_mode::Bypass::Hour);
        let err = call("gate-test-mode").unwrap_err();
        assert!(err.contains("nothing runs unasked"), "{mode:?}: {err}");
    }
    crate::mind_mode::person_set_mode(crate::mind_mode::Mode::Ask, crate::mind_mode::Bypass::Hour);
    assert!(!lines().iter().any(|v| v["app"] == "gate-test-mode"));
}

/// The call an app's gate makes, in Full bypass, as the person's own process with no token: written,
/// with the reply the gates read. No writer is injected: this is the handler and the real record.
#[test]
fn an_app_gate_in_full_bypass_is_written_through_the_handler() {
    let _mode = full_bypass();
    let _who = caller(unsafe { libc::getuid() }, false);
    assert_eq!(call("gate-test-ok").unwrap(), json!({ "written": true }));
    let line = lines().into_iter().rev().find(|v| v["app"] == "gate-test-ok").expect("the line is in the file");
    assert_eq!(line["action"], "run");
    assert!(line["requester"].as_str().unwrap().starts_with("an app's gate"));
    assert!(line["args_full"]["text"].as_str().unwrap().contains("ls"));
}

/// While the screen is locked an unasked app action still has to be recorded (D: the screen locks
/// after five idle minutes, and Full bypass must not stop whenever the person steps away). The
/// handler's own checks are what keep a Mind out, so this is the one action let through.
#[test]
fn the_record_is_let_through_while_the_desktop_is_locked_and_nothing_else_is() {
    assert!(crate::control::allowed_while_locked(yantrik_ipc_transport::unasked::ACTION));
    assert!(!crate::control::allowed_while_locked("run_command"));
}

#[test]
fn the_refusal_is_pure_and_each_way_in_is_one_case() {
    use crate::mind_mode::Mode;
    assert!(refusal(true, false, Mode::BypassAll).is_some());
    assert!(refusal(false, true, Mode::Bypass).is_some());
    assert!(refusal(false, false, Mode::Ask).is_some());
    assert!(refusal(false, false, Mode::Bypass).is_none() && refusal(false, false, Mode::BypassAll).is_none());
}

/// The gates' own call carries no token: it is `app.act` with `action` and `args` and nothing else,
/// so a gate is never mistaken for an agent.
#[test]
fn the_rust_gates_own_call_carries_no_token() {
    let params = yantrik_ipc_transport::unasked::request_params(&yantrik_ipc_transport::unasked::Record { app: "a", action: "b", grade: "sensitive", mode: "bypass", args: &json!({}) });
    assert!(params.get("agent_token").is_none() && params["args"].get("agent_token").is_none(), "{params}");
}

/// What `app.act` answers around the handler's result, as the control surface builds it.
fn act_reply(result: serde_json::Value) -> serde_json::Value {
    yantrik_ipc_contracts::control_surface::act_json_stateless("shell", "act-1", true, result)
}

fn python(code: &str, stdin: &str) -> String {
    use std::io::Write;
    let sdk = concat!(env!("CARGO_MANIFEST_DIR"), "/../../sdk/python");
    let program = format!("import sys, json\nsys.path.insert(0, {sdk:?})\nfrom yantrik_surface import unasked\n{code}");
    let mut child = std::process::Command::new("python3")
        .args(["-I", "-c", &program])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("python3 runs");
    child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "python failed");
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

/// A Rust app's gate and a Python `Surface` each make the call their clients make, and the real
/// `record_gate_decision` handler answers it in Full bypass, with no writer injected: the line is in
/// the file with the whole arguments, and each client reads "written" in the reply the control
/// surface builds. (The socket hop is the one thing left out: `must_be_the_shell` refuses a test
/// binary, as it should.)
#[test]
fn a_rust_gate_and_a_python_surface_reach_the_real_handler_and_read_its_reply() {
    use yantrik_ipc_transport::unasked::{recorded_in_reply, request_params, Record};
    let _mode = full_bypass();
    let _who = caller(unsafe { libc::getuid() }, false);

    // The Rust gate.
    let args = json!({ "command": "echo rust-gate" });
    let params = request_params(&Record { app: "e2e-rust-gate", action: "run", grade: "sensitive", mode: "bypass_all", args: &args });
    let reply = act_reply(handle(&params["args"]).expect("the handler writes it"));
    assert_eq!(reply["result"], json!({ "written": true }), "the reply shape the Rust gate reads");
    recorded_in_reply(&reply).expect("the Rust gate reads it as written");
    let line = lines().into_iter().rev().find(|v| v["app"] == "e2e-rust-gate").expect("the Rust gate's line");
    assert!(line["args_full"]["text"].as_str().unwrap().contains("echo rust-gate") && line["requester"].as_str().unwrap().starts_with("an app's gate"));

    // The Python Surface.
    let py_params = python(
        "print(json.dumps(unasked.request_params('e2e-python-surface', 'run', 'sensitive', 'bypass_all', {'command': 'echo py-gate'})))",
        "",
    );
    let py_params: serde_json::Value = serde_json::from_str(&py_params).unwrap();
    assert!(py_params.get("agent_token").is_none(), "{py_params}");
    let rpc = json!({ "jsonrpc": "2.0", "id": 1, "result": act_reply(handle(&py_params["args"]).expect("the handler writes it")) });
    assert_eq!(python("print(unasked._written(json.load(sys.stdin)))", &rpc.to_string()), "True", "the Python gate reads it as written");
    let line = lines().into_iter().rev().find(|v| v["app"] == "e2e-python-surface").expect("the Python gate's line");
    assert!(line["args_full"]["text"].as_str().unwrap().contains("echo py-gate"));
    // A refusal is not "written" to either client.
    assert_eq!(python("print(unasked._written(json.load(sys.stdin)))", &json!({ "error": { "message": "refused" } }).to_string()), "False");
    assert!(recorded_in_reply(&json!({ "error": "refused" })).is_err());
}
