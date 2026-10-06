//! The gateway end to end, with an account list and an upstream that are stand-ins.

use std::io::{Cursor, Read, Write};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use yantrik_ml::model_caps::{caps_for, Reasoning};

use crate::http::Request;
use crate::log::CallLog;
use crate::policy::{Refusal, Target};
use crate::server::{bind, Accounts, Gateway, ModelInfo};
use crate::tokens::{mint, Grant, Tokens};
use crate::upstream::{Reply, Unreachable, Upstream};

const KEY: &str = "sk-upstream-secret-key";

struct Fake {
    private_mode: bool,
}

const PICKED: yantrik_ml::model_caps::Effort = yantrik_ml::model_caps::Effort::Medium;

impl Accounts for Fake {
    fn models(&self) -> Vec<ModelInfo> {
        vec![
            ModelInfo { id: "cloud/gpt-oss-120b".into(), account: "cloud".into(), name: "gpt-oss-120b".into(), caps: caps_for("groq", "gpt-oss-120b", None, None), local: false, private_ok: false },
            ModelInfo { id: "home/qwen3:8b".into(), account: "home".into(), name: "qwen3:8b".into(), caps: caps_for("ollama", "qwen3:8b", None, None), local: true, private_ok: false },
        ]
    }
    fn target(&self, account: &str, model: &str) -> Result<Target, Refusal> {
        let (base, local, reasoning) = match account {
            "cloud" => ("https://cloud.example/v1", false, Reasoning::OpenAiEffort),
            "home" => ("http://127.0.0.1:11434/v1", true, Reasoning::OllamaThink { levels: false }),
            _ => return Err(Refusal::new(404, "account_not_found", format!("There is no account `{account}`."))),
        };
        Ok(Target {
            account: account.into(),
            model: model.into(),
            base_url: base.into(),
            key: (!local).then(|| KEY.to_string()),
            efforts: reasoning.efforts(),
            reasoning,
            local,
            private_ok: false,
        })
    }
    fn private_mode(&self) -> bool {
        self.private_mode
    }
    fn effort_for(&self, harness: &str, model: &str) -> Option<yantrik_ml::model_caps::Effort> {
        (harness == "pi" && model == "cloud/gpt-oss-120b").then_some(PICKED)
    }
    fn picked_model(&self, harness: &str) -> Option<String> {
        (harness == "pi").then(|| "home/qwen3:8b".to_string())
    }
}

/// Records what it was sent and answers with `answer`.
#[derive(Default)]
struct Echo {
    sent: Mutex<Vec<(String, Option<String>, Value)>>,
    answer: Mutex<(u16, String, String)>,
}

impl Upstream for Echo {
    fn chat(&self, base_url: &str, key: Option<&str>, body: &[u8]) -> Result<Reply, Unreachable> {
        self.sent.lock().unwrap().push((base_url.to_string(), key.map(str::to_string), serde_json::from_slice(body).unwrap()));
        let (status, ctype, body) = self.answer.lock().unwrap().clone();
        Ok(Reply { status, content_type: ctype, body: Box::new(Cursor::new(body.into_bytes())) })
    }
}

fn gateway(private_mode: bool) -> (Arc<Gateway>, Arc<Echo>, String, String) {
    let tokens = Arc::new(Tokens::in_memory());
    let pi = mint().unwrap();
    let mind = mint().unwrap();
    tokens.register(&pi, Grant { harness: "pi".into(), private_context: false }).unwrap();
    tokens.register(&mind, Grant { harness: "mind".into(), private_context: true }).unwrap();
    let echo = Arc::new(Echo::default());
    *echo.answer.lock().unwrap() = (200, "application/json".into(), json!({"choices": [{"message": {"content": "hello"}}], "usage": {"prompt_tokens": 12, "completion_tokens": 3}}).to_string());
    let g = Arc::new(Gateway {
        tokens,
        accounts: Arc::new(Fake { private_mode }),
        upstream: echo.clone(),
        log: Arc::new(CallLog::in_memory()),
    });
    (g, echo, pi, mind)
}

fn request(method: &str, path: &str, token: Option<&str>, body: Value) -> Request {
    let mut headers = vec![("Content-Type".to_string(), "application/json".to_string())];
    if let Some(t) = token {
        headers.push(("Authorization".into(), format!("Bearer {t}")));
    }
    Request { method: method.into(), path: path.into(), headers, body: body.to_string().into_bytes() }
}

/// The status and the body of what the gateway wrote.
fn answer(g: &Gateway, req: &Request) -> (u16, String) {
    let mut out = Vec::new();
    g.respond(req, &mut out);
    let text = String::from_utf8(out).unwrap();
    let status = text.split_whitespace().nth(1).unwrap().parse().unwrap();
    let body = text.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default();
    (status, body)
}

