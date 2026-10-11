//! Where a caller was started: desktop, terminal, ssh, timer, service, or unknown.
//!
//! # Why this is beside `peer_identity`
//!
//! [`crate::peer_identity`] names the **program** behind a socket. It does not say **where that
//! program was started**, so a Hermes typed in a terminal, one started by a systemd timer, one
//! started over ssh and one the desktop launched all look alike. This module classifies the
//! origin from the same `/proc` facts the walk already reads, plus a few it does not: the
//! controlling tty, the cgroup unit, and — only when the chain already says ssh — the
//! `SSH_CONNECTION` address out of the process's environment.
//!
//! # What it can and cannot establish
//!
//! Origin comes from facts a program running as the same user can arrange: it can open a pty, or
//! start itself from a unit, or exec a binary called `yantrik-ui`. So this is good evidence, not
//! proof. That is why the UI will show it as metadata and never as a verified identity — the
//! same boundary `peer_identity` records for the program name, and the same-user limit #154
//! records for the shell.

use std::io::Read;
use std::net::IpAddr;

use serde::Serialize;

use crate::peer_identity::{basename, clip, ProcessFacts, Program, StatFacts};

/// Where a caller was started.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Origin {
    Desktop,
    Terminal,
    Ssh { from: Option<IpAddr> },
    Timer { unit: String },
    Service { unit: String },
    Unknown,
}

impl Origin {
    /// One line for a card or a ledger row. A unit name is the caller's to choose, so it is
    /// cleaned and clipped like every other caller-written word.
    pub fn label(&self) -> String {
        match self {
            Origin::Desktop => "desktop".to_string(),
            Origin::Terminal => "terminal".to_string(),
            Origin::Ssh { from: Some(addr) } => format!("ssh from {addr}"),
            Origin::Ssh { from: None } => "ssh".to_string(),
            Origin::Timer { unit } => format!("timer \u{b7} {}", clean_unit(unit)),
            Origin::Service { unit } => format!("service \u{b7} {}", clean_unit(unit)),
            Origin::Unknown => "unknown".to_string(),
        }
    }
}

/// A unit name as one clean, bounded line.
fn clean_unit(unit: &str) -> String {
    clip(&crate::plain_text::one_line(unit), 40)
}

/// The extra `/proc` facts a classification rests on, read for one process.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OriginFacts {
    /// Field 7 of `/proc/<pid>/stat`: the controlling terminal, `0` when there is none.
    pub tty_nr: Option<i64>,
    /// The deepest cgroup path component ending in `.service`, or `None`.
    pub cgroup_unit: Option<String>,
    /// The peer address out of `SSH_CONNECTION`, read only when the chain already says ssh.
    pub ssh_from: Option<IpAddr>,
}

/// Classify a chain that has already been read, against facts that have already been read.
///
/// The rules are applied in order and the first that holds wins. Each one rests on a fact, never
/// a guess.
pub fn classify(
    chain: &[ProcessFacts],
    facts: &OriginFacts,
    timer_exists: impl Fn(&str) -> bool,
) -> Origin {
    // 1. Ssh: the chain names sshd, by binary or by a rewritten argv0.
    if chain.iter().any(is_ssh) {
        return Origin::Ssh { from: facts.ssh_from };
    }
    // 2. Terminal: the person's Terminal app is in the chain.
    if chain.iter().any(|f| basename(&f.exe) == "yantrik-terminal") {
        return Origin::Terminal;
    }
    // 3. Desktop: the shell is in the chain. This covers commands a mind runs through the agent
    // terminal, which have a tty, so it must come before the tty rule.
    if chain.iter().any(|f| basename(&f.exe) == "yantrik-ui") {
        return Origin::Desktop;
    }
    // 4. Terminal: any other emulator, or a VT console.
    if let Some(n) = facts.tty_nr {
        if n != 0 {
            return Origin::Terminal;
        }
    }
    // 5 and 6. A unit, timer before service.
    if let Some(unit) = &facts.cgroup_unit {
        if let Some(stem) = unit.strip_suffix(".service") {
            // A stem that could escape a directory is refused before it is ever asked about.
            if !stem.is_empty() && !stem.contains('/') && !stem.contains("..") && timer_exists(stem) {
                return Origin::Timer { unit: stem.to_string() };
            }
            if !is_session_manager(unit) {
                return Origin::Service { unit: stem.to_string() };
            }
        }
    }
    Origin::Unknown
}

