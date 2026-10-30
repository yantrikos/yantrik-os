//! `mind_desk`: what the bars around Mind View are told (#1023, phase 1).
//!
//! The desk is `apps/mind-desk`, a window of Mind View that runs on the Mind's own display. It
//! reads the Mind it follows from the shell, here, over the control surface every app and `yos`
//! already use; nothing new listens anywhere.
//!
//! # What it answers, and to whom
//!
//! A [`DeskSnapshot`](yantrik_ipc_contracts::mind_desk::DeskSnapshot): a name, a state, the Mind's
//! own line, one tool's name, counts, a model label and a kind of account (see
//! [`crate::mind_desk`]). It is graded `safe` because it reads and changes nothing, and it takes
//! no argument that could: `since` and `wait_seconds` only say how long the answer may wait.
//!
//! It is answered to the person's own processes, and **refused to a Mind** (the kernel's word on
//! the caller, an agent token, or descent from an attached mind: [`mind_view::requester_now`]).
//! The Mind's line is the person's to read, as `describe shell` already holds it back from an
//! agent (`agents::PERSONS_FIELDS`), and one Mind must not learn another's by asking the desk. A
//! Mind that wants to know about itself has `read_agent`; the desk's own screen is not a way
//! round that, because this is the only door it reads through.
//!
//! # Waiting
//!
//! `since` is the `tag` of the snapshot the caller last drew. The answer comes back when the shell
//! has something else to say, or after `wait_seconds` with the same words, so the desk never
//! polls on a timer and a quiet desk is one blocked socket. The store raises its revision on every
//! chunk of a Mind's text, so a revision moving is not yet a change: the snapshot is made again
//! and compared by its tag, and the caller is woken only by a different one.
//!
//! The wait runs where `hand_off`'s does ([`control::answer_later`]), off the UI thread and off
//! the socket's other callers.

use std::time::{Duration, Instant};

use serde_json::{json, Value};
use yantrik_app_runtime::control::{self, Action, App as ControlSurface, Param};
use yantrik_ipc_contracts::mind_desk::{DeskSnapshot, ACTION, LONGEST_WAIT_SECS};

use crate::agents;
use crate::mind_desk::{account, project};
use crate::mind_view::{self, Requester};

/// What the Mind desk may not be told, as the refusal says it.
pub const REFUSED_TO_MINDS: &str = "the Mind desk shows the person's own view of Mind View, so it is \
    answered to the person's processes only. `read_agent` reads your own session, and `describe shell` \
    says what you may know of the others.";

fn spec() -> Action {
    Action::new(
        ACTION,
        "What the Mind desk around Mind View shows: the Mind it follows (its name, state, its own \
         line, the tool it is running by name, its counts and model) and the apps open in Mind View. \
         Reads and changes nothing. Answered to the person's own processes only. With `since` (the \
         `tag` of the answer you last had) and `wait_seconds` it waits for a different answer.",
    )
    .risk("safe")
    // The answer is about the desk's Mind, not about the shell: no describe of the whole shell is
    // read again beside it, each time a wait ends.
    .stateless()
    .expected_seconds(LONGEST_WAIT_SECS as u32)
    .arg(Param::integer("since").optional().describe("The `tag` of the answer you last had"))
    .arg(Param::integer("wait_seconds").optional().describe(&format!(
        "How long to wait for an answer different from `since`'s. At most {LONGEST_WAIT_SECS}"
    )))
}

/// The snapshot, now, off the store's lock.
pub fn snapshot_now() -> DeskSnapshot {
    let apps = mind_view::app_names();
    let now = agents::model::now();
    let snapshot = agents::store().read(|s| project::project(s.agents(), now, &apps));
    // The harness host has a lock of its own; the store's is let go of first.
    project::with_account(snapshot, |harness| account::of_harness(harness))
}

/// The snapshot, once it is not `since`'s or `wait` has run out.
fn snapshot_after(since: Option<u64>, wait: Duration) -> DeskSnapshot {
    let mut seen = agents::store().revision();
    after(since, wait, snapshot_now, |left| seen = agents::store().changed_since(seen, left))
}

/// [`snapshot_after`] with the world given: how a snapshot is made, and how to wait (at most the
/// time left) for the store to move.
fn after(
    since: Option<u64>,
    wait: Duration,
    mut make: impl FnMut() -> DeskSnapshot,
    mut moved: impl FnMut(Duration),
) -> DeskSnapshot {
    let deadline = Instant::now() + wait;
    loop {
        let snapshot = make();
        let left = deadline.saturating_duration_since(Instant::now());
        if since != Some(snapshot.tag) || left.is_zero() {
            return snapshot;
        }
        // Nothing the desk would draw differently: wait for the next move.
        moved(left);
    }
}

