//! The headless browser as a process: how it is started, where its profile lives, and how it is
//! made to die with the app.
//!
//! A verify or a screenshot starts a Chromium with `--remote-debugging-port`, which any local
//! process can drive. Two rules keep one from being left behind or reached: the browser dies with
//! the app (`PR_SET_PDEATHSIG`, and `kill_live` when the app ends on purpose), and each job has a
//! profile of its own, made private and new, never a path another job or another user could
//! predict or share.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// A browser started and not yet stopped, and the two folders that go with it.
struct Live {
    cancel: Arc<AtomicBool>,
    pid: u32,
    profile: PathBuf,
    tmp: PathBuf,
}

/// The browsers started and not yet stopped.
static LIVE: Mutex<Vec<Live>> = Mutex::new(Vec::new());

/// Has the child receive SIGKILL when the thread that started it ends, which is the app's own end
/// whatever way it comes (a logout, a crash, a kill). The thread is a long-lived pool worker, so
/// the signal does not come early: the kernel ties it to the thread, not to the process.
pub fn die_with_parent(command: &mut Command) {
    #[cfg(unix)]
    {
        // A group of its own, so one signal reaches the browser and every helper it starts.
        std::os::unix::process::CommandExt::process_group(command, 0);
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt as _;
        // The parent as it is now. If it dies between the fork and the prctl below, the signal is
        // never armed; the child sees that its parent is no longer this one and ends itself.
        let parent = std::process::id() as libc::pid_t;
        // SAFETY: the closure runs between fork and exec and makes only async-signal-safe calls
        // (prctl, getppid, _exit) and reads errno.
        unsafe {
            command.pre_exec(move || {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL as libc::c_ulong, 0, 0, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::getppid() != parent {
                    libc::_exit(1);
                }
                Ok(())
            });
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = command;
}

/// Note a started browser, so the app ending on purpose can stop it and clear its profile.
pub fn register(pid: u32, profile: &Path, tmp: &Path, cancel: Arc<AtomicBool>) {
    LIVE.lock().unwrap_or_else(|e| e.into_inner()).push(Live { cancel, pid, profile: profile.to_path_buf(), tmp: tmp.to_path_buf() });
}

thread_local! {
    /// The cancellation flag of the browser job this thread is running, if it is running one.
    static CURRENT: RefCell<Option<Arc<AtomicBool>>> = const { RefCell::new(None) };
}

/// The flag of the job this thread is running, for the code that starts its browser.
pub fn current() -> Option<Arc<AtomicBool>> {
    CURRENT.with(|c| c.borrow().clone())
}

/// A job's cancellation flag as this thread's own, for as long as the scope lives. The job holds
/// its own `Arc` of the flag as well, so nothing that clears the thread's can clear the job's.
pub struct JobScope;

impl JobScope {
    pub fn enter(flag: Arc<AtomicBool>) -> JobScope {
        set_current(Some(flag));
        JobScope
    }
}

impl Drop for JobScope {
    fn drop(&mut self) {
        set_current(None);
    }
}

/// Makes `flag` the one this thread's waits look at.
pub fn set_current(flag: Option<Arc<AtomicBool>>) {
    CURRENT.with(|c| *c.borrow_mut() = flag);
}

/// Whether the browser job this thread runs has been stopped because the app is ending. Every
/// wait in a job looks at this, so a job returns within a poll of its browser being killed
/// instead of waiting out a timeout the framework would then wait out too.
pub fn cancelled() -> bool {
    CURRENT.with(|c| c.borrow().as_ref().is_some_and(|f| f.load(Ordering::SeqCst)))
}

/// The sentence a stopped job ends with.
pub const STOPPED: &str = "the browser job was stopped because Arcade is closing";

/// Sleeps for `total`, in short slices, and returns early when the job is cancelled.
pub fn nap(total: std::time::Duration) {
    let end = std::time::Instant::now() + total;
    while !cancelled() {
        let left = end.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return;
        }
        std::thread::sleep(left.min(std::time::Duration::from_millis(20)));
    }
}

/// Note that a browser is being stopped and reaped. Before the reap, never after: a reaped pid can
/// be reused, and `kill_live` must not signal it.
pub fn forget(pid: u32) {
    LIVE.lock().unwrap_or_else(|e| e.into_inner()).retain(|l| l.pid != pid);
}

/// Stops the browser started as `pid` and everything it started (its group), at once.
fn kill_group(pid: u32) {
    #[cfg(unix)]
    // SAFETY: kill(2) of the process group of a browser this app started; its group id is its pid.
    unsafe {
        libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
        // And the process itself, for one that is not a group leader.
        libc::kill(pid as libc::pid_t, libc::SIGKILL);
    }
}

/// Stops every browser still running and clears the folders of each: the app is ending. Returns
/// how many were signalled. A profile left for the next start's sweep is only for a kill -9.
///
/// The signals go out while the list is locked: a job's `Drop` calls `forget` before it reaps, so
/// it waits here until they are sent, and a reaped pid (which can be reused) is never signalled.
/// The folders are removed after the lock is let go.
pub fn kill_live() -> usize {
    let live: Vec<Live> = {
        let mut list = LIVE.lock().unwrap_or_else(|e| e.into_inner());
        for l in list.iter() {
            l.cancel.store(true, Ordering::SeqCst);
            kill_group(l.pid);
        }
        std::mem::take(&mut *list)
    };
    for l in &live {
        remove_profile(&l.profile);
        remove_profile(&l.tmp);
    }
    live.len()
}

/// Removes a job's profile. The browser's helper processes may take a moment to stop writing into
/// it after the group is signalled, so a folder that is not empty yet is tried again.
pub fn remove_profile(profile: &Path) {
    for attempt in 0..3 {
        match std::fs::remove_dir_all(profile) {
            Ok(()) => return,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
            Err(_) if attempt < 2 => std::thread::sleep(std::time::Duration::from_millis(100)),
            Err(e) => tracing::warn!(profile = %profile.display(), error = %e, "could not remove a browser profile"),
        }
    }
}

/// A headless browser, started for one job. Dropping it stops the browser and its helpers, waits
/// for them, and removes the profile, in that order.
pub struct Headless {
    child: std::process::Child,
    pub ws_url: String,
    pub browser: String,
    pub(crate) profile: PathBuf,
    pub(crate) tmp: PathBuf,
}

impl Headless {
    /// Starts `binary` on `url` with `profile` as its profile, in a group of its own, to die with
    /// the app, with its temporary files in a short private folder of its own: Chromium makes
    /// `$TMPDIR/org.chromium.Chromium.*` per run, with a socket in it whose path must fit 107
    /// bytes, so that folder cannot be inside a profile that sits under a long state directory.
    pub fn start(binary: &Path, url: &str, profile: &Path) -> Result<Headless, String> {
        let tmp = new_tmp(std::path::Path::new("/tmp"))?;
        if let Err(sentence) = check_socket_len(&tmp) {
            remove_profile(&tmp);
            remove_profile(profile);
            return Err(sentence);
        }
        let mut command = Command::new(binary);
        command
            // Software rendering: WSL here has no GPU path for headless Chromium.
            .args([
                "--headless=new",
                "--use-gl=angle",
                "--use-angle=swiftshader",
                "--enable-unsafe-swiftshader",
                "--no-sandbox",
                "--disable-dev-shm-usage",
                "--disable-crash-reporter",
                "--mute-audio",
                "--hide-scrollbars",
                "--window-size=1280,720",
                "--remote-debugging-port=0",
            ])
            .arg(format!("--user-data-dir={}", profile.display()))
            .arg(url)
            .env("TMPDIR", &tmp)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        Headless::spawn(command, binary, profile, &tmp)
    }

    /// The part of [`Headless::start`] that does not know Chromium's flags, so a test can start a
    /// stand-in.
    fn spawn(mut command: Command, binary: &Path, profile: &Path, tmp: &Path) -> Result<Headless, String> {
        die_with_parent(&mut command);
        let child = command.spawn().map_err(|e| {
            remove_profile(profile);
            remove_profile(tmp);
            format!("cannot start {}: {e}", binary.display())
        })?;
        // The job's own flag when there is one, so `kill_live` stops the job as well as its browser.
        let cancel = current().unwrap_or_default();
        register(child.id(), profile, tmp, cancel);
        Ok(Headless {
            child,
            ws_url: String::new(),
            browser: binary.display().to_string(),
            profile: profile.to_path_buf(),
            tmp: tmp.to_path_buf(),
        })
    }
}

/// The longest a unix socket's path may be (`sun_path` is 108 bytes with its NUL).
const SOCKET_PATH_MAX: usize = 107;

/// The path of the socket Chromium's process singleton makes in `tmp`.
pub fn singleton_socket_len(tmp: &Path) -> usize {
    tmp.as_os_str().len() + "/org.chromium.Chromium.XXXXXX/SingletonSocket".len()
}

/// Refuses, in a sentence, a temporary folder whose singleton socket path would not fit: Chromium
/// aborts on it with a fatal check and no message anyone would find.
pub fn check_socket_len(tmp: &Path) -> Result<(), String> {
    let len = singleton_socket_len(tmp);
    if len > SOCKET_PATH_MAX {
        return Err(format!(
            "the browser's socket path under {} would be {len} bytes, more than the {SOCKET_PATH_MAX} a unix socket allows; set TMPDIR to a shorter folder",
            tmp.display()
        ));
    }
    Ok(())
}

/// A short private temporary folder for one browser: `<base>/yarc-XXXXXX`, made by `mkdtemp`
/// (0700, unpredictable, never a fixed name), with the owning app's pid in it for the sweep.
pub fn new_tmp(base: &Path) -> Result<PathBuf, String> {
    use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};
    let mut template = base.join("yarc-XXXXXX").into_os_string().into_vec();
    template.push(0);
    // SAFETY: `template` is a writable NUL-terminated buffer; mkdtemp fills in the X's.
    let made = unsafe { libc::mkdtemp(template.as_mut_ptr().cast()) };
    if made.is_null() {
        return Err(format!("cannot make a temporary folder in {}: {}", base.display(), std::io::Error::last_os_error()));
    }
    template.pop();
    let tmp = PathBuf::from(std::ffi::OsStr::from_bytes(&template));
    let _ = std::fs::write(tmp.join("owner-pid"), std::process::id().to_string());
    Ok(tmp)
}

