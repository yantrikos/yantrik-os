//! What the kernel saw.
//!
//! One flat type rather than a hierarchy, because everything downstream — the event bus, the
//! companion's memory, an instinct deciding whether to speak — wants to ask the same three
//! questions of every observation: what happened, who did it, and does it matter.
//!
//! `salience` is the answer to the third, and it is why this service exists. A screenshot has no
//! salience: it is 8 megabytes that must be looked at to find out whether it was worth looking at.
//! An observation carries its own answer, so the expensive faculties can be spent only where the
//! cheap one pointed.

use serde::{Deserialize, Serialize};

/// A single thing the kernel reported.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    /// Monotonic within this run. A caller asks for everything after the last one it saw, which
    /// makes a dropped connection cost nothing but a gap it can detect.
    pub seq: u64,
    /// Unix seconds. Not the kernel's monotonic clock: these end up in memory next to
    /// conversations, and a timestamp nobody can compare to a wall clock is not worth storing.
    pub at: f64,
    pub kind: Kind,
    /// Who did it, when the kernel could say. This is the whole point of watching from down here:
    /// inotify reports that a file changed, and the kernel reports who changed it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<Actor>,
    /// 0.0 – 1.0. How much this deserves the attention of something expensive.
    pub salience: f32,
    /// One line, already readable. The companion should not have to render these.
    pub summary: String,
}

/// The process behind an observation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Actor {
    pub pid: i32,
    /// From `/proc/<pid>/comm`, read after the fact — a process that exited first leaves this
    /// empty rather than making the observation a lie.
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Kind {
    /// A program started. From the netlink process connector, which pushes rather than being
    /// polled, so nothing short-lived is missed between ticks.
    Launched { command: String },
    /// A program ended, and how.
    Ended { exit_code: i32, signal: i32 },
    /// Someone finished writing a file. `FAN_CLOSE_WRITE`, not "modified" — a save, not a
    /// keystroke, which is the difference between a signal and a flood.
    Saved { path: String },
    /// Something was executed from a path. `FAN_OPEN_EXEC` catches a script or binary being run
    /// from a place a program does not normally run from.
    Executed { path: String },
    /// The machine is struggling, in the terms it actually feels it: PSI reports the fraction of
    /// wall time in which work was stalled waiting for a resource, which is what "slow" means to
    /// the person at the keyboard. A CPU percentage does not.
    Pressure { resource: String, stalled_pct_10s: f32 },
    /// A source could not start, or stopped. Reported rather than logged: a perception system
    /// that has quietly gone blind must never look the same as one that sees nothing happening.
    SourceFailed { source: String, reason: String },
}

impl Observation {
    pub fn new(seq: u64, kind: Kind, actor: Option<Actor>) -> Self {
        let salience = kind.salience();
        let summary = kind.summary(actor.as_ref());
        Self { seq, at: now(), kind, actor, salience, summary }
    }
}

impl Kind {
    /// How much this is worth waking something expensive for.
    ///
    /// These are starting weights, not a model. The number that matters is the *relative* order:
    /// a save is worth more than a launch, a launch is worth more than an exit, and a source
    /// going blind outranks everything because it invalidates all the rest.
    fn salience(&self) -> f32 {
        match self {
            Kind::SourceFailed { .. } => 1.0,
            // Work reaching disk is the strongest ordinary signal that something happened that a
            // person would describe as an event.
            Kind::Saved { .. } => 0.6,
            Kind::Executed { .. } => 0.5,
            Kind::Launched { .. } => 0.35,
            // Only pressure that is actually being felt. Below a fifth of wall time stalled,
            // nobody notices, and an observation nobody would notice is noise.
            Kind::Pressure { stalled_pct_10s, .. } => {
                if *stalled_pct_10s >= 50.0 {
                    0.8
                } else if *stalled_pct_10s >= 20.0 {
                    0.5
                } else {
                    0.1
                }
            }
            Kind::Ended { exit_code, .. } if *exit_code != 0 => 0.45,
            Kind::Ended { .. } => 0.1,
        }
    }

    fn summary(&self, actor: Option<&Actor>) -> String {
        let who = actor.map(|a| a.name.as_str()).filter(|n| !n.is_empty());
        match self {
            Kind::Launched { command } => match who {
                Some(name) => format!("{name} started: {command}"),
                None => format!("started: {command}"),
            },
            Kind::Ended { exit_code, signal } if *signal != 0 => match who {
                Some(name) => format!("{name} was killed by signal {signal}"),
                None => format!("a process was killed by signal {signal}"),
            },
            Kind::Ended { exit_code, .. } if *exit_code != 0 => match who {
                Some(name) => format!("{name} exited with {exit_code}"),
                None => format!("a process exited with {exit_code}"),
            },
            Kind::Ended { .. } => match who {
                Some(name) => format!("{name} finished"),
                None => "a process finished".to_string(),
            },
            Kind::Saved { path } => match who {
                Some(name) => format!("{name} saved {path}"),
                None => format!("{path} was saved"),
            },
            Kind::Executed { path } => match who {
                Some(name) => format!("{name} ran {path}"),
                None => format!("{path} was run"),
            },
            Kind::Pressure { resource, stalled_pct_10s } => {
                format!("{resource} stalled {stalled_pct_10s:.0}% of the last 10s")
            }
            Kind::SourceFailed { source, reason } => {
                format!("perception cannot use its {source} source: {reason}")
            }
        }
    }
}

pub fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lost_source_says_which_one_and_why() {
        let o = Observation::new(
            1,
            Kind::SourceFailed { source: "processes".into(), reason: "needs CAP_NET_ADMIN".into() },
            None,
        );
        assert!(o.summary.contains("processes"));
        assert!(o.summary.contains("CAP_NET_ADMIN"));
    }

    #[test]
    fn a_source_going_blind_outranks_everything() {
        let blind = Kind::SourceFailed { source: "files".into(), reason: "EPERM".into() };
        let saved = Kind::Saved { path: "/home/p/notes.md".into() };
        assert!(
            blind.salience() > saved.salience(),
            "a perception system that has stopped seeing must never rank below what it can still see"
        );
    }

    #[test]
    fn pressure_nobody_would_feel_is_not_worth_reporting_loudly() {
        let quiet = Kind::Pressure { resource: "cpu".into(), stalled_pct_10s: 3.0 };
        let real = Kind::Pressure { resource: "cpu".into(), stalled_pct_10s: 60.0 };
        assert!(quiet.salience() < 0.2);
        assert!(real.salience() > 0.7);
    }

    #[test]
    fn a_failed_exit_matters_more_than_a_clean_one() {
        let bad = Kind::Ended { exit_code: 1, signal: 0 };
        let good = Kind::Ended { exit_code: 0, signal: 0 };
        assert!(bad.salience() > good.salience());
    }

    #[test]
    fn the_summary_names_the_actor_when_there_is_one() {
        let actor = Actor { pid: 42, name: "cargo".into(), parent: None };
        let o = Observation::new(1, Kind::Saved { path: "/tmp/x.rs".into() }, Some(actor));
        assert_eq!(o.summary, "cargo saved /tmp/x.rs");

        // And does not invent one when the process was already gone.
        let o = Observation::new(2, Kind::Saved { path: "/tmp/x.rs".into() }, None);
        assert_eq!(o.summary, "/tmp/x.rs was saved");
    }
}
