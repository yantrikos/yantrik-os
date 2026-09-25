//! What minds this machine could have, and what each one is still waiting for.
//!
//! "A harness exists because it is attached" is the right rule for the *answering* list — you
//! cannot talk to a mind that is not there — and it was the wrong rule for the Settings page. A
//! harness that had never been installed, or whose unit had died with a shell restart, was
//! simply absent from Settings → Harnesses. To the person the minds the image ships were just
//! *gone*: no hint that they exist, that one needs an `npm install`, that another needs a config
//! file, that a third only needs its unit started.
//!
//! So this module reads the manifests the image stages beside each harness's code
//! (`/opt/yantrik/share/harnesses/<id>/harness.yaml`) and works out, for every one of them, the
//! single next thing to do:
//!
//! | state | what it means | what the row offers |
//! |---|---|---|
//! | Not installed | something on `requires` is missing | *Install*, when the manifest has a command |
//! | Installing | that command is running right now | its output, streaming |
//! | Needs setup | installed, but a file only the person can write is not there | the path, and nothing in it |
//! | Ready | everything is there and the unit is not running | *Start* |
//! | Starting up | the unit is running and nothing has attached yet | a spinner |
//! | Would not start | the unit gave up | the last lines of its journal |
//! | Attached | it is here and can answer | *Use this* |
//! | Answering | it is the one talking | — |
//!
//! # Two lists, on purpose
//!
//! This is not the picker. The quick switcher and `use_harness` still work off the attach
//! registry alone, because only an attached mind can be handed a turn; offering "Pi" in the
//! switcher while Pi is not installed would be offering a button that cannot work. This list is
//! the one a person opens *because* a mind is missing, and it knows what the machine can offer.
//!
//! # Nothing here reads a credential
//!
//! A manifest names a file and may name an environment variable. Neither is ever opened: a
//! "Needs setup" row says `~/.config/yantrik/deepseek.json` and stops there. There is a test
//! that writes a key into a fixture config and asserts no field of any row contains it.
//!
//! The one thing here that carries text the desktop did not write is the journal of a unit that
//! failed, and that is the harness's own output. Each of them redacts its credential before it
//! reaches a log — `harnesses/deepseek` has a test that drives the whole loop against a server
//! which echoes the key back and asserts it appears nowhere — which is where that guarantee
//! belongs, because only the harness knows which of its strings is a key.
//!
//! # Why the gathering is separate from the deciding
//!
//! [`Machine`] is every fact about the machine — manifests found, what is on `PATH`, which
//! config files exist, what systemd says, which jobs this shell is running — and [`rows`] is a
//! pure function of it. So every state above is tested against a fixture directory, with no
//! systemd, no network and no compositor.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use yantrik_harness::Entry;

/// The file the desktop looks for in each harness's directory.
pub const MANIFEST: &str = "harness.yaml";

/// Where an installed machine stages the harness sources.
pub const SHARE: &str = "/opt/yantrik/share/harnesses";

/// How many lines of a job's output, or of a dead unit's journal, a row carries.
///
/// A row is a card on a settings page, not a log viewer. Enough to see what npm is doing or why
/// a unit gave up, and the README names `journalctl` for the rest.
pub const LOG_LINES: usize = 6;

// ── The manifest ────────────────────────────────────────────────────

/// One harness, as the file beside its code describes it.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// One line about what it is. Only used until it attaches: after that the harness's own
    /// account of itself is better, because it knows which model it actually loaded.
    #[serde(default)]
    pub detail: String,
    /// Where to read more, relative to the manifest's own directory.
    #[serde(default)]
    pub docs: String,
    /// The `systemctl --user` unit that starts it. Empty for a harness that something else
    /// starts — Hermes is a plugin inside its own gateway.
    #[serde(default)]
    pub unit: String,
    /// Must all be there before the unit can do anything at all.
    #[serde(default)]
    pub requires: Vec<Need>,
    /// Must all be there before it can work, and only the person can put them there.
    #[serde(default)]
    pub setup: Vec<Need>,
    #[serde(default)]
    pub install: Option<Install>,
    /// The directory this was read from. Not in the file; filled in by [`read_manifest`] so
    /// `file:` needs and `{dir}` in an install command resolve without a second lookup.
    #[serde(skip)]
    pub dir: PathBuf,
}

/// One thing that has to exist. Exactly one of the three fields is set.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct Need {
    /// A binary that must be on `PATH`.
    #[serde(default)]
    pub binary: String,
    /// A file: relative to the manifest's directory, absolute, or under `~`.
    #[serde(default)]
    pub file: String,
    /// A file in the person's own config directory — `~/.config/yantrik/<config>`. Sugar, so no
    /// manifest has to spell that path out and get it slightly wrong.
    #[serde(default)]
    pub config: String,
    /// Why it is needed, in words a person can act on.
    #[serde(default)]
    pub why: String,
}

impl Need {
    /// What to call it on screen.
    fn subject(&self, machine: &Machine, dir: &Path) -> String {
        if !self.binary.is_empty() {
            return self.binary.clone();
        }
        if !self.config.is_empty() {
            return display_path(&machine.config_dir.join(&self.config), &machine.home);
        }
        display_path(&self.resolve(machine, dir), &machine.home)
    }

