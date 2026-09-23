//! An agent's reach: what a role from the agent catalog may touch, held on every door that carries
//! the agent's token.
//!
//! # What it is
//!
//! A role — Researcher, Coder, Reviewer … (design/desk-and-mind-2026-09-23.md, section 5) — carries
//! a *reach*: the surfaces it may act on and a grade ceiling narrower than the machine's. A
//! Reviewer is `safe`; a Coder may ask for `sensitive`, and only in its own terminal and the
//! Editor. The shell starts a role's agent (`hand_off`), and from then on every `app.act` that
//! carries that agent's token is held to the reach here, before the handler runs and before any
//! grant is spent: an act outside the surfaces, or above the ceiling, is refused with `REACH:` and a
//! sentence that names the role and what it may touch.
//!
//! This is a second rule beside [`crate::gate`]'s, not a change to it. The gate asks whether the
//! machine and the person allow an act; this asks whether *this agent* was given it. Both must say
//! yes. A reach can only take away: nothing here lets an act past the machine's ceiling or the
//! person's mode.
//!
//! # Opening the apps it names
//!
//! A reach that names an app — `notes`, `notes.read_note`, `notes.list_*` — also lets its agent
//! open that app's window and bring it forward ([`OPENING`]: `shell.open_app name=notes`,
//! `shell.show_app name=notes`), whatever the reach's ceiling ([`within_call`]). Opening a window is
//! not an act on the app's data. Without this a Planner whose reach is `calendar, notes · safe`
//! could read neither while they were closed (#195), and raising it to `standard` to open them
//! would have let it write in them too: `safe` keeps meaning "reads only" inside the app.
//!
//! The name is resolved as the shell opens it and as the catalog names apps
//! ([`resolve_opened_apps_with`], which only the shell installs), so an alias opens the app it
//! names, and nothing else is opened on a reach's say-so: not another app, not a screen of the
//! desktop, not the launcher, not the browser. Only the reach's own ceiling steps aside; the
//! machine's ceiling and the person's mode still decide the act, as for any other.
//!
//! # Where the reach is kept
//!
//! A door knows the call's token (`gate::agent_token_of`) and nothing about roles. The shell, which
//! starts every agent, is the store, as it already is for grants: it knows each token's
//! [`Standing`] — held to a role's reach, a live agent with no role, or no live agent at all — and
//! answers [`ASK`] on its own socket with it, by the SHA-256 of the token ([`token_digest`]), never
//! the token. A door in another process asks once per token-carrying call ([`standing_of`]), and
//! only a process that is the shell ([`answered_by_the_shell`]); the shell's own dispatch reads its
//! registry in-process ([`keep_reach_with`]). The shell answers on its RPC thread, never through
//! its UI thread, so the question costs a local round trip and not a frame.
//!
//! It fails closed ([`decided`]). A token no live agent carries — its agent stopped, or the shell
//! restarted since it was handed out — is refused, and so is every token-carrying call while the
//! shell does not answer. The reach used to be a file, `agent-reach.json`, that each door read, and
//! a missing file was no reach for anyone: anything running as the person could delete it, or
//! rewrite it, and the next act carrying the token went through unheld (#189). A call with no
//! token is the person's own and never asks.
//!
//! # A surface
//!
//! `notes` is every action of the app published as `notes`; `shell.agent_run` is one action;
//! `shell.agent_*` is the shell's actions whose names begin `agent_`. A few acts are within every
//! reach ([`ALWAYS`]): asking the person, the steps that follow from asking, and an agent reading
//! its own session. Without them a role could not ask to be allowed what it may do.
//!
//! # What it does not cover
//!
//! Only doors that carry the token. A command the agent runs in its own terminal runs as the
//! person, with no token — so `shell.agent_run` in a reach is a promise of whatever a command can
//! do, and it is `sensitive` for that reason. A harness's own built-in tools, and the browser
//! driven over its debugging port, never reach an `app.act` at all. The reach bounds the desktop's
//! doors; the grade and the person's mode still bound everything else.

use std::sync::OnceLock;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::gate::{grade, LADDER};
use crate::server::{PeerCred, RpcServer};
use crate::sync_client::{PeerRule, SyncRpcClient};

#[cfg(test)]
mod vectors;

/// Within every reach: asking the person for an act, following that request up, spending the
/// grant it came to, writing down an act that ran unasked, and reading one's own session. None of
/// them does anything the person has not seen; without them a role could not ask to be allowed
/// what its reach does let it do.
pub const ALWAYS: [&str; 5] = [
    "shell.request_approval",
    "shell.approval_status",
    "shell.consume_approval",
    "shell.record_unasked_action",
    "shell.read_agent",
];

