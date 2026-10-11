//! The person's policy: where the mind may connect.
//!
//! Rules name a host (exactly, or `*.domain` for every name under it) and the ports it may be
//! reached on, and say why. The whole policy is in one of three modes, from the least strict:
//!
//! - **audit** — every destination on the internet is let through and counted, so a person can see
//!   where the mind goes before deciding anything. Where every machine starts.
//! - **guarded** — "home network closed, internet open": every destination on the internet is let
//!   through and counted as allowed, not watched; the local network (every prefix this machine is
//!   on and its routers, whatever their range: `crate::local::place`), private ranges, loopback and
//!   this machine are decided exactly as in enforce. The kernel refuses the mind DNS, as in
//!   enforce: this proxy resolves. A rule still decides for the host it names: a tunnel-only rule
//!   (`http: false`) refuses plain http to that host, which with no rule would be let through.
//! - **enforce** — only what a rule allows; everything else is refused and becomes a proposal.
//!
//! Private mode refuses everything, whatever the policy says. (A rule has no mode of its own: in an
//! allow-list, a rule that only watched would let through exactly what one in force does.)
//!
//! Some addresses are never a destination, in any mode: loopback, link-local (the cloud metadata
//! address is one), unspecified, multicast and broadcast. The mind reaches this machine through
//! the mind door, and a name that resolves to 127.0.0.1 must not turn this proxy into a way past
//! the loopback guards; neither is any address of this machine's own (`crate::local`). An address
//! that is not the internet — the local network, and every private or special-use range in
//! `crate::ranges` (CGNAT and Tailscale, NAT64, 6to4, Teredo, …) — is reached only by a rule that
//! says `lan`, in every mode, audit included: the Mind leaves that classification to this proxy
//! instead of resolving names itself.
//!
//! Nothing is looked up before it may be reached ([`Policy::before_resolve`]): with Private mode
//! on, or in enforce without a rule, the name is refused unresolved (guarded resolves it: where it
//! leads decides) — a lookup is itself a message
//! to whoever serves the name.

use std::net::IpAddr;

/// What this proxy promises about itself, one word each, printed by `yantrik-egress capabilities`
/// for root to read (`yantrik-update mind-egress apply` puts it in
/// /run/yantrik-mind-egress/mind-egress.json).
/// `refuses-private-all-modes`: an address that is not the internet is refused without a `lan`
/// rule in audit as well as enforce ([`Policy::decide`]; the test below holds it to that).
/// `public-door`: a second listener, `EGRESS_PUBLIC_LISTEN` (127.0.0.1:7451), that never reaches
/// the local network or a host a `lan` rule names (`crate::door`).
pub const CAPABILITIES: &[&str] = &["refuses-private-all-modes", "public-door"];

use serde::{Deserialize, Serialize};

/// Declared from the least strict to the most, so `Ord` is strictness: audit < guarded < enforce.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Audit,
    Guarded,
    Enforce,
}

impl Mode {
    /// Every mode, the least strict first: the order the person chooses from.
    pub const ALL: [Mode; 3] = [Mode::Audit, Mode::Guarded, Mode::Enforce];

    /// The word the policy file, the control socket and the status file use.
    pub fn word(self) -> &'static str {
        match self {
            Mode::Audit => "audit",
            Mode::Guarded => "guarded",
            Mode::Enforce => "enforce",
        }
    }

    /// What the person is shown when choosing, in plain words.
    pub fn label(self) -> &'static str {
        match self {
            Mode::Audit => "Watch only: everything allowed and recorded",
            Mode::Guarded => "Home network closed, internet open",
            Mode::Enforce => "Only places I approve",
        }
    }

    /// The one shown as Recommended.
    pub fn recommended(self) -> bool {
        self == Mode::Guarded
    }
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
    /// Made by the updater from the person's or root's own configuration (`crate::seed`), not
    /// written by the person. Re-seeding replaces these and never a rule of the person's.
    #[serde(default, skip_serializing_if = "is_false")]
    pub seeded: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
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

