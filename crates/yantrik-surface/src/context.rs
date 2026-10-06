//! What a handler can learn about the call it is running for, and how it finishes an answer that
//! takes time — none of which travels in the handler's arguments.
//!
//! All three are thread-locals installed for exactly the duration of one dispatch, on the thread
//! the handler runs on. A service dispatches on the socket's own worker thread, so the dispatch
//! installs them there; a window's handlers run on its UI thread, so `yantrik-app-runtime` carries
//! them across the hop and installs them on the far side with the same guards.

use std::cell::RefCell;

use yantrik_ipc_transport::server::PeerCred;

// ── Who is calling ──────────────────────────────────────────────────
//
// A handler used to have no way to find out. Everything it could see about its caller arrived
// inside the request, which means the caller wrote it — and the shell was printing one of those
// strings on an approval card under the words "asking to use this machine". Anything that could
// open the socket could put any name there (issue #43).
//
// The kernel knows better and says so for free: `SO_PEERCRED` on an accepted unix socket gives
// the peer's pid, uid and gid, filled in at `connect` time from the peer's own process. The
// transport reads it at accept (see `yantrik_ipc_transport::server::PeerCred`); this module's job
// is to get it to the place the handler actually runs.
//
// That is why this is a thread-local rather than a global: two connections can be in flight at
// once, and a "current caller" stored anywhere shared would be read by a handler that belongs to
// a different request. So the caller travels WITH the dispatch, and is installed on the thread
// the handler runs on for exactly the duration of that one dispatch.
//
// The handler signature is untouched: apps build `|args| { ... }` closures and none of them has
// to change. A handler that cares reads [`caller`]; every other one never learns this exists.

/// Who opened the socket this request came in on, as the kernel reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Caller {
    /// The peer process at `connect` time. It may well have exited by now — `yos` runs one call
    /// and stops — so anything that wants `/proc` facts about it must read them promptly.
    pub pid: i32,
    pub uid: u32,
    pub gid: u32,
    /// The peer's start time, read at accept (`PeerCred::started`): what pins a later `/proc`
    /// walk to this process rather than one that took its pid since.
    pub started: Option<u64>,
}

impl From<PeerCred> for Caller {
    fn from(p: PeerCred) -> Caller {
        Caller { pid: p.pid, uid: p.uid, gid: p.gid, started: p.started }
    }
}

impl From<Caller> for PeerCred {
    fn from(c: Caller) -> PeerCred {
        PeerCred { pid: c.pid, uid: c.uid, gid: c.gid, started: c.started }
    }
}

thread_local! {
    /// The caller of the dispatch currently running on THIS thread, or `None`.
    static CURRENT_CALLER: RefCell<Option<Caller>> = const { RefCell::new(None) };

    /// The agent token of the dispatch currently running on THIS thread, or `None`.
    static CURRENT_AGENT_TOKEN: RefCell<Option<String>> = const { RefCell::new(None) };

    /// During one dispatch: where [`answer_later`] leaves the rest of the answer. `None` outside a
    /// dispatch, which is how `answer_later` knows nothing will run it.
    static LATER_SLOT: RefCell<Option<Option<Later>>> = const { RefCell::new(None) };
}

/// Who is calling, inside an action or describe handler. `None` when nothing could be
/// established — a TCP dev connection, a peer that vanished, or a handler invoked directly.
///
/// Nothing in the dispatch refuses anything on the strength of it. Deciding what an identity is
/// worth is the shell's business (`crates/yantrik-ui/src/caller_identity.rs`); the dispatch's job
/// is only to make the fact available where it can be read honestly.
pub fn caller() -> Option<Caller> {
    CURRENT_CALLER.with(|cell| *cell.borrow())
}

/// Installs `who` as [`caller`] for as long as it is held, and puts back what was there before.
///
/// A guard rather than a set-then-clear pair, so a handler that panics cannot leave the next
/// dispatch on this thread reading the previous caller's pid — which would be worse than no
/// caller: the shell prints that pid on an approval card as a verified fact. It restores the
/// *previous* value rather than clearing, which costs nothing and keeps a nested call honest.
#[must_use = "the caller is uninstalled when this guard is dropped"]
pub struct CallerScope(Option<Caller>);

impl CallerScope {
    pub fn enter(who: Option<Caller>) -> CallerScope {
        CallerScope(CURRENT_CALLER.with(|cell| cell.replace(who)))
    }
}

