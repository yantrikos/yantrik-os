//! A Mind's workshop: the one folder it may build in, sealed (issue #1120).
//!
//! The rules of the folder and the grant record. The actions are `control_workshop`; the sandbox
//! is `yantrik_agent_terminal::Sealed`; the root and the check that keeps the companion's tools
//! out of it are `home_paths::workshops`.
//!
//! # The folder
//!
//! `/var/lib/yantrik/workshops/<uid>/<mind>/<name>`, and nowhere else (the root is `home_paths::workshops`:
//! root's parent, the person's 0700 `<uid>/` made by the installer and the updater, never by the shell,
//! pinned by an `O_PATH` descriptor and its (device, inode)). It is private (0700) and
//! always planted: what a Mind writes there is never run, read or opened by anything outside a
//! mount view that cannot see it (sealed, or the outside view that hides the root). A folder is
//! found by realpath and not by what the caller typed:
//!
//! - written absolute, exactly two components under the root, no `..`, no control character and no
//!   name starting with a dot;
//! - the root, the Mind's folder and the workshop are real directories, never links;
//! - a new workshop is made by the shell and must be empty at the grant.
//!
//! The same rules run again, minus the emptiness, every time a command is started: a folder
//! swapped for a link, or moved, after the grant is refused then.
//!
//! # The grant is the folder, not its name (#1124, M2)
//!
//! A grant records the folder's device, inode and birth time. A folder that was deleted and made
//! again under the same name is another folder: it is not listed and nothing runs in it. One
//! folder is one Mind's: a second grant of a folder another Mind holds is refused.
//!
//! # The grant record
//!
//! One workshop per Mind, in `<root>/.grants.json`: inside the root, under its 0700 folder, and
//! written by the shell only (atomically, mode 0600). Nothing a Mind drives can open it: the
//! sealed view has only its own workshop, the outside view has a tmpfs over the whole of
//! `/var/lib/yantrik/workshops`, and the companion's tools refuse every path under it
//! (`Root::holds`). A Mind's job that could
//! write the record could give itself another Mind's workshop with no card (#1124, M4), which is
//! why it does not sit beside the root. A name starting with a dot is never a Mind's id or a
//! workshop's name, so the file cannot be one. A record that cannot be read grants nothing.
//! Revoking is deleting the Mind's entry. A command is sealed because this record says so, never
//! because a caller says it. The run-time rules above are what a forged entry still meets.

use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use yantrik_ipc_contracts::home_paths::workshops;

/// One Mind's workshop.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    /// The agent's id, as the shell's token check names it.
    pub agent: String,
    /// The folder's realpath when it was granted.
    pub folder: String,
    pub granted_unix: u64,
    /// The folder's device and inode when it was granted (0 in a record that predates them, which
    /// matches nothing).
    #[serde(default)]
    pub dev: u64,
    #[serde(default)]
    pub ino: u64,
    /// When the folder was born (nanoseconds since the epoch; 0 where the filesystem does not
    /// say). A filesystem may hand a deleted folder's inode number to the next one made, and the
    /// birth time tells those two apart.
    #[serde(default)]
    pub born: u64,
}

#[derive(Default, Serialize, Deserialize)]
struct File {
    #[serde(default)]
    grants: Vec<Grant>,
}

/// Writers of the record take this, so two grants cannot lose each other.
static WRITE: Mutex<()> = Mutex::new(());

/// The record for a person whose home is `home`: a file in the root itself.
pub fn record_in(home: &Path) -> PathBuf {
    workshops::root(home).join(".grants.json")
}

/// Where the shell keeps the record. Under test, a file of the run's own.
pub fn record_path() -> PathBuf {
    #[cfg(test)]
    return std::env::temp_dir().join(format!("yantrik-workshops-under-test-{}.json", std::process::id()));
    #[cfg(not(test))]
    {
        // The old root's content moves into the new one before the record is read, once.
        crate::workshop_move::once(&home());
        record_in(&home())
    }
}

pub fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/root".into()))
}

/// Every grant, or why the record cannot be trusted. No file is no grants.
pub fn load(path: &Path) -> Result<Vec<Grant>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str::<File>(&text)
            .map(|f| f.grants)
            .map_err(|e| format!("the workshop record could not be read ({e}), so no workshop is granted")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("the workshop record could not be read ({e}), so no workshop is granted")),
    }
}

