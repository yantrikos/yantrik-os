//! The board: what the companion has been asked to do, and where your request sits in it.
//!
//! # Why this replaces blocking
//!
//! Every call into the companion used to block until it was finished. On a quiet machine that is
//! fine. On a busy one it is not: a tool call arriving while the model is composing an answer
//! waits for the whole generation, and I have measured that at thirty-six to fifty seconds.
//! Worse, the caller learns nothing while it waits — not that it was heard, not that anything is
//! ahead of it, not whether waiting is worth it.
//!
//! So a request is *accepted* rather than *served*. Submitting returns a ticket in about a
//! millisecond, along with the two facts that actually help: how many jobs are ahead of yours, and
//! how busy the lane is. A caller can then wait, do something else, or give up — which is a
//! decision it could not previously make.
//!
//! # Lanes
//!
//! A lane is a queue with its own worker. They exist because a one-millisecond tool call has no
//! business waiting behind a twenty-second generation, and no amount of priority ordering fixes
//! that: you cannot preempt a generation that has already started.
//!
//! The model lane is serialized by necessity — the companion owns the conversation, the bond and
//! the memory, and two generations at once would corrupt all three. Other lanes need not be.
//!
//! # On the estimate
//!
//! Of everything reported here, position and depth are facts and the estimate is not. It is the
//! median of recent jobs of the same kind, and it says so: `basis` names how many samples it came
//! from. Where there is no history it is `null` rather than a number, because an invented estimate
//! is worse than none — a caller can plan around "unknown" and cannot plan around a confident
//! wrong answer.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use serde::Serialize;

/// Jobs kept after they finish, so a caller that comes back late still gets its answer.
///
/// Small on purpose: this holds finished text, and a companion answering all day should not grow
/// a transcript in memory. A caller that waits longer than the last hundred jobs was not waiting.
const REMEMBERED: usize = 100;

/// Samples per kind used for the estimate. Long enough to be a median rather than a coin flip,
/// short enough to follow a machine that has become slower.
const SAMPLES: usize = 20;

/// The longest a caller may park on [`Board::wait`].
pub const MAX_WAIT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// Accepted, not started. This is the state a ticket is born in.
    Queued,
    Running,
    Done,
    Failed,
    /// Cancelled before it started, or asked to stop while running.
    Cancelled,
}

impl State {
    pub fn settled(self) -> bool {
        matches!(self, State::Done | State::Failed | State::Cancelled)
    }
}

struct Job {
    id: String,
    lane: String,
    /// What it is, finely enough to estimate by: `ask`, `recall`, `tool:list_apps`. Two tools with
    /// wildly different costs must not share a median.
    kind: String,
    state: State,
    submitted_at: f64,
    started_at: Option<f64>,
    finished_at: Option<f64>,
    /// Tokens so far. The reason to expose it: a caller watching a long generation can show
    /// progress instead of a spinner, and can decide the answer is already enough.
    partial: String,
    result: Option<String>,
    error: Option<String>,
    cancel: Arc<AtomicBool>,
}

impl Job {
    fn report(&self, ahead: usize, eta: Option<f64>, basis: &str) -> serde_json::Value {
        serde_json::json!({
            "ticket": self.id,
            "lane": self.lane,
            "kind": self.kind,
            "state": self.state,
            "ahead": ahead,
            "waited_seconds": (now() - self.submitted_at * 1.0).max(0.0),
            "ran_for_seconds": match (self.started_at, self.finished_at) {
                (Some(s), Some(f)) => Some(f - s),
                (Some(s), None) => Some(now() - s),
                _ => None,
            },
            "eta_seconds": eta,
            "eta_basis": basis,
            "partial": self.partial,
            "result": self.result,
            "error": self.error,
        })
    }
}

/// How long jobs of one kind have been taking.
#[derive(Default)]
struct Durations {
    samples: VecDeque<f64>,
}

impl Durations {
    fn record(&mut self, seconds: f64) {
        self.samples.push_back(seconds);
        if self.samples.len() > SAMPLES {
            self.samples.pop_front();
        }
    }

    /// The middle sample, or nothing. Median rather than mean because one pathological job — a
    /// package install, a cold model load — would drag a mean for the next twenty requests.
    fn median(&self) -> Option<f64> {
        if self.samples.is_empty() {
            return None;
        }
        let mut sorted: Vec<f64> = self.samples.iter().copied().collect();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        Some(sorted[sorted.len() / 2])
    }
}