    /// The path this need points at, or an empty path for a `binary:` need.
    fn resolve(&self, machine: &Machine, dir: &Path) -> PathBuf {
        if !self.config.is_empty() {
            return machine.config_dir.join(&self.config);
        }
        if self.file.is_empty() {
            return PathBuf::new();
        }
        if let Some(rest) = self.file.strip_prefix("~/") {
            return machine.home.join(rest);
        }
        let path = Path::new(&self.file);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            dir.join(path)
        }
    }

    fn met(&self, machine: &Machine, dir: &Path) -> bool {
        if !self.binary.is_empty() {
            return machine.path_dirs.iter().any(|d| is_program(&d.join(&self.binary)));
        }
        let path = self.resolve(machine, dir);
        !path.as_os_str().is_empty() && path.exists()
    }
}

/// How to install what `requires` is missing.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct Install {
    /// Run through `sh -lc`, as the person, with its output streamed into the row. `{dir}` is
    /// the manifest's own directory, so one command works from the image and from a checkout.
    pub command: String,
    /// The present participle for the row while it runs: "fetching …".
    #[serde(default)]
    pub doing: String,
}

impl Install {
    /// The command with `{dir}` filled in.
    pub fn command_in(&self, dir: &Path) -> String {
        self.command.replace("{dir}", &dir.display().to_string())
    }
}

// ── What the machine says ───────────────────────────────────────────

/// What systemd says about one unit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Unit {
    /// systemd can see a unit file by this name.
    pub loaded: bool,
    /// Active, or on its way there. A harness under `Restart=on-failure` spends its restarts in
    /// `activating`, and calling that "stopped" would flicker the row once every five seconds.
    pub active: bool,
    /// It tried and gave up.
    pub failed: bool,
    pub enabled: bool,
    /// The last few lines of its journal. Read only when it failed — there is no reason to spawn
    /// `journalctl` for a unit that is doing fine.
    pub log: String,
}

/// What a job was asked to do. The two fail differently enough to be worth distinguishing: an
/// install that fails is a package that is not there, a start that fails is a unit that would
/// not run — and while they are going, one row says "installing" and the other "starting up".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum JobKind {
    #[default]
    Install,
    Start,
}

impl JobKind {
    pub fn verb(self) -> &'static str {
        match self {
            JobKind::Install => "install",
            JobKind::Start => "start",
        }
    }
}

/// A job this shell is running for one harness, as a row needs to see it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JobView {
    pub kind: JobKind,
    /// `true` while it is still going.
    pub running: bool,
    /// What it is doing, for the row: "fetching @earendil-works/pi-coding-agent".
    pub doing: String,
    /// The tail of its output, oldest line first.
    pub log: String,
    /// Set when it exited non-zero. A failed install that went quiet is a person clicking the
    /// button again and again.
    pub error: String,
}

/// Every fact the rows are derived from, gathered once.
#[derive(Debug, Clone, Default)]
pub struct Machine {
    /// Manifests found, keyed by id.
    pub manifests: HashMap<String, Manifest>,
    /// `~/.config/yantrik`, where the files a person writes live.
    pub config_dir: PathBuf,
    /// What `~` means in a manifest path.
    pub home: PathBuf,
    /// The directories of `PATH`.
    pub path_dirs: Vec<PathBuf>,
    /// Keyed by unit name, for the units the manifests name.
    pub units: HashMap<String, Unit>,
    /// Keyed by harness id.
    pub jobs: HashMap<String, JobView>,
}

// ── The states a row can be in ──────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Something on `requires` is missing.
    NotInstalled,
    /// The install command is running right now.
    Installing,
    /// Installed, but a file only the person can write is not there.
    NeedsSetup,
    /// Everything is there; its unit is not running.
    Ready,
    /// The unit is running and nothing has attached under this id yet.
    Starting,
    /// The unit tried and gave up.
    Failed,
    /// It is attached and can be handed a turn.
    Attached,
    /// It is the one answering.
    Answering,
}

impl State {
    /// What the pill on the row reads.
    pub fn label(self) -> &'static str {
        match self {
            State::NotInstalled => "not installed",
            State::Installing => "installing",
            State::NeedsSetup => "needs setup",
            State::Ready => "ready to start",
            State::Starting => "starting up",
            State::Failed => "would not start",
            State::Attached => "attached",
            State::Answering => "answering",
        }
    }

    /// The name a caller on the control surface matches on. Stable, unspaced, and deliberately
    /// not the label — one is read by a person and the other by a program.
    pub fn key(self) -> &'static str {
        match self {
            State::NotInstalled => "not_installed",
            State::Installing => "installing",
            State::NeedsSetup => "needs_setup",
            State::Ready => "ready",
            State::Starting => "starting",
            State::Failed => "failed",
            State::Attached => "attached",
            State::Answering => "answering",
        }
    }

    /// Whether a mind in this state can be handed a turn. Only attachment can make this true —
    /// it is the same rule the picker has always had, written once.
    pub fn can_answer(self) -> bool {
        matches!(self, State::Attached | State::Answering)
    }
}

/// One line of the Settings list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub id: String,
    pub name: String,
    /// What it says it is: the harness's own word while it is attached, the manifest's before.
    pub detail: String,
    pub state: State,
    /// The one thing to do next, in a sentence. Empty when there is nothing to do.
    pub need: String,
    /// Streaming output, or a dead unit's last journal lines. Never a credential.
    pub log: String,
    /// A job this shell started is running for it. What the row's spinner is for.
    pub busy: bool,
    pub builtin: bool,
    pub active: bool,
    pub attached: bool,
    pub tools: bool,
    pub memory: bool,
    pub can_install: bool,
    pub can_start: bool,
    pub unit: String,
    /// Where to read more. A path, because it is a file on this machine.
    pub docs: String,
}

// ── Deriving the rows ───────────────────────────────────────────────

