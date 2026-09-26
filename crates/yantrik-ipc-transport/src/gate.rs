//! The one rule every `app.act` meets, whoever answers it: the machine's ceiling, the person's
//! mode, and the grant that stands in for their Allow.
//!
//! # Why it is here
//!
//! This lived in `yantrik-app-runtime::control`, where `Registry::act` enforced it for every app
//! window — the one function every `app.act` to a window crosses, whoever sent it (#116, #49).
//! Three services do not cross it. System Monitor, Notifications and Weather answer `app.act` in
//! their own `ServiceHandler` and dispatched the action straight away, so `kill_process` — graded
//! `dangerous` in the service's own surface — ran on any call to its socket: no ceiling, no mode,
//! no grant (#153). `yos act system-monitor kill_process pid=…` with the window closed falls
//! through to the service, and so did a raw JSON-RPC line.
//!
//! A service must not link Slint to be told no, and the check is not pure data either: a grant is
//! spent by a call to the shell. So it lives beside [`SyncRpcClient`], for the reason
//! [`crate::service`] does, and `yantrik_app_runtime::control` re-exports it unchanged. A window
//! and a service now refuse with one function and in the same words.
//!
//! # The order
//!
//! 1. **The ceiling** (`tool_permission` in `settings.yaml`), on the grade alone. Nothing reaches
//!    past it: not a mode, not a grant. `CEILING:`.
//! 2. **The grant**, if the call carries one, spent through the shell — and only once the ceiling
//!    has passed. Spent first, a person's Allow was used up on an act that was then refused for
//!    being above the ceiling, and never ran (#154).
//! 3. **The mode** (`mind-mode.json`, beside the settings): above what it runs unasked, with no
//!    grant spent and no session rule for the action, the call is refused with `GRANT:` and told
//!    how to get one. So is an action whose own published description says it cannot be undone
//!    ([`unrecoverable`]), in every mode but bypass and whatever its grade above `safe`: the
//!    shell's table and the MCP bridge already asked about those, and until this rule moved here
//!    `yos act` or a raw socket ran `calendar.delete_event` in auto with nobody asked (map gap 4
//!    of the surface SDK). A session rule never covers one, and in plan mode no session rule
//!    covers anything — plan raises no card, so there is no standing answer to one.
//!
//! The whole table is generated into `deploy/yantrik-os/surface-vectors.json` by this module's
//! tests, and every other implementation of it replays that file.
//!
//! [`permit`] is all three, for a caller that holds the grade where it holds the call — a
//! service. A window cannot: its grades live on the UI thread and file and socket IO does not
//! belong there, so its RPC thread spends the grant with [`Authority::spend`] and its UI thread
//! decides with [`decide`]. Same steps, same order, same sentences.
//!
//! `describe` never comes here. Reading an app is free.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::SyncRpcClient;

// ── The ceiling ─────────────────────────────────────────────────────

/// The grades an action can carry, lowest first. The same ladder the MCP bridge and the
/// companion's `parse_permission` use; held as strings because an `Action`'s own `permission` is
/// a `&'static str`.
pub const LADDER: [&str; 4] = ["safe", "standard", "sensitive", "dangerous"];

/// Where a grade sits on [`LADDER`], or `None` if it is not a level this OS defines.
pub fn grade(permission: &str) -> Option<usize> {
    LADDER.iter().position(|g| *g == permission)
}

/// Whether a caller capped at `cap` may use an action graded `graded` — the ceiling's comparison
/// alone, for a caller that holds a cap of its own rather than the machine's. The companion's
/// `app_action` asks this against `tools.max_permission` before it sends anything, so the ladder
/// it reads is this one and not a copy.
///
/// `None` when `graded` is not a level this OS defines: that is refused, never waved through. A
/// cap off the ladder reads as [`DEFAULT_CEILING`], as the machine's does.
pub fn permits(cap: &str, graded: &str) -> Option<bool> {
    let level = grade(graded)?;
    let cap = grade(cap).unwrap_or_else(|| grade(DEFAULT_CEILING).unwrap());
    Some(level <= cap)
}

// ── What cannot be undone ───────────────────────────────────────────

/// The wording that makes an action's own description a promise that it cannot be taken back.
///
/// Lowercased, matched as substrings. The same seven, in the same order, the shell's
/// `approvals::unrecoverable` used (it now asks this function) and the MCP bridge's
/// `UNRECOVERABLE_PHRASES` carries; `surface-vectors.json` publishes the list so a port can check
/// its own copy phrase for phrase.
pub const UNRECOVERABLE_PHRASES: [&str; 7] = [
    "not recoverable",
    "cannot be undone",
    "can't be undone",
    "irreversible",
    "permanently",
    "permanent",
    "no undo",
];

/// Does the app's own sentence about an action say it cannot be taken back?
///
/// The grade ladder has no rung for "recoverable", and `calendar.delete_event` — graded
/// `sensitive`, published as "It is not recoverable" — is the action that showed it matters: the
/// mode menu promises auto still asks about the destructive ones. So the sentence decides too, in
/// [`decide`], on every door. One reading of the sentence for the whole machine: the shell's card
/// draws its red warning line from this, and refuses to mint a session rule on the strength of it.
pub fn unrecoverable(purpose: &str) -> bool {
    let lower = purpose.to_lowercase();
    UNRECOVERABLE_PHRASES.iter().any(|phrase| lower.contains(phrase))
}

/// The ceiling used when `settings.yaml` is missing, unreadable, or says nothing usable —
/// the same default the shell's own `UserSettings` carries, so a machine that has never
/// opened Settings behaves the way Settings would show it.
pub const DEFAULT_CEILING: &str = "sensitive";

/// Path of the shell's settings file. The ceiling is read from it here, and the theme from it in
/// `yantrik_app_runtime::theme`, which asks this function so the two cannot name different files.
pub fn settings_path() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".config/yantrik/settings.yaml")
}

/// The machine's ceiling for programmatic callers, from the shell's settings file.
///
/// Read per call rather than cached at startup: the whole point of the setting is that a
/// person can tighten it while apps are running, and a boundary that only notices at launch
/// is a boundary the Settings screen lies about. The file is a few hundred bytes and an
/// `act` happens at human-or-model speed, so the read costs nothing that matters.
pub fn configured_ceiling() -> String {
    let Ok(text) = std::fs::read_to_string(settings_path()) else {
        return DEFAULT_CEILING.to_string();
    };
    ceiling_from(&text)
}

/// Pull `tool_permission` out of settings text. Only that key is parsed, for the same reason
/// the theme only parses its two: the rest of the file is the shell's business.
pub fn ceiling_from(text: &str) -> String {
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else { continue };
        if key.trim() != "tool_permission" {
            continue;
        }
        let value = value.trim().trim_matches('"').trim_matches('\'');
        if grade(value).is_some() {
            return value.to_string();
        }
        tracing::warn!(value = %value, "tool_permission is not a grade; using {DEFAULT_CEILING}");
        return DEFAULT_CEILING.to_string();
    }
    DEFAULT_CEILING.to_string()
}

