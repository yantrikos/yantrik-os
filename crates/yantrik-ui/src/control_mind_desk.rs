//! The Mind desk's three actions (#1023, phase 1): what its bars are told, a stop it asks the
//! person about, and the Agents screen.
//!
//! The desk is `apps/mind-desk`, a window of Mind View that runs on the Mind's own display, where
//! a Mind acting can read it and press what it offers. So **all three are answered to one process:
//! the desk the shell itself started** (`mind_view_desk`: its pid and its start time, checked
//! against what the kernel says of the caller), and to no other, however it is named, whoever runs
//! it and whatever it carries. A Mind in Mind View cannot call them by running the desk's binary, by
//! driving a Terminal there, or from an app the shell started for it.
//!
//! - **`mind_desk`** (read): a [`DeskSnapshot`](yantrik_ipc_contracts::mind_desk::DeskSnapshot),
//!   waiting on a socket for a change. It carries no line a Mind said (see the contract).
//! - **`mind_desk_stop { agent }`**: asks the *person*, on their own desktop, outside Mind View,
//!   whether to stop the Mind the desk is showing. It stops nothing: only the person's Allow on
//!   that card runs `stop_agent` ([`crate::mind_desk::stop`]). The desk is pressable by any Mind, and
//!   `stop_agent` takes a caller with no token for the person, who may stop any agent, so a stop
//!   made directly would let one Mind end another's turn and withdraw the approvals waiting on the
//!   person.
//! - **`mind_desk_open_agents`**: the person's Agents screen, and the only way the desk reaches it.
//!   It raises the shell, which is the interruption Mind View exists to end, so it is answered only
//!   while Mind View is the window in front (the person is looking at the desk), and at most once in
//!   [`AGENTS_EVERY`]. A person typing elsewhere is not something the shell can see without a new
//!   hook on their input; Mind View being in front is the signal it has, and it fails closed.
//!
//! # Waiting
//!
//! `since` is the `tag` of the snapshot the caller last drew. The answer comes back when the shell
//! has something else to say, or after `wait_seconds` with the same words, so the desk never polls
//! on a timer. The store raises its revision on every chunk of a Mind's text, so a revision moving
//! is not yet a change: the snapshot is made again and compared by its tag. The wait runs where
//! `hand_off`'s does ([`control::answer_later`]), off the UI thread, and at most [`WAITS`] at once.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use slint::ComponentHandle;
use yantrik_app_runtime::control::{self, Action, App as ControlSurface, Param};
use yantrik_ipc_contracts::mind_desk::{DeskSnapshot, ACTION, AGENTS_ACTION, LONGEST_WAIT_SECS, STOP_ACTION};

use crate::agents::{self, AgentId};
use crate::approvals;
use crate::mind_desk::{account, project, stop};
use crate::mind_view;
use crate::{AgentsState, App};

/// The least time between two raises of the shell from the desk.
pub const AGENTS_EVERY: Duration = Duration::from_secs(10);

/// How many waits may be blocked at once. The desk makes one; a second is its restart overlapping
/// the first, and anything beyond that is answered at once with what there is.
pub const WAITS: usize = 2;

fn read_spec() -> Action {
    Action::new(
        ACTION,
        "What the Mind desk around Mind View shows: the Mind it follows (its name, state, the tool it \
         is running by name, counts, model and kind of account) and the apps open in Mind View. Reads \
         and changes nothing. Answered only to the desk the shell started. With `since` (the `tag` of \
         the answer it last had) and `wait_seconds` it waits for a different answer.",
    )
    .risk("safe")
    // The answer is about the desk's Mind, not about the shell: no describe of the whole shell is
    // read again beside it, each time a wait ends.
    .stateless()
    .expected_seconds(LONGEST_WAIT_SECS as u32)
    .arg(Param::integer("since").optional().describe("The `tag` of the answer it last had"))
    .arg(Param::integer("wait_seconds").optional().describe(&format!(
        "How long to wait for an answer different from `since`'s. At most {LONGEST_WAIT_SECS}"
    )))
}

