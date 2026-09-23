//! The reach of every live agent started as a catalog role — kept here, and told to every door
//! that asks.
//!
//! `hand_off` holds an agent to its role's reach *before* its first turn is sent ([`hold`]), so
//! there is no moment in which it can act unheld. From then on the shell is the store, as it is for
//! grants (`yantrik_ipc_transport::reach`):
//!
//! - the shell's own dispatch reads this registry in-process (`control_agents::actions` installs
//!   [`standing`] with `reach::keep_reach_with`);
//! - every other app and service asks the shell over its socket (`agent.reach`, by the SHA-256 of
//!   the token), and the shell answers from here on its RPC thread.
//!
//! What a token is — held to a reach, a live agent with no role, or no live agent at all — is
//! decided by the harness host, which handed the token out, and this registry together: a token no
//! live agent carries is refused on every door, whatever reach it once had. There is no file any
//! more. A door that read one could be told anything by whatever could write it, and told nothing
//! at all once it was deleted (#189).
//!
//! An agent that is stopped is let go ([`release`]); its token names nothing any more anyway.
//!
//! [`opened_app`] is the other thing a door needs from the shell: which app `open_app name=X`
//! would open, so that a role may open the apps its reach names and nothing else (#195).

use std::sync::{Mutex, MutexGuard};

use yantrik_harness::Host;
use yantrik_ipc_transport::reach::{self, Entry, Reach, Standing};

use super::catalog::Role;
use super::model::AgentId;
use crate::apps::DesktopEntry;

/// The most agents with a reach kept at once. The host runs six agents at most; the rest are ones
/// whose harness went without a Stop, and the oldest of those go first.
const MOST: usize = 64;

static LIVE: Mutex<Vec<Entry>> = Mutex::new(Vec::new());

fn live() -> MutexGuard<'static, Vec<Entry>> {
    LIVE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Hold `agent` to `role`'s reach from now on, on every door. An `Err` means there is no live
/// agent to hold, and it must not be started: an agent a door cannot hold is not started.
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
    Ok(())
}

/// Let `agent` go: stopped, it acts no more.
pub fn release(agent: &AgentId) {
    live().retain(|e| e.reach.agent != agent.0);
}

/// The reach a token was held to, by the token's digest — whether or not its agent is still live.
/// [`standing`] is what a door is told.
pub fn lookup_digest(digest: &str) -> Option<Reach> {
    live().iter().find(|e| e.token_sha256 == digest).map(|e| e.reach.clone())
}

/// [`lookup_digest`], for a token in hand.
pub fn lookup(token: &str) -> Option<Reach> {
    lookup_digest(&reach::token_digest(token))
}

/// The reach `agent` is held to, if it was started as a role.
pub fn of(agent: &AgentId) -> Option<Reach> {
    live().iter().find(|e| e.reach.agent == agent.0).map(|e| e.reach.clone())
}

/// What a token is, by its digest, as every door is told: no live agent at all unless `host` has
/// one whose token this is — stopped, reaped, from before this shell started, or never handed out
/// — and then held to the reach it was started with, or a live agent with no role.
///
/// Liveness first, and from the host that handed the token out: a role's reach is let go when its
/// agent stops, and a token that outlived its agent must not read as an agent with no role.
pub fn standing(host: Option<&Host>, digest: &str) -> Standing {
    let Some(host) = host else { return Standing::Unknown };
    let live = host
        .agents()
        .iter()
        .any(|agent| host.with_agent_token(&agent.id, reach::token_digest).as_deref() == Some(digest));
    if !live {
        return Standing::Unknown;
    }
    match lookup_digest(digest) {
        Some(reach) => Standing::Held(reach),
        None => Standing::Plain,
    }
}

/// At the shell's start: nothing is held yet. An older shell kept the reach in a file beside the
/// mode file, and whatever it left there named tokens that no longer exist; it is removed, so
/// nobody reads it as the reach.
pub fn reset() {
    live().clear();
    #[cfg(not(test))]
    {
        let old = yantrik_ipc_transport::gate::settings_path().with_file_name("agent-reach.json");
        if let Err(e) = std::fs::remove_file(&old) {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(path = %old.display(), error = %e, "could not remove the old agents' reach file");
            }
        }
    }
}