/// Whether one chain entry names sshd: its binary is `sshd` or `sshd-session`, or its argv0 is,
/// with a rewritten title's colon stripped.
fn is_ssh(facts: &ProcessFacts) -> bool {
    let name = basename(&facts.exe);
    if name == "sshd" || name == "sshd-session" {
        return true;
    }
    let argv0 = facts.short_cmdline.split_whitespace().next().unwrap_or("");
    let argv0 = argv0.strip_suffix(':').unwrap_or(argv0);
    argv0 == "sshd" || argv0 == "sshd-session"
}

/// The user's session manager unit, `user@<n>.service`: not a unit anyone started.
fn is_session_manager(unit: &str) -> bool {
    let Some(rest) = unit.strip_prefix("user@") else {
        return false;
    };
    let Some(rest) = rest.strip_suffix(".service") else {
        return false;
    };
    !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit())
}

/// Classify a [`Program`] by reading `/proc` for the one process still likely to be alive.
#[cfg(target_os = "linux")]
pub fn origin_of(program: &Program) -> Origin {
    let Some(pid) = program.recognisable.as_ref().or(program.direct.as_ref()).map(|f| f.pid) else {
        return Origin::Unknown;
    };
    let Some(facts) = read_facts(pid) else {
        return Origin::Unknown;
    };
    let ssh = program.chain.iter().any(is_ssh);
    let ssh_from = if ssh { read_ssh_from(pid) } else { None };
    let origin_facts = OriginFacts {
        tty_nr: Some(facts.tty_nr),
        cgroup_unit: read_cgroup_unit(pid),
        ssh_from,
    };
    classify(&program.chain, &origin_facts, timer_exists)
}

/// No `/proc` to read. Everything on this path is honest about knowing nothing.
#[cfg(not(target_os = "linux"))]
pub fn origin_of(program: &Program) -> Origin {
    let _ = program;
    Origin::Unknown
}

/// One process's stat facts, or `None` if it moved underneath the read.
///
/// The same before-and-after `stat` discipline as `peer_identity::facts`: if the start time
/// moved, the pid was reused and nothing read in between belongs to one process.
#[cfg(target_os = "linux")]
fn read_facts(pid: i32) -> Option<StatFacts> {
    let before = crate::peer_identity::parse_stat(&std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)?;
    let after = crate::peer_identity::parse_stat(&std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)?;
    if before.started != after.started {
        return None;
    }
    Some(before)
}

/// The deepest cgroup v2 path component ending in `.service`, or `None`.
#[cfg(target_os = "linux")]
fn read_cgroup_unit(pid: i32) -> Option<String> {
    let text = std::fs::read_to_string(format!("/proc/{pid}/cgroup")).ok()?;
    let path = text.lines().find_map(|line| line.strip_prefix("0::"))?;
    path.split('/').filter(|c| c.ends_with(".service")).last().map(str::to_string)
}

/// The peer address out of `SSH_CONNECTION`, or `None`.
///
/// Only the first space-separated field of the first `SSH_CONNECTION=` entry is kept; nothing
/// else in the environment is read past, logged or kept — this file holds tokens.
#[cfg(target_os = "linux")]
fn read_ssh_from(pid: i32) -> Option<IpAddr> {
    let mut buf = [0u8; 64 * 1024];
    let n = std::fs::File::open(format!("/proc/{pid}/environ"))
        .ok()?
        .read(&mut buf)
        .ok()?;
    let text = String::from_utf8_lossy(&buf[..n]);
    for entry in text.split('\0') {
        if let Some(value) = entry.strip_prefix("SSH_CONNECTION=") {
            return value.split_whitespace().next()?.parse().ok();
        }
    }
    None
}

