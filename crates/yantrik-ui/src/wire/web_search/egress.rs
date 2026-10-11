//! What the egress proxy means for the person's search service, and the one rule that lets minds
//! reach it.
//!
//! Minds go out through `yantrik-egress` (`HTTPS_PROXY` 127.0.0.1:7450). A SearXNG the policy does
//! not let through is refused, and a research run comes back empty with nothing on screen to say
//! why. That is not only enforce mode: the local network needs a `lan` rule in every mode, and this
//! machine needs one for 127.0.0.1 or ::1 exactly. So after a save, Settings asks the proxy's
//! control socket for its policy, decides as the proxy does, and when nothing lets minds reach the
//! address, offers "Allow minds to reach this search service".
//!
//! The rule is added only by that press, never by a save, an agent or a start-up: [`check`] only
//! ever sends `status`, and [`allow`] reads the policy again and sends one `allow` with exactly
//! [`rule_for`]'s rule, or nothing when it is no longer missing.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, Ipv6Addr};
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
    /// The proxy only watches (audit) and this address is on the internet: let through now.
    Watching,
    /// The proxy's guarded mode opens the internet without a rule.
    Open,
    /// A rule already lets minds reach it.
    Covered,
    /// On this machine, but not at 127.0.0.1 or ::1, the only loopback addresses a rule opens.
    OtherLoopback,
    /// Nothing lets minds reach it: this is the rule that would.
    Missing(Rule),
}

/// The loopback address a rule opens for `url`: ::1 for an IPv6 loopback, else 127.0.0.1 (what
/// `localhost` reaches). `None` for any other loopback address, which no rule opens.
fn loopback_ip(url: &SearxUrl) -> Option<IpAddr> {
    match url.host.parse::<IpAddr>() {
        Err(_) => Some(IpAddr::from([127, 0, 0, 1])),
        Ok(ip) => {
            let ip = match ip {
                IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(IpAddr::V6(v6)),
                v4 => v4,
            };
            (ip == IpAddr::from([127, 0, 0, 1]) || ip == IpAddr::from(Ipv6Addr::LOCALHOST)).then_some(ip)
        }
    }
}

/// The one rule for `url`: its host, its port, plain http only if the address is http, the local
/// network only if the address is on it. On this machine, the rule names 127.0.0.1 or ::1 and says
/// `lan`: that is the only rule that opens loopback to the mind account (the proxy's
/// `direct::entries`), and a name such as `localhost` never does.
pub fn rule_for(url: &SearxUrl) -> Rule {
    let (host, lan) = match url.place {
        Place::Loopback => (loopback_ip(url).unwrap_or(IpAddr::from([127, 0, 0, 1])).to_string(), true),
        Place::Lan => (url.host.clone(), true),
        Place::Internet => (url.host.clone(), false),
    };
    Rule {
        host,
        ports: vec![url.port],
        http: url.http,
        lan,
        why: "Web search: the person's SearXNG (Settings → Network → Web search)".to_string(),
        seeded: false,
    }
}

/// What `policy` means for `url`, decided the way the proxy decides it: loopback by the rules that
/// open it in the kernel's table, in every mode; anything else by the proxy's own `decide`, so
/// audit does not open the local network and guarded opens the internet without a rule.
pub fn need(policy: &Policy, url: &SearxUrl) -> Need {
    let place = match url.place {
        Place::Loopback => {
            let Some(ip) = loopback_ip(url) else { return Need::OtherLoopback };
            let opened = policy.rules.iter().any(|r| {
                r.lan && r.ports.contains(&url.port) && r.host.parse::<IpAddr>().ok().map(|h| match h {
                    IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(IpAddr::V6(v6)),
                    v4 => v4,
                }) == Some(ip)
            });
            return if opened { Need::Covered } else { Need::Missing(rule_for(url)) };
        }
        Place::Lan => policy::Place::Lan,
        Place::Internet => policy::Place::Internet,
    };
    let covered = policy
        .rules
        .iter()
        .any(|r| r.matches(&url.host, url.port) && (!url.http || r.http) && (place != policy::Place::Lan || r.lan));
    match policy.decide(&url.host, url.port, url.http, place, false) {
        Verdict::Refuse(_) => Need::Missing(rule_for(url)),
        Verdict::Allow { .. } if covered => Need::Covered,
        Verdict::Allow { audit: true } => Need::Watching,
        Verdict::Allow { audit: false } => Need::Open,
    }
}

/// The sentence for the screen, in plain words.
pub fn line(need: &Need, _url: &SearxUrl) -> String {
    match need {
        Need::NoProxy => String::new(),
        Need::Watching => "Your minds can reach this address for now. If you make where minds connect strict, it will need allowing.".into(),
        Need::Open => "Your minds can reach this address: the internet is open to them in the current mode.".into(),
        Need::Covered => "Your minds are allowed to reach this address.".into(),
        Need::OtherLoopback => "Your minds can reach this machine only at 127.0.0.1 or [::1]. Use one of those, or localhost.".into(),
        Need::Missing(_) => "Your minds can't reach this address yet, so their searches there fail. The button allows this one address and port.".into(),
    }
}