fn stop_spec() -> Action {
    Action::new(
        STOP_ACTION,
        "Ask the person, on their own desktop, whether to stop the Mind the Mind desk is showing. Stops \
         nothing itself: only the person's yes on that card does. Answered only to the desk the shell \
         started, and only for the Mind its snapshot shows.",
    )
    .risk("standard")
    .arg(Param::text("agent").describe("The agent the desk is showing"))
}

fn agents_spec() -> Action {
    Action::new(
        AGENTS_ACTION,
        "Put the person's Agents screen up from the Mind desk. Answered only to the desk the shell \
         started, only while Mind View is in front, and at most once every ten seconds.",
    )
    .risk("safe")
}

// ── What the desk reads ──────────────────────────────────────────────

/// The snapshot, now, off the store's lock.
pub fn snapshot_now() -> DeskSnapshot {
    let apps = mind_view::app_names();
    let now = agents::model::now();
    let snapshot = agents::store().read(|s| project::project(s.agents(), now, &apps));
    // The harness host and the stops have locks of their own; the store's is let go of first.
    let snapshot = project::with_account(snapshot, |harness| account::of_harness(harness));
    project::with_stop(snapshot, |agent, working| stop::stops().status_of(agent, working, Instant::now()))
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

/// A place among the waits that may be blocked at once; given back when dropped.
struct Slot;

static WAITING: AtomicUsize = AtomicUsize::new(0);

impl Slot {
    fn take(most: usize) -> Option<Slot> {
        WAITING.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| (n < most).then_some(n + 1)).ok().map(|_| Slot)
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        WAITING.fetch_sub(1, Ordering::SeqCst);
    }
}

/// The read, for a caller that is `desk` (the answer of `require_the_desk`).
fn read(args: &Value, desk: Result<(), String>) -> Result<Value, String> {
    desk?;
    let since = args.get("since").and_then(Value::as_u64);
    let wait = wait_of(args);
    let work = move || {
        // A wait beyond the places there are is not a wait: it answers at once with what there is.
        let slot = Slot::take(WAITS);
        let wait = if slot.is_some() { wait } else { Duration::ZERO };
        let answer = serde_json::to_value(snapshot_after(since, wait)).map_err(|e| e.to_string());
        drop(slot);
        answer
    };
    if wait.is_zero() {
        return work();
    }
    control::answer_later(work).map(|()| json!({ "answering": "off the UI thread" })).or_else(|work| work())
}

// ── A stop, asked of the person ──────────────────────────────────────

/// The card on the person's desktop: the shell's own, outside Mind View, held as every card is.
fn raise_card(agent: &str, name: &str) -> Result<String, String> {
    let verified = approvals::Verified {
        line: "the Mind desk in Mind View, where a Mind can press its buttons as well as you".to_string(),
        raised_by_desktop: true,
        ..Default::default()
    };
    let purpose = format!(
        "Stop {name}? Its turn is ended, every command it is running is killed, and any approval it \
         waits on is withdrawn. This was asked from the Mind desk inside Mind View, where a Mind can \
         press the button too: allow it only if you meant to stop {name}."
    );
    let asked = approvals::Asked {
        app: "shell",
        action: "stop_agent",
        grade: "standard",
        purpose: &purpose,
        published: &purpose,
        target: "",
        explained: "",
    };
    approvals::request("Mind desk", verified, asked, json!({ "agent": agent })).map(|r| r.id)
}

/// Wait, off every thread that matters, for the person's answer to the card, and act on it: only an
/// Allow runs `stop_agent`, and the desk is told how it came out.
fn watch(id: String, agent: String) {
    let spawned = std::thread::Builder::new().name("mind-desk-stop".into()).spawn(move || {
        use approvals::Outcome;
        let allowed = loop {
            match approvals::outcome(&id) {
                None => std::thread::sleep(Duration::from_millis(500)),
                Some((Outcome::Allowed, _)) => break true,
                Some(_) => break false,
            }
        };
        let args = json!({ "agent": agent });
        let stop_now = || {
            // The Allow is spent for this act and no other.
            approvals::consume(&id, "shell", "stop_agent", &args)?;
            let host = crate::wire::harness::host().ok_or("the harness host is not running yet")?;
            crate::control_agents::stop_agent(host, &crate::control_agents::Caller::NoAgent, &AgentId(agent.clone())).map(|_| ())
        };
        stop::stops().settle(&agent, allowed, Instant::now(), stop_now);
        agents::store().poke();
    });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "could not watch the card a stop was asked on");
    }
}

