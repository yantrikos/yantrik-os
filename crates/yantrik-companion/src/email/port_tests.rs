//! The real client against a fake service on a real socket (#1022, 520 gate): a service that takes
//! seconds to answer a list must still be answered, and one that takes too long must be reported
//! as slow, in a sentence a mind can act on, without poisoning the next call.

#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use yantrik_ipc_contracts::email::ComposeRequest;
use yantrik_ipc_transport::SyncRpcClient;

use super::port::{MailPort, ServiceMail, BUSY, LOCAL_BUDGET, NETWORK_BUDGET, TIMED_OUT};

/// A mail service that answers every request after the delay it is given for that request.
struct Slow {
    path: PathBuf,
    asked: Arc<Mutex<Vec<String>>>,
}

impl Slow {
    /// `delays[n]` is how long request `n` takes to be answered; the last one repeats.
    fn start(delays: Vec<Duration>) -> Self {
        static N: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!("yos-1022-sock-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        let asked = Arc::new(Mutex::new(Vec::new()));
        let log = asked.clone();
        std::thread::spawn(move || {
            let mut n = 0usize;
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                let log = log.clone();
                let delay = delays[n.min(delays.len() - 1)];
                n += 1;
                std::thread::spawn(move || {
                    let mut line = String::new();
                    if BufReader::new(stream.try_clone().unwrap()).read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    let request: serde_json::Value = serde_json::from_str(&line).unwrap();
                    log.lock().unwrap().push(request["method"].as_str().unwrap_or("").to_string());
                    std::thread::sleep(delay);
                    let result = match request["method"].as_str() {
                        Some("email.accounts") => serde_json::json!({
                            "accounts": [{ "id": "a", "email": "a@example.com", "imap_server": "i", "imap_port": 993,
                                           "smtp_server": "s", "smtp_port": 587 }],
                            "config_path": "/x", "secrets_are_plaintext": false
                        }),
                        _ => serde_json::json!([]),
                    };
                    let reply = serde_json::json!({ "jsonrpc": "2.0", "id": request["id"], "result": result });
                    let mut stream = stream;
                    let _ = writeln!(stream, "{reply}");
                });
            }
        });
        Slow { path, asked }
    }

    fn mail(&self, network: Duration, local: Duration) -> ServiceMail {
        ServiceMail::at(self.path.to_str().unwrap(), network, local)
    }
}

