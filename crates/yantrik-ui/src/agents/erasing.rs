//! The shell's half of a harness `redact`: erasing words from an agent's session in memory and
//! on disk, once the host has accepted it (`yantrik_harness::host::erase`).
//!
//! In the host's steps: the texts are copied out under the store's lock ([`Agents::erasure_plan`]),
//! measured and searched with no lock held, and applied under the locks — `disk`, then `store`,
//! the one order every path takes them in — with the session's file rewritten before it returns
//! ([`Agents::apply_erasure`]).

use yantrik_harness::host::{ShellErased, ShellErasure, ShellPlan, ShellRedactor};
use yantrik_harness::redact::Search;

use super::store::{self, ErasurePlan};
use super::{AgentId, Agents};

impl Agents {
    /// Erase words from `id`'s session at the person's request: copy, search off the lock, apply.
    /// The host does the same in its own steps (through [`Redactor`]), so the search counts towards
    /// its limit; this is for callers that already know the session is small, and tests.
    pub fn redact(&self, id: &AgentId, erasure: &ShellErasure<'_>) -> Result<ShellErased, String> {
        let mut plan = self.erasure_plan(id);
        plan.find(&Search::new(erasure.needles));
        self.apply_erasure(&plan, erasure)
    }

    /// Copies of `id`'s texts, in canonical form. The store's lock is held only to copy.
    pub fn erasure_plan(&self, id: &AgentId) -> ErasurePlan {
        let texts = self.lock().erasure_texts(id);
        ErasurePlan::new(id.clone(), texts)
    }

    /// Apply what `plan` found (see `store/erase.rs` for what is erased and what is only shown
    /// erased). The pane changes at once, and the session's file is rewritten before this returns:
    /// written beside, flushed to the disk, renamed over the old one, and the directory flushed. A
    /// save the timer took before the erasure is never written after it.
    pub fn apply_erasure(&self, plan: &ErasurePlan, erasure: &ShellErasure<'_>) -> Result<ShellErased, String> {
        // `disk`, then `store`: the lock order (on `Agents::disk`).
        let disk = self.disk.lock().unwrap_or_else(|e| e.into_inner());
        let (done, file) = {
            let mut s = self.lock();
            let done = s.apply_erasure(&plan.id, erasure, plan);
            self.erased.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            (done, s.file_of(&self.dir, &plan.id))
        };
        if let Some((path, contents)) = file {
            store::write_durably(&self.dir, &path, &contents)
                .map_err(|e| format!("the session's file could not be rewritten: {e}"))?;
        }
        drop(disk);
        Ok(done)
    }
}

/// The shell's half of a `redact`, as the host asks for it (`wire/harness.rs`).
pub struct Redactor;

impl ShellRedactor for Redactor {
    fn size(&self, agent: &AgentId) -> u64 {
        super::store().read(|s| s.erasure_size(agent))
    }

    fn prepare(&self, agent: &AgentId) -> Result<Box<dyn ShellPlan>, String> {
        Ok(Box::new(Plan(super::store().erasure_plan(agent))))
    }
}

struct Plan(ErasurePlan);

impl ShellPlan for Plan {
    fn work(&self, search: &Search) -> u64 {
        self.0.cost(search)
    }

    fn search(&mut self, search: &Search) {
        self.0.find(search)
    }

    fn apply(self: Box<Self>, erasure: &ShellErasure<'_>) -> Result<ShellErased, String> {
        super::store().apply_erasure(&self.0, erasure)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use yantrik_harness::redact::{Needle, MARKER};

    use super::super::store;
    use super::*;

    /// The timed save and an erasure take the same two locks; in the same order, they can run at
    /// the same time as often as they like and always finish.
    #[test]
    fn an_erasure_and_saves_at_the_same_time_always_finish() {
        let pi = AgentId::new("erasing-race", "c-r1");
        store().open_turn(&pi, "the code is Quokka-7741");
        store().text(&pi, "Noted: Quokka-7741.");
        store().close_turn(&pi, true);
        let (finished, done) = std::sync::mpsc::channel();
        let erasing = pi.clone();
        std::thread::spawn(move || {
            let pi = erasing;
            let saving = Arc::new(AtomicBool::new(true));
            let saver = {
                let saving = saving.clone();
                std::thread::spawn(move || {
                    let mut saves = 0u64;
                    while saving.load(Ordering::Relaxed) {
                        store().save_now();
                        saves += 1;
                    }
                    saves
                })
            };
            let needles = [Needle::of("quokka-7741")];
            let mut places = 0;
            for _ in 0..200 {
                let erasure = ShellErasure { request_id: "race", needles: &needles, places_in_runs: 0 };
                places += store().redact(&pi, &erasure).unwrap().places;
            }
            saving.store(false, Ordering::Relaxed);
            let saves = saver.join().unwrap();
            let _ = finished.send((places, saves));
        });
        let (places, saves) =
            done.recv_timeout(Duration::from_secs(120)).expect("an erasure and the timed saves waited on each other");
        assert_eq!(places, 3, "the prompt, the title and the reply, once");
        assert!(saves > 0);
        let text = store().read(|s| s.transcript(&pi, 5)).unwrap();
        assert!(text.contains(MARKER) && !text.to_lowercase().contains("quokka"), "{text}");
    }
}