/// The Settings list: the built-in minds, then every manifest, then anything attached that no
/// manifest claims.
///
/// `minds` is the attach registry — [`yantrik_harness::Host::list`]. It wins over everything: a
/// harness that is attached is working, whatever a manifest thinks is missing, and saying "not
/// installed" about a mind that is answering would be the same bug the other way round.
pub fn rows(machine: &Machine, minds: &[Entry]) -> Vec<Row> {
    let mut out: Vec<Row> = Vec::new();

    // The built-ins first. They are compiled into the shell, so they have no manifest, nothing
    // to install and nothing to start.
    for entry in minds.iter().filter(|e| e.builtin) {
        out.push(Row {
            id: entry.id.clone(),
            name: entry.name.clone(),
            detail: entry.detail.clone().unwrap_or_default(),
            state: if entry.active { State::Answering } else { State::Attached },
            need: String::new(),
            log: String::new(),
            busy: false,
            builtin: true,
            active: entry.active,
            attached: true,
            tools: entry.capabilities.tools,
            memory: entry.capabilities.memory,
            can_install: false,
            can_start: false,
            unit: String::new(),
            docs: String::new(),
        });
    }

    let mut ids: Vec<&String> = machine.manifests.keys().collect();
    ids.sort();
    for id in ids {
        let manifest = &machine.manifests[id];
        out.push(from_manifest(machine, manifest, minds.iter().find(|e| &e.id == id)));
    }

    // Whatever else is attached — a mind from its own repo, somebody's own harness, the echo
    // example. It has no manifest and needs none: it is here, and that is the whole protocol.
    let mut strays: Vec<&Entry> = minds
        .iter()
        .filter(|e| !e.builtin && !machine.manifests.contains_key(&e.id))
        .collect();
    strays.sort_by(|a, b| a.id.cmp(&b.id));
    for entry in strays {
        out.push(Row {
            id: entry.id.clone(),
            name: entry.name.clone(),
            detail: entry.detail.clone().unwrap_or_default(),
            state: if entry.active { State::Answering } else { State::Attached },
            need: String::new(),
            log: String::new(),
            busy: false,
            builtin: false,
            active: entry.active,
            attached: true,
            tools: entry.capabilities.tools,
            memory: entry.capabilities.memory,
            can_install: false,
            can_start: false,
            unit: String::new(),
            docs: String::new(),
        });
    }

    out
}

fn from_manifest(machine: &Machine, manifest: &Manifest, attached: Option<&Entry>) -> Row {
    let job = machine.jobs.get(&manifest.id);
    let unit = machine.units.get(&manifest.unit).cloned().unwrap_or_default();
    let dir = manifest.dir.as_path();

    let missing_require = manifest.requires.iter().find(|n| !n.met(machine, dir));
    let missing_setup = manifest.setup.iter().find(|n| !n.met(machine, dir));

    // Ordered by what the person has to do next, and attachment first because it is the only
    // one of these that is not an inference.
    let state = if let Some(entry) = attached {
        if entry.active {
            State::Answering
        } else {
            State::Attached
        }
    } else if let Some(job) = job.filter(|j| j.running) {
        match job.kind {
            JobKind::Install => State::Installing,
            JobKind::Start => State::Starting,
        }
    } else if missing_require.is_some() {
        State::NotInstalled
    } else if missing_setup.is_some() {
        State::NeedsSetup
    } else if unit.failed {
        State::Failed
    } else if unit.active {
        State::Starting
    } else {
        State::Ready
    };

    let mut need = match state {
        State::NotInstalled => {
            let n = missing_require.expect("state says something is missing");
            sentence(&format!("{} is not here", n.subject(machine, dir)), &n.why)
        }
        State::NeedsSetup => {
            let n = missing_setup.expect("state says something is missing");
            sentence(&format!("write {}", n.subject(machine, dir)), &n.why)
        }
        State::Installing => job
            .map(|j| {
                if j.doing.is_empty() {
                    "installing".to_string()
                } else {
                    format!("installing — {}", j.doing)
                }
            })
            .unwrap_or_default(),
        // A unit that is up but has not attached yet, or a start job still running. Both are
        // the same thing to a person: it is on its way and there is nothing to do.
        State::Starting => job
            .filter(|j| j.running && !j.doing.is_empty())
            .map(|j| j.doing.clone())
            .unwrap_or_default(),
        State::Ready => {
            if manifest.unit.is_empty() {
                // Hermes: everything it needs is here, and starting it is its own gateway's
                // business. Saying "ready to start" beside no button would be a dead end.
                "everything it needs is here; it attaches when its own service runs".to_string()
            } else if unit.loaded {
                String::new()
            } else {
                // The unit file was never put where systemd looks. Worth saying, because
                // "enable --now" is about to fail with a sentence about a unit not found.
                format!("{} is not installed as a user unit yet", manifest.unit)
            }
        }
        State::Failed => format!("{} started and gave up", manifest.unit),
        State::Attached | State::Answering => String::new(),
    };

    // A job that failed says so wherever the row ended up: an install that fell over and then
    // went quiet is a person pressing the button again and again.
    if let Some(error) = job.map(|j| j.error.as_str()).filter(|e| !e.is_empty()) {
        need = if need.is_empty() { error.to_string() } else { format!("{error} — {need}") };
    }

    let log = match job {
        Some(job) if !job.log.is_empty() => job.log.clone(),
        _ if state == State::Failed => unit.log.clone(),
        _ => String::new(),
    };

    let busy = job.map(|j| j.running).unwrap_or(false);
    Row {
        id: manifest.id.clone(),
        name: if manifest.name.is_empty() { manifest.id.clone() } else { manifest.name.clone() },
        detail: attached
            .and_then(|e| e.detail.clone())
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| manifest.detail.clone()),
        state,
        need,
        log,
        busy,
        builtin: false,
        active: attached.map(|e| e.active).unwrap_or(false),
        attached: attached.is_some(),
        tools: attached.map(|e| e.capabilities.tools).unwrap_or(false),
        memory: attached.map(|e| e.capabilities.memory).unwrap_or(false),
        can_install: !busy && state == State::NotInstalled && manifest.install.is_some(),
        // Only where there is a unit to enable, and only from a state where starting it is the
        // next thing: a harness whose config file is missing would start and immediately die.
        can_start: !busy
            && !manifest.unit.is_empty()
            && matches!(state, State::Ready | State::Failed),
        unit: manifest.unit.clone(),
        docs: if manifest.docs.is_empty() {
            String::new()
        } else {
            display_path(&dir.join(&manifest.docs), &machine.home)
        },
    }
}

