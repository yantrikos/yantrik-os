//! Yantrik Perception — the kernel's account of what is happening, and who did it.
//!
//! # Why this exists
//!
//! The companion's only eyesight was a photograph: `grim` takes a screenshot, the PNG is base64'd
//! and posted to a vision model, and the model reports what the pixels look like. That works, and
//! it is the wrong shape for a continuous sense. It costs a GPU we do not always have, it is
//! current only as of the instant it was taken, and — the part that matters — it has no idea
//! *when* to fire. Every look costs the same whether the screen changed or not.
//!
//! Eyes do not work that way. The fovea is about two degrees wide; everything outside it is a
//! cheap change detector whose entire job is to say *look there*. Yantrik had only a fovea.
//!
//! This is the periphery, and it lives below the compositor because that is where the answers
//! are. The kernel cannot give us pixels — wlroots owns those — but it can give us causality:
//! which program started, what it opened, what it wrote, and how hard the machine is working.
//! Most questions a companion actually gets asked are answered outright by that, and the ones
//! that are not now come with a timestamp and a reason to look.
//!
//! # What it serves
//!
//! ```text
//! perception.since    { seq, wait_ms? }  → { observations, next_seq, missed }
//! perception.snapshot { }                → { counts, sources, scope, uptime }
//! perception.scope    { }                → what it may see, and what the kernel is enforcing
//! ```
//!
//! `since` is a long poll: a caller with nothing to read parks until something happens or its
//! patience runs out. Returning immediately would push the polling one layer up and give back
//! the entire saving.
//!
//! # Why it is privileged, and what holds it
//!
//! Joining the process connector needs `CAP_NET_ADMIN`; `fanotify_init` needs `CAP_SYS_ADMIN`.
//! Nothing else in Yantrik runs with either, and the shell must never.
//!
//! So the restraint is in the kernel too. Before a single source thread starts, this process asks
//! Landlock to hold it to its scope — read-only, a listed set of directories — and Landlock is
//! not bypassed by root or by those capabilities. What that leaves is the asymmetry the whole
//! design rests on:
//!
//! > It can be told that you saved a file. It cannot open it and read what you wrote.
//!
//! Better still, it does not keep them. Both capabilities are needed exactly once, to obtain two
//! descriptors: one netlink socket and one fanotify fd. After that this is a program that reads
//! from two descriptors, so it hands the capabilities back. Startup is therefore, in order:
//!
//!   1. open the two descriptors, while privileged
//!   2. apply the Landlock ruleset
//!   3. drop every capability, irreversibly
//!   4. only then spawn the source threads and serve
//!
//! Every step of that ordering is load-bearing. Landlock and capabilities are both per-thread and
//! inherited at creation, never retrofitted onto threads already running — a source thread
//! started before step 3 would keep `CAP_SYS_ADMIN` for the life of the process. And the
//! descriptors have to come first, because after step 3 they can no longer be opened.
//!
//! The result is checkable from outside, which is the only kind of claim worth making here:
//! `CapEff` in `/proc/<pid>/status` reads all zeroes while the service goes on watching.

mod bus;
mod caps;
mod observation;
mod scope;
mod sources {
    pub mod files;
    pub mod pressure;
    pub mod proc_events;
}

use std::time::{Duration, Instant};

use yantrik_service_sdk::prelude::*;

use bus::{Bus, MAX_WAIT};
use observation::Kind;