impl Drop for Slow {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[test]
fn a_list_that_takes_three_seconds_is_answered_not_failed() {
    // The transport's own default is two seconds; this is what the 520 gate hit, every time.
    let service = Slow::start(vec![Duration::from_secs(3)]);
    let mail = service.mail(NETWORK_BUDGET, LOCAL_BUDGET);

    let started = Instant::now();
    let listed = mail.list_messages("a", "INBOX", 1, 20).expect("a slow answer is still an answer");

    assert!(listed.is_empty());
    assert!(started.elapsed() >= Duration::from_secs(3), "the fake did not take its time");
    assert_eq!(service.asked.lock().unwrap().as_slice(), ["email.list_messages"]);
}

#[test]
fn the_account_list_is_a_local_call_with_its_own_budget() {
    let service = Slow::start(vec![Duration::from_millis(50)]);
    let accounts = service.mail(NETWORK_BUDGET, LOCAL_BUDGET).accounts().unwrap();
    assert_eq!(accounts[0].email, "a@example.com");
}

#[test]
fn a_service_that_is_too_slow_is_reported_as_slow_in_words_and_never_as_an_os_error() {
    let service = Slow::start(vec![Duration::from_secs(3)]);
    let mail = service.mail(Duration::from_secs(1), Duration::from_secs(1));

    let read = mail.list_messages("a", "INBOX", 1, 20).unwrap_err();

    assert_eq!(read.code, TIMED_OUT);
    assert_eq!(read.message, "Mail is taking longer than usual to answer. Nothing was changed; try again in a moment.");
    assert!(!read.message.contains("os error") && !read.message.contains("temporarily unavailable"), "{}", read.message);
}

#[test]
fn a_change_that_timed_out_does_not_claim_to_have_failed() {
    let service = Slow::start(vec![Duration::from_secs(3)]);
    let mail = service.mail(Duration::from_secs(1), Duration::from_secs(1));

    let sent = mail.delete_message("a", "INBOX", "7").unwrap_err();

    assert_eq!(sent.code, TIMED_OUT);
    assert!(sent.message.contains("can't say whether that went through"), "{}", sent.message);
}

#[test]
fn after_a_timeout_of_its_own_the_next_call_fails_at_once_in_words_and_asks_nobody() {
    // The breaker the timeout tripped is believed: clearing it would make every call against a
    // wedged service wait its full budget again (#1026 review, L3).
    let service = Slow::start(vec![Duration::from_secs(3), Duration::from_millis(10)]);
    let mail = service.mail(Duration::from_secs(1), Duration::from_secs(1));

    assert_eq!(mail.list_messages("a", "INBOX", 1, 20).unwrap_err().code, TIMED_OUT);
    let started = Instant::now();
    let next = mail.list_messages("a", "INBOX", 1, 20).unwrap_err();

    assert!(started.elapsed() < Duration::from_millis(500), "{:?}", started.elapsed());
    assert_eq!(next.code, TIMED_OUT);
    assert!(next.message.contains("Nothing was changed"), "{}", next.message);
    assert!(!next.message.contains("unreachable") && !next.message.contains(".sock"), "{}", next.message);
    assert_eq!(service.asked.lock().unwrap().len(), 1, "the second call reached the service");
}

#[test]
fn a_breaker_tripped_by_another_caller_is_cleared_and_the_call_goes_through() {
    // Another client of the same socket (the Email app, on a ten second budget) times out and
    // trips the process-wide breaker; this port did nothing wrong and must get through.
    let service = Slow::start(vec![Duration::from_secs(2), Duration::from_millis(10)]);
    let other = SyncRpcClient::new(service.path.to_str().unwrap()).with_timeout(Duration::from_millis(300));
    assert!(other.call("email.list_messages", serde_json::json!({})).is_err());

    let mail = service.mail(NETWORK_BUDGET, LOCAL_BUDGET);
    let listed = mail.list_messages("a", "INBOX", 1, 20);

    assert!(listed.is_ok(), "{:?}", listed.err().map(|e| e.message));
}

#[test]
fn a_send_that_timed_out_is_asked_once_and_never_again() {
    let service = Slow::start(vec![Duration::from_secs(3)]);
    let mail = service.mail(Duration::from_secs(1), Duration::from_secs(1));
    let compose = ComposeRequest {
        to: vec!["x@example.com".into()], cc: vec![], bcc: vec![], subject: "s".into(), body: "b".into(),
        reply_to_id: None, signature: None,
    };

    let sent = mail.send("a", &compose).unwrap_err();
    let again = mail.send("a", &compose).unwrap_err();

    assert!(sent.message.contains("can't say whether that went through"), "{}", sent.message);
    assert_eq!(again.code, TIMED_OUT);
    assert_eq!(service.asked.lock().unwrap().as_slice(), ["email.send_message"], "a send went out twice");
}

#[test]
fn a_service_that_is_gone_is_not_reported_as_slow() {
    // A refused connection is not a timeout; the person is not told "slow" about a service that
    // is not there.
    let service = Slow::start(vec![Duration::from_millis(10)]);
    let mail = service.mail(NETWORK_BUDGET, LOCAL_BUDGET);
    drop(service);
    let gone = mail.delete_message("a", "INBOX", "7").unwrap_err();
    assert_ne!(gone.code, TIMED_OUT, "{}", gone.message);
}

#[test]
fn a_runaway_cannot_keep_more_than_the_ceiling_waiting() {
    let service = Slow::start(vec![Duration::from_secs(2)]);
    let mail = service.mail(NETWORK_BUDGET, LOCAL_BUDGET);
    let waiting: Vec<_> = (0..4)
        .map(|_| {
            let mail = mail.clone();
            std::thread::spawn(move || mail.list_messages("a", "INBOX", 1, 20))
        })
        .collect();
    std::thread::sleep(Duration::from_millis(500));

    let over = mail.list_messages("a", "INBOX", 1, 20).unwrap_err();

    assert_eq!(over.code, BUSY);
    assert!(over.message.contains("Nothing was changed"), "{}", over.message);
    for w in waiting {
        assert!(w.join().unwrap().is_ok());
    }
    assert!(mail.list_messages("a", "INBOX", 1, 20).is_ok(), "places did not come back");
}

#[test]
fn the_budgets_are_set_in_one_place_and_are_not_the_transports_two_seconds() {
    assert!(NETWORK_BUDGET >= Duration::from_secs(30));
    assert!(LOCAL_BUDGET > yantrik_ipc_transport::sync_client::DEFAULT_TIMEOUT);
    let source = include_str!("port.rs");
    let production = source.split("#[cfg(test)]").next().unwrap();
    assert!(!production.contains("SyncRpcClient::for_service"), "a client built without a budget");
    assert_eq!(production.matches("with_timeout(").count(), 1, "the budget is applied in one place");
}
