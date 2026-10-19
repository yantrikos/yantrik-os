//! Hermes is the default mind until Yantrik Mind is ready.
//!
//! Pranab, 13 October 2026: "Until mind is ready we can have Hermes as default." The OS nightly
//! ships when the OS is ready, without waiting on the Mind, and Hermes is the mind proven to
//! install from nothing with YantrikDB memory and the desktop link (#758: about four minutes on
//! VM 541, all three proofs).
//!
//! What this file decides:
//! - **When the install starts by itself.** Once, after the first run's model step, through the
//!   same job Settings → Minds runs (`crate::wire::harness::install`): no second installer.
//!   A restart in the middle starts it once more (the installer is idempotent), and that is the
//!   only retry: a failure after it waits for Install again.
//! - **When the conversation moves to it.** Only once its three proofs have passed, which the
//!   installer records by writing `installed` (harnesses/lib/install/common.sh, #758). It
//!   attaches during the install, before memory is proven, so attachment alone is not enough for
//!   a mind the person never chose. A chosen mind is handed over on attach, as before.
//! - **What the person is told.** The same sentences in the first run and on the Hermes row, and
//!   once, on the desktop, that it answers from now on.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::harness_catalogue::JobView;
use crate::wire::settings::DEFAULT_MIND;

/// What the first run and the Hermes row say while it installs.
pub const SETTING_UP: &str = "Setting up Hermes, your mind, about 4 minutes. It needs the internet.";
/// What the desktop says once, when Hermes takes over, and what the first run says after.
pub const READY: &str = "Hermes is ready; it answers from now on.";
/// The row's reason when the internet was not there to install from.
pub const NO_INTERNET: &str =
    "Hermes was not set up: this computer cannot reach the internet. Press Install again once it can.";
/// The row's reason when no model was picked for it to use.
pub const NO_MODEL: &str =
    "Hermes was not set up: no model is picked for your minds yet. Choose one in Settings → AI accounts, then press Install again.";

/// The first try and one more after a restart. A machine that cannot install Hermes must not
/// spend every boot downloading three gigabytes to fail again.
const MOST_TRIES: u32 = 2;

/// How long a start waits for the internet before saying it is not there. At boot the network
/// often comes up after the shell, and one try used up on that would be a try wasted.
const WAIT_FOR_INTERNET: Duration = Duration::from_secs(90);

/// What this machine remembers about setting the default mind up.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Record {
    /// Installs started by the desktop itself, never by a button.
    pub tries: u32,
    /// "Hermes is ready" has been said.
    pub announced: bool,
}

/// Why the first run asked, which decides what may start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum When {
    /// The model step is done: start, unless an install was ever started here.
    FirstRun,
    /// The shell started: carry on an install the first run started and that never finished.
    Restart,
}

/// What a start did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Started,
    /// Installed and proven already; nothing to do.
    Proven,
    /// Started before (the first run), or tried as often as it may be (a restart).
    NotAgain,
    /// An install of it is running now.
    Running,
    NoModel,
    Offline,
    Failed(String),
}

/// The facts a start depends on and the two things it does, apart from the machine so the rules
/// can be tested without a network or a three-gigabyte download.
pub trait Machine {
    /// The gateway has a model picked for every mind.
    fn picked(&self) -> bool;
    /// The installer wrote `installed`: it answers, remembers and reaches the desktop.
    fn proven(&self) -> bool;
    /// An install of it is running: a job on this shell's board, or an installer an earlier shell
    /// started that is still going.
    fn installing(&self) -> bool;
    fn online(&self) -> bool;
    fn install(&self) -> Result<(), String>;
    /// Say on the row why it was not installed.
    fn refuse(&self, why: &str);
    fn record(&self) -> Record;
    fn keep(&self, record: &Record);
}

