use super::check::{self, ensure_private_as, prepare};
use super::*;
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};

fn mode(p: &Path) -> u32 {
    std::fs::symlink_metadata(p).unwrap().mode() & 0o777
}

fn canon(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap()
}

#[test]
fn makes_the_runtime_dir_private_and_reuses_it() {
    let root = tempfile::tempdir().unwrap();
    let dir = scratch_dir_from(Some(root.path().into()), None).unwrap();
    assert_eq!(dir, canon(root.path()).join(SCRATCH_NAME));
    assert_eq!(mode(&dir), 0o700);
    assert_eq!(std::fs::metadata(&dir).unwrap().uid(), current_uid());
    // A second call finds it and hands back the same place.
    assert_eq!(scratch_dir_from(Some(root.path().into()), None).unwrap(), dir);
}

#[test]
fn scratch_is_never_the_socket_dir() {
    // `$XDG_RUNTIME_DIR/yantrik` holds the service sockets and the apps' pid files; the file tools
    // may write into scratch, so the two must never be one directory.
    let root = tempfile::tempdir().unwrap();
    let dir = scratch_dir_from(Some(root.path().into()), None).unwrap();
    assert_eq!(dir.file_name().unwrap(), "yantrik-scratch");
    assert_ne!(dir, canon(root.path()).join("yantrik"));
    assert!(!root.path().join("yantrik").exists());
}

#[test]
fn tightens_a_loose_directory_of_our_own() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join(SCRATCH_NAME);
    std::fs::create_dir(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    scratch_dir_from(Some(root.path().into()), None).unwrap();
    assert_eq!(mode(&dir), 0o700);
}

#[test]
fn refuses_a_symlinked_leaf() {
    let root = tempfile::tempdir().unwrap();
    let elsewhere = root.path().join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    let dir = root.path().join(SCRATCH_NAME);
    std::os::unix::fs::symlink(&elsewhere, &dir).unwrap();
    let err = ensure_private_as(&dir, current_uid()).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::PermissionDenied, "{err}");
    // And the resolver goes past it to home rather than using it.
    let home = root.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let got = scratch_dir_from(Some(root.path().into()), Some(home.clone())).unwrap();
    assert_eq!(got, canon(&home).join(".cache/yantrik/tmp"));
}

#[test]
fn follows_a_symlinked_intermediate_whose_target_is_trusted() {
    // ~/.cache on another disk is an ordinary machine, not an attack.
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let disk = root.path().join("disk-cache");
    std::fs::create_dir(&home).unwrap();
    std::fs::create_dir(&disk).unwrap();
    std::os::unix::fs::symlink(&disk, home.join(".cache")).unwrap();
    let got = scratch_dir_from(None, Some(home)).unwrap();
    assert_eq!(got, canon(&disk).join("yantrik/tmp"));
    assert_eq!(mode(&got), 0o700);
}

#[test]
fn refuses_a_symlinked_intermediate_that_lands_somewhere_loose() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let loose = root.path().join("loose");
    std::fs::create_dir(&home).unwrap();
    std::fs::create_dir(&loose).unwrap();
    std::fs::set_permissions(&loose, std::fs::Permissions::from_mode(0o777)).unwrap();
    std::os::unix::fs::symlink(&loose, home.join(".cache")).unwrap();
    assert!(scratch_dir_from(None, Some(home)).is_err());
    // Judged before anything was made inside it.
    assert!(!loose.join("yantrik").exists());
}

#[test]
fn refuses_a_directory_someone_else_owns() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join(SCRATCH_NAME);
    std::fs::create_dir(&dir).unwrap();
    // Not root in a test run, so the stranger is simulated by asking on behalf of another uid.
    let err = ensure_private_as(&dir, current_uid().wrapping_add(1)).unwrap_err();
    assert!(err.to_string().contains("owned by uid"), "{err}");
}

#[test]
fn falls_back_to_home_when_the_runtime_dir_is_missing() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let got = scratch_dir_from(Some(root.path().join("no-such-run")), Some(home.clone())).unwrap();
    assert_eq!(got, canon(&home).join(".cache/yantrik/tmp"));
    assert_eq!(mode(&got), 0o700);
    let got = scratch_dir_from(None, Some(home.clone())).unwrap();
    assert_eq!(got, canon(&home).join(".cache/yantrik/tmp"));
}

#[test]
fn never_falls_back_to_tmp() {
    let err = scratch_dir_from(None, None).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::NotFound);
    assert!(state_dir_from(None, None, "quarantine").is_err());
    // A relative XDG value is ignored rather than resolved against the working directory.
    assert_eq!(absolute(Some("relative/run".into())), None);
}

