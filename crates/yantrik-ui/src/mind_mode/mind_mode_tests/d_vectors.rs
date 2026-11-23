//! The vectors for the owner's decision D (#1353, 22 November 2026) and its extension to the
//! always-asks sentence (#1357): the taint card and the "the person allows each one on a card"
//! sentence are asked in every mode but Full bypass, where they run unasked and are audited.
//!
//! Written by the same generator as the rest, from the shell's own `taint_ask::resolve` over
//! `Modes::decide`; the bridge (`deploy/yantrik-os/yos-mcp`) is driven through every one of them by
//! `yos-mcp-selftest.py`, so the two copies of the rule cannot move apart silently.

use super::*;

/// One row per mode, grade and (taint, declared) pair, on a command-shaped action (`shell.agent_run`,
/// a sink the taint is defined for) under a ceiling that refuses nothing.
pub(super) fn rows(now: Instant) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    for mode in Mode::ALL {
        for grade in ["safe", "standard", "sensitive", "dangerous"] {
            for (tainted, always) in [(true, false), (false, true), (true, true)] {
                let table = modes_for(mode, &[], now).decide(grade, "shell", "agent_run", Declared::default(), "dangerous", now);
                let purposes = if always { yantrik_ipc_transport::gate::ALWAYS_ASKS } else { "" };
                let flag = serde_json::Value::Bool(tainted);
                let decision = crate::taint_ask::resolve(table, mode == Mode::BypassAll, tainted.then_some(&flag), false, "shell", "agent_run", purposes);
                out.push(serde_json::json!({
                    "id": format!("decision-d/{}/{grade}/tainted={tainted}/always_asks={always}", mode.as_str()),
                    "layer": "shell",
                    "tool": "os_act",
                    "app": "shell",
                    "action": "agent_run",
                    "grade": grade,
                    "mode": mode.as_str(),
                    "ceiling": "dangerous",
                    "rules": Vec::<serde_json::Value>::new(),
                    "env_cap": serde_json::Value::Null,
                    "unrecoverable": false,
                    "open_ended": false,
                    "tainted": tainted,
                    "always_asks": always,
                    "expect": outcome_of(&decision),
                }));
            }
        }
    }
    out
}
