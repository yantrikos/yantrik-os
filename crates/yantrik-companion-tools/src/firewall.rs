//! Firewall tools — reading only.
//!
//! # The five that are gone
//!
//! This file used to register seven tools. Five of them changed the packet filter:
//! `firewall_allow_port`, `firewall_block_port`, `firewall_block_ip`, `firewall_enable` and
//! `firewall_disable`. They shelled `nft` and `iptables` directly. They are removed, and this is
//! why.
//!
//! **There is no scoped privileged path on this machine.** `nft` and `iptables` need
//! `CAP_NET_ADMIN`; a desktop session does not have it. The repository has no polkit action for
//! packet filtering, no setuid helper and no scoped sudoers entry — the only thing that exists is
//! `yantrik ALL=(ALL) NOPASSWD: ALL`, written by both build recipes, which is a way to become
//! root for *anything* and not a way to be allowed this one thing. So each of these tools had two
//! possible fates and both are bad: on a machine without that sudoers file it failed, and on the
//! machines `deploy/` builds it would succeed as an unscoped root change made from a tool call.
//!
//! **The app already decided this.** `design/network-2026-09-20.md` removed the identical four
//! controls from the Network Manager app (`toggle_firewall`, `firewall_allow_port`,
//! `firewall_block_port`, `firewall_apply_profile`) for exactly this reason, and said the
//! asymmetry with the companion "should be settled deliberately: either there is a privileged
//! path and the app may have it back, or there is not and the mind should not have it either."
//! There is not. This is that decision, recorded in the same file under "The mind's own network
//! tools".
//!
//! **Two of them could lock the owner out of the machine.** `firewall_enable` set
//! `iptables -P INPUT DROP` and then added the ACCEPT rules one at a time, reporting "Partially
//! enabled with N errors" if any of them failed — a window in which the policy is DROP and the
//! rule that lets ssh back in is not there yet. `firewall_disable` ran `iptables -F`, flushing
//! every rule on the machine including any this OS did not write, under a sentence that called it
//! "disabled". Neither re-read anything.
//!
//! **What would bring them back:** a polkit action and a small helper with a fixed vocabulary —
//! allow port N, block port N, apply named profile — shipped in the ISO, with the rule scoped to
//! the desktop session, and a method on `network-service` in front of it so there is one owner
//! for it. Until then the honest surface is the one below: read it, do not touch it.
//!
//! # The two that remain
//!
//! `firewall_status` and `firewall_list_rules` are callers of `network.firewall` now. The reading
//! is unprivileged on purpose — an unprivileged `nft list ruleset` fails, and the service reports
//! that as `unknown` with the tool's own refusal, rather than quietly becoming root to draw a
//! status line. The old counting is gone with them: it counted lines beginning with one of six
//! words (`meta`, `tcp`, `udp`, `ip`, `ct`, `iif`), missing `oifname`, `icmp`, `jump`, `log` and
//! `counter`, and called the result "~N rules". The service tracks brace depth instead, and a
//! ruleset it could not read has no number at all rather than a zero.

use std::sync::Arc;

use super::{PermissionLevel, Tool, ToolContext, ToolRegistry};

use crate::networking::backend::{NetworkBackend, ServiceNetwork};
use yantrik_ipc_contracts::network::{FirewallState, FirewallStatus};

pub fn register(reg: &mut ToolRegistry) {
    register_with(reg, Arc::new(ServiceNetwork));
}

/// Register against a given backend. The tests use it; `register` is the machine's own.
pub fn register_with(reg: &mut ToolRegistry, net: Arc<dyn NetworkBackend>) {
    reg.register(Box::new(FirewallStatusTool { net: net.clone() }));
    reg.register(Box::new(FirewallListRulesTool { net }));
}

fn refused(what: &str, why: &str) -> String {
    format!("Could not {what}: {why}")
}

/// The firewall in one paragraph, with four possible answers and not two.
///
/// `absent` is "no firewall tool is installed on this machine", which is the true answer on an
/// ISO-built machine — `--variant=minbase` and neither recipe installs `nftables`, `ufw` or
/// `firewalld`. `inactive` is only said when a tool looked and found nothing loaded. `unknown`
/// always carries a reason. The `bool` these replace was a Slint property nothing wrote, drawn as
/// "Firewall: Off", and it reached a security audit of this OS as a finding.
fn format_status(state: &FirewallState) -> String {
    let mut out = match (state.state, state.kind.as_deref()) {
        // Only the tools were looked for: the kernel's own rules were not read, so nothing is
        // said about them (security review of #648, L2; the Network app says the same).
        (FirewallStatus::Absent, _) => {
            "No supported firewall manager detected (looked for nft, ufw and firewall-cmd). Active \
             filtering rules: not checked. This is not the same as a firewall that is switched off."
                .to_string()
        }
        (FirewallStatus::Active, Some(kind)) => format!("Firewall: {kind}, active."),
        (FirewallStatus::Active, None) => "Firewall: active.".to_string(),
        (FirewallStatus::Inactive, Some(kind)) => {
            format!("Firewall: {kind} is installed and is not filtering.")
        }
        (FirewallStatus::Inactive, None) => {
            "Firewall: installed and not filtering.".to_string()
        }
        (FirewallStatus::Unknown, Some(kind)) => {
            format!("Firewall: {kind} is installed and its state could not be determined.")
        }
        (FirewallStatus::Unknown, None) => {
            "Firewall: state could not be determined.".to_string()
        }
    };

    match state.rule_count {
        Some(count) => out.push_str(&format!("\n  {count} rules loaded.")),
        // Deliberately not "0 rules". An active firewall whose ruleset this session may not read
        // is a real state, and a number nobody counted is how "Firewall: Off" got written down.
        None if state.state == FirewallStatus::Active => {
            out.push_str("\n  Rule count: unknown — the ruleset was not readable from here.")
        }
        None => {}
    }

    if let Some(reason) = &state.reason {
        out.push_str(&format!("\n  {reason}"));
    }
    out
}

