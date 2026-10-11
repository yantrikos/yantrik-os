//! The pool as a model, against stand-in providers on this machine: a refusal is read from the
//! provider's own status and headers, the call moves on, and the answer comes from the next one.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use super::backend::PoolBackend;
use super::keys::Keys;
use super::select::Need;
use super::{ledger, Pool, Settings};
use crate::traits::LLMBackend;
use crate::types::{ChatMessage, GenerationConfig};

/// A stand-in provider: answers every request with `status`, `headers` and `body`, and counts
/// how many it got.
fn provider(status: u16, headers: &'static str, body: &'static str) -> (String, Arc<Mutex<u32>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let hits = Arc::new(Mutex::new(0));
    let counted = hits.clone();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut conn) = conn else { continue };
            let _ = conn.set_read_timeout(Some(std::time::Duration::from_secs(5)));
            let mut got = Vec::new();
            let mut buf = [0u8; 8192];
            // Read the head, then as much body as it says it has.
            loop {
                let Ok(n) = conn.read(&mut buf) else { break };
                if n == 0 {
                    break;
                }
                got.extend_from_slice(&buf[..n]);
                if let Some(end) = got.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&got[..end]).to_ascii_lowercase();
                    let len = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(0);
                    if got.len() >= end + 4 + len {
                        break;
                    }
                }
            }
            *counted.lock().unwrap() += 1;
            let reply = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = conn.write_all(reply.as_bytes());
        }
    });
    (url, hits)
}

const ANSWER: &str = r#"{"choices":[{"message":{"content":"from ovh"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":2}}"#;
const REFUSAL: &str = r#"{"error":{"message":"rate limited"}}"#;

#[test]
fn a_refusal_moves_the_call_to_the_next_provider_and_the_pool_remembers_it() {
    let (groq, groq_hits) = provider(429, "retry-after: 30\r\nx-ratelimit-remaining-requests: 0\r\n", REFUSAL);
    let (ovh, ovh_hits) = provider(200, "", ANSWER);
    let ledger = ledger::Ledger::open(&std::env::temp_dir().join(format!("pool-backend-{}.json", std::process::id())));
    let settings = Settings { enabled: vec!["groq".into(), "ovh".into()], keyed: vec!["groq".into()], openrouter_paid_credit: false };
    let pool = Arc::new(Mutex::new(Pool::new(settings, ledger)));
    let keys = Keys::parse("groq=gsk_test\n");
    let backend = PoolBackend::new(pool.clone(), keys, Need { coding: true, ..Need::default() })
        .with_address("groq", &groq)
        .with_address("ovh", &ovh);

    let reply = backend.chat(&[ChatMessage::user("hello")], &GenerationConfig::default(), None).unwrap();
    assert_eq!(reply.text, "from ovh");
    assert!(*groq_hits.lock().unwrap() >= 1, "Groq was tried first");
    assert_eq!(*ovh_hits.lock().unwrap(), 1);

    // Groq's models that refused rest for the 30 s they asked; the next call goes straight to OVH.
    let before = *groq_hits.lock().unwrap();
    let again = backend.chat(&[ChatMessage::user("again")], &GenerationConfig::default(), None).unwrap();
    assert_eq!(again.text, "from ovh");
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    let groq_state = pool.lock().unwrap().status(now).into_iter().find(|s| s.id == "groq").unwrap();
    assert!(*groq_hits.lock().unwrap() - before <= 1, "at most Groq's one untried model is asked again");
    assert!(groq_state.used_today >= 1, "the refusals are in the ledger: {groq_state:?}");
}

#[test]
fn when_every_provider_refuses_the_caller_is_told_plainly() {
    let (groq, _) = provider(429, "retry-after: 120\r\n", REFUSAL);
    let ledger = ledger::Ledger::open(&std::env::temp_dir().join(format!("pool-backend-all-{}.json", std::process::id())));
    let settings = Settings { enabled: vec!["groq".into()], keyed: vec!["groq".into()], openrouter_paid_credit: false };
    let pool = Arc::new(Mutex::new(Pool::new(settings, ledger)));
    let backend = PoolBackend::new(pool, Keys::parse("groq=gsk_test\n"), Need::default()).with_address("groq", &groq);
    let err = backend.chat(&[ChatMessage::user("hi")], &GenerationConfig::default(), None).unwrap_err();
    let text = format!("{err:#}");
    assert!(text.starts_with("free pool:"), "{text}");
    assert!(!text.contains("gsk_test"), "a key never reaches an error: {text}");
}

#[test]
fn keys_print_which_providers_have_one_never_the_key() {
    let keys = Keys::parse("groq=gsk_secret\ncloudflare=cf_secret\n# a comment\nbad line\n");
    let shown = format!("{keys:?}");
    assert!(shown.contains("groq") && !shown.contains("gsk_secret") && !shown.contains("cf_secret"), "{shown}");
    assert_eq!(keys.keyed(), ["groq"], "Cloudflare without its account id is not usable");
    let with_account = Keys::parse("cloudflare=cf\ncloudflare_account=abc123\n");
    assert_eq!(with_account.keyed(), ["cloudflare"]);
    let tier = super::tiers::tier("cloudflare").unwrap();
    assert_eq!(with_account.base_url(tier), "https://api.cloudflare.com/client/v4/accounts/abc123/ai/v1");
}
