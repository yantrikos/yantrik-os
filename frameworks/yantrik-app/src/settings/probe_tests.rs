//! The security review's probes of findings F and G (#936, head 14fe65bd), kept as tests.
//!
//! Another component's file is put where the framework reads one of its own, made to look like
//! the framework's: with one name, as a won `ln; rm` race leaves it (F), or with the mark's
//! string set in one of its fields (G). Every read (the migration from the old place, the sweep
//! of stale temps, open, the watcher's look, a reload) gives up at most one probe of its bytes,
//! counted by the kernel (`/proc/thread-self/io`, `rchar`). Each probe is run still, where the
//! race is already won and the outcome is certain, and under the review's two races.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use super::file::KIND;
use super::*;
use crate::paths::{Paths, DEFAULT_PROFILE};
use crate::quiet::tests::{account_file, bytes_read_by, ACCOUNT_SIZE};
use crate::quiet::PROBE;

const APP: &str = "t";

/// Settings once kept at the old shared place, `old.json`, as the three-field shape or the
/// framework's envelope from before the mark.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
struct Kept {
    n: i32,
}
impl Settings for Kept {
    const SCHEMA: u32 = 1;
    const FORMER_FILE: Option<&'static str> = Some("old.json");
    const LEGACY_KEYS: &'static [&'static str] = &["n"];
}

/// At most `n` probes of a file, and the count's own read of `/proc/thread-self/io`.
fn probes(n: u64) -> u64 {
    n * PROBE + 512
}

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("yantrik-probe-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn fingerprint(path: &Path) -> Option<Fingerprint> {
    Fingerprint::of_settings(path, APP)
}

