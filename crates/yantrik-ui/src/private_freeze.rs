//! What Private mode stops outright: the agents that run as the person.
//!
//! The Mind runs as an account of its own, boxed away from the person's files, and the door shuts
//! on it while Private mode is on. The harnesses (pi, openclaw, deepseek, and Hermes inside its
//! gateway) run as the person: with their own shell and file tools they could read anything the
//! person can, whatever the door says. So their units are frozen — every process in them stopped
//! where it stands by the cgroup freezer, nothing lost — and thawed when Private mode ends. A
//! turn in flight stops mid-thought and goes on afterwards; a command half-run waits.
//!
//! Only units the desktop knows as agents: the harness manifests' units and Hermes' gateway.
//! A harness the person started by hand in a terminal is not the desktop's to freeze; the
//! harness socket refuses it instead.

use crate::harness_catalogue::{read_manifests, read_units, roots};

/// Units that run an agent but have no manifest of ours: Hermes is a plugin inside its gateway.
const OTHER_AGENT_UNITS: &[&str] = &["hermes-gateway.service"];

/// Every user unit that runs one of the person's agents.
fn agent_units() -> Vec<String> {
    let mut units: Vec<String> = read_manifests(&roots())
        .into_values()
        .map(|m| m.unit)
        .filter(|u| !u.trim().is_empty())
        .collect();
    units.extend(OTHER_AGENT_UNITS.iter().map(|u| u.to_string()));
    units.sort();
    units.dedup();
    units
}

fn systemctl(verb: &str, units: &[String]) -> Result<(), String> {
    if units.is_empty() {
        return Ok(());
    }
    let out = std::process::Command::new("systemctl")
        .arg("--user")
        .arg(verb)
        .args(units)
        .output()
        .map_err(|e| format!("systemctl could not be run: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Freeze every agent unit that is running. Returns the ones frozen, or why some were not.
pub fn freeze() -> Result<Vec<String>, String> {
    let units = agent_units();
    let running: Vec<String> = read_units(&units).into_iter().filter(|(_, u)| u.active).map(|(id, _)| id).collect();
    // One at a time, so one that cannot be frozen does not leave the others running.
    let mut failed = Vec::new();
    let mut frozen = Vec::new();
    for unit in running {
        match systemctl("freeze", std::slice::from_ref(&unit)) {
            Ok(()) => frozen.push(unit),
            Err(why) => failed.push(format!("{unit}: {why}")),
        }
    }
    if failed.is_empty() {
        Ok(frozen)
    } else {
        Err(failed.join("; "))
    }
}

/// Thaw every agent unit. Thawing one that is not frozen does nothing, so all of them are asked,
/// including any frozen by an earlier run of the shell.
pub fn thaw() {
    for unit in agent_units() {
        let _ = systemctl("thaw", std::slice::from_ref(&unit));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hermes_gateway_is_among_the_agent_units() {
        assert!(agent_units().iter().any(|u| u == "hermes-gateway.service"));
    }
}
