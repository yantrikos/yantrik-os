//! Where a Mind's workshops live, the one check that keeps in-process tools out of them, and the
//! view a program a Mind drives gets of the machine (#1120, #1124).
//!
//! A workshop is `<home>/.local/share/yantrik/workshops/<mind>/<name>`, and nowhere else. What a
//! Mind builds there can hold anything a program reads or runs (a git filter, a package hook, a
//! script), so every program a Mind drives is in one of two mount views:
//!
//! - **sealed** (`yantrik_agent_terminal::Sealed`): sees its workshop and nothing else;
//! - **outside** ([`outside_command`]): sees the host, with a tmpfs over the root, so the root
//!   does not exist for it.
//!
//! The outside view hides a *path*, not the person's session. From inside it, `systemd-run --user`,
//! a `tmux send-keys` to a tmux outside it, or the session bus all reach a process that sees the
//! root. That is the person's account being the person's account: the command that does it is on
//! a card the person read (`agent_run` asks), and a Mind's own Terminal tab is wrapped the same
//! way (#1124, H1, L3). The root exists from the first wrapped program on (`ensure_root`), so "no
//! root yet" is never "no mask"; a machine with no `bwrap` makes no workshop at all.
//!
//! The companion's own tools run in the shell's process and cannot be given a view. They refuse
//! any path for which [`Root::holds`] is true, which is a prefix test on where the path really is
//! and an inode test on every folder above it: one check, with no walk below the root and no
//! pattern to match, so a link, a `..`, a second mount of the root or a name the check has never
//! seen all land on the same answer.

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use super::resolved;

/// The root, relative to the home.
pub const ROOT_UNDER_HOME: &str = ".local/share/yantrik/workshops";

/// The root for a person whose home is `home`.
pub fn root(home: &Path) -> PathBuf {
    home.join(ROOT_UNDER_HOME)
}

/// The workshops root, as a place and as a folder (device and inode).
pub struct Root {
    path: PathBuf,
    id: Option<(u64, u64)>,
}

impl Root {
    /// The root at `path`, as it is now: where it really is, and which folder it is.
    pub fn at(path: &Path) -> Root {
        let real = resolved(path).unwrap_or_else(|| path.to_path_buf());
        let id = std::fs::metadata(&real).ok().map(|m| (m.dev(), m.ino()));
        Root { path: real, id }
    }

    /// Is `path` the root or anything under it? True when it resolves (through every link, the
    /// part not made yet included) to a place under the root, or when any folder on the way to it
    /// is the root itself, which is how a second mount of the root is caught.
    pub fn holds(&self, path: &Path) -> bool {
        self.holds_at(path, 0)
    }

    /// The folders that are the root, a Mind's folder or a workshop, by device and inode. A
    /// second view of any of them (a bind or FUSE mount) is as much the root as a second view of
    /// the root is (#1124, L2); this reads two levels of a 0700 folder and follows no link.
    fn folder_ids(&self) -> Vec<(u64, u64)> {
        let mut out: Vec<(u64, u64)> = self.id.into_iter().collect();
        let Ok(minds) = std::fs::read_dir(&self.path) else { return out };
        let dir_id = |e: &std::fs::DirEntry| e.metadata().ok().filter(|m| m.is_dir()).map(|m| (m.dev(), m.ino()));
        for mind in minds.flatten().take(256) {
            let Some(id) = dir_id(&mind) else { continue };
            out.push(id);
            if let Ok(shops) = std::fs::read_dir(mind.path()) {
                out.extend(shops.flatten().take(64).filter_map(|s| dir_id(&s)));
            }
        }
        out
    }

