//! Reach the companion from an app.
//!
//! The LLM, the memory and the bond live in the shell process. An app talks to them the same way
//! it talks to any service — over the socket bus — so an AI action is a normal RPC call and not
//! a stub.
//!
//! Every call answers `Err` when the shell is not running, which is a state an app must handle
//! rather than hide: these binaries are meant to run on their own too.

use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use yantrik_ipc_transport::SyncRpcClient;

/// How long to wait for an answer. A local model on a cold cache is slow, and a caller that
/// gives up early leaves the user with nothing.
const ASK_TIMEOUT: Duration = Duration::from_secs(90);

fn client() -> SyncRpcClient {
    SyncRpcClient::for_service("companion").with_timeout(ASK_TIMEOUT)
}

/// Why an ask did not produce an answer.
///
/// One type and one sentence per case, because a dozen apps used to wrap a raw string in a
/// dozen different sentences of their own — and three of them ended with "Is the Yantrik shell
/// running?", which is the wrong question when the shell is running and just said it has no
/// model.
#[derive(Debug)]
pub enum AskError {
    /// The shell answered, and no model did — none set up, or the one set up did not answer.
    /// Its canned fallback text stays inside the shell: show [`NO_MODEL_HINT`], and write
    /// nothing into a document, a note or a draft.
    NoModel,
    /// No answer at all: no shell, a timeout, or a refusal.
    Failed(String),
}

impl std::fmt::Display for AskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AskError::NoModel => f.write_str(NO_MODEL_HINT),
            AskError::Failed(reason) => write!(f, "The companion did not answer: {reason}"),
        }
    }
}

impl AskError {
    /// What the agent rail should say after this failure.
    ///
    /// A shell that answered "no model" is running, and telling the person to start it is the
    /// wrong hint — which is the one this app-facing sentence can still get wrong.
    pub fn hint(&self) -> &'static str {
        match self {
            AskError::NoModel => NO_MODEL_HINT,
            AskError::Failed(_) => OFFLINE_HINT,
        }
    }
}

/// Ask the companion something and get the finished answer.
///
/// `Ok` is always a model's words: a turn the shell's offline responder served comes back as
/// [`AskError::NoModel`], never as the canned text it produced.
pub fn ask(prompt: &str) -> Result<String, AskError> {
    let response = client()
        .call(
            "companion.ask",
            serde_json::json!({ "prompt": prompt, "timeout_ms": ASK_TIMEOUT.as_millis() as u64 }),
        )
        .map_err(|e| ask_error(e.code, &e.message))?;
    response
        .get("text")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| AskError::Failed("companion returned no text".to_string()))
}

/// The wire error to the one typed error, split out so a test can pin the mapping with no
/// socket and no shell.
fn ask_error(code: i32, message: &str) -> AskError {
    if code == yantrik_ipc_contracts::ERR_NO_MODEL {
        AskError::NoModel
    } else {
        AskError::Failed(format!("[{code}] {message}"))
    }
}

/// One memory the companion recalled.
#[derive(Debug, Clone)]
pub struct Recalled {
    pub rid: String,
    pub text: String,
    pub score: f64,
}

/// Search the companion's memory.
pub fn recall(query: &str, limit: usize) -> Result<Vec<Recalled>, String> {
    let response = client()
        .call("companion.recall", serde_json::json!({ "query": query, "limit": limit }))
        .map_err(|e| format!("[{}] {}", e.code, e.message))?;
    let rows = response
        .get("results")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "companion returned no results".to_string())?;
    Ok(rows
        .iter()
        .map(|r| Recalled {
            rid: r.get("rid").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            text: r.get("text").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            score: r.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0),
        })
        .collect())
}

/// Recall, filtered to what is actually relevant.
///
/// `recall` returns its best `limit` results, and "best" is not "relevant": on a fresh machine
/// the best match for a note about quarterly planning was the companion's own telemetry --
/// "App opened: yantrik-notes", scoring 9%. An agent rail that shows that is not surfacing
/// context, it is surfacing noise with a number on it, and one junk row costs more trust than
/// three good rows earn.
///
/// So every caller that puts recall results in front of a person goes through here. Over-fetch,
/// filter, then take: asking for `want` directly and filtering afterwards leaves you with two
/// rows when three were available.
pub fn recall_relevant(query: &str, floor: f64, want: usize) -> Vec<Recalled> {
    recall(query, (want * 3).max(6))
        .unwrap_or_default()
        .into_iter()
        .filter(|m| m.score >= floor)
        .take(want)
        .collect()
}