/// Whether `<stem>.timer` exists as a file in any of the systemd unit directories.
///
/// A stem that could escape the directory is refused and treated as false.
#[cfg(target_os = "linux")]
fn timer_exists(stem: &str) -> bool {
    if stem.is_empty() || stem.contains('/') || stem.contains("..") {
        return false;
    }
    let file = format!("{stem}.timer");
    for dir in timer_dirs() {
        if dir.join(&file).is_file() {
            return true;
        }
    }
    false
}

/// The systemd unit directories, user ones first.
#[cfg(target_os = "linux")]
fn timer_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        dirs.push(std::path::PathBuf::from(xdg).join("systemd/user"));
    } else if let Some(home) = std::env::var_os("HOME") {
        dirs.push(std::path::PathBuf::from(home).join(".config/systemd/user"));
    }
    for d in [
        "/etc/systemd/user",
        "/usr/lib/systemd/user",
        "/usr/local/lib/systemd/user",
        "/etc/systemd/system",
        "/usr/lib/systemd/system",
        "/lib/systemd/system",
        "/run/systemd/system",
        "/run/systemd/transient",
    ] {
        dirs.push(std::path::PathBuf::from(d));
    }
    // SAFETY: geteuid cannot fail and touches no memory.
    let uid = unsafe { libc::geteuid() };
    dirs.push(std::path::PathBuf::from(format!("/run/user/{uid}/systemd/transient")));
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(pid: i32, exe: &str, cmdline: &str) -> ProcessFacts {
        ProcessFacts {
            pid,
            exe: exe.to_string(),
            short_cmdline: cmdline.to_string(),
            started: pid as u64 * 100,
        }
    }

    fn no_tty() -> OriginFacts {
        OriginFacts { tty_nr: None, cgroup_unit: None, ssh_from: None }
    }

    fn tty(n: i64) -> OriginFacts {
        OriginFacts { tty_nr: Some(n), cgroup_unit: None, ssh_from: None }
    }

    fn unit(name: &str) -> OriginFacts {
        OriginFacts { tty_nr: None, cgroup_unit: Some(name.to_string()), ssh_from: None }
    }

    fn never_timer(_stem: &str) -> bool {
        false
    }

    #[test]
    fn hermes_over_ssh_is_ssh_with_the_peer_address() {
        let chain = vec![
            facts(1, "/usr/bin/python3.11", "python3 yos act shell x"),
            facts(2, "/usr/bin/python3.11", "python3 yos-mcp"),
            facts(3, "/usr/bin/python3", "python -m hermes_cli"),
            facts(4, "/usr/bin/bash", "bash"),
            facts(5, "", "sshd-session: yantrik@pts/0"),
        ];
        let facts = OriginFacts {
            tty_nr: Some(34816),
            cgroup_unit: None,
            ssh_from: Some("192.168.4.12".parse().unwrap()),
        };
        let origin = classify(&chain, &facts, never_timer);
        assert_eq!(origin, Origin::Ssh { from: Some("192.168.4.12".parse().unwrap()) });
        assert_eq!(origin.label(), "ssh from 192.168.4.12");
    }

    #[test]
    fn ssh_with_a_readable_exe_and_no_address_is_still_ssh() {
        let chain = vec![
            facts(1, "/usr/bin/python3.11", "python3 yos act shell x"),
            facts(2, "/usr/bin/python3.11", "python3 yos-mcp"),
            facts(3, "/usr/bin/python3", "python -m hermes_cli"),
            facts(4, "/usr/bin/bash", "bash"),
            facts(5, "/usr/lib/openssh/sshd-session", "sshd-session: yantrik@pts/0"),
        ];
        let origin = classify(&chain, &tty(34816), never_timer);
        assert_eq!(origin, Origin::Ssh { from: None });
        assert_eq!(origin.label(), "ssh");
    }

    #[test]
    fn hermes_typed_in_the_terminal_app_is_terminal() {
        let chain = vec![
            facts(1, "/usr/bin/python3.11", "python3 yos act shell x"),
            facts(2, "/usr/bin/python3.11", "python3 yos-mcp"),
            facts(3, "/usr/bin/python3", "python -m hermes_cli"),
            facts(4, "/usr/bin/bash", "bash"),
            facts(5, "/usr/bin/yantrik-terminal", "yantrik-terminal"),
            facts(6, "/usr/bin/yantrik-ui", "yantrik-ui"),
        ];
        let origin = classify(&chain, &tty(34817), never_timer);
        assert_eq!(origin, Origin::Terminal);
    }

    #[test]
    fn a_minds_command_through_the_agent_terminal_is_desktop() {
        let chain = vec![
            facts(1, "/usr/bin/ls", "ls"),
            facts(2, "/usr/bin/bash", "bash"),
            facts(3, "/usr/bin/yantrik-ui", "yantrik-ui"),
        ];
        let origin = classify(&chain, &tty(34818), never_timer);
        assert_eq!(origin, Origin::Desktop);
    }

    #[test]
    fn a_harness_unit_the_desktop_started_is_a_service() {
        let chain = vec![facts(1, "/usr/bin/python3", "python -m hermes_cli gateway run")];
        let origin = classify(&chain, &unit("hermes-gateway.service"), never_timer);
        assert_eq!(origin, Origin::Service { unit: "hermes-gateway".to_string() });
        assert_eq!(origin.label(), "service \u{b7} hermes-gateway");
    }

    #[test]
    fn a_timer_job_is_a_timer() {
        let chain = vec![facts(1, "/usr/bin/python3", "python3 nightly.py")];
        let origin = classify(&chain, &unit("nightly-sync.service"), |stem| stem == "nightly-sync");
        assert_eq!(origin, Origin::Timer { unit: "nightly-sync".to_string() });
        assert_eq!(origin.label(), "timer \u{b7} nightly-sync");
    }

    #[test]
    fn the_same_unit_without_a_timer_is_a_service() {
        let chain = vec![facts(1, "/usr/bin/python3", "python3 nightly.py")];
        let origin = classify(&chain, &unit("nightly-sync.service"), never_timer);
        assert_eq!(origin, Origin::Service { unit: "nightly-sync".to_string() });
    }

    #[test]
    fn the_session_manager_unit_is_unknown() {
        let chain = vec![facts(1, "/usr/bin/bash", "bash")];
        let origin = classify(&chain, &unit("user@1000.service"), never_timer);
        assert_eq!(origin, Origin::Unknown);
    }

    #[test]
    fn an_empty_chain_is_unknown() {
        assert_eq!(classify(&[], &no_tty(), never_timer), Origin::Unknown);
    }

    #[test]
    fn a_unit_name_is_one_clean_line_in_the_label() {
        let origin = Origin::Service { unit: "foo\n\u{202E}bar".to_string() };
        let label = origin.label();
        assert!(!label.contains('\n'), "{label:?}");
        assert!(!label.chars().any(|c| c.is_control() || crate::plain_text::is_bidi_control(c)), "{label:?}");
    }

    #[test]
    fn timer_exists_is_never_asked_about_a_stem_with_a_slash_or_dotdot() {
        let chain = vec![facts(1, "/usr/bin/python3", "python3 x.py")];
        let asked = std::cell::RefCell::new(Vec::new());
        let probe = |stem: &str| {
            asked.borrow_mut().push(stem.to_string());
            false
        };
        classify(&chain, &unit("app.slice/../evil.service"), &probe);
        classify(&chain, &unit("app.slice/a/b.service"), &probe);
        assert!(asked.borrow().is_empty(), "a stem with / or .. must be refused before the closure: {:?}", asked.borrow());
    }
}

