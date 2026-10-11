//! Report a problem: the records this desktop wrote when something went wrong, and the one act
//! that sends one of them anywhere.
//!
//! The records are written by `yantrik_app_runtime::problems` — by an app's own panic hook, or by
//! the launcher's reaper when a child dies badly. This module reads them, shows one in full, and
//! sends it only when the person presses Send (or a mind calls `report_problem`, which is graded
//! `sensitive` and so raises a card). The intake it sends to holds the project's GitHub token;
//! nothing on this machine does. See `design/problem-reports-2026-09-23.md`.

use std::path::PathBuf;
use std::time::Duration;

use slint::{ComponentHandle, Model, ModelRc, SharedString, Timer, TimerMode, VecModel};
use yantrik_app_runtime::problems::{self, Problem};

use crate::app_context::AppContext;
use crate::{App, ProblemRow};

/// Where a report goes. The service creates or updates one GitHub issue per crash signature and
/// answers with where it landed. It records the sender's address for its rate limiter only, and
/// puts nothing about the sender in the issue.
pub const INTAKE_URL: &str = "https://report.yantrikos.com/v1/report";

/// How often the desktop looks for a record it has not seen, to say so once.
const POLL: Duration = Duration::from_secs(30);

pub fn wire(ui: &App, _ctx: &AppContext) {
    let weak = ui.as_weak();
    refresh(&weak);

    let w = weak.clone();
    ui.on_problem_refresh(move || refresh(&w));

    let w = weak.clone();
    ui.on_problem_select(move |index| {
        let Some(ui) = w.upgrade() else { return };
        ui.set_problem_selected(index);
        ui.set_problem_status("".into());
        match nth(index) {
            Some((_, problem)) => {
                let json = serde_json::to_string_pretty(&problem).unwrap_or_default();
                ui.set_problem_json(json.into());
            }
            None => ui.set_problem_json("".into()),
        }
    });

    let w = weak.clone();
    ui.on_problem_delete(move || {
        let Some(ui) = w.upgrade() else { return };
        if let Some((path, _)) = nth(ui.get_problem_selected()) {
            let _ = std::fs::remove_file(&path);
            tracing::info!(path = %path.display(), "Problem record deleted by the person");
        }
        ui.set_problem_selected(-1);
        ui.set_problem_json("".into());
        refresh(&w);
    });

    let w = weak.clone();
    ui.on_problem_send(move || {
        let Some(ui) = w.upgrade() else { return };
        let Some((path, problem)) = nth(ui.get_problem_selected()) else {
            ui.set_problem_status("Pick a record first.".into());
            return;
        };
        let note = ui.get_problem_note().to_string();
        send_from_ui(&ui, path, problem, note);
    });

    // Say so, once, when a record appears that the desktop has not mentioned. The record is on
    // disk either way; the toast is only so the person knows there is something to look at.
    let w = weak.clone();
    let mut seen: Option<PathBuf> = newest().map(|(p, _)| p);
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, POLL, move || {
        let Some((path, problem)) = newest() else { return };
        if seen.as_ref() == Some(&path) {
            return;
        }
        seen = Some(path);
        let what = match problem.kind.as_str() {
            "panic" | "crash" => format!("{} crashed", problem.program),
            _ => format!("{} reported a failure", problem.program),
        };
        super::toast::local(
            &w,
            "Yantrik",
            &what,
            "It left a record on this machine. Open Problems to read it and decide whether to send it.",
            1,
        );
        refresh(&w);
    });
    // The timer lives as long as the shell; leaking it is the idiom the other wire modules use.
    std::mem::forget(timer);
}

/// Every record on disk, newest first — the order a person wants.
fn records() -> Vec<(PathBuf, Problem)> {
    let mut all = problems::list();
    all.reverse();
    all
}

fn newest() -> Option<(PathBuf, Problem)> {
    records().into_iter().next()
}

fn nth(index: i32) -> Option<(PathBuf, Problem)> {
    if index < 0 {
        return None;
    }
    records().into_iter().nth(index as usize)
}

/// The record a caller named by file name, or the newest when it named none.
pub(crate) fn pick(name: &str) -> Option<(PathBuf, Problem)> {
    if name.is_empty() {
        return newest();
    }
    records()
        .into_iter()
        .find(|(p, _)| p.file_name().is_some_and(|f| f.to_string_lossy() == name))
}

fn refresh(weak: &slint::Weak<App>) {
    let Some(ui) = weak.upgrade() else { return };
    let rows: Vec<ProblemRow> = records()
        .iter()
        .map(|(path, p)| ProblemRow {
            id: path
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_default()
                .into(),
            title: title_of(p).into(),
            when: when_of(p.when).into(),
            kind: p.kind.clone().into(),
        })
        .collect();
    let count = rows.len() as i32;
    ui.set_problem_rows(ModelRc::new(VecModel::from(rows)));
    if ui.get_problem_selected() >= count {
        ui.set_problem_selected(-1);
        ui.set_problem_json("".into());
    }
}

/// "yantrik-studio panicked" — the program and what it did, as a person would say it.
pub(crate) fn title_of(p: &Problem) -> String {
    let verb = match p.kind.as_str() {
        "panic" => "panicked",
        "crash" => "crashed",
        _ => "failed",
    };
    format!("{} {verb}", p.program)
}

