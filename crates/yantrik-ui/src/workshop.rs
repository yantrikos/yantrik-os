//! A Mind's workshop: the one folder it may build in, sealed (issue #1120).
//!
//! The rules of the folder and the grant record. The actions are `control_workshop`; the sandbox
//! is `yantrik_agent_terminal::Sealed`; the root and the check that keeps the companion's tools
//! out of it are `home_paths::workshops`.
//!
//! # The folder
//!
//! `~/.local/share/yantrik/workshops/<mind>/<name>`, and nowhere else. It is private (0700) and
//! always planted: what a Mind writes there is never run, read or opened by anything outside a
//! mount view that cannot see it (sealed, or the outside view that hides the root). A folder is
//! found by realpath and not by what the caller typed:
//!
//! - written with `~/` or absolute, exactly two components under the root, no `..`, no control
//!   character and no name starting with a dot;
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
//! sealed view has only its own workshop, the outside view has a tmpfs where the root is, and the
//! companion's tools refuse every path under the root (`Root::holds`). A Mind's job that could
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
    return record_in(&home());
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
    let dir = path.parent().ok_or("the workshop record has no folder")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot make {}: {e}", dir.display()))?;
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

/// The folder for `name` in `agent`'s place under the root, as `check_folder` would read it.
pub fn folder_for(home: &Path, agent: &str, name: &str, for_grant: bool) -> Result<PathBuf, String> {
    plain_name("`name`", name.trim())?;
    plain_name("the Mind's id", agent)?;
    let home_real = home.canonicalize().map_err(|e| format!("the home folder cannot be resolved: {e}"))?;
    let at = workshops::root(&home_real).join(agent).join(name.trim());
    check_folder(home, &at.to_string_lossy(), for_grant)
}