#[test]
fn refuses_a_base_every_account_can_write() {
    let root = tempfile::tempdir().unwrap();
    let run = root.path().join("run");
    std::fs::create_dir(&run).unwrap();
    std::fs::set_permissions(&run, std::fs::Permissions::from_mode(0o777)).unwrap();
    let home = root.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let got = scratch_dir_from(Some(run.clone()), Some(home.clone())).unwrap();
    assert_eq!(got, canon(&home).join(".cache/yantrik/tmp"));
    // Judged before anything was made in it.
    assert!(!run.join(SCRATCH_NAME).exists());
    assert!(scratch_dir_from(Some(run), None).is_err());
}

#[test]
fn a_group_writable_home_cache_follows_whether_our_group_is_private() {
    // A umask-002 system: ~/.cache is 0775 in our primary group. That is ours alone only under
    // the user-private-group convention (group named after us); a shared group like `users` is
    // not. Which one this test machine is decides the expected answer.
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    std::fs::create_dir_all(home.join(".cache")).unwrap();
    std::fs::set_permissions(home.join(".cache"), std::fs::Permissions::from_mode(0o775)).unwrap();
    let got = scratch_dir_from(None, Some(home.clone()));
    if super::upg::private_group(current_uid()).is_some() {
        assert_eq!(got.unwrap(), canon(&home).join(".cache/yantrik/tmp"));
    } else {
        assert!(got.is_err());
    }
}

#[test]
fn group_writable_is_accepted_only_in_our_private_group() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("shared");
    std::fs::create_dir(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o770)).unwrap();
    let meta = std::fs::metadata(&dir).unwrap();
    // Not root in a test run, so the groups are simulated by what we pass as our private one.
    assert!(check::trusted_as(&dir, &meta, current_uid(), Some(meta.gid())).is_ok());
    for private in [None, Some(meta.gid().wrapping_add(1))] {
        let err = check::trusted_as(&dir, &meta, current_uid(), private).unwrap_err();
        assert!(err.to_string().contains("not a group of ours alone"), "{err}");
    }
    // Not group-writable: the group does not matter.
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o750)).unwrap();
    let meta = std::fs::metadata(&dir).unwrap();
    assert!(check::trusted_as(&dir, &meta, current_uid(), None).is_ok());
}

#[test]
fn refuses_a_loose_directory_on_the_way_down() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    std::fs::create_dir_all(home.join(".cache")).unwrap();
    std::fs::set_permissions(home.join(".cache"), std::fs::Permissions::from_mode(0o777)).unwrap();
    let err = scratch_dir_from(None, Some(home)).unwrap_err();
    assert!(err.to_string().contains("writable by every account"), "{err}");
}

#[test]
fn a_home_or_runtime_dir_that_is_tmp_is_refused() {
    // Only meaningful where /tmp is the shared, world-writable directory it usually is.
    let tmp = Path::new("/tmp");
    if std::fs::metadata(tmp).map(|m| m.mode() & 0o002 == 0).unwrap_or(true) {
        return;
    }
    assert!(scratch_dir_from(None, Some(tmp.into())).is_err());
    assert!(scratch_dir_from(Some(tmp.into()), None).is_err());
    assert!(state_dir_from(Some(tmp.into()), None, "quarantine").is_err());
}

#[test]
fn state_dir_is_private_and_named() {
    let root = tempfile::tempdir().unwrap();
    let got = state_dir_from(Some(root.path().into()), None, "quarantine").unwrap();
    assert_eq!(got, canon(root.path()).join("yantrik/quarantine"));
    assert_eq!(mode(&got), 0o700);
    let home = root.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let got = state_dir_from(None, Some(home.clone()), "quarantine").unwrap();
    assert_eq!(got, canon(&home).join(".local/state/yantrik/quarantine"));
}

#[test]
fn prepare_takes_only_plain_relative_paths() {
    let root = tempfile::tempdir().unwrap();
    assert!(prepare(root.path(), Path::new("../escape")).is_err());
    assert!(prepare(root.path(), Path::new("/etc")).is_err());
    assert!(prepare(root.path(), Path::new("")).is_err());
}

#[test]
fn file_names_cannot_climb_out() {
    for bad in ["", ".", "..", "../x", "a/b"] {
        assert!(plain_name(bad).is_err(), "{bad:?}");
    }
    assert_eq!(plain_name("yantrik-see-payload.json").unwrap(), "yantrik-see-payload.json");
}

// ── Creating files ───────────────────────────────────────────────────────────────────────────

#[test]
fn creates_a_private_file_and_empties_it_on_reuse() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("payload.json");
    create_private_file(&path).unwrap().write_all(b"first, and longer").unwrap();
    assert_eq!(mode(&path), 0o600);
    create_private_file(&path).unwrap().write_all(b"second").unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
}