/// The seconds asked for, held to the longest a call may wait.
fn wait_of(args: &Value) -> Duration {
    let asked = args.get("wait_seconds").and_then(Value::as_u64).unwrap_or(0);
    Duration::from_secs(asked.min(LONGEST_WAIT_SECS))
}

fn answer(args: &Value) -> Result<Value, String> {
    // Read here, on the dispatch's thread, where the caller is in scope: nowhere else has it.
    if let Requester::Mind(_) = mind_view::requester_now() {
        return Err(REFUSED_TO_MINDS.to_string());
    }
    let since = args.get("since").and_then(Value::as_u64);
    let wait = wait_of(args);
    let work = move || serde_json::to_value(snapshot_after(since, wait)).map_err(|e| e.to_string());
    if wait.is_zero() {
        return work();
    }
    control::answer_later(work).map(|()| json!({ "answering": "off the UI thread" })).or_else(|work| work())
}

/// The action, on the shell's surface.
pub fn actions(surface: ControlSurface) -> ControlSurface {
    surface.action(spec(), |args| answer(args))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_is_a_safe_read_with_no_argument_that_could_change_anything() {
        let spec = spec();
        assert_eq!(spec.name, ACTION);
        assert_eq!(spec.permission, "safe");
        assert!(spec.stateless, "its answer is the Mind's, not the shell's whole describe");
        let names: Vec<&str> = spec.params.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["since", "wait_seconds"], "only how long to wait: nothing it could write");
        assert!(spec.params.iter().all(|a| !a.required && a.kind == "integer"));
    }

    #[test]
    fn a_wait_is_held_to_the_longest_a_call_may_take() {
        assert_eq!(wait_of(&json!({})), Duration::ZERO);
        assert_eq!(wait_of(&json!({ "wait_seconds": 5 })), Duration::from_secs(5));
        assert_eq!(wait_of(&json!({ "wait_seconds": 100_000 })), Duration::from_secs(LONGEST_WAIT_SECS));
        assert_eq!(wait_of(&json!({ "wait_seconds": "soon" })), Duration::ZERO);
    }

    #[test]
    fn the_person_is_answered_and_the_answer_is_a_snapshot() {
        // No dispatch, so no caller at all: a click or a test, which is the person.
        let said = answer(&json!({})).expect("the person is answered");
        let snapshot: DeskSnapshot = serde_json::from_value(said).expect("in the contract's shape");
        assert_eq!(snapshot.tag, snapshot.tag_of());
    }

    fn said(tag: u64) -> DeskSnapshot {
        DeskSnapshot { tag, ..Default::default() }
    }

    /// The wait ends on a different answer, not on the store moving: a Mind's text raises the
    /// revision on every chunk, and the desk is not woken for words it would draw the same.
    #[test]
    fn a_call_returns_on_a_different_answer_and_not_on_a_move_that_draws_the_same() {
        let mut tags = [5, 5, 5, 6].into_iter();
        let mut waits = 0;
        let got = after(Some(5), Duration::from_secs(20), || said(tags.next().unwrap()), |_| waits += 1);
        assert_eq!((got.tag, waits), (6, 3), "three moves that said the same, then the change");

        // A caller with nothing yet is answered at once, and so is one behind.
        let mut waits = 0;
        assert_eq!(after(None, Duration::from_secs(20), || said(5), |_| waits += 1).tag, 5);
        assert_eq!(after(Some(4), Duration::from_secs(20), || said(5), |_| waits += 1).tag, 5);
        assert_eq!(waits, 0);
    }

    /// With nothing new, the call answers at the end of its wait with the words it has, and not
    /// before.
    #[test]
    fn with_nothing_new_it_answers_at_the_end_of_its_wait() {
        let began = Instant::now();
        let got = after(Some(5), Duration::from_millis(120), || said(5), |left| std::thread::sleep(left.min(Duration::from_millis(30))));
        assert_eq!(got.tag, 5);
        assert!(began.elapsed() >= Duration::from_millis(120), "{:?}", began.elapsed());
        // No wait at all is one look.
        let mut looks = 0;
        let _ = after(Some(5), Duration::ZERO, || { looks += 1; said(5) }, |_| unreachable!("it does not wait"));
        assert_eq!(looks, 1);
    }

    /// The refusal is the one sentence, and it names the door a Mind has instead.
    #[test]
    fn a_mind_is_told_where_to_read_its_own_session() {
        assert!(REFUSED_TO_MINDS.contains("read_agent") && REFUSED_TO_MINDS.contains("describe shell"));
        assert!(!REFUSED_TO_MINDS.contains("hunter"), "and says nothing of anyone");
    }
}