fn ask_stop(args: &Value, desk: Result<(), String>) -> Result<Value, String> {
    desk?;
    let agent = args.get("agent").and_then(Value::as_str).map(str::trim).filter(|a| !a.is_empty()).ok_or("`agent` is empty.")?.to_string();
    let shown = snapshot_now().mind;
    let name = shown.as_ref().map(|m| m.name.clone()).unwrap_or_default();
    let id = stop::stops().ask(&agent, shown.as_ref().map(|m| m.agent.as_str()), Instant::now(), || raise_card(&agent, &name))?;
    watch(id, agent.clone());
    agents::store().poke();
    Ok(json!({ "agent": agent }))
}

// ── The Agents screen ────────────────────────────────────────────────

/// Whether the desk may raise the shell now: Mind View is the window in front, and it is not too
/// soon after the last time.
#[derive(Default)]
struct Raises {
    last: Mutex<Option<Instant>>,
}

impl Raises {
    fn allow(&self, front: Option<&str>, now: Instant) -> Result<(), String> {
        if front != Some(mind_view::TITLE) {
            return Err("Agents opens from the Mind desk only while Mind View is in front.".to_string());
        }
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        if last.is_some_and(|at| now.saturating_duration_since(at) < AGENTS_EVERY) {
            return Err("Agents was opened from the Mind desk a moment ago; it opens at most once every ten seconds.".to_string());
        }
        *last = Some(now);
        Ok(())
    }
}

fn open_agents(ui: &slint::Weak<App>, raises: &Raises, desk: Result<(), String>) -> Result<Value, String> {
    desk?;
    raises.allow(crate::toplevel_watch::front_title().as_deref(), Instant::now())?;
    // Held while a card waits, like every act that raises the shell.
    crate::card_watch::hold_windows(AGENTS_ACTION)?;
    let ui = ui.upgrade().ok_or("the shell is gone")?;
    ui.global::<AgentsState>().invoke_show_section("workroom".into());
    ui.set_current_screen(crate::wire::agents::SCREEN);
    ui.invoke_navigate(crate::wire::agents::SCREEN);
    Ok(json!({ "raised": crate::windows::raise_shell().is_ok() }))
}