/// Where `ip` leads. Never a destination: see the module's notes. Not the internet (`Lan`): the
/// local network, and every other private or special-use range (`crate::ranges`: CGNAT and
/// Tailscale's 100.64/10, the documentation and benchmark ranges, NAT64, 6to4, Teredo, …).
pub fn place_of(ip: IpAddr) -> Place {
    match base_place(ip) {
        Place::Internet if crate::ranges::special(ip) => Place::Lan,
        p => p,
    }
}

fn base_place(ip: IpAddr) -> Place {
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
            // Not the internet, in audit and guarded too: only a rule that says `lan` reaches it.
            // The Mind leaves this to the proxy when it is behind one, instead of resolving names.
            (Mode::Audit | Mode::Guarded, None) if place == Place::Lan => Verdict::Refuse(format!(
                "{host} is on the local network or a private address, which the mind reaches only by a rule that says so. \
                 The person has been asked."
            )),
            (Mode::Audit, None) => Verdict::Allow { audit: true },
            // The internet, open: allowed and counted as allowed, not as watching.
            (Mode::Guarded, None) => Verdict::Allow { audit: false },
            (Mode::Enforce, None) => Verdict::Refuse(format!(
                "{host}:{port} is not a place the person has allowed the mind to connect. They have been asked; \
                 try again once they answer."
            )),
        }
    }

    /// Replace the seeded rules with `rules` (`crate::seed`), leaving every rule of the person's as
    /// it is. A seed the person's own `lan` rule already covers is not added. Answers what was.
    pub fn seed(&mut self, rules: Vec<Rule>) -> Result<Vec<Rule>, String> {
        if rules.len() > crate::seed::MOST {
            return Err(format!("at most {} seeded rules", crate::seed::MOST));
        }
        for r in &rules {
            valid(r)?;
            if !r.lan {
                return Err("a seeded rule says lan".into());
            }
        }
        self.rules.retain(|r| !r.seeded);
        let mut added = Vec::new();
        for mut r in rules {
            r.seeded = true;
            let covered = self.rules.iter().any(|p| p.lan && p.host == r.host && r.ports.iter().all(|port| p.ports.contains(port)));
            if !covered {
                self.rules.push(r.clone());
                added.push(r);
            }
        }
        Ok(added)
    }

    /// Add a rule, replacing one for the same host and ports. The person's, never a seeded one.
    pub fn allow(&mut self, mut rule: Rule) -> Result<(), String> {
        rule.seeded = false;
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

    /// The policy at `path`, as written. No file yet is a new machine, which starts by watching.
    /// A file that is there and does not read (its mode, not text, not a policy, or a rule the
    /// person could not have written) is an error, never taken for no policy. The proxy and the
    /// kernel's direct set (`crate::direct`) read it through this one door.
    pub fn read(path: &std::path::Path) -> Result<Policy, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let mut p = serde_yaml::from_str::<Policy>(&text).map_err(|e| format!("{}: {e}", path.display()))?;
                // A rule written before `valid` refused a trailing dot is read without it: the
                // name it meant, as `lan_hosts` already published it and the public door refuses
                // it. Refusing the whole policy for it would refuse everything, and the next seed
                // or allow would be saved over the person's rules.
                for r in &mut p.rules {
                    let trimmed = r.host.trim_end_matches('.');
                    if trimmed.len() != r.host.len() {
                        r.host = trimmed.to_string();
                    }
                }
                match p.rules.iter().find_map(|r| valid(r).err()) {
                    None => Ok(p),
                    Some(e) => Err(format!("{}: {e}", path.display())),
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Policy::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    pub fn load(path: &std::path::Path) -> Policy {
        Policy::read(path).unwrap_or_else(|e| {
            // A policy that is there and does not read is not half-used, and not opened wide
            // either — audit would let everything through. It refuses everything until the
            // person writes it again; the shell says so.
            tracing::error!(error = %e, "the egress policy does not read; refusing everything until it is written again");
            Policy { mode: Mode::Enforce, rules: Vec::new() }
        })
    }
}

/// A rule the person could have written: a host that is a name or `*.name`, and real ports. A
/// host is written one way only — lowercase, no trailing dot — the way a request's host is made
/// (`crate::request`), so a rule and the request it names compare as text.
pub fn valid(r: &Rule) -> Result<(), String> {
    let name = r.host.strip_prefix("*.").unwrap_or(&r.host);
    let ok = !name.is_empty()
        && !name.ends_with('.')
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
        Rule { host: host.into(), ports: ports.to_vec(), http: false, lan: false, why: "because".into(), seeded: false }
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
        for mode in Mode::ALL {
            p.mode = mode;
            assert!(matches!(p.decide("anywhere.com", 443, false, Place::Internet, true), Verdict::Refuse(_)));
            assert!(matches!(p.decide("other.com", 443, false, Place::Internet, true), Verdict::Refuse(_)), "{mode:?}: no rule either");
            assert!(matches!(p.before_resolve("other.com", 443, false, true), Some(Verdict::Refuse(_))), "{mode:?}: not even resolved");
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
    fn audit_refuses_what_is_not_the_internet_without_a_lan_rule() {
        let mut p = Policy::default();
        for ip in ["192.168.4.42", "100.100.1.1", "2001:0:4136:e378::1", "2002:c0a8:42::1", "64:ff9b::808:808", "198.18.0.9"] {
            let place = place_of(ip.parse().unwrap());
            assert_eq!(place, Place::Lan, "{ip}");
            assert!(matches!(p.decide(ip, 8888, true, place, false), Verdict::Refuse(_)), "audit, no rule: {ip} refused");
        }
        assert!(matches!(p.decide("searx.lan", 8888, true, Place::Lan, false), Verdict::Refuse(_)), "a name that resolves there too");
        p.allow(rule("searx.lan", &[8888])).unwrap();
        assert!(matches!(p.decide("searx.lan", 8888, false, Place::Lan, false), Verdict::Refuse(_)), "a rule without lan is not enough");
        let mut r = rule("192.168.4.42", &[8888]);
        r.lan = true;
        r.http = true;
        p.allow(r).unwrap();
        assert_eq!(p.decide("192.168.4.42", 8888, true, Place::Lan, false), Verdict::Allow { audit: true }, "a lan rule reaches it");
        assert!(matches!(p.decide("192.168.4.42", 22, false, Place::Lan, false), Verdict::Refuse(_)), "only on its ports");
        assert_eq!(p.decide("example.com", 443, false, Place::Internet, false), Verdict::Allow { audit: true }, "the internet still audits");
    }

    #[test]
    fn the_capability_it_prints_is_one_it_has() {
        assert!(CAPABILITIES.contains(&"refuses-private-all-modes"));
        for mode in Mode::ALL {
            let p = Policy { mode, ..Policy::default() };
            assert!(matches!(p.decide("10.0.0.7", 80, true, place_of("10.0.0.7".parse().unwrap()), false), Verdict::Refuse(_)), "{mode:?}");
        }
    }

    #[test]
    fn guarded_opens_the_internet_and_keeps_the_home_network_closed() {
        let mut p = Policy { mode: Mode::Guarded, rules: vec![] };
        assert_eq!(p.decide("example.com", 443, false, Place::Internet, false), Verdict::Allow { audit: false }, "allowed, not watched");
        assert_eq!(p.decide("example.com", 80, true, Place::Internet, false), Verdict::Allow { audit: false }, "plain http too");
        assert_eq!(p.before_resolve("example.com", 443, false, false), None, "resolved: where it leads decides");
        for ip in ["192.168.4.42", "10.0.0.7", "100.100.1.1", "fd00::5", "198.18.0.9", "127.0.0.1", "::1", "169.254.169.254"] {
            let place = place_of(ip.parse().unwrap());
            assert_ne!(place, Place::Internet, "{ip}");
            assert!(matches!(p.decide(ip, 8888, false, place, false), Verdict::Refuse(_)), "guarded, no rule: {ip} refused");
        }
        assert!(matches!(p.decide("searx.lan", 8888, false, Place::Lan, false), Verdict::Refuse(_)), "a name that resolves there");
        assert!(matches!(p.decide("localhost", 11434, false, Place::Forbidden, false), Verdict::Refuse(_)), "this machine");
        // A rule without lan does not open the home network; one that says lan does, on its ports.
        p.allow(rule("192.168.4.42", &[8888])).unwrap();
        assert!(matches!(p.decide("192.168.4.42", 8888, false, Place::Lan, false), Verdict::Refuse(_)));
        let mut r = rule("192.168.4.42", &[8888]);
        r.lan = true;
        p.allow(r).unwrap();
        assert_eq!(p.decide("192.168.4.42", 8888, false, Place::Lan, false), Verdict::Allow { audit: false });
        assert!(matches!(p.decide("192.168.4.42", 22, false, Place::Lan, false), Verdict::Refuse(_)), "only its ports");
        // A lan rule never opens loopback or this machine.
        let mut r = rule("127.0.0.1", &[11434]);
        r.lan = true;
        p.allow(r).unwrap();
        assert!(matches!(p.decide("127.0.0.1", 11434, false, Place::Forbidden, false), Verdict::Refuse(_)));
    }

    #[test]
    fn guarded_sits_between_audit_and_enforce_and_decides_the_rest_as_enforce_does() {
        assert!(Mode::Audit < Mode::Guarded && Mode::Guarded < Mode::Enforce);
        assert_eq!(Mode::ALL.map(Mode::word), ["audit", "guarded", "enforce"]);
        for m in Mode::ALL {
            assert_eq!(serde_yaml::to_string(&m).unwrap().trim(), m.word());
        }
        let mut lan = rule("gpu.lan", &[11434]);
        lan.lan = true;
        let tunnel_only = rule("api.x.ai", &[443]);
        let rules = vec![lan, tunnel_only];
        let guarded = Policy { mode: Mode::Guarded, rules: rules.clone() };
        let enforce = Policy { mode: Mode::Enforce, rules };
        for (host, port, http, place) in [
            ("gpu.lan", 11434, false, Place::Lan),
            ("gpu.lan", 22, false, Place::Lan),
            ("other.lan", 11434, false, Place::Lan),
            ("10.0.0.7", 80, true, Place::Lan),
            ("localhost", 80, false, Place::Forbidden),
            ("api.x.ai", 443, false, Place::Internet),
            ("api.x.ai", 443, true, Place::Internet),
        ] {
            let same = matches!(
                (guarded.decide(host, port, http, place, false), enforce.decide(host, port, http, place, false)),
                (Verdict::Allow { .. }, Verdict::Allow { .. }) | (Verdict::Refuse(_), Verdict::Refuse(_))
            );
            assert!(same, "{host}:{port} http={http} {place:?}");
        }
    }

    #[test]
    fn reseeding_replaces_seeded_rules_and_never_the_persons() {
        let seed = |host: &str, port: u16| Rule { host: host.into(), ports: vec![port], http: true, lan: true, why: "seeded from config.yaml".into(), seeded: false };
        let mut p = Policy::default();
        p.allow(rule("api.x.ai", &[443])).unwrap();
        let mut own_lan = rule("192.168.4.42", &[8888]);
        own_lan.lan = true;
        p.allow(own_lan).unwrap();
        let added = p.seed(vec![seed("192.168.4.35", 11434), seed("192.168.4.42", 8888), seed("ha.local", 8123)]).unwrap();
        assert_eq!(added.iter().map(|r| r.host.as_str()).collect::<Vec<_>>(), ["192.168.4.35", "ha.local"], "the person's own lan rule covers SearXNG");
        assert!(added.iter().all(|r| r.seeded));
        assert_eq!(p.rules.len(), 4);
        // The source changed: the old seeds go, the person's stay.
        p.seed(vec![seed("192.168.4.36", 11434)]).unwrap();
        let hosts: Vec<(&str, bool)> = p.rules.iter().map(|r| (r.host.as_str(), r.seeded)).collect();
        assert_eq!(hosts, [("api.x.ai", false), ("192.168.4.42", false), ("192.168.4.36", true)]);
        // Nothing seeded any more: only the person's.
        p.seed(vec![]).unwrap();
        assert_eq!(p.rules.len(), 2);
        assert!(p.rules.iter().all(|r| !r.seeded));
        // The person allowing the same host and ports makes it theirs.
        p.seed(vec![seed("192.168.4.36", 11434)]).unwrap();
        let mut mine = seed("192.168.4.36", 11434);
        mine.seeded = true;
        p.allow(mine).unwrap();
        p.seed(vec![]).unwrap();
        assert!(p.rules.iter().any(|r| r.host == "192.168.4.36" && !r.seeded), "now the person's: a re-seed keeps it");
        // Seeds say lan, read as rules, and there are not many.
        let mut not_lan = seed("192.168.4.37", 80);
        not_lan.lan = false;
        assert!(p.seed(vec![not_lan]).is_err());
        assert!(p.seed(vec![seed("*", 80)]).is_err());
        assert!(p.seed((0..=crate::seed::MOST as u16).map(|i| seed("192.168.4.38", 1000 + i)).collect()).is_err());
        // Saved and read back with the mark; a person's rule is written without it.
        let text = serde_yaml::to_string(&p).unwrap();
        assert!(!text.contains("seeded: false"));
        assert_eq!(serde_yaml::from_str::<Policy>(&text).unwrap(), p);
    }

    #[test]
    fn nothing_is_looked_up_that_may_not_be_reached() {
        let mut p = Policy::default();
        assert!(matches!(p.before_resolve("x.example", 443, false, true), Some(Verdict::Refuse(_))), "private: not even resolved");
        p.mode = Mode::Guarded;
        assert!(matches!(p.before_resolve("x.example", 443, false, true), Some(Verdict::Refuse(_))), "private in guarded too");
        p.mode = Mode::Audit;
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
        for bad in ["", "*", "*.", "localhost", "a b.com", "Example.com", "x.com/path", "x.com.", "*.x.com.", "x.com.."] {
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

    /// A policy written before `valid` refused a trailing dot is read with the dot taken off, not
    /// refused whole: the name it meant, the one `lan_hosts` published and the public door refuses.
    #[test]
    fn a_rule_written_with_a_trailing_dot_before_is_read_without_it() {
        let d = std::env::temp_dir().join(format!("yantrik-egress-dot-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("policy.yaml");
        std::fs::write(
            &p,
            "mode: enforce\nrules:\n  - host: gpu.example.ts.net.\n    ports: [11434]\n    lan: true\n    why: the model\n  \
             - host: '*.lab.example..'\n    ports: [80]\n    lan: true\n    why: the lab\n",
        )
        .unwrap();
        let got = Policy::read(&p).unwrap();
        assert_eq!(got.rules.iter().map(|r| r.host.as_str()).collect::<Vec<_>>(), ["gpu.example.ts.net", "*.lab.example"]);
        assert_eq!(got.decide("gpu.example.ts.net", 11434, false, Place::Lan, false), Verdict::Allow { audit: false });
        for host in [".", "*."] {
            std::fs::write(&p, format!("rules:\n  - host: '{host}'\n    ports: [80]\n    why: nothing\n")).unwrap();
            assert!(Policy::read(&p).is_err(), "{host} is still no host");
        }
        let _ = std::fs::remove_dir_all(&d);
    }
}
