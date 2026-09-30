//! The person's policy: where the mind may connect.
//!
//! Rules name a host (exactly, or `*.domain` for every name under it) and the ports it may be
//! reached on, and say why. The whole policy is in one of two modes:
//!
//! - **audit** — every destination is let through and counted, so a person can see where the mind
//!   goes before deciding anything. Where every machine starts.
//! - **enforce** — only what a rule allows; everything else is refused and becomes a proposal.
//!
//! Private mode refuses everything, whatever the policy says. (A rule has no mode of its own: in an
//! allow-list, a rule that only watched would let through exactly what one in force does.)
//!
//! Some addresses are never a destination, in any mode: loopback, link-local (the cloud metadata
//! address is one), unspecified, multicast and broadcast. The mind reaches this machine through
//! the mind door, and a name that resolves to 127.0.0.1 must not turn this proxy into a way past
//! the loopback guards; neither is any address of this machine's own (`crate::local`). In
//! enforce, an address on the local network is reached only by a rule that says `lan`; audit lets
//! it through, marked, as it lets everything through.
//!
//! Nothing is looked up before it may be reached ([`Policy::before_resolve`]): with Private mode
//! on, or in enforce without a rule, the name is refused unresolved — a lookup is itself a message
//! to whoever serves the name.

use std::net::IpAddr;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Audit,
    Enforce,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    /// `api.example.com`, `*.example.com`, or an address.
    pub host: String,
    pub ports: Vec<u16>,
    /// Plain `http://` forwarding, as well as tunnels.
    #[serde(default)]
    pub http: bool,
    /// It may resolve to an address on the local network.
    #[serde(default)]
    pub lan: bool,
    pub why: String,
}

impl Rule {
    pub fn matches(&self, host: &str, port: u16) -> bool {
        self.ports.contains(&port) && host_matches(&self.host, host)
    }
}

/// `*.example.com` covers `a.example.com` and `a.b.example.com`, not `example.com` itself.
pub fn host_matches(pattern: &str, host: &str) -> bool {
    match pattern.strip_prefix("*.") {
        Some(domain) => host.len() > domain.len() + 1 && host.ends_with(domain) && host[..host.len() - domain.len()].ends_with('.'),
        None => pattern == host,
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    #[serde(default)]
    pub mode: Mode,
    #[serde(default)]
    pub rules: Vec<Rule>,
}

/// What happens to one request.
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Let through: by a rule in force, or because the policy or the rule is only watching.
    Allow { audit: bool },
    /// Refused, with the sentence the caller is given.
    Refuse(String),
}

/// What kind of address a destination resolved to.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Place {
    Internet,
    Lan,
    /// Never a destination.
    Forbidden,
}

pub fn place_of(ip: IpAddr) -> Place {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            if v4.is_loopback() || v4.is_unspecified() || v4.is_link_local() || v4.is_multicast() || v4.is_broadcast() || o[0] == 0 {
                Place::Forbidden
            } else if v4.is_private() || (o[0] == 100 && (64..128).contains(&o[1])) {
                Place::Lan
            } else {
                Place::Internet
            }
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return place_of(IpAddr::V4(v4));
            }
            let seg = v6.segments();
            let b = v6.octets();
            // An IPv4 address carried inside an IPv6 one is where it leads: NAT64 (64:ff9b::/96)
            // and 6to4 (2002::/16) are classed by the address inside. The old IPv4-compatible
            // form (::a.b.c.d) is never a destination.
            if seg[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
                return place_of(IpAddr::V4(std::net::Ipv4Addr::new(b[12], b[13], b[14], b[15])));
            }
            if seg[0] == 0x2002 {
                return place_of(IpAddr::V4(std::net::Ipv4Addr::new(b[2], b[3], b[4], b[5])));
            }
            if seg[..6] == [0, 0, 0, 0, 0, 0] {
                return Place::Forbidden;
            }
            if v6.is_loopback() || v6.is_unspecified() || v6.is_multicast() || (seg[0] & 0xffc0) == 0xfe80 {
                Place::Forbidden
            } else if (seg[0] & 0xfe00) == 0xfc00 {
                Place::Lan
            } else {
                Place::Internet
            }
        }
    }
}

impl Policy {
    /// What can be decided before the name is looked up: Private mode, and in enforce, whether any
    /// rule covers it. `Some` is the answer; `None` is "resolve it, then [`Policy::decide`]".
    pub fn before_resolve(&self, host: &str, port: u16, http: bool, private: bool) -> Option<Verdict> {
        if private {
            return Some(Verdict::Refuse("Private mode is on: the mind reaches nothing until the person turns it off.".into()));
        }
        if self.mode == Mode::Enforce && !self.rules.iter().any(|r| r.matches(host, port) && (!http || r.http)) {
            return Some(self.decide(host, port, http, Place::Internet, false));
        }
        None
    }