/// Ask the proxy what `url` needs. Sends `status` and nothing else.
pub fn check(socket: &Path, url: &SearxUrl) -> Need {
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
        assert!(line(&need, &url).starts_with("Your minds can't reach this address yet"));
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
    fn nothing_is_added_while_the_proxy_lets_it_through_or_a_rule_covers_it() {
        let url = yantrik_web_search::check("https://search.example.com").unwrap();
        let (_dir, path, seen, _) = proxy(Policy { mode: Mode::Audit, rules: vec![] });
        assert_eq!(check(&path, &url), Need::Watching);
        assert!(line(&Need::Watching, &url).contains("for now"));
        assert_eq!(allow(&path, &url).unwrap(), Pressed::NotNeeded(Need::Watching));
        assert!(!ops(&seen).contains(&"allow".to_string()));

        let covering = Rule { host: "*.example.com".into(), ports: vec![443], http: false, lan: false, why: "x".into(), seeded: false };
        let (_dir, path, seen, _) = proxy(enforce(vec![covering]));
        assert_eq!(check(&path, &url), Need::Covered);
        assert_eq!(allow(&path, &url).unwrap(), Pressed::NotNeeded(Need::Covered));
        assert!(!ops(&seen).contains(&"allow".to_string()));
    }

    /// Audit lets the internet through while it watches, but never the local network: only a rule
    /// that says `lan` reaches that, in every mode (the proxy's `decide`).
    #[test]
    fn audit_still_needs_a_rule_for_the_local_network() {
        let url = yantrik_web_search::check("http://192.168.4.42:8888").unwrap();
        let audit = Policy { mode: Mode::Audit, rules: vec![] };
        assert_eq!(need(&audit, &url), Need::Missing(rule_for(&url)));
        let guarded = Policy { mode: Mode::Guarded, rules: vec![] };
        assert_eq!(need(&guarded, &url), Need::Missing(rule_for(&url)));
    }

    /// Guarded opens the internet without a rule: the screen must not claim one exists.
    #[test]
    fn guarded_says_the_internet_is_open_not_that_a_rule_allows_it() {
        let url = yantrik_web_search::check("https://search.example.com").unwrap();
        let guarded = Policy { mode: Mode::Guarded, rules: vec![] };
        assert_eq!(need(&guarded, &url), Need::Open);
        let said = line(&Need::Open, &url);
        assert!(said.contains("open") && !said.contains("rule"), "{said}");
    }

    #[test]
    fn a_rule_for_https_only_does_not_cover_plain_http() {
        let url = yantrik_web_search::check("http://10.0.0.5:8080").unwrap();
        let tunnel_only = Rule { host: "10.0.0.5".into(), ports: vec![8080], http: false, lan: true, why: "x".into(), seeded: false };
        assert!(matches!(need(&enforce(vec![tunnel_only]), &url), Need::Missing(_)));
    }

    /// The mind account reaches this machine only through a `lan` rule for 127.0.0.1 or ::1 exactly
    /// (the proxy's `direct::entries`), in every mode; `localhost` never makes one. So a SearXNG on
    /// localhost is offered that rule, and a check still only reads.
    #[test]
    fn this_machine_needs_its_loopback_rule_in_every_mode() {
        let url = yantrik_web_search::check("http://localhost:8888").unwrap();
        let rule = rule_for(&url);
        assert_eq!((rule.host.as_str(), rule.ports.as_slice(), rule.http, rule.lan), ("127.0.0.1", &[8888u16][..], true, true));
        for mode in Mode::ALL {
            assert_eq!(need(&Policy { mode, rules: vec![] }, &url), Need::Missing(rule.clone()), "{mode:?}");
        }
        let (_dir, path, seen, state) = proxy(Policy { mode: Mode::Audit, rules: vec![] });
        assert_eq!(check(&path, &url), Need::Missing(rule.clone()));
        assert_eq!(ops(&seen), vec!["status"]);
        assert_eq!(allow(&path, &url).unwrap(), Pressed::Added);
        assert_eq!(state.lock().unwrap().rules[0].host, "127.0.0.1");
        assert_eq!(check(&path, &url), Need::Covered, "the rule it added opens it");

        let named = Rule { host: "localhost".into(), ports: vec![8888], http: true, lan: true, why: "x".into(), seeded: false };
        assert!(matches!(need(&enforce(vec![named]), &url), Need::Missing(_)), "a name opens nothing");

        let v6 = yantrik_web_search::check("http://[::1]:8888").unwrap();
        assert_eq!(rule_for(&v6).host, "::1");
        let other = yantrik_web_search::check("http://127.0.0.2:8888").unwrap();
        assert_eq!(need(&enforce(vec![]), &other), Need::OtherLoopback);
    }

    #[test]
    fn no_proxy_says_nothing_and_adds_nothing() {
        let lan = yantrik_web_search::check("http://192.168.1.2:8888").unwrap();
        assert_eq!(check(Path::new("/nonexistent/control"), &lan), Need::NoProxy);
        assert_eq!(line(&Need::NoProxy, &lan), "");
        assert!(allow(Path::new("/nonexistent/control"), &lan).is_err());
    }
}