fn main() {
    // Before anything else. Everything that matters here — the descriptors, the ruleset, the
    // capability drop — happens before the RPC server starts, and a startup that logs nothing is
    // a startup whose failures are invisible.
    yantrik_service_sdk::init_tracing("perception");

    let policy = match scope::Scope::load() {
        Ok(p) => p,
        Err(e) => {
            // A policy that does not parse is not a reason to fall back to our own idea of one.
            // Someone wrote down what this may watch and we cannot read it; starting anyway would
            // be the worst outcome available.
            eprintln!("perception: {e}");
            std::process::exit(1);
        }
    };
    let mut scope = policy.resolve();
    let bus = Bus::new();
    let started = Instant::now();

    // ── 1. While privileged: open the descriptors ──
    //
    // Failures are recorded rather than fatal. A machine where the connector is unavailable still
    // has PSI and file events worth having, and the failure reaches the caller as an observation
    // instead of a log line nobody reads.
    let proc_socket = match sources::proc_events::subscribe() {
        Ok(fd) => Some(fd),
        Err(e) => {
            bus.push(Kind::SourceFailed { source: "processes".into(), reason: e.clone() }, None);
            tracing::warn!(error = %e, "Process connector unavailable, no launch or exit events");
            None
        }
    };
    let fan = match sources::files::init_and_mark(&scope) {
        Ok((fd, marks)) => {
            tracing::info!(marks, "fanotify watching");
            Some(fd)
        }
        Err(e) => {
            bus.push(Kind::SourceFailed { source: "files".into(), reason: e.clone() }, None);
            tracing::warn!(error = %e, "fanotify unavailable, no save or exec events");
            None
        }
    };

    // ── 2. Close the eyelid ──
    //
    // The socket directory is created here rather than by the server, because after this call we
    // may only write inside it and creating it would be a write to its parent.
    let socket_dir = yantrik_ipc_transport::server::socket_dir();
    scope.restrict_self(&socket_dir);
    // And then walk into it, because a ruleset the kernel accepted is not the same thing as a
    // ruleset that denies anything.
    scope.verify();

    // ── 3. Hand the capabilities back ──
    let dropped = match caps::drop_all() {
        Ok(()) => {
            tracing::info!(capabilities = %caps::effective(), "Capabilities dropped");
            caps::none_held()
        }
        Err(e) => {
            tracing::warn!(error = %e, "Could not drop capabilities");
            false
        }
    };

    // ── 4. Now, and only now, start reading ──
    if let Some(fd) = proc_socket {
        let bus = bus.clone();
        std::thread::Builder::new()
            .name("perception-proc".into())
            .spawn(move || sources::proc_events::run(bus, fd))
            .ok();
    }
    {
        let bus = bus.clone();
        std::thread::Builder::new()
            .name("perception-pressure".into())
            .spawn(move || sources::pressure::run(bus))
            .ok();
    }
    if let Some(fd) = fan {
        let bus = bus.clone();
        // Cloned rather than borrowed: the source needs the scope for the life of the thread, and
        // the main thread is about to block in the RPC server.
        let watch = scope.clone();
        std::thread::Builder::new()
            .name("perception-files".into())
            .spawn(move || sources::files::run(bus, &watch, fd))
            .ok();
    }

    ServiceBuilder::new("perception")
        .handler(Perception { bus, scope, started, dropped })
        .run();
}

struct Perception {
    bus: Bus,
    scope: scope::Resolved,
    started: Instant,
    /// Whether every capability was actually given back. Reported, not assumed: a drop that
    /// silently failed would leave a privileged daemon describing itself as an unprivileged one.
    dropped: bool,
}

impl ServiceHandler for Perception {
    fn service_id(&self) -> &str {
        "perception"
    }

    fn handle(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, ServiceError> {
        match method {
            "perception.since" => {
                let since = params["seq"].as_u64().unwrap_or(0);
                // Zero is a legitimate ask — "what do you have right now" — so it is not
                // defaulted away. The ceiling stops a client pinning a connection thread.
                let wait = Duration::from_millis(params["wait_ms"].as_u64().unwrap_or(0))
                    .min(MAX_WAIT);

                let page = self.bus.since(since, wait);
                Ok(serde_json::json!({
                    "observations": page.observations,
                    "next_seq": page.next_seq,
                    // Non-zero means this caller fell off the back of the ring. Reported rather
                    // than papered over: a gap you can see is a different thing from one you
                    // cannot.
                    "missed": page.missed,
                }))
            }

            "perception.snapshot" => {
                let counts = self.bus.counts();
                Ok(serde_json::json!({
                    "counts": counts,
                    "uptime_seconds": self.started.elapsed().as_secs(),
                    // Its own, so nothing outside has to find it in a process list. `pgrep -f`
                    // matching the binary path finds the `sudo` that launched it just as readily,
                    // and reads that process's capabilities instead of ours.
                    "pid": std::process::id(),
                    "scope": self.scope_report(),
                }))
            }

            // Deliberately its own method, and deliberately answerable by anyone who can reach the
            // socket. A component that watches should be able to say exactly what it may see;
            // otherwise the policy is something the user has to take on trust.
            "perception.scope" => Ok(self.scope_report()),

            other => Err(ServiceError {
                code: -32601,
                message: format!(
                    "unknown method `{other}`; this service serves \
                     perception.since, perception.snapshot, perception.scope"
                ),
            }),
        }
    }
}

impl Perception {
    fn scope_report(&self) -> serde_json::Value {
        serde_json::json!({
            "watching": self.scope.watch.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
            "never": self.scope.never.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
            // The important line. It says what the kernel is actually enforcing, including when
            // the answer is "nothing" — a restriction that silently failed to apply would be
            // worse than one that was never claimed.
            "enforcement": self.scope.enforcement,
            "verified": self.scope.verified,
            "capabilities": if self.dropped {
                "none: CAP_NET_ADMIN and CAP_SYS_ADMIN were dropped after startup".to_string()
            } else {
                format!("still held: {}", caps::effective())
            },
            "also_readable": ["/proc, needed to name the process behind an observation"],
            "note": "Reports that a file was saved; cannot open it. Landlock is read-only \
                     and applies before any source thread starts.",
        })
    }
}
