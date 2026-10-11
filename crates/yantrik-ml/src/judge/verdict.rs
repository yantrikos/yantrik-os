//! The verdict: the one shape every decision model answers in, and its wire form.

use std::collections::HashMap;

use serde_json::{json, Map, Value};

use super::Answer;

/// Where the judge runs, which is where the state it was asked about went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Locality {
    /// On this machine: nothing left it.
    ThisMachine,
    /// On a machine on the person's network, such as the GPU box serving the household.
    Home,
    /// A service on the internet: the state left the house.
    Cloud,
    /// Nowhere: no model was asked (the judge is off).
    Nowhere,
}

impl Locality {
    pub fn as_str(self) -> &'static str {
        match self {
            Locality::ThisMachine => "this_machine",
            Locality::Home => "home",
            Locality::Cloud => "cloud",
            Locality::Nowhere => "nowhere",
        }
    }

    /// Where an endpoint is, read from its address: loopback is this machine, a private or
    /// link-local address or a `.local`/`.lan` name is home, anything else is the cloud. A name
    /// that cannot be told apart is called cloud: saying the state stayed home when it did not is
    /// the mistake that matters.
    pub fn of_endpoint(endpoint: &str) -> Locality {
        // The authority ends at the first `/`, `?` or `#`, as every URL parser reads it: a host
        // cut at `/` alone took `http://evil.example?@127.0.0.1` for loopback (security review,
        // 29 Sep 2026). Userinfo is what comes before the last `@` inside it.
        let rest = endpoint.trim().split_once("://").map(|(_, r)| r).unwrap_or(endpoint.trim());
        let authority = rest.split(['/', '?', '#', '\\']).next().unwrap_or("");
        let host = authority.rsplit_once('@').map(|(_, h)| h).unwrap_or(authority);
        let host = if let Some(bracketed) = host.strip_prefix('[') {
            bracketed.split(']').next().unwrap_or("")
        } else {
            host.split(':').next().unwrap_or("")
        }
        .trim_end_matches('.')
        .to_ascii_lowercase();
        // Only an address that IS loopback is this machine: `127.attacker.example` is a name.
        if host == "localhost" {
            return Locality::ThisMachine;
        }
        if let Ok(ip) = host.parse::<std::net::Ipv4Addr>() {
            if ip.is_loopback() {
                return Locality::ThisMachine;
            }
            if ip.is_private() || ip.is_link_local() {
                return Locality::Home;
            }
            return Locality::Cloud;
        }
        if let Ok(ip) = host.parse::<std::net::Ipv6Addr>() {
            if ip.is_loopback() {
                return Locality::ThisMachine;
            }
            let first = ip.segments()[0];
            if (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80 {
                return Locality::Home;
            }
            return Locality::Cloud;
        }
        if !host.is_empty()
            && host.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
            && (host.ends_with(".local") || host.ends_with(".lan") || host.ends_with(".home.arpa"))
        {
            return Locality::Home;
        }
        Locality::Cloud
    }
}

/// Who answered.
#[derive(Debug, Clone, PartialEq)]
pub struct JudgeInfo {
    /// How it was asked: `systemone`, `chat_model` or `off`.
    pub adapter: &'static str,
    /// Which provider: `jev`, `kev`, `laya`, `jeff`, `systemone` for any other such server, the
    /// chat backend's name, or `off`.
    pub provider: String,
    /// The model the answers came from.
    pub model: String,
    /// Where it ran.
    pub locality: Locality,
    /// Whether its probabilities are trained to be calibrated (a System One model) or only the
    /// numbers a chat model wrote down, which are not.
    pub calibrated: bool,
}

/// The answers to one set of questions, and who gave them.
#[derive(Debug, Clone, PartialEq)]
pub struct Verdict {
    pub answers: HashMap<String, Answer>,
    pub by: JudgeInfo,
    pub latency_ms: u64,
}

impl Verdict {
    pub fn get(&self, id: &str) -> Option<&Answer> {
        self.answers.get(id)
    }