// ── The mode, and the grant that stands in for it ───────────────────
//
// The ceiling is the machine's wall. Under it the PERSON has a mode — plan, ask, auto or bypass
// — that says what a caller may do without being asked, and for a while the mode lived only in
// the shell and the MCP bridge: the bridge read it off `describe shell`, raised a card when the
// mode said to, and ran the action once the person had pressed Allow. Nothing else did. `yos
// act` and a raw JSON-RPC client on the socket ran a `sensitive` action in `ask` mode with no
// card and no record (issues #49 and #116).
//
// So the mode is read here too, the way the ceiling is: the shell writes it to a small file
// beside `settings.yaml` whenever it changes (`mind_mode::publish_policy_file` in the shell), and
// every dispatch reads it per call. The file also names the shell that wrote it — its pid, the
// start time the kernel gives that pid, and the boot the machine was in — and a file whose shell
// is not running reads as `ask`: a shell that died in bypass, or with "allow for this session"
// rules, must not keep either in force until the next shell start happens to rewrite the file
// (#154). The boot id is the part a reboot cannot leave standing: the file itself survives one
// on disk, and in theory the kernel could hand a new process the same pid at the same start
// tick, so without it the old shell's name could still match (#333).
//
// A call above what the mode allows must carry a GRANT — the
// `request_id` the shell's `request_approval` minted and a person's Allow turned into one — and
// the dispatch spends it through the shell's `consume_approval` before the handler runs. The
// bridge and `yos act` ask for the card on the caller's behalf; a raw client can do the same
// three steps itself. Whichever door a call came through, it meets the same question.
//
// What an app learns from all of this is one bit: a grant was, or was not, attached and spent.
// The card, the countdown and the store are the shell's.

/// The file the shell publishes the mode in, beside the settings file.
pub const MODE_FILE: &str = "mind-mode.json";

/// The modes a desktop can be in, strictest first, and what each runs without asking: the
/// highest grade on [`LADDER`] a caller may use with no grant. One column of the table in the
/// shell's `mind_mode::Modes::decide` and the bridge's `decide`, which stay the definition.
pub const MODES: [(&str, &str); 4] =
    [("plan", "safe"), ("ask", "standard"), ("auto", "sensitive"), ("bypass", "dangerous")];

/// What the dispatch runs without a grant in every mode, plan included.
///
/// Plan mode's own column says `safe`, and the bridge enforces that for a mind on it. The
/// dispatch cannot: the desktop's own processes call `standard` actions on these sockets to work
/// at all — every app's notifications and Calendar's and Email's services are started on demand
/// through the shell's `start_service`, and a second launch of an editor hands its file to the
/// open window with `open` — and nothing here can tell those callers from a mind until the
/// socket carries identity (#43). Refusing them would stop the person's own desktop working the
/// moment they chose plan for the mind. Everything above this still needs a grant in plan, and
/// the shell mints none there.
pub const SOCKET_FLOOR: &str = "standard";

/// The mode assumed when the shell has published nothing usable: `ask`, the strictest mode that
/// still lets ordinary work happen and the one the shell itself boots into. A missing file is not
/// a permission, so this fails closed, exactly as the bridge does when `describe shell` says
/// nothing about the mode.
pub const DEFAULT_MODE: &str = "ask";

/// Where the shell publishes the mode.
pub fn mode_path() -> PathBuf {
    settings_path().with_file_name(MODE_FILE)
}

/// The mode as the shell last published it: its name, and the session rules beside it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mode {
    pub name: String,
    /// `(app, action)` pairs a person allowed for the rest of the session from the card's
    /// "Allow for this session". A rule covers any arguments, but only its own action.
    pub session_rules: Vec<(String, String)>,
}

impl Mode {
    pub fn named(name: &str) -> Mode {
        Mode { name: name.to_string(), session_rules: Vec::new() }
    }

    /// The highest grade this mode runs unasked, as a position on [`LADDER`]. A name that is
    /// not a mode reads as `ask`, never as something looser.
    pub fn allows(&self) -> usize {
        MODES
            .iter()
            .find(|(name, _)| *name == self.name)
            .and_then(|(_, top)| grade(top))
            .unwrap_or_else(|| grade("standard").unwrap())
    }

    /// Whether a session rule is the person's standing answer for `app.action`.
    pub fn covers(&self, app: &str, action: &str) -> bool {
        self.session_rules.iter().any(|(a, x)| a == app && x == action)
    }
}

/// The mode right now, from the file the shell writes. Read per call for the reason the ceiling
/// is: a person changes the mode from the chip while apps are running, and a dispatch that read
/// it once at launch would be enforcing a mode the chip no longer shows.
pub fn configured_mode() -> Mode {
    let Ok(text) = std::fs::read_to_string(mode_path()) else {
        return Mode::named(DEFAULT_MODE);
    };
    mode_from(&text, unix_now())
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// The start time of `pid` — field 22 of `/proc/<pid>/stat`, clock ticks since boot — or
/// `None` when there is no such process or no `/proc` to ask.
///
/// A pid on its own says nothing: the kernel reuses them, and a recycled pid would resurrect a
/// dead shell's mode. A pid and the start time it was recorded with name one process, because
/// whatever reuses the pid does not also reuse the boot tick it started at.
pub fn proc_start_ticks(pid: u32) -> Option<u64> {
    proc_stat(pid).map(|(_, start)| start)
}

/// `pid`'s state character (field 3 of `/proc/<pid>/stat`) and start time (field 22), or `None`
/// when there is no such process or no `/proc` to ask.
fn proc_stat(pid: u32) -> Option<(char, u64)> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // Field 2, the command name, may hold spaces and parentheses — a shell called `(tmux)` is
    // one field — so the fields are counted from the LAST `)`, which closes it. The token after
    // that is field 3, the state, and starttime is field 22: index 19 from there.
    let after_comm = stat.rsplit_once(')')?.1;
    let mut fields = after_comm.split_whitespace();
    let state = fields.next()?.chars().next()?;
    let start = fields.nth(18)?.parse().ok()?;
    Some((state, start))
}

/// The boot this machine is in — `/proc/sys/kernel/random/boot_id` — or `None` when there is no
/// `/proc` to ask. The kernel picks a fresh random id on every boot, so an identity recorded
/// under a different one names a machine that has since restarted (#333).
pub fn boot_id() -> Option<String> {
    std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|id| !id.is_empty())
}

/// Whether the shell that wrote `doc` is the process still running under that pid, in this boot.
///
/// A file that names no shell — one an older shell wrote, or a program wrote by hand — reads as
/// it always did: anybody who can write this file can write any mode into it, so demanding an
/// identity from such a writer would close no door the same-uid limit leaves open (#154, item 5).
/// A file that DOES name one is trusted only while that shell runs, and less than the whole
/// identity — a pid with no start time, or a file from before the boot id existed with no boot
/// to tie the pair to — names no process anybody can find alive, so it fails closed like a dead
/// one.
fn names_a_live_shell(doc: &serde_json::Value) -> bool {
    let pid = doc.get("shell_pid").and_then(|v| v.as_u64());
    let start = doc.get("shell_start_ticks").and_then(|v| v.as_u64());
    let boot = doc.get("boot_id").and_then(|v| v.as_str());
    let (Some(pid), Some(start), Some(boot)) = (pid, start, boot) else {
        return pid.is_none() && start.is_none() && boot.is_none();
    };
    let Ok(pid) = u32::try_from(pid) else { return false };
    let Some(this_boot) = boot_id() else { return false };
    if boot.trim() != this_boot {
        return false;
    }
    match proc_stat(pid) {
        // A zombie has exited but not been reaped: it keeps its pid and its start time in
        // /proc, and the shell behind them is gone all the same. `X` is the kernel's own
        // "dead", which some kernels show instead of removing the entry.
        Some((state, started)) => started == start && state != 'Z' && state != 'X',
        None => false,
    }
}