/// Local clock time for today, the date otherwise. Unix seconds in, a short string out.
pub(crate) fn when_of(unix: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let age = now.saturating_sub(unix);
    if age < 60 {
        "just now".to_string()
    } else if age < 3600 {
        format!("{} min ago", age / 60)
    } else if age < 86_400 {
        format!("{} h ago", age / 3600)
    } else {
        format!("{} days ago", age / 86_400)
    }
}

/// The one thing that sends: a POST of the record and the note, off the UI thread, with the
/// outcome — where it landed, or why it did not — written back into the screen's status line.
pub(crate) fn send_from_ui(ui: &App, path: PathBuf, problem: Problem, note: String) {
    ui.set_problem_sending(true);
    ui.set_problem_status("Sending…".into());
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let outcome = send(&problem, &note);
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = weak.upgrade() else { return };
            ui.set_problem_sending(false);
            match outcome {
                Ok(sent) => {
                    tracing::info!(record = %path.display(), issue = %sent.issue_url, "Problem report sent");
                    ui.set_problem_status(SharedString::from(sent.sentence()));
                    ui.set_problem_note("".into());
                }
                Err(why) => {
                    tracing::warn!(record = %path.display(), %why, "Problem report not sent");
                    ui.set_problem_status(
                        format!("Not sent: {why}. The record is still here; try again later.").into(),
                    );
                }
            }
        });
    });
}

/// What the intake answered.
#[derive(Debug, serde::Deserialize)]
pub(crate) struct Sent {
    pub issue_url: String,
    #[serde(default)]
    pub issue_number: u64,
    /// How many reports share this signature, this one included.
    #[serde(default = "one")]
    pub count: u64,
    #[serde(default)]
    pub created: bool,
}

fn one() -> u64 {
    1
}

impl Sent {
    pub fn sentence(&self) -> String {
        if self.created {
            format!("Sent. It opened issue #{} — {}", self.issue_number, self.issue_url)
        } else if self.count > 1 {
            format!(
                "Sent. It joined {} earlier report{} of the same problem on issue #{} — {}",
                self.count - 1,
                if self.count == 2 { "" } else { "s" },
                self.issue_number,
                self.issue_url
            )
        } else {
            format!("Sent — {}", self.issue_url)
        }
    }
}

/// POST the record. The body is the record as the file holds it plus the note; nothing about the
/// machine or the person is added here, and the intake adds nothing either.
fn send(problem: &Problem, note: &str) -> Result<Sent, String> {
    let body = serde_json::json!({ "record": problem, "note": note });
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(20)).build();
    let response = agent
        .post(INTAKE_URL)
        .set("Content-Type", "application/json")
        .set("User-Agent", "yantrik-os problem report")
        .send_string(&body.to_string())
        .map_err(|e| match e {
            ureq::Error::Status(code, resp) => {
                let text = resp.into_string().unwrap_or_default();
                format!("the intake answered {code}: {}", text.chars().take(200).collect::<String>())
            }
            ureq::Error::Transport(t) => format!("could not reach report.yantrikos.com ({t})"),
        })?;
    let text = response.into_string().map_err(|e| format!("unreadable answer: {e}"))?;
    serde_json::from_str::<Sent>(&text)
        .map_err(|e| format!("the intake's answer was not understood: {e}"))
}

/// The list `describe shell` carries under `problems`, so a mind can see what went wrong and name
/// a record to `report_problem`.
pub(crate) fn for_describe() -> serde_json::Value {
    let rows: Vec<serde_json::Value> = records()
        .iter()
        .take(20)
        .map(|(path, p)| {
            serde_json::json!({
                "record": path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default(),
                "kind": p.kind,
                "program": p.program,
                "message": p.message.chars().take(160).collect::<String>(),
                "when": p.when,
            })
        })
        .collect();
    serde_json::Value::Array(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn problem(kind: &str, program: &str) -> Problem {
        Problem {
            kind: kind.into(),
            program: program.into(),
            version: "v0".into(),
            git: "abc".into(),
            message: "m".into(),
            location: None,
            backtrace: None,
            log_tail: vec![],
            machine: serde_json::json!({}),
            when: 0,
        }
    }

    #[test]
    fn a_record_is_titled_the_way_a_person_would_say_it() {
        assert_eq!(title_of(&problem("panic", "yantrik-studio")), "yantrik-studio panicked");
        assert_eq!(title_of(&problem("crash", "yantrik-arcade")), "yantrik-arcade crashed");
        assert_eq!(title_of(&problem("failure", "yantrik-ui")), "yantrik-ui failed");
    }

    #[test]
    fn the_intakes_answer_becomes_one_honest_sentence() {
        let created = Sent { issue_url: "https://x/1".into(), issue_number: 1, count: 1, created: true };
        assert_eq!(created.sentence(), "Sent. It opened issue #1 — https://x/1");
        let joined = Sent { issue_url: "https://x/1".into(), issue_number: 1, count: 3, created: false };
        assert_eq!(
            joined.sentence(),
            "Sent. It joined 2 earlier reports of the same problem on issue #1 — https://x/1"
        );
        let second = Sent { issue_url: "https://x/1".into(), issue_number: 1, count: 2, created: false };
        assert!(second.sentence().contains("1 earlier report of"), "{}", second.sentence());
    }

    #[test]
    fn ages_read_as_a_person_would_write_them() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert_eq!(when_of(now), "just now");
        assert_eq!(when_of(now - 300), "5 min ago");
        assert_eq!(when_of(now - 7200), "2 h ago");
        assert_eq!(when_of(now - 3 * 86_400), "3 days ago");
    }
}