/// The floor every app uses, so "relevant" means the same thing in all of them.
pub const RELEVANCE_FLOOR: f64 = 0.35;

/// What to tell someone when the shell is not there.
///
/// One sentence, said once at the top of the rail rather than by every row failing separately.
/// Running an app on its own is a supported thing to do, not an error.
pub const OFFLINE_HINT: &str = "Not connected. Start the Yantrik shell for memory and suggestions.";

/// What to tell someone when the shell is there and no model answered.
///
/// Its own sentence because "start the Yantrik shell" — what every AI control said whenever the
/// backend had not answered — sends the person to start the one thing that is already running.
/// It says only what is known, that nothing answered: the machine that reported the bug had a
/// model set up (Ollama on the host) that was not running, so "no model is set up" — what this
/// said first — told that person something false.
pub const NO_MODEL_HINT: &str = "No AI model answered. Check Settings \u{2192} AI.";

/// What to tell someone when an AI control exists in the markup but nothing is built behind it.
///
/// Shelved apps used to open the panel and leave it empty — the handlers wrote a log line and
/// returned — so a press looked like a request in flight that never landed. One sentence shared
/// by every shelved app, because "not yet" should mean the same thing everywhere it appears.
pub const NOT_BUILT_YET: &str = "Not available yet — the AI in this app is still being built.";

/// Run one of the companion's tools by name, without a model in the loop.
///
/// The companion carries ~178 of them — files, windows, browser, containers, packages — behind
/// the permission ceiling in its config. An app that wants one thing done should ask for that
/// thing rather than describe it in a prompt and hope: this is a function call, and it works
/// when the model is unavailable.
///
/// The result is the tool's own prose, which is what tools return; a caller wanting structure
/// parses it.
pub fn tool(name: &str, args: serde_json::Value) -> Result<String, String> {
    let response = client()
        .call(
            "companion.tool",
            serde_json::json!({
                "name": name,
                "args": args,
                "timeout_ms": ASK_TIMEOUT.as_millis() as u64,
            }),
        )
        .map_err(|e| format!("[{}] {}", e.code, e.message))?;
    response
        .get("result")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "companion returned no result".to_string())
}

/// Whether the shell can answer at all, and when it cannot, which of two different situations
/// the person is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// The shell is running and its model answered the last time it was asked.
    Ready,
    /// The shell is running and no model answered — none set up, or the one set up did not
    /// answer and the last ask fell back to canned text.
    NoModel,
    /// No shell is running to ask.
    NoShell,
}

impl Reach {
    /// The one sentence to show in place of an AI action, or `None` when the action can go
    /// ahead.
    pub fn hint(self) -> Option<&'static str> {
        match self {
            Reach::Ready => None,
            Reach::NoModel => Some(NO_MODEL_HINT),
            Reach::NoShell => Some(OFFLINE_HINT),
        }
    }
}

/// Which situation this is, as last heard from the shell — without waiting on it.
///
/// Asked before offering an AI action or saying why one cannot run. It replaced `is_online`, a
/// bool that could not tell "no shell" from "shell with no model".
///
/// It answers from memory. A worker thread asks the shell (`companion.status`) every
/// [`REACH_EVERY`] while anybody in this process keeps asking, and this reads what it last heard.
///
/// It used to ask the shell itself, on whatever thread called it, and the thread that calls it is
/// the window's: every app's agent rail runs it on a four-second Slint timer. A control surface's
/// `describe` runs on that same thread (see `control`), so it queued behind a socket round trip
/// to another process with a ninety-second ceiling, and `yos check weather` measured describe
/// at ~600 ms. Reading a kept answer costs nothing, so neither the rail nor a describe waits on
/// the shell now.
///
/// The one wait left is bounded: the call that starts the watcher — the first in a process, or
/// the first after nobody asked for [`REACH_IDLE`] — waits up to [`REACH_FIRST_WAIT`] for its
/// first answer, so a rail filled at startup says the right thing rather than a guess. Past that
/// it answers with what it last knew, or [`Reach::NoShell`] when it has never heard anything.
pub fn reach() -> Reach {
    static WATCH: OnceLock<Arc<ReachWatch>> = OnceLock::new();
    WATCH
        .get_or_init(|| {
            ReachWatch::new(Box::new(probe_reach), REACH_EVERY, REACH_FIRST_WAIT, REACH_IDLE)
        })
        .get()
}

