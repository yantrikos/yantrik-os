//! The direct set: where the mind account may connect without this proxy.
//!
//! The kernel holds the mind account to its way out (`yantrik-update mind-egress apply`, the
//! table `inet yantrik_mind_egress`): this proxy and the memory server on loopback; DNS to the
//! machine's resolvers in audit only; and these entries. Everything else from the account is
//! refused, whatever the program does with its proxy variables.
//!
//! The entries come only from the person's policy, which only root and the desktop's owner can
//! change (the control socket), never from anything the mind account can write. An entry is a
//! rule that says `lan` and whose host is a literal address, one entry per port:
//! - on the local network: a **direct** entry, a LAN service the person allowed by address, which
//!   a program that ignores proxy variables may then reach;
//! - 127.0.0.1 or ::1: a **loopback** entry, a service on this machine opened to the mind (a local
//!   Ollama on 11434).
//!
//! A name never makes an entry; it keeps going through this proxy, which decides on the address it
//! resolves to. Neither does a public address: the internet is reached through the proxy, where it
//! is counted. In Private mode there are none.
//!
//! `yantrik-egress direct [STATE_DIR]` prints them, run by the updater as this proxy's own account
//! (so the policy is read by the code that enforces it, never parsed as root):
//!
//! ```text
//! private off
//! mode enforce
//! direct 192.168.4.42 8888
//! direct fd00::5 443
//! loopback 127.0.0.1 11434
//! ```
//!
//! A policy that does not read is an error (exit 1), and the updater loads an empty set for it.
//! The updater checks every line again as root, and caps the set.
//!
//! `yantrik-egress lan-hosts [STATE_DIR]` prints, the same way, every host a `lan` rule names —
//! names, `*.domain` and literal addresses, seeded or the person's — with its ports, for the
//! status file a mind reads (`lan_hosts` in /run/yantrik-mind-egress/mind-egress.json). The
//! proxy grants the local network on the name asked for, so a mind fetching for an untrusted
//! caller must refuse these hosts itself. Private mode does not change it: it lists the policy.
//!
//! ```text
//! lan homeassistant.local 8123
//! lan 192.168.4.42 8080,8888
//! ```

use std::net::IpAddr;
use std::path::Path;

use crate::policy::Policy;

/// The literal address a rule's host names, an IPv4 address written as IPv6 (`::ffff:a.b.c.d`)
/// taken as the IPv4 one, which is what the kernel sees on the wire.
fn literal(host: &str) -> Option<IpAddr> {
    Some(match host.parse::<IpAddr>().ok()? {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(IpAddr::V6(v6)),
        v4 => v4,
    })
}

/// The ranges a direct entry may be in: the local network (10/8, 172.16/12, 192.168/16, and
/// 100.64/10, which is CGNAT and Tailscale's) and unique-local IPv6 (fc00::/7). Only these: an
/// address that is "local" only because of the IPv4 address carried inside it (NAT64, 6to4) leaves
/// this machine as a packet to a router. The updater accepts exactly these again, as root
/// (PY_MIND_DIRECT in deploy/yantrik-os/yantrik-update); its selftest holds the two lists equal,
/// so keep this one on one line.
pub const DIRECT_RANGES: [&str; 5] = ["10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16", "100.64.0.0/10", "fc00::/7"];

fn direct_address(ip: IpAddr) -> bool {
    crate::ranges::in_cidrs(ip, &DIRECT_RANGES)
}

/// What a rule makes in the kernel's table, if anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Entry {
    /// A LAN service the person allowed by address, reached without the proxy.
    Direct(IpAddr, u16),
    /// A service on this machine's loopback (127.0.0.1 or ::1 exactly) the person opened to the
    /// mind, beyond the ones it always has (the proxy, the memory server): a local Ollama on 11434
    /// is the usual one. Without a rule, a local model server would be a way out past Private mode
    /// and enforce (Ollama pulls and pushes models by name, to any registry).
    Loopback(IpAddr, u16),
}

