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
    /// A document now holds new content. The inferred fact, not the raw one: [`SaveShape`] says
    /// which observation established it, because "a writable descriptor closed" and "a name was
    /// renamed over the document" are different things that both mean a person saved.
    Saved { path: String, how: SaveShape },
    /// A writable descriptor was closed on a file that is not a document — an editor's swap file,
    /// a lock, a partial download.
    ///
    /// Kept rather than dropped. This is the machinery of a save, and the rename that follows is
    /// the event; but a scratch write with no rename after it is the shape of a crashed editor,
    /// and a perception service that had silently discarded it could not say so.
    Wrote { path: String },
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

/// How we came to believe a document was saved.
///
/// Carried on the observation rather than resolved away, because the two have different
/// confidence and a reader is entitled to know which one it got.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SaveShape {
    /// A writable descriptor was closed, and the inode behind it is now at this path.
    ///
    /// Deliberately *not* called `InPlace`, because it does not establish that. fanotify hands
    /// over a descriptor and the path is read afterwards from `/proc/self/fd`, which resolves to
    /// whatever that inode is called *at the moment we look* — so a scratch file already renamed
    /// over the document resolves to the document, and one not yet renamed resolves to the
    /// scratch file. Which happens is a race with the editor. The name is usually right; the
    /// mechanism is unknown; and the variant claims exactly that much.
    ClosedWrite,
    /// `FAN_MOVED_TO`: a directory entry now points at something else — the rename that every
    /// serious editor's save ends with. The only deterministic statement of the two, and the one
    /// the first version of this service could not make at all.
    Replaced,
}

/// What a path or a command line that names a protected file is shown as (review R11-H3).
const A_PROTECTED_FILE: &str = "a protected file";
const A_PROTECTED_COMMAND: &str = "a command that names a protected file";

impl Observation {
    /// An observation as every reader gets it: a path in a protected place or with a secret's name,
    /// and a command line that names one, are replaced by what they are, here and not at each
    /// reader, since the line goes to the harnesses' `os_perception` (read-only, no card), to the
    /// bus's subscribers and to memory. The kind and the salience are kept.
    pub fn new(seq: u64, kind: Kind, actor: Option<Actor>) -> Self {
        let kind = kind.without_protected_names();
        let salience = kind.salience();
        let summary = kind.summary(actor.as_ref());
        Self { seq, at: now(), kind, actor, salience, summary }
    }
}

