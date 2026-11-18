//! Running the container runtime, off the UI thread.
//!
//! Every process this app starts is started here, from a job: `which podman`, the listing
//! (`ps`, `images`, `volume ls`) and each command a button or the Mind asks for. Before the
//! framework they all ran on the window's thread, which froze the window for as long as the
//! daemon took to answer, and serialised them by accident. [`Machine`] keeps the serialising on
//! purpose: one turn at a time, each command followed by the listing it changed, under one lock,
//! so two jobs never interleave their commands and listings, and each turn is numbered so the
//! model can tell a newer listing from an older one that happened to land later.

use std::process::Command;
use std::sync::{Arc, Mutex};

use crate::runtime::{self, Availability, Container, Exit, Image, Volume};

/// How a process is started: the live one, or a test's fake.
pub trait Exec: Send + Sync {
    /// Whether this binary is on the machine (`which`).
    fn has(&self, binary: &str) -> bool;
    /// Run it once and keep everything it said.
    fn run(&self, binary: &str, argv: &[&str]) -> Exit;
}

/// The machine's own processes.
pub struct Live;

impl Exec for Live {
    fn has(&self, binary: &str) -> bool {
        Command::new("which")
            .arg(binary)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    fn run(&self, binary: &str, argv: &[&str]) -> Exit {
        match Command::new(binary).args(argv).output() {
            Ok(out) => Exit::Ran {
                code: out.status.code(),
                stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Exit::Missing,
            Err(e) => Exit::Unstartable(e.to_string()),
        }
    }
}

/// What the runtime listed, read in one turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    /// The turn it was read in: a listing from an earlier turn than the one on screen is stale.
    pub turn: u64,
    /// The runtime asked: `podman` or `docker`.
    pub runtime: &'static str,
    pub availability: Availability,
    pub containers: Vec<Container>,
    pub images: Vec<Image>,
    pub volumes: Vec<Volume>,
}

/// One command, and the listing read straight after it, in the same turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Performed {
    pub exit: Exit,
    pub listing: Listing,
}

/// The runtime, one turn at a time.
#[derive(Clone)]
pub struct Machine {
    exec: Arc<dyn Exec>,
    /// The last turn's number, and the lock that makes turns one at a time.
    turns: Arc<Mutex<u64>>,
}

impl Machine {
    pub fn new(exec: Arc<dyn Exec>) -> Machine {
        Machine {
            exec,
            turns: Arc::new(Mutex::new(0)),
        }
    }

    /// The machine this app manages: the live one, or a test's fake installed for these paths.
    #[cfg(not(test))]
    pub fn for_app(_paths: &yantrik_app::Paths) -> Machine {
        Machine::new(Arc::new(Live))
    }

    #[cfg(test)]
    pub fn for_app(paths: &yantrik_app::Paths) -> Machine {
        fake::machine_for(paths)
    }

    /// Re-read containers, images and volumes.
    pub fn list(&self) -> Listing {
        let mut turn = self.turns.lock().unwrap_or_else(|p| p.into_inner());
        *turn += 1;
        let runtime = self.detect();
        self.read(*turn, runtime)
    }

    /// Run one command, then re-read what it changed.
    pub fn perform(&self, argv: &[String]) -> Performed {
        let mut turn = self.turns.lock().unwrap_or_else(|p| p.into_inner());
        *turn += 1;
        let runtime = self.detect();
        let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
        let exit = self.exec.run(runtime, &argv);
        Performed {
            exit,
            listing: self.read(*turn, runtime),
        }
    }

