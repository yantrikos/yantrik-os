//! What the egress proxy means for the person's search service, and the one rule that lets minds
//! reach it.
//!
//! Minds go out through `yantrik-egress` (`HTTPS_PROXY` 127.0.0.1:7450). In enforce mode a SearXNG
//! no rule covers is refused, and a research run comes back empty with nothing on screen to say
//! why. So after a save, Settings asks the proxy's control socket for its policy, and when it is
//! enforcing with no rule for this address, offers "Allow minds to reach this search service".
//!
//! The rule is added only by that press, never by a save, an agent or a start-up: [`check`] only
//! ever sends `status`, and [`allow`] reads the policy again and sends one `allow` with exactly
//! [`rule_for`]'s rule, or nothing when it is no longer missing.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};
use yantrik_egress::policy::{self, Mode, Policy, Rule, Verdict};
use yantrik_web_search::{Place, SearxUrl};

/// The proxy's control socket: `EGRESS_CONTROL`, as the proxy itself reads it, or its default.
pub fn socket() -> PathBuf {
    std::env::var_os("EGRESS_CONTROL").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/run/yantrik-egress/control"))
}

/// What the proxy means for one address.
#[derive(Debug, PartialEq, Eq)]
pub enum Need {
    /// No proxy answering on this machine: nothing to say.
    NoProxy,
    /// On this machine. Minds reach it directly (`NO_PROXY`), not through the proxy.
    Loopback,
    /// The proxy only watches: nothing is refused now.
    Audit,
    /// A rule already lets minds reach it.
    Covered,
    /// Enforcing, and nothing lets minds reach it: this is the rule that would.
    Missing(Rule),
}

/// The one rule for `url`: its host, its port, plain http only if the address is http, the local
/// network only if the address is on it.
pub fn rule_for(url: &SearxUrl) -> Rule {
    Rule {
        host: url.host.clone(),
        ports: vec![url.port],
        http: url.http,
        lan: url.place == Place::Lan,
        why: "Web search: the person's SearXNG (Settings → Network → Web search)".to_string(),
    }
}

/// What `policy` means for `url`, decided the way the proxy decides it.
pub fn need(policy: &Policy, url: &SearxUrl) -> Need {
    if url.place == Place::Loopback {
        return Need::Loopback;
    }
    if policy.mode == Mode::Audit {
        return Need::Audit;
    }
    let place = if url.place == Place::Lan { policy::Place::Lan } else { policy::Place::Internet };
    match policy.decide(&url.host, url.port, url.http, place, false) {
        Verdict::Allow { .. } => Need::Covered,
        Verdict::Refuse(_) => Need::Missing(rule_for(url)),
    }
}

/// The sentence for the screen.
pub fn line(need: &Need, url: &SearxUrl) -> String {
    let at = format!("{}:{}", url.host, url.port);
    match need {
        Need::NoProxy => String::new(),
        Need::Loopback => "This address is on this machine: minds reach it directly, not through the egress proxy, so no rule is needed.".into(),
        Need::Audit => format!(
            "The egress proxy is in audit mode, so no rule is needed now. If you switch it to enforce, minds will need a rule for {at}."
        ),
        Need::Covered => format!("A rule already lets minds reach {at}."),
        Need::Missing(rule) => format!(
            "The egress proxy is enforcing and no rule lets minds reach {at}{}{}, so their searches there are refused. The button adds one rule for exactly that.",
            if rule.http { " over plain http" } else { "" },
            if rule.lan { " on your local network" } else { "" },
        ),
    }
}

/// Ask the proxy what `url` needs. Sends `status` and nothing else.
pub fn check(socket: &Path, url: &SearxUrl) -> Need {
    if url.place == Place::Loopback {
        return Need::Loopback;
    }
    match status(socket) {
        Ok(policy) => need(&policy, url),
        Err(_) => Need::NoProxy,
    }
}

/// What a press of the button did.
#[derive(Debug, PartialEq, Eq)]
pub enum Pressed {
    Added,
    /// Read again at the press, nothing was missing any more: nothing was sent.
    NotNeeded(Need),
}

/// The button: read the policy again and, only if the rule is still missing, add exactly it.
pub fn allow(socket: &Path, url: &SearxUrl) -> Result<Pressed, String> {
    match need(&status(socket)?, url) {
        Need::Missing(rule) => {
            ask(socket, &json!({ "op": "allow", "rule": rule }))?;
            Ok(Pressed::Added)
        }
        other => Ok(Pressed::NotNeeded(other)),
    }
}

fn status(socket: &Path) -> Result<Policy, String> {
    let v = ask(socket, &json!({ "op": "status" }))?;
    Ok(Policy {
        mode: serde_json::from_value(v["mode"].clone()).map_err(|_| "the proxy did not say its mode".to_string())?,
        rules: serde_json::from_value(v["rules"].clone()).unwrap_or_default(),
    })
}