impl Drop for CallerScope {
    fn drop(&mut self) {
        CURRENT_CALLER.with(|cell| *cell.borrow_mut() = self.0);
    }
}

// ── Which agent a call is for ───────────────────────────────────────
//
// A mind running as one of the person's agents carries a token its harness was given (design
// `agents-workspace-2026-09-23.md`, decision 3). It travels BESIDE `args` on `app.act`, the way a
// grant does, and never inside them — because `args` is what gets shown and kept: the approval
// card draws it, `record_unasked_action` writes it to `mind-audit.jsonl`, a grant is bound to it.
// A token in any of those is a token anyone reading the screen or the log can replay.
//
// So the dispatch lifts the token off the call (`gate::agent_token_of`, beside `grant_of`), strips
// any copy a caller put inside `args`, and hands it to the handler the way it hands over the
// caller: for the duration of the one dispatch, on the thread the handler runs on. What the token
// is worth is the handler's business — the shell resolves it against the kernel's account of the
// caller; here it is only carried.

/// The agent token the call being handled carried beside its `args`, inside an action handler.
/// `None` when it carried none, or outside a dispatch.
///
/// Like [`caller`], it is a fact about the call and not a verdict: nothing here checks it.
pub fn agent_token() -> Option<String> {
    CURRENT_AGENT_TOKEN.with(|cell| cell.borrow().clone())
}

/// Installs a dispatch's token for its duration and puts back what was there, panic or not.
#[must_use = "the token is uninstalled when this guard is dropped"]
pub struct AgentTokenScope(Option<String>);

impl AgentTokenScope {
    pub fn enter(token: Option<String>) -> AgentTokenScope {
        AgentTokenScope(CURRENT_AGENT_TOKEN.with(|cell| cell.replace(token)))
    }
}

impl Drop for AgentTokenScope {
    fn drop(&mut self) {
        let previous = self.0.take();
        CURRENT_AGENT_TOKEN.with(|cell| *cell.borrow_mut() = previous);
    }
}

// ── Answers that take time ──────────────────────────────────────────
//
// A window's handler has a few seconds, on the thread that paints the window. Some acts are worth
// waiting for anyway: the shell's `agent_run` starts a command and owes its caller the exit code,
// which may be two minutes away. Deferring (`settled: false`, "go and look later") is the right
// answer for work whose result lands on screen; it is the wrong one for work whose result IS the
// answer.
//
// So a handler can say "the rest of my answer is this closure". It returns at once — the window
// never waits — and the dispatch runs the closure on the socket's side, where the only thing
// waiting is the one caller who asked, stepped out of the way of every other caller of the same
// socket (`off_the_reactor`). A service has no window to protect, and the same call works there:
// the closure runs after the handler, on the worker, stepped aside the same way.

/// The rest of an answer, finished after the handler has returned.
pub type Later = Box<dyn FnOnce() -> Result<serde_json::Value, String> + Send>;

/// Finish this action's answer after the handler has returned: `work` runs on the socket's side,
/// off any UI thread, and what it returns is the caller's `result` (an `Err` is the caller's
/// refusal, exactly as if the handler had returned it). The envelope's view is read again once it
/// has, so the state beside the result is the state the result came from.
///
/// Call it from inside a handler, as its last act, and return anything — the value is replaced.
/// `work` must carry everything it needs: in a window it does not run on the UI thread, so it
/// cannot touch the window, and [`caller`] is not set where it runs (read it in the handler and
/// move it in).
///
/// `Err(work)` hands the work back when nothing will run it — the handler was called directly, not
/// through a dispatch — so the handler can run it itself:
/// `answer_later(work).map(|()| placeholder).or_else(|work| work())`.
///
/// If a window's UI thread answered too late for the caller, `work` is dropped without running:
/// do the whole of the act inside it and a late reply starts nothing.
pub fn answer_later<F>(work: F) -> Result<(), F>
where
    F: FnOnce() -> Result<serde_json::Value, String> + Send + 'static,
{
    LATER_SLOT.with(|cell| match cell.borrow_mut().as_mut() {
        Some(slot) => {
            *slot = Some(Box::new(work));
            Ok(())
        }
        None => Err(work),
    })
}

