//! The companion's side of "Prepare for me": the real writer (the person's chosen model, no
//! tools) and the thread the job runs on (#1416, slice 5, round 2).
//!
//! The Think arm is the one loop. Each tick it asks [`crate::agenda::triggers`] what is due, and
//! for the soonest not yet prepared item calls [`CompanionService::spawn_prepare_for_me`], which
//! returns at once: the job runs on **a thread of its own**, so a person's own turn is never
//! behind it on the companion's worker. Like a sub-agent (`sub_agent.rs`) it opens its own
//! database connection and builds its own tool registry; it shares the model backend.
//!
//! It is the scheduler's background class (#1407): the thread marks its calls self-started
//! (`yantrik_ml::asked::self_started`, so the gateway classes them `own`), and the shell's pause
//! check (`Start::paused`, `asked::skip_self_started_then` / `keeps_for_asked`) holds it while a
//! person's turn is open and lets it go on when the turn closes. No second scheduler.

use yantrik_ml::{ChatMessage, GenerationConfig};

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::{live, Controls, Item, Outcome, Settings, Writer};
use crate::companion::CompanionService;
use crate::tools::{parse_permission, ToolContext};

/// Writes content with the companion's model: one chat call, no tools, low temperature.
pub struct ModelWriter {
    llm: Arc<dyn yantrik_ml::LLMBackend>,
}

impl Writer for ModelWriter {
    fn write(&mut self, instruction: &str, material: &str, max_tokens: usize) -> Result<(String, usize), String> {
        let prompt = format!(
            "{instruction}\n\nWrite plain text only. Use only the material below and what the instruction says. \
             You have no tools and must not ask for any.\n\n{material}"
        );
        let config = GenerationConfig { max_tokens, temperature: 0.3, ..Default::default() };
        let r = self.llm.chat(&[ChatMessage::user(&prompt)], &config, None).map_err(|e| e.to_string())?;
        Ok((r.text.clone(), r.prompt_tokens + r.completion_tokens))
    }
}

/// What the shell tells the job when it starts it.
pub struct Start {
    pub private: bool,
    /// The Brain switch.
    pub brain: bool,
    /// A person's turn is open, or the account is tight: the job waits (the shell's answer, from
    /// `asked`). The person's own Pause button is [`live`], read here as well.
    pub paused: Arc<dyn Fn() -> bool + Send + Sync>,
    /// Private mode, the Brain switch and the Prepare for me switch, **read live** at every gate:
    /// `Some(why)` when one is off now. `private` and `brain` above are only the start's check.
    pub allowed: Arc<dyn Fn() -> Option<&'static str> + Send + Sync>,
}

/// One job at a time.
static RUNNING: AtomicBool = AtomicBool::new(false);

/// Ends the job's live state and its place in line on every way out of the thread.
struct Over;

impl Drop for Over {
    fn drop(&mut self) {
        live::end();
        RUNNING.store(false, Ordering::SeqCst);
    }
}

impl CompanionService {
    /// Prepare `item` as a background job **on a thread of its own**, and return at once. `Ok`
    /// when it was started (`done` is called from that thread with how it went); `Err` when a job
    /// is already running or the thread could not start. Never in incognito. The caller holds no
    /// database connection.
    pub fn spawn_prepare_for_me(&self, item: Item, start: Start, settings: Settings, done: Box<dyn FnOnce(Outcome) + Send>) -> Result<(), String> {
        let (llm, config) = (self.llm.clone(), self.config.clone());
        let title = item.title.clone();
        in_own_thread(&title, move || {
            let outcome = run_here(llm, &config, &item, &start, &settings);
            done(outcome);
        })
    }
}

/// Runs `work` on a thread named `yos-prepare-job`, the only one at a time, with the job's live
/// state set for as long as it runs. Returns at once.
pub(crate) fn in_own_thread(title: &str, work: impl FnOnce() + Send + 'static) -> Result<(), String> {
    if RUNNING.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        return Err("a preparation is already running".into());
    }
    live::begin(title);
    let spawned = std::thread::Builder::new().name("yos-prepare-job".into()).spawn(move || {
        let _over = Over;
        work();
    });
    spawned.map(drop).map_err(|e| {
        live::end();
        RUNNING.store(false, Ordering::SeqCst);
        format!("could not start the job's thread: {e}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The job is not on the caller's thread (the companion's worker): the call returns while the
    /// work is still blocked, a second job is refused meanwhile, and the live state is Mind View's.
    #[test]
    fn the_job_runs_on_a_thread_of_its_own_and_the_caller_is_free() {
        let _one = live::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let caller = std::thread::current().id();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        in_own_thread("Record the 60 s demo video", move || {
            started_tx.send((std::thread::current().id(), std::thread::current().name().map(String::from))).unwrap();
            live::step("searched the web");
            release_rx.recv().unwrap();
        })
        .unwrap();
        // Back on the caller's thread with the job still blocked.
        let (worker, name) = started_rx.recv().unwrap();
        assert_ne!(worker, caller);
        assert_eq!(name.as_deref(), Some("yos-prepare-job"));
        assert!(in_own_thread("another", || {}).is_err(), "one at a time");
        let s = live::snapshot().expect("Mind View can see it");
        assert_eq!((s.title.as_str(), s.steps.as_slice()), ("Record the 60 s demo video", ["searched the web".to_string()].as_slice()));
        release_tx.send(()).unwrap();
        for _ in 0..200 {
            if !live::is_active() && !RUNNING.load(Ordering::SeqCst) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!live::is_active(), "the live state ends with the job");
        in_own_thread("next", || {}).expect("room again");
        // The last job's thread ends before the lock does: `live` is process-wide.
        for _ in 0..200 {
            if !live::is_active() && !RUNNING.load(Ordering::SeqCst) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}

/// The job's thread: its own connection and registry, the background class, the engine.
fn run_here(llm: Arc<dyn yantrik_ml::LLMBackend>, config: &crate::config::CompanionConfig, item: &Item, start: &Start, settings: &Settings) -> Outcome {
    let db = match yantrikdb_core::YantrikDB::new(&config.yantrikdb.db_path, config.yantrikdb.embedding_dim) {
        Ok(db) => db,
        Err(e) => {
            tracing::warn!(error = %e, "a preparation could not open its database");
            return Outcome::Waiting;
        }
    };
    // The agenda's and the audit's tables are written by other connections too.
    let _ = db.conn().execute_batch("PRAGMA busy_timeout = 5000;");
    let registry = crate::tools::build_registry(config);
    let ctx = ToolContext {
        db: &db,
        max_permission: parse_permission(&config.tools.max_permission),
        registry_metadata: None,
        task_manager: None,
        incognito: false,
        agent_spawner: None,
    };
    // The scheduler's background class: the gateway classes a self-started call `own`.
    let _own = yantrik_ml::asked::self_started();
    let mut writer = ModelWriter { llm };
    let paused = || live::person_paused() || (start.paused)();
    let step = |s: &str| live::step(s);
    let waiting = |on: bool| live::waiting(on);
    let allowed = || (start.allowed)();
    let controls = Controls { enabled: true, private: start.private, brain: start.brain, allowed: &allowed, paused: &paused, stopped: &live::stopped, step: &step, waiting: &waiting, poll: std::time::Duration::from_millis(250) };
    super::run(&registry, &ctx, &mut writer, item, &controls, settings)
}
