//! Seeded rules: the LAN services this machine is already configured to use.
//!
//! The proxy refuses every address that is not the internet unless a rule that says `lan` covers
//! it, in every mode (`crate::policy`). Until the person can write such rules from the desktop, a
//! machine that worked yesterday — a model on the home GPU box, Home Assistant, a SearXNG, a
//! Tailscale peer — would stop working at the upgrade. So the updater, as root, reads the
//! endpoints the person (or root) configured, and this module turns each one that leads to the
//! local network into a rule marked `seeded`:
//!
//! `{host, ports: [port], http: scheme == http, lan: true, why: "seeded from <source>"}`
//!
//! The sources are the updater's to choose, and are only files the person or root owns (never the
//! mind's own settings: a mind must not shape its own policy). Re-seeding replaces the seeded
//! rules and never touches the person's own.
//!
//! `yantrik-egress seed-plan` reads `<source> <url>` lines on stdin and prints the plan, run by
//! the updater as this proxy's account:
//!
//! ```text
//! seed 192.168.4.35 11434 http config.yaml
//! seed homeassistant.local 8123 http mind-person.env:YM_HA_URL
//! skip config.yaml api.openai.com public
//! ```
//!
//! The updater checks the plan again as root and hands it to the `seed` op of the control socket.

use std::net::{IpAddr, ToSocketAddrs};

use crate::policy::{place_of, valid, Place, Rule};

/// The most seeded rules a policy holds.
pub const MOST: usize = 16;

/// Names that can only be on the local network (RFC 6762 `.local`, RFC 8375 `.home.arpa`, and the
/// conventional private `.internal` and `.lan`): seeded without a lookup, which may well fail from
/// the proxy's account (mDNS) while the service is there.
const LOCAL_SUFFIXES: [&str; 4] = [".local", ".home.arpa", ".internal", ".lan"];

/// A URL's scheme, host and port, or why it is not one a rule can be made from.
pub fn parse_url(url: &str) -> Result<(bool, String, u16), &'static str> {
    let (http, rest) = if let Some(r) = url.strip_prefix("http://") {
        (true, r)
    } else if let Some(r) = url.strip_prefix("https://") {
        (false, r)
    } else {
        return Err("not an http or https URL");
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.contains('@') {
        return Err("a URL with credentials in it");
    }
    let (host, port) = if let Some(v6) = authority.strip_prefix('[') {
        let (h, after) = v6.split_once(']').ok_or("an unclosed IPv6 address")?;
        (h, after.strip_prefix(':'))
    } else {
        match authority.rsplit_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (authority, None),
        }
    };
    let port = match port {
        None => if http { 80 } else { 443 },
        Some(p) => match p.parse::<u16>() {
            Ok(n) if n > 0 && !p.starts_with('+') => n,
            _ => return Err("a port that is not 1-65535"),
        },
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() || host.contains('%') {
        return Err("no host, or an address with a scope id");
    }
    Ok((http, host, port))
}