/// "pi is not here — Pi itself, which npm installs per user".
fn sentence(head: &str, why: &str) -> String {
    let why = why.trim();
    if why.is_empty() {
        head.to_string()
    } else {
        format!("{head} — {why}")
    }
}

/// A path as a person would write it: their home as `~`, and no `..` left in the middle.
///
/// The `..` matters because a manifest may point at documentation above its own directory, and
/// `/opt/yantrik/share/harnesses/hermes/../../docs/harness.md` on a settings card is a path
/// nobody can read at a glance. Cleaned lexically rather than with `canonicalize`, which would
/// touch the disk and fail on a file that is not there — and a path we are about to say is
/// missing is exactly the case that matters.
fn display_path(path: &Path, home: &Path) -> String {
    use std::path::Component;
    let mut parts: Vec<std::ffi::OsString> = Vec::new();
    let mut prefix = String::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                // Only a real name can be cancelled. A leading `../..` in a relative path stays,
                // because dropping it would change where the path points.
                if parts.last().map(|last| last != "..").unwrap_or(false) {
                    parts.pop();
                } else {
                    parts.push("..".into());
                }
            }
            Component::RootDir => prefix = "/".to_string(),
            Component::Prefix(p) => prefix = p.as_os_str().to_string_lossy().to_string(),
            Component::Normal(name) => parts.push(name.to_os_string()),
        }
    }
    let joined: PathBuf = parts.iter().collect();
    let cleaned = PathBuf::from(prefix).join(joined);

    if home.as_os_str().is_empty() {
        return cleaned.display().to_string();
    }
    match cleaned.strip_prefix(home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => cleaned.display().to_string(),
    }
}

// ── Reading the machine ─────────────────────────────────────────────

/// Where `<id>/harness.yaml` is looked for, in order. The first directory that has a manifest
/// for an id wins, so a checkout can shadow the image without moving anything.
pub fn roots() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    if let Ok(dir) = std::env::var("YANTRIK_HARNESSES_DIR") {
        if !dir.trim().is_empty() {
            out.push(PathBuf::from(dir));
        }
    }
    // Beside the binary, which is how a relocated install finds everything else it ships.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(bin) = exe.parent() {
            out.push(bin.join("../share/harnesses"));
        }
    }
    out.push(PathBuf::from(SHARE));
    // A checkout, run from its root.
    out.push(PathBuf::from("harnesses"));
    out
}

fn home() -> PathBuf {
    std::env::var("HOME").map(PathBuf::from).unwrap_or_default()
}

/// Where a person's own systemd user units live — the directory each README says to copy a unit
/// into. `XDG_CONFIG_HOME` is honoured because systemd honours it.
pub fn user_unit_dir() -> PathBuf {
    match std::env::var("XDG_CONFIG_HOME") {
        Ok(dir) if !dir.trim().is_empty() => PathBuf::from(dir).join("systemd/user"),
        _ => home().join(".config/systemd/user"),
    }
}

/// Read every manifest the roots offer. A directory with no manifest is not a harness — `lib`
/// is shared Python and `tests` is a test suite, and neither belongs in the picker.
pub fn read_manifests(roots: &[PathBuf]) -> HashMap<String, Manifest> {
    let mut out: HashMap<String, Manifest> = HashMap::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root) else { continue };
        for entry in entries.flatten() {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            match read_manifest(&dir) {
                Ok(Some(manifest)) => {
                    out.entry(manifest.id.clone()).or_insert(manifest);
                }
                Ok(None) => {}
                // A manifest we cannot parse is worth one line in the log and no more: the rest
                // of the page is still true, and a settings screen that refuses to draw because
                // one file has a typo in it is worse than a screen with one harness missing.
                Err(e) => tracing::warn!(dir = %dir.display(), error = %e, "unreadable harness manifest"),
            }
        }
    }
    out
}

/// Read one directory's manifest, or `Ok(None)` when it has none.
pub fn read_manifest(dir: &Path) -> Result<Option<Manifest>, String> {
    let path = dir.join(MANIFEST);
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let mut manifest: Manifest = serde_yaml::from_str(&text).map_err(|e| e.to_string())?;
    if manifest.id.trim().is_empty() {
        return Err("no `id` in the manifest".to_string());
    }
    // An id is what a person types into `use_harness`, so the same rule the protocol enforces on
    // attach applies here: no spaces, no surprises.
    if manifest.id.contains(char::is_whitespace) {
        return Err(format!("`{}` is not usable as an id — it has a space in it", manifest.id));
    }
    manifest.dir = dir.to_path_buf();
    Ok(Some(manifest))
}

/// `PATH`, split. Empty entries mean the working directory, and a harness found "on PATH"
/// because it happened to sit in whatever directory the shell was started from is not found.
pub fn path_dirs() -> Vec<PathBuf> {
    std::env::var("PATH")
        .unwrap_or_default()
        .split(':')
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .collect()
}

