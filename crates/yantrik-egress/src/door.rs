//! Two doors into one proxy (design/mind-egress-2026-09-29.md, "Two doors").
//!
//! - The **endpoint door** (`EGRESS_LISTEN`, 127.0.0.1:7450) is the proxy as it always was: a rule
//!   that says `lan` reaches the local network. Only the Mind's clients for its own configured
//!   endpoints (its model servers, Home Assistant, SearXNG) use it.
//! - The **public door** (`EGRESS_PUBLIC_LISTEN`, 127.0.0.1:7451) never reaches anything but the
//!   internet, whatever the rules say. Everything that fetches what someone else chose — yt-dlp,
//!   ffmpeg, the browser, the fetch tool, search results, images, papers — uses it, so a redirect
//!   or an HLS segment the Mind never saw cannot ride a `lan` rule to a service on the LAN.
//!
//! On the public door a host that any `lan` rule names (exactly or by `*.domain`, on any port) is
//! refused before it is looked up, so a `lan` rule can never let it through; and what is
//! resolved must be the internet (`crate::policy::place_of`, `crate::ranges`): the local network,
//! every private or special-use range, loopback and this machine are refused. The rest — the
//! head, Private mode, audit, guarded and enforce, the ledger, the connection — is the same code for both.

use std::net::IpAddr;

use crate::policy::{host_matches, Place, Policy, Verdict};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Door {
    /// Honours `lan` rules: the Mind's own endpoints.
    Endpoint,
    /// Never the local network: anything fetched for someone else.
    Public,
}

impl Door {
    pub fn name(self) -> &'static str {
        match self {
            Door::Endpoint => "endpoint",
            Door::Public => "public",
        }
    }

    /// The door's answer before the name is looked up: `Some` refuses it unresolved.
    pub fn before_resolve(self, policy: &Policy, host: &str) -> Option<Verdict> {
        if self == Door::Endpoint {
            return None;
        }
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        policy.rules.iter().any(|r| r.lan && names(&r.host, &host)).then(|| {
            Verdict::Refuse(format!(
                "{host} is one of the Mind's own endpoints (a rule that says lan names it); the public door never reaches it."
            ))
        })
    }

    /// The door's answer once the name resolved to `place`: `Some` refuses it.
    pub fn after_resolve(self, host: &str, place: Place) -> Option<Verdict> {
        (self == Door::Public && place != Place::Internet).then(|| {
            Verdict::Refuse(format!(
                "{host} is on the local network, this machine or an address that is not the internet; the public door reaches the internet only."
            ))
        })
    }
}