/// Start the install when the rules say so. At most one start per call, and none after
/// [`MOST_TRIES`] across the life of the machine.
pub fn begin(machine: &dyn Machine, when: When) -> Outcome {
    if machine.proven() {
        return Outcome::Proven;
    }
    let mut record = machine.record();
    let may = match when {
        When::FirstRun => record.tries == 0,
        When::Restart => record.tries > 0 && record.tries < MOST_TRIES,
    };
    if !may {
        return Outcome::NotAgain;
    }
    if machine.installing() {
        return Outcome::Running;
    }
    if !machine.picked() {
        // Not counted: nothing was tried, and nothing tries again by itself either.
        machine.refuse(NO_MODEL);
        return Outcome::NoModel;
    }
    // Counted before anything runs, so a shell that dies in the next second does not get a
    // fresh try out of it at the next boot.
    record.tries += 1;
    machine.keep(&record);
    if !machine.online() {
        machine.refuse(NO_INTERNET);
        return Outcome::Offline;
    }
    match machine.install() {
        Ok(()) => Outcome::Started,
        Err(why) => {
            machine.refuse(&why);
            Outcome::Failed(why)
        }
    }
}

/// What the first run says about the default mind: nothing before it was started, then the
/// row's own words.
pub fn words(record: &Record, proven: bool, job: Option<&JobView>) -> String {
    if proven {
        return if record.tries > 0 { READY.to_string() } else { String::new() };
    }
    match job {
        Some(job) if job.running => SETTING_UP.to_string(),
        Some(job) if !job.error.is_empty() => job.error.clone(),
        _ if record.tries > 0 => SETTING_UP.to_string(),
        _ => String::new(),
    }
}

// ── This machine ────────────────────────────────────────────────────────────────────────────

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/root"))
}

/// `~/.local/state/yantrik/harnesses/<id>/installed`, which the installer writes only once all
/// three proofs pass (harnesses/lib/install/common.sh, `harness_state`).
pub fn proven_marker(home: &Path, id: &str) -> PathBuf {
    home.join(".local/state/yantrik/harnesses").join(id).join("installed")
}

/// Whether the default mind is installed and proven on this machine.
pub fn proven() -> bool {
    proven_marker(&home(), DEFAULT_MIND).exists()
}

fn record_path(home: &Path) -> PathBuf {
    home.join(".local/state/yantrik/default-mind.json")
}

pub fn load_record(path: &Path) -> Record {
    std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn keep_record(path: &Path, record: &Record) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match serde_json::to_string(record) {
        Ok(text) => {
            if let Err(e) = std::fs::write(path, text + "\n") {
                tracing::warn!(error = %e, "could not keep what was done about the default mind");
            }
        }
        Err(e) => tracing::warn!(error = %e, "could not write what was done about the default mind"),
    }
}

/// An installer for `id` running under `proc`, whoever started it. A shell that crashed leaves
/// its job's `sh` behind, still installing, and a second one beside it would be two processes
/// writing one `~/.hermes`.
pub fn installer_running(proc: &Path, id: &str) -> bool {
    let needle = format!("lib/install/{id}.sh");
    let Ok(entries) = std::fs::read_dir(proc) else { return false };
    entries.flatten().any(|entry| {
        let name = entry.file_name();
        if !name.to_string_lossy().bytes().all(|b| b.is_ascii_digit()) {
            return false;
        }
        std::fs::read(entry.path().join("cmdline"))
            .map(|raw| String::from_utf8_lossy(&raw).replace('\0', " ").contains(&needle))
            .unwrap_or(false)
    })
}

/// Whether the place Hermes installs from answers, waiting up to `within` for it.
fn reachable(within: Duration) -> bool {
    use std::net::{TcpStream, ToSocketAddrs};
    let deadline = Instant::now() + within;
    loop {
        let reached = ("github.com", 443)
            .to_socket_addrs()
            .ok()
            .and_then(|mut addrs| addrs.next())
            .is_some_and(|addr| TcpStream::connect_timeout(&addr, Duration::from_secs(5)).is_ok());
        if reached || Instant::now() >= deadline {
            return reached;
        }
        std::thread::sleep(Duration::from_secs(10));
    }
}