/// What a folder may be called, and where it is, for the rules above. `for_grant` adds the
/// first-grant rules (the folder is new, or empty) and creates the root, the Mind's folder and the
/// workshop when missing, each private.
pub fn check_folder(home: &Path, raw: &str, for_grant: bool) -> Result<PathBuf, String> {
    let raw = raw.trim();
    // No control character and no format character (bidi controls, zero-width, tags): a name that
    // draws differently on the card than it reads on disk is a name nobody can approve (#1124, L4).
    if raw.is_empty() || raw.len() > 400 || raw.chars().any(|c| c.is_control() || crate::approvals::is_format_char(c)) {
        return Err("`folder` is a plain path.".into());
    }
    let home_real = home.canonicalize().map_err(|e| format!("the home folder cannot be resolved: {e}"))?;
    let lexical = match raw.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None => PathBuf::from(raw),
    };
    if !lexical.is_absolute() {
        return Err("`folder` is written from ~/ or from /, never relative.".into());
    }
    let nowhere = || format!("a workshop is ~/{}/<mind>/<name>, and nowhere else.", workshops::ROOT_UNDER_HOME);
    let root = workshops::root(&home_real);
    let below = [workshops::root(home), root.clone()].iter().find_map(|r| lexical.strip_prefix(r).ok().map(Path::to_path_buf)).ok_or_else(nowhere)?;
    let parts: Vec<&std::ffi::OsStr> = match below.components().map(|c| if let Component::Normal(n) = c { Some(n) } else { None }).collect() {
        Some(parts) => parts,
        None => return Err("`folder` has no `..` or `.` in it.".into()),
    };
    if parts.len() != 2 {
        return Err(nowhere());
    }
    let mut cur = root.clone();
    for name in std::iter::once(None).chain(parts.iter().map(|n| Some(*n))) {
        if let Some(name) = name {
            let name = name.to_str().ok_or("`folder` has a name that is not text.")?;
            if name.starts_with('.') {
                return Err(format!("`{name}` starts with a dot: a workshop holds no hidden folders."));
            }
            cur.push(name);
        }
        match std::fs::symlink_metadata(&cur) {
            Ok(m) if m.file_type().is_symlink() => return Err(format!("{} is a symlink; a workshop is never reached through one.", cur.display())),
            Ok(m) if !m.is_dir() => return Err(format!("{} is not a folder.", cur.display())),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && for_grant => {
                std::fs::create_dir_all(&cur).map_err(|e| format!("cannot make {}: {e}", cur.display()))?;
                set_private(&cur);
            }
            Err(_) => return Err(format!("{} does not exist.", cur.display())),
        }
    }
    let real = cur.canonicalize().map_err(|e| format!("{} cannot be resolved: {e}", cur.display()))?;
    if real != cur {
        return Err(format!("{} does not resolve to where it says; refused.", cur.display()));
    }
    if for_grant {
        let mut entries = std::fs::read_dir(&real).map_err(|e| format!("{} cannot be read: {e}", real.display()))?;
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
mod tests {
    use super::*;

    fn home() -> tempfile::TempDir {
        let h = tempfile::tempdir().unwrap();
        std::fs::create_dir(h.path().join("Documents")).unwrap();
        h
    }

    fn refused(h: &tempfile::TempDir, raw: &str) -> String {
        check_folder(h.path(), raw, true).expect_err(raw)
    }

    const AT: &str = "~/.local/share/yantrik/workshops";

    #[test]
    fn a_new_folder_under_the_root_is_made_private_and_resolved() {
        let h = home();
        let got = check_folder(h.path(), &format!("{AT}/hermes/sitcom"), true).unwrap();
        let real = h.path().canonicalize().unwrap();
        assert_eq!(got, real.join(".local/share/yantrik/workshops/hermes/sitcom"));
        use std::os::unix::fs::PermissionsExt;
        for dir in [got.clone(), got.parent().unwrap().to_path_buf(), workshops::root(&real)] {
            assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700, "{}", dir.display());
        }
        // Run time: the same folder, now holding files, passes without the grant rules.
        std::fs::write(got.join("a.txt"), "x").unwrap();
        assert_eq!(check_folder(h.path(), got.to_str().unwrap(), false).unwrap(), got);
        assert!(refused(&h, &format!("{AT}/hermes/sitcom")).contains("already holds"));
        assert_eq!(folder_for(h.path(), "hermes", "sitcom", false).unwrap(), got);
    }

    #[test]
    fn only_the_root_is_accepted() {
        let h = home();
        let p = h.path().display().to_string();
        for raw in [
            "~", "~/", "~/Documents", "~/Documents/x", "~/Desktop/x", "~/Downloads/x", "/tmp/x", "/", "relative/dir", "", &p,
            &format!("{p}/Documents/x"),
            AT, &format!("{AT}/"), &format!("{AT}/hermes"), &format!("{AT}/hermes/a/b"), &format!("{AT}/hermes/"),
            "~/.local/share/yantrik/workshops2/hermes/x", "~/.local/share/yantrik/hermes/x",
        ] {
            assert!(check_folder(h.path(), raw, true).is_err(), "{raw:?} must be refused");
        }
        assert!(!workshops::root(h.path()).exists(), "a refused folder makes nothing");
    }

    #[test]
    fn dotdot_and_hidden_names_are_refused() {
        let h = home();
        assert!(check_folder(h.path(), &format!("{AT}/../../../../Documents/x"), true).is_err());
        assert!(refused(&h, &format!("{AT}/hermes/../pi/x")).contains(".."));
        assert!(refused(&h, &format!("{AT}/hermes/.hidden")).contains("dot"));
        assert!(refused(&h, &format!("{AT}/.hermes/x")).contains("dot"));
        assert!(refused(&h, &format!("{AT}/hermes/a\nb")).contains("plain path"));
        assert!(!workshops::root(h.path()).join("hermes/.hidden").exists());
        for bad in ["a/b", ".x", "..", "", "."] {
            assert!(folder_for(h.path(), "hermes", bad, true).is_err(), "{bad:?}");
        }
        assert!(folder_for(h.path(), "../pi", "x", true).is_err());
    }

    #[test]
    fn symlinks_are_refused_at_every_step() {
        let h = home();
        let outside = tempfile::tempdir().unwrap();
        let real = h.path().canonicalize().unwrap();
        let root = workshops::root(&real);
        std::fs::create_dir_all(root.join("hermes")).unwrap();
        std::os::unix::fs::symlink(outside.path(), root.join("hermes/link")).unwrap();
        std::os::unix::fs::symlink(outside.path(), root.join("pi")).unwrap();
        assert!(refused(&h, &format!("{AT}/hermes/link")).contains("symlink"));
        assert!(refused(&h, &format!("{AT}/pi/new")).contains("symlink"));
        // The root itself a link to somewhere else.
        let h2 = home();
        std::fs::create_dir_all(h2.path().join(".local/share/yantrik")).unwrap();
        std::os::unix::fs::symlink(outside.path(), workshops::root(h2.path())).unwrap();
        assert!(check_folder(h2.path(), &format!("{AT}/hermes/x"), true).is_err());
        // Granted, then swapped for a link: the run-time check refuses.
        let shop = check_folder(h.path(), &format!("{AT}/hermes/shop"), true).unwrap();
        std::fs::remove_dir(&shop).unwrap();
        std::os::unix::fs::symlink(outside.path(), &shop).unwrap();
        assert!(check_folder(h.path(), shop.to_str().unwrap(), false).unwrap_err().contains("symlink"));
    }

    #[test]
    fn a_folder_with_files_is_not_new() {
        let h = home();
        let shop = check_folder(h.path(), &format!("{AT}/hermes/old"), true).unwrap();
        std::fs::write(shop.join(".env"), "k=v").unwrap();
        assert!(refused(&h, &format!("{AT}/hermes/old")).contains("already holds"));
        std::fs::create_dir(workshops::root(&h.path().canonicalize().unwrap()).join("hermes/empty")).unwrap();
        assert!(check_folder(h.path(), &format!("{AT}/hermes/empty"), true).is_ok());
        // Not made for a card's sentence, or for a run.
        assert!(check_folder(h.path(), &format!("{AT}/hermes/never"), false).unwrap_err().contains("does not exist"));
    }

    #[test]
    fn the_record_grants_per_mind_and_revoking_deletes() {
        let h = home();
        let rec = h.path().join("workshops.json");
        assert!(load(&rec).unwrap().is_empty());
        let shop = folder_for(h.path(), "hermes", "shop", true).unwrap();
        grant(&rec, "hermes", &shop, 5).unwrap();
        assert_eq!(granted_for(&rec, h.path(), "hermes", shop.to_str().unwrap()).unwrap(), shop);
        // Another Mind has none, and a different folder is not the grant.
        assert!(granted_for(&rec, h.path(), "pi", shop.to_str().unwrap()).unwrap_err().contains("no workshop"));
        let other = folder_for(h.path(), "hermes", "other", true).unwrap();
        assert!(granted_for(&rec, h.path(), "hermes", other.to_str().unwrap()).unwrap_err().contains("not the workshop"));
        // A new grant replaces; the file is private.
        grant(&rec, "hermes", &other, 6).unwrap();
        assert_eq!(load(&rec).unwrap().len(), 1);
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&rec).unwrap().permissions().mode() & 0o777, 0o600);
        assert!(revoke(&rec, "hermes").unwrap());
        assert!(!revoke(&rec, "hermes").unwrap());
        assert!(granted_for(&rec, h.path(), "hermes", other.to_str().unwrap()).is_err());
    }

    #[test]
    fn a_record_that_cannot_be_read_grants_nothing() {
        let h = home();
        let rec = h.path().join("workshops.json");
        std::fs::write(&rec, "{not json").unwrap();
        let shop = folder_for(h.path(), "hermes", "shop", true).unwrap();
        assert!(granted_for(&rec, h.path(), "hermes", shop.to_str().unwrap()).unwrap_err().contains("no workshop is granted"));
        assert!(grant(&rec, "hermes", &shop, 1).is_err(), "a corrupt record is not overwritten");
    }

    #[test]
    fn cwd_stays_inside_the_workshop() {
        let h = home();
        let shop = folder_for(h.path(), "hermes", "shop", true).unwrap();
        std::fs::create_dir(shop.join("app")).unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), shop.join("escape")).unwrap();
        assert_eq!(cwd_inside(&shop, None).unwrap(), shop);
        assert_eq!(cwd_inside(&shop, Some(Path::new("app"))).unwrap(), shop.join("app"));
        assert_eq!(cwd_inside(&shop, Some(&shop.join("app"))).unwrap(), shop.join("app"));
        for bad in ["..", "app/../..", "escape", "escape/x", "/", "/tmp", "missing"] {
            assert!(cwd_inside(&shop, Some(Path::new(bad))).is_err(), "{bad}");
        }
        assert!(cwd_inside(&shop, Some(h.path())).is_err());
        // A sibling that merely shares the prefix.
        let sibling = folder_for(h.path(), "hermes", "shop2", true).unwrap();
        assert!(cwd_inside(&shop, Some(&sibling)).is_err());
    }

    /// #1124 M2: the grant is the folder. Deleting it revokes the grant, and a new folder that
    /// takes the same name, with the person's own files in it, gets nothing.
    #[test]
    fn a_folder_deleted_and_made_again_under_the_same_name_is_not_the_granted_folder() {
        let h = home();
        let rec = h.path().join("workshops.json");
        let shop = folder_for(h.path(), "hermes", "project", true).unwrap();
        grant(&rec, "hermes", &shop, 1).unwrap();
        assert!(granted_for(&rec, h.path(), "hermes", shop.to_str().unwrap()).is_ok());
        std::fs::remove_dir(&shop).unwrap();
        assert!(granted_for(&rec, h.path(), "hermes", shop.to_str().unwrap()).is_err());
        assert!(prune(&rec).unwrap().is_empty(), "a gone folder is not a workshop");
        // Replaced by another folder with the same name.
        let shop = folder_for(h.path(), "hermes", "project", true).unwrap();
        grant(&rec, "hermes", &shop, 2).unwrap();
        std::fs::remove_dir(&shop).unwrap();
        std::fs::create_dir(&shop).unwrap();
        std::fs::write(shop.join(".env"), "KEY=secret").unwrap();
        let why = granted_for(&rec, h.path(), "hermes", shop.to_str().unwrap()).unwrap_err();
        assert!(why.contains("no workshop"), "{why}");
    }

    #[test]
    fn a_record_without_an_inode_matches_no_folder() {
        let h = home();
        let rec = h.path().join("workshops.json");
        let shop = folder_for(h.path(), "hermes", "old", true).unwrap();
        let old = format!(r#"{{"grants":[{{"agent":"hermes","folder":"{}","granted_unix":1}}]}}"#, shop.display());
        std::fs::write(&rec, old).unwrap();
        assert!(granted_for(&rec, h.path(), "hermes", shop.to_str().unwrap()).is_err());
    }

    /// #1124 L2: a folder belongs to one Mind.
    #[test]
    fn a_second_mind_cannot_take_a_folder_the_first_holds() {
        let h = home();
        let rec = h.path().join("workshops.json");
        let shop = folder_for(h.path(), "hermes", "shared", true).unwrap();
        grant(&rec, "hermes", &shop, 1).unwrap();
        assert!(grant(&rec, "pi", &shop, 2).unwrap_err().contains("another Mind's"));
        assert!(grant(&rec, "hermes", &shop, 3).is_ok(), "the holder may ask again");
    }

    /// #1124 L4: nothing that draws differently from how it reads.
    #[test]
    fn bidi_and_other_format_characters_in_a_folder_name_are_refused() {
        let h = home();
        for bad in ["\u{202E}gpj.x", "a\u{200B}b", "a\u{2066}b", "a\u{FEFF}b", "a\u{00AD}b"] {
            assert!(folder_for(h.path(), "hermes", bad, true).is_err(), "{bad:?}");
        }
        assert!(folder_for(h.path(), "hermes", "sitcom-\u{e9}pisode", true).is_ok());
    }

    /// #1124 M4: the record is inside the root, so every view that hides the root hides it, and
    /// every in-process tool that refuses the root refuses it. No name a Mind chooses can be it.
    #[test]
    fn the_grant_record_is_inside_the_root_where_no_view_of_a_mind_reaches() {
        let h = home();
        let home = h.path().canonicalize().unwrap();
        let rec = record_in(&home);
        assert_eq!(rec, workshops::root(&home).join(".grants.json"));
        let shop = folder_for(&home, "hermes", "shop", true).unwrap();
        grant(&rec, "hermes", &shop, 1).unwrap();
        assert!(rec.is_file() && workshops::holds(&rec, &home), "the tools that refuse the root refuse the record");
        // The outside view is a tmpfs over the root, so the record is not in it; the sealed view has
        // only the one workshop.
        let masked = workshops::outside_args(&workshops::root(&home));
        assert!(masked.windows(2).any(|w| w[0] == "--tmpfs" && Path::new(&w[1]) == rec.parent().unwrap()));
        assert!(!masked.iter().any(|a| a.contains(".grants")), "nothing re-exposes it");
        for name in [".grants.json", ".grants"] {
            assert!(plain_name("the Mind's id", name).is_err() && folder_for(&home, name, "x", true).is_err(), "{name}");
            assert!(folder_for(&home, "hermes", name, true).is_err(), "{name}");
        }
        assert!(!home.join(".local/share/yantrik/workshops.json").exists(), "not beside the root");
    }
}
