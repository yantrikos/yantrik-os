//! Who an agent is — never a caller's word.
//!
//! An `agent` argument would be an impersonation switch: any process that could reach the socket
//! could run commands in another agent's pane, read its jobs and kill them. So the agent comes from
//! a **token** and from the **kernel**, together (design, decision 3, "Routing, and who an agent
//! is"):
//!
//! 1. When the host hands an agent its first turn, the assignment carries an agent token — 128
//!    random bits, known to the shell and that harness. The harness passes it to the `yos-mcp` it
//!    starts for that conversation (`YANTRIK_AGENT_TOKEN`), and every act from that bridge carries
//!    it — beside `args` on `app.act`, never among them, because the arguments are what an
//!    approval card shows and an audit line keeps.
//! 2. The shell resolves the token to the agent **and** checks that the process on the socket
//!    (its pid from `SO_PEERCRED`, `yantrik_app_runtime::control::caller()`) descends from the
//!    harness process that attached — the host records that pid from the peer credentials at
//!    attach. A token presented from the wrong process tree is refused.
//!
//! # The contract the shell implements
//!
//! [`AgentResolver`] is that check. The shell's implementation is [`HostTokens`], over the
//! harness host (piece 1 of the design): the host says which agent a token names and which
//! process attached for it, and the kernel says who is calling.
//!
//! ```rust,ignore
//! install_resolver(Arc::new(HostTokens::new(host.clone())));
//! // Host::agent_for_token(&self, token: &str) -> Option<(AgentId, Option<u32> /* harness pid */)>
//! ```
//!
//! Every resolver refuses an unknown token, a harness with no recorded pid, a caller with no pid,
//! and a caller that does not descend from the harness. [`Lookup`] is the same rule over any
//! lookup function, [`TokenTable`] over an in-memory table, for tests; [`NoAgents`] knows no
//! tokens at all, which is the shell's resolver until the host is wired in.
//!
//! # The limit, stated
//!
//! Processes of the same user can read each other's environment, so a token is not a secret from
//! a hostile program running as the person. This stops confusion — a bridge acting for the wrong
//! agent — and casual impersonation. What stops a hostile program is the grade on every action.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::AgentId;

/// What a call is told when its token names no agent.
pub const NO_AGENT: &str = "no agent holds this token";

/// How far up the process tree the caller check walks. The real chain — `yos` ← `yos-mcp` ←
/// the mind's own process ← the harness — is four or five deep; this is a bound on a loop, not a
/// guess about depth.
const ANCESTRY_BOUND: usize = 64;

/// Turn a token and the kernel's account of the caller into an agent, or a sentence saying why not.
pub trait AgentResolver: Send + Sync {
    /// `caller_pid` is the socket peer's pid as the kernel reported it, never anything the caller
    /// wrote.
    fn resolve(&self, token: &str, caller_pid: Option<u32>) -> Result<AgentId, String>;
}

/// Knows no tokens. Every call is answered [`NO_AGENT`] — inert, but a real answer rather than a
/// missing action.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoAgents;

impl AgentResolver for NoAgents {
    fn resolve(&self, token: &str, caller_pid: Option<u32>) -> Result<AgentId, String> {
        verify(token, None, caller_pid)
    }
}

/// The resolver over any lookup `token → (agent, harness pid)`: the shell's, over
/// `Host::agent_for_token`.
pub struct Lookup<F>(pub F);

impl<F> AgentResolver for Lookup<F>
where
    F: Fn(&str) -> Option<(AgentId, Option<u32>)> + Send + Sync,
{
    fn resolve(&self, token: &str, caller_pid: Option<u32>) -> Result<AgentId, String> {
        let found = if token.trim().is_empty() { None } else { (self.0)(token.trim()) };
        verify(token, found, caller_pid)
    }
}

/// The shell's resolver: tokens are the harness host's, callers are the kernel's.
///
/// [`Host::agent_for_token`](yantrik_harness::Host::agent_for_token) says which live agent a
/// token names and the pid of the harness process that attached for it (read from `SO_PEERCRED`
/// at attach); the caller's pid comes from the socket the call arrived on. The token is believed
/// only when the caller is that harness or runs under it. A harness the host has no pid for — it
/// attached over a transport that could not say — vouches for nobody, and the refusal says why.
///
/// `D` is the process-tree check, [`descends_from`] over `/proc` unless a test supplies its own,
/// so the rule can be driven with pids that belong to no real process.
pub struct HostTokens<D = fn(u32, u32) -> bool> {
    host: yantrik_harness::Host,
    descends: D,
}

