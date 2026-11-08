//! A Mind's workshop: the one folder it may build in, sealed (issue #1120).
//!
//! The rules of the folder and the grant record. The actions are `control_workshop`; the sandbox
//! is `yantrik_agent_terminal::Sealed`.
//!
//! # The folder
//!
//! A new folder under `~/Documents`, found by realpath and not by what the caller typed:
//!
//! - written with `~/` or absolute, never relative, and with no `..` and no control character;
//! - never `$HOME`, Desktop or Downloads, and `~/Documents` itself must be a real directory
//!   directly in the home (a `Documents` that is a link elsewhere is refused);
//! - no component from `Documents` down is a symlink, and none starts with a dot;
//! - at most the last component is missing (the shell creates it), and a folder that exists must
//!   be empty, so it holds no dotfile and nothing the person already keeps there.
//!
//! The same rules run again, minus the emptiness, every time a command is started: a folder
//! swapped for a link, or moved, after the grant is refused then.
//!
//! # The grant is the folder, not its name (#1124, M2)
//!
//! A grant records the folder's device and inode, and the marker the shell put on it
//! (`yantrik_agent_terminal::planted`). Every start checks all three. A folder that was deleted
//! and made again, or unzipped over, under the same name is another folder: the grant is deleted
//! and nothing runs. A folder that is gone revokes the grant too ("revoked by deleting the
//! folder"), and `list` drops such grants as it reads. The window between this check and the
//! sandbox's mount is closed by the same fact: only a process outside the sandbox can rename the
//! folder, and that is the person's own account, which already holds everything the grant could give.
//!
//! One folder is one Mind's: a second grant of a folder another Mind holds is refused, so no
//! folder is shared between two conversations (#1124, L2, and the channel M1 closes).
//!
//! # The grant
//!
//! One workshop per Mind, in `~/.local/share/yantrik/workshops.json`, written by the shell only
//! (atomically, mode 0600). A record that cannot be read grants nothing. Revoking is deleting the
//! Mind's entry. A command is sealed because this record says so, never because a caller says it.
//!
//! The record is as safe as the person's own account: a program running as the person, outside
//! the sandbox, could write it. The run-time rules above are what such a forged entry still meets.

use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

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