fn save(path: &Path, grants: Vec<Grant>) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    // The folder is never made here: the root is the installer's and the updater's to make.
    path.parent().filter(|d| d.is_dir()).ok_or("the workshop record has no folder")?;
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_vec_pretty(&File { grants }).map_err(|e| e.to_string())?;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)
        .map_err(|e| format!("cannot write the workshop record: {e}"))?;
    f.write_all(&body).and_then(|()| f.sync_all()).map_err(|e| format!("cannot write the workshop record: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("cannot write the workshop record: {e}"))
}

/// Record `folder` (already checked) as `agent`'s workshop, replacing its earlier one. Refused when
/// another Mind holds the folder (by name or by inode).
pub fn grant(path: &Path, agent: &str, folder: &Path, now: u64) -> Result<Grant, String> {
    use std::os::unix::fs::MetadataExt;
    let _w = WRITE.lock().unwrap_or_else(|e| e.into_inner());
    let mut grants = load(path)?;
    let meta = std::fs::metadata(folder).map_err(|e| format!("{} cannot be read: {e}", folder.display()))?;
    let (dev, ino, born) = (meta.dev(), meta.ino(), born_of(&meta));
    if grants.iter().any(|g| g.agent != agent && (Path::new(&g.folder) == folder || (g.dev == dev && g.ino == ino && g.ino != 0))) {
        return Err("that folder is already another Mind's workshop; each Mind has a folder of its own.".into());
    }
    grants.retain(|g| g.agent != agent);
    let made = Grant {
        agent: agent.to_string(),
        folder: folder.display().to_string(),
        granted_unix: now,
        dev,
        ino,
        born,
    };
    grants.push(made.clone());
    save(path, grants)?;
    Ok(made)
}

/// Delete `agent`'s grant. Whether there was one.
pub fn revoke(path: &Path, agent: &str) -> Result<bool, String> {
    let _w = WRITE.lock().unwrap_or_else(|e| e.into_inner());
    let mut grants = load(path)?;
    let before = grants.len();
    grants.retain(|g| g.agent != agent);
    let had = grants.len() != before;
    if had {
        save(path, grants)?;
    }
    Ok(had)
}

/// Is `v` one plain name, as a Mind's id or a workshop's name must be?
pub fn plain_name(what: &str, v: &str) -> Result<(), String> {
    let one = matches!(Path::new(v).components().collect::<Vec<_>>().as_slice(), [Component::Normal(_)]);
    let clean = !v.chars().any(|c| c.is_control() || crate::approvals::is_format_char(c));
    (one && clean && !v.starts_with('.')).then_some(()).ok_or_else(|| format!("{what} is one plain name, with no `/` and no leading dot."))
}

/// The root for `home`, held, or why there is none; the reason is also logged (the notice: the
/// person's updater or installer has to make it).
pub fn require_root(home: &Path) -> Result<workshops::OpenRoot, String> {
    workshops::open_root(home).inspect_err(|why| tracing::warn!("workshops: {why}"))
}

/// The folder for `name` in `agent`'s place under the root, as `check_folder` would read it.
pub fn folder_for(home: &Path, agent: &str, name: &str, for_grant: bool) -> Result<PathBuf, String> {
    plain_name("`name`", name.trim())?;
    plain_name("the Mind's id", agent)?;
    let at = workshops::root(home).join(agent).join(name.trim());
    check_folder(home, &at.to_string_lossy(), for_grant)
}

/// What a folder may be called, and where it is, for the rules above. `for_grant` adds the
/// first-grant rules (the folder is new, or empty) and creates the Mind's folder and the
/// workshop when missing, each private, below the root, which is never made here. Every step is
/// resolved through the root's pinned descriptor.
pub fn check_folder(home: &Path, raw: &str, for_grant: bool) -> Result<PathBuf, String> {
    let raw = raw.trim();
    // No control character and no format character (bidi controls, zero-width, tags): a name that
    // draws differently on the card than it reads on disk is a name nobody can approve (#1124, L4).
    if raw.is_empty() || raw.len() > 400 || raw.chars().any(|c| c.is_control() || crate::approvals::is_format_char(c)) {
        return Err("`folder` is a plain path.".into());
    }
    let lexical = PathBuf::from(raw);
    if !lexical.is_absolute() {
        return Err("`folder` is written from /, never relative.".into());
    }
    let nowhere = || format!("a workshop is {}/<mind>/<name>, and nowhere else.", workshops::root(home).display());
    let held = require_root(home)?;
    let root = held.path().to_path_buf();
    let below = lexical.strip_prefix(&root).map(Path::to_path_buf).map_err(|_| nowhere())?;
    let parts: Vec<&std::ffi::OsStr> = match below.components().map(|c| if let Component::Normal(n) = c { Some(n) } else { None }).collect() {
        Some(parts) => parts,
        None => return Err("`folder` has no `..` or `.` in it.".into()),
    };
    if parts.len() != 2 {
        return Err(nowhere());
    }
    let mut cur = root.clone();
    for name in parts {
        let name = name.to_str().ok_or("`folder` has a name that is not text.")?;
        if name.starts_with('.') {
            return Err(format!("`{name}` starts with a dot: a workshop holds no hidden folders."));
        }
        cur.push(name);
        let seen = held.via(&cur);
        match std::fs::symlink_metadata(&seen) {
            Ok(m) if m.file_type().is_symlink() => return Err(format!("{} is a symlink; a workshop is never reached through one.", cur.display())),
            Ok(m) if !m.is_dir() => return Err(format!("{} is not a folder.", cur.display())),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && for_grant => {
                std::fs::create_dir(&seen).map_err(|e| format!("cannot make {}: {e}", cur.display()))?;
                set_private(&seen);
            }
            Err(_) => return Err(format!("{} does not exist.", cur.display())),
        }
    }
    let real = held.via(&cur).canonicalize().map_err(|e| format!("{} cannot be resolved: {e}", cur.display()))?;
    if real != cur {
        return Err(format!("{} does not resolve to where it says; refused.", cur.display()));
    }
    if for_grant {
        let mut entries = std::fs::read_dir(held.via(&real)).map_err(|e| format!("{} cannot be read: {e}", real.display()))?;
        if entries.next().is_some() {
            return Err(format!("{} already holds files; a workshop is a new, empty folder.", real.display()));
        }
    }
    Ok(real)
}

fn set_private(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
}

fn born_of(meta: &std::fs::Metadata) -> u64 {
    meta.created()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos() as u64)
}

