//! Where the mind went: every destination, counted.
//!
//! What Settings shows in the audit week ("Where the Mind connects"), and what a refusal in
//! guarded or enforce turns into: a destination with refusals and no rule is a proposal, one per
//! host and port however many times it was tried. A refusal no rule could ever answer is never a
//! proposal ([`Outcome::Never`]: a name that did not resolve, a sinkhole's 0.0.0.0, loopback, this
//! machine), so in guarded, where nothing public is refused, only the home network and the private
//! ranges are ever proposals. Kept to a fixed number of destinations — the least
//! recently seen goes first — so a mind trying a million names cannot grow it without end.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// The most destinations kept.
pub const MOST: usize = 4096;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Seen {
    pub host: String,
    pub port: u16,
    /// Let through by a rule in force.
    pub allowed: u64,
    /// Let through because the policy, or the rule, was only watching.
    pub audited: u64,
    pub refused: u64,
    /// Of the refusals, those no rule could answer ([`Outcome::Never`]).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub never: u64,
    /// Unix seconds.
    pub first: u64,
    pub last: u64,
    /// It resolved to the local network at least once; it was plain http at least once.
    pub lan: bool,
    pub http: bool,
    /// The last refusal's sentence, for the card.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub why: String,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Allowed,
    Audited,
    Refused,
    /// Refused, and no rule could change that: the name did not resolve, or it leads only to an
    /// address that is never a destination. Counted as refused, never a proposal.
    Never,
}