impl Kind {
    fn without_protected_names(self) -> Self {
        use yantrik_ipc_contracts::home_paths::{is_protected, title_names_a_secret};
        let shown = |path: String| if is_protected(std::path::Path::new(&path)) { A_PROTECTED_FILE.to_string() } else { path };
        match self {
            Kind::Launched { command } if title_names_a_secret(&command) => Kind::Launched { command: A_PROTECTED_COMMAND.to_string() },
            Kind::Saved { path, how } => Kind::Saved { path: shown(path), how },
            Kind::Wrote { path } => Kind::Wrote { path: shown(path) },
            Kind::Executed { path } => Kind::Executed { path: shown(path) },
            other => other,
        }
    }

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
            // The same syscall, and almost never worth waking anything for: a swap file being
            // written is not news. Low rather than zero so it is still visible to anyone looking
            // at the record on purpose — which is how you find out an editor died mid-save.
            Kind::Wrote { .. } => 0.15,
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
            Kind::Saved { path, .. } => match who {
                Some(name) => format!("{name} saved {path}"),
                None => format!("{path} was saved"),
            },
            // Deliberately not the word "saved". This line ends up in memory next to
            // conversations, and the difference between "vim saved report.odt" and "vim wrote
            // .report.odt.swp" is the difference between an event and a noise the machine makes.
            Kind::Wrote { path } => match who {
                Some(name) => format!("{name} wrote {path}"),
                None => format!("{path} was written"),
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

    /// R11-H3: the summaries and the kinds a harness's `os_perception` returns name no protected file.
    #[test]
    fn an_observation_of_a_protected_file_names_none_of_it() {
        let actor = || Some(Actor { pid: 1, name: "nvim".into(), parent: None });
        for kind in [
            Kind::Launched { command: "nvim /home/p/backup/tokens.json".into() },
            Kind::Saved { path: "/home/p/.kube/config".into(), how: SaveShape::Replaced },
            Kind::Wrote { path: "/home/p/.local/state/nvim/swap/%home%p%backup%tokens.json.swp".into() },
            Kind::Executed { path: "/home/p/.hermes.yantrik-moved/run.sh".into() },
        ] {
            let o = Observation::new(1, kind, actor());
            let all = format!("{} {}", o.summary, serde_json::to_string(&o).unwrap_or_default());
            for word in ["tokens", "kube", "hermes", "%home%"] {
                assert!(!all.contains(word), "{word}: {all}");
            }
            assert!(all.contains("a protected file") || all.contains("names a protected file"), "{all}");
        }
        let saved = Observation::new(2, Kind::Saved { path: "/home/p/report.odt".into(), how: SaveShape::Replaced }, actor());
        assert_eq!(saved.summary, "nvim saved /home/p/report.odt");
        let o = Observation::new(3, Kind::Launched { command: "nvim /home/p/notes.md".into() }, None);
        assert_eq!(o.summary, "started: nvim /home/p/notes.md");
        assert!(saved.salience > o.salience, "the kind and the salience are kept");
    }

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
        let saved = Kind::Saved { path: "/home/p/notes.md".into(), how: SaveShape::ClosedWrite };
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
        let o = Observation::new(
            1,
            Kind::Saved { path: "/tmp/x.rs".into(), how: SaveShape::Replaced },
            Some(actor),
        );
        assert_eq!(o.summary, "cargo saved /tmp/x.rs");

        // And does not invent one when the process was already gone.
        let o = Observation::new(
            2,
            Kind::Saved { path: "/tmp/x.rs".into(), how: SaveShape::ClosedWrite },
            None,
        );
        assert_eq!(o.summary, "/tmp/x.rs was saved");
    }

    #[test]
    fn a_scratch_write_does_not_claim_to_be_a_save() {
        // The bug this pair of variants exists to prevent. `FAN_CLOSE_WRITE` on `.report.odt.swp`
        // used to be reported as "vim saved .report.odt.swp" — a sentence that is wrong about the
        // verb and about the file, and the only sentence the old code could produce for the way
        // almost every editor actually saves.
        let actor = Actor { pid: 7, name: "vim".into(), parent: None };
        let scratch = Observation::new(
            1,
            Kind::Wrote { path: "/home/p/.report.odt.swp".into() },
            Some(actor.clone()),
        );
        assert_eq!(scratch.summary, "vim wrote /home/p/.report.odt.swp");
        assert!(!scratch.summary.contains("saved"));

        let real = Observation::new(
            2,
            Kind::Saved { path: "/home/p/report.odt".into(), how: SaveShape::Replaced },
            Some(actor),
        );
        assert_eq!(real.summary, "vim saved /home/p/report.odt");
    }

    #[test]
    fn the_machinery_of_a_save_ranks_far_below_the_save() {
        // Both come from the same syscall. If they scored alike, an editor with autosave would
        // wake the expensive faculties every thirty seconds for a swap file.
        let scratch = Kind::Wrote { path: "/home/p/.notes.md.swp".into() };
        let saved = Kind::Saved { path: "/home/p/notes.md".into(), how: SaveShape::Replaced };
        assert!(scratch.salience() < 0.2);
        assert!(saved.salience() > scratch.salience() * 3.0);
    }

    #[test]
    fn how_a_save_was_established_survives_the_wire() {
        // A reader is entitled to know whether it got the near-certain fact or the judgement.
        let o = Observation::new(
            1,
            Kind::Saved { path: "/home/p/report.odt".into(), how: SaveShape::Replaced },
            None,
        );
        let json = serde_json::to_value(&o).unwrap();
        assert_eq!(json["kind"]["type"], "saved");
        assert_eq!(json["kind"]["how"], "replaced");
    }
}