/// Opens the slot for one dispatch and closes it afterwards, even on a panic, so one handler's
/// work can never be run as another's answer.
#[must_use = "the slot closes when this guard is dropped"]
pub struct LaterScope(Option<Option<Later>>);

impl LaterScope {
    pub fn enter() -> LaterScope {
        LaterScope(LATER_SLOT.with(|cell| cell.replace(Some(None))))
    }

    /// What the handler left with [`answer_later`], once.
    pub fn take(&self) -> Option<Later> {
        LATER_SLOT.with(|cell| cell.borrow_mut().as_mut().and_then(Option::take))
    }
}

impl Drop for LaterScope {
    fn drop(&mut self) {
        let previous = self.0.take();
        LATER_SLOT.with(|cell| *cell.borrow_mut() = previous);
    }
}

/// Run `work` without holding up the socket's other callers: on a multi-threaded tokio runtime
/// this worker steps aside and another takes its connections. On a plain thread, or a
/// current-thread runtime, it simply runs.
pub fn off_the_reactor<T>(work: impl FnOnce() -> T) -> T {
    use tokio::runtime::{Handle, RuntimeFlavor};
    match Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(work)
        }
        _ => work(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_caller_is_current_only_while_its_own_dispatch_runs() {
        // The reason this is a thread-local with a guard rather than a global: two connections
        // can be in flight at once, and a handler must never read the pid of somebody else's
        // request. Outside a scope there is no caller at all — not a stale one.
        assert_eq!(caller(), None, "nothing is calling before anything has called");

        let hermes = Caller { pid: 696, uid: 1000, gid: 1000, started: None };
        {
            let _scope = CallerScope::enter(Some(hermes));
            assert_eq!(caller(), Some(hermes));

            // Nested, because `describe` inside an `act` is a real shape.
            {
                let _inner = CallerScope::enter(Some(Caller { pid: 4242, uid: 1000, gid: 1000, started: None }));
                assert_eq!(caller().map(|c| c.pid), Some(4242));
            }
            assert_eq!(caller(), Some(hermes), "the outer dispatch gets its own caller back");
        }
        assert_eq!(caller(), None, "and nothing is left behind");
    }

    #[test]
    fn a_handler_that_panics_does_not_leave_its_caller_behind() {
        // A leaked caller would be worse than none: the next request on this thread would be
        // attributed to the process that crashed the previous one, and the shell would print
        // that pid on an approval card as a verified fact.
        let panicked = std::panic::catch_unwind(|| {
            let _scope = CallerScope::enter(Some(Caller { pid: 7, uid: 0, gid: 0, started: None }));
            assert_eq!(caller().map(|c| c.pid), Some(7));
            panic!("a handler blew up");
        });
        assert!(panicked.is_err(), "the panic has to actually happen for this to prove anything");
        assert_eq!(caller(), None);
    }

    #[test]
    fn an_agent_token_is_current_only_while_its_own_dispatch_runs() {
        assert_eq!(agent_token(), None);
        {
            let _outer = AgentTokenScope::enter(Some("tok-a".into()));
            assert_eq!(agent_token().as_deref(), Some("tok-a"));
            {
                let _inner = AgentTokenScope::enter(None);
                assert_eq!(agent_token(), None, "a nested call without one has none");
            }
            assert_eq!(agent_token().as_deref(), Some("tok-a"));
        }
        assert_eq!(agent_token(), None, "and nothing is left behind for the next dispatch");
    }

    #[test]
    fn a_handler_called_directly_is_handed_its_work_back_to_run_itself() {
        // No socket, no dispatch: nothing would run the work, so it comes back.
        let back = answer_later(|| Ok(serde_json::json!("ran inline")));
        let work = back.err().expect("no dispatch is in progress on this thread");
        assert_eq!(work(), Ok(serde_json::json!("ran inline")));

        // And inside a dispatch's scope it is kept, once, for the dispatch to finish.
        let scope = LaterScope::enter();
        assert!(answer_later(|| Ok(serde_json::json!(1))).is_ok());
        let kept = scope.take().expect("the work was kept");
        assert_eq!(kept(), Ok(serde_json::json!(1)));
        assert!(scope.take().is_none(), "taken once");
        drop(scope);
        assert!(answer_later(|| Ok(serde_json::json!(2))).is_err(), "the scope closed with the dispatch");
    }
}
