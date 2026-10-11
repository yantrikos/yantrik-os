//! The reach of every live agent started as a catalog role — kept here, and published for every
//! door that does not live in this process.
//!
//! `hand_off` holds an agent to its role's reach *before* its first turn is sent ([`hold`]), so
//! there is no moment in which it can act unheld. From then on:
//!
//! - the shell's own dispatch reads this registry in-process (`control_agents::actions` installs
//!   [`lookup`] with `reach::read_reach_with`, as the shell spends grants in-process);
//! - every other app and service reads the file this writes, `agent-reach.json` beside the mode
//!   file, which keeps a SHA-256 of each agent's token and never the token.
//!
//! An agent that is stopped is let go ([`release`]); its token names nothing any more anyway. The
//! file is written whole each time, to a temporary name and renamed over, so a door reads the old
//! file or the new one and never half of either.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use yantrik_harness::Host;
use yantrik_ipc_transport::reach::{self, Entry, Reach};

use super::catalog::Role;
use super::model::AgentId;

/// The most agents with a reach kept at once. The host runs six agents at most; the rest are ones
/// whose harness went without a Stop, and the oldest of those go first.
const MOST: usize = 64;

static LIVE: Mutex<Vec<Entry>> = Mutex::new(Vec::new());

fn live() -> MutexGuard<'static, Vec<Entry>> {
    LIVE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Where the file goes. Under test, a file of the test run's own: a test must never write the
/// person's.
pub fn path() -> PathBuf {
    #[cfg(not(test))]
    return reach::reach_path();
    #[cfg(test)]
    return std::env::temp_dir().join(format!("yantrik-agent-reach-under-test-{}.json", std::process::id()));
}

/// Hold `agent` to `role`'s reach from now on, on every door. An `Err` means the reach could not
/// be published, and the agent must not be started: an agent a door cannot hold is not started.
pub fn hold(host: &Host, agent: &AgentId, role: &Role) -> Result<(), String> {
    let digest = host
        .with_agent_token(agent, reach::token_digest)
        .ok_or_else(|| format!("`{agent}` is not live, so there is no token to hold to the {}'s reach", role.name))?;
    let mut entries = live();
    entries.retain(|e| e.reach.agent != agent.0 && e.token_sha256 != digest);
    entries.push(Entry { token_sha256: digest, reach: role.reach_for(&agent.0) });
    if entries.len() > MOST {
        let extra = entries.len() - MOST;
        entries.drain(..extra);
    }
    write(&entries).map_err(|why| format!("the {}'s reach could not be published: {why}", role.name))
}

/// The agents held for a turn from a phone: how many such turns each is answering, and the reach
/// it was under before the first, to put back when the last ends (`None`: it had no role).
static REMOTE: Mutex<Vec<(String, usize, Option<Entry>)>> = Mutex::new(Vec::new());

/// What an agent answering a turn from the person's phone may do (design/channels-2026-09-29.md):
/// read unasked ([`REMOTE_ASKS_ABOVE`]); anything up to [`REMOTE_CEILING`] only once the person
/// allows it (on the phone, or at the machine); nothing above it. Not `standard` unasked: for the
/// built-in companion that grade includes sending email and overwriting a file (security review,
/// 29 Sep).
pub const REMOTE_ASKS_ABOVE: &str = "safe";
pub const REMOTE_CEILING: &str = "sensitive";

/// A turn from a phone, holding its agent to [`REMOTE_CEILING`] on every door until dropped.
pub struct RemoteHold {
    agent: AgentId,
    token_sha256: String,
}

impl RemoteHold {
    /// Whether `token` is the one this hold was made for.
    pub fn holds(&self, token: &str) -> bool {
        reach::token_digest(token) == self.token_sha256
    }
}

impl Drop for RemoteHold {
    fn drop(&mut self) {
        release_remote(&self.agent);
    }
}