/// What one candidate makes: a seeded rule, or the reason it makes none.
#[derive(Debug, PartialEq, Eq)]
pub enum Planned {
    Seed(Rule),
    Skip { source: String, host: String, why: &'static str },
}

/// The plan for `<source> <url>` candidates. `resolve` looks a name up (the proxy's own lookup in
/// production, a table in tests).
pub fn plan(candidates: &[(String, String)], resolve: &dyn Fn(&str, u16) -> Option<Vec<IpAddr>>) -> Vec<Planned> {
    let mut out = Vec::new();
    for (source, url) in candidates {
        let skip = |host: &str, why| Planned::Skip { source: source.clone(), host: host.to_string(), why };
        let (http, host, port) = match parse_url(url) {
            Ok(p) => p,
            Err(why) => {
                out.push(skip("", why));
                continue;
            }
        };
        let rule = Rule { host: host.clone(), ports: vec![port], http, lan: true, why: format!("seeded from {source}"), seeded: true };
        if valid(&rule).is_err() {
            out.push(skip(&host, "not a host a rule can hold"));
            continue;
        }
        let place = match host.parse::<IpAddr>() {
            Ok(ip) => place_of(ip),
            Err(_) if LOCAL_SUFFIXES.iter().any(|s| host.ends_with(s)) => Place::Lan,
            Err(_) => match resolve(&host, port) {
                Some(addrs) if !addrs.is_empty() => {
                    if addrs.iter().all(|a| place_of(*a) == Place::Lan) {
                        Place::Lan
                    } else if addrs.iter().any(|a| place_of(*a) == Place::Internet) {
                        Place::Internet
                    } else {
                        Place::Forbidden
                    }
                }
                _ => {
                    out.push(skip(&host, "did not resolve"));
                    continue;
                }
            },
        };
        match place {
            Place::Internet => out.push(skip(&host, "public")),
            // Loopback, link-local: never a destination through the proxy, and a loopback service
            // is opened to the mind only by the person's own rule.
            Place::Forbidden => out.push(skip(&host, "this machine or never a destination")),
            Place::Lan => out.push(Planned::Seed(rule)),
        }
    }
    out
}

/// The system's lookup, for `seed-plan`.
pub fn system_resolve(host: &str, port: u16) -> Option<Vec<IpAddr>> {
    (host, port).to_socket_addrs().ok().map(|a| a.map(|s| s.ip()).collect())
}

/// `yantrik-egress seed-plan`: candidates on stdin, the plan on stdout.
pub fn plan_text(input: &str) -> String {
    let candidates: Vec<(String, String)> = input
        .lines()
        .filter_map(|l| l.split_once(' '))
        .map(|(s, u)| (s.to_string(), u.trim().to_string()))
        .collect();
    let mut out = String::new();
    for p in plan(&candidates, &system_resolve) {
        match p {
            Planned::Seed(r) => {
                out.push_str(&format!("seed {} {} {} {}\n", r.host, r.ports[0], if r.http { "http" } else { "https" }, &r.why["seeded from ".len()..]))
            }
            Planned::Skip { source, host, why } => out.push_str(&format!("skip {source} {} {why}\n", if host.is_empty() { "-" } else { &host })),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(host: &str, _port: u16) -> Option<Vec<IpAddr>> {
        match host {
            "gpu.example.ts.net" => Some(vec!["100.101.102.103".parse().unwrap()]),
            "api.openai.com" => Some(vec!["104.18.7.192".parse().unwrap()]),
            "split.example" => Some(vec!["192.168.1.9".parse().unwrap(), "8.8.8.8".parse().unwrap()]),
            "localhost.example" => Some(vec!["127.0.0.1".parse().unwrap()]),
            _ => None,
        }
    }

    fn plan_of(lines: &[(&str, &str)]) -> Vec<String> {
        let c: Vec<(String, String)> = lines.iter().map(|(s, u)| (s.to_string(), u.to_string())).collect();
        plan(&c, &table)
            .into_iter()
            .map(|p| match p {
                Planned::Seed(r) => format!("seed {} {} {} {}", r.host, r.ports[0], r.http, r.why),
                Planned::Skip { host, why, .. } => format!("skip {host} {why}"),
            })
            .collect()
    }

    #[test]
    fn local_endpoints_are_seeded_and_public_ones_are_not() {
        assert_eq!(
            plan_of(&[
                ("config.yaml", "http://192.168.4.35:11434/v1"),
                ("settings.yaml", "http://homeassistant.local:8123"),
                ("settings.yaml", "http://HomeAssistant.local.:8124"),
                ("mind-person.env:YM_OLLAMA_LOCAL_URL", "http://gpu.example.ts.net:11434"),
                ("providers.yaml", "https://[fd00::5]/v1"),
                ("config.yaml", "https://api.openai.com/v1"),
                ("config.yaml", "http://8.8.8.8:80"),
                ("config.yaml", "http://split.example:80"),
                ("config.yaml", "http://127.0.0.1:8341/v1"),
                ("config.yaml", "http://localhost.example:11434"),
                ("config.yaml", "http://nowhere.example:80"),
                ("config.yaml", "ftp://192.168.4.35/"),
                ("config.yaml", "http://user:pw@192.168.4.35/"),
                ("config.yaml", "http://192.168.4.35:0/"),
                ("config.yaml", "http://searx:8888"),
            ]),
            [
                "seed 192.168.4.35 11434 true seeded from config.yaml",
                "seed homeassistant.local 8123 true seeded from settings.yaml",
                "seed homeassistant.local 8124 true seeded from settings.yaml",
                "seed gpu.example.ts.net 11434 true seeded from mind-person.env:YM_OLLAMA_LOCAL_URL",
                "seed fd00::5 443 false seeded from providers.yaml",
                "skip api.openai.com public",
                "skip 8.8.8.8 public",
                "skip split.example public",
                "skip 127.0.0.1 this machine or never a destination",
                "skip localhost.example this machine or never a destination",
                "skip nowhere.example did not resolve",
                "skip  not an http or https URL",
                "skip  a URL with credentials in it",
                "skip  a port that is not 1-65535",
                "skip searx not a host a rule can hold",
            ]
        );
    }

    #[test]
    fn the_plan_prints_one_line_each() {
        let t = plan_text("config.yaml http://192.168.4.35:11434/v1\nconfig.yaml https://8.8.8.8/\n");
        assert_eq!(t, "seed 192.168.4.35 11434 http config.yaml\nskip config.yaml 8.8.8.8 public\n");
    }
}