/// One request line, one answer line.
fn ask(socket: &Path, request: &Value) -> Result<Value, String> {
    let mut stream = UnixStream::connect(socket).map_err(|e| format!("the egress proxy is not answering ({e})"))?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(3)));
    let mut line = request.to_string();
    line.push('\n');
    stream.write_all(line.as_bytes()).map_err(|e| format!("the egress proxy did not take the request ({e})"))?;
    let mut answer = String::new();
    BufReader::new(stream.take(1024 * 1024)).read_line(&mut answer).map_err(|e| format!("the egress proxy did not answer ({e})"))?;
    let v: Value = serde_json::from_str(&answer).map_err(|_| "the egress proxy's answer was not JSON".to_string())?;
    if v["ok"] != true {
        return Err(v["error"].as_str().unwrap_or("the egress proxy refused").to_string());
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;
    use std::sync::{Arc, Mutex};

    /// A control socket that answers like the proxy's (`control::handle`, by hand): `status` with
    /// `policy`, `allow` by adding the rule. Every request it was sent is recorded.
    fn proxy(policy: Policy) -> (Gone, PathBuf, Arc<Mutex<Vec<Value>>>, Arc<Mutex<Policy>>) {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("yantrik-egress-test-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("control");
        let listener = UnixListener::bind(&path).unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let state = Arc::new(Mutex::new(policy));
        let (s, p) = (seen.clone(), state.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    continue;
                }
                let req: Value = serde_json::from_str(&line).unwrap();
                s.lock().unwrap().push(req.clone());
                let answer = match req["op"].as_str() {
                    Some("status") => {
                        let p = p.lock().unwrap();
                        json!({ "ok": true, "mode": p.mode, "private": false, "rules": p.rules })
                    }
                    Some("allow") => {
                        let rule: Rule = serde_json::from_value(req["rule"].clone()).unwrap();
                        p.lock().unwrap().allow(rule).unwrap();
                        json!({ "ok": true })
                    }
                    _ => json!({ "ok": false, "error": "no" }),
                };
                let mut out = answer.to_string();
                out.push('\n');
                let _ = (&stream).write_all(out.as_bytes());
            }
        });
        (Gone(dir), path, seen, state)
    }

    /// The socket's directory, removed when the test ends.
    struct Gone(PathBuf);
    impl Drop for Gone {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn enforce(rules: Vec<Rule>) -> Policy {
        Policy { mode: Mode::Enforce, rules }
    }

    fn ops(seen: &Arc<Mutex<Vec<Value>>>) -> Vec<String> {
        seen.lock().unwrap().iter().map(|v| v["op"].as_str().unwrap_or_default().to_string()).collect()
    }

    #[test]
    fn checking_only_ever_reads() {
        let url = yantrik_web_search::check("http://192.168.4.42:8888").unwrap();
        let (_dir, path, seen, state) = proxy(enforce(vec![]));
        let need = check(&path, &url);
        assert_eq!(need, Need::Missing(rule_for(&url)));
        assert!(line(&need, &url).contains("192.168.4.42:8888 over plain http on your local network"));
        assert_eq!(ops(&seen), vec!["status"], "a check never adds a rule");
        assert!(state.lock().unwrap().rules.is_empty());
    }

    #[test]
    fn the_press_adds_exactly_the_one_rule() {
        let url = yantrik_web_search::check("http://192.168.4.42:8888/").unwrap();
        let (_dir, path, seen, state) = proxy(enforce(vec![]));
        assert_eq!(allow(&path, &url).unwrap(), Pressed::Added);
        assert_eq!(ops(&seen), vec!["status", "allow"]);
        let rules = state.lock().unwrap().rules.clone();
        assert_eq!(rules.len(), 1);
        assert_eq!(
            (rules[0].host.as_str(), rules[0].ports.as_slice(), rules[0].http, rules[0].lan),
            ("192.168.4.42", &[8888u16][..], true, true)
        );
        // Now covered: a second press sends nothing more.
        assert_eq!(allow(&path, &url).unwrap(), Pressed::NotNeeded(Need::Covered));
        assert_eq!(ops(&seen), vec!["status", "allow", "status"]);
        assert_eq!(state.lock().unwrap().rules.len(), 1);
    }

    #[test]
    fn nothing_is_added_in_audit_or_when_a_rule_covers_it() {
        let url = yantrik_web_search::check("https://search.example.com").unwrap();
        let (_dir, path, seen, _) = proxy(Policy { mode: Mode::Audit, rules: vec![] });
        assert_eq!(check(&path, &url), Need::Audit);
        assert!(line(&Need::Audit, &url).contains("no rule is needed now"));
        assert_eq!(allow(&path, &url).unwrap(), Pressed::NotNeeded(Need::Audit));
        assert!(!ops(&seen).contains(&"allow".to_string()));

        let covering = Rule { host: "*.example.com".into(), ports: vec![443], http: false, lan: false, why: "x".into() };
        let (_dir, path, seen, _) = proxy(enforce(vec![covering]));
        assert_eq!(check(&path, &url), Need::Covered);
        assert_eq!(allow(&path, &url).unwrap(), Pressed::NotNeeded(Need::Covered));
        assert!(!ops(&seen).contains(&"allow".to_string()));
    }

    #[test]
    fn a_rule_for_https_only_does_not_cover_plain_http() {
        let url = yantrik_web_search::check("http://10.0.0.5:8080").unwrap();
        let tunnel_only = Rule { host: "10.0.0.5".into(), ports: vec![8080], http: false, lan: true, why: "x".into() };
        assert!(matches!(need(&enforce(vec![tunnel_only]), &url), Need::Missing(_)));
    }

    #[test]
    fn this_machine_and_no_proxy_ask_nothing() {
        let local = yantrik_web_search::check("http://127.0.0.1:8888").unwrap();
        let (_dir, path, seen, _) = proxy(enforce(vec![]));
        assert_eq!(check(&path, &local), Need::Loopback);
        assert!(seen.lock().unwrap().is_empty());
        let lan = yantrik_web_search::check("http://192.168.1.2:8888").unwrap();
        assert_eq!(check(Path::new("/nonexistent/control"), &lan), Need::NoProxy);
        assert!(allow(Path::new("/nonexistent/control"), &lan).is_err());
    }
}