/// Every entry the policy makes, sorted, each once: rules that say `lan` and name a literal
/// address on the local network (direct) or 127.0.0.1 / ::1 (loopback), one per port.
pub fn entries(policy: &Policy) -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::new();
    for r in policy.rules.iter().filter(|r| r.lan) {
        let Some(ip) = literal(&r.host) else { continue };
        let make: fn(IpAddr, u16) -> Entry = if direct_address(ip) {
            Entry::Direct
        } else if ip == IpAddr::from([127, 0, 0, 1]) || ip == IpAddr::from(std::net::Ipv6Addr::LOCALHOST) {
            Entry::Loopback
        } else {
            continue;
        };
        out.extend(r.ports.iter().filter(|p| **p != 0).map(|p| make(ip, *p)));
    }
    out.sort();
    out.dedup();
    out
}

/// What `yantrik-egress direct` prints for the state in `dir`: Private mode, the policy's mode
/// (the kernel lets the mind ask DNS only in audit; guarded and enforce resolve in the proxy), then the entries, none in Private mode.
pub fn export(dir: &Path) -> Result<String, String> {
    let policy = Policy::read(&dir.join("policy.yaml"))?;
    Ok(direct_text(&policy, crate::state::private_at(dir)))
}

fn direct_text(policy: &Policy, private: bool) -> String {
    let mode = policy.mode.word();
    if private {
        return format!("private on\nmode {mode}\n");
    }
    let mut out = format!("private off\nmode {mode}\n");
    for e in entries(policy) {
        match e {
            Entry::Direct(ip, port) => out.push_str(&format!("direct {ip} {port}\n")),
            Entry::Loopback(ip, port) => out.push_str(&format!("loopback {ip} {port}\n")),
        }
    }
    out
}

/// Every host a `lan` rule names, lowercased and without a trailing dot, each once with all its
/// ports, sorted: names and `*.domain` as written, literal addresses as written.
pub fn lan_hosts(policy: &Policy) -> Vec<(String, Vec<u16>)> {
    let mut out: std::collections::BTreeMap<String, Vec<u16>> = std::collections::BTreeMap::new();
    for r in policy.rules.iter().filter(|r| r.lan) {
        let host = r.host.trim_end_matches('.').to_ascii_lowercase();
        if host.is_empty() {
            continue;
        }
        out.entry(host).or_default().extend(r.ports.iter().filter(|p| **p != 0));
    }
    out.into_iter()
        .filter_map(|(h, mut ports)| {
            ports.sort();
            ports.dedup();
            (!ports.is_empty()).then_some((h, ports))
        })
        .collect()
}

/// What `yantrik-egress lan-hosts` prints for the state in `dir`: `lan <host> <port>[,<port>…]`
/// a line. A policy that does not read is an error, as for [`export`].
pub fn export_lan_hosts(dir: &Path) -> Result<String, String> {
    let policy = Policy::read(&dir.join("policy.yaml"))?;
    Ok(lan_hosts_text(&policy))
}

fn lan_hosts_text(policy: &Policy) -> String {
    let mut out = String::new();
    for (host, ports) in lan_hosts(policy) {
        let ports: Vec<String> = ports.iter().map(u16::to_string).collect();
        out.push_str(&format!("lan {host} {}\n", ports.join(",")));
    }
    out
}