/// Read the mode out of what the shell wrote. Public so the shell's own test can prove that
/// what it writes is what every app will read.
///
/// `now_unix` is for a bypass. The shell folds an expired bypass back on its own tick and
/// rewrites the file, but a shell that crashed mid-bypass leaves a file saying `bypass` with
/// nobody left to fold it — so the file carries when the bypass ends and this honours it. A
/// bypass "until restart" carries no end and is trusted while the shell that wrote the file is
/// running: the file names that shell and the boot it wrote in, and this checks the name
/// against the process table and the machine's boot id, so a shell that died — and a machine
/// that rebooted — leave `ask` behind rather than its last mode (#154, #333).
pub fn mode_from(text: &str, now_unix: u64) -> Mode {
    let Ok(doc) = serde_json::from_str::<serde_json::Value>(text) else {
        tracing::warn!("{MODE_FILE} is not JSON; using {DEFAULT_MODE}");
        return Mode::named(DEFAULT_MODE);
    };
    if !names_a_live_shell(&doc) {
        // The rules go with the shell: "allow for this session" was an answer to cards a
        // process that is gone will never raise again.
        tracing::warn!("{MODE_FILE} names a shell that is not running; using {DEFAULT_MODE}");
        return Mode::named(DEFAULT_MODE);
    }
    let is_mode = |name: &str| MODES.iter().any(|(m, _)| *m == name);
    let mut name = doc["mode"].as_str().unwrap_or("").to_string();
    if !is_mode(&name) {
        tracing::warn!(mode = %name, "{MODE_FILE} names no mode this OS defines; using {DEFAULT_MODE}");
        name = DEFAULT_MODE.to_string();
    }
    if name == "bypass" {
        if let Some(until) = doc["bypass_expires_unix"].as_u64() {
            if now_unix >= until {
                let previous = doc["previous"].as_str().unwrap_or(DEFAULT_MODE);
                name = if is_mode(previous) && previous != "bypass" {
                    previous.to_string()
                } else {
                    DEFAULT_MODE.to_string()
                };
            }
        }
    }
    let session_rules = doc["session_rules"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|r| {
                    Some((r["app"].as_str()?.to_string(), r["action"].as_str()?.to_string()))
                })
                .collect()
        })
        .unwrap_or_default();
    Mode { name, session_rules }
}

// ── Spending a grant ────────────────────────────────────────────────

/// How this process spends a grant: the token and the exact triple in, and either it is burned
/// or the reason it was not.
type Spender = dyn Fn(&str, &str, &str, &serde_json::Value) -> Result<(), String> + Send + Sync;

static SPENDER: OnceLock<Box<Spender>> = OnceLock::new();

/// How long a dispatch waits for the shell to spend a grant. One hop to the shell's UI thread
/// and back; anything slower is a shell that is not answering, and the honest outcome then is
/// a refusal that says so, not a handler that ran on a grant nobody checked.
const GRANT_ROUNDTRIP: Duration = Duration::from_secs(5);

/// The shell's control surface, where grants are kept and spent.
const SHELL: &str = "app-shell";

/// Install the function this process spends grants with.
///
/// The shell calls this once, with its own `approvals::consume`, because the shell IS the store
/// — and asking itself over its own socket from its own RPC thread is a call that cannot be
/// answered until the call returns. Every other process leaves it unset and spends grants over
/// the shell's socket. A second call changes nothing: the store does not move.
pub fn spend_grants_with(
    spend: impl Fn(&str, &str, &str, &serde_json::Value) -> Result<(), String>
        + Send
        + Sync
        + 'static,
) {
    let _ = SPENDER.set(Box::new(spend));
}

/// Burn `id` for exactly `app.action(args)`, or say why it could not be.
///
/// Through the shell's published `consume_approval`, which is what the bridge used to call
/// itself before running the action. The check is the shell's — granted, unspent, unexpired,
/// bound to this app, this action and these arguments — and the refusal is the shell's own
/// sentence, which already names the part that differed.
///
/// And only to the shell. Whatever answers `consume_approval` decides whether a person's Allow
/// stands behind this call, so before the grant is written to the socket the process listening on
/// it must be a `yantrik-ui` binary (`owner::must_be_the_shell`, from `SO_PEERCRED` and
/// `/proc/<pid>/exe`). Anything else that bound `app-shell.sock` is refused and never sees it.
fn spend_grant(id: &str, app: &str, action: &str, args: &serde_json::Value) -> Result<(), String> {
    if let Some(spend) = SPENDER.get() {
        return spend(id, app, action, args);
    }
    SyncRpcClient::for_service(SHELL)
        .with_timeout(GRANT_ROUNDTRIP)
        .expecting_peer(crate::owner::must_be_the_shell)
        .call(
            "app.act",
            serde_json::json!({
                "action": "consume_approval",
                "args": { "request_id": id, "app": app, "action": action, "args_json": args },
            }),
        )
        .map(|_| ())
        .map_err(|e| e.message)
}

/// The grant an `app.act` call carries: the `request_id` the shell answered `request_approval`
/// with, once a person has pressed Allow. Optional, and deliberately so — most calls need none —
/// and an empty one is none.
pub fn grant_of(params: &serde_json::Value) -> Option<String> {
    params
        .get("grant")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|g| !g.is_empty())
        .map(str::to_string)
}

/// The key an agent token travels under: beside `args` on `app.act`, never inside them.
///
/// A mind running as one of the person's agents carries a token its harness was given. `args` is
/// what gets shown and kept — the approval card draws it, the audit log writes it, and a grant is
/// bound to it here — so a token among them is a token anyone reading the screen or the log can
/// replay. What the token is worth is the handler's business; the dispatch only carries it (see
/// `yantrik_app_runtime::control::agent_token`).
pub const AGENT_TOKEN: &str = "agent_token";

/// The token a call carries, from beside its `args` — and any copy inside `args` taken out.
///
/// Call it before anything reads `args`, and before a grant is spent against them: what a grant
/// is bound to is the arguments, and a token is not one. The copy inside is removed and NOT used.
/// Defence in depth: whatever put it there has already shown it to anything that prints the
/// arguments, and honouring it would teach callers that the arguments are a place a token may go.
pub fn agent_token_of(params: &serde_json::Value, args: &mut serde_json::Value) -> Option<String> {
    if args.as_object_mut().and_then(|given| given.remove(AGENT_TOKEN)).is_some() {
        tracing::warn!(
            "an agent token arrived inside `args`; it was removed and not used. It travels beside \
             `args` on app.act, never among them"
        );
    }
    params
        .get(AGENT_TOKEN)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
}

// ── The decision ────────────────────────────────────────────────────

/// What is known about one call before its action runs: the ceiling and the mode as the files
/// say them, and whether a grant was attached and spent.
#[derive(Clone, Debug)]
pub struct Authority {
    pub ceiling: String,
    pub mode: Mode,
    /// A grant was attached to the call and the shell spent it. Never true for a grant the
    /// shell refused: that refusal ends the call.
    pub granted: bool,
}

impl Authority {
    /// The ceiling and the mode as the files say them now, and no grant yet.
    ///
    /// Both are IO. A window builds this on its RPC thread, never on the one that paints; a
    /// service builds it in its handler. Tests build the struct instead, so the machine running
    /// them lends them neither its ceiling nor its mode.
    pub fn now() -> Authority {
        Authority { ceiling: configured_ceiling(), mode: configured_mode(), granted: false }
    }