/// The app `open_app name=<name>` (and `show_app`) would open, as the id it publishes — or `None`
/// when the name opens anything else: a screen of the desktop, the launcher, the browser, the
/// desktop itself, an app this build shelved, or nothing. For `reach::resolve_opened_apps_with`:
/// a role may open the apps its reach names (#195), and this is how "names" is read.
///
/// Two answers must agree. The launch's own ([`crate::wire::dock::resolve`], which is what
/// `open_app` runs) says which surface the window that opens will publish; the catalog's folding
/// ([`crate::surfaces::surface_id`], which is how a role's reach names apps) says which app the
/// name is. Only when both say the same app is it opened on a reach's say-so — so an alias opens
/// its app, and a name some other entry on this disk won opens nothing a reach did not name.
pub fn opened_app(name: &str) -> Option<String> {
    opened_app_in(name, &crate::apps::Catalogue::shared().get())
}

/// [`opened_app`], against a catalogue the caller holds.
pub fn opened_app_in(name: &str, installed: &[DesktopEntry]) -> Option<String> {
    use crate::wire::dock::{resolve, route_surface, Resolved};
    let opens = match resolve(name, installed) {
        Resolved::Catalogue { surface: Some(surface), .. } => surface,
        // Blender is a route, and an app: its route runs the addon that publishes `blender`. Every
        // other route is the desktop's own (`shell`) or the browser, which publishes nothing.
        Resolved::Route(launch) => route_surface(launch).filter(|s| *s != "shell")?.to_string(),
        _ => return None,
    };
    (crate::surfaces::surface_id(name).as_deref() == Some(opens.as_str())).then_some(opens)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::catalog::{Catalog, SHIPPED};
    use serde_json::json;
    use yantrik_ipc_transport::reach::within_call_with;

    /// #195 against this desktop's own catalogue and the shipped roles: the Planner opens Notes
    /// and Calendar and brings them forward, and opens no other app, no screen, not the launcher
    /// and not the browser; the Reviewer opens the Editor by its other name; and once an app is
    /// open the role is held to its ceiling there.
    #[test]
    fn a_role_opens_the_apps_its_reach_names_and_nothing_else_on_this_desktop() {
        let installed = crate::surfaces::shipped_catalogue();
        let opens = |name: &str| opened_app_in(name, &installed);
        let catalog = Catalog::from_layers(&SHIPPED, &[]);
        let held = |role: &str| catalog.find(role).unwrap().reach_for("deepseek:c-test1");
        let open = |role: &str, action: &str, name: &str| {
            within_call_with(&held(role), "shell", action, "standard", &json!({ "name": name }), &opens)
        };

        for name in ["notes", "Notes", "calendar", "yantrik-notes"] {
            assert_eq!(open("planner", "open_app", name), Ok(()), "the Planner opens {name}");
            assert_eq!(open("planner", "show_app", name), Ok(()), "and brings {name} forward");
        }
        for name in [
            "terminal", "email", "text-editor", "files", "settings", "problems", "agents", "recipes",
            "launchpad", "browser", "shell", "yantrik", "skills", "music", "no-such-app", "",
        ] {
            let err = open("planner", "open_app", name).unwrap_err();
            assert!(err.starts_with("REACH: shell.open_app"), "the Planner opened {name:?}: {err}");
        }
        // An alias opens the app it names: `text-editor` is the Editor, which the Reviewer reads.
        for name in ["text-editor", "editor", "document-editor", "documents", "notes"] {
            assert_eq!(open("reviewer", "open_app", name), Ok(()), "the Reviewer opens {name}");
        }
        assert!(open("reviewer", "open_app", "terminal").is_err());
        // The Coder's `shell.agent_*` names no app; `editor` does.
        assert_eq!(open("coder", "open_app", "text-editor"), Ok(()));
        assert!(open("coder", "open_app", "terminal").is_err());
        // The Red team names nothing, and opens nothing.
        assert!(open("red-team", "open_app", "notes").is_err());

        // What the names are: an app by the id it publishes, and the desktop's own things none.
        assert_eq!(opens("text-editor").as_deref(), Some("editor"));
        assert_eq!(opens("sysmonitor").as_deref(), Some("system-monitor"));
        for screen in ["settings", "files", "problems", "launchpad", "browser", "shell", "music"] {
            assert_eq!(opens(screen), None, "{screen} is not an app a reach can name");
        }

        // Open, Notes holds the Planner to `safe`: it reads, and writes nothing.
        let planner = held("planner");
        assert!(within_call_with(&planner, "notes", "list_notes", "safe", &json!({}), &opens).is_ok());
        let err = within_call_with(&planner, "notes", "new_note", "standard", &json!({}), &opens).unwrap_err();
        assert!(err.starts_with("REACH: notes.new_note is graded `standard`, above the Planner's `safe` ceiling"), "{err}");
        assert!(err.contains("and may open calendar and notes (`shell.open_app name=<app>`)"), "{err}");
    }
}