#[cfg(unix)]
fn is_program(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_program(path: &Path) -> bool {
    path.is_file()
}

/// Ask systemd about every unit the manifests name, in one call.
///
/// One `systemctl show` with every unit on the command line rather than one per harness: this is
/// read on a timer while the page is open, and four process spawns every couple of seconds on an
/// idle desktop is exactly the kind of thing that turns up later as a battery complaint.
pub fn read_units(units: &[String]) -> HashMap<String, Unit> {
    let mut out: HashMap<String, Unit> = HashMap::new();
    if units.is_empty() {
        return out;
    }
    let output = std::process::Command::new("systemctl")
        .arg("--user")
        .arg("show")
        .args(units)
        .arg("--property=Id,LoadState,ActiveState,SubState,UnitFileState")
        .output();
    let text = match output {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).to_string(),
        // No user systemd (a container, a checkout on a developer's machine) is not an error to
        // report: every row simply says what it needs rather than what its unit is doing.
        Ok(_) | Err(_) => return out,
    };

    for block in text.split("\n\n") {
        let mut fields: HashMap<&str, &str> = HashMap::new();
        for line in block.lines() {
            if let Some((key, value)) = line.split_once('=') {
                fields.insert(key.trim(), value.trim());
            }
        }
        let Some(id) = fields.get("Id").copied().filter(|i| !i.is_empty()) else { continue };
        let active = fields.get("ActiveState").copied().unwrap_or("");
        let unit = Unit {
            loaded: fields.get("LoadState").copied().unwrap_or("") == "loaded",
            active: matches!(active, "active" | "activating" | "reloading"),
            failed: active == "failed",
            enabled: fields.get("UnitFileState").copied().unwrap_or("") == "enabled",
            log: if active == "failed" { journal(id) } else { String::new() },
        };
        out.insert(id.to_string(), unit);
    }
    out
}

/// The last lines of a unit's journal, for a row that has to say why it gave up.
fn journal(unit: &str) -> String {
    let output = std::process::Command::new("journalctl")
        .args(["--user", "-u", unit, "-n"])
        .arg(LOG_LINES.to_string())
        .args(["--no-pager", "-o", "cat"])
        .output();
    match output {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).trim().to_string(),
        _ => String::new(),
    }
}

/// How long a gathered [`Machine`] is reused before the disk and systemd are asked again.
///
/// Two callers want this: the Settings page, on a two-second timer while it is open, and a mind
/// reading `describe shell`, which can be several times a minute while it works. Neither is a
/// reason to run `systemctl` twice in the same second, and nothing being described here changes
/// faster than a person can see. Jobs are not cached — they are passed in on every call, because
/// streamed output is the one thing on the row that has to be current.
const CACHE: std::time::Duration = std::time::Duration::from_secs(2);

/// Gather everything, now — or reuse what was gathered a moment ago. `jobs` comes from whatever
/// this shell has running; see `crate::harness_install`.
pub fn machine(jobs: HashMap<String, JobView>) -> Machine {
    static LAST: std::sync::Mutex<Option<(std::time::Instant, Machine)>> =
        std::sync::Mutex::new(None);
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, cached)) = last.as_ref() {
        if at.elapsed() < CACHE {
            let mut machine = cached.clone();
            machine.jobs = jobs;
            return machine;
        }
    }
    let mut machine = gather();
    *last = Some((std::time::Instant::now(), machine.clone()));
    machine.jobs = jobs;
    machine
}