    /// The verdict for `host:port`, resolved to `place`. `http` is a plain-HTTP request rather
    /// than a tunnel. `private` is the person's Private mode.
    pub fn decide(&self, host: &str, port: u16, http: bool, place: Place, private: bool) -> Verdict {
        if private {
            return Verdict::Refuse("Private mode is on: the mind reaches nothing until the person turns it off.".into());
        }
        if place == Place::Forbidden {
            return Verdict::Refuse(format!("{host} is this machine or an address that is never a destination; the mind reaches this desktop through its door."));
        }
        // The rule that allows this request, if any does; otherwise the first that names the host
        // and port, for the sentence that says why not — the same choice `before_resolve` made.
        let rule = self
            .rules
            .iter()
            .find(|r| r.matches(host, port) && (!http || r.http) && (place != Place::Lan || r.lan))
            .or_else(|| self.rules.iter().find(|r| r.matches(host, port)));
        match (self.mode, rule) {
            (_, Some(r)) if http && !r.http => {
                Verdict::Refuse(format!("{host}:{port} is allowed as a tunnel only (https), not plain http."))
            }
            (_, Some(r)) if place == Place::Lan && !r.lan => {
                Verdict::Refuse(format!("{host} resolved to an address on the local network, which its rule does not allow."))
            }
            (mode, Some(_)) => Verdict::Allow { audit: mode == Mode::Audit },
            (Mode::Audit, None) => Verdict::Allow { audit: true },
            (Mode::Enforce, None) => Verdict::Refuse(format!(
                "{host}:{port} is not a place the person has allowed the mind to connect. They have been asked; \
                 try again once they answer."
            )),
        }
    }

    /// Add a rule, replacing one for the same host and ports.
    pub fn allow(&mut self, rule: Rule) -> Result<(), String> {
        valid(&rule)?;
        self.rules.retain(|r| !(r.host == rule.host && r.ports == rule.ports));
        self.rules.push(rule);
        Ok(())
    }

    /// Remove every rule for `host`. Answers how many went.
    pub fn remove(&mut self, host: &str) -> usize {
        let before = self.rules.len();
        self.rules.retain(|r| r.host != host);
        before - self.rules.len()
    }

    pub fn load(path: &std::path::Path) -> Policy {
        let refuse_all = Policy { mode: Mode::Enforce, rules: Vec::new() };
        match std::fs::read_to_string(path) {
            Ok(text) => match serde_yaml::from_str::<Policy>(&text) {
                Ok(p) if p.rules.iter().all(|r| valid(r).is_ok()) => p,
                _ => {
                    // A policy that is there and does not read is not half-used, and not opened
                    // wide either — audit would let everything through. It refuses everything
                    // until the person writes it again; the shell says so.
                    tracing::error!(path = %path.display(), "the egress policy does not read; refusing everything until it is written again");
                    refuse_all
                }
            },
            // No policy yet: a new machine, which starts by watching. Only that: a policy that is
            // there and cannot be read (its mode, not text) is not taken for no policy.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Policy::default(),
            Err(e) => {
                tracing::error!(path = %path.display(), error = %e, "the egress policy cannot be read; refusing everything");
                refuse_all
            }
        }
    }
}