#[derive(Default)]
struct Inner {
    jobs: HashMap<String, Job>,
    /// Submitted and not yet started, per lane, in order. The only thing that makes "how many are
    /// ahead of me" answerable.
    queued: HashMap<String, VecDeque<String>>,
    /// Started and not yet settled, per lane.
    running: HashMap<String, Vec<String>>,
    /// Finished ids in order, so the oldest can be dropped.
    settled: VecDeque<String>,
    stats: HashMap<String, Durations>,
    next_id: u64,
}

/// Shared, cloneable. The RPC layer submits and waits; the lane workers start and finish.
#[derive(Clone)]
pub struct Board {
    inner: Arc<(Mutex<Inner>, Condvar)>,
}

impl Default for Board {
    fn default() -> Self {
        Self::new()
    }
}

/// What a caller gets back the moment it asks for something.
#[derive(Debug, Clone, Serialize)]
pub struct Receipt {
    pub ticket: String,
    pub lane: String,
    /// Jobs in this lane submitted before yours and not yet started.
    pub ahead: usize,
    /// Jobs in this lane being worked on right now.
    pub active: usize,
    pub eta_seconds: Option<f64>,
    pub eta_basis: String,
    /// Held so the caller can stop the work without another lookup.
    #[serde(skip)]
    pub cancel: Arc<AtomicBool>,
}

impl Board {
    pub fn new() -> Self {
        Self { inner: Arc::new((Mutex::new(Inner::default()), Condvar::new())) }
    }

    /// Accept a request. Returns immediately; nothing has been done yet.
    pub fn submit(&self, lane: &str, kind: &str) -> Receipt {
        let (lock, cv) = &*self.inner;
        let mut inner = match lock.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };

        inner.next_id += 1;
        let id = format!("j{}", inner.next_id);
        let cancel = Arc::new(AtomicBool::new(false));

        let ahead = inner.queued.get(lane).map(|q| q.len()).unwrap_or(0);
        let active = inner.running.get(lane).map(|r| r.len()).unwrap_or(0);
        let (eta, basis) = inner.estimate(lane, kind, ahead);

        inner.jobs.insert(
            id.clone(),
            Job {
                id: id.clone(),
                lane: lane.to_string(),
                kind: kind.to_string(),
                state: State::Queued,
                submitted_at: now(),
                started_at: None,
                finished_at: None,
                partial: String::new(),
                result: None,
                error: None,
                cancel: cancel.clone(),
            },
        );
        inner.queued.entry(lane.to_string()).or_default().push_back(id.clone());
        drop(inner);
        cv.notify_all();