/// Where the shell keeps the record. Under test, a file of the run's own.
pub fn record_path() -> PathBuf {
    #[cfg(test)]
    return std::env::temp_dir().join(format!("yantrik-workshops-under-test-{}.json", std::process::id()));
    #[cfg(not(test))]
    return home().join(".local/share/yantrik/workshops.json");
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

/// What a folder may be called, and where it is, for the rules above. `for_grant` adds the
/// first-grant rules (the folder is new, or empty) and creates the last component when missing.
pub fn check_folder(home: &Path, raw: &str, for_grant: bool) -> Result<PathBuf, String> {
    let raw = raw.trim();
    // No control character and no format character (bidi controls, zero-width, tags): a name that
    // draws differently on the card than it reads on disk is a name nobody can approve (#1124, L4).
    if raw.is_empty() || raw.len() > 400 || raw.chars().any(|c| c.is_control() || crate::approvals::is_format_char(c)) {
        return Err("`folder` is a plain path like ~/Documents/sitcom.".into());
    }
    let home_real = home.canonicalize().map_err(|e| format!("the home folder cannot be resolved: {e}"))?;
    let lexical = match raw.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None => PathBuf::from(raw),
    };
    if !lexical.is_absolute() {
        return Err("`folder` is written from ~/ or from /, never relative.".into());
    }
    let mut parts: Vec<&std::ffi::OsStr> = Vec::new();
    for c in lexical.components() {
        match c {
            Component::Normal(n) => parts.push(n),
            Component::RootDir | Component::Prefix(_) => {}
            Component::ParentDir | Component::CurDir => return Err("`folder` has no `..` or `.` in it.".into()),
        }
    }
    // Under Documents, by name: the home as it was written, or as it resolves.
    let documents = home_real.join("Documents");
    let below: Vec<&std::ffi::OsStr> = [home, home_real.as_path()]
        .into_iter()
        .find_map(|h| {
            let n = h.components().filter(|c| matches!(c, Component::Normal(_))).count();
            let base: Vec<&std::ffi::OsStr> = h.components().filter_map(|c| if let Component::Normal(n) = c { Some(n) } else { None }).collect();
            (parts.len() > n && parts[..n] == base[..]).then(|| parts[n..].to_vec())
        })
        .ok_or("a workshop is a new folder under ~/Documents, and never the home folder, Desktop or Downloads.")?;
    if below.first().map(|n| n.to_str()) != Some(Some("Documents")) || below.len() < 2 {
        return Err("a workshop is a new folder under ~/Documents, and never the home folder, Desktop or Downloads.".into());
    }
    let docs_meta = std::fs::symlink_metadata(&documents).map_err(|_| "~/Documents does not exist here.".to_string())?;
    if !docs_meta.is_dir() {
        return Err("~/Documents is not a plain folder (it is a link or a file), so nothing is granted under it.".into());
    }
    let mut cur = documents.clone();
    let rest = &below[1..];
    for (i, name) in rest.iter().enumerate() {
        let name = name.to_str().ok_or("`folder` has a name that is not text.")?;
        if name.starts_with('.') {
            return Err(format!("`{name}` starts with a dot: a workshop holds no hidden folders."));
        }
        cur.push(name);
        match std::fs::symlink_metadata(&cur) {
            Ok(m) if m.file_type().is_symlink() => return Err(format!("{} is a symlink; a workshop is never reached through one.", cur.display())),
            Ok(m) if !m.is_dir() => return Err(format!("{} is not a folder.", cur.display())),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && for_grant && i + 1 == rest.len() => {
                std::fs::create_dir(&cur).map_err(|e| format!("cannot make {}: {e}", cur.display()))?;
                set_private(&cur);
            }
            Err(_) => return Err(format!("{} does not exist; only the last folder of a workshop is made for you.", cur.display())),
        }
    }
    let real = cur.canonicalize().map_err(|e| format!("{} cannot be resolved: {e}", cur.display()))?;
    if real != cur || !real.starts_with(&documents) || real == documents || real == home_real {
        return Err(format!("{} does not resolve to where it says; refused.", cur.display()));
    }
    for banned in ["Desktop", "Downloads"] {
        if real.starts_with(home_real.join(banned)) {
            return Err(format!("{banned} is never a workshop."));
        }
    }
    if for_grant {
        let mut entries = std::fs::read_dir(&real).map_err(|e| format!("{} cannot be read: {e}", real.display()))?;
        if let Some(first) = entries.next() {
            let name = first.map(|e| e.file_name().to_string_lossy().into_owned()).unwrap_or_default();
            return Err(if name.starts_with('.') {
                format!("{} holds a hidden file ({name}); a workshop is a new, empty folder.", real.display())
            } else {
                format!("{} already holds files; a workshop is a new, empty folder.", real.display())
            });
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
        std::fs::create_dir(h.path().join("Desktop")).unwrap();
        std::fs::create_dir(h.path().join("Downloads")).unwrap();
        std::fs::create_dir(h.path().join(".config")).unwrap();
        h
    }

    fn refused(h: &tempfile::TempDir, raw: &str) -> String {
        check_folder(h.path(), raw, true).expect_err(raw)
    }

    #[test]
    fn a_new_folder_under_documents_is_made_and_resolved() {
        let h = home();
        let got = check_folder(h.path(), "~/Documents/sitcom", true).unwrap();
        assert_eq!(got, h.path().canonicalize().unwrap().join("Documents/sitcom"));
        assert!(got.is_dir());
        // Run time: the same folder, now existing and still empty or not, passes without the grant rules.
        std::fs::write(got.join("a.txt"), "x").unwrap();
        assert_eq!(check_folder(h.path(), got.to_str().unwrap(), false).unwrap(), got);
        assert!(refused(&h, "~/Documents/sitcom").contains("already holds"));
    }

    #[test]
    fn home_desktop_downloads_and_elsewhere_are_refused() {
        let h = home();
        let p = h.path().display().to_string();
        for raw in [
            "~", "~/", "~/Desktop", "~/Desktop/x", "~/Downloads/x", "~/Documents", "~/Documents/",
            "/tmp/x", "/", "relative/dir", "", "~/other/x", &p, &format!("{p}/Desktop/x"),
        ] {
            assert!(check_folder(h.path(), raw, true).is_err(), "{raw:?} must be refused");
        }
    }

    #[test]
    fn dotdot_and_hidden_names_are_refused() {
        let h = home();
        assert!(refused(&h, "~/Documents/../.config").contains(".."));
        assert!(refused(&h, "~/Documents/x/../../Desktop/y").contains(".."));
        assert!(refused(&h, "~/Documents/.hidden").contains("dot"));
        assert!(refused(&h, "~/Documents/a\nb").contains("plain path"));
        assert!(!h.path().join("Documents/.hidden").exists(), "a refused folder is not made");
    }

    #[test]
    fn symlinks_are_refused_at_every_step() {
        let h = home();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), h.path().join("Documents/link")).unwrap();
        assert!(refused(&h, "~/Documents/link").contains("symlink"));
        assert!(refused(&h, "~/Documents/link/new").contains("symlink"));
        // A Documents that is itself a link to somewhere else.
        let h2 = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), h2.path().join("Documents")).unwrap();
        assert!(check_folder(h2.path(), "~/Documents/x", true).is_err());
        // Granted, then swapped for a link: the run-time check refuses.
        let real = check_folder(h.path(), "~/Documents/shop", true).unwrap();
        std::fs::remove_dir(&real).unwrap();
        std::os::unix::fs::symlink(outside.path(), &real).unwrap();
        assert!(check_folder(h.path(), real.to_str().unwrap(), false).unwrap_err().contains("symlink"));
    }

    #[test]
    fn a_folder_with_dotfiles_or_files_is_not_new() {
        let h = home();
        std::fs::create_dir(h.path().join("Documents/old")).unwrap();
        std::fs::write(h.path().join("Documents/old/.env"), "k=v").unwrap();
        assert!(refused(&h, "~/Documents/old").contains("hidden"));
        std::fs::create_dir(h.path().join("Documents/empty")).unwrap();
        assert!(check_folder(h.path(), "~/Documents/empty", true).is_ok());
        // Only the last folder is made for you.
        assert!(refused(&h, "~/Documents/a/b").contains("does not exist"));
    }

    #[test]
    fn the_record_grants_per_mind_and_revoking_deletes() {
        let h = home();
        let rec = h.path().join("workshops.json");
        assert!(load(&rec).unwrap().is_empty());
        let shop = check_folder(h.path(), "~/Documents/shop", true).unwrap();
        grant(&rec, "hermes", &shop, 5).unwrap();
        assert_eq!(granted_for(&rec, h.path(), "hermes", shop.to_str().unwrap()).unwrap(), shop);
        // Another Mind has none, and a different folder is not the grant.
        assert!(granted_for(&rec, h.path(), "pi", shop.to_str().unwrap()).unwrap_err().contains("no workshop"));
        let other = check_folder(h.path(), "~/Documents/other", true).unwrap();
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
        let shop = check_folder(h.path(), "~/Documents/shop", true).unwrap();
        assert!(granted_for(&rec, h.path(), "hermes", shop.to_str().unwrap()).unwrap_err().contains("no workshop is granted"));
        assert!(grant(&rec, "hermes", &shop, 1).is_err(), "a corrupt record is not overwritten");
    }

    #[test]
    fn cwd_stays_inside_the_workshop() {
        let h = home();
        let shop = check_folder(h.path(), "~/Documents/shop", true).unwrap();
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
        std::fs::create_dir(h.path().join("Documents/shop2")).unwrap();
        assert!(cwd_inside(&shop, Some(&h.path().canonicalize().unwrap().join("Documents/shop2"))).is_err());
    }
}

