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

/// A home whose root is made the way the installer makes it: 0700, the person's.
fn home_with_root() -> (Tmp, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    roots_under_home_for_tests();
    let h = Tmp::new();
    let home = h.path().canonicalize().unwrap();
    std::fs::create_dir_all(root(&home).join("hermes/shop")).unwrap();
    std::fs::set_permissions(root(&home), std::fs::Permissions::from_mode(0o700)).unwrap();
    (h, home)
}

#[test]
fn the_root_and_everything_under_it_is_held_and_nothing_else() {
    let (_h, home) = home_with_root();
    let r = root(&home);
    for p in [r.clone(), r.join("hermes"), r.join("hermes/shop"), r.join("hermes/shop/a/b/c.txt"), r.join("not/made/yet")] {
        assert!(holds(&p, &home), "{}", p.display());
    }
    // Another account's folder is in the base, so it is held too.
    assert!(holds(&base(&home).join("4242/pi/shop/x"), &home));
    for p in [home.clone(), home.join(".local/share/yantrik"), home.join("Documents/x"), PathBuf::from("/tmp/x"), PathBuf::from("relative")] {
        assert!(!holds(&p, &home), "{}", p.display());
    }
    // A sibling that merely shares the prefix is not under it.
    std::fs::create_dir_all(home.join(".test-workshops2")).unwrap();
    assert!(!holds(&home.join(".test-workshops2/x"), &home));
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
    let via_docs = home.join("Documents/..").join(r.strip_prefix(&home).unwrap()).join("hermes/shop");
    assert!(holds(&via_docs, &home));
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
fn an_account_a_mind_and_a_workshop_are_known_by_inode_as_well_as_the_base() {
    let (_h, home) = home_with_root();
    let r = root(&home);
    std::fs::create_dir_all(r.join("pi/other")).unwrap();
    std::fs::write(r.join("note"), "not a folder").unwrap();
    std::os::unix::fs::symlink(&home, r.join("link-out")).unwrap();
    let ids = Root::at(&base(&home)).folder_ids();
    let id = |p: PathBuf| std::fs::metadata(p).map(|m| (m.dev(), m.ino())).unwrap();
    for p in [base(&home), r.clone(), r.join("hermes"), r.join("hermes/shop"), r.join("pi"), r.join("pi/other")] {
        assert!(ids.contains(&id(p.clone())), "{}", p.display());
    }
    assert!(!ids.contains(&id(home.clone())), "a link out of the root is not followed");
    assert!(!Root::at(&home.join("nowhere")).folder_ids().contains(&id(r.join("hermes/shop"))));
}

/// Decision, 8 Nov: the whole of the base and the session's runtime folder are masked, with
/// nothing bound back (no bus, no `systemd/private`, no Yantrik socket, no PipeWire, no token).
#[test]
fn the_outside_argv_masks_both_places_and_binds_nothing_back() {
    let hidden = [PathBuf::from("/var/lib/yantrik/workshops"), PathBuf::from("/run/user/1000")];
    let a = outside_args(&hidden);
    for h in &hidden {
        assert!(a.windows(2).any(|w| w[0] == "--tmpfs" && Path::new(&w[1]) == h), "{}", h.display());
    }
    assert!(a.windows(3).any(|w| w == ["--dev-bind", "/", "/"]), "the host as it is, devices too");
    assert!(a.iter().any(|x| x == "--unshare-pid") && a.windows(2).any(|w| w == ["--proc", "/proc"]));
    assert!(a.windows(2).any(|w| w == ["--cap-drop", "ALL"]));
    assert_eq!(a.last().unwrap(), "--");
    assert_eq!(a.iter().filter(|x| x.as_str() == "--tmpfs").count(), 2);
    // Nothing is bound back over a mask: the only bind is the host itself.
    for bound in ["--bind", "--ro-bind", "--bind-try", "--ro-bind-try", "--setenv", "--symlink", "--file"] {
        assert!(!a.iter().any(|x| x == bound), "{bound} would put something back");
    }
    // It hides those places: the program keeps its network, its namespaces and its environment.
    for kept in ["--unshare-all", "--unshare-net", "--unshare-user", "--clearenv", "--new-session", "--seccomp"] {
        assert!(!a.iter().any(|x| x == kept), "{kept} is not part of the outside view");
    }
}

#[test]
fn a_command_is_wrapped_over_the_base_and_the_runtime_folder_where_bwrap_is() {
    let (_h, home) = home_with_root();
    let args = vec!["-c".to_string(), "ls".to_string()];
    match outside_command(&home, "/bin/bash", &args) {
        Ok((bwrap, argv)) => {
            assert!(bwrap.ends_with("bwrap"));
            assert_eq!(&argv[argv.len() - 3..], ["/bin/bash", "-c", "ls"]);
            let masked = |p: &Path| argv.windows(2).any(|w| w[0] == "--tmpfs" && Path::new(&w[1]) == p);
            assert!(masked(&base(&home)), "the whole base, not one account's folder");
            assert!(!masked(&root(&home)));
            assert_eq!(masked(&run_user(euid())), run_user(euid()).is_dir());
        }
        // No bwrap here (the build host): a root with things in it starts nothing unhidden.
        Err(e) => assert!(e.contains("bwrap") && e.contains("nothing was run"), "{e}"),
    }
    // Nothing held and no bwrap: the command is as it was.
    let bare = Tmp::new();
    if find_bwrap().is_none() {
        assert_eq!(outside_command(bare.path(), "/bin/bash", &args).unwrap(), ("/bin/bash".to_string(), args));
    }
}

/// A missing root, a root that is not the person's, and a root with open permissions all refuse,
/// and nothing makes them.
#[test]
fn a_missing_or_misowned_root_refuses_and_is_never_made() {
    use std::os::unix::fs::PermissionsExt;
    roots_under_home_for_tests();
    let h = Tmp::new();
    let home = h.path().canonicalize().unwrap();
    let why = open_root(&home).err().expect("no root yet");
    assert!(why.contains("missing") && why.contains("never does"), "{why}");
    assert!(!root(&home).exists() && !base(&home).exists(), "the shell made nothing");
    std::fs::create_dir_all(root(&home)).unwrap();
    for mode in [0o755, 0o750, 0o701, 0o770] {
        std::fs::set_permissions(root(&home), std::fs::Permissions::from_mode(mode)).unwrap();
        assert!(open_root(&home).err().expect("loose").contains("not the person's alone"), "{mode:o}");
    }
    std::fs::set_permissions(root(&home), std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(open_root(&home).is_ok());
    // A link in the root's place is not followed.
    let h2 = Tmp::new();
    let home2 = h2.path().canonicalize().unwrap();
    std::fs::create_dir_all(base(&home2)).unwrap();
    let real = home2.join("real");
    std::fs::create_dir(&real).unwrap();
    std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::os::unix::fs::symlink(&real, root(&home2)).unwrap();
    assert!(open_root(&home2).is_err(), "a link is not the root");
}

/// Finding 1a: the folder is pinned by (device, inode). Moving it, or putting another where it
/// was, is refused, and what is read through the old name is the folder that was pinned.
#[test]
fn the_root_is_pinned_by_device_and_inode_and_a_rename_of_any_ancestor_exposes_nothing() {
    use std::os::unix::fs::PermissionsExt;
    let (_h, home) = home_with_root();
    let held = open_root(&home).unwrap();
    let (dev, ino) = {
        let m = std::fs::metadata(root(&home)).unwrap();
        (m.dev(), m.ino())
    };
    let seen = std::fs::metadata(held.via(&root(&home))).unwrap();
    assert_eq!((seen.dev(), seen.ino()), (dev, ino), "the descriptor is the folder");
    assert!(open_root(&home).is_ok(), "the same folder again");
    // Rename an ancestor (the base): the old name no longer leads to the pinned folder.
    let moved = home.join("y2");
    std::fs::rename(base(&home), &moved).unwrap();
    let why = open_root(&home).err().expect("moved");
    assert!(why.contains("missing"), "{why}");
    // A fresh root made in its place (by anyone) is not the pinned one.
    std::fs::create_dir_all(root(&home)).unwrap();
    std::fs::set_permissions(root(&home), std::fs::Permissions::from_mode(0o700)).unwrap();
    let why = open_root(&home).err().expect("replaced");
    assert!(why.contains("not the folder it was"), "{why}");
    // The held descriptor still names the folder that was pinned: it is where it moved to.
    let still = std::fs::metadata(held.via(&root(&home))).unwrap();
    assert_eq!((still.dev(), still.ino()), (dev, ino));
    // Put back, it is the root again.
    std::fs::remove_dir(root(&home)).unwrap();
    std::fs::rename(&moved, base(&home)).unwrap();
    assert!(open_root(&home).is_ok());
}

/// Round 6, finding 3: a folder above the workshops that group or others can write to, or that is
/// not root's, lets whoever owns it rename what is below it.
#[test]
fn a_folder_above_the_workshops_that_is_not_roots_is_named() {
    use std::os::unix::fs::PermissionsExt;
    let h = Tmp::new();
    let top = h.path().canonicalize().unwrap();
    std::fs::create_dir_all(top.join("a/b")).unwrap();
    for mode in [0o777, 0o775, 0o757, 0o755] {
        std::fs::set_permissions(top.join("a"), std::fs::Permissions::from_mode(mode)).unwrap();
        assert!(writable_ancestor(&top.join("a/b/c")).is_some(), "{mode:o}: the test's folders are not root's");
    }
    assert!(writable_ancestor(Path::new("/nonexistent-yantrik/x")).is_some(), "an ancestor that cannot be read is not safe");
}

/// Decision, 9 Nov: the person's Browser and Mind View's browser start in the outside view with
/// the base hidden and nothing else, so a browser's Wayland, PipeWire and bus sockets stay.
#[test]
fn a_browser_is_wrapped_over_the_base_only() {
    let (_h, home) = home_with_root();
    let args = vec!["--no-first-run".to_string()];
    match browser_command(&home, "/usr/bin/chromium", &args) {
        Ok((bwrap, argv)) => {
            assert!(bwrap.ends_with("bwrap"));
            assert_eq!(&argv[argv.len() - 2..], ["/usr/bin/chromium", "--no-first-run"]);
            let tmpfs: Vec<&String> = argv.windows(2).filter(|w| w[0] == "--tmpfs").map(|w| &w[1]).collect();
            assert_eq!(tmpfs, [&base(&home).to_string_lossy().into_owned()], "the base and nothing else");
            for kept in [["--unshare-pid"].as_slice(), &["--proc", "/proc"], &["--cap-drop", "ALL"]] {
                assert!(argv.windows(kept.len()).any(|w| w == kept), "{kept:?} in {argv:?}");
            }
        }
        Err(e) => assert!(e.contains("bwrap") && e.contains("nothing was run"), "{e}"),
    }
    // The job's view hides the runtime folder too; the browser's does not.
    if let (Ok((_, job)), Ok((_, browser))) = (outside_command(&home, "/bin/true", &[]), browser_command(&home, "/bin/true", &[])) {
        let run = run_user(euid()).to_string_lossy().into_owned();
        assert_eq!(job.contains(&run), run_user(euid()).is_dir());
        assert!(!browser.contains(&run));
    }
}