/// How often the watcher asks the shell while somebody is listening. Under the rails' own four
/// seconds, so a rail is never more than one of its own ticks behind the shell.
pub const REACH_EVERY: Duration = Duration::from_secs(2);

/// The longest the watcher's first answer is waited for. A shell that is there answers a status
/// in a millisecond or two; this is a ceiling, not an expected cost.
pub const REACH_FIRST_WAIT: Duration = Duration::from_millis(250);

/// How long the watcher keeps asking after the last time anybody wanted the answer, so an app
/// that asks only on a button press does not keep a thread polling the shell all day.
pub const REACH_IDLE: Duration = Duration::from_secs(30);

/// Ask the shell, once, and wait for it. Only the watcher thread calls this.
fn probe_reach() -> Reach {
    reach_of(client().call("companion.status", serde_json::json!({})))
}

/// The last answer about [`Reach`], kept fresh by a worker thread.
///
/// Its own type with the probe passed in, so a test can stand a slow shell behind it and show
/// that asking does not wait on the shell.
pub(crate) struct ReachWatch {
    probe: Box<dyn Fn() -> Reach + Send + Sync>,
    every: Duration,
    first_wait: Duration,
    idle: Duration,
    state: Mutex<Watched>,
    landed: Condvar,
}

struct Watched {
    /// What the shell said last; `None` until it first says anything.
    last: Option<Reach>,
    /// How many answers have landed, so a wait can tell a new one from the one it already had.
    landings: u64,
    /// When somebody last asked. The watcher stops once nobody has for `idle`.
    asked: Instant,
    /// Whether a watcher thread is alive.
    running: bool,
}

impl ReachWatch {
    pub(crate) fn new(
        probe: Box<dyn Fn() -> Reach + Send + Sync>,
        every: Duration,
        first_wait: Duration,
        idle: Duration,
    ) -> Arc<Self> {
        Arc::new(ReachWatch {
            probe,
            every,
            first_wait,
            idle,
            state: Mutex::new(Watched {
                last: None,
                landings: 0,
                asked: Instant::now(),
                running: false,
            }),
            landed: Condvar::new(),
        })
    }

    /// The last answer; see [`reach`].
    pub(crate) fn get(self: &Arc<Self>) -> Reach {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.asked = Instant::now();
        if s.running {
            // The watcher is alive, so what it last heard is at most one probe old. When the
            // probe it is in is slow, that is the watcher's wait, never this caller's.
            return s.last.unwrap_or(Reach::NoShell);
        }
        s.running = true;
        let watcher = Arc::clone(self);
        if std::thread::Builder::new()
            .name("companion-reach".into())
            .spawn(move || watcher.watch())
            .is_err()
        {
            // No thread to ask with. Asking inline would put the wait back on the caller, so say
            // what was known and let the next call try again.
            s.running = false;
            return s.last.unwrap_or(Reach::NoShell);
        }
        let seen = s.landings;
        let (s, _) = self
            .landed
            .wait_timeout_while(s, self.first_wait, |s| s.landings == seen)
            .unwrap_or_else(|e| e.into_inner());
        s.last.unwrap_or(Reach::NoShell)
    }

    /// The watcher thread: ask, keep the answer, sleep — until nobody is asking any more.
    fn watch(self: Arc<Self>) {
        loop {
            let reach = (self.probe)();
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            s.last = Some(reach);
            s.landings += 1;
            self.landed.notify_all();
            if s.asked.elapsed() >= self.idle {
                s.running = false;
                return;
            }
            drop(s);
            std::thread::sleep(self.every);
        }
    }
}