    /// Which runtime this machine has. Podman wins when both are present, as it always did.
    fn detect(&self) -> &'static str {
        if self.exec.has("podman") {
            "podman"
        } else {
            "docker"
        }
    }

    fn read(&self, turn: u64, runtime: &'static str) -> Listing {
        let ps = self.exec.run(runtime, &["ps", "-a", "--format", runtime::PS_FORMAT]);
        let availability = runtime::availability(&ps);
        let containers = match &ps {
            Exit::Ran { code: Some(0), stdout, .. } => runtime::parse_containers(stdout),
            _ => Vec::new(),
        };
        // Nothing to ask when the runtime cannot answer: two more processes that would fail the
        // same way, and two more empty lists that would look like emptiness rather than absence.
        let (images, volumes) = if availability.is_ready() {
            (
                self.listed(runtime, &["images", "--format", runtime::IMAGES_FORMAT], runtime::parse_images),
                self.listed(runtime, &["volume", "ls", "--format", runtime::VOLUMES_FORMAT], runtime::parse_volumes),
            )
        } else {
            (Vec::new(), Vec::new())
        };
        Listing {
            turn,
            runtime,
            availability,
            containers,
            images,
            volumes,
        }
    }

    fn listed<T>(&self, runtime: &str, argv: &[&str], parse: fn(&str) -> Vec<T>) -> Vec<T> {
        match self.exec.run(runtime, argv) {
            Exit::Ran { code: Some(0), stdout, .. } => parse(&stdout),
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod live_tests {
    use super::{Exec, Live};
    use crate::runtime::Exit;

    #[test]
    fn a_binary_that_is_not_on_the_machine_is_missing_not_a_failure_to_start() {
        let nowhere = "yantrik-no-such-container-runtime";
        assert_eq!(Live.run(nowhere, &["ps"]), Exit::Missing);
        assert!(!Live.has(nowhere));
    }
}

#[cfg(test)]
pub mod fake {
    //! A container runtime for tests: it answers what the test told it to, records what it was
    //! asked, and starts no process. An app under test gets the one installed for its paths, or
    //! one with no runtime at all.

    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex, OnceLock};

    use super::{Exec, Machine};
    use crate::runtime::Exit;

    #[derive(Default)]
    pub struct FakeExec {
        podman: bool,
        answers: Mutex<HashMap<String, Exit>>,
        asked: Mutex<Vec<String>>,
        panics_on: Mutex<Option<String>>,
    }

    impl FakeExec {
        pub fn new() -> Arc<FakeExec> {
            Arc::new(FakeExec::default())
        }

        pub fn podman() -> Arc<FakeExec> {
            Arc::new(FakeExec {
                podman: true,
                ..FakeExec::default()
            })
        }

        /// `argv` (after the binary, joined by spaces) answers `exit` from now on.
        pub fn answer(&self, argv: &str, exit: Exit) {
            self.answers.lock().unwrap().insert(argv.to_string(), exit);
        }

        /// `argv` exits 0 with this on stdout.
        pub fn ok(&self, argv: &str, stdout: &str) {
            self.answer(argv, ran(0, stdout, ""));
        }

        /// The `ps` listing, as the runtime prints it.
        pub fn ps(&self, stdout: &str) {
            self.ok(&format!("ps -a --format {}", crate::runtime::PS_FORMAT), stdout);
        }

        /// A working runtime with these containers and no images or volumes.
        pub fn ready(&self, ps: &str) {
            self.ps(ps);
            self.ok(&format!("images --format {}", crate::runtime::IMAGES_FORMAT), "");
            self.ok(&format!("volume ls --format {}", crate::runtime::VOLUMES_FORMAT), "");
        }

        /// Running `argv` panics, as a worker whose parsing went wrong would.
        pub fn panic_on(&self, argv: &str) {
            *self.panics_on.lock().unwrap() = Some(argv.to_string());
        }

        /// Every command run so far, as `binary argv…`.
        pub fn asked(&self) -> Vec<String> {
            self.asked.lock().unwrap().clone()
        }
    }

    pub fn ran(code: i32, stdout: &str, stderr: &str) -> Exit {
        Exit::Ran {
            code: Some(code),
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
        }
    }

    impl Exec for FakeExec {
        fn has(&self, binary: &str) -> bool {
            binary == "podman" && self.podman
        }

        fn run(&self, binary: &str, argv: &[&str]) -> Exit {
            let key = argv.join(" ");
            self.asked.lock().unwrap().push(format!("{binary} {key}"));
            if self.panics_on.lock().unwrap().as_deref() == Some(key.as_str()) {
                panic!("the runtime printed something this app could not read");
            }
            // Anything the test did not script is a machine with no runtime installed.
            self.answers.lock().unwrap().get(&key).cloned().unwrap_or(Exit::Missing)
        }
    }

    fn installed() -> &'static Mutex<HashMap<PathBuf, Arc<FakeExec>>> {
        static INSTALLED: OnceLock<Mutex<HashMap<PathBuf, Arc<FakeExec>>>> = OnceLock::new();
        INSTALLED.get_or_init(Default::default)
    }

    /// The app that starts at `paths` runs against `exec`.
    pub fn install(paths: &yantrik_app::Paths, exec: Arc<FakeExec>) {
        installed()
            .lock()
            .unwrap()
            .insert(paths.config_file().to_path_buf(), exec);
    }

    pub(super) fn machine_for(paths: &yantrik_app::Paths) -> Machine {
        let exec = installed()
            .lock()
            .unwrap()
            .get(paths.config_file())
            .cloned()
            .unwrap_or_default();
        Machine::new(exec)
    }
}