/// Is the folder a grant names still the folder that was granted: there, the same device and
/// inode, and still carrying the shell's marker? `Err` says which of these failed.
fn still_the_folder(g: &Grant) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::symlink_metadata(&g.folder).map_err(|_| "the workshop folder is gone, so its grant was deleted.".to_string())?;
    if g.ino == 0 || meta.dev() != g.dev || meta.ino() != g.ino || (g.born != 0 && born_of(&meta) != g.born) {
        return Err("the workshop folder was replaced by another one with the same name, so its grant was deleted.".into());
    }
    Ok(())
}

/// The grants whose folder is still the folder granted; a gone or replaced one is not listed.
pub fn prune(path: &Path) -> Result<Vec<Grant>, String> {
    Ok(load(path)?.into_iter().filter(|g| still_the_folder(g).is_ok()).collect())
}

/// The workshop `agent` was granted, if it is `named` (the `workshop` argument, checked here and
/// never believed): the record names it, it still passes the folder rules, and it is the folder
/// that was granted (device, inode, marker). The one place a command becomes sealed.
pub fn granted_for(path: &Path, home: &Path, agent: &str, named: &str) -> Result<PathBuf, String> {
    let grants = prune(path)?;
    let Some(g) = grants.iter().find(|g| g.agent == agent) else {
        return Err("this Mind has no workshop: ask for one with `workshop_create`, and the person decides. Nothing was run.".into());
    };
    let real = check_folder(home, named, false)?;
    if Path::new(&g.folder) == real {
        Ok(real)
    } else {
        Err("that is not the workshop this Mind was granted, so nothing was run.".into())
    }
}

/// `cwd` (relative to the workshop when relative; the workshop when none) as a real path inside
/// `workshop`. A `..`, a link out and a missing directory are refused.
pub fn cwd_inside(workshop: &Path, cwd: Option<&Path>) -> Result<PathBuf, String> {
    let Some(cwd) = cwd else { return Ok(workshop.to_path_buf()) };
    if cwd.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err("`cwd` has no `..` in it: a sealed command stays in its workshop.".into());
    }
    let full = if cwd.is_absolute() { cwd.to_path_buf() } else { workshop.join(cwd) };
    let real = full.canonicalize().map_err(|_| format!("`cwd` {} is not a directory in the workshop.", full.display()))?;
    if !real.starts_with(workshop) || !real.is_dir() {
        return Err("`cwd` is outside the workshop, so nothing was run.".into());
    }
    Ok(real)
}

#[cfg(all(test, unix))]
#[path = "workshop_tests.rs"]
mod tests;