#[test]
fn a_wrong_or_missing_token_is_refused_before_anything_else() {
    let (g, echo, _, _) = gateway(false);
    let chat = json!({"model": "cloud/gpt-oss-120b", "messages": []});
    for token in [None, Some("ygw-0000"), Some("sk-not-ours"), Some(&*format!("ygw-{}", "0".repeat(64)))] {
        let (status, body) = answer(&g, &request("POST", "/v1/chat/completions", token, chat.clone()));
        assert_eq!(status, 401, "{token:?}");
        assert!(body.contains("invalid_token"));
    }
    assert!(echo.sent.lock().unwrap().is_empty(), "nothing went upstream");
    assert_eq!(answer(&g, &request("GET", "/v1/models", None, json!({}))).0, 401);
}

#[test]
fn each_harness_has_its_own_token_and_a_new_one_replaces_the_old() {
    let tokens = Tokens::in_memory();
    let first = mint().unwrap();
    let second = mint().unwrap();
    assert_ne!(first, second);
    assert_eq!(first.len(), 4 + 64);
    tokens.register(&first, Grant { harness: "pi".into(), private_context: false }).unwrap();
    tokens.register(&mint().unwrap(), Grant { harness: "hermes".into(), private_context: false }).unwrap();
    assert_eq!(tokens.verify(&first).unwrap().harness, "pi");
    tokens.register(&second, Grant { harness: "pi".into(), private_context: false }).unwrap();
    assert!(tokens.verify(&first).is_none(), "one token per harness");
    assert_eq!(tokens.verify(&second).unwrap().harness, "pi");
    assert!(tokens.revoke("pi").unwrap());
    assert!(tokens.verify(&second).is_none(), "Revert stops it");
    assert_eq!(tokens.harnesses(), ["hermes"]);
    assert!(tokens.register("not-a-token", Grant { harness: "x".into(), private_context: false }).is_err());
}

#[test]
fn the_token_file_holds_hashes_never_tokens() {
    let dir = std::env::temp_dir().join(format!("gw-tokens-{}", std::process::id()));
    let path = dir.join("tokens.json");
    let t = mint().unwrap();
    Tokens::open(&path).register(&t, Grant { harness: "pi".into(), private_context: false }).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains(&t[4..]), "{text}");
    assert_eq!(Tokens::open(&path).verify(&t).unwrap().harness, "pi", "a restart keeps it");
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_call_goes_to_the_account_with_its_key_and_the_harness_never_sees_it() {
    let (g, echo, pi, _) = gateway(false);
    let (status, body) = answer(&g, &request("POST", "/v1/chat/completions", Some(&pi), json!({"model": "cloud/gpt-oss-120b", "effort": "high", "messages": [{"role": "user", "content": "secret plans"}]})));
    assert_eq!(status, 200);
    assert!(body.contains("hello") && !body.contains(KEY));
    let sent = echo.sent.lock().unwrap();
    let (base, key, upstream) = &sent[0];
    assert_eq!(base, "https://cloud.example/v1");
    assert_eq!(key.as_deref(), Some(KEY));
    assert_eq!(upstream["model"], "gpt-oss-120b");
    assert_eq!(upstream["reasoning_effort"], "high");
    let call = g.log.last_for("pi").unwrap();
    assert_eq!((call.account.as_str(), call.model.as_str(), call.effort.as_deref()), ("cloud", "gpt-oss-120b", Some("high")));
    assert_eq!((call.prompt_tokens, call.completion_tokens), (12, 3));
    let logged = serde_json::to_string(&call).unwrap();
    assert!(!logged.contains("secret plans") && !logged.contains("hello") && !logged.contains(KEY), "{logged}");
}

#[test]
fn a_stream_passes_through_as_it_comes_and_its_usage_is_counted() {
    let (g, echo, pi, _) = gateway(false);
    let stream = "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":1}}\n\ndata: [DONE]\n\n";
    *echo.answer.lock().unwrap() = (200, "text/event-stream".into(), stream.into());
    let mut out = Vec::new();
    g.respond(&request("POST", "/v1/chat/completions", Some(&pi), json!({"model": "home/qwen3:8b", "stream": true, "effort": "easy"})), &mut out);
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("Content-Type: text/event-stream") && text.ends_with(stream));
    let sent = &echo.sent.lock().unwrap()[0].2;
    assert_eq!(sent["think"], false, "easy on a think-switch model is thinking off");
    assert_eq!(sent["stream_options"]["include_usage"], true);
    let call = g.log.last_for("pi").unwrap();
    assert_eq!((call.prompt_tokens, call.completion_tokens), (5, 1));
}