    fn holds_at(&self, path: &Path, depth: u8) -> bool {
        let Some(real) = resolved(path) else { return false };
        if real.starts_with(&self.path) {
            return true;
        }
        let ids = self.folder_ids();
        if !ids.is_empty() && real.ancestors().any(|a| std::fs::metadata(a).is_ok_and(|m| ids.contains(&(m.dev(), m.ino())))) {
            return true;
        }
        // A link on the way that leads nowhere yet (a write through it would make the file there)
        // is judged by where it points.
        let dangling = path.ancestors().find(|a| a.symlink_metadata().is_ok_and(|m| m.is_symlink()) && a.canonicalize().is_err());
        let Some(link) = dangling.filter(|_| depth < 8) else { return false };
        let (Ok(target), Ok(rest)) = (std::fs::read_link(link), path.strip_prefix(link)) else { return false };
        let target = link.parent().map_or(target.clone(), |p| p.join(&target));
        self.holds_at(&target.join(rest), depth + 1)
    }
}

/// Is `path` in a workshop of the person whose home is `home`?
pub fn holds(path: &Path, home: &Path) -> bool {
    Root::at(&root(home)).holds(path)
}

/// `Err(why)` for a tool to return as is when `dir` is in a workshop of this session's home (`$HOME`):
/// a program is not started with its working directory there, because a program keeps a
/// directory it starts in whatever is mounted over its path.
pub fn refuse(dir: &str) -> Result<(), String> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    match holds(Path::new(dir), &home) {
        true => Err("Error: that is in a Mind's workshop; a program is not started there. Nothing was run.".to_string()),
        false => Ok(()),
    }
}

/// The `bwrap` words that hide the root: the host as it is (devices included), a tmpfs over the
/// root, a fresh `/proc` of a pid namespace so no host process's `/proc/<pid>/root` shows it, and
/// no capability left to take the tmpfs off. Up to and including the `--`.
pub fn outside_args(root: &Path) -> Vec<String> {
    let root = root.to_string_lossy().into_owned();
    [
        "--dev-bind", "/", "/", "--tmpfs", root.as_str(), "--proc", "/proc", "--unshare-pid", "--die-with-parent",
        "--cap-drop", "ALL", "--",
    ]
    .map(String::from)
    .to_vec()
}

/// Where `bwrap` is on this machine, if it is.
pub fn find_bwrap() -> Option<&'static str> {
    ["/usr/bin/bwrap", "/bin/bwrap", "/usr/local/bin/bwrap"].into_iter().find(|p| Path::new(p).exists())
}

/// The root, made (0700) if it is not there. A mask needs something to cover: with the root made
/// before the first program a Mind drives is started, "no root yet" never means "no mask" for a
/// long-running one that starts first (#1124, L3).
pub fn ensure_root(home: &Path) -> Result<PathBuf, String> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    let at = root(home);
    if let Some(parent) = at.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{} cannot be made: {e}", parent.display()))?;
    }
    match std::fs::DirBuilder::new().mode(0o700).create(&at) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(format!("{} cannot be made: {e}", at.display())),
    }
    std::fs::set_permissions(&at, std::fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    at.canonicalize().map_err(|e| format!("{} cannot be resolved: {e}", at.display()))
}