impl HostTokens {
    pub fn new(host: yantrik_harness::Host) -> Self {
        HostTokens { host, descends: descends_from }
    }
}

impl<D> HostTokens<D>
where
    D: Fn(u32, u32) -> bool + Send + Sync,
{
    /// The same rule with the process tree supplied: `descends(caller, harness)` answers whether
    /// `caller` is `harness` or runs under it.
    pub fn with_ancestry(host: yantrik_harness::Host, descends: D) -> Self {
        HostTokens { host, descends }
    }
}

impl<D> AgentResolver for HostTokens<D>
where
    D: Fn(u32, u32) -> bool + Send + Sync,
{
    fn resolve(&self, token: &str, caller_pid: Option<u32>) -> Result<AgentId, String> {
        let found = if token.trim().is_empty() { None } else { self.host.agent_for_token(token.trim()) };
        verify_with(token, found, caller_pid, &self.descends)
    }
}

/// Tokens held in memory. For tests, and for anything that issues tokens itself.
#[derive(Default)]
pub struct TokenTable {
    entries: Mutex<HashMap<String, (AgentId, Option<u32>)>>,
}

impl TokenTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that `token` belongs to `agent`, whose harness is the process `harness_pid`.
    pub fn issue(&self, token: &str, agent: AgentId, harness_pid: Option<u32>) {
        self.entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(token.to_string(), (agent, harness_pid));
    }

    /// Forget a token: its agent's calls are refused from now on.
    pub fn revoke(&self, token: &str) {
        self.entries.lock().unwrap_or_else(|e| e.into_inner()).remove(token);
    }
}

impl AgentResolver for TokenTable {
    fn resolve(&self, token: &str, caller_pid: Option<u32>) -> Result<AgentId, String> {
        let found = self.entries.lock().unwrap_or_else(|e| e.into_inner()).get(token.trim()).cloned();
        verify(token, found, caller_pid)
    }
}

/// The one rule every resolver applies to what its lookup found, against `/proc`.
fn verify(
    token: &str,
    found: Option<(AgentId, Option<u32>)>,
    caller_pid: Option<u32>,
) -> Result<AgentId, String> {
    verify_with(token, found, caller_pid, &descends_from)
}

/// The rule, with the process-tree check handed in.
fn verify_with(
    token: &str,
    found: Option<(AgentId, Option<u32>)>,
    caller_pid: Option<u32>,
    descends: &dyn Fn(u32, u32) -> bool,
) -> Result<AgentId, String> {
    if token.trim().is_empty() {
        return Err("no agent token came with this call, so it belongs to no agent. An agent's \
                    calls carry the token its harness was given with the agent's first turn, \
                    beside `args` on app.act (`agent_token`; `yos act --agent-token`, or \
                    YANTRIK_AGENT_TOKEN in yos's environment) — never among the arguments."
            .to_string());
    }
    let Some((agent, harness)) = found else {
        return Err(format!(
            "{NO_AGENT}. Tokens are issued by this desktop when an agent starts, and a token from \
             an earlier session or another machine names nothing here."
        ));
    };
    let Some(harness) = harness else {
        return Err(format!(
            "the harness holding this token for {agent} attached without a process this machine \
             could see, so the token cannot be checked against the caller; refused."
        ));
    };
    let Some(caller) = caller_pid.filter(|pid| *pid > 0) else {
        return Err("this call arrived without a process the kernel could name, so its token \
                    cannot be checked against it; refused."
            .to_string());
    };
    if !descends(caller, harness) {
        return Err(format!(
            "this token was not issued to the process that sent it: pid {caller} does not descend \
             from the harness the token belongs to. A token works only from the process tree its \
             harness started; refused."
        ));
    }
    Ok(agent)
}