#[cfg(all(test, unix))]
mod grant_tests {
    use super::*;

    fn home() -> tempfile::TempDir {
        let h = tempfile::tempdir().unwrap();
        std::fs::create_dir(h.path().join("Documents")).unwrap();
        h
    }

    /// #1124 M2: the grant is the folder. Deleting it revokes the grant, and a new folder that
    /// takes the same name, with the person's own files in it, gets nothing.
    #[test]
    fn a_folder_deleted_and_made_again_under_the_same_name_is_not_the_granted_folder() {
        let h = home();
        let rec = h.path().join("workshops.json");
        let shop = check_folder(h.path(), "~/Documents/project", true).unwrap();
        grant(&rec, "hermes", &shop, 1).unwrap();
        assert!(granted_for(&rec, h.path(), "hermes", shop.to_str().unwrap()).is_ok());
        // Gone: the grant is deleted as the record is read.
        std::fs::remove_dir(&shop).unwrap();
        assert!(granted_for(&rec, h.path(), "hermes", shop.to_str().unwrap()).is_err());
        assert!(load(&rec).unwrap().is_empty(), "deleting the folder revoked the grant");
        // Replaced: the person unzips their own project there, dotfiles and all.
        grant_again(&rec, h.path(), "hermes");
        let shop = h.path().canonicalize().unwrap().join("Documents/project");
        std::fs::remove_dir(&shop).unwrap();
        std::fs::create_dir(&shop).unwrap();
        std::fs::write(shop.join(".env"), "KEY=secret").unwrap();
        let why = granted_for(&rec, h.path(), "hermes", shop.to_str().unwrap()).unwrap_err();
        // The grant is deleted as it is read, so the Mind is told it has none.
        assert!(why.contains("no workshop"), "{why}");
        assert!(load(&rec).unwrap().is_empty(), "the replaced folder's grant was deleted");
    }