/// Hold `agent` to [`REMOTE_CEILING`] while it answers a turn asked from a phone: a role's reach
/// keeps its surfaces at the lower ceiling; an agent with no role gets every app at that ceiling.
/// Counted, so two turns from the phone at once end together. An `Err` means it could not be
/// held, and the turn must not be sent.
pub fn hold_remote(host: &Host, agent: &AgentId) -> Result<RemoteHold, String> {
    let digest = host
        .with_agent_token(agent, reach::token_digest)
        .ok_or_else(|| format!("`{agent}` is not live, so there is no token to hold for a turn from a phone"))?;
    let mut remote = REMOTE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(held) = remote.iter_mut().find(|(a, _, _)| a == &agent.0) {
        held.1 += 1;
        return Ok(RemoteHold { agent: agent.clone(), token_sha256: digest });
    }
    let mut entries = live();
    let before = entries.iter().find(|e| e.reach.agent == agent.0).cloned();
    let mut held = before.clone().unwrap_or_else(|| Entry {
        token_sha256: digest.clone(),
        reach: Reach {
            agent: agent.0.clone(),
            role: "remote".into(),
            name: "turn asked from a phone".into(),
            surfaces: vec!["*".into()],
            ceiling: REMOTE_CEILING.into(),
            asks_above: None,
        },
    });
    held.token_sha256 = digest.clone();
    held.reach.asks_above = Some(REMOTE_ASKS_ABOVE.into());
    let lower = |c: &str| yantrik_ipc_transport::gate::grade(c).unwrap_or(usize::MAX);
    if lower(&held.reach.ceiling) > lower(REMOTE_CEILING) {
        held.reach.ceiling = REMOTE_CEILING.into();
    }
    entries.retain(|e| e.reach.agent != agent.0);
    entries.push(held);
    write(&entries).map_err(|why| {
        tracing::error!(agent = %agent, error = %why, "a turn from a phone could not be held");
        "this mind could not be held to what a phone may ask".to_string()
    })?;
    remote.push((agent.0.clone(), 1, before));
    Ok(RemoteHold { agent: agent.clone(), token_sha256: digest })
}

/// Whether `agent` is answering a turn from a phone now.
pub fn is_held_remote(agent: &AgentId) -> bool {
    REMOTE.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|(a, _, _)| a == &agent.0)
}

/// One turn from a phone has ended; when it was the last, `agent` is under what it was before.
fn release_remote(agent: &AgentId) {
    let before = {
        let mut remote = REMOTE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(at) = remote.iter().position(|(a, _, _)| a == &agent.0) else { return };
        remote[at].1 -= 1;
        if remote[at].1 > 0 {
            return;
        }
        remote.remove(at).2
    };
    let mut entries = live();
    entries.retain(|e| e.reach.agent != agent.0);
    if let Some(entry) = before {
        entries.push(entry);
    }
    if let Err(why) = write(&entries) {
        tracing::warn!(agent = %agent, error = %why, "could not write the agents' reach after a turn from a phone");
    }
}

/// Let `agent` go: stopped, it acts no more.
pub fn release(agent: &AgentId) {
    let mut entries = live();
    let before = entries.len();
    entries.retain(|e| e.reach.agent != agent.0);
    if entries.len() != before {
        if let Err(why) = write(&entries) {
            tracing::warn!(agent = %agent, error = %why, "could not write the agents' reach after a stop");
        }
    }
}

/// The reach a token carries, for the shell's own dispatch.
pub fn lookup(token: &str) -> Option<Reach> {
    let digest = reach::token_digest(token);
    live().iter().find(|e| e.token_sha256 == digest).map(|e| e.reach.clone())
}

/// The reach the token with this digest carries: what a door in another process asks the shell
/// for (#189), by digest so the token itself never travels.
pub fn lookup_digest(token_sha256: &str) -> Option<Reach> {
    let digest = token_sha256.trim().to_ascii_lowercase();
    live().iter().find(|e| e.token_sha256 == digest).map(|e| e.reach.clone())
}

/// The reach `agent` is held to, if it was started as a role.
pub fn of(agent: &AgentId) -> Option<Reach> {
    live().iter().find(|e| e.reach.agent == agent.0).map(|e| e.reach.clone())
}

/// At the shell's start: nothing is held yet, and whatever an earlier run left in the file named
/// tokens that no longer exist.
pub fn reset() {
    let mut entries = live();
    entries.clear();
    if let Err(why) = write(&entries) {
        tracing::warn!(error = %why, "could not clear the agents' reach file");
    }
}

fn write(entries: &[Entry]) -> Result<(), String> {
    let path = path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let tmp = path.with_extension(format!("json.{}", std::process::id()));
    std::fs::write(&tmp, reach::file_text(entries)).map_err(|e| format!("{}: {e}", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))
}

/// The file as a door in another process reads it, for a test to check what it would be told.
#[cfg(test)]
pub fn read_as_a_door(token: &str) -> Result<Option<Reach>, String> {
    let text = std::fs::read_to_string(path()).map_err(|e| e.to_string())?;
    reach::reach_from(&text, token)
}