#[test]
fn private_context_goes_only_where_the_person_allowed_it() {
    let (g, echo, pi, mind) = gateway(false);
    let ask = |t: &str, model: &str| answer(&g, &request("POST", "/v1/chat/completions", Some(t), json!({"model": model, "messages": []})));
    let (status, body) = ask(&mind, "cloud/gpt-oss-120b");
    assert_eq!(status, 403);
    assert!(body.contains("private_context_not_allowed"));
    assert_eq!(ask(&mind, "home/qwen3:8b").0, 200, "a local account keeps it on this network");
    assert_eq!(ask(&pi, "cloud/gpt-oss-120b").0, 200, "a harness that sends none may use any account");
    assert_eq!(echo.sent.lock().unwrap().len(), 2, "the refused call never left");
    assert_eq!(g.log.recent().iter().filter(|c| c.outcome == "private_context_not_allowed").count(), 1, "and was logged");
    // The mind is listed only what it may use.
    let (_, models) = answer(&g, &request("GET", "/v1/models", Some(&mind), json!({})));
    let ids: Vec<String> = serde_json::from_str::<Value>(&models).unwrap()["data"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap().to_string()).collect();
    assert_eq!(ids, ["picked", "home/qwen3:8b"]);
    let (_, models) = answer(&g, &request("GET", "/v1/models", Some(&pi), json!({})));
    let v: Value = serde_json::from_str(&models).unwrap();
    assert_eq!(v["data"].as_array().unwrap().len(), 3);
    assert_eq!(v["data"][0]["yantrik"]["picked"], "home/qwen3:8b");
    assert_eq!(v["data"][1]["yantrik"]["efforts"], json!(["easy", "medium", "high"]));
    assert!(!models.contains(KEY));
}

#[test]
fn private_mode_sends_nothing_off_this_network() {
    let (g, echo, pi, _) = gateway(true);
    let (status, body) = answer(&g, &request("POST", "/v1/chat/completions", Some(&pi), json!({"model": "cloud/gpt-oss-120b"})));
    assert_eq!(status, 403);
    assert!(body.contains("private_mode"));
    assert!(echo.sent.lock().unwrap().is_empty());
}

#[test]
fn unknown_models_web_pages_and_other_paths_are_refused_with_a_reason() {
    let (g, _, pi, _) = gateway(false);
    let (s, b) = answer(&g, &request("POST", "/v1/chat/completions", Some(&pi), json!({"model": "gpt-4o"})));
    assert_eq!(s, 404);
    assert!(b.contains("<account>/<model>"));
    assert_eq!(answer(&g, &request("POST", "/v1/chat/completions", Some(&pi), json!({"model": "nobody/x"}))).0, 404);
    assert_eq!(answer(&g, &request("GET", "/v1/chat/completions", Some(&pi), json!({}))).0, 405);
    assert_eq!(answer(&g, &request("GET", "/admin", Some(&pi), json!({}))).0, 404);
    let mut from_page = request("GET", "/v1/models", Some(&pi), json!({}));
    from_page.headers.push(("Origin".into(), "https://evil.example".into()));
    assert_eq!(answer(&g, &from_page).0, 403);
}

#[test]
fn it_listens_on_loopback_only_and_answers_over_a_real_socket() {
    for addr in ["0.0.0.0:0", "192.168.1.5:0", "[::]:0", "localhost:7460"] {
        assert!(bind(addr).is_err(), "{addr}");
    }
    assert_eq!(crate::ADDR, format!("127.0.0.1:{}", crate::PORT));
    let listener = bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let (g, _, pi, _) = gateway(false);
    std::thread::spawn(move || g.serve(listener));
    let mut s = std::net::TcpStream::connect(addr).unwrap();
    write!(s, "GET /v1/models HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {pi}\r\n\r\n").unwrap();
    let mut text = String::new();
    s.read_to_string(&mut text).unwrap();
    assert!(text.starts_with("HTTP/1.1 200"), "{text}");
    assert!(text.contains("cloud/gpt-oss-120b"));
}

#[test]
fn usage_is_read_from_a_body_or_the_end_of_a_stream() {
    use crate::server::usage;
    assert_eq!(usage(br#"{"usage":{"prompt_tokens":7,"completion_tokens":2}}"#), Some((7, 2)));
    assert_eq!(usage(b"data: {\"x\":1}\n\ndata: {\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":9}}\n\ndata: [DONE]\n"), Some((1, 9)));
    assert_eq!(usage(b"data: {\"x\":1}\n"), None);
}

#[test]
fn the_effort_picked_for_a_mind_applies_when_its_request_names_none() {
    let (g, echo, pi, _) = gateway(false);
    answer(&g, &request("POST", "/v1/chat/completions", Some(&pi), json!({"model": "cloud/gpt-oss-120b"})));
    answer(&g, &request("POST", "/v1/chat/completions", Some(&pi), json!({"model": "cloud/gpt-oss-120b", "effort": "high"})));
    let sent = echo.sent.lock().unwrap();
    assert_eq!(sent[0].2["reasoning_effort"], "medium", "the picker's choice, for a harness that sends none");
    assert_eq!(sent[1].2["reasoning_effort"], "high", "a request's own word wins");
}

#[test]
fn picked_is_the_model_the_person_picked_for_that_mind_at_each_call() {
    let (g, echo, pi, mind) = gateway(false);
    let (status, _) = answer(&g, &request("POST", "/v1/chat/completions", Some(&pi), json!({"model": "picked"})));
    assert_eq!(status, 200);
    assert_eq!(echo.sent.lock().unwrap()[0].2["model"], "qwen3:8b");
    assert_eq!(g.log.last_for("pi").unwrap().account, "home");
    let (status, body) = answer(&g, &request("POST", "/v1/chat/completions", Some(&mind), json!({"model": "picked"})));
    assert_eq!(status, 404, "nothing picked for it: said, not guessed");
    assert!(body.contains("pick one in the ask bar"));
}
