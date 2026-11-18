//! What the app is: the library as last read, what the editor and the selection hold, and the
//! record of the browser jobs. The library itself is on disk and the lane's (`yantrik_app::lane`); the model
//! holds the last listing the lane read, so `describe` and the window never touch the disk.

use std::sync::Arc;

use serde_json::Value;

use crate::command::Listing;
use yantrik_app::lane::Lane;
use crate::library::{GameEntry, Library};
use crate::slow::Slow;

/// The parts of the window `render` draws separately.
pub const PART_LISTS: &str = "lists";
pub const PART_NOTICE: &str = "notice";
pub const PART_EDITOR: &str = "editor";

/// The name of the draft that holds the spec editor's unsaved text.
pub const DRAFT: &str = "spec";

/// How many browser jobs run at once. The framework's pool has four threads and a job here holds
/// one for as long as the browser takes; two leave the others for the library's commands.
pub const SLOW_AT_ONCE: usize = 2;

#[derive(Clone)]
pub enum JobStatus {
    Running,
    Done(String),
    Failed(String),
}

#[derive(Clone)]
pub struct Job {
    pub id: u64,
    pub action: String,
    pub subject: String,
    pub status: JobStatus,
}

/// The record of slow work, newest last: a glance, not a transcript.
#[derive(Default)]
pub struct Jobs {
    list: Vec<Job>,
    next_id: u64,
}

impl Jobs {
    pub fn start(&mut self, action: &str, subject: &str) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        self.list.push(Job { id, action: action.into(), subject: subject.into(), status: JobStatus::Running });
        // The list is a glance, not a transcript: keep the newest eight.
        while self.list.len() > 8 {
            self.list.remove(0);
        }
        id
    }

    pub fn finish(&mut self, id: u64, result: Result<String, String>) {
        if let Some(job) = self.list.iter_mut().find(|j| j.id == id) {
            job.status = match result {
                Ok(msg) => JobStatus::Done(msg),
                Err(msg) => JobStatus::Failed(msg),
            };
        }
    }

    pub fn running(&self) -> Vec<&Job> {
        self.list.iter().filter(|j| matches!(j.status, JobStatus::Running)).collect()
    }

    /// Ends every running job as failed, with `why`: the app is closing, and no browser is left
    /// to answer. Returns how many.
    pub fn fail_running(&mut self, why: &str) -> usize {
        let mut n = 0;
        for job in self.list.iter_mut().filter(|j| matches!(j.status, JobStatus::Running)) {
            job.status = JobStatus::Failed(why.to_string());
            n += 1;
        }
        n
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn json(&self) -> Value {
        Value::Array(
            self.list
                .iter()
                .rev()
                .map(|j| {
                    let (status, detail) = match &j.status {
                        JobStatus::Running => ("running", String::new()),
                        JobStatus::Done(m) => ("done", m.clone()),
                        JobStatus::Failed(m) => ("failed", m.clone()),
                    };
                    serde_json::json!({
                        "id": j.id, "action": j.action, "subject": j.subject,
                        "status": status, "detail": detail,
                    })
                })
                .collect(),
        )
    }
}

pub struct Model {
    pub library: Library,
    pub lane: Lane<Library>,
    pub slow: Arc<dyn Slow>,
    pub listing: Listing,
    /// The order of the last command asked, and of the last listing applied.
    pub asked: u64,
    pub applied: u64,
    /// Commands on the lane that have not landed.
    pub work: usize,
    pub reload_asked: bool,
    /// Browser jobs asked of the lane that have not landed, so the limit counts them too.
    pub slow_asked: usize,
    pub jobs: Jobs,

    // ── What the window is set to show ──
    pub spec_text: String,
    /// The text the library last kept (or loaded) for the editor; text that differs is unsaved.
    pub kept_text: String,
    pub game_name: String,
    pub notice: String,
    /// Whether a draft of the editor's text is in the draft lane.
    pub drafted: bool,

    /// Drawn for a Mind (Mind View): the window is read-only and the person's draft stays hidden.
    pub for_mind: bool,

    // ── The app's life ──
    pub closing: bool,
    pub ending: bool,
    /// The answer the intent being settled is owed, written in the command that settles it.
    pub answer: Value,
}

impl Model {
    /// Whether a command is on the lane: what a close has to wait for.
    pub fn busy(&self) -> bool {
        self.work > 0
    }

    /// Whether the editor holds text the library does not.
    pub fn unsaved(&self) -> bool {
        !self.spec_text.trim().is_empty() && self.spec_text != self.kept_text
    }

    /// The status line under the title.
    pub fn status(&self) -> String {
        match self.jobs.running().first() {
            Some(job) => format!("{} {} running\u{2026}", job.action, job.subject),
            None => format!("{} games, {} characters", self.listing.games.len(), self.listing.characters.len()),
        }
    }
}

/// A game row's second line: what exists on disk, terse.
pub fn detail_line(g: &GameEntry) -> String {
    if !g.built {
        return "not built".into();
    }
    match (&g.last_build, &g.verification) {
        (Some(when), Some(v)) => format!("built {when} \u{b7} {v}"),
        (Some(when), None) => format!("built {when} \u{b7} not verified"),
        (None, Some(v)) => v.clone(),
        (None, None) => "built".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jobs_start_running_and_land() {
        let mut jobs = Jobs::default();
        let id = jobs.start("verify", "meadow-run");
        assert_eq!(jobs.running().len(), 1);
        assert_eq!(jobs.running()[0].action, "verify");
        jobs.finish(id, Ok("passed all 7 gates".into()));
        assert!(jobs.running().is_empty());
        assert_eq!(jobs.json()[0]["status"], "done");
        assert_eq!(jobs.json()[0]["detail"], "passed all 7 gates");
    }

    #[test]
    fn a_failed_job_says_failed() {
        let mut jobs = Jobs::default();
        let id = jobs.start("play", "x");
        jobs.finish(id, Err("the Browser did not answer".into()));
        assert_eq!(jobs.json()[0]["status"], "failed");
        assert_eq!(jobs.json()[0]["detail"], "the Browser did not answer");
    }

    #[test]
    fn the_job_list_stays_a_glance() {
        let mut jobs = Jobs::default();
        for i in 0..20 {
            let id = jobs.start("verify", &format!("game-{i}"));
            jobs.finish(id, Ok("ok".into()));
        }
        assert!(jobs.len() <= 8);
    }

    #[test]
    fn game_rows_say_what_exists_on_disk() {
        let unbuilt = GameEntry { slug: "a".into(), title: "A".into(), built: false, last_build: None, verification: None };
        assert_eq!(detail_line(&unbuilt), "not built");
        let built = GameEntry {
            slug: "a".into(),
            title: "A".into(),
            built: true,
            last_build: Some("2026-09-22 10:00".into()),
            verification: Some("verified 2026-09-22 10:05: passed all 7 gates".into()),
        };
        assert_eq!(detail_line(&built), "built 2026-09-22 10:00 \u{b7} verified 2026-09-22 10:05: passed all 7 gates");
        let stale = GameEntry { verification: None, ..built };
        assert_eq!(detail_line(&stale), "built 2026-09-22 10:00 \u{b7} not verified");
    }
}