    /// Spend grant `id` for exactly `app_id.action(args)`, whose surface grades it `graded` —
    /// but only if the ceiling lets that grade be used at all.
    ///
    /// The ceiling first, because a grant spent on an act the ceiling then refuses is a
    /// person's Allow used up on nothing (#154): the card said yes, the act never ran, and the
    /// grant cannot be offered again. So above the ceiling this answers with the ceiling's own
    /// refusal and the grant is left for the shell to hold. Any grant attached is spent once the
    /// ceiling passes, whether or not the mode would have asked: a replayed, swapped or invented
    /// grant ends the call here, in the shell's words, rather than being ignored.
    pub fn spend(
        &mut self,
        id: &str,
        app_id: &str,
        action: &str,
        graded: &str,
        args: &serde_json::Value,
    ) -> Result<(), String> {
        within_ceiling(&self.ceiling, app_id, action, graded)?;
        spend_grant(id, app_id, action, args).map_err(|why| {
            format!(
                "GRANT: `{id}` does not authorise {app_id}.{action} — {why} Nothing was run; \
                 a grant covers one action, once, with the arguments the person was shown."
            )
        })?;
        self.granted = true;
        Ok(())
    }
}

/// May `app_id.action`, graded `graded` and described as `purpose` by the surface that offers
/// it, run under `authority`?
///
/// The ceiling, then the mode — on the grade and the action's own description alone, before the
/// arguments, the revision guard or the handler, because "may this caller use this action at
/// all" is a question about the action, and answering a narrower question first would mean doing
/// work for a call that was never allowed. Pure: no file is read and nothing is spent here, so a
/// window's UI thread can call it inside the same turn of the event loop as the handler.
///
/// `purpose` is the description the surface publishes for the action — the sentence a person
/// reads on the approval card — not anything the caller sent. [`unrecoverable`] reads it.
pub fn decide(
    authority: &Authority,
    app_id: &str,
    action: &str,
    graded: &str,
    purpose: &str,
) -> Result<(), String> {
    let level = within_ceiling(&authority.ceiling, app_id, action, graded)?;

    // The mode, and the grant that stands in for it. After the ceiling — no mode and no grant
    // reaches past that. A grant is a person's Allow for exactly this call, and it answers every
    // question below. Nothing here asks anybody: raising the card is the shell's, and the
    // caller's job is to have done it (`yos act` does it for a caller that has not).
    if authority.granted {
        return Ok(());
    }
    let mode = &authority.mode;
    let everything = LADDER.len() - 1;

    // The app's own sentence, and the one input here that is not a grade. `safe` is excluded:
    // a read destroys nothing, so wording that happens to match cannot turn a look into a
    // question. The shell's `Modes::decide` and the bridge's `decide` draw the same line.
    let irreversible = level > 0 && unrecoverable(purpose);

    // Bypass runs everything under the ceiling — "Stop asking me anything" is an answer already.
    // Every other mode runs what its column says, never less than the socket floor, and asks
    // about anything the app says cannot be undone.
    let asks = mode.allows() < everything
        && (irreversible || level > mode.allows().max(grade(SOCKET_FLOOR).unwrap()));
    if !asks {
        return Ok(());
    }

    // A session rule is the person's standing answer for this one action and covers it the way a
    // grant would — except for an action that cannot be undone, which the card never offers a
    // rule for, and except in plan mode, which raises no card and so has no standing answers.
    let plan = mode.allows() == 0;
    if !plan && !irreversible && mode.covers(app_id, action) {
        return Ok(());
    }
    Err(grant_refusal(app_id, action, graded, mode, irreversible))
}

/// The whole rule for one call, for a caller that holds the grade where it holds the call.
///
/// A service answering `app.act` in its own handler calls this before it dispatches, with the
/// grade and description from the same table it hands `describe_json` — so what a caller is shown
/// is what is enforced. The ceiling, then the grant (spent only past the ceiling), then the mode:
/// the steps a window's dispatch takes, in its order, with its sentences.
pub fn permit(
    authority: &mut Authority,
    app_id: &str,
    action: &str,
    graded: &str,
    purpose: &str,
    args: &serde_json::Value,
    grant: Option<&str>,
) -> Result<(), String> {
    if let Some(id) = grant {
        authority.spend(id, app_id, action, graded, args)?;
    }
    decide(authority, app_id, action, graded, purpose)
}

/// Where `graded` sits on the ladder, or the ceiling's refusal. An unrecognised ceiling falls
/// back to the default rather than failing open — the choice the companion's `parse_permission`
/// makes — and an unrecognised grade is refused rather than waved through: a typo in a
/// `.risk(...)` must fail closed, or the typo silently becomes an exemption.
fn within_ceiling(ceiling: &str, app_id: &str, action: &str, graded: &str) -> Result<usize, String> {
    let Some(within) = permits(ceiling, graded) else {
        return Err(format!(
            "CEILING: {}.{} is graded `{}`, which is not a level this OS defines ({}), \
             so it was not run.",
            app_id,
            action,
            graded,
            LADDER.join(" < ")
        ));
    };
    let level = grade(graded).unwrap_or_default();
    if !within {
        return Err(format!(
            "CEILING: {app_id}.{action} is graded `{graded}`, above this machine's `{ceiling}` \
             ceiling (`tool_permission` in ~/.config/yantrik/settings.yaml), so it was not \
             run. An action at that grade needs a person to authorise it directly — raise \
             the ceiling in Settings if that is the intent."
        ));
    }
    Ok(level)
}