    /// Whether every answer is an abstention — the judge is off, down, or could not tell.
    pub fn abstained(&self) -> bool {
        self.answers.values().all(Answer::is_abstain)
    }

    /// The wire form (`docs/decisions.md`): the same for every adapter, so a caller in Python or
    /// over a socket reads it without knowing which model answered.
    pub fn to_json(&self) -> Value {
        let mut answers = Map::new();
        let mut ids: Vec<&String> = self.answers.keys().collect();
        ids.sort();
        for id in ids {
            answers.insert(id.clone(), answer_json(&self.answers[id]));
        }
        json!({
            "answers": answers,
            "by": {
                "adapter": self.by.adapter,
                "provider": self.by.provider,
                "model": self.by.model,
                "locality": self.by.locality.as_str(),
                "calibrated": self.by.calibrated,
            },
            "latency_ms": self.latency_ms,
        })
    }
}

fn answer_json(a: &Answer) -> Value {
    match a {
        Answer::Noul(p) => json!({"type": "noul", "yes": p}),
        Answer::Choice { choice, probabilities, confidence } => {
            let mut probs: Vec<(&String, &f64)> = probabilities.iter().collect();
            probs.sort_by(|a, b| a.0.cmp(b.0));
            let probs: Map<String, Value> = probs.into_iter().map(|(k, v)| (k.clone(), json!(v))).collect();
            json!({"type": "choice", "choice": choice, "probabilities": probs, "confidence": confidence})
        }
        Answer::Score { value, distribution } => json!({"type": "score", "value": value, "distribution": distribution}),
        Answer::Abstain { reason } => json!({"type": "abstain", "reason": reason}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn where_an_endpoint_runs_is_read_from_its_address_and_unknown_is_cloud() {
        for (endpoint, want) in [
            ("http://127.0.0.1:8009", Locality::ThisMachine),
            ("http://localhost:8000/v1/systemone", Locality::ThisMachine),
            ("http://[::1]:8009", Locality::ThisMachine),
            ("http://192.168.4.20:8009", Locality::Home),
            ("http://10.0.0.5", Locality::Home),
            ("http://gpu-box.local:8009", Locality::Home),
            ("https://api.typesafe.ai", Locality::Cloud),
            ("https://192.168.4.20.evil.example", Locality::Cloud),
            ("http://user@8.8.8.8:80", Locality::Cloud),
            ("http://127.attacker.example:8009", Locality::Cloud),
            ("http://evil.example?@127.0.0.1:8009", Locality::Cloud),
            ("http://evil.example#@127.0.0.1", Locality::Cloud),
            ("http://evil.example\\@127.0.0.1", Locality::Cloud),
            ("http://localhost.evil.example", Locality::Cloud),
            ("http://LOCALHOST.:8009", Locality::ThisMachine),
            ("http://127.0.0.2:8009", Locality::ThisMachine),
            ("http://[::ffff:127.0.0.1]:8009", Locality::Cloud),
        ] {
            assert_eq!(Locality::of_endpoint(endpoint), want, "{endpoint}");
        }
    }

    #[test]
    fn the_wire_form_is_the_same_shape_for_every_answer_kind() {
        let v = Verdict {
            answers: HashMap::from([
                ("a".to_string(), Answer::Noul(0.8)),
                ("b".to_string(), Answer::Score { value: 1.5, distribution: vec![0.1, 0.3, 0.6] }),
                ("c".to_string(), Answer::Abstain { reason: "off".into() }),
            ]),
            by: JudgeInfo { adapter: "systemone", provider: "kev".into(), model: "kev-latest".into(),
                            locality: Locality::Home, calibrated: true },
            latency_ms: 41,
        };
        let j = v.to_json();
        assert_eq!(j["answers"]["a"], json!({"type": "noul", "yes": 0.8}));
        assert_eq!(j["answers"]["b"]["type"], "score");
        assert_eq!(j["answers"]["c"], json!({"type": "abstain", "reason": "off"}));
        assert_eq!(j["by"]["locality"], "home");
        assert!(!v.abstained());
    }
}