/// Whether a rule's host names `host` (already lowercased, without a trailing dot). The rule's
/// host is made the same way, so a rule written `GPU.example.` names `gpu.example`, as it does in
/// the status file's `lan_hosts` (`crate::direct::lan_hosts`); an address is compared as an
/// address, so `2001:470:0::1` names `[2001:470::1]` and `1.2.3.4` names `[::ffff:1.2.3.4]`.
fn names(rule_host: &str, host: &str) -> bool {
    let pattern = rule_host.trim_end_matches('.').to_ascii_lowercase();
    match (pattern.parse::<IpAddr>(), host.parse::<IpAddr>()) {
        (Ok(a), Ok(b)) => a.to_canonical() == b.to_canonical(),
        _ => host_matches(&pattern, host),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::{place_of, Mode, Rule};

    fn lan(host: &str, ports: &[u16]) -> Rule {
        Rule { host: host.into(), ports: ports.to_vec(), http: true, lan: true, why: "an endpoint".into(), seeded: true }
    }

    fn policy(mode: Mode) -> Policy {
        let mut p = Policy { mode, rules: vec![] };
        p.seed(vec![lan("gpu.example.ts.net", &[11434]), lan("*.home.arpa", &[8123]), lan("192.168.4.42", &[8888])]).unwrap();
        // Written as `valid` no longer lets a rule be, but as a policy could hold them before: a
        // trailing dot, and an IPv6 address not in its canonical form.
        p.rules.push(lan("nas.example.ts.net.", &[445]));
        p.rules.push(lan("*.lab.example.", &[80]));
        p.rules.push(lan("2001:470:0:0::1", &[8080]));
        p.rules.push(lan("203.0.113.9", &[9000]));
        p
    }

    #[test]
    fn the_public_door_refuses_every_host_a_lan_rule_names_on_every_port() {
        for mode in Mode::ALL {
            let p = policy(mode);
            for host in [
                "gpu.example.ts.net", "GPU.Example.ts.net.", "ha.home.arpa", "a.b.home.arpa", "192.168.4.42",
                "nas.example.ts.net", "NAS.example.ts.net.", "x.lab.example", "2001:470::1", "2001:0470:0000::0001",
                "203.0.113.9", "::ffff:203.0.113.9", "::ffff:192.168.4.42",
            ] {
                for port in [11434, 8123, 8888, 445, 8080, 9000, 443, 80, 1] {
                    assert!(matches!(Door::Public.before_resolve(&p, host), Some(Verdict::Refuse(_))), "{mode:?} {host}:{port}");
                }
                assert_eq!(Door::Endpoint.before_resolve(&p, host), None, "the endpoint door leaves it to the policy");
            }
            for host in ["example.com", "home.arpa", "evilhome.arpa", "gpu.example.ts.net.evil.com", "lab.example", "2001:470::2", "203.0.113.8"] {
                assert_eq!(Door::Public.before_resolve(&p, host), None, "{host} is not named by a lan rule");
            }
        }
    }

    #[test]
    fn the_capability_it_prints_is_one_it_has() {
        assert!(crate::policy::CAPABILITIES.contains(&"public-door"));
        assert!(matches!(Door::Public.after_resolve("10.0.0.7", place_of("10.0.0.7".parse().unwrap())), Some(Verdict::Refuse(_))));
    }

    #[test]
    fn a_rule_without_lan_is_no_reason_to_refuse() {
        let mut p = Policy::default();
        p.allow(Rule { host: "api.x.ai".into(), ports: vec![443], http: false, lan: false, why: "the model".into(), seeded: false }).unwrap();
        assert_eq!(Door::Public.before_resolve(&p, "api.x.ai"), None);
    }

    #[test]
    fn the_public_door_reaches_the_internet_only() {
        for ip in [
            "127.0.0.1", "10.0.0.7", "172.16.0.1", "192.168.4.42", "169.254.169.254", "100.64.0.1", "100.100.100.100",
            "fd00::5", "fe80::1", "::1", "::ffff:192.168.4.42", "::ffff:127.0.0.1", "::ffff:100.64.0.1", "64:ff9b::c0a8:42",
            "2002:c0a8:42::1", "198.18.0.9",
        ] {
            let place = place_of(ip.parse().unwrap());
            assert!(matches!(Door::Public.after_resolve(ip, place), Some(Verdict::Refuse(_))), "{ip}");
        }
        for ip in ["1.1.1.1", "2606:4700::1111", "::ffff:8.8.8.8"] {
            assert_eq!(Door::Public.after_resolve(ip, place_of(ip.parse().unwrap())), None, "{ip}");
        }
        assert_eq!(Door::Endpoint.after_resolve("gpu.lan", Place::Lan), None, "the endpoint door leaves lan to the rules");
    }

    /// A name is judged by what it resolved to (`crate::proxy::classify`): every address private,
    /// loopback, link-local, CGNAT, ULA or IPv4 written as IPv6 is refused at the public door; a
    /// name that also gives the internet is reached there only.
    #[test]
    fn a_name_is_judged_by_the_addresses_it_resolved_to() {
        let own = crate::local::Net { own: vec!["127.0.0.1".parse().unwrap(), "::1".parse().unwrap()], ..Default::default() };
        let at = |ips: &[&str]| ips.iter().map(|i| std::net::SocketAddr::new(i.parse().unwrap(), 443)).collect::<Vec<_>>();
        for ips in [
            &["10.0.0.7"][..], &["127.0.0.1"], &["::1"], &["169.254.169.254"], &["100.64.0.7"], &["fd00::5"],
            &["::ffff:192.168.4.42"], &["::ffff:127.0.0.1"], &["10.0.0.7", "127.0.0.1", "fd00::5"],
        ] {
            let (place, _) = crate::proxy::classify(&at(ips), Some(&own));
            assert!(matches!(Door::Public.after_resolve("name.example", place), Some(Verdict::Refuse(_))), "{ips:?}");
        }
        let (place, usable) = crate::proxy::classify(&at(&["10.0.0.7", "1.1.1.1", "::ffff:192.168.4.42"]), Some(&own));
        assert_eq!(Door::Public.after_resolve("name.example", place), None);
        assert_eq!(usable, at(&["1.1.1.1"]), "reached at its internet address only");
    }
}