/// A rule the person could have written: a host that is a name or `*.name`, and real ports.
pub fn valid(r: &Rule) -> Result<(), String> {
    let name = r.host.strip_prefix("*.").unwrap_or(&r.host);
    let ok = !name.is_empty()
        && name.len() <= 253
        && name.contains(['.', ':'])
        && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-.:".contains(&b));
    if !ok {
        return Err(format!("`{}` is not a host name, `*.domain` or an address", r.host));
    }
    if r.ports.is_empty() || r.ports.contains(&0) || r.ports.len() > 16 {
        return Err("a rule names between one and sixteen ports".into());
    }
    if r.why.trim().is_empty() || r.why.len() > 200 {
        return Err("a rule says why, in a sentence".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(host: &str, ports: &[u16]) -> Rule {
        Rule { host: host.into(), ports: ports.to_vec(), http: false, lan: false, why: "because".into() }
    }

    #[test]
    fn wildcards_cover_names_under_a_domain_and_nothing_that_only_ends_the_same() {
        assert!(host_matches("*.example.com", "api.example.com"));
        assert!(host_matches("*.example.com", "a.b.example.com"));
        assert!(!host_matches("*.example.com", "example.com"));
        assert!(!host_matches("*.example.com", "evilexample.com"));
        assert!(host_matches("api.example.com", "api.example.com"));
        assert!(!host_matches("api.example.com", "api.example.com.evil"));
    }

    #[test]
    fn audit_lets_through_and_enforce_asks_for_a_rule() {
        let mut p = Policy::default();
        assert_eq!(p.decide("anywhere.com", 443, false, Place::Internet, false), Verdict::Allow { audit: true });
        p.mode = Mode::Enforce;
        assert!(matches!(p.decide("anywhere.com", 443, false, Place::Internet, false), Verdict::Refuse(_)));
        p.allow(rule("anywhere.com", &[443])).unwrap();
        assert_eq!(p.decide("anywhere.com", 443, false, Place::Internet, false), Verdict::Allow { audit: false });
        assert!(matches!(p.decide("anywhere.com", 80, false, Place::Internet, false), Verdict::Refuse(_)), "only its ports");
    }

    #[test]
    fn private_mode_and_forbidden_addresses_refuse_in_every_mode() {
        let mut p = Policy::default();
        p.allow(rule("anywhere.com", &[443])).unwrap();
        for mode in [Mode::Audit, Mode::Enforce] {
            p.mode = mode;
            assert!(matches!(p.decide("anywhere.com", 443, false, Place::Internet, true), Verdict::Refuse(_)));
            assert!(matches!(p.decide("anywhere.com", 443, false, Place::Forbidden, false), Verdict::Refuse(_)));
        }
    }

    #[test]
    fn the_local_network_and_plain_http_need_a_rule_that_says_so() {
        let mut p = Policy { mode: Mode::Enforce, rules: vec![] };
        p.allow(rule("gpu.lan", &[11434])).unwrap();
        assert!(matches!(p.decide("gpu.lan", 11434, false, Place::Lan, false), Verdict::Refuse(_)));
        assert!(matches!(p.decide("gpu.lan", 11434, true, Place::Internet, false), Verdict::Refuse(_)));
        let mut r = rule("gpu.lan", &[11434]);
        r.lan = true;
        r.http = true;
        p.allow(r).unwrap();
        assert_eq!(p.rules.len(), 1, "the same host and ports are replaced, not added twice");
        assert_eq!(p.decide("gpu.lan", 11434, true, Place::Lan, false), Verdict::Allow { audit: false });
    }

    #[test]
    fn nothing_is_looked_up_that_may_not_be_reached() {
        let mut p = Policy::default();
        assert!(matches!(p.before_resolve("x.example", 443, false, true), Some(Verdict::Refuse(_))), "private: not even resolved");
        assert_eq!(p.before_resolve("x.example", 443, false, false), None, "audit resolves everything");
        p.mode = Mode::Enforce;
        assert!(matches!(p.before_resolve("x.example", 443, false, false), Some(Verdict::Refuse(_))), "enforce, no rule: unresolved");
        p.allow(rule("x.example", &[443])).unwrap();
        assert_eq!(p.before_resolve("x.example", 443, false, false), None);
        assert!(matches!(p.before_resolve("x.example", 443, true, false), Some(Verdict::Refuse(_))), "a tunnel rule is not an http one");
    }

    #[test]
    fn places() {
        for (ip, want) in [
            ("127.0.0.1", Place::Forbidden),
            ("169.254.169.254", Place::Forbidden),
            ("0.0.0.0", Place::Forbidden),
            ("224.0.0.1", Place::Forbidden),
            ("::1", Place::Forbidden),
            ("::ffff:127.0.0.1", Place::Forbidden),
            ("fe80::1", Place::Forbidden),
            ("192.168.4.20", Place::Lan),
            ("10.1.2.3", Place::Lan),
            ("100.64.0.1", Place::Lan),
            ("fd00::1", Place::Lan),
            ("1.1.1.1", Place::Internet),
            ("2606:4700::1111", Place::Internet),
            ("64:ff9b::7f00:1", Place::Forbidden),
            ("64:ff9b::c0a8:414", Place::Lan),
            ("2002:7f00:1::", Place::Forbidden),
            ("::127.0.0.1", Place::Forbidden),
            ("::8.8.8.8", Place::Forbidden),
        ] {
            assert_eq!(place_of(ip.parse().unwrap()), want, "{ip}");
        }
    }

    #[test]
    fn a_rule_the_person_could_not_have_meant_is_refused() {
        for bad in ["", "*", "*.", "localhost", "a b.com", "Example.com", "x.com/path"] {
            assert!(valid(&rule(bad, &[443])).is_err(), "{bad}");
        }
        assert!(valid(&rule("x.com", &[])).is_err());
        assert!(valid(&rule("x.com", &[0])).is_err());
        let mut r = rule("x.com", &[443]);
        r.why = " ".into();
        assert!(valid(&r).is_err());
    }

    #[test]
    fn a_policy_that_does_not_read_refuses_everything_and_no_policy_watches() {
        let d = std::env::temp_dir().join(format!("yantrik-egress-policy-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("policy.yaml");
        std::fs::write(&p, "mode: enforce\nrules:\n  - host: '*'\n    ports: [443]\n    why: all\n").unwrap();
        assert_eq!(Policy::load(&p), Policy { mode: Mode::Enforce, rules: vec![] }, "never half-used, never wide open");
        assert_eq!(Policy::load(&d.join("none.yaml")), Policy::default(), "a new machine audits");
        std::fs::write(&p, b"mode: enforce\n\xff\n").unwrap();
        assert_eq!(Policy::load(&p), Policy { mode: Mode::Enforce, rules: vec![] }, "not text: refused, not audit");
        std::fs::write(&p, "mode: enforce\nrules:\n  - host: api.x.ai\n    ports: [443]\n    why: the model\n").unwrap();
        let got = Policy::load(&p);
        assert_eq!(got.mode, Mode::Enforce);
        assert_eq!(got.rules.len(), 1);
    }
}