impl Drop for Headless {
    fn drop(&mut self) {
        let pid = self.child.id();
        // Off the list before the reap: a reaped pid can be reused, and kill_live must not hit it.
        forget(pid);
        // The helpers (zygote, GPU, renderers) are in the group and keep writing into the profile
        // until they die, so they go first; only then is the profile removed.
        kill_group(pid);
        let _ = self.child.kill();
        let _ = self.child.wait();
        remove_profile(&self.profile);
        remove_profile(&self.tmp);
    }
}

/// The folder profiles are made in: under the runtime directory when the session has one, else
/// under the state directory. Never `/tmp`.
pub fn profile_base(runtime: Option<OsString>, state: Option<OsString>, home: Option<OsString>) -> PathBuf {
    if let Some(dir) = runtime.filter(|d| !d.is_empty()).map(PathBuf::from).filter(|d| d.is_dir()) {
        return dir.join("yantrik-arcade");
    }
    let state = state
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(home.unwrap_or_else(|| "/".into())).join(".local/state"));
    state.join("yantrik/arcade/profiles")
}

/// A new private profile for one job: a folder that did not exist, 0700, named by the process, a
/// serial and the clock.
pub fn new_profile() -> Result<PathBuf, String> {
    let base = profile_base(
        std::env::var_os("XDG_RUNTIME_DIR"),
        std::env::var_os("XDG_STATE_HOME"),
        std::env::var_os("HOME"),
    );
    profile_in(&base)
}

