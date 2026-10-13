//! The gateway end to end, with an account list and an upstream that are stand-ins.

use std::io::{Cursor, Read, Write};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use yantrik_ml::model_caps::{caps_for, Reasoning};

use crate::http::Request;
use crate::log::CallLog;
use crate::policy::{Refusal, Sharing, Target};
use crate::server::{bind, Accounts, Gateway, ModelInfo};
use crate::tokens::{mint, Grant, Tokens};
use crate::upstream::{Reply, Unreachable, Upstream};

const KEY: &str = "sk-upstream-secret-key";

struct Fake {
    private_mode: bool,
    /// What the person chose for `cloud`.
    cloud: Sharing,
    lane: Option<&'static str>,
}

const PICKED: yantrik_ml::model_caps::Effort = yantrik_ml::model_caps::Effort::Medium;

impl Accounts for Fake {
    fn models(&self) -> Vec<ModelInfo> {
        vec![
            ModelInfo { id: "cloud/gpt-oss-120b".into(), account: "cloud".into(), name: "gpt-oss-120b".into(), caps: caps_for("groq", "gpt-oss-120b", None, None), local: false, sharing: self.cloud },
            ModelInfo { id: "home/qwen3:8b".into(), account: "home".into(), name: "qwen3:8b".into(), caps: caps_for("ollama", "qwen3:8b", None, None), local: true, sharing: Sharing::Share },
        ]
    }
    fn target(&self, account: &str, model: &str) -> Result<Target, Refusal> {
        let (base, local, reasoning) = match account {
            "cloud" => ("https://cloud.example/v1", false, Reasoning::OpenAiEffort),
            "home" => ("http://127.0.0.1:11434/v1", true, Reasoning::OllamaThink { levels: false }),
            "elsewhere" => ("https://elsewhere.example/v1", false, Reasoning::None),
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
            sharing: if local { Sharing::Share } else { self.cloud },
            provider: format!("{account} provider"),
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
    fn private_lane(&self) -> Option<String> {
        self.lane.map(str::to_string)
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
    gateway_with(Fake { private_mode, cloud: Sharing::Off, lane: None })
}

fn gateway_with(fake: Fake) -> (Arc<Gateway>, Arc<Echo>, String, String) {
    let tokens = Arc::new(Tokens::in_memory());
    let pi = mint().unwrap();
    let mind = mint().unwrap();
    tokens.register(&pi, Grant { harness: "pi".into(), private_context: false }).unwrap();
    tokens.register(&mind, Grant { harness: "mind".into(), private_context: true }).unwrap();
    let echo = Arc::new(Echo::default());
    *echo.answer.lock().unwrap() = (200, "application/json".into(), json!({"choices": [{"message": {"content": "hello"}}], "usage": {"prompt_tokens": 12, "completion_tokens": 3}}).to_string());
    let g = Arc::new(Gateway {
        tokens,
        accounts: Arc::new(fake),
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
fn whoami_names_the_tokens_own_harness_and_nothing_without_one() {
    let (g, _, pi, mind) = gateway(false);
    let (status, body) = answer(&g, &request("GET", "/v1/whoami", Some(&mind), json!({})));
    assert_eq!(status, 200);
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap(), json!({"harness": "mind"}), "only the harness id");
    let (status, body) = answer(&g, &request("GET", "/v1/whoami", Some(&pi), json!({})));
    assert_eq!((status, serde_json::from_str::<Value>(&body).unwrap()), (200, json!({"harness": "pi"})));
    for token in [None, Some("ygw-0000"), Some(&*format!("ygw-{}", "0".repeat(64)))] {
        assert_eq!(answer(&g, &request("GET", "/v1/whoami", token, json!({}))).0, 401, "{token:?}");
    }
    assert_eq!(answer(&g, &request("POST", "/v1/whoami", Some(&mind), json!({}))).0, 405);
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
fn a_token_issued_without_private_context_is_marked_as_carrying_it() {
    let tokens = Tokens::in_memory();
    let t = mint().unwrap();
    tokens.register(&t, Grant { harness: "pi".into(), private_context: false }).unwrap();
    assert_eq!(tokens.mark_all_private().unwrap(), 1);
    assert!(tokens.verify(&t).unwrap().private_context);
    assert_eq!(tokens.mark_all_private().unwrap(), 0);
}

#[test]
fn a_token_another_process_wrote_is_accepted_and_kept() {
    let dir = std::env::temp_dir().join(format!("gw-tokens-shared-{}", std::process::id()));
    let path = dir.join("tokens.json");
    let shell = Tokens::open(&path);
    let mind = mint().unwrap();
    shell.register(&mind, Grant { harness: "mind".into(), private_context: true }).unwrap();
    // `yantrik-ui --wire-model hermes`, another process, while the shell runs. Its write lands
    // in a later second than the shell's, as it would.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let hermes = mint().unwrap();
    Tokens::open(&path).register(&hermes, Grant { harness: "hermes".into(), private_context: true }).unwrap();
    assert_eq!(shell.verify(&hermes).unwrap().harness, "hermes", "the shell reads what was wired");
    // And the shell's next change keeps it.
    shell.register(&mint().unwrap(), Grant { harness: "pi".into(), private_context: true }).unwrap();
    assert!(Tokens::open(&path).verify(&hermes).is_some());
    assert!(shell.verify(&mind).is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_grant_in_the_file_cannot_say_it_carries_no_private_context() {
    // The file is writable by anything running as the person: a grant there marked as carrying
    // no private context would send a call past the person's Share / Private lane / Off choice.
    let dir = std::env::temp_dir().join(format!("gw-tokens-forged-{}", std::process::id()));
    let path = dir.join("tokens.json");
    let shell = Tokens::open(&path);
    shell.register(&mint().unwrap(), Grant { harness: "mind".into(), private_context: true }).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let forged = mint().unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let mut file: Value = serde_json::from_str(&text).unwrap();
    let d: String = <sha2::Sha256 as sha2::Digest>::digest(forged.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
    file["tokens"][d] = json!({"harness": "hermes", "private_context": false});
    std::fs::write(&path, file.to_string()).unwrap();
    assert!(shell.verify(&forged).unwrap().private_context, "read again, and held to the person's choice");
    assert!(Tokens::open(&path).verify(&forged).unwrap().private_context, "and at a restart");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_token_another_process_revoked_stops_at_once() {
    let dir = std::env::temp_dir().join(format!("gw-tokens-revoked-{}", std::process::id()));
    let path = dir.join("tokens.json");
    let shell = Tokens::open(&path);
    let hermes = mint().unwrap();
    shell.register(&hermes, Grant { harness: "hermes".into(), private_context: true }).unwrap();
    assert!(shell.verify(&hermes).is_some());
    std::thread::sleep(std::time::Duration::from_millis(1100));
    assert!(Tokens::open(&path).revoke("hermes").unwrap());
    assert!(shell.verify(&hermes).is_none(), "a token found before is not kept after it was withdrawn");
    let _ = std::fs::remove_dir_all(&dir);
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
fn private_context_off_refuses_and_names_the_fix() {
    let (g, echo, pi, mind) = gateway(false);
    let ask = |t: &str, model: &str| answer(&g, &request("POST", "/v1/chat/completions", Some(t), json!({"model": model, "messages": []})));
    let (status, body) = ask(&mind, "cloud/gpt-oss-120b");
    assert_eq!(status, 403);
    assert!(body.contains("private_context_not_shared"));
    assert!(body.contains("Set up a private lane, or allow sharing with cloud provider in AI accounts"), "{body}");
    assert_eq!(ask(&mind, "home/qwen3:8b").0, 200, "a local account keeps it on this network");
    assert_eq!(ask(&pi, "cloud/gpt-oss-120b").0, 200, "a harness that sends none may use any account");
    assert_eq!(echo.sent.lock().unwrap().len(), 2, "the refused call never left");
    assert_eq!(g.log.recent().iter().filter(|c| c.outcome == "private_context_not_shared").count(), 1, "and was logged");
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
fn shared_private_context_goes_to_the_account_unchanged() {
    let (g, echo, _, mind) = gateway_with(Fake { private_mode: false, cloud: Sharing::Share, lane: Some("home/qwen3:8b") });
    let body = json!({"model": "cloud/gpt-oss-120b", "messages": [{"role": "user", "content": "you live in Pune"}]});
    assert_eq!(answer(&g, &request("POST", "/v1/chat/completions", Some(&mind), body)).0, 200);
    let sent = echo.sent.lock().unwrap();
    assert_eq!(sent[0].0, "https://cloud.example/v1");
    assert_eq!(sent[0].2["messages"][0]["content"], "you live in Pune", "nothing stripped");
    let call = g.log.last_for("mind").unwrap();
    assert_eq!((call.account.as_str(), call.lane_for.as_deref()), ("cloud", None));
}

#[test]
fn the_private_lane_answers_calls_that_carry_private_context_and_the_log_says_so() {
    let (g, echo, pi, mind) = gateway_with(Fake { private_mode: false, cloud: Sharing::Lane, lane: Some("home/qwen3:8b") });
    let ask = |t: &str| answer(&g, &request("POST", "/v1/chat/completions", Some(t), json!({"model": "cloud/gpt-oss-120b", "messages": []})));
    assert_eq!(ask(&mind).0, 200);
    {
        let sent = echo.sent.lock().unwrap();
        assert_eq!((sent[0].0.as_str(), sent[0].1.as_deref()), ("http://127.0.0.1:11434/v1", None), "to the lane, with no cloud key");
        assert_eq!(sent[0].2["model"], "qwen3:8b");
    }
    let call = g.log.last_for("mind").unwrap();
    assert_eq!((call.account.as_str(), call.model.as_str()), ("home", "qwen3:8b"), "the log names who answered");
    assert_eq!(call.lane_for.as_deref(), Some("cloud/gpt-oss-120b"));
    // A call without private context still goes to the account.
    assert_eq!(ask(&pi).0, 200);
    assert_eq!(echo.sent.lock().unwrap()[1].0, "https://cloud.example/v1");
    assert_eq!(g.log.last_for("pi").unwrap().lane_for, None);
    // Listed, since the lane answers them.
    let (_, models) = answer(&g, &request("GET", "/v1/models", Some(&mind), json!({})));
    assert!(models.contains("cloud/gpt-oss-120b"));
}

#[test]
fn a_private_lane_that_is_missing_or_not_local_refuses_with_the_fix_and_sends_nothing() {
    for lane in [None, Some("elsewhere/big"), Some("not a model")] {
        let (g, echo, _, mind) = gateway_with(Fake { private_mode: false, cloud: Sharing::Lane, lane });
        let (status, body) = answer(&g, &request("POST", "/v1/chat/completions", Some(&mind), json!({"model": "cloud/gpt-oss-120b"})));
        assert_eq!(status, 403, "{lane:?}");
        assert!(body.contains("private_context_not_shared") && body.contains("Set up a private lane"), "{body}");
        assert!(echo.sent.lock().unwrap().is_empty(), "{lane:?}: nothing left");
    }
}

#[test]
fn private_mode_refuses_every_cloud_call_whatever_the_sharing() {
    for cloud in [Sharing::Share, Sharing::Lane, Sharing::Off] {
        let (g, echo, pi, mind) = gateway_with(Fake { private_mode: true, cloud, lane: Some("home/qwen3:8b") });
        for t in [&pi, &mind] {
            let (status, body) = answer(&g, &request("POST", "/v1/chat/completions", Some(t), json!({"model": "cloud/gpt-oss-120b"})));
            assert_eq!(status, 403);
            assert!(body.contains("private_mode"));
        }
        assert!(echo.sent.lock().unwrap().is_empty());
    }
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

#[test]
fn a_host_that_is_not_loopback_is_refused_so_a_rebound_page_gets_nothing() {
    let (g, _, pi, _) = gateway(false);
    for (host, status) in [("127.0.0.1:7460", 200), ("localhost:7460", 200), ("127.0.0.1", 200), ("[::1]:7460", 200), ("evil.example:7460", 403), ("127.0.0.1.evil.example", 403), ("localhost.evil.example:7460", 403)] {
        let mut req = request("GET", "/v1/models", Some(&pi), json!({}));
        req.headers.push(("Host".into(), host.into()));
        assert_eq!(answer(&g, &req).0, status, "{host}");
    }
}

/// Counts what is read from it.
struct Counted<R> {
    inner: R,
    read: Arc<Mutex<usize>>,
}

impl<R: Read> Read for Counted<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        *self.read.lock().unwrap() += n;
        Ok(n)
    }
}

#[test]
fn a_header_that_never_ends_is_refused_without_being_read_whole() {
    let mut endless = b"GET /v1/models HTTP/1.1\r\nX-Pad: ".to_vec();
    endless.extend(std::iter::repeat(b'a').take(8 * 1024 * 1024));
    let read = Arc::new(Mutex::new(0));
    let got = crate::http::read_request(Counted { inner: Cursor::new(endless), read: read.clone() });
    assert_eq!(got.unwrap_err().0, 431);
    assert!(*read.lock().unwrap() <= 64 * 1024, "read {} bytes of an endless header", read.lock().unwrap());
    let mut endless_line = b"GET /".to_vec();
    endless_line.extend(std::iter::repeat(b'a').take(8 * 1024 * 1024));
    assert_eq!(crate::http::read_request(Cursor::new(endless_line)).unwrap_err().0, 431);
}

#[test]
fn a_request_that_drips_in_is_cut_off_at_its_deadline() {
    let listener = bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let (g, _, _, _) = gateway(false);
    let served = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let started = std::time::Instant::now();
        g.handle_within(stream, std::time::Duration::from_millis(400));
        started.elapsed()
    });
    let mut s = std::net::TcpStream::connect(addr).unwrap();
    write!(s, "GET /v1/models HTTP/1.1\r\n").unwrap();
    // One byte every 50 ms: never quiet long enough for a per-read timeout.
    for _ in 0..60 {
        if s.write_all(b"a").is_err() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let took = served.join().unwrap();
    assert!(took < std::time::Duration::from_secs(2), "held for {took:?}");
}

#[test]
fn connections_past_the_limit_are_closed_unread() {
    let listener = bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let (g, _, _, _) = gateway(false);
    std::thread::spawn(move || g.serve(listener));
    // Every slot held by a connection that says nothing.
    let held: Vec<_> = (0..crate::server::MAX_CONNECTIONS).map(|_| std::net::TcpStream::connect(addr).unwrap()).collect();
    let mut extra = std::net::TcpStream::connect(addr).unwrap();
    extra.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
    let mut buf = [0u8; 16];
    assert_eq!(extra.read(&mut buf).unwrap(), 0, "closed at once, not served");
    drop(held);
}