/// The refusal for a call above what the mode allows, with no grant to stand in for it.
///
/// It says how to get one, because the caller reading it is usually a program — `yos`, or a
/// mind with a terminal — and "no" without a way forward is what teaches a program to look for
/// another door. `GRANT:` in front so a caller can branch on it the way it branches on
/// `CEILING:` and `STALE:`; `yos act` does, and asks on the caller's behalf. Every variant says
/// "graded `<grade>`", which is where `yos act` reads the grade to ask with.
///
/// Four sentences: plan or not, and whether the reason is the grade or the app's own word that
/// the action cannot be undone. The second reason is named when it applies, because it is the
/// one a session rule does not answer — a caller holding a rule for the action needs to know why
/// the rule did not cover it.
fn grant_refusal(app: &str, action: &str, graded: &str, mode: &Mode, irreversible: bool) -> String {
    const HOW: &str = "Ask the shell for approval first (`request_approval` with this app, action \
         and these exact arguments, poll `approval_status`, then send the granted request_id as \
         `grant` on app.act — `yos act` does all of that for you), or have the person at the \
         machine press Allow when the card appears.";
    const PLAN: &str = "Say what you would do and let the person decide; they switch the mode \
         from the chip in the status bar.";
    let final_word = "its own description says it cannot be undone";
    match (mode.name == "plan", irreversible) {
        (true, false) => format!(
            "GRANT: {app}.{action} is graded `{graded}` and this machine is in plan mode, which \
             raises no card for anything above `{SOCKET_FLOOR}` — so it was not run. {PLAN}"
        ),
        (true, true) => format!(
            "GRANT: {app}.{action} is graded `{graded}` and {final_word}, and this machine is in \
             plan mode, which raises no card for that — so it was not run. {PLAN}"
        ),
        (false, false) => format!(
            "GRANT: {app}.{action} is graded `{graded}` and this machine is in {mode} mode, which \
             runs nothing above `{allowed}` without asking — so it was not run. {HOW}",
            mode = mode.name,
            allowed = LADDER[mode.allows().max(grade(SOCKET_FLOOR).unwrap())],
        ),
        (false, true) => format!(
            "GRANT: {app}.{action} is graded `{graded}` and {final_word}, and this machine is in \
             {mode} mode, which asks before anything that cannot be undone — so it was not run. \
             {HOW}",
            mode = mode.name,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(ceiling: &str, mode: &str) -> Authority {
        Authority { ceiling: ceiling.into(), mode: Mode::named(mode), granted: false }
    }

    /// What System Monitor publishes for `kill_process`. Recoverable wording, so these tests are
    /// about the grade; the description's own rule has tests of its own below.
    const KILL: &str = "End a running process by pid";

    /// A stand-in for the shell's store: `ok-*` ids hold once, for exactly
    /// `system-monitor.kill_process {"pid": 42}`; anything else is refused in the shell's words.
    /// Installed once, because the spender is process-wide as the shell's is.
    fn spend_through_a_stand_in_shell() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            let spent = std::sync::Mutex::new(std::collections::HashSet::<String>::new());
            spend_grants_with(move |id, app, action, args| {
                if !id.starts_with("ok-") {
                    return Err(format!("no approval request `{id}`."));
                }
                if app != "system-monitor" || action != "kill_process" || *args != serde_json::json!({"pid": 42}) {
                    return Err(format!("`{id}` was approved for another call, and this call carries {args}."));
                }
                let mut spent = spent.lock().unwrap_or_else(|e| e.into_inner());
                if !spent.insert(id.to_string()) {
                    return Err(format!("`{id}` was already used."));
                }
                Ok(())
            });
        });
    }

    #[test]
    fn the_order_is_ceiling_then_mode_and_each_says_which_it_was() {
        let err = decide(&at("sensitive", "bypass"), "system-monitor", "kill_process", "dangerous", KILL).unwrap_err();
        assert!(err.starts_with("CEILING:") && err.contains("above this machine's `sensitive`"), "{err}");

        let err = decide(&at("dangerous", "ask"), "system-monitor", "kill_process", "dangerous", KILL).unwrap_err();
        assert!(err.starts_with("GRANT:") && err.contains("ask mode"), "{err}");

        assert!(decide(&at("dangerous", "bypass"), "system-monitor", "kill_process", "dangerous", KILL).is_ok());
        let mut granted = at("dangerous", "ask");
        granted.granted = true;
        assert!(decide(&granted, "system-monitor", "kill_process", "dangerous", KILL).is_ok());
    }

    #[test]
    fn standard_is_the_floor_in_every_mode_and_the_ceiling_still_binds_it() {
        for mode in ["plan", "ask", "auto", "bypass"] {
            assert!(decide(&at("sensitive", mode), "notifications", "notify", "standard", "Post a notification").is_ok(), "{mode}");
        }
        let err = decide(&at("safe", "bypass"), "notifications", "notify", "standard", "Post a notification").unwrap_err();
        assert!(err.starts_with("CEILING:"), "{err}");
    }

    #[test]
    fn a_grade_off_the_ladder_is_refused_whatever_the_ceiling() {
        let err = decide(&at("dangerous", "bypass"), "weather", "set_location", "catastrophic", "").unwrap_err();
        assert!(err.starts_with("CEILING:") && err.contains("not a level this OS defines"), "{err}");
    }

    /// Map gap 4 of the surface SDK, and the defect of 21 September one door further in:
    /// `calendar.delete_event` is `sensitive` and says "It is not recoverable". The shell and the
    /// bridge asked about it in auto; the dispatch ran it, so `yos act` or a raw socket did too.
    #[test]
    fn what_its_own_description_says_cannot_be_undone_is_asked_about_on_every_door() {
        let delete = "Take an event off the calendar. It is not recoverable";
        let update = "Change an event's title, time or notes";

        assert!(decide(&at("sensitive", "auto"), "calendar", "update_event", "sensitive", update).is_ok());
        let err = decide(&at("sensitive", "auto"), "calendar", "delete_event", "sensitive", delete).unwrap_err();
        assert_eq!(
            err,
            "GRANT: calendar.delete_event is graded `sensitive` and its own description says it \
             cannot be undone, and this machine is in auto mode, which asks before anything that \
             cannot be undone — so it was not run. Ask the shell for approval first \
             (`request_approval` with this app, action and these exact arguments, poll \
             `approval_status`, then send the granted request_id as `grant` on app.act — `yos act` \
             does all of that for you), or have the person at the machine press Allow when the card \
             appears."
        );

        // Below the floor's grade too: a `standard` action that says so is asked about in ask.
        let err = decide(&at("sensitive", "ask"), "blender", "delete_object", "standard",
                         "Delete an object. Past that undo it is not recoverable.").unwrap_err();
        assert!(err.starts_with("GRANT:") && err.contains("cannot be undone"), "{err}");

        // Bypass asks nobody, a grant answers it, and a `safe` read is never turned into a card.
        assert!(decide(&at("sensitive", "bypass"), "calendar", "delete_event", "sensitive", delete).is_ok());
        let mut granted = at("sensitive", "auto");
        granted.granted = true;
        assert!(decide(&granted, "calendar", "delete_event", "sensitive", delete).is_ok());
        assert!(decide(&at("sensitive", "plan"), "files", "describe_trash", "safe",
                       "Lists what was deleted permanently").is_ok());
    }

    #[test]
    fn a_session_rule_never_covers_what_cannot_be_undone_nor_anything_in_plan() {
        let delete = "Take an event off the calendar. It is not recoverable";
        let with_rule = |mode: &str, action: &str| Authority {
            ceiling: "dangerous".into(),
            mode: Mode { name: mode.into(), session_rules: vec![("calendar".into(), action.into())] },
            granted: false,
        };
        assert!(decide(&with_rule("ask", "update_event"), "calendar", "update_event", "sensitive", "Move it").is_ok());
        let err = decide(&with_rule("ask", "delete_event"), "calendar", "delete_event", "sensitive", delete).unwrap_err();
        assert!(err.contains("cannot be undone"), "the rule did not cover it and the refusal says why: {err}");
        let err = decide(&with_rule("plan", "update_event"), "calendar", "update_event", "sensitive", "Move it").unwrap_err();
        assert!(err.starts_with("GRANT:") && err.contains("plan mode"), "{err}");
    }

    #[test]
    fn the_phrases_are_read_the_way_the_shell_and_the_bridge_read_them() {
        assert!(unrecoverable("Take an event off the calendar. It is not recoverable"));
        assert!(unrecoverable("THIS CANNOT BE UNDONE"));
        assert!(unrecoverable("there is no undo to argue with"));
        assert!(!unrecoverable("Move a file or folder to recoverable Trash"));
        assert!(!unrecoverable(""));
        assert_eq!(UNRECOVERABLE_PHRASES.len(), 7);
    }

    #[test]
    fn a_cap_is_compared_on_this_ladder() {
        assert_eq!(permits("standard", "safe"), Some(true));
        assert_eq!(permits("standard", "dangerous"), Some(false));
        assert_eq!(permits("nonsense", "sensitive"), Some(true), "an unreadable cap is the default, sensitive");
        assert_eq!(permits("nonsense", "dangerous"), Some(false));
        assert_eq!(permits("dangerous", "spicy"), None, "a grade off the ladder is never within anything");
    }

    /// #154, item 2: a grant was spent and then the ceiling refused the act, so the person's
    /// Allow was used up on something that never ran. The ceiling comes first now; the same
    /// grant, offered again once the ceiling allows the act, still holds.
    #[test]
    fn a_grant_is_not_spent_on_an_act_the_ceiling_refuses() {
        spend_through_a_stand_in_shell();
        let args = serde_json::json!({"pid": 42});

        let mut tight = at("sensitive", "ask");
        let err = permit(&mut tight, "system-monitor", "kill_process", "dangerous", KILL, &args, Some("ok-154"))
            .unwrap_err();
        assert!(err.starts_with("CEILING:"), "the ceiling's refusal, not the grant's: {err}");
        assert!(!tight.granted);

        let mut raised = at("dangerous", "ask");
        permit(&mut raised, "system-monitor", "kill_process", "dangerous", KILL, &args, Some("ok-154"))
            .expect("the grant was left unspent by the refusal, so it holds now");
        assert!(raised.granted);

        let err = permit(&mut at("dangerous", "ask"), "system-monitor", "kill_process", "dangerous", KILL, &args, Some("ok-154"))
            .unwrap_err();
        assert!(err.starts_with("GRANT:") && err.contains("already used"), "and holds once: {err}");
    }

    #[test]
    fn a_grant_that_does_not_hold_ends_the_call_in_the_shells_words() {
        spend_through_a_stand_in_shell();
        let err = permit(&mut at("dangerous", "bypass"), "system-monitor", "kill_process", "dangerous", KILL,
                         &serde_json::json!({"pid": 42}), Some("made-up"))
            .unwrap_err();
        assert!(err.starts_with("GRANT: `made-up` does not authorise system-monitor.kill_process"), "{err}");
        assert!(err.contains("no approval request"), "{err}");
    }

    #[test]
    fn a_grant_rides_on_the_call_as_text_and_an_empty_one_is_none() {
        assert_eq!(grant_of(&serde_json::json!({"grant": " appr-7 "})), Some("appr-7".into()));
        assert_eq!(grant_of(&serde_json::json!({"grant": ""})), None);
        assert_eq!(grant_of(&serde_json::json!({"grant": 7})), None);
        assert_eq!(grant_of(&serde_json::json!({})), None);
    }

    /// A token among the arguments is taken out before a grant is spent against them: the shell
    /// is handed the arguments alone, and the token beside them is what the call carries.
    #[test]
    fn a_grant_is_spent_against_the_arguments_without_an_agent_token() {
        spend_through_a_stand_in_shell();
        let params = serde_json::json!({
            "action": "kill_process",
            "args": { "pid": 42, "agent_token": "smuggled" },
            "agent_token": "tok-1",
            "grant": "ok-token",
        });
        let mut args = params["args"].clone();
        assert_eq!(agent_token_of(&params, &mut args).as_deref(), Some("tok-1"));
        assert_eq!(args, serde_json::json!({"pid": 42}));
        let mut authority = at("dangerous", "ask");
        permit(&mut authority, "system-monitor", "kill_process", "dangerous", KILL, &args, grant_of(&params).as_deref())
            .expect("bound to {\"pid\": 42}, which is what the shell was handed");
        assert!(authority.granted);
    }

    #[test]
    fn the_mode_file_sits_beside_the_settings_file() {
        assert_eq!(mode_path().parent(), settings_path().parent());
        assert!(mode_path().ends_with(MODE_FILE));
    }

    /// #154, item 1: a shell that died in bypass — or with "allow for this session" rules —
    /// left its last mode in the file, and nothing rewrote it until the next shell start. The
    /// file names the shell that wrote it, and a name that is not running reads as `ask`.
    #[test]
    fn a_mode_file_that_names_a_dead_shell_reads_as_ask() {
        // The dead pid is deterministic, not a guess at the process table: a child that has
        // been waited is gone from /proc, and if the kernel hands the pid out again, the start
        // time recorded here belongs to the child that was reaped, so the pair still names
        // nothing alive. Nothing sleeps and nothing races.
        let mut child = std::process::Command::new("true").spawn().expect("a child to reap");
        let started = proc_start_ticks(child.id()).expect("a running child has a start time");
        child.wait().expect("the child can be waited");
        let boot = boot_id().expect("this machine has booted");

        let dead = serde_json::json!({
            "mode": "bypass",
            "previous": "auto",
            "bypass_expires_unix": null,
            "shell_pid": child.id(),
            "shell_start_ticks": started,
            "boot_id": boot,
            "session_rules": [{"app": "calendar", "action": "delete_event"}],
        });
        let read = mode_from(&dead.to_string(), 0);
        assert_eq!(read.name, DEFAULT_MODE, "a dead shell's bypass is not in force");
        assert!(read.session_rules.is_empty(), "and its session rules died with it");

        // A live pid is not enough on its own: this process's own pid under a start time that
        // is not the kernel's — what a reused pid would look like — also reads as `ask`.
        let pid = std::process::id();
        let real = proc_start_ticks(pid).expect("this test is itself running");
        let reused = serde_json::json!({"mode": "auto", "shell_pid": pid,
                                        "shell_start_ticks": real + 1, "boot_id": boot});
        assert_eq!(mode_from(&reused.to_string(), 0).name, DEFAULT_MODE);

        // A live shell's own identity is honoured, and half an identity — a pid with no start
        // time to check it against — names no process anybody can find alive.
        let alive = serde_json::json!({"mode": "auto", "shell_pid": pid,
                                       "shell_start_ticks": real, "boot_id": boot});
        assert_eq!(mode_from(&alive.to_string(), 0).name, "auto");
        let half = serde_json::json!({"mode": "auto", "shell_pid": pid});
        assert_eq!(mode_from(&half.to_string(), 0).name, DEFAULT_MODE);

        // A file that names no shell at all reads the way it always has.
        assert_eq!(mode_from(r#"{"mode":"auto"}"#, 0).name, "auto");
    }

    /// #333, item 1: the identity used to be the pid and start time alone, and the file
    /// outlives a reboot on disk — in theory a new process could come up under the same pair
    /// and resurrect the mode a dead shell left behind. The boot id in the file is the part no
    /// reboot leaves standing: the kernel picks a fresh one every boot.
    #[test]
    fn an_identity_from_another_boot_reads_as_ask() {
        let pid = std::process::id();
        let start = proc_start_ticks(pid).expect("this test is itself running");
        let boot = boot_id().expect("this machine has booted");

        // The whole identity, recorded in this boot, is honoured.
        let alive = serde_json::json!({"mode": "bypass", "shell_pid": pid,
                                       "shell_start_ticks": start, "boot_id": boot});
        assert_eq!(mode_from(&alive.to_string(), 0).name, "bypass");

        // The same pid under the same start time, recorded in a boot that has ended: what the
        // file left on disk across a reboot would look like if the kernel handed the pair out
        // again.
        let stale_boot = serde_json::json!({"mode": "bypass", "shell_pid": pid,
                                            "shell_start_ticks": start,
                                            "boot_id": "00000000-0000-0000-0000-000000000000"});
        assert_eq!(mode_from(&stale_boot.to_string(), 0).name, DEFAULT_MODE);

        // A file from before the boot id existed names a live pid under a live start time and
        // nothing to tie the pair to this boot: two thirds of an identity fails closed like
        // half of one.
        let pre_upgrade = serde_json::json!({"mode": "bypass", "shell_pid": pid,
                                             "shell_start_ticks": start});
        assert_eq!(mode_from(&pre_upgrade.to_string(), 0).name, DEFAULT_MODE);

        // And a boot id on its own is no identity at all.
        let lonely = serde_json::json!({"mode": "bypass", "boot_id": boot});
        assert_eq!(mode_from(&lonely.to_string(), 0).name, DEFAULT_MODE);
    }

    /// #333, item 2: a child that has exited but not been reaped keeps its pid and its start
    /// time in /proc — as a zombie. An identity checked against the pair alone would call the
    /// shell behind them alive; the state character says it is not.
    #[test]
    fn a_shell_that_is_a_zombie_is_not_alive_either() {
        let mut child = std::process::Command::new("true").spawn().expect("a child to exit");
        let pid = child.id();
        // Wait for the state the assertion needs — the child a zombie, unreaped — not for a
        // fixed time.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let zombied = loop {
            if proc_stat(pid).map(|(state, _)| state) == Some('Z') {
                break true;
            }
            if std::time::Instant::now() >= deadline {
                break false;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let started = proc_start_ticks(pid).expect("a zombie keeps its start time");
        let doc = serde_json::json!({
            "mode": "bypass", "shell_pid": pid, "shell_start_ticks": started,
            "boot_id": boot_id().expect("this machine has booted"),
        });
        // Read while the zombie is still unreaped: the pid and start time are both in /proc
        // and both match the file, so only the state stands between this and `bypass`.
        let read = mode_from(&doc.to_string(), 0).name;
        child.wait().expect("the zombie can be reaped");
        assert!(zombied, "the child never reached the zombie state");
        assert_eq!(read, DEFAULT_MODE, "a zombie's bypass died with it");
    }

    // ── The vectors every other implementation replays ─────────────────
    //
    // `deploy/yantrik-os/surface-vectors.json` is this module's decision written out for every
    // combination of grade, ceiling, mode, session rule, grant and recoverability, with the
    // sentence it refuses in. The Blender addon, the MCP bridge and the shell's own mode table
    // replay it in their tests, so a copy that drifts fails its own build — and this test fails
    // when the file is not what `decide` produces, so the file cannot drift either.

    use yantrik_ipc_contracts::control_surface::{act_json, describe_json, Action, Param, View};

    /// The action a vector is about, so an id reads like something a person could check on a
    /// real machine. The pair is cosmetic to `decide`; the description is not.
    fn subject(unrecoverable: bool) -> (&'static str, &'static str, &'static str) {
        if unrecoverable {
            ("calendar", "delete_event", "Take an event off the calendar. It is not recoverable")
        } else {
            ("calendar", "update_event", "Change an event's title, time or notes")
        }
    }

    /// What a door that raises cards — the shell's `request_approval`, the MCP bridge — does
    /// with the same inputs, which do not include a grant (the card is how one is made).
    ///
    /// Derived from the dispatch's answer without a grant, and that derivation is the claim the
    /// spec makes: a door asks exactly when the dispatch would refuse for want of a grant, and
    /// refuses when the dispatch refuses on the ceiling — or when the machine is in plan mode,
    /// where a door refuses everything above `safe` while the dispatch still runs `standard`
    /// (`SOCKET_FLOOR`). The shell's `Modes::decide` and the bridge's `decide` are held to it.
    fn door(without_grant: &Result<(), String>, mode: &str, graded: &str) -> &'static str {
        match without_grant {
            Err(why) if why.starts_with("CEILING:") => "refuse",
            _ if mode == "plan" && grade(graded) != Some(0) => "refuse",
            Err(_) => "ask",
            Ok(()) => "run",
        }
    }

    fn decision_vectors() -> Vec<serde_json::Value> {
        let mut out = Vec::new();
        for mode in ["plan", "ask", "auto", "bypass"] {
            // The fifth is not a grade: `None` is not `safe`, and the case most likely to be got
            // wrong twice.
            for graded in ["safe", "standard", "sensitive", "dangerous", "spicy"] {
                for ceiling in LADDER {
                    for cannot_undo in [false, true] {
                        let (app, action, purpose) = subject(cannot_undo);
                        for session_rule in [false, true] {
                            for granted in [false, true] {
                                let rules = if session_rule {
                                    vec![(app.to_string(), action.to_string())]
                                } else {
                                    Vec::new()
                                };
                                let authority = |granted| Authority {
                                    ceiling: ceiling.to_string(),
                                    mode: Mode { name: mode.to_string(), session_rules: rules.clone() },
                                    granted,
                                };
                                let decided = decide(&authority(granted), app, action, graded, purpose);
                                let without = decide(&authority(false), app, action, graded, purpose);
                                let door = door(&without, mode, graded);
                                let (outcome, refusal) = match &decided {
                                    Ok(()) => ("allow", serde_json::Value::Null),
                                    Err(why) => (
                                        if why.starts_with("CEILING:") { "CEILING" } else { "GRANT" },
                                        serde_json::Value::String(why.clone()),
                                    ),
                                };
                                let mut vector = serde_json::json!({
                                    "id": format!(
                                        "{mode}/{graded}/ceiling={ceiling}/unrecoverable={cannot_undo}/rule={session_rule}/grant={granted}"
                                    ),
                                    "app": app,
                                    "action": action,
                                    "purpose": purpose,
                                    "grade": graded,
                                    "ceiling": ceiling,
                                    "mode": mode,
                                    "session_rule": session_rule,
                                    "grant": granted,
                                    "unrecoverable": unrecoverable(purpose),
                                    "outcome": outcome,
                                    "refusal": refusal,
                                    "door": door,
                                });
                                if without.is_ok() && door == "refuse" {
                                    vector["note"] = "socket floor: the dispatch runs `standard` in plan mode; a door that raises cards refuses it".into();
                                }
                                out.push(vector);
                            }
                        }
                    }
                }
            }
        }
        out
    }

    fn purpose_vectors() -> Vec<serde_json::Value> {
        [
            "Take an event off the calendar. It is not recoverable",
            "Move a file or folder to recoverable Trash",
            "Throw the current scene away and start an empty one. Anything unsaved in it is lost, and in a background Blender there is no undo to argue with.",
            "Erase the disk. This CANNOT BE UNDONE.",
            "Remove the saved network permanently",
            "An irreversible change",
            "It can't be undone",
            "Keeps a permanent record of what ran",
            "Undo is one click away",
            "",
        ]
        .into_iter()
        .map(|p| serde_json::json!({ "purpose": p, "unrecoverable": unrecoverable(p) }))
        .collect()
    }

    fn revision_vectors() -> Vec<serde_json::Value> {
        let views = [
            View::new(""),
            View::new("Blender — \"monkey.blend\", 3 objects, Cycles 1920x1080").state(serde_json::json!({
                "scene": "Scene", "file": "/tmp/monkey.blend", "unsaved": false,
                "objects": [{"name": "Suzanne", "type": "MESH", "location": [0.0, 0.0, 0.0], "dimensions": [2.0, 2.0, 2.0]}],
                "objects_total": 3, "camera": {"name": "Camera", "location": [4.0, -4.0, 3.0]},
                "render": {"engine": "cycles", "resolution": "1920x1080", "samples": 32, "output": "/tmp/monkey.png"},
                "last_render": null, "notice": "", "background": true
            })),
            View::new("Notes — “Kernel asks”, 412 words, unsaved")
                .with("title", "Kernel asks")
                .with("words", 412)
                .with("unsaved", true)
                .with("tags", serde_json::json!(["ünïcødé", "🙂", "a/b"])),
            View::new("keys are sorted, not kept in the order they were added")
                .with("zebra", 1)
                .with("apple", serde_json::json!({"y": null, "b": [3, 2, 1]}))
                .with("Mango", "capitals sort before lowercase"),
            View::new("tab\there, a newline\nand a \"quote\"")
                .with("backslash", "\\")
                .with("control", "\u{1}\u{1f}")
                .with("quote", "\"quoted\""),
            View::new("numbers")
                .with("int", 42)
                .with("negative", -7)
                .with("past_2_53", 9_007_199_254_740_993_u64)
                .with("float", 21.5)
                .with("zero_float", 0.0)
                .with("small", 0.001),
        ];
        views
            .iter()
            .map(|v| serde_json::json!({ "summary": v.summary, "state": v.state, "revision": v.revision() }))
            .collect()
    }

    /// Floats whose shortest rendering differs between `serde_json` and a naive port (Python's
    /// `json.dumps` writes `1e-05` and `1e+16`). Normative — a port renders them as `serde_json`
    /// does or its revisions disagree — and kept apart because the ports written so far do not.
    fn float_edge_vectors() -> Vec<serde_json::Value> {
        [0.00001_f64, 1.5e-7, 1e16, 1.25e21, 0.0001]
            .into_iter()
            .map(|x| {
                let view = View::new("float").with("x", x);
                serde_json::json!({
                    "summary": view.summary,
                    "state": view.state,
                    "rendered": view.state.to_string(),
                    "revision": view.revision(),
                })
            })
            .collect()
    }

    /// One envelope of each kind, as the Rust builders make them, so the JSON Schemas beside the
    /// spec are checked against what the code actually sends (`yos-selftest.py` validates these).
    fn envelopes() -> serde_json::Value {
        let view = View::new("Weather — 21°C in Dallas").with("temp", 21);
        let actions = [
            Action::new("refresh", "Fetch the weather again").risk("safe"),
            Action::new("set_location", "Show the weather somewhere else")
                .arg(Param::number("lat").describe("Latitude"))
                .arg(Param::number("lon").describe("Longitude"))
                .arg(Param::text("label").optional())
                .defers(),
        ];
        serde_json::json!({
            "describe": describe_json("weather", &view, &actions),
            "act": act_json("weather", "app-weather#1", false, serde_json::json!({"refreshing": true}), &view),
        })
    }

    fn vectors_document() -> String {
        let decisions = decision_vectors();
        let lines = |list: &[serde_json::Value]| -> String {
            list.iter()
                .map(|v| format!("    {}", serde_json::to_string(v).expect("plain json")))
                .collect::<Vec<_>>()
                .join(",\n")
        };
        let header = serde_json::json!({
            "_": [
                "Generated. Do not hand-edit: YANTRIK_WRITE_VECTORS=1 cargo test -p yantrik-ipc-transport --lib surface_vectors_write",
                "",
                "The surface protocol's policy, written out from yantrik_ipc_transport::gate::decide (docs/surface-protocol.md).",
                "Every implementation of the decision replays `decide`: the Blender addon's port and the SDKs replay",
                "`outcome` and `refusal` to the byte; the shell's mind_mode::Modes::decide and the MCP bridge's decide",
                "replay `door`. crates/yantrik-ipc-transport/src/gate.rs fails when this file is not what the code decides.",
                "",
                "Inputs: grade (spicy is not a grade), ceiling (tool_permission), mode, session_rule (a rule for this very",
                "app.action), grant (a grant was attached and the shell spent it), purpose and unrecoverable (the action's",
                "own published description, and what gate::unrecoverable reads in it).",
                "outcome: allow | CEILING | GRANT, and refusal is the exact sentence (null when allowed).",
                "door: run | ask | refuse — what a door that raises cards does with the same inputs, which never include",
                "a grant. It differs from the dispatch only where a vector carries `note` (the socket floor in plan)."
            ],
            "protocol": yantrik_ipc_contracts::control_surface::PROTOCOL,
            "ladder": LADDER,
            "modes": MODES.iter().map(|(m, top)| serde_json::json!([m, top])).collect::<Vec<_>>(),
            "socket_floor": SOCKET_FLOOR,
            "phrases": UNRECOVERABLE_PHRASES,
        });
        let mut text = serde_json::to_string_pretty(&header).expect("plain json");
        // Reopen the object: drop its closing brace and the newline before it, and carry on inside.
        text.truncate(text.trim_end().len() - 2);
        text.push_str(",\n  \"decide\": [\n");
        text.push_str(&lines(&decisions));
        text.push_str("\n  ],\n  \"purposes\": [\n");
        text.push_str(&lines(&purpose_vectors()));
        text.push_str("\n  ],\n  \"revision\": [\n");
        text.push_str(&lines(&revision_vectors()));
        text.push_str("\n  ],\n  \"revision_float_edges\": [\n");
        text.push_str(&lines(&float_edge_vectors()));
        text.push_str("\n  ],\n  \"envelopes\": ");
        text.push_str(&serde_json::to_string(&envelopes()).expect("plain json"));
        text.push_str("\n}\n");
        text
    }

    fn vectors_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../deploy/yantrik-os/surface-vectors.json")
    }

    /// Write the vectors out. Gated, because a test that rewrites its own expectation is not a
    /// test — it is a way for a change to `decide` to pass CI by regenerating what it broke.
    ///
    /// ```sh
    /// YANTRIK_WRITE_VECTORS=1 cargo test -p yantrik-ipc-transport --lib surface_vectors_write
    /// ```
    #[test]
    fn surface_vectors_write() {
        if std::env::var("YANTRIK_WRITE_VECTORS").as_deref() != Ok("1") {
            return;
        }
        let path = vectors_path();
        std::fs::write(&path, vectors_document())
            .unwrap_or_else(|e| panic!("cannot write {}: {e}", path.display()));
        println!("wrote {} decision vectors to {}", decision_vectors().len(), path.display());
    }

    /// And the checked-in file is what the code produces today. Changing `decide` without
    /// regenerating fails here; regenerating without changing the ports fails in theirs.
    #[test]
    fn surface_vectors_are_what_decide_produces() {
        let path = vectors_path();
        let checked_in = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "cannot read {}: {e}\n\nGenerate it with:\n  YANTRIK_WRITE_VECTORS=1 cargo test \
                 -p yantrik-ipc-transport --lib surface_vectors_write",
                path.display()
            )
        });
        let produced = vectors_document();
        if checked_in == produced {
            return;
        }
        let old: serde_json::Value = serde_json::from_str(&checked_in).expect("the checked-in vectors are json");
        let new: serde_json::Value = serde_json::from_str(&produced).expect("the generator makes json");
        let empty = Vec::new();
        let old_v = old["decide"].as_array().unwrap_or(&empty);
        let mut moved: Vec<String> = Vec::new();
        for want in new["decide"].as_array().unwrap_or(&empty) {
            let id = want["id"].as_str().unwrap_or_default();
            match old_v.iter().find(|v| v["id"].as_str() == Some(id)) {
                Some(had) if had == want => {}
                Some(had) => moved.push(format!(
                    "{id}: was {} / {}, is now {} / {}",
                    had["outcome"], had["door"], want["outcome"], want["door"]
                )),
                None => moved.push(format!("{id}: new")),
            }
        }
        panic!(
            "deploy/yantrik-os/surface-vectors.json is not what gate::decide produces any more.\n{}\n\n\
             If the change is meant, regenerate it and make every port agree:\n  \
             YANTRIK_WRITE_VECTORS=1 cargo test -p yantrik-ipc-transport --lib surface_vectors_write\n  \
             python3 -m unittest discover -s tests/blender-core\n  \
             python3 deploy/yantrik-os/yos-mcp-selftest.py",
            if moved.is_empty() { "(every decision is the same; another section of the file changed)".to_string() } else { moved.join("\n") }
        );
    }

    /// The file covers what it says it covers: every outcome and every door occurs, and the
    /// recoverability axis changes an answer somewhere — a dimension that never changes anything
    /// is a column, not a test.
    #[test]
    fn surface_vectors_cover_every_outcome_and_every_axis() {
        let all = decision_vectors();
        assert_eq!(all.len(), 4 * 5 * 4 * 2 * 2 * 2);
        for outcome in ["allow", "CEILING", "GRANT"] {
            assert!(all.iter().any(|v| v["outcome"] == outcome), "{outcome}");
        }
        for door in ["run", "ask", "refuse"] {
            assert!(all.iter().any(|v| v["door"] == door), "{door}");
        }
        let flips = all.iter().filter(|v| v["unrecoverable"] == true && v["outcome"] == "GRANT").filter(|v| {
            let id = v["id"].as_str().unwrap().replace("unrecoverable=true", "unrecoverable=false");
            all.iter().any(|w| w["id"] == id && w["outcome"] == "allow")
        }).count();
        assert!(flips > 0, "the app's own sentence changes the answer somewhere");
    }
}