        Receipt { ticket: id, lane: lane.to_string(), ahead, active, eta_seconds: eta, eta_basis: basis, cancel }
    }

    /// A lane worker has picked this up.
    pub fn start(&self, id: &str) {
        self.with(|inner| {
            let Some(job) = inner.jobs.get_mut(id) else { return };
            if job.state != State::Queued {
                return;
            }
            job.state = State::Running;
            job.started_at = Some(now());
            let lane = job.lane.clone();
            if let Some(q) = inner.queued.get_mut(&lane) {
                q.retain(|queued| queued != id);
            }
            inner.running.entry(lane).or_default().push(id.to_string());
        });
    }

    /// Another token arrived. Cheap: this is called once per token on a generation.
    pub fn progress(&self, id: &str, token: &str, replace: bool) {
        self.with(|inner| {
            let Some(job) = inner.jobs.get_mut(id) else { return };
            if replace {
                job.partial.clear();
            }
            job.partial.push_str(token);
        });
    }

    pub fn finish(&self, id: &str, outcome: Result<String, String>) {
        self.with(|inner| {
            let Some(job) = inner.jobs.get_mut(id) else { return };
            job.finished_at = Some(now());
            let kind = job.kind.clone();
            let lane = job.lane.clone();
            let ran = job.started_at.map(|s| now() - s);

            match outcome {
                Ok(text) => {
                    // A job cancelled mid-flight that finished anyway is reported as cancelled:
                    // the caller asked for it to stop and should not be handed a result it may
                    // already have decided not to use.
                    job.state = if job.cancel.load(Ordering::Relaxed) {
                        State::Cancelled
                    } else {
                        State::Done
                    };
                    job.result = Some(text);
                }
                Err(e) => {
                    job.state = State::Failed;
                    job.error = Some(e);
                }
            }

            if let Some(r) = inner.running.get_mut(&lane) {
                r.retain(|running| running != id);
            }
            if let Some(q) = inner.queued.get_mut(&lane) {
                q.retain(|queued| queued != id);
            }

            // Only successful runs teach the estimate. A failure is usually fast and would make
            // the median optimistic for everything queued behind it.
            if let (Some(seconds), State::Done) = (ran, inner.jobs[id].state) {
                inner.stats.entry(kind).or_default().record(seconds);
            }

            inner.settled.push_back(id.to_string());
            while inner.settled.len() > REMEMBERED {
                if let Some(old) = inner.settled.pop_front() {
                    inner.jobs.remove(&old);
                }
            }
        });
    }

    /// Ask for a job to stop. Returns what it was doing when asked.
    ///
    /// A queued job is cancelled outright. A running one is *asked*: the flag is set and the lane
    /// checks it between tokens, because there is no safe way to tear a generation out from under
    /// the companion mid-write.
    pub fn cancel(&self, id: &str) -> Option<State> {
        let mut was = None;
        self.with(|inner| {
            let Some(job) = inner.jobs.get_mut(id) else { return };
            was = Some(job.state);
            job.cancel.store(true, Ordering::Relaxed);
            if job.state == State::Queued {
                job.state = State::Cancelled;
                job.finished_at = Some(now());
                let lane = job.lane.clone();
                if let Some(q) = inner.queued.get_mut(&lane) {
                    q.retain(|queued| queued != id);
                }
                inner.settled.push_back(id.to_string());
            }
        });
        was
    }

    /// Where a job stands, waiting up to `wait` for it to change.
    ///
    /// Woken by any board change rather than by this job specifically: a job moving from third in
    /// the queue to second has not changed state, but it is exactly what a caller watching its
    /// position wants to hear about.
    pub fn wait(&self, id: &str, wait: Duration) -> Option<serde_json::Value> {
        let (lock, cv) = &*self.inner;
        let mut inner = match lock.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };

        let before = inner.jobs.get(id).map(|j| (j.state, j.partial.len(), inner.ahead_of(j)))?;
        if !before.0.settled() && !wait.is_zero() {
            let (guard, _) = cv
                .wait_timeout_while(inner, wait.min(MAX_WAIT), |i| {
                    match i.jobs.get(id) {
                        Some(j) => {
                            let nowish = (j.state, j.partial.len(), i.ahead_of(j));
                            nowish == before
                        }
                        // Aged out of the board while we waited; stop rather than spin.
                        None => false,
                    }
                })
                .unwrap_or_else(|e| e.into_inner());
            inner = guard;
        }

        let job = inner.jobs.get(id)?;
        let ahead = inner.ahead_of(job);
        let (eta, basis) = inner.estimate(&job.lane, &job.kind, ahead);
        Some(job.report(ahead, eta, &basis))
    }

    /// Everything at once: how each lane is doing, and what has been happening.
    pub fn overview(&self) -> serde_json::Value {
        let (lock, _) = &*self.inner;
        let inner = match lock.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };

        let mut lanes: Vec<serde_json::Value> = Vec::new();
        let mut names: Vec<&String> = inner.queued.keys().chain(inner.running.keys()).collect();
        names.sort();
        names.dedup();
        for lane in names {
            let queued = inner.queued.get(lane).map(|q| q.len()).unwrap_or(0);
            let running: Vec<&Job> = inner
                .running
                .get(lane)
                .map(|ids| ids.iter().filter_map(|i| inner.jobs.get(i)).collect())
                .unwrap_or_default();
            lanes.push(serde_json::json!({
                "lane": lane,
                "queued": queued,
                "active": running.len(),
                "working_on": running.iter().map(|j| serde_json::json!({
                    "ticket": j.id,
                    "kind": j.kind,
                    "ran_for_seconds": j.started_at.map(|s| now() - s),
                })).collect::<Vec<_>>(),
            }));
        }

        let mut kinds: Vec<serde_json::Value> = inner
            .stats
            .iter()
            .filter_map(|(kind, d)| {
                d.median().map(|m| {
                    serde_json::json!({
                        "kind": kind,
                        "median_seconds": (m * 100.0).round() / 100.0,
                        "samples": d.samples.len(),
                    })
                })
            })
            .collect();
        kinds.sort_by(|a, b| a["kind"].as_str().cmp(&b["kind"].as_str()));

        serde_json::json!({ "lanes": lanes, "typical": kinds, "remembered": inner.jobs.len() })
    }

    fn with<T>(&self, f: impl FnOnce(&mut Inner) -> T) -> T {
        let (lock, cv) = &*self.inner;
        let mut inner = match lock.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        let out = f(&mut inner);
        drop(inner);
        // Any change can move somebody's position, so everyone waiting is woken to re-check.
        cv.notify_all();
        out
    }
}