pub fn profile_in(base: &Path) -> Result<PathBuf, String> {
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    crate::library::private_folder(base).map_err(|e| format!("cannot make {} private: {e}", base.display()))?;
    sweep_dead(base);
    sweep_dead_tmp(Path::new("/tmp"));
    for _ in 0..8 {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let profile = base.join(format!("job-{}-{}-{nanos:09}", std::process::id(), SERIAL.fetch_add(1, Ordering::Relaxed)));
        let mut made = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt as _;
            made.mode(0o700);
        }
        match made.create(&profile) {
            Ok(()) => return Ok(profile),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("cannot create a profile directory: {e}")),
        }
    }
    Err("cannot find a free profile directory".into())
}

/// Removes the temporary folders (`yarc-*` directly in `base`) of apps that are gone: real
/// directories owned by this user, never a link, and only when the pid in `owner-pid` is dead (or,
/// with no pid written, the folder is over a minute old).
fn sweep_dead_tmp(base: &Path) {
    use std::os::unix::fs::MetadataExt as _;
    let Ok(entries) = std::fs::read_dir(base) else { return };
    // SAFETY: geteuid has no preconditions.
    let me = unsafe { libc::geteuid() };
    for entry in entries.flatten() {
        if !entry.file_name().to_str().is_some_and(|n| n.starts_with("yarc-")) {
            continue;
        }
        let path = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
        if !meta.file_type().is_dir() || meta.uid() != me {
            continue;
        }
        let owner = std::fs::read_to_string(path.join("owner-pid")).ok().and_then(|t| t.trim().parse::<u32>().ok());
        let dead = match owner {
            Some(pid) => pid != std::process::id() && !process_exists(pid),
            None => meta.modified().ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age.as_secs() > 60),
        };
        if dead {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

/// Removes the profiles of jobs whose app is gone (it crashed or was killed, so `Drop` never ran):
/// directories named `job-<pid>-...` directly in `base`, for a pid that no longer exists. Nothing
/// else is touched: not a link, not a file, not another name, not our own pid's.
fn sweep_dead(base: &Path) {
    let Ok(entries) = std::fs::read_dir(base) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = name.to_str().and_then(|n| n.strip_prefix("job-")).and_then(|n| n.split('-').next()).and_then(|p| p.parse::<u32>().ok()) else {
            continue;
        };
        let path = entry.path();
        let plain_dir = std::fs::symlink_metadata(&path).map(|m| m.file_type().is_dir()).unwrap_or(false);
        if plain_dir && pid != std::process::id() && !process_exists(pid) {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

#[cfg(unix)]
fn process_exists(pid: u32) -> bool {
    // SAFETY: signal 0 only asks whether the process exists; EPERM means it does, and is not ours.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM) }
}

#[cfg(not(unix))]
fn process_exists(_: u32) -> bool {
    true
}

#[cfg(all(test, unix))]
pub(crate) mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;

    fn alive(pid: u32) -> bool {
        // SAFETY: signal 0 only asks whether the process exists.
        unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
    }

    fn gone_within(pid: u32, secs: u64) -> bool {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(secs);
        while std::time::Instant::now() < until {
            let mut status = 0;
            // SAFETY: reaps a child of this test process, if it is one.
            let reaped = unsafe { libc::waitpid(pid as libc::pid_t, &mut status, libc::WNOHANG) };
            if reaped == pid as libc::pid_t || !alive(pid) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        false
    }

    #[test]
    fn a_browser_dies_when_the_thread_that_started_it_ends() {
        let pid = std::thread::spawn(|| {
            let mut sleeper = Command::new("sleep");
            sleeper.arg("60");
            die_with_parent(&mut sleeper);
            sleeper.spawn().unwrap().id()
        })
        .join()
        .unwrap();
        assert!(gone_within(pid, 5), "the child outlived its parent");
    }

    #[test]
    fn ending_on_purpose_stops_every_live_browser() {
        let mut child = Command::new("sleep").arg("60").spawn().unwrap();
        register(child.id(), &std::env::temp_dir().join("arcade-nonexistent-profile"), &std::env::temp_dir().join("arcade-nonexistent-tmp"), Arc::new(AtomicBool::new(false)));
        // The list is shared with every test that ends an app, so nothing here counts it: this
        // child must be dead, whoever's `kill_live` it was that stopped it.
        kill_live();
        let status = child.wait().unwrap();
        assert!(!status.success(), "killed, not finished");
    }

    #[test]
    fn a_profile_left_by_a_dead_app_is_removed_and_nothing_else_is() {
        let base = std::env::temp_dir().join(format!("arcade-sweep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let mut gone = Command::new("true").spawn().unwrap();
        let dead = gone.id();
        gone.wait().unwrap();
        let own = base.join(format!("job-{}-0-0", std::process::id()));
        let stale = base.join(format!("job-{dead}-3-5"));
        let other_name = base.join(format!("keep-{dead}-3-5"));
        let a_file = base.join(format!("job-{dead}-file"));
        let elsewhere = base.join("elsewhere");
        let link = base.join(format!("job-{dead}-link"));
        for dir in [&own, &stale, &other_name, &elsewhere] {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(elsewhere.join("precious"), "x").unwrap();
        std::fs::write(&a_file, "x").unwrap();
        std::os::unix::fs::symlink(&elsewhere, &link).unwrap();
        profile_in(&base).unwrap();
        assert!(!stale.exists(), "the dead app's profile went");
        for kept in [&own, &other_name, &a_file, &elsewhere] {
            assert!(kept.exists(), "{} was not ours to remove", kept.display());
        }
        assert!(std::fs::symlink_metadata(&link).is_ok(), "a link is left alone");
        assert!(elsewhere.join("precious").exists(), "and never followed");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn each_job_gets_a_profile_of_its_own_that_only_the_person_can_enter() {
        let base = std::env::temp_dir().join(format!("arcade-profiles-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let a = profile_in(&base).unwrap();
        let b = profile_in(&base).unwrap();
        assert_ne!(a, b);
        for dir in [&base, &a, &b] {
            assert_eq!(std::fs::metadata(dir).unwrap().permissions().mode() & 0o777, 0o700, "{}", dir.display());
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn profiles_go_under_the_runtime_directory_or_the_state_directory_never_tmp() {
        let runtime = std::env::temp_dir();
        assert_eq!(profile_base(Some(runtime.clone().into()), None, None), runtime.join("yantrik-arcade"));
        let fallback = profile_base(Some("/no/such/runtime".into()), Some("/state".into()), Some("/home/p".into()));
        assert_eq!(fallback, PathBuf::from("/state/yantrik/arcade/profiles"));
        let home = profile_base(None, None, Some("/home/p".into()));
        assert_eq!(home, PathBuf::from("/home/p/.local/state/yantrik/arcade/profiles"));
        assert!(!home.starts_with("/tmp"));
    }

    /// A stand-in browser: a main process and a helper in its group that keeps writing into the
    /// profile and into its temporary folder, as Chromium's zygote and renderers do.
    pub(crate) fn stand_in(profile: &Path) -> Result<Headless, String> {
        let tmp = new_tmp(Path::new("/tmp"))?;
        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg("( while :; do mkdir -p \"$TMPDIR/org.chromium.Chromium.$$\"; touch \"$1/cache-$$-$RANDOM\"; sleep 0.005; done ) & sleep 60")
            .arg("fake-browser")
            .arg(profile)
            .env("TMPDIR", &tmp);
        Headless::spawn(command, Path::new("sh"), profile, &tmp)
    }

    /// 20 jobs' worth of a browser whose helpers are still writing when the job ends: none leaves
    /// its profile or its temporary folder behind.
    #[test]
    fn twenty_jobs_leave_no_profile_and_no_browser_temporary_folder() {
        let base = std::env::temp_dir().join(format!("arcade-leak-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let mut tmps = Vec::new();
        for _ in 0..20 {
            let profile = profile_in(&base).unwrap();
            let job = stand_in(&profile).unwrap();
            tmps.push(job.tmp.clone());
            std::thread::sleep(std::time::Duration::from_millis(40));
            // Other tests end apps, and an app ending stops every live browser, this one's too: so
            // only what is left after the job is asserted.
            drop(job);
            assert!(!profile.exists(), "{} was left behind", profile.display());
        }
        for tmp in tmps {
            assert!(!tmp.exists(), "{} was left behind", tmp.display());
        }
        let left: Vec<_> = std::fs::read_dir(&base).unwrap().flatten().map(|e| e.path()).collect();
        assert!(left.is_empty(), "left in the profile folder: {left:?}");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The app ending on purpose (a window close or SIGTERM) clears the profile and the temporary
    /// folder of a live job, and does not leave them for the next start's sweep.
    #[test]
    fn ending_on_purpose_clears_the_folders_of_a_live_job() {
        let base = std::env::temp_dir().join(format!("arcade-live-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let profile = profile_in(&base).unwrap();
        let job = stand_in(&profile).unwrap();
        let tmp = job.tmp.clone();
        std::thread::sleep(std::time::Duration::from_millis(40));
        kill_live();
        assert!(!profile.exists() && !tmp.exists(), "the live job's folders went with it");
        drop(job);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Review M1: Chromium aborts when its singleton socket path is over 107 bytes. The temporary
    /// folder is short and not under the profile, so no profile base can push it over, the state
    /// directory fallback included.
    #[test]
    fn the_socket_path_fits_whatever_the_profile_base_is() {
        let tmp = new_tmp(Path::new("/tmp")).unwrap();
        assert!(singleton_socket_len(&tmp) <= 107, "{}", singleton_socket_len(&tmp));
        assert!(check_socket_len(&tmp).is_ok());
        // What the profile-inside design put under the state-directory fallback of a long home.
        let base = profile_base(None, None, Some("/home/pranab".into()));
        let old = base.join("job-2096961-1-123456789/tmp");
        assert!(singleton_socket_len(&old) > 107, "that is the path that aborted: {}", singleton_socket_len(&old));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn a_temporary_folder_that_would_not_fit_is_refused_in_a_sentence() {
        let long = Path::new("/home/someone-with-a-long-name/.local/state/yantrik/arcade/profiles/job-1234567-12-123456789/tmp");
        let err = check_socket_len(long).unwrap_err();
        assert!(err.contains("107") && err.contains("TMPDIR"), "{err}");
    }

    #[test]
    fn a_temporary_folder_is_private_unpredictable_and_swept_only_when_its_owner_is_gone() {
        let a = new_tmp(Path::new("/tmp")).unwrap();
        let b = new_tmp(Path::new("/tmp")).unwrap();
        assert_ne!(a, b);
        assert_eq!(std::fs::metadata(&a).unwrap().permissions().mode() & 0o777, 0o700);
        // Owned by a pid that is gone: the sweep takes it. Ours stays.
        let mut gone = Command::new("true").spawn().unwrap();
        let dead = gone.id();
        gone.wait().unwrap();
        std::fs::write(a.join("owner-pid"), dead.to_string()).unwrap();
        let elsewhere = std::env::temp_dir().join(format!("arcade-sweep-target-{}", std::process::id()));
        std::fs::create_dir_all(&elsewhere).unwrap();
        let link = Path::new("/tmp").join(format!("yarc-link-{}", std::process::id()));
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&elsewhere, &link).unwrap();
        sweep_dead_tmp(Path::new("/tmp"));
        assert!(!a.exists(), "a dead owner's folder is swept");
        assert!(b.exists(), "this app's is not");
        assert!(std::fs::symlink_metadata(&link).is_ok() && elsewhere.exists(), "a link is never followed");
        let _ = std::fs::remove_file(&link);
        let _ = std::fs::remove_dir_all(&elsewhere);
        let _ = std::fs::remove_dir_all(&b);
    }

    /// A job's wait ends within a poll of the app ending, not after its timeout: the job runs on a
    /// thread of its own, as in the app, waiting as the verifier does.
    #[test]
    fn a_job_waiting_on_its_browser_returns_at_once_when_the_app_ends() {
        let base = std::env::temp_dir().join(format!("arcade-cancel-{}", std::process::id()));
        let (done, heard) = std::sync::mpsc::channel();
        let job_base = base.clone();
        std::thread::spawn(move || {
            let _scope = JobScope::enter(Default::default());
            let profile = profile_in(&job_base).unwrap();
            let browser = stand_in(&profile).unwrap();
            nap(std::time::Duration::from_secs(30));
            let _ = done.send(cancelled());
            drop(browser);
        });
        std::thread::sleep(std::time::Duration::from_millis(100));
        let closed = std::time::Instant::now();
        kill_live();
        let was_cancelled = heard.recv_timeout(std::time::Duration::from_millis(300));
        assert_eq!(was_cancelled, Ok(true), "the job did not return within 300 ms of the close ({:?})", closed.elapsed());
        let _ = std::fs::remove_dir_all(&base);
    }
}
