//! A 400 `tool_use_failed` (the model's own malformed tool call) is sent again once.

use std::io::Cursor;
use std::sync::{Arc, Mutex};

use serde_json::json;

use crate::log::CallLog;
use crate::policy::Sharing;
use crate::server::Gateway;
use crate::tests::{answer, request, Fake};
use crate::tokens::{mint, Grant, Tokens};
use crate::upstream::{Reply, Unreachable, Upstream};

fn refused() -> (u16, String) {
    (400, json!({"error": {"message": "Tool call validation failed: attempted to call tool 'os_act</channel/>assistant</channel/>commentary' which was not in request.tools", "type": "invalid_request_error", "code": "tool_use_failed", "failed_generation": "{}"}}).to_string())
}

fn good() -> (u16, String) {
    (200, json!({"choices": [{"message": {"content": "hello"}}]}).to_string())
}

/// Answers its steps in order and keeps what it was sent.
struct Steps {
    steps: Mutex<Vec<(u16, String)>>,
    sent: Mutex<Vec<Vec<u8>>>,
    content_type: &'static str,
}

impl Upstream for Steps {
    fn chat(&self, _base_url: &str, _key: Option<&str>, body: &[u8]) -> Result<Reply, Unreachable> {
        self.sent.lock().unwrap().push(body.to_vec());
        let (status, text) = self.steps.lock().unwrap().remove(0);
        Ok(Reply { status, content_type: if status == 200 { self.content_type } else { "application/json" }.into(), retry_after: None, body: Box::new(Cursor::new(text.into_bytes())) })
    }
}

fn run(steps: Vec<(u16, String)>) -> (u16, String, Arc<Steps>, Arc<CallLog>) {
    run_as(steps, "application/json")
}

fn run_sse(steps: Vec<(u16, String)>) -> (u16, String, Arc<Steps>, Arc<CallLog>) {
    run_as(steps, "text/event-stream")
}

fn run_as(steps: Vec<(u16, String)>, content_type: &'static str) -> (u16, String, Arc<Steps>, Arc<CallLog>) {
    let tokens = Arc::new(Tokens::in_memory());
    let pi = mint().unwrap();
    tokens.register(&pi, Grant { harness: "pi".into(), private_context: false }).unwrap();
    let up = Arc::new(Steps { steps: Mutex::new(steps), sent: Mutex::new(vec![]), content_type });
    let log = Arc::new(CallLog::in_memory());
    let g = Gateway {
        tokens,
        accounts: Arc::new(Fake { private_mode: false, cloud: Sharing::Share, lane: None }),
        upstream: up.clone(),
        log: log.clone(),
        template_refused: Default::default(),
        params_refused: Default::default(), health: Default::default(), holds: Default::default(),
    };
    let ask = json!({"model": "cloud/gpt-oss-120b", "messages": [{"role": "user", "content": "hi"}]});
    let (status, body) = answer(&g, &request("POST", "/v1/chat/completions", Some(&pi), ask));
    (status, body, up, log)
}

#[test]
fn a_malformed_tool_call_is_sent_once_more_and_the_client_gets_the_answer() {
    let (status, body, up, log) = run(vec![refused(), good()]);
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("hello"), "{body}");
    let sent = up.sent.lock().unwrap();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0], sent[1], "the same call");
    let rows = log.recent();
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!(rows[0].rid, rows[1].rid);
    assert_eq!((rows[0].status, rows[0].resent_why.as_deref()), (400, None));
    assert_eq!((rows[1].status, rows[1].resent_why.as_deref()), (200, Some("model_tool_call")));
}

#[test]
fn a_second_refusal_reaches_the_client_and_nothing_is_sent_a_third_time() {
    let (status, body, up, log) = run(vec![refused(), refused(), good()]);
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("tool_use_failed"), "{body}");
    assert_eq!(up.sent.lock().unwrap().len(), 2);
    let rows = log.recent();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].rid, rows[1].rid);
    assert_eq!(rows[1].resent_why.as_deref(), Some("model_tool_call"));
}

#[test]
fn another_400_is_not_sent_again() {
    let other = (400, json!({"error": {"message": "context too long", "code": "context_length_exceeded"}}).to_string());
    let (status, body, up, log) = run(vec![other, good()]);
    assert_eq!(status, 400, "{body}");
    assert_eq!(up.sent.lock().unwrap().len(), 1);
    assert_eq!(log.recent().len(), 1);
}

fn stream(events: &[&str]) -> (u16, String) {
    (200, events.iter().map(|e| format!("data: {e}\n\n")).collect::<String>())
}

fn error_event() -> String {
    json!({"error": {"message": "Tool call validation failed: attempted to call tool 'os_act<|channel|>assistant<|channel|>commentary' which was not in request.tools", "type": "invalid_request_error", "code": "tool_use_failed"}}).to_string()
}

fn chunk() -> String {
    json!({"choices": [{"delta": {"content": "hi"}}]}).to_string()
}

#[test]
fn an_error_event_opening_a_200_stream_is_sent_again_and_the_client_gets_one_clean_stream() {
    let err = error_event();
    let ch = chunk();
    let (status, body, up, log) = run_sse(vec![stream(&[&err]), stream(&[&ch, "[DONE]"])]);
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("\"hi\"") && body.contains("[DONE]") && !body.contains("tool_use_failed"), "{body}");
    assert_eq!(up.sent.lock().unwrap().len(), 2);
    let rows = log.recent();
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!(rows[0].rid, rows[1].rid);
    assert_eq!(rows[0].resent_why, None);
    assert_eq!(rows[1].resent_why.as_deref(), Some("model_tool_call"));
}

#[test]
fn an_error_event_after_a_data_event_is_not_sent_again() {
    let err = error_event();
    let ch = chunk();
    let (status, body, up, log) = run_sse(vec![stream(&[&ch, &err]), stream(&[&ch, "[DONE]"])]);
    assert_eq!(status, 200);
    assert!(body.contains("tool_use_failed"), "the error reaches the client: {body}");
    assert_eq!(up.sent.lock().unwrap().len(), 1);
    assert_eq!(log.recent().len(), 1);
}

#[test]
fn an_error_event_twice_reaches_the_client_after_one_resend() {
    let err = error_event();
    let (status, body, up, log) = run_sse(vec![stream(&[&err]), stream(&[&err])]);
    assert_eq!(status, 200);
    assert!(body.contains("tool_use_failed"), "{body}");
    assert_eq!(up.sent.lock().unwrap().len(), 2);
    assert_eq!(log.recent().len(), 2);
}