/// The actions, on the shell's surface.
pub fn actions(surface: ControlSurface, ui: &App) -> ControlSurface {
    let ui = ui.as_weak();
    let raises = std::sync::Arc::new(Raises::default());
    surface
        .action(read_spec(), |args| read(args, crate::mind_view_desk::require_the_desk()))
        .action(stop_spec(), |args| ask_stop(args, crate::mind_view_desk::require_the_desk()))
        .action(agents_spec(), move |_| open_agents(&ui, &raises, crate::mind_view_desk::require_the_desk()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refused() -> Result<(), String> {
        Err(crate::mind_view_desk::REFUSED.to_string())
    }

    #[test]
    fn the_read_is_a_safe_stateless_action_that_takes_only_how_long_to_wait() {
        let spec = read_spec();
        assert_eq!(spec.name, ACTION);
        assert_eq!(spec.permission, "safe");
        assert!(spec.stateless, "its answer is the Mind's, not the shell's whole describe");
        let names: Vec<&str> = spec.params.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["since", "wait_seconds"], "only how long to wait: nothing it could write");
        assert!(spec.params.iter().all(|a| !a.required && a.kind == "integer"));
    }

    /// The three are the desk's: the same refusal comes from each, before anything is read or
    /// raised, to a caller that is not the desk the shell started.
    #[test]
    fn a_peer_that_is_not_the_desk_is_refused_by_all_three() {
        let said = crate::mind_view_desk::REFUSED.to_string();
        assert_eq!(read(&json!({}), refused()).unwrap_err(), said);
        assert_eq!(ask_stop(&json!({ "agent": "hermes:main" }), refused()).unwrap_err(), said);
        assert_eq!(open_agents(&slint::Weak::default(), &Raises::default(), refused()).unwrap_err(), said);
        // And the real check, with no desk started and no socket in a test, refuses too.
        assert!(read(&json!({}), crate::mind_view_desk::require_the_desk()).is_err());
        assert!(ask_stop(&json!({ "agent": "x" }), crate::mind_view_desk::require_the_desk()).is_err());
    }

    #[test]
    fn the_desk_is_answered_and_the_answer_is_a_snapshot() {
        let said = read(&json!({}), Ok(())).expect("the desk is answered");
        let snapshot: DeskSnapshot = serde_json::from_value(said).expect("in the contract's shape");
        assert_eq!(snapshot.tag, snapshot.tag_of());
    }

    /// A stop asked for a Mind the desk is not showing is refused, and no card is raised: the card
    /// is the only way a stop happens.
    #[test]
    fn a_stop_for_another_mind_raises_nothing() {
        let said = ask_stop(&json!({ "agent": "someone-else:main" }), Ok(())).unwrap_err();
        assert!(said.contains("not the Mind the desk is showing"), "{said}");
        assert!(ask_stop(&json!({}), Ok(())).is_err(), "no agent named");
    }

    #[test]
    fn a_wait_is_held_to_the_longest_a_call_may_take() {
        assert_eq!(wait_of(&json!({})), Duration::ZERO);
        assert_eq!(wait_of(&json!({ "wait_seconds": 5 })), Duration::from_secs(5));
        assert_eq!(wait_of(&json!({ "wait_seconds": 100_000 })), Duration::from_secs(LONGEST_WAIT_SECS));
        assert_eq!(wait_of(&json!({ "wait_seconds": "soon" })), Duration::ZERO);
    }

    #[test]
    fn only_so_many_waits_block_at_once() {
        let first = Slot::take(2).expect("a place");
        let second = Slot::take(2).expect("a second place");
        assert!(Slot::take(2).is_none(), "a third is answered at once");
        drop(first);
        assert!(Slot::take(2).is_some(), "a place given back is a place");
        drop(second);
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

        let mut waits = 0;
        assert_eq!(after(None, Duration::from_secs(20), || said(5), |_| waits += 1).tag, 5);
        assert_eq!(after(Some(4), Duration::from_secs(20), || said(5), |_| waits += 1).tag, 5);
        assert_eq!(waits, 0);
    }

    #[test]
    fn with_nothing_new_it_answers_at_the_end_of_its_wait() {
        let began = Instant::now();
        let got = after(Some(5), Duration::from_millis(120), || said(5), |left| std::thread::sleep(left.min(Duration::from_millis(30))));
        assert_eq!(got.tag, 5);
        assert!(began.elapsed() >= Duration::from_millis(120), "{:?}", began.elapsed());
        let mut looks = 0;
        let _ = after(Some(5), Duration::ZERO, || { looks += 1; said(5) }, |_| unreachable!("it does not wait"));
        assert_eq!(looks, 1);
    }

    /// The Agents screen is raised from the desk at most once in ten seconds, and only while Mind
    /// View is the window in front: a Mind pressing it while the person works elsewhere does nothing.
    #[test]
    fn the_agents_screen_is_rate_limited_and_only_while_mind_view_is_in_front() {
        let raises = Raises::default();
        let t0 = Instant::now();
        let front = Some(mind_view::TITLE);
        // Not in front, or not known to be: refused, and it does not use up the turn.
        assert!(raises.allow(Some("Notes"), t0).is_err());
        assert!(raises.allow(None, t0).is_err());
        assert!(raises.allow(front, t0).is_ok(), "the first");
        for secs in [0, 1, 5, 9] {
            assert!(raises.allow(front, t0 + Duration::from_secs(secs)).is_err(), "{secs} s after");
        }
        assert!(raises.allow(front, t0 + AGENTS_EVERY).is_ok(), "ten seconds after");
        assert!(raises.allow(front, t0 + AGENTS_EVERY + Duration::from_secs(1)).is_err(), "and ten more are owed");
    }

    #[test]
    fn the_desk_has_no_other_way_to_the_shell_than_these_three_actions() {
        let names = [read_spec().name, stop_spec().name, agents_spec().name];
        assert_eq!(names, ["mind_desk", "mind_desk_stop", "mind_desk_open_agents"]);
        assert_eq!(stop_spec().permission, "standard");
    }
}
