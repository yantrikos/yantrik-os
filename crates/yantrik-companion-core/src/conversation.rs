//! What a whole conversation has read, as against one turn of it.
//!
//! `taint` forgets at every `begin_turn`: right for a credential (one fetched ten minutes ago must
//! not license a leak now), wrong for text. An instruction read in turn N is still in the history
//! the model reads in turn N+1, where the person types something unrelated and the turn has read
//! nothing new. So for the rules in [`crate::changes`], what was read stays read until the
//! conversation itself is cleared (`clear`: New chat, the idle reset). `secret_from` and the
//! outside mark stay per turn, in `taint`.
//!
//! Per thread, like `taint`, and carried the same ways: onto a sub-agent's thread (`carry`/`adopt`)
//! and back with its result (`source`/`absorb`).

use std::cell::RefCell;

thread_local! {
    static READ: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// The first tool that brought untrusted content into this conversation, if one has.
pub fn source() -> Option<String> {
    READ.with(|r| r.borrow().clone())
}

/// Called after every tool call `by`: what this turn has read now belongs to the conversation. The
/// first call after which the turn holds untrusted content is the one that brought it in.
pub fn keep(by: &str) {
    if crate::taint::state().0.is_some() {
        absorb(Some(by.to_string()));
    }
}

/// Take in what another thread read for this conversation (a sub-agent's result). The turn is
/// marked too, so the secret rule sees it before the parent uses the result.
pub fn absorb(read: Option<String>) {
    let Some(from) = read else { return };
    READ.with(|r| {
        let mut r = r.borrow_mut();
        if r.is_none() {
            *r = Some(from);
        }
    });
    crate::taint::note("browser_read", "browser");
}

/// The conversation is over: New chat, or the idle timeout.
pub fn clear() {
    READ.with(|r| *r.borrow_mut() = None);
}

/// Set the conversation's state aside, leaving it empty (a turn for someone else, in between).
pub fn take() -> Option<String> {
    READ.with(|r| r.borrow_mut().take())
}

/// Put back what `take` set aside.
pub fn restore(held: Option<String>) {
    READ.with(|r| *r.borrow_mut() = held);
}

/// This thread's state, for `adopt` on a thread that works for the conversation.
pub fn carry() -> Option<String> {
    source()
}

/// Continue the carried state on this thread.
pub fn adopt(carried: Option<String>) {
    restore(carried);
}

/// What each recipe run has read, kept between its steps (they are separate calls, with other work
/// between them, and a step that deletes must see what an earlier step read). In memory: a restart
/// forgets it.
static RUNS: std::sync::Mutex<Vec<(String, String)>> = std::sync::Mutex::new(Vec::new());
const MOST_RUNS: usize = 256;

/// A turn nobody started (an urge, a queued task, a recipe step), held while the guard lives: its
/// own taint turn and its own conversation state, so it neither inherits what the person's
/// conversation read nor loses what it reads itself. Inside a person's turn (a recipe the person
/// asked for) it is inert and the turn goes on.
pub struct Unattended {
    held: Option<(crate::taint::Carried, Option<String>)>,
    run: Option<String>,
}

pub fn unattended_turn() -> Unattended {
    start(None)
}

/// `unattended_turn` for one recipe run: its state is the run's, across its steps.
pub fn unattended_run(recipe_id: &str) -> Unattended {
    start(Some(recipe_id.to_string()))
}

fn start(run: Option<String>) -> Unattended {
    if crate::background_research::person_started() {
        return Unattended { held: None, run: None };
    }
    let held = (crate::taint::carry(), take());
    crate::taint::begin_turn();
    if let Some(id) = &run {
        let kept = RUNS.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(k, _)| k == id).map(|(_, v)| v.clone());
        restore(kept);
        if source().is_some() {
            crate::taint::note("browser_read", "browser");
        }
    }
    Unattended { held: Some(held), run }
}

impl Drop for Unattended {
    fn drop(&mut self) {
        let Some((turn, held)) = self.held.take() else { return };
        keep("a step of the run");
        if let (Some(id), Some(read)) = (self.run.take(), source()) {
            let mut runs = RUNS.lock().unwrap_or_else(|e| e.into_inner());
            if !runs.iter().any(|(k, _)| *k == id) {
                if runs.len() >= MOST_RUNS {
                    runs.remove(0);
                }
                runs.push((id, read));
            }
        }
        crate::taint::end_turn();
        crate::taint::adopt(turn);
        restore(held);
    }
}