    fn grant_again(rec: &Path, home: &Path, agent: &str) {
        let shop = check_folder(home, "~/Documents/project", true).unwrap();
        grant(rec, agent, &shop, 2).unwrap();
    }

    #[test]
    fn a_record_without_an_inode_matches_no_folder() {
        let h = home();
        let rec = h.path().join("workshops.json");
        let shop = check_folder(h.path(), "~/Documents/old", true).unwrap();
        let old = format!(r#"{{"grants":[{{"agent":"hermes","folder":"{}","granted_unix":1}}]}}"#, shop.display());
        std::fs::write(&rec, old).unwrap();
        assert!(granted_for(&rec, h.path(), "hermes", shop.to_str().unwrap()).is_err());
    }

    /// #1124 L2: a folder belongs to one Mind.
    #[test]
    fn a_second_mind_cannot_take_a_folder_the_first_holds() {
        let h = home();
        let rec = h.path().join("workshops.json");
        let shop = check_folder(h.path(), "~/Documents/shared", true).unwrap();
        grant(&rec, "hermes", &shop, 1).unwrap();
        // Still empty, so it passes the folder rules for a second grant; the record refuses it.
        let again = check_folder(h.path(), "~/Documents/shared", true).unwrap();
        assert!(grant(&rec, "pi", &again, 2).unwrap_err().contains("another Mind's"));
        assert!(grant(&rec, "hermes", &again, 3).is_ok(), "the holder may ask again");
    }

    /// #1124 L4: nothing that draws differently from how it reads.
    #[test]
    fn bidi_and_other_format_characters_in_a_folder_name_are_refused() {
        let h = home();
        for bad in ["\u{202E}gpj.x", "a\u{200B}b", "a\u{2066}b", "a\u{FEFF}b", "a\u{00AD}b"] {
            let raw = format!("~/Documents/{bad}");
            assert!(check_folder(h.path(), &raw, true).is_err(), "{bad:?}");
        }
        assert!(check_folder(h.path(), "~/Documents/sitcom-épisode", true).is_ok());
    }
}
