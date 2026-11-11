//! Which SSH hosts the person has: the ones they wrote in `~/.ssh/config` and the ones they have been to.
//!
//! `ssh_check_host` is a service of the person's only for those (`reach`). For any other name it is a
//! probe of someone else's address, which the model could aim anywhere, and the lookup alone can carry
//! text in a label. `ssh_list_hosts` shows the same two files; the tools keep their own display, this
//! only answers "is this one the person's".

use std::path::PathBuf;

fn ssh_file(name: &str) -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/root"));
    home.join(".ssh").join(name)
}

/// Whether `host` is a `Host` alias or `HostName` in the person's SSH config, or an unhashed entry of
/// their `known_hosts`.
pub fn is_known(host: &str) -> bool {
    let read = |name: &str| std::fs::read_to_string(ssh_file(name)).unwrap_or_default();
    known_in(host, &read("config"), &read("known_hosts"))
}

/// [`is_known`], for the text of the two files. A pattern (`*`, `?`, `!`) is not a host, and a hashed
/// `known_hosts` entry (`|1|…`) cannot be read back, so neither counts. Names compare without case.
pub fn known_in(host: &str, config: &str, known_hosts: &str) -> bool {
    let host = host.trim().to_ascii_lowercase();
    if host.is_empty() {
        return false;
    }
    let is_host = |name: &str| !name.contains(['*', '?', '!']) && name.to_ascii_lowercase() == host;
    let configured = config.lines().map(str::trim).filter(|l| !l.starts_with('#')).any(|line| {
        let mut words = line.split_whitespace();
        let key = words.next().unwrap_or("").trim_end_matches('=').to_ascii_lowercase();
        matches!(key.as_str(), "host" | "hostname") && words.map(|w| w.trim_start_matches('=')).any(is_host)
    });
    configured
        || known_hosts.lines().map(str::trim).filter(|l| !l.starts_with('#')).any(|line| {
            let names = line.split_whitespace().next().unwrap_or("");
            !names.starts_with('|')
                && names.split(',').any(|n| is_host(n.trim_start_matches('[').split(']').next().unwrap_or(n)))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = "# my machines\nHost nas box\n  HostName 192.168.4.28\n  User me\nHost *.corp !jump\n  User work\nHost *\n  ServerAliveInterval 30\n";
    const KNOWN: &str = "github.com,140.82.1.1 ssh-ed25519 AAAA\n[lab.example]:2222 ssh-rsa BBBB\n|1|abc=|def= ssh-ed25519 CCCC\n";

    #[test]
    fn a_host_the_person_wrote_or_has_been_to_is_theirs() {
        for host in ["nas", "BOX", "192.168.4.28", "github.com", "140.82.1.1", "lab.example"] {
            assert!(known_in(host, CONFIG, KNOWN), "{host}");
        }
    }

    #[test]
    fn any_other_name_is_a_probe_of_someone_elses_address() {
        for host in ["", "  ", "example.org", "jump", "x.corp", "*", "*.corp", "def=", "a.attacker-label.example"] {
            assert!(!known_in(host, CONFIG, KNOWN), "{host:?}");
        }
        assert!(!known_in("nas", "", ""), "no files, no hosts");
    }
}