fn gather() -> Machine {
    let home = home();
    let manifests = read_manifests(&roots());
    let units: Vec<String> = {
        let mut names: Vec<String> = manifests
            .values()
            .map(|m| m.unit.clone())
            .filter(|u| !u.is_empty())
            .collect();
        names.sort();
        names.dedup();
        names
    };
    Machine {
        config_dir: home.join(".config/yantrik"),
        home,
        path_dirs: path_dirs(),
        units: read_units(&units),
        manifests,
        jobs: HashMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yantrik_harness::Capabilities;

    /// A fixture machine: one manifest directory, a config directory, a bin directory, and
    /// nothing real. Every state below is reached by adding or removing a file in it.
    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new(name: &str) -> Fixture {
            let root = std::env::temp_dir()
                .join(format!("harness-catalogue-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join("share")).unwrap();
            std::fs::create_dir_all(root.join("home/.config/yantrik")).unwrap();
            std::fs::create_dir_all(root.join("bin")).unwrap();
            Fixture { root }
        }

        fn share(&self) -> PathBuf {
            self.root.join("share")
        }

        fn harness(&self, id: &str, manifest: &str) -> PathBuf {
            let dir = self.share().join(id);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(MANIFEST), manifest).unwrap();
            dir
        }

        fn program(&self, name: &str) {
            let path = self.root.join("bin").join(name);
            std::fs::write(&path, "#!/bin/sh\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
        }

        fn config(&self, name: &str, body: &str) {
            std::fs::write(self.root.join("home/.config/yantrik").join(name), body).unwrap();
        }

        fn machine(&self) -> Machine {
            let home = self.root.join("home");
            Machine {
                manifests: read_manifests(&[self.share()]),
                config_dir: home.join(".config/yantrik"),
                home,
                path_dirs: vec![self.root.join("bin")],
                units: HashMap::new(),
                jobs: HashMap::new(),
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    const PI: &str = r#"
id: pi
name: Pi
detail: The pi coding agent
docs: README.md
unit: yantrik-pi.service
requires:
  - binary: pi
    why: Pi itself, which npm installs per user
  - file: yantrik_pi.py
    why: the harness script
install:
  command: npm install -g @earendil-works/pi-coding-agent
  doing: fetching @earendil-works/pi-coding-agent
setup:
  - config: pi.json
    why: which provider and model Pi should use
"#;

    fn entry(id: &str, active: bool) -> Entry {
        Entry {
            id: id.to_string(),
            name: id.to_string(),
            detail: Some(format!("{id} says hello")),
            builtin: false,
            active,
            capabilities: Capabilities { streaming: true, tools: true, memory: false },
            pid: None,
        }
    }

    fn builtin() -> Entry {
        Entry {
            id: "companion".to_string(),
            name: "Yantrik Companion".to_string(),
            detail: None,
            builtin: true,
            active: true,
            capabilities: Capabilities { streaming: true, tools: true, memory: true },
            pid: None,
        }
    }

    fn row<'a>(rows: &'a [Row], id: &str) -> &'a Row {
        rows.iter().find(|r| r.id == id).unwrap_or_else(|| panic!("no row for {id}"))
    }

    #[test]
    fn a_harness_that_is_not_installed_is_a_row_that_says_so_and_offers_to_install() {
        // The whole defect: before this, nothing on the page mentioned Pi at all.
        let fixture = Fixture::new("not-installed");
        fixture.harness("pi", PI);
        let rows = rows(&fixture.machine(), &[builtin()]);

        let pi = row(&rows, "pi");
        assert_eq!(pi.state, State::NotInstalled);
        assert!(pi.need.contains("pi is not here"), "{}", pi.need);
        assert!(pi.need.contains("npm installs per user"), "{}", pi.need);
        assert!(pi.can_install);
        assert!(!pi.can_start, "nothing to start until it is installed");
        assert!(!pi.state.can_answer());
        // And the manifest's own line about itself, so the row is not just a complaint.
        assert_eq!(pi.detail, "The pi coding agent");
    }

    #[test]
    fn installed_but_unconfigured_names_the_file_and_nothing_in_it() {
        let fixture = Fixture::new("needs-setup");
        let dir = fixture.harness("pi", PI);
        std::fs::write(dir.join("yantrik_pi.py"), "# harness\n").unwrap();
        fixture.program("pi");

        let rows = rows(&fixture.machine(), &[builtin()]);
        let pi = row(&rows, "pi");
        assert_eq!(pi.state, State::NeedsSetup);
        assert_eq!(pi.need, "write ~/.config/yantrik/pi.json — which provider and model Pi should use");
        assert!(!pi.can_install, "there is nothing left to install");
        assert!(!pi.can_start, "it would start and die without its config");
    }

    #[test]
    fn everything_present_and_the_unit_stopped_offers_to_start_it() {
        // The state the owner hit: Pi and DeepSeek had been started by hand and died with a
        // shell restart, and the page said nothing about a unit that merely needed starting.
        let fixture = Fixture::new("ready");
        let dir = fixture.harness("pi", PI);
        std::fs::write(dir.join("yantrik_pi.py"), "# harness\n").unwrap();
        fixture.program("pi");
        fixture.config("pi.json", "{}");

        let mut machine = fixture.machine();
        machine.units.insert(
            "yantrik-pi.service".into(),
            Unit { loaded: true, ..Default::default() },
        );

        let rows = rows(&machine, &[builtin()]);
        let pi = row(&rows, "pi");
        assert_eq!(pi.state, State::Ready);
        assert!(pi.can_start);
        assert_eq!(pi.need, "");
    }

    #[test]
    fn a_running_unit_with_nothing_attached_is_starting_up_not_missing() {
        let fixture = Fixture::new("starting");
        let dir = fixture.harness("pi", PI);
        std::fs::write(dir.join("yantrik_pi.py"), "# harness\n").unwrap();
        fixture.program("pi");
        fixture.config("pi.json", "{}");

        let mut machine = fixture.machine();
        machine.units.insert(
            "yantrik-pi.service".into(),
            Unit { loaded: true, active: true, enabled: true, ..Default::default() },
        );

        let pi = row(&rows(&machine, &[builtin()]), "pi").clone();
        assert_eq!(pi.state, State::Starting);
        assert!(!pi.can_start, "it is already going");
        assert!(!pi.state.can_answer(), "nothing has attached yet");
    }

    #[test]
    fn a_unit_that_gave_up_says_so_and_carries_its_last_log_lines() {
        let fixture = Fixture::new("failed");
        let dir = fixture.harness("pi", PI);
        std::fs::write(dir.join("yantrik_pi.py"), "# harness\n").unwrap();
        fixture.program("pi");
        fixture.config("pi.json", "{}");

        let mut machine = fixture.machine();
        machine.units.insert(
            "yantrik-pi.service".into(),
            Unit {
                loaded: true,
                failed: true,
                log: "ModuleNotFoundError: No module named 'yantrik_harness'".into(),
                ..Default::default()
            },
        );

        let pi = row(&rows(&machine, &[builtin()]), "pi").clone();
        assert_eq!(pi.state, State::Failed);
        assert!(pi.need.contains("gave up"), "{}", pi.need);
        assert!(pi.log.contains("ModuleNotFoundError"));
        assert!(pi.can_start, "it can be tried again");
    }

    #[test]
    fn attached_wins_over_everything_a_manifest_thinks_is_missing() {
        // A harness the person installed by hand somewhere else is working, and a page that told
        // them it was "not installed" while it answered their questions would be the same defect
        // with the sign flipped.
        let fixture = Fixture::new("attached");
        fixture.harness("pi", PI);
        let rows = rows(&fixture.machine(), &[builtin(), entry("pi", false)]);

        let pi = row(&rows, "pi");
        assert_eq!(pi.state, State::Attached);
        assert!(pi.state.can_answer());
        assert!(!pi.can_install);
        assert_eq!(pi.need, "");
        // What it said about itself, not what the manifest guessed.
        assert_eq!(pi.detail, "pi says hello");
    }

    #[test]
    fn the_one_answering_is_the_one_the_host_says_is_answering() {
        let fixture = Fixture::new("answering");
        fixture.harness("pi", PI);
        let rows = rows(&fixture.machine(), &[builtin(), entry("pi", true)]);
        assert_eq!(row(&rows, "pi").state, State::Answering);
        assert!(row(&rows, "pi").active);
    }

    #[test]
    fn a_listed_mind_is_attached_even_while_its_unit_says_stopped() {
        // A harness run by hand from a terminal — or by anything other than its unit — polls
        // happily while `systemctl --user show` says the unit is inactive. The registry, which
        // drops a session the moment the kernel says its process is gone (#67), is the only
        // witness that counts here: a row that preferred the unit's word would call this
        // answering mind stopped and take *Use this* away from it.
        let fixture = Fixture::new("hand-run");
        fixture.harness("pi", PI);
        let mut machine = fixture.machine();
        machine.units.insert(
            "yantrik-pi.service".into(),
            Unit { loaded: true, enabled: true, ..Default::default() },
        );

        let pi = row(&rows(&machine, &[builtin(), entry("pi", true)]), "pi").clone();
        assert_eq!(pi.state, State::Answering);
        assert!(pi.state.can_answer() && pi.attached && pi.active);
        assert_eq!(pi.need, "");
        assert_eq!(pi.detail, "pi says hello");
    }

    #[test]
    fn a_running_install_says_installing_with_its_output() {
        // The owner's words: "if not installed then show installation in progress or starting
        // up". A row that falls back to "not installed" while npm is fetching is a button that
        // looks like it did nothing.
        let fixture = Fixture::new("installing");
        fixture.harness("pi", PI);
        let mut machine = fixture.machine();
        machine.jobs.insert(
            "pi".into(),
            JobView {
                kind: JobKind::Install,
                running: true,
                doing: "fetching @earendil-works/pi-coding-agent".into(),
                log: "added 41 packages".into(),
                error: String::new(),
            },
        );

        let pi = row(&rows(&machine, &[builtin()]), "pi").clone();
        assert_eq!(pi.state, State::Installing);
        assert_eq!(pi.need, "installing — fetching @earendil-works/pi-coding-agent");
        assert_eq!(pi.log, "added 41 packages");
        assert!(!pi.can_install, "one at a time");
    }

    #[test]
    fn an_install_that_failed_says_so_instead_of_going_quiet() {
        let fixture = Fixture::new("install-failed");
        fixture.harness("pi", PI);
        let mut machine = fixture.machine();
        machine.jobs.insert(
            "pi".into(),
            JobView {
                kind: JobKind::Install,
                running: false,
                doing: "fetching @earendil-works/pi-coding-agent".into(),
                log: "npm ERR! 404 Not Found".into(),
                error: "npm install exited 1".into(),
            },
        );

        let pi = row(&rows(&machine, &[builtin()]), "pi").clone();
        assert_eq!(pi.state, State::NotInstalled);
        assert!(pi.need.starts_with("npm install exited 1"), "{}", pi.need);
        assert!(pi.log.contains("404"));
        assert!(pi.can_install, "and it can be tried again");
    }

    #[test]
    fn a_harness_with_no_install_command_is_named_rather_than_offered() {
        // OpenClaw. Its own documentation is the only thing that knows how to install it, so
        // the row says what is missing and points at the README instead of running a guess.
        let fixture = Fixture::new("no-install");
        fixture.harness(
            "openclaw",
            "id: openclaw\nname: OpenClaw\ndocs: README.md\nunit: yantrik-openclaw.service\n\
             requires:\n  - binary: openclaw\n    why: install it from its own documentation\n",
        );
        let claw = row(&rows(&fixture.machine(), &[builtin()]), "openclaw").clone();
        assert_eq!(claw.state, State::NotInstalled);
        assert!(!claw.can_install);
        assert!(claw.docs.ends_with("openclaw/README.md"), "{}", claw.docs);
    }

    #[test]
    fn a_harness_with_no_unit_never_offers_to_start_one() {
        // Hermes lives inside its own gateway. `systemctl --user enable yantrik-hermes` would
        // fail with a sentence about a unit that was never meant to exist.
        let fixture = Fixture::new("no-unit");
        fixture.harness("hermes", "id: hermes\nname: Hermes\n");
        let hermes = row(&rows(&fixture.machine(), &[builtin()]), "hermes").clone();
        assert_eq!(hermes.state, State::Ready);
        assert!(!hermes.can_start);
        assert!(hermes.need.contains("its own service"), "{}", hermes.need);
    }

    #[test]
    fn no_row_can_carry_what_is_in_a_config_file() {
        // The rule the issue states outright: a row names the file and the variable, nothing
        // else. Nothing here opens a config, and this is the test that keeps it that way.
        let fixture = Fixture::new("no-keys");
        let dir = fixture.harness("pi", PI);
        std::fs::write(dir.join("yantrik_pi.py"), "# harness\n").unwrap();
        fixture.program("pi");
        fixture.config("pi.json", r#"{"api_key": "sk-do-not-print-me-0001"}"#);

        for row in rows(&fixture.machine(), &[builtin(), entry("pi", true)]) {
            let everything = format!(
                "{} {} {} {} {} {}",
                row.name, row.detail, row.need, row.log, row.docs, row.unit
            );
            assert!(!everything.contains("sk-do-not-print-me"), "a row carried a key: {everything}");
        }
    }

    #[test]
    fn the_builtin_comes_first_and_strays_come_last() {
        let fixture = Fixture::new("order");
        fixture.harness("pi", PI);
        fixture.harness("deepseek", "id: deepseek\nname: DeepSeek\n");
        let rows = rows(&fixture.machine(), &[builtin(), entry("mind", false)]);
        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["companion", "deepseek", "pi", "mind"]);
        assert!(rows[0].builtin);
        assert_eq!(rows[3].state, State::Attached, "an attached mind with no manifest is just here");
    }

    #[test]
    fn a_directory_with_no_manifest_is_not_a_harness() {
        // `lib` is shared Python and `tests` is a test suite. Neither is a mind.
        let fixture = Fixture::new("no-manifest");
        std::fs::create_dir_all(fixture.share().join("lib")).unwrap();
        std::fs::write(fixture.share().join("lib/yantrik_harness.py"), "# shared\n").unwrap();
        fixture.harness("pi", PI);
        let rows = rows(&fixture.machine(), &[builtin()]);
        assert_eq!(rows.len(), 2, "{:?}", rows.iter().map(|r| &r.id).collect::<Vec<_>>());
    }

    #[test]
    fn a_broken_manifest_costs_one_row_and_not_the_page() {
        let fixture = Fixture::new("broken");
        fixture.harness("pi", PI);
        fixture.harness("wat", "id: [this is not a string\n");
        fixture.harness("noid", "name: Nameless\n");
        let rows = rows(&fixture.machine(), &[builtin()]);
        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["companion", "pi"]);
    }

    #[test]
    fn the_first_root_that_has_an_id_wins() {
        // A checkout shadowing the image, which is how this gets developed.
        let fixture = Fixture::new("shadow");
        fixture.harness("pi", PI);
        let other = fixture.root.join("checkout");
        std::fs::create_dir_all(other.join("pi")).unwrap();
        std::fs::write(other.join("pi").join(MANIFEST), "id: pi\nname: Pi from a checkout\n")
            .unwrap();

        let manifests = read_manifests(&[other.clone(), fixture.share()]);
        assert_eq!(manifests["pi"].name, "Pi from a checkout");
        let manifests = read_manifests(&[fixture.share(), other]);
        assert_eq!(manifests["pi"].name, "Pi");
    }

    #[test]
    fn a_path_a_row_shows_is_one_a_person_can_read() {
        // Hermes's documentation is two directories above its own, and
        // `/opt/yantrik/share/harnesses/hermes/../../docs/harness.md` on a settings card is a
        // path nobody reads at a glance.
        let home = Path::new("/home/someone");
        assert_eq!(
            display_path(Path::new("/opt/yantrik/share/harnesses/hermes/../../docs/harness.md"), home),
            "/opt/yantrik/share/docs/harness.md"
        );
        assert_eq!(
            display_path(Path::new("/home/someone/.config/yantrik/pi.json"), home),
            "~/.config/yantrik/pi.json"
        );
        // Cleaned lexically, so a `..` that cannot be cancelled is left where it is rather than
        // quietly changing which directory the path names.
        assert_eq!(display_path(Path::new("../../elsewhere"), home), "../../elsewhere");
    }

    #[test]
    fn dir_in_an_install_command_becomes_where_the_manifest_was_found() {
        let fixture = Fixture::new("dir-substitution");
        let dir = fixture.harness(
            "hermes",
            "id: hermes\ninstall:\n  command: cp -r \"{dir}/.\" ~/.hermes/plugins/yantrik/\n",
        );
        let manifests = read_manifests(&[fixture.share()]);
        let install = manifests["hermes"].install.clone().unwrap();
        assert_eq!(
            install.command_in(&manifests["hermes"].dir),
            format!("cp -r \"{}/.\" ~/.hermes/plugins/yantrik/", dir.display())
        );
    }

    #[test]
    fn every_manifest_this_repo_ships_parses_and_says_enough_to_act_on() {
        // The manifests are data, and data in a repository rots quietly. This reads the real
        // ones from the checkout rather than a fixture.
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../harnesses");
        if !repo.exists() {
            return; // built somewhere the sources are not, which is fine
        }
        let manifests = read_manifests(&[repo]);
        for id in ["hermes", "pi", "deepseek", "openclaw"] {
            let manifest = manifests
                .get(id)
                .unwrap_or_else(|| panic!("harnesses/{id} has no {MANIFEST}"));
            assert!(!manifest.name.is_empty(), "{id} has no name");
            assert!(!manifest.docs.is_empty(), "{id} points at no documentation");
            assert!(!manifest.requires.is_empty(), "{id} claims to need nothing at all");
            for need in manifest.requires.iter().chain(&manifest.setup) {
                let named = [&need.binary, &need.file, &need.config]
                    .iter()
                    .filter(|f| !f.is_empty())
                    .count();
                assert_eq!(named, 1, "{id}: a need names exactly one of binary/file/config");
                assert!(!need.why.trim().is_empty(), "{id}: a need with no `why` is a dead end");
            }
        }
        // The one that has to be startable from the page, because it is the one that was found
        // dead on the machine.
        assert_eq!(manifests["pi"].unit, "yantrik-pi.service");
        assert_eq!(manifests["deepseek"].unit, "yantrik-deepseek.service");
        assert!(manifests["hermes"].unit.is_empty(), "Hermes is started by Hermes");
    }
}
