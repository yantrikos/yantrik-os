//! A tight account is kept for what somebody asked: the companion's own marked calls to it are
//! refused before anything is sent, and nothing else is.

use std::io::Cursor;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use crate::http::Request;
use crate::log::{Call, CallLog};
use crate::policy::{Refusal, Sharing, Target};
use crate::server::{Accounts, Gateway, ModelInfo};
use crate::tests::{answer, request, Fake};
use crate::tokens::{mint, Grant, Tokens};
use crate::upstream::{Reply, Unreachable, Upstream};

/// `Fake`'s accounts, with `cloud` allowing few requests or not.
struct Tightness {
    inner: Fake,
    cloud_is_tight: bool,
}

impl Accounts for Tightness {
    fn models(&self) -> Vec<ModelInfo> {
        self.inner.models()
    }
    fn target(&self, account: &str, model: &str) -> Result<Target, Refusal> {
        let mut target = self.inner.target(account, model)?;
        target.tight = target.account == "cloud" && self.cloud_is_tight;
        Ok(target)
    }
    fn private_mode(&self) -> bool {
        false
    }
}

/// Counts what reaches the provider.
#[derive(Default)]
struct Upstairs {
    sent: Mutex<usize>,
}

impl Upstream for Upstairs {
    fn chat(&self, _base_url: &str, _key: Option<&str>, _body: &[u8]) -> Result<Reply, Unreachable> {
        *self.sent.lock().unwrap() += 1;
        let body = json!({"choices": [{"message": {"content": "hello"}}]}).to_string();
        Ok(Reply { status: 200, content_type: "application/json".into(), retry_after: None, body: Box::new(Cursor::new(body.into_bytes())) })
    }
}

struct Rig {
    gateway: Gateway,
    upstream: Arc<Upstairs>,
    companion: String,
    pi: String,
}

fn rig(cloud_is_tight: bool) -> Rig {
    let tokens = Arc::new(Tokens::in_memory());
    let (companion, pi) = (mint().unwrap(), mint().unwrap());
    tokens.register(&companion, Grant { harness: crate::asked::COMPANION.into(), private_context: true }).unwrap();
    tokens.register(&pi, Grant { harness: "pi".into(), private_context: false }).unwrap();
    let upstream = Arc::new(Upstairs::default());
    let gateway = Gateway {
        tokens,
        accounts: Arc::new(Tightness { inner: Fake { private_mode: false, cloud: Sharing::Share, lane: None }, cloud_is_tight }),
        upstream: upstream.clone(),
        log: Arc::new(CallLog::in_memory()),
        template_refused: Default::default(),
        params_refused: Default::default(),
    };
    Rig { gateway, upstream, companion, pi }
}

/// A chat call to `cloud`, carrying `mark` as the asked header when there is one.
fn chat(token: &str, mark: Option<&str>) -> Request {
    let mut req = request("POST", "/v1/chat/completions", Some(token), json!({"model": "cloud/gpt-oss-120b", "messages": [{"role": "user", "content": "hi"}]}));
    if let Some(mark) = mark {
        req.headers.push((yantrik_ml::asked::HEADER.to_string(), mark.to_string()));
    }
    req
}

fn row(r: &Rig) -> Call {
    r.gateway.log.recent().pop().unwrap()
}

fn sent(r: &Rig) -> usize {
    *r.upstream.sent.lock().unwrap()
}

#[test]
fn the_companions_own_call_to_a_tight_account_is_refused_and_nothing_is_sent() {
    let r = rig(true);
    let (status, body) = answer(&r.gateway, &chat(&r.companion, Some("self")));
    assert_eq!(status, 429, "{body}");
    let said: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(said["error"]["code"], "kept_for_asked");
    let message = said["error"]["message"].as_str().unwrap();
    assert!(message.contains("allows few requests") && message.contains("kept for what you ask") && message.contains("not sent"), "{message}");
    assert_eq!(sent(&r), 0, "the provider was never called");
    let row = row(&r);
    assert_eq!((row.status, row.outcome.as_str(), row.asked.as_deref(), row.harness.as_str()), (429, "kept_for_asked", Some("self"), "companion"));
    assert_eq!(row.account, "cloud", "the row still says which account it was kept from");
}

#[test]
fn the_refusal_is_one_the_client_reads_as_kept_for_asked() {
    let r = rig(true);
    let (status, body) = answer(&r.gateway, &chat(&r.companion, Some("self")));
    let kept = yantrik_ml::asked::kept(status, &body).expect("yantrik_ml reads the gateway's own refusal");
    assert!(kept.said.contains("kept for what you ask"), "{}", kept.said);
}

#[test]
fn the_same_call_without_the_mark_is_sent() {
    let r = rig(true);
    let (status, body) = answer(&r.gateway, &chat(&r.companion, None));
    assert_eq!(status, 200, "{body}");
    assert_eq!(sent(&r), 1);
    let row = row(&r);
    assert_eq!((row.status, row.outcome.as_str(), row.asked.clone()), (200, "", None));
    assert!(!serde_json::to_string(&row).unwrap().contains("asked"), "a call somebody asked for leaves the word out of its row");
}

#[test]
fn the_same_marked_call_on_an_account_that_is_not_tight_is_sent_and_says_so_in_its_row() {
    let r = rig(false);
    let (status, body) = answer(&r.gateway, &chat(&r.companion, Some("self")));
    assert_eq!(status, 200, "{body}");
    assert_eq!(sent(&r), 1);
    assert_eq!(row(&r).asked.as_deref(), Some("self"));
    assert!(serde_json::to_string(&row(&r)).unwrap().contains("\"asked\":\"self\""));
}

#[test]
fn another_harness_sending_the_header_changes_nothing_for_itself() {
    let r = rig(true);
    let (status, body) = answer(&r.gateway, &chat(&r.pi, Some("self")));
    assert_eq!(status, 200, "{body}");
    assert_eq!(sent(&r), 1, "the rule reads the grant's harness, not only the header");
    assert_eq!(row(&r).asked.as_deref(), Some("self"), "the row still records the mark that came");
}

#[test]
fn only_the_one_value_is_a_mark() {
    let r = rig(true);
    for value in ["asked", "", "true", "selfish"] {
        let (status, body) = answer(&r.gateway, &chat(&r.companion, Some(value)));
        assert_eq!(status, 200, "{value:?}: {body}");
        assert_eq!(row(&r).asked, None, "{value:?}");
    }
    assert_eq!(sent(&r), 4);
}
