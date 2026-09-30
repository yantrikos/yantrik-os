//! The optic nerve — perception-service to the companion.
//!
//! # Why this file exists
//!
//! perception-service has been running, correctly, watching the kernel: fanotify saves with the
//! acting process named, the netlink process connector, PSI. It filled a ring with 1120
//! observations. And `grep -rn "perception.since" crates/` returned **zero**. Nothing in the OS
//! read it. The eye worked and was wired to nothing.
//!
//! That is worth stating plainly because it is not a small oversight — the service was measured,
//! probed, fixed and deployed, and at no point did anyone ask whether something was listening. A
//! source with no consumer looks exactly like a source with a consumer, from the source's side.
//!
//! # The gate is the design
//!
//! yantrikdb-core, who runs the memory substrate, put it this way after measuring their own
//! production store: *continuous capture is cheap; continuous memory is not.* Their retrieval
//! collapsed to MRR 0.054 while every record it needed was present — the firehose degraded
//! retrieval long before it degraded storage, and the failure was silent. Their advice was
//! specific: **put the gate between capture and anything that later gets searched.**
//!
//! So this reader admits very little. Of those 1120 observations, the overwhelming majority were
//! the operator's own ssh session — `sudo`, `unix_chkpwd`, `python3`, `ls`. A memory store fed
//! that raw would be a diary of its own administration.
//!
//! Three filters, in the order they are cheapest to apply:
//!
//! 1. **Ours is not news.** An observation caused by the OS itself — its own apps, its own
//!    services, its own companion — is recorded as a transition and never becomes a memory. This
//!    is causal tagging rather than a suppression window: a window is defined by exactly the
//!    property a real event shares with our own effect (happening now, here) and so cannot tell
//!    them apart. A name can.
//! 2. **Salience.** The service already scores every observation for how much it deserves
//!    something expensive. Below [`MEMORABLE`] it reaches the activity feed and stops there.
//! 3. **Rate.** A bounded number of memories per minute, whatever happens. A compile loop, a
//!    `cargo build`, an editor autosaving — any of them can produce hundreds of legitimate,
//!    high-salience observations, and the store must not take all of them.
//!
//! # It parks rather than polls
//!
//! `perception.since` accepts `wait_ms` and blocks on a condvar until something arrives. The
//! whole argument for watching from the kernel is that it costs nothing while nothing is
//! happening, and a reader that polled would hand that back. yantrik-mind put the requirement
//! best, from the driver's side: *reflex must never wait for me, and I must never be the thing
//! that notices.* If a consumer has to poll to find out something happened, the boundary leaked.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::bridge::CompanionBridge;

/// How long each read parks waiting for something to happen.
///
/// Long, deliberately. The service caps it at 30s; asking for that means one wakeup per half
/// minute of complete silence, and none at all while events are flowing.
const WAIT: Duration = Duration::from_millis(30_000);

/// Salience at or above which an observation is worth remembering rather than merely showing.
///
/// The service's own scale: a source going blind is 1.0, a save 0.6, an exec 0.5, a launch 0.35,
/// a scratch write 0.15. So this admits saves, execs and real pressure, and leaves the constant
/// churn of process launches to the activity feed.
const MEMORABLE: f32 = 0.5;

/// Most memories a minute, regardless of how interesting the machine claims to be.
///
/// Not tuned — chosen. A build, a sync or an editor with autosave can each produce a legitimate
/// flood, and the point of a ceiling is that it holds when the input is legitimate.
const MEMORIES_PER_MINUTE: usize = 6;

/// Reconnect delay when the service is not there.
///
/// perception-service is optional and often absent: it needs `CAP_SYS_ADMIN` at startup - the
/// installers grant it as file capabilities on the binary (see build-debian-iso.sh) - and a
/// kernel with fanotify and Landlock. Its absence is normal and must not become a log flood.
const RETRY: Duration = Duration::from_secs(20);

#[derive(Debug, Deserialize)]
struct Page {
    observations: Vec<Observation>,
    next_seq: u64,
    /// How many fell off the ring before we read it. Never silent: a gap the reader knows about
    /// is recoverable, a gap it does not know about is a wrong answer delivered confidently.
    missed: u64,
}