struct Here {
    home: PathBuf,
}

impl Machine for Here {
    fn picked(&self) -> bool {
        !crate::picker::choices::load(&crate::picker::choices::path()).model.is_empty()
    }
    fn proven(&self) -> bool {
        proven_marker(&self.home, DEFAULT_MIND).exists()
    }
    fn installing(&self) -> bool {
        crate::harness_install::running(DEFAULT_MIND) || installer_running(Path::new("/proc"), DEFAULT_MIND)
    }
    fn online(&self) -> bool {
        reachable(WAIT_FOR_INTERNET)
    }
    fn install(&self) -> Result<(), String> {
        crate::wire::harness::install(DEFAULT_MIND).map(|command| {
            tracing::info!(harness = DEFAULT_MIND, command = %command, "setting up the default mind");
        })
    }
    fn refuse(&self, why: &str) {
        crate::harness_install::refuse(DEFAULT_MIND, why);
    }
    fn record(&self) -> Record {
        load_record(&record_path(&self.home))
    }
    fn keep(&self, record: &Record) {
        keep_record(&record_path(&self.home), record);
    }
}

/// One start at a time in this shell: the first run's pick and its Try again can both arrive
/// while the first is still waiting for the internet.
static STARTING: Mutex<()> = Mutex::new(());

/// Start the default mind's install in the background if the rules say so. Never on the live
/// image: the person has not installed yet, and what it would download is lost at reboot.
pub fn start(when: When) {
    if crate::lock::live_session() {
        return;
    }
    let spawned = std::thread::Builder::new().name("default-mind".into()).spawn(move || {
        let _one = STARTING.lock().unwrap_or_else(|e| e.into_inner());
        let outcome = begin(&Here { home: home() }, when);
        tracing::info!(harness = DEFAULT_MIND, ?when, ?outcome, "default mind");
    });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "no thread to set up the default mind");
    }
}

/// The first run's line about the default mind.
pub fn words_now() -> String {
    let home = home();
    let job = crate::harness_install::views().remove(DEFAULT_MIND);
    words(&load_record(&record_path(&home)), proven_marker(&home, DEFAULT_MIND).exists(), job.as_ref())
}