/// The reading of a status reply, split out so a test can pin the distinction with no socket
/// and no shell.
fn reach_of(status: Result<serde_json::Value, yantrik_ipc_transport::RpcError>) -> Reach {
    match status {
        // A reply at all is the shell running; the flag inside says whether its model answered
        // last time.
        Ok(v) if v.get("online").and_then(|o| o.as_bool()) == Some(true) => Reach::Ready,
        Ok(_) => Reach::NoModel,
        Err(_) => Reach::NoShell,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rpc_error(code: i32) -> yantrik_ipc_transport::RpcError {
        yantrik_ipc_transport::RpcError { code, message: "no".into(), data: None }
    }

    /// The core of the defect: a no-model refusal used to arrive as an ordinary string, and
    /// every app showed it — or something wrapped around it — as the answer.
    #[test]
    fn the_no_model_code_maps_to_the_one_sentence() {
        let e = ask_error(yantrik_ipc_contracts::ERR_NO_MODEL, "no AI model is set up");
        assert!(matches!(e, AskError::NoModel));
        assert_eq!(e.to_string(), "No AI model answered. Check Settings \u{2192} AI.");
        assert_eq!(e.hint(), NO_MODEL_HINT);
    }

    #[test]
    fn any_other_failure_keeps_its_reason_and_the_old_hint() {
        let e = ask_error(-32000, "companion timed out");
        assert_eq!(e.to_string(), "The companion did not answer: [-32000] companion timed out");
        assert_eq!(e.hint(), OFFLINE_HINT);
    }

    /// A shell that answers "not online" is running: the hint must not send the person to
    /// start it. That wrong sentence is what every AI control showed after the first offline
    /// reply flipped the flag.
    #[test]
    fn reach_tells_a_missing_shell_apart_from_a_shell_with_no_model() {
        assert_eq!(reach_of(Ok(serde_json::json!({ "online": true }))), Reach::Ready);
        assert_eq!(reach_of(Ok(serde_json::json!({ "online": false }))), Reach::NoModel);
        assert_eq!(reach_of(Err(rpc_error(-1))), Reach::NoShell);

        assert_eq!(Reach::Ready.hint(), None);
        assert_eq!(Reach::NoModel.hint(), Some(NO_MODEL_HINT));
        assert_eq!(Reach::NoShell.hint(), Some(OFFLINE_HINT));
    }

    /// A probe that takes `ms` and then answers `reach`, counting how often it was asked.
    fn slow_probe(
        ms: u64,
        reach: Reach,
        asked: Arc<std::sync::atomic::AtomicUsize>,
    ) -> Box<dyn Fn() -> Reach + Send + Sync> {
        Box::new(move || {
            asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(ms));
            reach
        })
    }

    /// The defect: the rail asked the shell on the window's thread, and a describe queued behind
    /// the round trip. Asking now costs a bounded first wait and nothing after it, however slow
    /// the shell is.
    #[test]
    fn asking_does_not_wait_on_a_slow_shell() {
        let asked = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let watch = ReachWatch::new(
            slow_probe(1500, Reach::Ready, asked.clone()),
            Duration::from_millis(50),
            Duration::from_millis(100),
            Duration::from_secs(30),
        );

        let first = Instant::now();
        assert_eq!(watch.get(), Reach::NoShell, "nothing heard yet");
        assert!(first.elapsed() < Duration::from_millis(600), "the first ask waited {:?}", first.elapsed());

        // While the probe is still out, every ask is a read.
        for _ in 0..20 {
            let t = Instant::now();
            assert_eq!(watch.get(), Reach::NoShell);
            assert!(t.elapsed() < Duration::from_millis(50), "an ask waited {:?}", t.elapsed());
        }
        assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), 1, "one probe, not one per ask");

        // And once the shell answers, the answer is what the next ask reads.
        let deadline = Instant::now() + Duration::from_secs(10);
        while watch.get() != Reach::Ready {
            assert!(Instant::now() < deadline, "the probe's answer never landed");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// A shell that is there answers inside the first wait, so a rail filled at startup says the
    /// right thing rather than "not connected" for a tick.
    #[test]
    fn a_quick_shell_is_read_on_the_first_ask() {
        let asked = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let watch = ReachWatch::new(
            slow_probe(0, Reach::NoModel, asked),
            Duration::from_millis(50),
            Duration::from_secs(5),
            Duration::from_secs(30),
        );
        assert_eq!(watch.get(), Reach::NoModel);
    }

    /// Nobody asking, nobody polling: the watcher stops, and the next ask starts it again.
    #[test]
    fn the_watcher_stops_when_nobody_asks_and_starts_again_when_somebody_does() {
        let asked = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let watch = ReachWatch::new(
            slow_probe(0, Reach::Ready, asked.clone()),
            Duration::from_millis(10),
            Duration::from_secs(5),
            Duration::from_millis(60),
        );
        assert_eq!(watch.get(), Reach::Ready);

        let deadline = Instant::now() + Duration::from_secs(10);
        while watch.state.lock().unwrap().running {
            assert!(Instant::now() < deadline, "the watcher never went idle");
            std::thread::sleep(Duration::from_millis(10));
        }
        let idle_at = asked.load(std::sync::atomic::Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), idle_at, "an idle watcher asked the shell");

        assert_eq!(watch.get(), Reach::Ready);
        assert!(watch.state.lock().unwrap().running, "an ask wakes the watcher");
    }
}