/// An account file whose every entry carries the mark's words in its fields, as an account
/// label a tool can set would: `kind` and `app` too, so only where they sit tells it apart.
fn forged_account_file(d: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    let account = d.join("email.json");
    let entry = format!(r#"{{"kind":"{KIND}","app":"{APP}","label":"{KIND}","password":"SENTINEL-FORGED-MARK"}},"#);
    std::fs::write(&account, format!("[{}{{}}]", entry.repeat(ACCOUNT_SIZE / entry.len()))).unwrap();
    std::fs::set_permissions(&account, std::fs::Permissions::from_mode(0o600)).unwrap();
    account
}

fn intact(account: &Path) {
    let bytes = std::fs::read(account).unwrap();
    assert!(bytes.len() >= ACCOUNT_SIZE - 128 && bytes.windows(8).any(|w| w == b"SENTINEL"), "the account file is as it was");
}

/// A thread changing what is at a path as fast as it can, until it is dropped.
struct Racer {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Racer {
    fn run(mut round: impl FnMut() + Send + 'static) -> Racer {
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let thread = std::thread::spawn(move || {
            while !stopping.load(Ordering::Relaxed) {
                round();
            }
        });
        Racer { stop, thread: Some(thread) }
    }

    /// The review's first loop: `ln account path; rm path`.
    fn link_unlink(account: &Path, path: &Path) -> Racer {
        let (account, path) = (account.to_path_buf(), path.to_path_buf());
        Racer::run(move || {
            let _ = std::fs::hard_link(&account, &path);
            let _ = std::fs::remove_file(&path);
        })
    }

    /// The review's second: `ours` and a fresh hard link to `account` renamed over `path` in turn.
    fn swap(ours: Vec<u8>, account: &Path, path: &Path) -> Racer {
        let (account, path) = (account.to_path_buf(), path.to_path_buf());
        let (file, link) = (path.with_file_name("stage-file"), path.with_file_name("stage-link"));
        Racer::run(move || {
            let _ = std::fs::write(&file, &ours);
            let _ = std::fs::rename(&file, &path);
            let _ = std::fs::remove_file(&link);
            let _ = std::fs::hard_link(&account, &link);
            let _ = std::fs::rename(&link, &path);
        })
    }
}

impl Drop for Racer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The app's paths under a fresh root, and the old shared place of its `FORMER_FILE`.
fn migration(name: &str) -> (Paths, PathBuf) {
    let root = dir(name);
    let paths = Paths::under(&root, APP, DEFAULT_PROFILE);
    let shared = root.join("config/yantrik");
    std::fs::create_dir_all(&shared).unwrap();
    (paths, shared.join("old.json"))
}

/// Nothing taken from a round may carry a byte of the account file.
fn nothing_copied(paths: &Paths) {
    if let Ok(copied) = std::fs::read(paths.config_file()) {
        assert!(!copied.windows(8).any(|w| w == b"SENTINEL"), "the account file was copied");
    }
}

/// F, still: the account file at the old place with one name, as a won race leaves it.
#[test]
fn the_migration_reads_a_probe_of_a_foreign_file_with_one_name_and_takes_nothing() {
    let (paths, former) = migration("f-still");
    let account = account_file(former.parent().unwrap());
    std::fs::rename(&account, &former).unwrap();
    let (imported, read) = bytes_read_by(|| former::import::<Kept>(&paths));
    assert_eq!(imported, former::Imported::NotOurs);
    assert!(read <= probes(1), "{read} bytes read");
    assert!(!paths.config_file().exists());
    intact(&former);
}

/// F: the migration under `ln; rm` (the review: 30 of 20,000 read the 512 KiB file).
#[test]
fn the_migration_under_link_and_unlink_never_reads_past_a_probe() {
    let (paths, former) = migration("f-ln-rm");
    let account = account_file(former.parent().unwrap());
    let racer = Racer::link_unlink(&account, &former);
    let (mut refused, mut worst) = (0u32, 0u64);
    for _ in 0..10_000 {
        let _ = std::fs::remove_file(paths.imported_record());
        let (imported, read) = bytes_read_by(|| former::import::<Kept>(&paths));
        worst = worst.max(read);
        refused += u32::from(imported == former::Imported::NotOurs);
        assert_ne!(imported, former::Imported::Copied, "the account file was taken");
    }
    drop(racer);
    assert!(refused > 0, "the race reached the read");
    assert!(worst <= probes(1), "a round read {worst} bytes of the account file");
    nothing_copied(&paths);
    intact(&account);
}

/// F: the migration under rename against a hard link (the review: 25 of 20,000).
#[test]
fn the_migration_under_rename_and_hard_link_never_reads_past_a_probe() {
    let (paths, former) = migration("f-swap");
    let account = account_file(former.parent().unwrap());
    let racer = Racer::swap(br#"{"n": 7}"#.to_vec(), &account, &former);
    let (mut copied, mut worst) = (0u32, 0u64);
    for _ in 0..5_000 {
        let _ = std::fs::remove_file(paths.imported_record());
        let _ = std::fs::remove_file(paths.config_file());
        let (imported, read) = bytes_read_by(|| former::import::<Kept>(&paths));
        worst = worst.max(read);
        copied += u32::from(imported == former::Imported::Copied);
        nothing_copied(&paths);
    }
    drop(racer);
    assert!(copied > 0, "the app's own old file is still taken between the swaps");
    assert!(worst <= probes(1), "a round read {worst} bytes of the account file");
    intact(&account);
}

/// The sweep's temp name, and the account file there with one name and a day's age.
fn stale_temp(d: &Path) -> (PathBuf, PathBuf) {
    let account = account_file(d);
    let day_old = std::time::SystemTime::now() - Duration::from_secs(86_400);
    std::fs::File::options().write(true).open(&account).unwrap().set_modified(day_old).unwrap();
    (d.join("app.json"), d.join(".app.json.11-1.tmp"))
}

/// F, still: an old temp beside the settings file that is the account file, with one name.
#[test]
fn the_sweep_reads_a_probe_of_a_foreign_temp_with_one_name_and_keeps_it() {
    let d = dir("f-sweep-still");
    let (path, temp) = stale_temp(&d);
    std::fs::rename(d.join("email.json"), &temp).unwrap();
    let ((), read) = bytes_read_by(|| file::sweep_own_temps(&path, APP, Duration::ZERO));
    assert!(read <= probes(1), "{read} bytes read");
    intact(&temp);
}

/// F: the sweep at every open, under `ln; rm` (the review: 11 of 20,000).
#[test]
fn the_sweep_under_link_and_unlink_never_reads_past_a_probe() {
    let d = dir("f-sweep-ln-rm");
    let (path, temp) = stale_temp(&d);
    let account = d.join("email.json");
    let racer = Racer::link_unlink(&account, &temp);
    let mut worst = 0u64;
    for _ in 0..20_000 {
        let ((), read) = bytes_read_by(|| file::sweep_own_temps(&path, APP, Duration::ZERO));
        worst = worst.max(read);
    }
    drop(racer);
    assert!(worst <= probes(1), "a round read {worst} bytes of the account file");
    intact(&account);
}

/// What each of the store's reads gave up of the file at `path`, at most, over `rounds`.
#[derive(Default, Debug)]
struct Worst {
    open: u64,
    changed_outside: u64,
    reload: u64,
    read_file: u64,
    of_settings: u64,
}

impl Worst {
    fn round(&mut self, path: &Path, store: &mut Store<Kept>) {
        let ours = |k: &Kept| k == &Kept::default() || k == &Kept { n: 7 };
        let (opened, read) = bytes_read_by(|| Store::<Kept>::open(path.to_path_buf(), APP));
        assert!(ours(opened.get()), "the account file was taken as settings: {:?}", opened.get());
        self.open = self.open.max(read);
        let (_, read) = bytes_read_by(|| store.changed_outside());
        self.changed_outside = self.changed_outside.max(read);
        let (reloaded, read) = bytes_read_by(|| store.reload());
        assert!(!matches!(reloaded, Reloaded::Adopted(ref k) if !ours(k)), "{reloaded:?}");
        self.reload = self.reload.max(read);
        let (opened, read) = bytes_read_by(|| read_file::<Kept>(path, APP));
        assert!(!matches!(opened, Opened::Read { ref value, .. } if !ours(value)), "the account file was read as settings");
        self.read_file = self.read_file.max(read);
        let (_, read) = bytes_read_by(|| fingerprint(path));
        self.of_settings = self.of_settings.max(read);
    }

    /// Open reads the file twice (the read and the fingerprint), as a reload that sees it changed.
    fn within_a_probe(&self) {
        assert!(self.open <= probes(2), "open read {} bytes: {self:?}", self.open);
        assert!(self.reload <= probes(2), "reload read {} bytes: {self:?}", self.reload);
        for (what, read) in [("changed_outside", self.changed_outside), ("read_file", self.read_file), ("of_settings", self.of_settings)] {
            assert!(read <= probes(1), "{what} read {read} bytes: {self:?}");
        }
    }
}

/// G, still: the forged account file at the settings path, with one name.
#[test]
fn a_foreign_file_with_the_mark_in_a_field_gives_up_only_a_probe() {
    let d = dir("g-still");
    let path = d.join("app.json");
    std::fs::rename(forged_account_file(&d), &path).unwrap();
    let mut store = Store::<Kept>::open(path.clone(), APP);
    assert_eq!(store.health(), &Health::Recovery(SettingsError::NotOurs));
    let mut worst = Worst::default();
    worst.round(&path, &mut store);
    worst.within_a_probe();
    assert!(matches!(read_file::<Kept>(&path, APP), Opened::Recovery(SettingsError::NotOurs)));
    intact(&path);
}

/// G under `ln; rm` (the review: up to 69 of 20,000 per read).
#[test]
fn a_forged_mark_under_link_and_unlink_never_gets_past_a_probe() {
    let d = dir("g-ln-rm");
    let path = d.join("app.json");
    let account = forged_account_file(&d);
    let mut store = Store::<Kept>::open(path.clone(), APP);
    let racer = Racer::link_unlink(&account, &path);
    let mut worst = Worst::default();
    for _ in 0..10_000 {
        worst.round(&path, &mut store);
    }
    drop(racer);
    worst.within_a_probe();
    intact(&account);
}

/// G under rename against a hard link (the review: up to 314 of 20,000 per read).
#[test]
fn a_forged_mark_under_rename_and_hard_link_never_gets_past_a_probe() {
    let d = dir("g-swap");
    let path = d.join("app.json");
    let account = forged_account_file(&d);
    let ours = render(APP, &Kept { n: 7 }).unwrap();
    let mut store = Store::<Kept>::open(path.clone(), APP);
    let racer = Racer::swap(ours, &account, &path);
    let mut worst = Worst::default();
    for _ in 0..10_000 {
        worst.round(&path, &mut store);
    }
    drop(racer);
    worst.within_a_probe();
    intact(&account);
}
