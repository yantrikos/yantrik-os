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

use super::port::{ran_out_of_time, MailPort, ServiceMail, LOCAL_BUDGET, NETWORK_BUDGET, TIMED_OUT};

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
fn one_slow_call_does_not_make_the_next_one_fail_as_unreachable() {
    // Request 0 is slow and times out, which trips the transport's breaker for the address;
    // request 1 is quick and must get through.
    let service = Slow::start(vec![Duration::from_secs(3), Duration::from_millis(10)]);
    let mail = service.mail(Duration::from_secs(1), Duration::from_secs(1));

    assert_eq!(mail.list_messages("a", "INBOX", 1, 20).unwrap_err().code, TIMED_OUT);
    let next = mail.list_messages("a", "INBOX", 1, 20);

    assert!(next.is_ok(), "{:?}", next.err().map(|e| e.message));
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

#[test]
fn the_operating_systems_words_for_a_timeout_are_recognised() {
    for m in [
        "Read error (/run/user/1000/yantrik/email.sock): Resource temporarily unavailable (os error 11)",
        "connection timed out",
        "Read error: Connection timed out (os error 110)",
    ] {
        assert!(ran_out_of_time(m), "{m}");
    }
    assert!(!ran_out_of_time("no such folder"));
    assert!(!ran_out_of_time("Mail is waiting for the vault. Unlock it and mail will check again."));
}
