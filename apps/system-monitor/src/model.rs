//! What System Monitor is: the model every command changes and every render reads, and the
//! commands.

use std::sync::Arc;
use std::time::Instant;

use yantrik_app::mind::{AskError, Mind};
use yantrik_app::rail::{Rail, RailCommand};
use yantrik_app::shown::Shown;
use yantrik_ipc_contracts::machine_status::MachineStatus;
use yantrik_ipc_contracts::system_monitor::ProcessInfo;

use crate::machine::Machine;
use crate::outcome::{self, Observed, Provenance};
use crate::readout::Readout;
use crate::sample::Reading;

/// The parts of the model `render` redraws by name.
pub const PART_READINGS: &str = "readings";
pub const PART_PROCESSES: &str = "processes";
pub const PART_NOTICE: &str = "notice";
pub const PART_AI: &str = "ai";
/// What a command marks when it changes nothing on screen (a reading started, a stale one
/// dropped, a kill started): `render` draws nothing for it.
pub const PART_QUIET: &str = "quiet";

/// The busiest hundred; the list shows them all and `describe` names the top ten.
pub const PROCESS_LIMIT: u32 = 100;

/// The header's AI Explain panel: the one question about the machine.
#[derive(Debug, Clone, Default)]
pub struct Ai {
    pub working: bool,
    pub response: String,
}

pub struct Model {
    /// The last reading, as the cards say it.
    pub readout: Readout,
    /// What the last reading adds up to: the status row, kept whole for `describe`.
    pub status: MachineStatus,
    /// The interval the CPU figures on screen cover; `None` while there is none.
    pub cpu_window_ms: Option<u64>,
    /// Where the numbers on screen came from, as of the most recent reading.
    pub reading: Option<Provenance>,
    /// Why the last thing someone asked for could not be done. Cleared by the next one that can.
    pub failure: Option<String>,
    /// Every process the last reading listed, before the search filter. Kept so a word typed in
    /// the search box narrows the list at once instead of on the next two-second poll.
    pub procs: Vec<ProcessInfo>,
    /// What the search box holds.
    pub search: String,
    /// The list's order: 0 CPU, 1 memory (the screen's `sort-column`). The next reading is
    /// asked for in it.
    pub sort_column: i32,
    /// The word the last `sort_processes` was given, which it answers with.
    pub sorted_by: String,
    /// The reading being taken, by number; `None` when none is. One at a time: a service slower
    /// than the poll would otherwise queue readings faster than they land, and the window would
    /// draw an ever older machine.
    pub sampling: Option<u64>,
    pub next_sample: u64,
    /// When the last reading was asked for, on the app's clock.
    pub sampled_at: Option<Instant>,
    /// Whether the last reading that changed the screen was drawn: a window off screen is read
    /// every [`crate::OFF_SCREEN_EVERY`], not every [`crate::POLL_EVERY`].
    pub shown: Shown,
    /// A poll found the window off screen and let a reading go.
    pub off_screen: bool,
    /// The process the last successful kill ended, which `kill_process` answers with.
    pub ended: Option<Observed>,
    pub ai: Ai,
    pub rail: Rail,
    pub machine: Arc<dyn Machine>,
    pub mind: Arc<dyn Mind>,
}

impl Model {
    pub fn new(machine: Arc<dyn Machine>, mind: Arc<dyn Mind>) -> Model {
        Model {
            readout: Readout::default(),
            status: MachineStatus::default(),
            cpu_window_ms: None,
            reading: None,
            failure: None,
            procs: Vec::new(),
            search: String::new(),
            sort_column: 0,
            sorted_by: String::new(),
            sampling: None,
            next_sample: 1,
            sampled_at: None,
            shown: Shown::default(),
            off_screen: false,
            ended: None,
            ai: Ai::default(),
            rail: Rail::new(mind.clone()),
            machine,
            mind,
        }
    }

    /// The last reading's processes through the search box: the list on screen.
    pub fn shown(&self) -> Vec<ProcessInfo> {
        filter_processes(&self.procs, &self.search)
    }

    /// The order the service is asked for.
    pub fn sort_key(&self) -> &'static str {
        if self.sort_column == 1 {
            "mem"
        } else {
            "cpu"
        }
    }

    /// The one line on the strip: why the last request could not be done, and why the readings
    /// are not the service's, both kept and neither overwriting the other.
    pub fn notice(&self) -> String {
        let degraded = self.reading.as_ref().and_then(|p| p.notice());
        outcome::compose_notice(self.failure.as_deref(), degraded.as_deref())
    }

    /// What the list calls this pid, so the answer says what was ended and not only which
    /// number. The list is the top hundred (filtered), so a pid outside it has no name here.
    pub fn process_name(&self, pid: u32) -> String {
        self.shown()
            .into_iter()
            .find(|p| p.pid as i32 == pid as i32)
            .map(|p| p.name)
            .unwrap_or_else(|| "unknown".to_string())
    }
}

/// The processes whose name contains `query`, ignoring case and the spaces around it. An empty
/// query is no filter.
pub fn filter_processes(procs: &[ProcessInfo], query: &str) -> Vec<ProcessInfo> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return procs.to_vec();
    }
    procs.iter().filter(|p| p.name.to_lowercase().contains(&needle)).cloned().collect()
}

#[derive(Debug, Clone)]
pub enum Command {
    /// Take a reading, unless one is being taken (the two-second timer, and once at start), or
    /// the window is off screen and was read less than [`crate::OFF_SCREEN_EVERY`] ago.
    Poll,
    /// The window was brought back to the person (activated, or presented by a second launch).
    Back,
    /// A reading landed. One whose number is not the reading in flight is stale and dropped.
    Sampled { seq: u64, reading: Reading },
    /// The reading's job ended without one (a panic, the pool's bound): the next poll asks again.
    SampleFailed { seq: u64 },
    /// A column header pressed: 0 CPU, 1 memory.
    SortBy(i32),
    /// The `sort_processes` intent: the column, and the word it was given.
    SortProcesses { column: i32, by: String },
    /// The search box's text (typed, or the `filter_processes` intent).
    Search(String),
    /// End a process: the End button, Force Kill, and `kill_process`.
    End { pid: i32, force: bool },
    /// What ending it was observed to do.
    Ended { outcome: Result<Observed, String> },
    /// The header's AI Explain.
    Explain,
    Explained(Result<String, AskError>),
    DismissAi,
    Rail(RailCommand),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::fake::process;

    #[test]
    fn the_search_narrows_by_name_ignoring_case_and_spaces() {
        let procs = vec![process(7, "pi", 1.0, 1.0), process(9, "Chromium", 1.0, 1.0), process(11, "chrome_crashpad", 1.0, 1.0)];
        let names = |q: &str| filter_processes(&procs, q).into_iter().map(|p| p.pid).collect::<Vec<_>>();
        assert_eq!(names("chrom"), vec![9, 11]);
        assert_eq!(names("  CHROMIUM "), vec![9]);
        assert_eq!(names(""), vec![7, 9, 11], "an empty box is no filter");
        assert_eq!(names("   "), vec![7, 9, 11], "nor is one holding only spaces");
        assert!(names("firefox").is_empty());
    }
}
