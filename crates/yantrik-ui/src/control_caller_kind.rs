//! `caller_kind`: whether a process is a mind's, by the shell's one rule (#1085).
//!
//! A surface that serves the person and the minds differently (the browser's: a Mind may not drive
//! the person's signed-in Chromium) has the caller's pid from `SO_PEERCRED` but not the list of
//! minds attached to this desktop, nor how the desktop names one. Both are the shell's. Hermes,
//! OpenClaw, Pi and DeepSeek run as the person and Hermes's bridge carries no agent token, so
//! neither the uid nor the token says what they are; the process walk does (`caller_identity`).
//! This is that walk, asked about a pid, so the rule is not written a second time anywhere.
//!
//! The answer is a mind when the ancestry belongs to an attached mind (by its recorded pid, or by
//! a word of its program's name: the wide match that classifies a caller), when it runs under
//! `yos-mcp`, or when it is this process. `known` is false when `/proc` gave no ancestry for the
//! pid; the asker must read that as not the person.

use yantrik_app_runtime::control::{Action, App as ControlSurface, Param};
use yantrik_ipc_transport::peer_identity::{basename, ProcessFacts};

use crate::caller_identity::{self, CallerIdentity};

pub fn actions(surface: ControlSurface) -> ControlSurface {
    surface.action(
        Action::new(
            "caller_kind",
            "Whether the process `pid` is a mind's (an attached mind's, or run under yos-mcp) by the process walk the desktop uses to name a caller. `mind` true or false; `known` false when its ancestry could not be read",
        )
        .risk("safe")
        .arg(Param::text("pid").describe("The process id, as the kernel reported it for a socket peer")),
        |args| {
            let pid = args["pid"].as_str().map(str::to_string).or_else(|| args["pid"].as_i64().map(|n| n.to_string()));
            let Some(pid) = pid.and_then(|p| p.trim().parse::<i32>().ok()).filter(|p| *p > 0) else {
                return Err("`pid` is a process id".to_string());
            };
            if pid as u32 == std::process::id() {
                return Ok(serde_json::json!({ "mind": true, "known": true, "who": "the companion" }));
            }
            Ok(verdict(&caller_identity::resolve(pid)))
        },
    )
}

/// Whether this ancestry is a mind's.
fn verdict(identity: &CallerIdentity) -> serde_json::Value {
    if identity.chain.is_empty() {
        return serde_json::json!({ "mind": false, "known": false });
    }
    let under_bridge = identity.chain.iter().any(runs_yos_mcp);
    let mind = identity.attached_mind.clone();
    serde_json::json!({ "mind": under_bridge || mind.is_some(), "known": true, "who": mind })
}

/// `yos-mcp` is the bridge every harness's tools go through, whatever the harness is called.
fn runs_yos_mcp(facts: &ProcessFacts) -> bool {
    basename(&facts.exe) == "yos-mcp"
        || facts.short_cmdline.split_whitespace().take(2).any(|token| basename(token) == "yos-mcp")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::caller_identity::{identify, Mind};

    fn facts(pid: i32, exe: &str, cmdline: &str) -> ProcessFacts {
        ProcessFacts { pid, exe: exe.to_string(), short_cmdline: cmdline.to_string(), started: pid as u64 * 100 }
    }

    fn hermes() -> Vec<Mind> {
        vec![Mind { id: "hermes".into(), name: "Hermes Agent".into(), pid: None }]
    }

    #[test]
    fn a_yos_started_by_a_harness_terminal_tool_is_a_mind() {
        let chain = vec![
            facts(30, "/usr/bin/python3", "python3 yos act browser go"),
            facts(20, "/bin/bash", "bash -c yos act browser go"),
            facts(10, "/home/p/hermes-agent/venv/bin/python", "python -m hermes_cli.main gateway run"),
            facts(1, "/usr/lib/systemd/systemd", "systemd --user"),
        ];
        let v = verdict(&identify(chain, &hermes()));
        assert_eq!(v["mind"], true);
        assert_eq!(v["who"], "Hermes Agent");
    }

    #[test]
    fn anything_under_yos_mcp_is_a_mind_even_when_no_mind_is_named() {
        let chain = vec![
            facts(30, "/usr/bin/python3", "python3 yos act browser go"),
            facts(20, "/usr/bin/python3", "python3 yos-mcp"),
            facts(1, "/usr/lib/systemd/systemd", "systemd --user"),
        ];
        assert_eq!(verdict(&identify(chain, &[]))["mind"], true);
    }

    #[test]
    fn the_persons_own_terminal_is_not_a_mind() {
        let chain = vec![
            facts(30, "/usr/bin/python3", "python3 yos act browser go"),
            facts(20, "/bin/bash", "-bash"),
            facts(15, "/usr/bin/foot", "foot"),
            facts(10, "/opt/yantrik/bin/yantrik-ui", "yantrik-ui"),
        ];
        let v = verdict(&identify(chain, &hermes()));
        assert_eq!(v["mind"], false);
        assert_eq!(v["known"], true);
    }

    #[test]
    fn an_ancestry_that_could_not_be_read_is_not_known() {
        let v = verdict(&identify(Vec::new(), &hermes()));
        assert_eq!((v["mind"].clone(), v["known"].clone()), (false.into(), false.into()));
    }

    #[test]
    fn the_action_is_safe_and_takes_one_pid() {
        let code = include_str!("control_caller_kind.rs").split("#[cfg(test)]").next().unwrap();
        assert_eq!(code.matches(".risk(\"safe\")").count(), 1);
        assert_eq!(code.matches("Param::").count(), 1);
    }
}