#[derive(Debug, Deserialize)]
struct Observation {
    kind: Kind,
    #[serde(default)]
    actor: Option<Actor>,
    salience: f32,
    summary: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Kind {
    Saved { path: String, #[serde(default)] how: Option<String> },
    Wrote { path: String },
    Executed { path: String },
    Launched { command: String },
    Ended { exit_code: i32, signal: i32 },
    Pressure { resource: String, stalled_pct_10s: f32 },
    SourceFailed { source: String, reason: String },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Deserialize)]
struct Actor {
    #[serde(default)]
    name: String,
}

/// Whether an observation was caused by the OS itself.
///
/// The names are ours: every app binary is `yantrik-*`, every service ends in `-service`, and the
/// shell is `yantrik-ui`. An observation attributed to one of those is the body watching its own
/// hand move.
///
/// Deliberately conservative about what it claims. This does not say the event is uninteresting —
/// a service crashing is extremely interesting — only that it is *not news arriving from
/// outside*, and so should not be written into memory as though the world had acted.
fn is_ours(actor: Option<&Actor>) -> bool {
    let Some(name) = actor.map(|a| a.name.as_str()).filter(|n| !n.is_empty()) else {
        return false;
    };
    name.starts_with("yantrik") || name.ends_with("-service") || name == "labwc"
}

/// One line a person would actually want to read, or nothing.
///
/// Returning `None` is the common case and the point: most of what the kernel reports is true,
/// uninteresting, and would only dilute a store that has to stay searchable.
fn worth_remembering(o: &Observation) -> Option<(String, f64)> {
    match &o.kind {
        // A source going blind invalidates everything else it would have said, so it outranks
        // every ordinary event and is always kept.
        Kind::SourceFailed { .. } => Some((o.summary.clone(), 0.9)),

        // A document reaching disk is the strongest ordinary signal that a person did something.
        // `how` is carried through because "the kernel saw a rename" and "a writable descriptor
        // closed" are different degrees of certainty about the same claim.
        Kind::Saved { how, .. } => {
            let certainty = if how.as_deref() == Some("replaced") { 0.65 } else { 0.55 };
            Some((o.summary.clone(), certainty))
        }

        // Something ran from somewhere. Worth keeping; it is how an unexpected binary shows up.
        Kind::Executed { .. } => Some((o.summary.clone(), 0.55)),

        // Only pressure a person would actually feel. PSI below a fifth of wall time stalled is
        // a number, not an experience.
        Kind::Pressure { stalled_pct_10s, .. } if *stalled_pct_10s >= 30.0 => {
            Some((o.summary.clone(), 0.5))
        }

        // A process that died badly, and only badly. Ordinary exits are the machine working.
        Kind::Ended { exit_code, signal } if *exit_code != 0 || *signal != 0 => {
            Some((o.summary.clone(), 0.45))
        }

        // Everything else — launches, clean exits, scratch writes, quiet pressure — is shown and
        // forgotten. It is real, and it is not worth the room it would take in recall.
        _ => None,
    }
}

/// Start the reader. Returns immediately; the work happens on its own thread, for as long as the
/// shell runs.
///
/// Failing to start is not fatal and not even unusual — the OS runs perfectly well without
/// perception, it simply cannot see. That is said once at info level rather than repeated.
pub fn spawn(bridge: Arc<CompanionBridge>) {
    std::thread::Builder::new()
        .name("perception-reader".into())
        .spawn(move || run(bridge))
        .ok();
}

fn run(bridge: Arc<CompanionBridge>) {
    let address = yantrik_ipc_transport::server::RpcServer::default_address("perception");
    let mut cursor: u64 = 0;
    let mut announced = false;

    // Rate limiting state: a count and the minute it belongs to.
    let mut window_started = Instant::now();
    let mut admitted_this_minute = 0usize;
    let mut suppressed_this_minute = 0usize;

    loop {
        let client = yantrik_ipc_transport::SyncRpcClient::new(&address)
            .with_timeout(WAIT + Duration::from_secs(5));

        let page = client.call(
            "perception.since",
            serde_json::json!({ "seq": cursor, "wait_ms": WAIT.as_millis() as u64 }),
        );

        // A transport error and a reply that is not a page are the same thing here: nothing
        // readable came back, so wait and try again.
        let page: Page = match page.ok().and_then(|v| serde_json::from_value(v).ok()) {
            Some(p) => {
                if !announced {
                    tracing::info!(socket = %address, "Perception connected; the OS can see");
                    announced = true;
                }
                p
            }
            None => {
                // Absent is the normal case. Say it once, then keep quiet and keep trying.
                if announced {
                    tracing::warn!("Perception went away; retrying");
                    announced = false;
                }
                std::thread::sleep(RETRY);
                continue;
            }
        };

        // A gap is a fact about our own blindness and is recorded as one. It is the only thing
        // here that bypasses every filter below, because a reader that quietly lost events is
        // indistinguishable from a machine where nothing happened.
        if page.missed > 0 {
            tracing::warn!(missed = page.missed, "Perception ring overran; observations lost");
            bridge.record_system_event(
                format!(
                    "Perception lost {} observations before they could be read — \
                     the machine did more than is recorded for this period.",
                    page.missed
                ),
                "system/perception/gap".into(),
                0.7,
            );
        }

        cursor = page.next_seq;

        if window_started.elapsed() >= Duration::from_secs(60) {
            if suppressed_this_minute > 0 {
                // Counted, not silent — the same rule the service applies to its own eyelid.
                tracing::debug!(
                    suppressed = suppressed_this_minute,
                    "Perception observations held back by the rate ceiling"
                );
            }
            window_started = Instant::now();
            admitted_this_minute = 0;
            suppressed_this_minute = 0;
        }

        // There is no activity feed on this desktop to hand every observation to (#58): the
        // record of all of them is perception-service's own ring. What reaches the shell is
        // only what is worth searching later.
        for o in &page.observations {
            if is_ours(o.actor.as_ref()) {
                continue;
            }
            if o.salience < MEMORABLE {
                continue;
            }
            let Some((text, importance)) = worth_remembering(o) else { continue };

            if admitted_this_minute >= MEMORIES_PER_MINUTE {
                suppressed_this_minute += 1;
                continue;
            }
            admitted_this_minute += 1;
            // Under `system/`: this is the machine's telemetry, stored with source `system`
            // beside the person's memories and not counted as theirs (#31). A domain outside
            // `system/` would be rewritten to `system/general` by the bridge anyway.
            bridge.record_system_event(text, "system/perception".into(), importance);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(kind: Kind, salience: f32, actor: Option<&str>) -> Observation {
        Observation {
            summary: "a thing happened".into(),
            kind,
            salience,
            actor: actor.map(|n| Actor { name: n.into() }),
        }
    }

    #[test]
    fn the_os_watching_its_own_hand_is_not_news() {
        // The measured case. On the deployed VM the perception ring held 1120 observations and
        // almost all of them were the operator's own session plus the OS's own processes. Fed to
        // memory raw, the store becomes a diary of its own administration.
        for ours in ["yantrik-ui", "yantrik-notes", "perception-service", "a11y-service", "labwc"] {
            assert!(is_ours(Some(&Actor { name: ours.into() })), "{ours} is us");
        }
        for theirs in ["vim", "soffice", "chromium", "cargo", "sshd"] {
            assert!(!is_ours(Some(&Actor { name: theirs.into() })), "{theirs} is not us");
        }
    }

    #[test]
    fn an_unattributed_observation_is_treated_as_foreign() {
        // The safe direction. A process that exited before /proc could be read leaves no name,
        // and treating an unknown actor as "probably ours" would silently drop real events.
        assert!(!is_ours(None));
        assert!(!is_ours(Some(&Actor { name: String::new() })));
    }

    #[test]
    fn a_save_is_remembered_and_carries_its_certainty() {
        let replaced = obs(
            Kind::Saved { path: "/home/p/report.odt".into(), how: Some("replaced".into()) },
            0.6,
            Some("soffice"),
        );
        let closed = obs(
            Kind::Saved { path: "/home/p/notes.md".into(), how: Some("closed_write".into()) },
            0.6,
            Some("nano"),
        );

        let (_, sure) = worth_remembering(&replaced).expect("a rename-save is memorable");
        let (_, less) = worth_remembering(&closed).expect("an in-place save is memorable");
        assert!(
            sure > less,
            "the kernel seeing a directory entry change is a stronger claim than a descriptor \
             closing, and the memory should carry that difference"
        );
    }

    #[test]
    fn the_machinery_of_a_save_is_shown_and_forgotten() {
        // A scratch write is true, and recording it would put an editor's swap file in the
        // user's memory next to their conversations.
        let scratch = obs(Kind::Wrote { path: "/home/p/.report.odt.swp".into() }, 0.15, Some("vim"));
        assert!(worth_remembering(&scratch).is_none());
    }

    #[test]
    fn launches_and_clean_exits_do_not_reach_memory() {
        // The bulk of the firehose. Every one of these is real and none is worth searching for.
        assert!(worth_remembering(&obs(Kind::Launched { command: "ls -a".into() }, 0.35, Some("ls"))).is_none());
        assert!(worth_remembering(&obs(Kind::Ended { exit_code: 0, signal: 0 }, 0.1, Some("ls"))).is_none());
        // A bad exit is different: that is how a failure becomes findable later.
        assert!(worth_remembering(&obs(Kind::Ended { exit_code: 1, signal: 0 }, 0.45, Some("cargo"))).is_some());
        assert!(worth_remembering(&obs(Kind::Ended { exit_code: 0, signal: 9 }, 0.45, Some("cargo"))).is_some());
    }

    #[test]
    fn a_blind_source_outranks_everything_and_is_always_kept() {
        // The one observation that invalidates all the others. A perception system that has
        // quietly gone blind must never look like one where nothing is happening.
        let blind = obs(
            Kind::SourceFailed { source: "files".into(), reason: "EPERM".into() },
            1.0,
            None,
        );
        let (_, importance) = worth_remembering(&blind).expect("always kept");
        let (_, save) = worth_remembering(&obs(
            Kind::Saved { path: "/x".into(), how: Some("replaced".into()) },
            0.6,
            Some("vim"),
        ))
        .unwrap();
        assert!(importance > save);
    }

    #[test]
    fn a_page_as_perception_service_writes_it_reads() {
        // The shape `perception.since` answers with (services/perception-service/src/bus.rs and
        // observation.rs). This module sat uncompiled for as long as it did (#58) partly because
        // nothing held it to the service; a field renamed there now fails here.
        let page: Page = serde_json::from_value(serde_json::json!({
            "observations": [
                {
                    "seq": 7, "at": 1790213170.5,
                    "kind": { "type": "saved", "path": "/home/p/report.odt", "how": "replaced" },
                    "actor": { "pid": 4242, "name": "soffice", "parent": 1 },
                    "salience": 0.6,
                    "summary": "soffice saved report.odt"
                },
                {
                    "seq": 8, "at": 1790213171.0,
                    "kind": { "type": "source_failed", "source": "files", "reason": "EPERM" },
                    "salience": 1.0,
                    "summary": "file watching stopped: EPERM"
                }
            ],
            "next_seq": 9,
            "missed": 0
        }))
        .expect("the service's page deserializes");
        assert_eq!(page.next_seq, 9);
        assert!(matches!(&page.observations[0].kind, Kind::Saved { how: Some(h), .. } if h == "replaced"));
        assert_eq!(page.observations[0].actor.as_ref().map(|a| a.name.as_str()), Some("soffice"));
        assert!(page.observations[1].actor.is_none());
        assert!(worth_remembering(&page.observations[1]).is_some());
    }

    #[test]
    fn quiet_pressure_is_not_an_experience() {
        // PSI reports a number continuously. Only the part a person would feel is worth keeping.
        let quiet = obs(Kind::Pressure { resource: "cpu".into(), stalled_pct_10s: 5.0 }, 0.1, None);
        let felt = obs(Kind::Pressure { resource: "cpu".into(), stalled_pct_10s: 60.0 }, 0.8, None);
        assert!(worth_remembering(&quiet).is_none());
        assert!(worth_remembering(&felt).is_some());
    }
}
