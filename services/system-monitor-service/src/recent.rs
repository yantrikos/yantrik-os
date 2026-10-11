//! The last process sample, so `describe` answers without taking one.
//!
//! A process's CPU share is a fact about an interval, and the table's interval is half a second
//! (`CPU_SAMPLE_WINDOW`). `describe` took a fresh sample on every call, so every read of this
//! surface slept for that half second and `yos check` timed it at about 545 ms — over the 500 ms
//! a caller should wait for a read. The window polls `sysmon.processes` every two seconds while it
//! is open, so a sample is usually a moment old already; `describe` answers from it and says how
//! old it is, and only a caller with no recent sample at all waits for one.
//!
//! The keeping, the refresh behind the answer and the bounded wait are the service SDK's
//! (`yantrik_service_sdk::recent`), shared with the weather service; what is here is what this
//! service keeps and for how long.

use std::sync::OnceLock;
use std::time::Duration;

use yantrik_ipc_contracts::system_monitor::ProcessInfo;
use yantrik_service_sdk::recent::{Policy, Reading, Recent};

/// How many of the busiest processes `describe` names.
pub const SHOWN: usize = 5;

const POLICY: Policy = Policy {
    // A sample this young is as current as a new one would be: the window samples this often.
    current_for: Duration::from_secs(2),
    // Past this, a sample is no longer "what the machine is doing", and the caller waits for a
    // new one rather than being told about a machine as it was.
    usable_for: Duration::from_secs(30),
    // A sample takes half a second; this bounds the wait if one ever takes far longer.
    wait: Duration::from_secs(2),
    // Reading /proc does not fail in a way that waiting would cure.
    retry_after: Duration::ZERO,
};

fn recent() -> &'static Recent<(), Vec<ProcessInfo>> {
    static RECENT: OnceLock<Recent<(), Vec<ProcessInfo>>> = OnceLock::new();
    RECENT.get_or_init(|| {
        Recent::new(POLICY, |_| {
            crate::read_processes("cpu", SHOWN as u32).map(|procs| busiest_of(&procs)).map_err(|e| e.message)
        })
    })
}

/// The busiest few of a full sample, whatever order its caller asked for.
fn busiest_of(procs: &[ProcessInfo]) -> Vec<ProcessInfo> {
    let mut top = procs.to_vec();
    top.sort_by(|a, b| b.cpu_percent.total_cmp(&a.cpu_percent));
    top.truncate(SHOWN);
    top
}

/// Keep the busiest few of a sample a caller just took.
pub fn remember(procs: &[ProcessInfo]) {
    recent().put((), busiest_of(procs));
}

/// The busiest processes, and how old the sample they come from is.
pub fn busiest() -> (Vec<ProcessInfo>, Duration) {
    match recent().read(&()) {
        Reading::Known { value, age, .. } => (value, age),
        Reading::Fetching | Reading::Failed { .. } => (Vec::new(), Duration::ZERO),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proc(name: &str, cpu: f64) -> ProcessInfo {
        ProcessInfo {
            pid: 1,
            name: name.to_string(),
            cpu_percent: cpu,
            mem_percent: 0.0,
            mem_bytes: 0,
            state: "R".to_string(),
            user: "root".to_string(),
        }
    }

    #[test]
    fn the_busiest_five_are_kept_busiest_first() {
        let procs: Vec<ProcessInfo> =
            [1.0, 9.0, 3.0, 7.0, 5.0, 2.0, 8.0].iter().map(|c| proc(&format!("p{c}"), *c)).collect();
        let top = busiest_of(&procs);
        let shares: Vec<f64> = top.iter().map(|p| p.cpu_percent).collect();
        assert_eq!(shares, vec![9.0, 8.0, 7.0, 5.0, 3.0]);
    }

    #[test]
    fn a_sample_just_taken_is_what_describe_is_given() {
        remember(&[proc("busy", 42.0)]);
        let (top, age) = busiest();
        assert_eq!(top[0].name, "busy");
        assert!(age < POLICY.current_for);
    }
}
