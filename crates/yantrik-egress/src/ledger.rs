//! Where the mind went: every destination, counted.
//!
//! What Settings shows in the audit week ("Where the Mind connects"), and what a refusal in
//! enforce turns into: a destination with refusals and no rule is a proposal, one per host and
//! port however many times it was tried. Kept to a fixed number of destinations — the least
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Allowed,
    Audited,
    Refused,
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
            Outcome::Refused => {
                s.refused += 1;
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

    /// What the person has not answered: destinations refused, with no rule for them now.
    pub fn proposals(&self, policy: &crate::policy::Policy) -> Vec<Seen> {
        self.list()
            .into_iter()
            .filter(|s| s.refused > 0 && !policy.rules.iter().any(|r| r.matches(&s.host, s.port)))
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
        p.allow(Rule { host: "new.example".into(), ports: vec![443], http: false, lan: false, why: "asked".into() }).unwrap();
        assert!(l.proposals(&p).is_empty());
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