impl Outcome {
    /// What `verdict` for a destination that resolved to `place` is counted as.
    pub fn of(verdict: &crate::policy::Verdict, place: crate::policy::Place, private: bool) -> Outcome {
        use crate::policy::{Place, Verdict};
        match verdict {
            Verdict::Allow { audit: true } => Outcome::Audited,
            Verdict::Allow { audit: false } => Outcome::Allowed,
            // Private mode's refusal is the person's to lift, not the address's.
            Verdict::Refuse(_) if place == Place::Forbidden && !private => Outcome::Never,
            Verdict::Refuse(_) => Outcome::Refused,
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
pub struct Ledger {
    seen: HashMap<String, Seen>,
    #[serde(skip)]
    pub changed: bool,
}

fn key(host: &str, port: u16) -> String {
    format!("{host}:{port}")
}

impl Ledger {
    pub fn record(&mut self, host: &str, port: u16, outcome: Outcome, lan: bool, http: bool, why: &str, now: u64) {
        let k = key(host, port);
        if !self.seen.contains_key(&k) && self.seen.len() >= MOST {
            if let Some(oldest) = self.seen.iter().min_by_key(|(_, s)| s.last).map(|(k, _)| k.clone()) {
                self.seen.remove(&oldest);
            }
        }
        let s = self.seen.entry(k).or_insert_with(|| Seen { host: host.into(), port, first: now, ..Seen::default() });
        match outcome {
            Outcome::Allowed => s.allowed += 1,
            Outcome::Audited => s.audited += 1,
            Outcome::Refused | Outcome::Never => {
                s.refused += 1;
                s.never += u64::from(outcome == Outcome::Never);
                s.why = why.chars().take(300).collect();
            }
        }
        s.last = now;
        s.lan |= lan;
        s.http |= http;
        self.changed = true;
    }

    /// Every destination, the most recent first.
    pub fn list(&self) -> Vec<Seen> {
        let mut v: Vec<Seen> = self.seen.values().cloned().collect();
        v.sort_by(|a, b| b.last.cmp(&a.last).then_with(|| a.host.cmp(&b.host)));
        v
    }

    /// What the person has not answered: destinations refused in a way a rule could answer, with
    /// no rule for them now.
    pub fn proposals(&self, policy: &crate::policy::Policy) -> Vec<Seen> {
        self.list()
            .into_iter()
            .filter(|s| s.refused > s.never && !policy.rules.iter().any(|r| r.matches(&s.host, s.port)))
            .collect()
    }

    /// Forget a destination: the person answered No and does not want to be asked again soon.
    pub fn forget(&mut self, host: &str, port: u16) {
        if self.seen.remove(&key(host, port)).is_some() {
            self.changed = true;
        }
    }

    pub fn load(path: &std::path::Path) -> Ledger {
        std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::{Mode, Policy, Rule};

    #[test]
    fn each_destination_is_counted_once_with_its_outcomes() {
        let mut l = Ledger::default();
        l.record("api.x.ai", 443, Outcome::Audited, false, false, "", 10);
        l.record("api.x.ai", 443, Outcome::Allowed, false, false, "", 20);
        l.record("evil.example", 443, Outcome::Refused, false, false, "not allowed", 30);
        l.record("evil.example", 443, Outcome::Refused, false, false, "not allowed", 40);
        let v = l.list();
        assert_eq!(v.len(), 2);
        assert_eq!((v[0].host.as_str(), v[0].refused, v[0].first, v[0].last), ("evil.example", 2, 30, 40));
        assert_eq!((v[1].audited, v[1].allowed), (1, 1));
    }

    #[test]
    fn a_refusal_is_one_proposal_until_a_rule_answers_it() {
        let mut l = Ledger::default();
        for t in 0..50 {
            l.record("new.example", 443, Outcome::Refused, false, false, "not allowed", t);
        }
        let mut p = Policy { mode: Mode::Enforce, rules: vec![] };
        assert_eq!(l.proposals(&p).len(), 1, "fifty tries, one card");
        p.allow(Rule { host: "new.example".into(), ports: vec![443], http: false, lan: false, why: "asked".into(), seeded: false }).unwrap();
        assert!(l.proposals(&p).is_empty());
    }

    #[test]
    fn in_guarded_a_public_host_is_never_a_proposal_and_a_lan_one_is() {
        use crate::policy::{place_of, Place, Verdict};
        let p = Policy { mode: Mode::Guarded, rules: vec![] };
        let mut l = Ledger::default();
        // As the proxy counts them: a name that did not resolve leads nowhere (`Place::Forbidden`
        // for the count); otherwise the place its address is.
        let dests: [(&str, u16, Option<&str>); 6] = [
            ("example.com", 443, Some("93.184.215.14")),
            ("nas.lan", 443, Some("192.168.4.20")),
            ("nonexistent.invalid", 443, None),
            ("sink.example.com", 443, Some("0.0.0.0")),
            ("127.0.0.1", 7450, Some("127.0.0.1")),
            ("[2a02:8070:abcd:1::20]", 445, None),
        ];
        for (host, port, ip) in dests {
            let place = match (host, ip) {
                // A home device on the ISP's global prefix: the local network (`crate::local::place`).
                ("[2a02:8070:abcd:1::20]", _) => Place::Lan,
                (_, Some(ip)) => place_of(ip.parse().unwrap()),
                (_, None) => Place::Forbidden,
            };
            let verdict = p.decide(host, port, false, place, false);
            l.record(host, port, Outcome::of(&verdict, place, false), place == Place::Lan, false, "", 1);
        }
        let mut hosts: Vec<String> = l.proposals(&p).into_iter().map(|s| s.host).collect();
        hosts.sort();
        assert_eq!(hosts, ["[2a02:8070:abcd:1::20]", "nas.lan"], "nothing public, unresolvable, sinkholed or forbidden is a proposal");
        assert_eq!(l.list().iter().find(|s| s.host == "example.com").map(|s| (s.allowed, s.audited)), Some((1, 0)), "recorded, not watched");
        for host in ["nonexistent.invalid", "sink.example.com", "127.0.0.1"] {
            let s = l.list().into_iter().find(|s| s.host == host).unwrap();
            assert_eq!((s.refused, s.never), (1, 1), "{host}: still counted as refused");
        }
        // Private mode's refusals are the person's, and stay proposals as before.
        assert_eq!(Outcome::of(&Verdict::Refuse("private".into()), Place::Forbidden, true), Outcome::Refused);
    }

    #[test]
    fn a_refusal_no_rule_could_answer_is_never_a_proposal() {
        let p = Policy { mode: Mode::Guarded, rules: vec![] };
        let mut l = Ledger::default();
        l.record("nonexistent.invalid", 443, Outcome::Never, false, false, "did not resolve", 1);
        l.record("sink.example.com", 443, Outcome::Never, false, false, "never a destination", 2);
        l.record("127.0.0.1", 7450, Outcome::Never, false, false, "this machine", 3);
        assert!(l.proposals(&p).is_empty());
        assert_eq!(l.list().iter().map(|s| (s.refused, s.never)).collect::<Vec<_>>(), [(1, 1); 3], "still counted");
        // The same destination refused once for a reason a rule could answer is asked about.
        l.record("sink.example.com", 443, Outcome::Refused, true, false, "local network", 4);
        assert_eq!(l.proposals(&p).iter().map(|s| s.host.as_str()).collect::<Vec<_>>(), ["sink.example.com"]);
        // A ledger written before `never` existed reads, every refusal a proposal as before.
        let old: Ledger = serde_json::from_str(r#"{"seen":{"a.example:443":{"host":"a.example","port":443,"allowed":0,"audited":0,"refused":2,"first":1,"last":1,"lan":false,"http":false}}}"#).unwrap();
        assert_eq!(old.proposals(&p).len(), 1);
    }

    #[test]
    fn it_never_grows_past_its_size_and_the_least_recent_goes_first() {
        let mut l = Ledger::default();
        for i in 0..(MOST as u64 + 10) {
            l.record(&format!("h{i}.example"), 443, Outcome::Audited, false, false, "", i);
        }
        assert_eq!(l.list().len(), MOST);
        assert!(l.list().iter().all(|s| s.host != "h0.example"));
        assert!(l.list().iter().any(|s| s.host == format!("h{}.example", MOST + 9)));
    }
}