/// What `yantrik-egress snapshot` prints: [`export`] then [`export_lan_hosts`], both from one read
/// of the policy, so the table and the status file's `lan_hosts` describe the same policy.
pub fn export_snapshot(dir: &Path) -> Result<String, String> {
    let policy = Policy::read(&dir.join("policy.yaml"))?;
    Ok(direct_text(&policy, crate::state::private_at(dir)) + &lan_hosts_text(&policy))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::{Mode, Rule};

    fn rule(host: &str, ports: &[u16], lan: bool) -> Rule {
        Rule { host: host.into(), ports: ports.to_vec(), http: true, lan, why: "because".into(), seeded: false }
    }

    #[test]
    fn only_lan_rules_naming_a_local_address_make_entries() {
        let p = Policy {
            mode: Mode::Audit,
            rules: vec![
                rule("192.168.4.42", &[8888, 8080], true),
                rule("fd00::5", &[443], true),
                rule("::ffff:10.0.0.7", &[11434], true),
                // A name: through the proxy, which classifies what it resolves to.
                rule("searx.lan", &[8888], true),
                rule("*.home.arpa", &[443], true),
                // Not `lan`: the person did not say the local network.
                rule("192.168.4.43", &[22], false),
                // Public, other loopback, link-local, NAT64, Teredo: never an entry.
                rule("1.1.1.1", &[443], true),
                rule("127.0.0.2", &[7440], true),
                rule("169.254.169.254", &[80], true),
                rule("64:ff9b::c0a8:414", &[80], true),
                rule("2001:0:4136:e378::1", &[80], true),
                rule("2606:4700::1111", &[443], true),
                // The same entry twice is one entry.
                rule("192.168.4.42", &[8888], true),
                // Loopback, exactly 127.0.0.1 or ::1, and only with lan.
                rule("127.0.0.1", &[11434], true),
                rule("::1", &[8341], true),
                rule("127.0.0.1", &[22], false),
            ],
        };
        let got: Vec<String> = entries(&p)
            .iter()
            .map(|e| match e {
                Entry::Direct(ip, port) => format!("direct {ip} {port}"),
                Entry::Loopback(ip, port) => format!("loopback {ip} {port}"),
            })
            .collect();
        assert_eq!(
            got,
            [
                "direct 10.0.0.7 11434",
                "direct 192.168.4.42 8080",
                "direct 192.168.4.42 8888",
                "direct fd00::5 443",
                "loopback 127.0.0.1 11434",
                "loopback ::1 8341"
            ]
        );
    }

    #[test]
    fn the_export_says_private_mode_and_refuses_a_policy_that_does_not_read() {
        let d = std::env::temp_dir().join(format!("yantrik-egress-direct-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        assert_eq!(export(&d).unwrap(), "private off\nmode audit\n", "no policy: a new machine audits, nothing direct");

        std::fs::write(
            d.join("policy.yaml"),
            "mode: enforce\nrules:\n  - host: 192.168.4.42\n    ports: [8888]\n    http: true\n    lan: true\n    why: SearXNG\n  - host: api.x.ai\n    ports: [443]\n    why: the model\n  - host: 127.0.0.1\n    ports: [11434]\n    lan: true\n    why: Ollama here\n",
        )
        .unwrap();
        assert_eq!(export(&d).unwrap(), "private off\nmode enforce\ndirect 192.168.4.42 8888\nloopback 127.0.0.1 11434\n");

        std::fs::write(d.join("private"), "on\n").unwrap();
        assert_eq!(export(&d).unwrap(), "private on\nmode enforce\n", "Private mode: no entries");
        std::fs::remove_file(d.join("private")).unwrap();

        std::fs::write(d.join("policy.yaml"), "mode: guarded\nrules:\n  - host: 192.168.4.42\n    ports: [8888]\n    lan: true\n    why: SearXNG\n").unwrap();
        assert_eq!(export(&d).unwrap(), "private off\nmode guarded\ndirect 192.168.4.42 8888\n", "guarded: its word, and the same entries");
        std::fs::write(d.join("private"), "on\n").unwrap();
        assert_eq!(export(&d).unwrap(), "private on\nmode guarded\n");
        std::fs::remove_file(d.join("private")).unwrap();

        std::fs::write(d.join("policy.yaml"), "mode: enforce\nrules:\n  - host: 192.168.4.42\n    ports: [0]\n    lan: true\n    why: x\n").unwrap();
        assert!(export(&d).is_err(), "a rule the person could not have written: no set at all");
        std::fs::write(d.join("policy.yaml"), "mode: enforce\nrules: [\n").unwrap();
        assert!(export(&d).is_err(), "not a policy");
        std::fs::write(d.join("policy.yaml"), b"mode: audit\n\xff\n").unwrap();
        assert!(export(&d).is_err(), "not text");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn every_lan_rule_is_a_lan_host_names_included() {
        let seeded = |host: &str, ports: &[u16]| Rule { seeded: true, ..rule(host, ports, true) };
        let p = Policy {
            mode: Mode::Audit,
            rules: vec![
                seeded("homeassistant.local", &[8123]),
                seeded("gpu.example.ts.net", &[11434]),
                rule("searx.lan.", &[8888], true),
                rule("*.home.arpa", &[443], true),
                rule("192.168.4.42", &[8888, 8080], true),
                rule("192.168.4.42", &[8888], true),
                rule("fd00::5", &[443], true),
                rule("127.0.0.1", &[11434], true),
                // Not `lan`: not a LAN host.
                rule("api.x.ai", &[443], false),
                rule("192.168.4.43", &[22], false),
            ],
        };
        let got: Vec<String> = lan_hosts(&p).iter().map(|(h, ports)| format!("{h} {ports:?}")).collect();
        assert_eq!(
            got,
            [
                "*.home.arpa [443]",
                "127.0.0.1 [11434]",
                "192.168.4.42 [8080, 8888]",
                "fd00::5 [443]",
                "gpu.example.ts.net [11434]",
                "homeassistant.local [8123]",
                "searx.lan [8888]"
            ]
        );
    }

    #[test]
    fn the_lan_hosts_export_reads_the_policy_and_refuses_one_that_does_not_read() {
        let d = std::env::temp_dir().join(format!("yantrik-egress-lan-hosts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        assert_eq!(export_lan_hosts(&d).unwrap(), "", "no policy: no lan hosts");
        std::fs::write(
            d.join("policy.yaml"),
            "mode: enforce\nrules:\n  - host: homeassistant.local\n    ports: [8123]\n    http: true\n    lan: true\n    why: seeded from mind-person.env\n    seeded: true\n  - host: 192.168.4.42\n    ports: [8888, 8080]\n    lan: true\n    why: SearXNG\n  - host: api.x.ai\n    ports: [443]\n    why: the model\n",
        )
        .unwrap();
        assert_eq!(export_lan_hosts(&d).unwrap(), "lan 192.168.4.42 8080,8888\nlan homeassistant.local 8123\n");
        std::fs::write(d.join("private"), "on\n").unwrap();
        assert_eq!(export_lan_hosts(&d).unwrap(), "lan 192.168.4.42 8080,8888\nlan homeassistant.local 8123\n", "Private mode: still the policy's");
        std::fs::write(d.join("policy.yaml"), "mode: enforce\nrules: [\n").unwrap();
        assert!(export_lan_hosts(&d).is_err(), "not a policy");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_snapshot_is_both_exports_from_one_read() {
        let d = std::env::temp_dir().join(format!("yantrik-egress-snapshot-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        assert_eq!(export_snapshot(&d).unwrap(), "private off\nmode audit\n", "no policy: audit, nothing direct, no lan hosts");
        std::fs::write(
            d.join("policy.yaml"),
            "mode: enforce\nrules:\n  - host: homeassistant.local\n    ports: [8123]\n    lan: true\n    why: HA\n  - host: 192.168.4.42\n    ports: [8888]\n    http: true\n    lan: true\n    why: SearXNG\n",
        )
        .unwrap();
        let both = export(&d).unwrap() + &export_lan_hosts(&d).unwrap();
        assert_eq!(export_snapshot(&d).unwrap(), both);
        assert_eq!(both, "private off\nmode enforce\ndirect 192.168.4.42 8888\nlan 192.168.4.42 8888\nlan homeassistant.local 8123\n");
        std::fs::write(d.join("private"), "on\n").unwrap();
        assert_eq!(
            export_snapshot(&d).unwrap(),
            "private on\nmode enforce\nlan 192.168.4.42 8888\nlan homeassistant.local 8123\n",
            "Private mode: no entries, the lan hosts still the policy's"
        );
        std::fs::write(d.join("policy.yaml"), "mode: enforce\nrules: [\n").unwrap();
        assert!(export_snapshot(&d).is_err(), "not a policy");
        let _ = std::fs::remove_dir_all(&d);
    }
}