/// Whether `pid` is `ancestor` or runs under it, walking up `/proc` with
/// [`yantrik_ipc_transport::peer_identity::parse_stat`].
///
/// Cycle-safe and bounded. Any `/proc` read that fails ends the walk with `false`: a process that
/// cannot be traced to the harness is not the harness's.
pub fn descends_from(pid: u32, ancestor: u32) -> bool {
    use yantrik_ipc_transport::peer_identity::parse_stat;

    let target = ancestor as i32;
    let mut at = pid as i32;
    let mut seen = std::collections::HashSet::new();
    for _ in 0..ANCESTRY_BOUND {
        if at == target {
            return true;
        }
        if at <= 1 || !seen.insert(at) {
            return false;
        }
        let Some(stat) = std::fs::read_to_string(format!("/proc/{at}/stat"))
            .ok()
            .and_then(|text| parse_stat(&text))
        else {
            return false;
        };
        at = stat.ppid;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent() -> AgentId {
        AgentId::new("pi", "c-7f3a91")
    }

    #[test]
    fn a_desktop_that_has_issued_no_tokens_answers_every_call_the_same_way() {
        let err = NoAgents.resolve("0123456789abcdef", Some(std::process::id())).unwrap_err();
        assert!(err.starts_with(NO_AGENT), "{err}");
        let err = NoAgents.resolve("  ", Some(std::process::id())).unwrap_err();
        assert!(err.contains("no agent token came with this call"), "{err}");
        assert!(err.contains("beside `args`"), "and it says where the token goes: {err}");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_token_resolves_only_from_under_the_harness_it_was_issued_to() {
        // This test process stands in for the harness.
        let harness = std::process::id();
        let table = TokenTable::new();
        table.issue("tok-pi", agent(), Some(harness));

        assert_eq!(table.resolve("tok-pi", Some(harness)), Ok(agent()), "the harness itself");

        // A child of the harness — what yos-mcp is — resolves too.
        let mut child = std::process::Command::new("sleep").arg("5").spawn().unwrap();
        assert_eq!(table.resolve("tok-pi", Some(child.id())), Ok(agent()));
        let _ = child.kill();
        let _ = child.wait();

        // pid 1 is nobody's child: the same token from there is refused, with a sentence.
        let err = table.resolve("tok-pi", Some(1)).unwrap_err();
        assert!(err.contains("not issued to the process that sent it"), "{err}");

        // No pid from the kernel, no check, no agent.
        let err = table.resolve("tok-pi", None).unwrap_err();
        assert!(err.contains("without a process the kernel could name"), "{err}");

        // A token nobody issued, and one taken back.
        assert!(table.resolve("tok-other", Some(harness)).unwrap_err().starts_with(NO_AGENT));
        table.revoke("tok-pi");
        assert!(table.resolve("tok-pi", Some(harness)).unwrap_err().starts_with(NO_AGENT));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_harness_with_no_recorded_process_cannot_vouch_for_anyone() {
        let lookup = Lookup(|token: &str| (token == "t").then(|| (agent(), None::<u32>)));
        let err = lookup.resolve("t", Some(std::process::id())).unwrap_err();
        assert!(err.contains("attached without a process"), "{err}");
        assert!(lookup.resolve("u", Some(1)).unwrap_err().starts_with(NO_AGENT));
    }

    // ── The shell's own resolver, over the harness host ─────────────

    use serde_json::json;
    use yantrik_harness::{protocol, Host, Turn};

    /// A harness attached over a socket the kernel named (`harness_pid`) — or over one it could
    /// not (`None`) — with one agent that has been handed a turn: (host, agent, its token).
    fn host_with_an_agent(harness_pid: Option<u32>) -> (Host, AgentId, String) {
        // The registry asks the kernel about the process that attached (#67). These tests
        // invent pids on purpose — the tree below them is invented too — so the host gets a
        // probe that vouches for the pid the harness claims instead of a faked `/proc`.
        let host = Host::new(vec![]).with_liveness(move |pid| Some(pid) == harness_pid);
        let attached = host
            .handle_from(protocol::ATTACH, &json!({ "id": "pi", "name": "Pi", "conversations": true }), harness_pid)
            .unwrap();
        let session = attached["session"].as_str().unwrap().to_string();
        let agent = host.start_agent("pi").unwrap();
        let _answer = host.send_to(&agent, Turn::new("tidy the photos")).unwrap();
        let handed = host.handle(protocol::POLL, &json!({ "session": session })).unwrap();
        (host, agent, handed["agent_token"].as_str().unwrap().to_string())
    }

    /// A process tree that exists only here: child → parent.
    fn tree(links: &'static [(u32, u32)]) -> impl Fn(u32, u32) -> bool + Send + Sync {
        move |pid, ancestor| {
            let mut at = pid;
            for _ in 0..ANCESTRY_BOUND {
                if at == ancestor {
                    return true;
                }
                match links.iter().find(|(child, _)| *child == at) {
                    Some((_, parent)) => at = *parent,
                    None => return false,
                }
            }
            false
        }
    }

    /// The harness is 4242; `yos` (5002) runs under `yos-mcp` (5001) under pi (5000) under it.
    /// 6001 is a program the person started from their own terminal.
    const PI_TREE: &[(u32, u32)] = &[(5002, 5001), (5001, 5000), (5000, 4242), (4242, 1), (6001, 900), (900, 1)];

    #[test]
    fn the_host_names_the_agent_and_the_process_tree_decides_whether_to_believe_it() {
        let (host, agent, token) = host_with_an_agent(Some(4242));
        let resolver = HostTokens::with_ancestry(host.clone(), tree(PI_TREE));

        assert_eq!(resolver.resolve(&token, Some(5002)), Ok(agent.clone()), "yos under the bridge under pi");
        assert_eq!(resolver.resolve(&token, Some(4242)), Ok(agent.clone()), "the harness itself");
        assert_eq!(resolver.resolve(&format!("  {token}\n"), Some(5002)), Ok(agent.clone()));

        // The same token from outside the harness's tree: refused, with a sentence naming the pid.
        let err = resolver.resolve(&token, Some(6001)).unwrap_err();
        assert!(err.contains("not issued to the process that sent it") && err.contains("pid 6001"), "{err}");
        // No pid from the kernel, no check, no agent.
        let err = resolver.resolve(&token, None).unwrap_err();
        assert!(err.contains("without a process the kernel could name"), "{err}");
        // A token the host never issued, and one it has taken back.
        assert!(resolver.resolve(&"0".repeat(32), Some(5002)).unwrap_err().starts_with(NO_AGENT));
        assert!(resolver.resolve("", Some(5002)).unwrap_err().contains("no agent token came with this call"));
        host.stop_agent(&agent);
        assert!(resolver.resolve(&token, Some(5002)).unwrap_err().starts_with(NO_AGENT));
    }

    #[test]
    fn the_tree_is_asked_about_the_caller_and_the_harness_the_host_recorded() {
        let (host, agent, token) = host_with_an_agent(Some(4242));
        let asked = std::sync::Arc::new(Mutex::new(Vec::new()));
        let resolver = HostTokens::with_ancestry(host, {
            let asked = asked.clone();
            move |pid, ancestor| {
                asked.lock().unwrap().push((pid, ancestor));
                pid == 5001
            }
        });
        assert_eq!(resolver.resolve(&token, Some(5001)), Ok(agent));
        assert!(resolver.resolve(&token, Some(7)).is_err());
        assert_eq!(*asked.lock().unwrap(), [(5001, 4242), (7, 4242)]);
    }

    #[test]
    fn a_harness_the_host_has_no_process_for_vouches_for_nobody_and_says_why() {
        // Attached over the TCP dev path, where there is no peer to read: fail closed.
        let (host, agent, token) = host_with_an_agent(None);
        let resolver = HostTokens::with_ancestry(host, |_: u32, _: u32| true);
        let err = resolver.resolve(&token, Some(5002)).unwrap_err();
        assert!(err.contains("attached without a process") && err.contains(&agent.to_string()), "{err}");
        assert!(err.contains("cannot be checked against the caller"), "{err}");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_shells_resolver_walks_the_real_proc() {
        // This test process stands in for the harness, and a child of it for the bridge.
        let me = std::process::id();
        let (host, agent, token) = host_with_an_agent(Some(me));
        let resolver = HostTokens::new(host);
        let mut child = std::process::Command::new("sleep").arg("5").spawn().unwrap();
        assert_eq!(resolver.resolve(&token, Some(child.id())), Ok(agent));
        let _ = child.kill();
        let _ = child.wait();
        assert!(resolver.resolve(&token, Some(1)).unwrap_err().contains("not issued to the process"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn descent_is_read_from_proc_and_a_stranger_is_not_a_descendant() {
        let me = std::process::id();
        assert!(descends_from(me, me));
        assert!(descends_from(me, 1), "everything descends from init");
        assert!(!descends_from(1, me));
        assert!(!descends_from(u32::MAX / 2, me), "a pid that does not exist descends from nothing");
    }
}