/// `program` and `args` as a program a Mind drives should be started: in the outside view. A
/// machine with no `bwrap` can hold no workshop (`workshop_create` refuses there), so with no
/// root either the command is as it was. Where `bwrap` is, the root is made first and the mask
/// is always there. With a root and no `bwrap`, nothing starts.
pub fn outside_command(home: &Path, program: &str, args: &[String]) -> Result<(String, Vec<String>), String> {
    let bwrap = find_bwrap();
    let real = match (root(home).canonicalize(), bwrap) {
        (Ok(real), _) => real,
        (Err(_), None) => return Ok((program.to_string(), args.to_vec())),
        (Err(_), Some(_)) => ensure_root(home)?,
    };
    let bwrap = bwrap.ok_or("this machine holds Minds' workshops and has no bubblewrap (bwrap) to hide them from a program; nothing was run.")?;
    let mut argv = outside_args(&real);
    argv.push(program.to_string());
    argv.extend(args.iter().cloned());
    Ok((bwrap.to_string(), argv))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// A folder of the test's own, removed when it goes.
    struct Tmp(PathBuf);
    impl Tmp {
        fn new() -> Tmp {
            use std::sync::atomic::{AtomicU32, Ordering};
            static N: AtomicU32 = AtomicU32::new(0);
            let dir = std::env::temp_dir().join(format!("yantrik-workshops-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
            std::fs::create_dir_all(&dir).unwrap();
            Tmp(dir)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn home_with_root() -> (Tmp, PathBuf) {
        let h = Tmp::new();
        let home = h.path().canonicalize().unwrap();
        std::fs::create_dir_all(root(&home).join("hermes/shop")).unwrap();
        (h, home)
    }

    #[test]
    fn the_root_and_everything_under_it_is_held_and_nothing_else() {
        let (_h, home) = home_with_root();
        let r = root(&home);
        for p in [r.clone(), r.join("hermes"), r.join("hermes/shop"), r.join("hermes/shop/a/b/c.txt"), r.join("not/made/yet")] {
            assert!(holds(&p, &home), "{}", p.display());
        }
        for p in [home.clone(), home.join(".local/share/yantrik"), home.join("Documents/x"), PathBuf::from("/tmp/x"), PathBuf::from("relative")] {
            assert!(!holds(&p, &home), "{}", p.display());
        }
        // A sibling that merely shares the prefix is not under it.
        std::fs::create_dir_all(home.join(".local/share/yantrik/workshops2")).unwrap();
        assert!(!holds(&home.join(".local/share/yantrik/workshops2/x"), &home));
    }

    #[test]
    fn a_link_into_the_root_is_the_root() {
        let (_h, home) = home_with_root();
        let r = root(&home);
        std::fs::write(r.join("hermes/shop/secret"), "x").unwrap();
        std::os::unix::fs::symlink(r.join("hermes/shop"), home.join("l")).unwrap();
        std::os::unix::fs::symlink(&r, home.join("to-root")).unwrap();
        std::os::unix::fs::symlink("l", home.join("chain")).unwrap();
        std::os::unix::fs::symlink(r.join("hermes/shop/secret"), home.join("file-link")).unwrap();
        for p in ["l", "l/secret", "to-root/hermes", "chain/secret", "file-link", "l/not-yet"] {
            assert!(holds(&home.join(p), &home), "{p}");
        }
        // A link to a place in the root that is not made yet leads there too.
        std::os::unix::fs::symlink(r.join("hermes/later"), home.join("dangling")).unwrap();
        assert!(holds(&home.join("dangling"), &home));
    }

    #[test]
    fn dotdot_gets_no_special_case() {
        let (_h, home) = home_with_root();
        let r = root(&home);
        std::fs::create_dir(home.join("Documents")).unwrap();
        assert!(holds(&home.join("Documents/../.local/share/yantrik/workshops/hermes/shop"), &home));
        assert!(holds(&r.join("hermes/shop/../../hermes/shop/x"), &home));
        assert!(!holds(&r.join("hermes/shop/../../../../../../Documents"), &home));
        assert!(!holds(&home.join("Documents/./x/../y"), &home));
    }

    /// A second mount of the root shows its folders under another name; the path test cannot see
    /// that, the inode test does. A mount cannot be made here, so the recorded root is a name that
    /// no longer leads anywhere, and its identity is the folder that is reached by another name.
    #[test]
    fn a_folder_that_is_the_root_by_inode_is_the_root_whatever_its_name() {
        let h = Tmp::new();
        let alias = h.path().join("mnt/alias");
        std::fs::create_dir_all(alias.join("hermes/shop")).unwrap();
        let id = std::fs::metadata(&alias).ok().map(|m| (m.dev(), m.ino()));
        let recorded = Root { path: PathBuf::from("/nonexistent/workshops"), id };
        assert!(recorded.holds(&alias.join("hermes/shop/a.txt")));
        assert!(recorded.holds(&alias));
        assert!(!recorded.holds(&h.path().join("mnt")));
        assert!(!recorded.holds(&h.path().join("other/x")));
    }

    #[test]
    fn a_mind_folder_and_a_workshop_are_known_by_inode_as_well_as_the_root() {
        let (_h, home) = home_with_root();
        let r = root(&home);
        std::fs::create_dir_all(r.join("pi/other")).unwrap();
        std::fs::write(r.join("note"), "not a folder").unwrap();
        std::os::unix::fs::symlink(&home, r.join("link-out")).unwrap();
        let ids = Root::at(&r).folder_ids();
        let id = |p: PathBuf| std::fs::metadata(p).map(|m| (m.dev(), m.ino())).unwrap();
        for p in [r.clone(), r.join("hermes"), r.join("hermes/shop"), r.join("pi"), r.join("pi/other")] {
            assert!(ids.contains(&id(p.clone())), "{}", p.display());
        }
        assert!(!ids.contains(&id(home.clone())), "a link out of the root is not followed");
        assert!(!Root::at(&home.join("nowhere")).folder_ids().contains(&id(r.join("hermes/shop"))));
    }

    #[test]
    fn the_outside_view_masks_the_root_and_nothing_else() {
        let a = outside_args(Path::new("/home/p/.local/share/yantrik/workshops"));
        assert!(a.windows(2).any(|w| w == ["--tmpfs", "/home/p/.local/share/yantrik/workshops"]));
        assert!(a.windows(3).any(|w| w == ["--dev-bind", "/", "/"]), "the host as it is, devices too");
        assert!(a.iter().any(|x| x == "--unshare-pid") && a.windows(2).any(|w| w == ["--proc", "/proc"]));
        assert!(a.windows(2).any(|w| w == ["--cap-drop", "ALL"]));
        assert_eq!(a.last().unwrap(), "--");
        // It hides one place: the program keeps its network, its namespaces and its environment.
        for kept in ["--unshare-all", "--unshare-net", "--unshare-user", "--clearenv", "--ro-bind", "--new-session", "--seccomp"] {
            assert!(!a.iter().any(|x| x == kept), "{kept} is not part of the outside view");
        }
        assert_eq!(a.iter().filter(|x| x.as_str() == "--tmpfs").count(), 1);
    }

    #[test]
    fn with_no_root_a_command_is_wrapped_where_bwrap_is_and_unchanged_where_it_is_not() {
        let h = Tmp::new();
        let args = vec!["-c".to_string(), "ls".to_string()];
        let first = outside_command(h.path(), "/bin/bash", &args).unwrap();
        match find_bwrap() {
            // The mask is there from the first program on: the root is made for it to cover.
            Some(_) => {
                use std::os::unix::fs::PermissionsExt;
                assert!(first.0.ends_with("bwrap"));
                assert_eq!(std::fs::metadata(root(h.path())).unwrap().permissions().mode() & 0o777, 0o700);
            }
            None => assert_eq!(first, ("/bin/bash".to_string(), args.clone())),
        }
        std::fs::create_dir_all(root(h.path())).unwrap();
        match outside_command(h.path(), "/bin/bash", &args) {
            Ok((bwrap, argv)) => {
                assert!(bwrap.ends_with("bwrap"));
                assert_eq!(&argv[argv.len() - 3..], ["/bin/bash", "-c", "ls"]);
                let real = root(h.path()).canonicalize().unwrap().display().to_string();
                assert!(argv.windows(2).any(|w| w[0] == "--tmpfs" && w[1] == real));
            }
            // No bwrap here (the build host): the command does not start unhidden.
            Err(e) => assert!(e.contains("bwrap") && e.contains("nothing was run"), "{e}"),
        }
    }
}