impl Inner {
    fn ahead_of(&self, job: &Job) -> usize {
        if job.state != State::Queued {
            return 0;
        }
        self.queued
            .get(&job.lane)
            .map(|q| q.iter().take_while(|id| **id != job.id).count())
            .unwrap_or(0)
    }

    /// How long until this is finished, and what that estimate rests on.
    fn estimate(&self, lane: &str, kind: &str, ahead: usize) -> (Option<f64>, String) {
        let own = self.stats.get(kind).and_then(|d| d.median());
        let Some(own) = own else {
            return (None, format!("no completed {kind} yet"));
        };

        // Everything in front, at its own kind's median rather than this one's — a tool call
        // queued behind a generation should be told about the generation's twenty seconds.
        let queue_cost: f64 = self
            .queued
            .get(lane)
            .map(|q| {
                q.iter()
                    .take(ahead)
                    .filter_map(|id| self.jobs.get(id))
                    .filter_map(|j| self.stats.get(&j.kind).and_then(|d| d.median()))
                    .sum()
            })
            .unwrap_or(0.0);

        // Whatever is already running, counted at its remaining median. Not perfect — a job that
        // has already run past its median has an unknowable remainder — but ignoring it entirely
        // would tell a caller "one second" while a generation is mid-sentence in front of it.
        let running_cost: f64 = self
            .running
            .get(lane)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| self.jobs.get(id))
                    .filter_map(|j| {
                        let median = self.stats.get(&j.kind).and_then(|d| d.median())?;
                        let elapsed = j.started_at.map(|s| now() - s).unwrap_or(0.0);
                        Some((median - elapsed).max(0.0))
                    })
                    .sum()
            })
            .unwrap_or(0.0);

        let samples = self.stats.get(kind).map(|d| d.samples.len()).unwrap_or(0);
        let total = own + queue_cost + running_cost;
        (
            Some((total * 10.0).round() / 10.0),
            format!("median of the last {samples} {kind}, plus {ahead} ahead and what is running"),
        )
    }
}

fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_submission_is_answered_immediately_and_says_where_it_stands() {
        let board = Board::new();
        let first = board.submit("model", "ask");
        assert_eq!(first.ahead, 0);
        assert_eq!(first.active, 0);

        let second = board.submit("model", "ask");
        assert_eq!(second.ahead, 1, "the second caller must be told someone is in front");
        assert_ne!(first.ticket, second.ticket);
    }

    #[test]
    fn lanes_do_not_see_each_other() {
        // The whole reason lanes exist: a tool call must not be told it is behind a generation.
        let board = Board::new();
        board.submit("model", "ask");
        board.submit("model", "ask");
        let tool = board.submit("tools", "tool:list_apps");
        assert_eq!(tool.ahead, 0);
    }

    #[test]
    fn position_moves_as_the_queue_drains() {
        let board = Board::new();
        let a = board.submit("model", "ask");
        let b = board.submit("model", "ask");
        assert_eq!(b.ahead, 1);

        board.start(&a.ticket);
        let status = board.wait(&b.ticket, Duration::ZERO).unwrap();
        assert_eq!(status["ahead"], 0, "once the job in front starts, nothing is queued ahead");
        assert_eq!(status["state"], "queued");
    }

    #[test]
    fn an_estimate_is_absent_rather_than_invented() {
        let board = Board::new();
        let first = board.submit("model", "ask");
        assert!(first.eta_seconds.is_none(), "nothing has ever run; there is nothing to estimate from");
        assert!(first.eta_basis.contains("no completed"), "{}", first.eta_basis);
    }

    #[test]
    fn an_estimate_appears_once_there_is_history_and_says_what_it_rests_on() {
        let board = Board::new();
        for _ in 0..3 {
            let t = board.submit("model", "ask");
            board.start(&t.ticket);
            // Backdate the start so the recorded duration is not zero.
            board.with(|inner| {
                if let Some(job) = inner.jobs.get_mut(&t.ticket) {
                    job.started_at = Some(now() - 2.0);
                }
            });
            board.finish(&t.ticket, Ok("done".into()));
        }
        let next = board.submit("model", "ask");
        let eta = next.eta_seconds.expect("three completed asks is a basis");
        assert!((1.0..4.0).contains(&eta), "roughly two seconds, got {eta}");
        assert!(next.eta_basis.contains("3 ask"), "{}", next.eta_basis);
    }

    #[test]
    fn a_failure_does_not_teach_the_estimate() {
        // Failures are usually instant, and letting them into the median would promise every
        // queued caller a speed the lane cannot deliver.
        let board = Board::new();
        let t = board.submit("model", "ask");
        board.start(&t.ticket);
        board.finish(&t.ticket, Err("backend unreachable".into()));

        let next = board.submit("model", "ask");
        assert!(next.eta_seconds.is_none());
    }

    #[test]
    fn a_waiting_caller_is_woken_when_its_job_finishes() {
        let board = Board::new();
        let t = board.submit("model", "ask");
        let worker = board.clone();
        let ticket = t.ticket.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(80));
            worker.start(&ticket);
            worker.finish(&ticket, Ok("the answer".into()));
        });

        let started = std::time::Instant::now();
        let status = board.wait(&t.ticket, Duration::from_secs(5)).unwrap();
        assert!(started.elapsed() < Duration::from_secs(2), "should wake on the change");
        assert_eq!(status["state"], "done");
        assert_eq!(status["result"], "the answer");
    }

    #[test]
    fn partial_text_is_visible_before_the_answer_is() {
        let board = Board::new();
        let t = board.submit("model", "ask");
        board.start(&t.ticket);
        board.progress(&t.ticket, "Once upon", false);
        board.progress(&t.ticket, " a time", false);

        let status = board.wait(&t.ticket, Duration::ZERO).unwrap();
        assert_eq!(status["state"], "running");
        assert_eq!(status["partial"], "Once upon a time");
        assert!(status["result"].is_null(), "there is no finished answer yet");
    }

    #[test]
    fn a_replacement_token_supersedes_what_came_before() {
        // The worker's `__REPLACE__` sentinel: an error mid-stream discards the draft.
        let board = Board::new();
        let t = board.submit("model", "ask");
        board.start(&t.ticket);
        board.progress(&t.ticket, "half an answer", false);
        board.progress(&t.ticket, "the backend went away", true);

        let status = board.wait(&t.ticket, Duration::ZERO).unwrap();
        assert_eq!(status["partial"], "the backend went away");
    }

    #[test]
    fn a_queued_job_can_be_cancelled_outright() {
        let board = Board::new();
        let a = board.submit("model", "ask");
        let b = board.submit("model", "ask");
        board.start(&a.ticket);

        assert_eq!(board.cancel(&b.ticket), Some(State::Queued));
        let status = board.wait(&b.ticket, Duration::ZERO).unwrap();
        assert_eq!(status["state"], "cancelled");
    }

    #[test]
    fn a_running_job_that_was_asked_to_stop_is_not_reported_as_done() {
        // It may finish anyway — a generation cannot be torn out mid-write — but the caller asked
        // for it to stop and should not be handed a result it may have moved on from.
        let board = Board::new();
        let t = board.submit("model", "ask");
        board.start(&t.ticket);
        board.cancel(&t.ticket);
        board.finish(&t.ticket, Ok("finished anyway".into()));

        let status = board.wait(&t.ticket, Duration::ZERO).unwrap();
        assert_eq!(status["state"], "cancelled");
    }

    #[test]
    fn the_overview_reports_each_lane_separately() {
        let board = Board::new();
        let a = board.submit("model", "ask");
        board.start(&a.ticket);
        board.submit("model", "ask");
        board.submit("tools", "tool:list_apps");

        let view = board.overview();
        let lanes = view["lanes"].as_array().unwrap();
        let model = lanes.iter().find(|l| l["lane"] == "model").unwrap();
        assert_eq!(model["active"], 1);
        assert_eq!(model["queued"], 1);
        let tools = lanes.iter().find(|l| l["lane"] == "tools").unwrap();
        assert_eq!(tools["active"], 0);
        assert_eq!(tools["queued"], 1);
    }

    #[test]
    fn the_board_does_not_grow_without_bound() {
        // It holds finished answers. A companion working all day must not accumulate a transcript.
        let board = Board::new();
        for _ in 0..(REMEMBERED + 50) {
            let t = board.submit("tools", "tool:noop");
            board.start(&t.ticket);
            board.finish(&t.ticket, Ok("x".repeat(100)));
        }
        let view = board.overview();
        assert!(view["remembered"].as_u64().unwrap() <= REMEMBERED as u64 + 1);
    }
}
