//! One proof run at a time, and the last report kept: in memory, and in one private file so
//! `describe` has it after a restart.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use super::{run, Gate, Report};
use crate::wire::onboarding::{Proof, Proofs};

/// The least time between one run and the next, from the last report's `at`, for every caller:
/// each run is a model turn on the person's provider, and `prove` is graded `standard`, which
/// raises no card in `ask` mode, so this is the limit on how often it can be asked.
pub const MIN_GAP_SECS: i64 = 300;

/// A file bigger than this is not a report this code wrote.
const MAX_FILE_BYTES: u64 = 64 * 1024;

/// What a request got: the words, and the report they come from.
pub struct Reply {
    /// `proven`, `recent`, `already-running` or `refused`.
    pub state: &'static str,
    /// What a person reads: the report, or why there is none.
    pub text: String,
    /// The report this reply is about: the new one, or the last one kept.
    pub report: Option<Report>,
}

impl Reply {
    /// The action's reply: `readable` says `text` is what to print, so `yos act shell prove`
    /// shows it and not the JSON around it.
    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({ "state": self.state, "readable": true, "text": self.text, "report": self.report })
    }
}

/// The proofs' one door.
pub struct Prover {
    running: AtomicBool,
    last: Mutex<Option<Report>>,
    path: Option<PathBuf>,
}

/// Held while a run is on; lets the next one in when it ends, however it ended.
struct Turn<'a>(&'a AtomicBool);

impl Drop for Turn<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl Prover {
    /// A prover keeping its report at `path` (`None`: in memory only). The last report is read
    /// back now; a file that cannot be read is "never proven".
    pub fn new(path: Option<PathBuf>) -> Self {
        let last = path.as_deref().and_then(read);
        Prover { running: AtomicBool::new(false), last: Mutex::new(last), path }
    }

    /// The shell's one prover, its file in the shell's own state directory.
    pub fn shell() -> &'static Prover {
        static SHELL: OnceLock<Prover> = OnceLock::new();
        SHELL.get_or_init(|| Prover::new(yantrik_ml::private_dir::state_dir("proofs").ok().map(|d| d.join("last.json"))))
    }

    pub fn last(&self) -> Option<Report> {
        self.last.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// `describe shell` › `proofs`.
    pub fn describe(&self, now: i64) -> serde_json::Value {
        self.last().map_or_else(|| serde_json::json!("never"), |r| r.describe(now))
    }

    /// Run the proofs, unless the gate says no, one finished less than [`MIN_GAP_SECS`] ago, or one is running. Blocking: seconds.
    pub fn prove(&self, gate: &dyn Gate, proofs: &dyn Proofs, ask: &dyn Fn(&str) -> Proof, build: &str, now: i64) -> Reply {
        let last = self.last();
        let beside = |said: &str| match &last {
            Some(r) => format!("{said}\nThe last result:\n{}", r.text()),
            None => format!("{said}\nThe companion has never been proven."),
        };
        if let Some(why) = gate.refusal() {
            return Reply { state: "refused", text: beside(&why), report: last };
        }
        if let Some(r) = last.as_ref().filter(|r| (0..MIN_GAP_SECS).contains(&(now - r.at))) {
            let ago = now - r.at;
            let when = if ago < 60 { format!("{ago} s") } else { format!("{} min", ago / 60) };
            let said = format!("Proven {when} ago, so nothing was asked again: it can be proven once in {} minutes.", MIN_GAP_SECS / 60);
            return Reply { state: "recent", text: beside(&said), report: last };
        }
        if self.running.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err() {
            return Reply { state: "already-running", text: beside("Already running: a proof started a moment ago is still working."), report: last };
        }
        let _turn = Turn(&self.running);
        let report = run(proofs, ask, build, now);
        *self.last.lock().unwrap_or_else(|e| e.into_inner()) = Some(report.clone());
        if let Some(path) = &self.path {
            if let Err(why) = write(path, &report) {
                tracing::warn!("the proofs' report was not kept: {why}");
            }
        }
        Reply { state: "proven", text: report.text(), report: Some(report) }
    }
}

fn write(path: &std::path::Path, report: &Report) -> Result<(), String> {
    let text = serde_json::to_string_pretty(report).map_err(|e| e.to_string())?;
    crate::private_file::write(path, format!("{text}\n").as_bytes(), crate::private_file::Publish::Replace, || Ok(()))
}

fn read(path: &std::path::Path) -> Option<Report> {
    let size = std::fs::metadata(path).ok()?.len();
    if size > MAX_FILE_BYTES {
        return None;
    }
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}