// ── Firewall status ──

pub struct FirewallStatusTool {
    net: Arc<dyn NetworkBackend>,
}

impl Tool for FirewallStatusTool {
    fn name(&self) -> &'static str { "firewall_status" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "firewall" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "firewall_status",
                "description": "Report which firewall this machine has and whether it is filtering",
                "parameters": { "type": "object", "properties": {} }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, _args: &serde_json::Value) -> String {
        status_answer(&*self.net)
    }
}

fn status_answer(net: &dyn NetworkBackend) -> String {
    match net.firewall() {
        Ok(state) => format_status(&state),
        Err(why) => refused("read this machine's firewall state", &why),
    }
}

// ── Firewall rules ──

pub struct FirewallListRulesTool {
    net: Arc<dyn NetworkBackend>,
}

impl Tool for FirewallListRulesTool {
    fn name(&self) -> &'static str { "firewall_list_rules" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "firewall" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "firewall_list_rules",
                "description": "List the firewall rules loaded on this machine, in the firewall's own words",
                "parameters": { "type": "object", "properties": {} }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, _args: &serde_json::Value) -> String {
        rules_answer(&*self.net)
    }
}

fn rules_answer(net: &dyn NetworkBackend) -> String {
    let state = match net.firewall() {
        Ok(state) => state,
        Err(why) => return refused("read this machine's firewall rules", &why),
    };

    if state.rules.is_empty() {
        // The header says which of the several reasons this is: no tool, nothing loaded, or a
        // ruleset that could not be read. An empty list on its own means all three.
        return format!("{}\nNo rules to list.", format_status(&state));
    }

    let mut out = format!("{}\n", format_status(&state));
    for rule in &state.rules {
        // The tool's own text, not reworded: a person comparing this against `nft list ruleset`
        // has to see the same line. `action` is `unknown` where the rule carries no verdict word
        // — a jump to another chain — rather than guessed at, because a fabricated ACCEPT in a
        // security display is the same bug one size smaller.
        out.push_str(&format!("  [{}/{}] {}\n", rule.chain, rule.action, rule.text));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::networking::backend::fake::FakeNetwork;
    use yantrik_ipc_contracts::network::FirewallRuleInfo;

    #[test]
    fn the_write_tools_are_not_registered() {
        // `firewall_enable` set INPUT DROP and `firewall_disable` ran `iptables -F`, neither
        // through any scoped privileged path. If one of these names comes back, the skill
        // manifest in `skills/firewall.yaml` has to come back with it — the tool list offered to
        // the model must not name anything the registry does not hold, in either direction.
        let mut reg = ToolRegistry::new();
        register_with(&mut reg, Arc::new(FakeNetwork::default()));
        let names: Vec<&str> = reg
            .list_metadata(PermissionLevel::Dangerous)
            .iter()
            .map(|m| m.name)
            .collect();
        assert_eq!(names, vec!["firewall_status", "firewall_list_rules"]);
    }

    #[test]
    fn no_firewall_installed_is_not_reported_as_a_firewall_that_is_off() {
        let text = format_status(&FirewallState {
            kind: None,
            state: FirewallStatus::Absent,
            rule_count: None,
            rules: Vec::new(),
            reason: Some("looked for nft, ufw and firewall-cmd; none is installed".into()),
        });
        assert!(text.contains("No supported firewall manager detected"), "{text}");
        assert!(text.contains("Active filtering rules: not checked"), "{text}");
        assert!(text.contains("not the same as a firewall that is switched off"), "{text}");
        assert!(!text.contains("0 rules"), "{text}");
    }

    #[test]
    fn an_active_firewall_with_an_unreadable_ruleset_gets_no_rule_number() {
        let text = format_status(&FirewallState {
            kind: Some("nftables".into()),
            state: FirewallStatus::Active,
            rule_count: None,
            rules: Vec::new(),
            reason: Some(
                "reading the nftables ruleset needs root and this session is not privileged".into(),
            ),
        });
        assert!(text.contains("nftables, active"), "{text}");
        assert!(text.contains("Rule count: unknown"), "{text}");
        assert!(!text.contains("0 rules"), "a count nobody took must not be printed: {text}");
    }

    #[test]
    fn a_rule_with_no_verdict_word_is_listed_as_unknown_rather_than_guessed() {
        let net = FakeNetwork {
            firewall: Some(Ok(FirewallState {
                kind: Some("nftables".into()),
                state: FirewallStatus::Active,
                rule_count: Some(2),
                rules: vec![
                    FirewallRuleInfo {
                        chain: "input".into(),
                        action: "accept".into(),
                        text: "ct state established,related accept".into(),
                    },
                    FirewallRuleInfo {
                        chain: "input".into(),
                        action: "unknown".into(),
                        text: "jump docker-user".into(),
                    },
                ],
                reason: None,
            })),
            ..Default::default()
        };
        let text = rules_answer(&net);
        assert!(text.contains("[input/unknown] jump docker-user"), "{text}");
        assert!(!text.contains("[input/accept] jump"), "{text}");
        // And the text is the firewall's own, unreworded.
        assert!(text.contains("ct state established,related accept"), "{text}");
    }

    #[test]
    fn a_service_that_is_down_is_named_rather_than_drawn_as_no_firewall() {
        let net = FakeNetwork {
            firewall: Some(Err("the network service did not come up".into())),
            ..Default::default()
        };
        let text = status_answer(&net);
        assert!(text.starts_with("Could not read"), "{text}");
        assert!(!text.contains("No supported firewall manager detected"), "{text}");
    }
}