#[test]
fn will_not_write_through_a_link_at_the_name() {
    let root = tempfile::tempdir().unwrap();
    let precious = root.path().join("authorized_keys");
    std::fs::write(&precious, "ssh-ed25519 AAAA").unwrap();
    let planted = root.path().join("notes.pid");
    std::os::unix::fs::symlink(&precious, &planted).unwrap();
    assert!(create_private_file(&planted).is_err());
    assert_eq!(std::fs::read_to_string(&precious).unwrap(), "ssh-ed25519 AAAA");
}

#[test]
fn will_not_empty_a_file_that_has_another_name() {
    let root = tempfile::tempdir().unwrap();
    let precious = root.path().join("authorized_keys");
    std::fs::write(&precious, "ssh-ed25519 AAAA").unwrap();
    let planted = root.path().join("task.out");
    std::fs::hard_link(&precious, &planted).unwrap();
    assert!(create_private_file(&planted).is_err());
    assert_eq!(std::fs::read_to_string(&precious).unwrap(), "ssh-ed25519 AAAA");
}

// ── Reading files back ───────────────────────────────────────────────────────────────────────

#[test]
fn reads_a_file_of_ours() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("yantrik-scrollback.txt");
    create_private_file(&path).unwrap().write_all(b"error: it broke").unwrap();
    let mut text = String::new();
    open_private_file(&path).unwrap().read_to_string(&mut text).unwrap();
    assert_eq!(text, "error: it broke");
}

#[test]
fn will_not_read_through_a_link_at_the_name() {
    // The attack: an archive extracted into scratch leaves the terminal's dump name pointing at a
    // key, and "read the terminal" hands the key to the model.
    let root = tempfile::tempdir().unwrap();
    let key = root.path().join("id_ed25519");
    std::fs::write(&key, "-----BEGIN OPENSSH PRIVATE KEY-----").unwrap();
    let planted = root.path().join("yantrik-scrollback.txt");
    std::os::unix::fs::symlink(&key, &planted).unwrap();
    assert!(open_private_file(&planted).is_err());
}

#[test]
fn will_not_read_a_fifo_or_a_directory() {
    let root = tempfile::tempdir().unwrap();
    let fifo = root.path().join("task.out");
    let c = std::ffi::CString::new(std::os::unix::ffi::OsStrExt::as_bytes(fifo.as_os_str())).unwrap();
    // SAFETY: a valid NUL-terminated path; mkfifo touches nothing else.
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    assert!(open_private_file(&fifo).is_err());
    assert!(open_private_file(root.path()).is_err());
}

#[test]
fn will_not_read_a_file_that_has_another_name() {
    // A hard link to the key is the key: reads refuse what writes refuse.
    let root = tempfile::tempdir().unwrap();
    let key = root.path().join("id_ed25519");
    std::fs::write(&key, "-----BEGIN OPENSSH PRIVATE KEY-----").unwrap();
    let planted = root.path().join("yantrik-task-t0001.out");
    std::fs::hard_link(&key, &planted).unwrap();
    assert!(open_private_file(&planted).is_err());
}

// ── Work directories ─────────────────────────────────────────────────────────────────────────

#[test]
fn the_work_dir_is_its_own_directory_not_scratch() {
    let root = tempfile::tempdir().unwrap();
    let work = work_dir_from(Some(root.path().into()), None).unwrap();
    assert_eq!(work, canon(root.path()).join(WORK_NAME));
    assert_ne!(work, scratch_dir_from(Some(root.path().into()), None).unwrap());
    assert_eq!(mode(&work), 0o700);
    let home = root.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let work = work_dir_from(None, Some(home.clone())).unwrap();
    assert_eq!(work, canon(&home).join(WORK_HOME_REL));
    assert_ne!(work, scratch_dir_from(None, Some(home)).unwrap());
}

#[test]
fn a_fresh_dir_is_new_private_and_gone_when_dropped() {
    let a = fresh_work_dir("test-fresh").unwrap();
    let b = fresh_work_dir("test-fresh").unwrap();
    assert_ne!(a.path(), b.path());
    assert_eq!(a.path().parent().unwrap(), work_dir().unwrap(), "made in the work dir, not scratch");
    assert_eq!(mode(a.path()), 0o700);
    assert!(std::fs::read_dir(a.path()).unwrap().next().is_none(), "nothing can be waiting inside");
    std::fs::write(a.file("out.txt").unwrap(), "x").unwrap();
    assert!(a.file("../escape").is_err());
    let path = a.path().to_path_buf();
    drop(a);
    assert!(!path.exists());
}

#[test]
fn a_fifo_at_the_name_is_an_error_not_a_hang() {
    let root = tempfile::tempdir().unwrap();
    let fifo = root.path().join("payload.json");
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
    // SAFETY: a valid NUL-terminated path; mkfifo touches nothing else.
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    assert!(create_private_file(&fifo).is_err());
}