/// The acts that put an app's window in front of the person — opening it, and bringing it
/// forward — each naming the app as `name`. Within a reach that names the app, whatever the
/// reach's ceiling; see [`within_call`].
pub const OPENING: [&str; 2] = ["shell.open_app", "shell.show_app"];

/// One agent's reach, as the shell keeps it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reach {
    /// The agent, `<mind>:<conversation>`.
    pub agent: String,
    /// The role's id in the catalog (`reviewer`).
    pub role: String,
    /// The role's name as a person reads it (`Reviewer`).
    pub name: String,
    /// What it may touch: `app`, `app.action` or `app.prefix*`.
    pub surfaces: Vec<String>,
    /// The highest grade it may use, on [`LADDER`].
    pub ceiling: String,
}

/// One agent the shell holds to a reach: the digest of its token, and the reach.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub token_sha256: String,
    pub reach: Reach,
}

/// The SHA-256 of a token, as lowercase hex: what a door asks the shell about in the token's
/// place. Whatever hears the question learns which agent is asking, and cannot present its token.
pub fn token_digest(token: &str) -> String {
    let digest = Sha256::digest(token.trim().as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

// ── What the shell says about a token ────────────────────────────────

/// What a token is, as the shell — which handed it out — knows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Standing {
    /// A live agent started as a role: held to the role's reach.
    Held(Reach),
    /// A live agent started on a mind with no role. No reach holds it; the gate still does.
    Plain,
    /// No live agent carries it: its agent was stopped, the shell has restarted since it was
    /// handed out, or it was never handed out at all.
    Unknown,
}

impl Standing {
    /// The shell's answer to [`ASK`], as it travels.
    pub fn to_json(&self) -> Value {
        match self {
            Standing::Held(reach) => json!({ "standing": "held", "reach": reach }),
            Standing::Plain => json!({ "standing": "plain" }),
            Standing::Unknown => json!({ "standing": "unknown" }),
        }
    }

    /// Read the shell's answer. Anything that is not one of the three is an error, never a guess:
    /// a door that cannot read the answer has not been answered.
    pub fn from_json(answer: &Value) -> Result<Standing, String> {
        match answer.get("standing").and_then(Value::as_str) {
            Some("held") => serde_json::from_value(answer.get("reach").cloned().unwrap_or(Value::Null))
                .map(Standing::Held)
                .map_err(|_| "its answer held a reach this door cannot read".to_string()),
            Some("plain") => Ok(Standing::Plain),
            Some("unknown") => Ok(Standing::Unknown),
            Some(other) => Err(format!("its answer named a standing this door does not know: `{other}`")),
            None => Err("its answer named no standing".to_string()),
        }
    }
}

/// The method a door asks the shell a token's standing with, on the shell's own socket:
/// `{"token_sha256": "<64 hex digits>"}` in, [`Standing::to_json`] out. Not an action — it is not
/// in `describe`, a mind has no use for it, and it is answered on the RPC thread, never the UI one.
pub const ASK: &str = "agent.reach";

/// How long a door waits for the shell's answer. The shell answers from memory on its RPC thread,
/// normally well under a millisecond; this bounds a shell that is not answering, and the outcome
/// then is a refusal that says so, as for a grant.
pub const ASK_ROUNDTRIP: Duration = Duration::from_secs(5);

/// The shell's control surface, where every agent's standing is kept.
const SHELL: &str = "app-shell";

type Reader = dyn Fn(&str) -> Standing + Send + Sync;

static READER: OnceLock<Box<Reader>> = OnceLock::new();

/// Keep the agents' standing in this process: `read` answers for a token's digest. The shell
/// calls it once, with its registry and its harness host, because the shell starts every agent and
/// is where the reach is kept; from then on it answers [`ASK`] for every other door. Every other
/// process leaves it unset and asks the shell. A second call changes nothing: the store does not
/// move.
pub fn keep_reach_with(read: impl Fn(&str) -> Standing + Send + Sync + 'static) {
    let _ = READER.set(Box::new(read));
}

/// Whether this process keeps the agents' standing — the shell, or a test's stand-in for it.
pub fn keeps_reach() -> bool {
    READER.get().is_some()
}

/// The answer to [`ASK`] in the process that keeps the reach; `Err` in any other, or for a
/// question that is not a digest. `yantrik_app_runtime::control` calls it on the shell's socket.
pub fn answer(params: &Value) -> Result<Value, String> {
    match READER.get() {
        Some(read) => answer_with(read.as_ref(), params),
        None => Err(format!("this process does not keep the agents' reach; the shell answers {ASK}")),
    }
}

/// [`answer`], from `read`.
pub fn answer_with(read: &(dyn Fn(&str) -> Standing + Send + Sync), params: &Value) -> Result<Value, String> {
    let digest = params.get("token_sha256").and_then(Value::as_str).unwrap_or("").trim();
    let is_digest = digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if !is_digest {
        return Err(format!(
            "{ASK} takes `token_sha256`: the SHA-256 of an agent token, as 64 lowercase hex digits — never the token"
        ));
    }
    Ok(read(digest).to_json())
}

/// What `token` is right now. In the process that keeps the reach, its own answer; in every other,
/// the shell's, asked over its socket. `Err` says why the shell did not answer.
///
/// IO. A window asks on its RPC thread, beside reading the ceiling and the mode.
pub fn standing_of(token: &str) -> Result<Standing, String> {
    let digest = token_digest(token);
    match READER.get() {
        Some(read) => Ok(read(&digest)),
        None => ask(&RpcServer::default_address(SHELL), answered_by_the_shell, &digest),
    }
}

/// Ask whatever answers at `address` for a digest's standing — once `rule` has said, from the
/// kernel's account of the listener, that it is the shell. Nothing is written to anything else.
pub fn ask(address: &str, rule: PeerRule, digest: &str) -> Result<Standing, String> {
    let answer = SyncRpcClient::new(address)
        .with_timeout(ASK_ROUNDTRIP)
        .expecting_peer(rule)
        .call(ASK, json!({ "token_sha256": digest }))
        .map_err(|e| e.message)?;
    Standing::from_json(&answer)
}

/// The rule for who may answer [`ASK`]: the process listening on `app-shell.sock` must be a
/// `yantrik-ui` binary, as it must be to spend a grant (`owner::must_be_the_shell`). Otherwise
/// anything that bound the name while the shell was down could answer "no role" for every token.
///
/// Same-uid limits stand, as they do there: a process running as the person can copy a binary
/// under that name. This stops accidents and casual impersonation, not code that already runs as
/// the person.
pub fn answered_by_the_shell(peer: Option<PeerCred>) -> Result<(), String> {
    use crate::owner::{exe_of, is_shell_binary, SHELL_BINARY};
    let Some(peer) = peer else {
        return Err("the kernel would not say which process is answering as the shell, so it was not asked".to_string());
    };
    match exe_of(peer.pid) {
        Some(exe) if is_shell_binary(&exe) => Ok(()),
        Some(exe) => Err(format!(
            "the process answering as the shell is {exe} (pid {}), not the desktop's own {SHELL_BINARY}, so it was not asked",
            peer.pid
        )),
        None => Err(format!(
            "the process answering as the shell (pid {}) could not be identified from /proc, so it was not asked",
            peer.pid
        )),
    }
}

/// The refusal for a token no live agent carries.
const NO_LIVE_AGENT: &str = "REACH: the agent token this call carries names no live agent on this desktop — \
                             its agent was stopped, or the shell has restarted since it was handed out — so \
                             nothing carrying it runs. Nothing was run.";

/// What a call carrying a token of this standing is held to: `Some(reach)` for a role's agent,
/// `None` for a live agent with no role (the gate alone decides for it), and a `REACH:` refusal for
/// a token no live agent carries or when the shell did not answer (`Err(why)`).
///
/// Pure: the rule every door applies to what it learned. `deploy/yantrik-os/reach-vectors.json`
/// writes it out for the Python SDK.
pub fn decided(standing: Result<Standing, String>) -> Result<Option<Reach>, String> {
    match standing {
        Ok(Standing::Held(reach)) => Ok(Some(reach)),
        Ok(Standing::Plain) => Ok(None),
        Ok(Standing::Unknown) => Err(NO_LIVE_AGENT.to_string()),
        Err(why) => Err(format!(
            "REACH: the shell, which keeps every agent's reach, did not answer ({why}), so no act \
             carrying an agent token runs until it does. Nothing was run."
        )),
    }
}

/// The reach a call carrying `token` is held to, or the `REACH:` refusal that ends it: the whole of
/// what a door learns about an agent before it decides. See [`decided`].
pub fn reach_of(token: &str) -> Result<Option<Reach>, String> {
    decided(standing_of(token))
}

// ── Holding an act to a reach ────────────────────────────────────────

/// Does one of `surfaces` cover `app_id.action`?
pub fn covers(surfaces: &[String], app_id: &str, action: &str) -> bool {
    surfaces.iter().any(|surface| {
        let surface = surface.trim();
        match surface.split_once('.') {
            None => surface.eq_ignore_ascii_case(app_id),
            Some((app, named)) if app.eq_ignore_ascii_case(app_id) => match named.strip_suffix('*') {
                Some(prefix) => action.starts_with(prefix),
                None => named == action,
            },
            Some(_) => false,
        }
    })
}

/// The surfaces as a sentence reads them: "editor, documents and notes".
pub fn surfaces_text(surfaces: &[String]) -> String {
    match surfaces {
        [] => "nothing on this desktop beyond asking the person and reading its own session".to_string(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

/// The apps a reach names, each once, in the order it names them: the app part of every surface
/// (`notes` for `notes`, `notes.read_note` and `notes.list_*`) — but never the desktop's own
/// `shell`, whose actions a surface like `shell.agent_*` names, and which is not an app to open.
pub fn apps_named(surfaces: &[String]) -> Vec<String> {
    let mut apps: Vec<String> = Vec::new();
    for surface in surfaces {
        let app = surface.trim().split('.').next().unwrap_or("").trim().to_lowercase();
        if app.is_empty() || app == "shell" || app.contains('*') || apps.contains(&app) {
            continue;
        }
        apps.push(app);
    }
    apps
}

type Opener = dyn Fn(&str) -> Option<String> + Send + Sync;

static OPENS: OnceLock<Box<Opener>> = OnceLock::new();

/// Install how this process tells which app an [`OPENING`] act would open, from the name it was
/// asked with: the id the app publishes, or `None` for anything that is not an app a reach can
/// name — a screen of the desktop, the launcher, the browser, a name nothing answers to. The shell
/// serves `open_app` and is the only process that installs this; without it no opening act is
/// within a reach by the app it names. A second call changes nothing.
pub fn resolve_opened_apps_with(opens: impl Fn(&str) -> Option<String> + Send + Sync + 'static) {
    let _ = OPENS.set(Box::new(opens));
}

fn opened_here(name: &str) -> Option<String> {
    OPENS.get().and_then(|opens| opens(name))
}

/// Who the agent is and what it may touch, for the end of a refusal.
fn who(reach: &Reach) -> String {
    let apps = apps_named(&reach.surfaces);
    let opens = if apps.is_empty() {
        String::new()
    } else {
        format!(", and may open {} (`shell.open_app name=<app>`)", surfaces_text(&apps))
    };
    format!(
        "`{agent}` is the {name}, which may touch {surfaces}, at most `{ceiling}`{opens}",
        agent = reach.agent,
        name = reach.name,
        surfaces = surfaces_text(&reach.surfaces),
        ceiling = reach.ceiling,
    )
}

const NEXT: &str = "Say in your answer what else needs doing; the person, or whoever handed you \
                    this, can do it.";

/// May the agent `reach` belongs to use `app_id.action`, which its surface grades `graded`?
///
/// Pure: the reach was read beforehand. The grade is the one the surface publishes, never one the
/// caller declared. An act within [`ALWAYS`] passes whatever the reach; anything else must be on
/// one of its surfaces and at or below its ceiling. A grade off the ladder, or a ceiling that is,
/// is refused: a typo must not widen a reach.
///
/// This is the rule without the call's arguments, so it never lets an [`OPENING`] act through by
/// the app it names; the dispatch uses [`within_call`].
pub fn within(reach: &Reach, app_id: &str, action: &str, graded: &str) -> Result<(), String> {
    let what = format!("{app_id}.{action}");
    if ALWAYS.contains(&what.as_str()) {
        return Ok(());
    }
    if !covers(&reach.surfaces, app_id, action) {
        return Err(format!(
            "REACH: {what} is outside the {name}'s reach, so it was not run. {who}. {NEXT}",
            name = reach.name,
            who = who(reach)
        ));
    }
    let Some(ceiling) = grade(&reach.ceiling) else {
        return Err(format!(
            "REACH: the {name}'s ceiling `{ceiling}` is not a level this OS defines ({ladder}), so \
             {what} was not run. {who}.",
            name = reach.name,
            ceiling = reach.ceiling,
            ladder = LADDER.join(" < "),
            who = who(reach)
        ));
    };
    let Some(level) = grade(graded) else {
        return Err(format!(
            "REACH: {what} is graded `{graded}`, which is not a level this OS defines ({ladder}), so \
             it was not run.",
            ladder = LADDER.join(" < ")
        ));
    };
    if level > ceiling {
        return Err(format!(
            "REACH: {what} is graded `{graded}`, above the {name}'s `{cap}` ceiling, so it was not \
             run, and nobody was asked. {who}. {NEXT}",
            name = reach.name,
            cap = reach.ceiling,
            who = who(reach)
        ));
    }
    Ok(())
}

/// [`within`], for a call with its arguments as sent — what every dispatch holds a call to.
///
/// One thing more: an [`OPENING`] act naming an app the reach names is within it whatever the
/// reach's ceiling, the app resolved the way this process opens it ([`resolve_opened_apps_with`]).
/// Any other name — another app, a screen, the launcher, the browser, a name that is not text —
/// is held as any other act: refused unless the reach names the act itself.
pub fn within_call(reach: &Reach, app_id: &str, action: &str, graded: &str, args: &Value) -> Result<(), String> {
    within_call_with(reach, app_id, action, graded, args, &opened_here)
}

/// [`within_call`], with `opens` saying which app a name opens. Pure; the vectors replay it.
pub fn within_call_with(
    reach: &Reach,
    app_id: &str,
    action: &str,
    graded: &str,
    args: &Value,
    opens: &dyn Fn(&str) -> Option<String>,
) -> Result<(), String> {
    let what = format!("{app_id}.{action}");
    let named = OPENING
        .contains(&what.as_str())
        .then(|| args.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()))
        .flatten();
    let Some(name) = named else { return within(reach, app_id, action, graded) };
    let apps = apps_named(&reach.surfaces);
    if opens(name).is_some_and(|app| apps.iter().any(|named| named.eq_ignore_ascii_case(&app))) {
        return Ok(());
    }
    // A reach that names the act itself (a person's role with `shell.open_app` in it) holds it as
    // any other act, ceiling and all.
    if covers(&reach.surfaces, app_id, action) {
        return within(reach, app_id, action, graded);
    }
    Err(format!(
        "REACH: {what} `{name}` is outside the {role}'s reach, so it was not run: a role may open \
         only the apps its reach names. {who}. {NEXT}",
        role = reach.name,
        who = who(reach)
    ))
}

/// The whole rule for one call: the token it carried (if any), its standing asked of the shell,
/// and the act held to the reach. No token passes untouched — the person's own call asks nothing —
/// and so does a live agent with no role; a token no live agent carries, or one the shell could
/// not be asked about, is refused.
pub fn permits(token: Option<&str>, app_id: &str, action: &str, graded: &str, args: &Value) -> Result<(), String> {
    let Some(token) = token else { return Ok(()) };
    match reach_of(token)? {
        Some(reach) => within_call(&reach, app_id, action, graded, args),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reviewer() -> Reach {
        Reach {
            agent: "deepseek:c-1a2b3c".into(),
            role: "reviewer".into(),
            name: "Reviewer".into(),
            surfaces: vec!["editor".into(), "documents".into(), "notes".into()],
            ceiling: "safe".into(),
        }
    }

    fn coder() -> Reach {
        Reach {
            agent: "pi:c-9f8e7d".into(),
            role: "coder".into(),
            name: "Coder".into(),
            surfaces: vec!["shell.agent_*".into(), "editor".into()],
            ceiling: "sensitive".into(),
        }
    }

    fn planner() -> Reach {
        Reach {
            agent: "deepseek:c-p1an".into(),
            role: "planner".into(),
            name: "Planner".into(),
            surfaces: vec!["calendar".into(), "notes".into()],
            ceiling: "safe".into(),
        }
    }

    /// What the shell's `open_app` opens for a name, as far as these tests need it: the apps
    /// under their ids and one alias, and the desktop's own things as nothing a reach can name.
    fn opens(name: &str) -> Option<String> {
        match name {
            "notes" | "calendar" | "terminal" | "documents" | "blender" => Some(name.to_string()),
            "text-editor" | "editor" => Some("editor".into()),
            _ => None,
        }
    }

    fn open(reach: &Reach, action: &str, name: Value) -> Result<(), String> {
        within_call_with(reach, "shell", action, "standard", &json!({ "name": name }), &opens)
    }

    #[test]
    fn an_act_on_its_surfaces_and_under_its_ceiling_runs() {
        assert!(within(&reviewer(), "notes", "list_notes", "safe").is_ok());
        assert!(within(&coder(), "shell", "agent_run", "sensitive").is_ok());
        assert!(within(&coder(), "shell", "agent_job", "standard").is_ok());
        assert!(within(&coder(), "editor", "save", "standard").is_ok());
    }

    #[test]
    fn an_act_off_its_surfaces_is_refused_naming_the_role_and_its_reach() {
        let err = within(&reviewer(), "files", "move", "safe").unwrap_err();
        assert!(err.starts_with("REACH: files.move is outside the Reviewer's reach"), "{err}");
        assert!(err.contains("`deepseek:c-1a2b3c` is the Reviewer, which may touch editor, documents and notes, at most `safe`"), "{err}");
        // `shell.agent_*` is the agent_ actions and no other shell action.
        let err = within(&coder(), "shell", "new_agent", "sensitive").unwrap_err();
        assert!(err.contains("outside the Coder's reach"), "{err}");
        assert!(within(&coder(), "shell", "files_delete", "dangerous").is_err());
        // A surface named for one app is not a prefix of another's name.
        assert!(within(&coder(), "editorial", "x", "safe").is_err());
    }

    #[test]
    fn an_act_above_its_ceiling_is_refused_even_on_its_surfaces() {
        let err = within(&reviewer(), "notes", "new_note", "standard").unwrap_err();
        assert!(err.starts_with("REACH: notes.new_note is graded `standard`, above the Reviewer's `safe` ceiling"), "{err}");
        assert!(err.contains("nobody was asked"), "{err}");
        let err = within(&coder(), "shell", "agent_run", "dangerous").unwrap_err();
        assert!(err.contains("above the Coder's `sensitive` ceiling"), "{err}");
    }

    #[test]
    fn asking_the_person_and_reading_itself_are_within_every_reach() {
        let nothing = Reach { surfaces: vec![], ..reviewer() };
        for always in ALWAYS {
            let (app, action) = always.split_once('.').unwrap();
            assert!(within(&nothing, app, action, "safe").is_ok(), "{always}");
        }
        let err = within(&nothing, "shell", "show_agent", "safe").unwrap_err();
        assert!(err.contains("nothing on this desktop beyond asking the person"), "{err}");
    }

    #[test]
    fn a_grade_or_a_ceiling_off_the_ladder_never_widens_a_reach() {
        assert!(within(&coder(), "shell", "agent_run", "catastrophic").is_err());
        let typo = Reach { ceiling: "sensitve".into(), ..coder() };
        assert!(within(&typo, "shell", "agent_job", "safe").is_err());
    }

    // ── Opening the apps it names (#195) ──

    /// The Planner, `calendar, notes · safe`, seen live unable to open either: it opens both, and
    /// brings them forward, though opening is `standard` — and nothing else.
    #[test]
    fn a_role_opens_the_apps_its_reach_names_whatever_its_ceiling() {
        for action in ["open_app", "show_app"] {
            for app in ["notes", "calendar"] {
                assert_eq!(open(&planner(), action, json!(app)), Ok(()), "{action} {app}");
            }
            assert_eq!(open(&planner(), action, json!("  notes ")), Ok(()), "the name is read as the handler trims it");
        }
        // An alias opens the app it names: `text-editor` is the Editor the Reviewer may read.
        assert_eq!(open(&reviewer(), "open_app", json!("text-editor")), Ok(()));
        // `shell.agent_*` names the shell's actions, not an app; `editor` is an app.
        assert_eq!(open(&coder(), "open_app", json!("editor")), Ok(()));
        assert_eq!(apps_named(&coder().surfaces), ["editor"]);
        assert_eq!(apps_named(&["notes.list_*".into(), "notes.read_note".into(), "Calendar".into()]), ["notes", "calendar"]);
    }

    /// Any other app, a screen, the launcher, the browser, a name nothing answers to, a name that
    /// is not text: refused, naming what it asked to open and what the role may open.
    #[test]
    fn a_role_opens_nothing_its_reach_does_not_name() {
        for name in ["terminal", "settings", "problems", "browser", "launchpad", "shell", "no-such-app", "text-editor"] {
            let err = open(&planner(), "open_app", json!(name)).unwrap_err();
            assert!(
                err.starts_with(&format!("REACH: shell.open_app `{name}` is outside the Planner's reach, so it was not run: a role may open only the apps its reach names.")),
                "{err}"
            );
            assert!(err.contains("which may touch calendar and notes, at most `safe`, and may open calendar and notes (`shell.open_app name=<app>`)"), "{err}");
        }
        for name in [json!(7), json!(null), json!(""), json!(["notes"])] {
            let err = open(&planner(), "open_app", name.clone()).unwrap_err();
            assert!(err.starts_with("REACH: shell.open_app is outside the Planner's reach"), "{name}: {err}");
        }
        // Without its arguments the act is held as any other: the rule needs the name.
        assert!(within(&planner(), "shell", "open_app", "standard").is_err());
        // A role with no surfaces names no app, and its refusal offers none.
        let nothing = Reach { surfaces: vec![], ..planner() };
        let err = open(&nothing, "open_app", json!("notes")).unwrap_err();
        assert!(!err.contains("and may open"), "{err}");
        // Only the shell's two opening acts: an app's own action of that name is its own act.
        let err = within_call_with(&planner(), "notes", "open_app", "standard", &json!({"name": "notes"}), &opens).unwrap_err();
        assert!(err.contains("above the Planner's `safe` ceiling"), "{err}");
        let err = within_call_with(&planner(), "shell", "close_window", "standard", &json!({"name": "notes"}), &opens).unwrap_err();
        assert!(err.starts_with("REACH: shell.close_window is outside"), "{err}");
    }

    /// Opened, the app is held to the reach like any other: the Planner reads Notes and writes
    /// nothing in it.
    #[test]
    fn once_open_the_app_is_held_to_the_reach_ceiling() {
        assert!(open(&planner(), "open_app", json!("notes")).is_ok());
        assert!(within_call(&planner(), "notes", "list_notes", "safe", &json!({})).is_ok());
        let err = within_call(&planner(), "notes", "new_note", "standard", &json!({})).unwrap_err();
        assert!(err.starts_with("REACH: notes.new_note is graded `standard`, above the Planner's `safe` ceiling"), "{err}");
        let err = within_call(&planner(), "notes", "delete_note", "sensitive", &json!({"name": "notes"})).unwrap_err();
        assert!(err.contains("above the Planner's `safe` ceiling"), "{err}");
    }

    /// A person's role that names `shell.open_app` itself holds the act as any other, ceiling and
    /// all; and this process, which installed no resolver, opens nothing on a reach's say-so.
    #[test]
    fn a_reach_that_names_the_act_holds_it_as_any_other_and_no_resolver_opens_nothing() {
        let opener = Reach { surfaces: vec!["shell.open_app".into()], ceiling: "standard".into(), ..planner() };
        assert!(open(&opener, "open_app", json!("terminal")).is_ok(), "its surfaces name the act");
        let capped = Reach { ceiling: "safe".into(), ..opener };
        let err = open(&capped, "open_app", json!("terminal")).unwrap_err();
        assert!(err.contains("above the Planner's `safe` ceiling"), "{err}");
        // No test in this crate installs a resolver, as no process but the shell does.
        let err = within_call(&planner(), "shell", "open_app", "standard", &json!({"name": "notes"})).unwrap_err();
        assert!(err.starts_with("REACH: shell.open_app `notes` is outside the Planner's reach"), "{err}");
    }

    // ── Where the reach is kept, and failing closed (#189) ──

    #[test]
    fn no_token_is_the_persons_call_and_meets_no_reach() {
        assert!(permits(None, "system-monitor", "kill_process", "dangerous", &json!({})).is_ok());
    }

    #[test]
    fn a_token_no_live_agent_carries_and_a_shell_that_did_not_answer_are_refused() {
        assert_eq!(decided(Ok(Standing::Held(planner()))), Ok(Some(planner())));
        assert_eq!(decided(Ok(Standing::Plain)), Ok(None), "a plain agent is the gate's alone");
        let err = decided(Ok(Standing::Unknown)).unwrap_err();
        assert!(err.starts_with("REACH: the agent token this call carries names no live agent"), "{err}");
        let err = decided(Err("Connection failed (/run/user/1000/yantrik/app-shell.sock): No such file or directory (os error 2)".into())).unwrap_err();
        assert_eq!(
            err,
            "REACH: the shell, which keeps every agent's reach, did not answer (Connection failed \
             (/run/user/1000/yantrik/app-shell.sock): No such file or directory (os error 2)), so no act \
             carrying an agent token runs until it does. Nothing was run."
        );
    }

    #[test]
    fn the_shells_answer_is_asked_and_given_by_digest_and_read_whole_or_not_at_all() {
        let token = "0123456789abcdef0123456789abcdef";
        let digest = token_digest(token);
        assert_eq!(digest.len(), 64);
        assert_eq!(token_digest(&format!("  {token}\n")), digest, "read as the dispatch trims it");
        let known = digest.clone();
        let read = move |d: &str| if d == known { Standing::Held(reviewer()) } else { Standing::Unknown };
        let answered = answer_with(&read, &json!({ "token_sha256": digest })).unwrap();
        assert!(!answered.to_string().contains(token), "{answered}");
        assert_eq!(Standing::from_json(&answered), Ok(Standing::Held(reviewer())));
        assert_eq!(answer_with(&read, &json!({ "token_sha256": "f".repeat(64) })).unwrap(), json!({"standing": "unknown"}));
        for asked in [json!({ "token_sha256": token }), json!({}), json!({ "token_sha256": digest.to_uppercase() })] {
            let err = answer_with(&read, &asked).unwrap_err();
            assert!(err.contains("never the token"), "{asked}: {err}");
        }
        assert!(answer(&json!({ "token_sha256": "f".repeat(64) })).unwrap_err().contains("the shell answers agent.reach"));

        for (said, wanted) in [
            (json!({"standing": "plain"}), Ok(Standing::Plain)),
            (json!({"standing": "unknown", "reach": reviewer()}), Ok(Standing::Unknown)),
            (json!({"standing": "sideways"}), Err("its answer named a standing this door does not know: `sideways`".to_string())),
            (json!({"standing": "held"}), Err("its answer held a reach this door cannot read".to_string())),
            (json!({"standing": "held", "reach": {"agent": "x"}}), Err("its answer held a reach this door cannot read".to_string())),
            (json!("spent"), Err("its answer named no standing".to_string())),
        ] {
            assert_eq!(Standing::from_json(&said), wanted, "{said}");
        }
    }

    /// Over a real socket: the question goes only to a listener the rule accepts, carries the
    /// digest and never the token, and each answer is read as the shell gave it. A listener the
    /// rule refuses is sent nothing; no listener at all is a refusal naming why.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_door_asks_only_the_shell_and_hears_its_answer() {
        use std::io::{BufRead, BufReader, Write};
        use std::os::unix::net::UnixListener;
        use std::sync::{Arc, Mutex};

        let dir = std::env::temp_dir().join(format!("yantrik-reach-ask-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("app-shell.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let heard = Arc::new(Mutex::new(Vec::<Value>::new()));
        let held = token_digest("tok-held");
        let plain = token_digest("tok-plain");
        {
            let heard = heard.clone();
            let (held, plain) = (held.clone(), plain.clone());
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(stream) = stream else { return };
                    let mut writer = stream.try_clone().unwrap();
                    let mut line = String::new();
                    if BufReader::new(stream).read_line(&mut line).unwrap_or(0) == 0 {
                        continue;
                    }
                    let asked: Value = serde_json::from_str(&line).unwrap();
                    heard.lock().unwrap().push(asked.clone());
                    let digest = asked["params"]["token_sha256"].as_str().unwrap_or("");
                    let standing = if digest == held {
                        Standing::Held(planner())
                    } else if digest == plain {
                        Standing::Plain
                    } else {
                        Standing::Unknown
                    };
                    let reply = json!({"jsonrpc": "2.0", "id": asked["id"], "result": standing.to_json()});
                    let _ = writer.write_all(format!("{reply}\n").as_bytes());
                }
            });
        }
        let address = path.to_string_lossy().to_string();

        let err = ask(&address, answered_by_the_shell, &token_digest("tok-held")).unwrap_err();
        assert!(err.contains("not the desktop's own yantrik-ui, so it was not asked"), "a test binary is not the shell: {err}");
        std::thread::sleep(Duration::from_millis(50));
        assert!(heard.lock().unwrap().is_empty(), "nothing was written to a listener that is not the shell");

        crate::sync_client::SyncRpcClient::clear_breaker(&address);
        fn this_process(peer: Option<PeerCred>) -> Result<(), String> {
            match peer {
                Some(p) if p.pid as u32 == std::process::id() => Ok(()),
                other => Err(format!("unexpected peer {other:?}")),
            }
        }
        assert_eq!(ask(&address, this_process, &held), Ok(Standing::Held(planner())));
        assert_eq!(ask(&address, this_process, &plain), Ok(Standing::Plain));
        assert_eq!(ask(&address, this_process, &token_digest("tok-stopped")), Ok(Standing::Unknown));
        let asked = heard.lock().unwrap().clone();
        assert_eq!(asked.len(), 3);
        assert!(asked.iter().all(|a| a["method"] == ASK && a["params"] == json!({"token_sha256": a["params"]["token_sha256"]})));
        assert!(!serde_json::to_string(&asked).unwrap().contains("tok-"), "the token never left: {asked:?}");

        // Nobody listening: refused, and the sentence says why.
        let _ = std::fs::remove_file(&path);
        let err = decided(ask(&address, this_process, &held)).unwrap_err();
        assert!(err.starts_with("REACH: the shell, which keeps every agent's reach, did not answer (Connection failed ("), "{err}");
        assert!(err.ends_with("so no act carrying an agent token runs until it does. Nothing was run."), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
