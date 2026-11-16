//! A call sent a second time (the template-switch or parameter-drop retry) is routed again by the
//! same function as the first send, so what changed during the first send stops the resend.

use std::io::Cursor;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use crate::log::CallLog;
use crate::policy::{Refusal, Sharing, Target};
use crate::server::{Accounts, Gateway, ModelInfo};
use crate::tests::{answer, request, Fake, TEMPERATURE_DEPRECATED};
use crate::tokens::{mint, Grant, Tokens};
use crate::upstream::{Reply, Unreachable, Upstream};

/// What changes after the first send.
#[derive(Clone, Copy)]
enum Change {
    TierOff,
    PrivateMode,
    MovedTo(&'static str),
}

/// Answers like `Fake` for the first route, then `change` has happened.
struct ChangesAfterFirst {
    inner: Fake,
    asks: AtomicUsize,
    change: Change,
}

impl Accounts for ChangesAfterFirst {
    fn models(&self) -> Vec<ModelInfo> {
        self.inner.models()
    }
    fn target(&self, account: &str, model: &str) -> Result<Target, Refusal> {
        let later = self.asks.fetch_add(1, Ordering::SeqCst) >= 1;
        match (later, self.change) {
            (true, Change::TierOff) => Err(Refusal::new(403, "tier_off", "It isn't offered any more. Nothing was sent to it.")),
            (true, Change::MovedTo(other)) => self.inner.target(other, model),
            _ => self.inner.target(account, model),
        }
    }
    fn private_mode(&self) -> bool {
        matches!(self.change, Change::PrivateMode) && self.asks.load(Ordering::SeqCst) > 1
    }
}

/// Answers 400 to anything naming a parameter it dislikes, so a retry is always due.
#[derive(Default)]
struct RefusesBoth {
    sent: Mutex<Vec<Value>>,
}

impl Upstream for RefusesBoth {
    fn chat(&self, _base_url: &str, _key: Option<&str>, body: &[u8]) -> Result<Reply, Unreachable> {
        let v: Value = serde_json::from_slice(body).unwrap();
        let text = if v.get("chat_template_kwargs").is_some() {
            json!({"error": {"message": "Unrecognized request argument supplied: chat_template_kwargs"}})
        } else {
            serde_json::from_str(TEMPERATURE_DEPRECATED).unwrap()
        };
        self.sent.lock().unwrap().push(v);
        Ok(Reply { status: 400, content_type: "application/json".into(), body: Box::new(Cursor::new(text.to_string().into_bytes())) })
    }
}

#[test]
fn what_changed_during_the_first_send_stops_the_retry() {
    let cases = [
        (Change::TierOff, 403, "tier_off"),
        (Change::PrivateMode, 403, "private_mode"),
        (Change::MovedTo("elsewhere"), 503, "route_changed"),
    ];
    for (change, want_status, want_code) in cases {
        for (model, what) in [("typed/qwen3.5:27b", "the template-switch retry"), ("cloud/gpt-oss-120b", "the parameter-drop retry")] {
            let tokens = Arc::new(Tokens::in_memory());
            let pi = mint().unwrap();
            tokens.register(&pi, Grant { harness: "pi".into(), private_context: false }).unwrap();
            let upstream = Arc::new(RefusesBoth::default());
            let g = Gateway {
                tokens,
                accounts: Arc::new(ChangesAfterFirst {
                    inner: Fake { private_mode: false, cloud: Sharing::Share, lane: None },
                    asks: AtomicUsize::new(0),
                    change,
                }),
                upstream: upstream.clone(),
                log: Arc::new(CallLog::in_memory()),
                template_refused: Default::default(),
                params_refused: Default::default(),
            };
            let ask = json!({"model": model, "effort": "easy", "temperature": 0.2, "messages": [{"role": "user", "content": "hi"}]});
            let (status, body) = answer(&g, &request("POST", "/v1/chat/completions", Some(&pi), ask));
            assert_eq!(status, want_status, "{what}: {body}");
            assert!(body.contains(want_code), "{what}: {body}");
            assert_eq!(upstream.sent.lock().unwrap().len(), 1, "{what}: only the first send left");
        }
    }
}
