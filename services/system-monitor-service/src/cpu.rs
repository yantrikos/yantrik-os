//! CPU use between two reads of /proc/stat.
//!
//! The counters in /proc/stat are totals since boot, and this service divided one read of them
//! by itself — so "CPU 8%" was the machine's average since it booted, and read 8% through a
//! minute at 100%. A share of CPU is a fact about an interval, so it is now the busy time over
//! the total time between the last two reads, and it comes with the interval it covers. With
//! only one read there is no interval, and so no figure.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// One `cpu` line's counters: time spent idle (idle + iowait) and in total, in ticks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Times {
    pub idle: u64,
    pub total: u64,
}

/// The aggregate line first, then one per core, in file order.
pub fn parse_stat(content: &str) -> Vec<Times> {
    content
        .lines()
        .filter(|l| l.starts_with("cpu"))
        .filter_map(|line| {
            let values: Vec<u64> = line.split_whitespace().skip(1).filter_map(|s| s.parse().ok()).collect();
            if values.len() < 4 {
                return None;
            }
            // user nice system idle iowait irq softirq steal; guest and guest_nice are already
            // counted inside user and nice, so adding them would count them twice.
            let counted = &values[..values.len().min(8)];
            Some(Times { idle: values[3] + values.get(4).copied().unwrap_or(0), total: counted.iter().sum() })
        })
        .collect()
}

/// The busy share of the time between two reads; `None` when no time passed between them.
pub fn busy_percent(before: Times, after: Times) -> Option<f64> {
    let total = after.total.checked_sub(before.total).filter(|t| *t > 0)?;
    let idle = after.idle.saturating_sub(before.idle).min(total);
    Some((total - idle) as f64 / total as f64 * 100.0)
}

/// CPU use over one interval.
#[derive(Clone, Debug, PartialEq)]
pub struct Delta {
    pub overall: f64,
    pub cores: Vec<f64>,
    pub window: Duration,
}

/// Reads closer together than this are not a measurement: the kernel counts in 10 ms ticks, so
/// a few milliseconds' difference is mostly rounding. A caller arriving that soon after another
/// (the window's poll and a `describe` together) is given the last interval instead.
const MIN_WINDOW: Duration = Duration::from_millis(250);

/// The last read, and the last interval measured from it.
#[derive(Default)]
pub struct Sampler {
    previous: Option<(Instant, Vec<Times>)>,
    last: Option<Delta>,
}

impl Sampler {
    /// Take a read made at `now`. `None` until there have been two reads far enough apart.
    pub fn read(&mut self, now: Instant, times: Vec<Times>) -> Option<Delta> {
        let Some((at, before)) = self.previous.as_ref() else {
            self.previous = Some((now, times));
            return None;
        };
        let window = now.saturating_duration_since(*at);
        if window < MIN_WINDOW {
            return self.last.clone();
        }
        let mut shares = before.iter().zip(&times).map(|(b, a)| busy_percent(*b, *a));
        let delta = shares.next().flatten().map(|overall| Delta {
            overall,
            cores: shares.map(|s| s.unwrap_or(0.0)).collect(),
            window,
        });
        self.previous = Some((now, times));
        if delta.is_some() {
            self.last = delta.clone();
        }
        delta.or_else(|| self.last.clone())
    }
}

static SAMPLER: Mutex<Option<Sampler>> = Mutex::new(None);

/// Measure a read of /proc/stat taken now against the read before it.
pub fn measure(times: Vec<Times>) -> Option<Delta> {
    let mut sampler = SAMPLER.lock().ok()?;
    sampler.get_or_insert_with(Sampler::default).read(Instant::now(), times)
}

#[cfg(test)]
mod tests {
    use super::*;

    // user nice system idle iowait irq softirq steal guest guest_nice
    const BEFORE: &str = "cpu  1000 0 500 8000 500 0 0 0 0 0\ncpu0 500 0 250 4000 250 0 0 0 0 0\ncpu1 500 0 250 4000 250 0 0 0 0 0\n";
    // 200 more ticks: 150 busy (user 100, system 50), 50 idle — and core 1 did all of it.
    const AFTER: &str = "cpu  1100 0 550 8040 510 0 0 0 0 0\ncpu0 500 0 250 4040 260 0 0 0 0 0\ncpu1 600 0 300 4000 250 0 0 0 0 0\n";

    #[test]
    fn two_reads_give_the_busy_share_of_the_time_between_them() {
        let before = parse_stat(BEFORE);
        let after = parse_stat(AFTER);
        assert_eq!(before[0], Times { idle: 8500, total: 10000 });
        assert_eq!(busy_percent(before[0], after[0]), Some(75.0));
        assert_eq!(busy_percent(before[1], after[1]), Some(0.0));
        assert_eq!(busy_percent(before[2], after[2]), Some(100.0));
    }

    #[test]
    fn no_figure_before_the_second_read() {
        let mut sampler = Sampler::default();
        let start = Instant::now();
        assert_eq!(sampler.read(start, parse_stat(BEFORE)), None, "one read is a since-boot total, not a share");
        let delta = sampler.read(start + Duration::from_secs(2), parse_stat(AFTER)).expect("two reads are an interval");
        assert_eq!(delta.overall, 75.0);
        assert_eq!(delta.cores, vec![0.0, 100.0]);
        assert_eq!(delta.window, Duration::from_secs(2));
    }

    #[test]
    fn a_read_too_soon_after_the_last_repeats_the_last_interval() {
        let mut sampler = Sampler::default();
        let start = Instant::now();
        sampler.read(start, parse_stat(BEFORE));
        let measured = sampler.read(start + Duration::from_secs(2), parse_stat(AFTER));
        let again = sampler.read(start + Duration::from_millis(2010), parse_stat(AFTER));
        assert_eq!(again, measured);
    }

    #[test]
    fn counters_that_did_not_move_are_no_measurement() {
        let t = Times { idle: 5, total: 10 };
        assert_eq!(busy_percent(t, t), None);
    }
}