/// The desktop has just handed the conversation to the default mind, which nobody chose: say so,
/// once for the life of the machine, not at every boot.
pub fn handed_over() {
    let path = record_path(&home());
    let mut record = load_record(&path);
    if record.announced {
        return;
    }
    record.announced = true;
    keep_record(&path, &record);
    use yantrik_app_runtime::notify::{Level, Notification};
    yantrik_app_runtime::notify::send(Notification::new("Yantrik", READY).urgency(Level::Normal));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[derive(Default)]
    struct Fake {
        picked: bool,
        proven: bool,
        installing: Cell<bool>,
        offline: bool,
        fails: bool,
        installs: Cell<u32>,
        refused: RefCell<Vec<String>>,
        record: RefCell<Record>,
    }

    impl Machine for Fake {
        fn picked(&self) -> bool {
            self.picked
        }
        fn proven(&self) -> bool {
            self.proven
        }
        fn installing(&self) -> bool {
            self.installing.get()
        }
        fn online(&self) -> bool {
            !self.offline
        }
        fn install(&self) -> Result<(), String> {
            if self.fails {
                return Err("nothing on this machine describes a harness called `hermes`".into());
            }
            self.installs.set(self.installs.get() + 1);
            // The job is on the board from here on.
            self.installing.set(true);
            Ok(())
        }
        fn refuse(&self, why: &str) {
            self.refused.borrow_mut().push(why.to_string());
        }
        fn record(&self) -> Record {
            self.record.borrow().clone()
        }
        fn keep(&self, record: &Record) {
            *self.record.borrow_mut() = record.clone();
        }
    }

    fn picked() -> Fake {
        Fake { picked: true, ..Default::default() }
    }

    #[test]
    fn the_first_run_starts_exactly_one_hermes_install_once_a_model_is_picked() {
        let m = picked();
        assert_eq!(begin(&m, When::FirstRun), Outcome::Started);
        // The pick again, Try again, going back and choosing another model: still one.
        assert_eq!(begin(&m, When::FirstRun), Outcome::NotAgain);
        m.installing.set(false);
        assert_eq!(begin(&m, When::FirstRun), Outcome::NotAgain);
        assert_eq!(m.installs.get(), 1);
        assert_eq!(m.record().tries, 1);
    }

    #[test]
    fn no_install_starts_without_a_model_and_the_row_says_why() {
        let m = Fake::default();
        assert_eq!(begin(&m, When::FirstRun), Outcome::NoModel);
        assert_eq!(m.installs.get(), 0);
        assert_eq!(*m.refused.borrow(), [NO_MODEL]);
        assert_eq!(m.record().tries, 0, "nothing was tried, so nothing is used up");
    }

    #[test]
    fn no_internet_is_said_on_the_row_and_tried_once_more_after_a_restart_and_never_again() {
        let m = Fake { picked: true, offline: true, ..Default::default() };
        assert_eq!(begin(&m, When::FirstRun), Outcome::Offline);
        assert_eq!(*m.refused.borrow(), [NO_INTERNET]);
        assert_eq!(begin(&m, When::Restart), Outcome::Offline);
        for _ in 0..5 {
            assert_eq!(begin(&m, When::Restart), Outcome::NotAgain, "no loop: two tries in all");
        }
        assert_eq!(m.record().tries, MOST_TRIES);
        assert_eq!(m.installs.get(), 0);
    }

    #[test]
    fn a_reboot_mid_install_starts_it_once_more_and_a_proven_one_is_left_alone() {
        let m = picked();
        begin(&m, When::FirstRun);
        // The reboot: the board is empty and nothing is installing.
        m.installing.set(false);
        assert_eq!(begin(&m, When::Restart), Outcome::Started);
        m.installing.set(false);
        assert_eq!(begin(&m, When::Restart), Outcome::NotAgain);
        assert_eq!(m.installs.get(), 2);

        let done = Fake { picked: true, proven: true, ..Default::default() };
        assert_eq!(begin(&done, When::FirstRun), Outcome::Proven);
        assert_eq!(begin(&done, When::Restart), Outcome::Proven);
        assert_eq!(done.installs.get(), 0);
    }

    #[test]
    fn a_restart_never_starts_an_install_the_first_run_did_not() {
        // A machine set up before the default existed, or one whose first run had no model:
        // a restart installs nothing it was not asked to.
        let m = picked();
        assert_eq!(begin(&m, When::Restart), Outcome::NotAgain);
        assert_eq!(m.installs.get(), 0);
    }

    #[test]
    fn two_installs_never_run_at_once() {
        let m = picked();
        m.installing.set(true);
        assert_eq!(begin(&m, When::FirstRun), Outcome::Running);
        assert_eq!(m.installs.get(), 0);
        assert_eq!(m.record().tries, 0, "the one running is not this desktop's try to count");
    }

    #[test]
    fn an_install_that_cannot_start_says_why_on_the_row() {
        let m = Fake { picked: true, fails: true, ..Default::default() };
        assert!(matches!(begin(&m, When::FirstRun), Outcome::Failed(_)));
        assert_eq!(m.refused.borrow().len(), 1);
    }

    #[test]
    fn the_first_run_says_setting_up_then_ready_or_the_rows_reason() {
        let fresh = Record::default();
        let started = Record { tries: 1, announced: false };
        let running = JobView { running: true, ..Default::default() };
        let failed = JobView { error: "install failed: it exited 1".into(), ..Default::default() };
        assert_eq!(words(&fresh, false, None), "", "nothing is said before it starts");
        assert_eq!(words(&started, false, Some(&running)), SETTING_UP);
        assert_eq!(words(&started, false, None), SETTING_UP, "after a reboot, before the job is back");
        assert_eq!(words(&started, false, Some(&failed)), "install failed: it exited 1");
        assert_eq!(words(&started, true, None), READY);
        assert_eq!(words(&fresh, true, None), "", "a Hermes the first run did not set up is not news here");
    }

    #[test]
    fn the_sentences_are_the_ones_the_owner_asked_for() {
        assert_eq!(SETTING_UP, "Setting up Hermes, your mind, about 4 minutes. It needs the internet.");
        assert_eq!(READY, "Hermes is ready; it answers from now on.");
    }

    #[test]
    fn the_proof_is_the_file_the_installer_writes_after_all_three_proofs() {
        let home = Path::new("/home/p");
        assert_eq!(proven_marker(home, "hermes"), Path::new("/home/p/.local/state/yantrik/harnesses/hermes/installed"));
    }

    #[test]
    fn an_installer_left_running_by_an_earlier_shell_is_found() {
        let proc = std::env::temp_dir().join(format!("default-mind-proc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&proc);
        std::fs::create_dir_all(proc.join("self")).unwrap();
        std::fs::create_dir_all(proc.join("41")).unwrap();
        std::fs::write(proc.join("41/cmdline"), b"sh\0-lc\0sh /opt/yantrik/share/harnesses/pi/../lib/install/pi.sh\0").unwrap();
        assert!(!installer_running(&proc, "hermes"));
        std::fs::create_dir_all(proc.join("42")).unwrap();
        std::fs::write(proc.join("42/cmdline"), b"sh\0/opt/yantrik/share/harnesses/hermes/../lib/install/hermes.sh\0/x\0").unwrap();
        assert!(installer_running(&proc, "hermes"));
        let _ = std::fs::remove_dir_all(&proc);
    }

    /// The nightly may be built without Yantrik Mind (`YANTRIK_ISO_WITHOUT_MIND=1`). What is drawn
    /// before any mind has attached, or whatever is attached, must not name it.
    #[test]
    fn with_no_mind_installed_the_dock_the_home_box_and_private_mode_name_no_yantrik_mind() {
        let ui = Path::new(env!("CARGO_MANIFEST_DIR")).join("../yantrik-ui-slint/ui");
        let read = |f: &str| std::fs::read_to_string(ui.join(f)).unwrap();
        let mind = ["Yantrik", " Mind"].concat();
        let live = |src: &str| -> Vec<String> {
            src.lines()
                .map(|l| l.split("//").next().unwrap_or(""))
                .filter(|l| l.contains(&mind))
                .map(str::to_string)
                .collect()
        };
        for f in ["components/grounded_dock.slint", "components/desktop_home.slint", "components/mind_mode_menu.slint", "onboarding_first_run.slint"] {
            assert_eq!(live(&read(f)), Vec::<String>::new(), "{f} names a mind that may not be here");
        }
        let menu = read("components/mind_mode_menu.slint");
        assert!(menu.contains("\"Turn every mind off and record nothing, until you end it.\""), "\"the Mind\" read as Yantrik Mind");
        assert!(!menu.contains("\"The Mind is off") && !menu.contains("\"Turn the Mind off"));
        assert!(read("app.slint").contains("mind-label: root.active-harness-name"), "the dock names the mind that answers");
    }

    #[test]
    fn the_record_survives_a_restart_and_a_missing_one_is_a_machine_never_tried() {
        let dir = std::env::temp_dir().join(format!("default-mind-record-{}", std::process::id()));
        let path = record_path(&dir);
        assert_eq!(load_record(&path), Record::default());
        keep_record(&path, &Record { tries: 1, announced: true });
        assert_eq!(load_record(&path), Record { tries: 1, announced: true });
        let _ = std::fs::remove_dir_all(&dir);
    }
}
