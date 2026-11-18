//! The first review of #939 (head 76cce3b6), each finding as a test that failed before its fix.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};
use yantrik_app::app::WindowId;
use yantrik_app::control::AgentTokenScope;
use yantrik_app::documents::disk::{Disk, DiskError, Exchanged, RealDisk};
use yantrik_testing::{Pending, TestApp};

use super::{act, draft_file, file, killed_and_started_again, modified, notice, opened, outside_writes, press, scratch, settle, started, state, tabs, text, typed, with_file};
use crate::command::{Command, WindowAction};
use crate::TextEditor;

/// `describe` as an agent: its notice and its summary.
fn agent_reads(app: &TestApp<TextEditor>) -> String {
    app.model(|m| {
        let _agent = AgentTokenScope::enter(Some("tok-review".into()));
        let seen = crate::describe::view(m);
        format!("{} | {}", seen.state["notice"], seen.summary)
    })
}

fn outside_home(dir: &Path) -> bool {
    !dir.starts_with(PathBuf::from(std::env::var_os("HOME").unwrap_or_default()))
}

/// M1: an outside change to a tab an agent is not shown is said to the person by name, and to an
/// agent as `(hidden)`, with no path, for the tab in front and for one behind it.
#[test]
fn outside_change_notices_never_name_a_hidden_tab_to_an_agent() {
    let app = started();
    let dir = scratch("m1-payroll");
    if !outside_home(&dir) {
        return;
    }
    let behind = dir.join("payroll-2026-secret.txt");
    let front = dir.join("bank-login-secret.txt");
    opened(&app, &behind, "salaries\n");
    opened(&app, &front, "passwords\n");
    let leaks = |seen: &str| seen.contains("secret") || seen.contains(&dir.display().to_string());

    // A tab behind, with nothing unsaved, follows the file.
    outside_writes(&app, &behind, "salaries, changed\n");
    assert!(notice(&app).contains("payroll-2026-secret.txt"), "the person is told which: {}", notice(&app));
    let seen = agent_reads(&app);
    assert!(!leaks(&seen) && seen.contains("(hidden) was changed outside"), "{seen}");

    // The tab in front, unsaved, in conflict.
    typed(&app, "passwords, mine\n");
    outside_writes(&app, &front, "theirs\n");
    let seen = agent_reads(&app);
    assert!(!leaks(&seen) && seen.contains("(hidden) was changed outside the editor while"), "{seen}");

    // The tab behind, renamed: followed, and no new name or path said to an agent.
    std::fs::rename(&behind, dir.join("payroll-renamed-secret.txt")).unwrap();
    app.look_at_files();
    settle(&app);
    assert!(notice(&app).contains("payroll-renamed-secret.txt"), "{}", notice(&app));
    let seen = agent_reads(&app);
    assert!(!leaks(&seen) && seen.contains("A file an agent is not shown moved"), "{seen}");

    // Gone and not found.
    std::fs::remove_file(dir.join("payroll-renamed-secret.txt")).unwrap();
    app.look_at_files();
    settle(&app);
    let seen = agent_reads(&app);
    assert!(!leaks(&seen) && seen.contains("(hidden) is not there any more"), "{seen}");
}

/// Applies jobs as the runner would, and once the app has ended, shuts it down as the runner does
/// after its loop stops: whatever is still owed then is refused.
fn run_as_the_runner_would(app: &TestApp<TextEditor>, pending: Pending) -> Result<Value, String> {
    for _ in 0..32 {
        if app.exit_reason().is_some() {
            app.shutdown();
            break;
        }
        if app.jobs_waiting() == 0 {
            break;
        }
        app.run_jobs();
    }
    pending.settle()
}

/// M2: the prompt's `save` and `discard`, and a save that ends a linger, are answered from their
/// own work even though that work closes the window.
#[test]
fn a_call_whose_own_work_closes_the_window_is_answered_before_the_end() {
    let (app, path) = with_file("m2-save", "v1\n");
    typed(&app, "v2\n");
    app.close_requested(WindowId::PRIMARY);
    assert_eq!(state(&app)["dialog"], 3);
    let saved = run_as_the_runner_would(&app, app.act("save", json!({})).unwrap()).expect("the save is on disk");
    assert_eq!((saved["saved"].clone(), saved["closed_the_tab"].clone()), (json!(true), json!(true)), "{saved}");
    assert_eq!(file(&path), "v2\n");
    assert!(app.exit_reason().is_some());

    let (app, _) = with_file("m2-discard", "v1\n");
    typed(&app, "v2\n");
    app.close_requested(WindowId::PRIMARY);
    let discarded = run_as_the_runner_would(&app, app.act("discard", json!({})).unwrap()).expect("discarded");
    assert_eq!(discarded["discarded"], "notes.txt", "{discarded}");

    let (app, path) = with_file("m2-linger", "v1\n");
    typed(&app, "v2\n");
    let pending = app.act("save", json!({})).unwrap();
    app.close_requested(WindowId::PRIMARY);
    assert!(app.lingering());
    let saved = run_as_the_runner_would(&app, pending).expect("the save that ended the linger");
    assert_eq!(saved["saved"], true, "{saved}");
    assert_eq!(file(&path), "v2\n");
}

/// L1: a re-read that lands while a save of the same tab is in flight leaves that save's landing
/// its own: the call is answered from the save, here its refusal.
#[test]
fn a_reread_landing_during_a_save_leaves_the_save_its_own_answer() {
    let (app, path) = with_file("l1", "first\n");
    std::fs::write(&path, "theirs\n").unwrap();
    app.look_at_files();
    assert_eq!(app.jobs_waiting(), 1, "the re-read is out");
    let pending = app.act("save", json!({})).unwrap();
    settle(&app);
    let answer = pending.settle();
    assert!(answer.as_ref().is_err_and(|e| e.contains("changed on disk")), "{answer:?}");
    assert_eq!(file(&path), "theirs\n");
}

/// L2: a Save As onto the file another tab holds is refused, naming that tab, and no draft is lost.
#[test]
fn a_save_as_onto_a_file_another_tab_holds_is_refused_and_no_draft_is_lost() {
    let (app, shared) = with_file("l2", "shared\n");
    typed(&app, "person edit\n");
    act(&app, "new", json!({ "text": "the other tab\n" })).unwrap();
    let refused = act(&app, "save_as", json!({ "path": shared.display().to_string(), "overwrite": true })).unwrap_err();
    assert!(refused.contains("open in another tab") && refused.contains("Nothing was written"), "{refused}");
    assert_eq!(file(&shared), "shared\n");
    act(&app, "append", json!({ "text": "more\n" })).unwrap();
    app.run_writes();
    let again = killed_and_started_again(app);
    assert_eq!(tabs(&again), 2, "both drafts came back: {}", notice(&again));
    let texts: Vec<String> =
        again.model(|m| m.tabs.iter().map(|t| m.text(t).to_string()).collect());
    assert_eq!(texts, vec!["person edit\n".to_string(), "the other tab\nmore\n".to_string()]);
}

/// A disk where another program writes the file in the instant between the check and the replace.
struct Racing(Vec<u8>);

impl Disk for Racing {
    fn write_atomically(&self, path: &Path, bytes: &[u8]) -> Result<(), DiskError> {
        RealDisk.write_atomically(path, bytes)
    }
    fn create_new(&self, path: &Path, bytes: &[u8]) -> Result<(), DiskError> {
        RealDisk.create_new(path, bytes)
    }
    fn read(&self, path: &Path) -> std::io::Result<Option<Vec<u8>>> {
        RealDisk.read(path)
    }
    fn remove(&self, path: &Path) -> std::io::Result<()> {
        RealDisk.remove(path)
    }
    fn exchange(&self, path: &Path, bytes: &[u8]) -> Result<Exchanged, DiskError> {
        std::fs::write(path, &self.0).unwrap();
        RealDisk.exchange(path, bytes)
    }
}

/// L3: another program's version is kept when its first name is too long for the folder, or taken
/// by a link that leads nowhere.
#[test]
fn another_programs_version_is_kept_whatever_stands_in_its_first_names() {
    // A name the save's own temp file still fits beside (255 bytes), but "<stem> (outside
    // version).txt" and its temp do not.
    let app = started();
    let long = scratch("l3-long").join(format!("{}.txt", "s".repeat(226)));
    opened(&app, &long, "first\n");
    app.run(Command::UseDisk(Arc::new(Racing(b"theirs, long\n".to_vec()))));
    typed(&app, "mine\n");
    press(&app, WindowAction::Save);
    let kept = long.with_file_name("outside version 1");
    assert_eq!(file(&kept), "theirs, long\n", "{}", notice(&app));

    // A dangling link at the first name: passed by, never written through.
    let (app, path) = with_file("l3-link", "first\n");
    let first = path.with_file_name("notes (outside version).txt");
    std::os::unix::fs::symlink(path.with_file_name("nowhere"), &first).unwrap();
    app.run(Command::UseDisk(Arc::new(Racing(b"theirs, linked\n".to_vec()))));
    typed(&app, "mine\n");
    press(&app, WindowAction::Save);
    assert_eq!(file(&path.with_file_name("notes (outside version 2).txt")), "theirs, linked\n", "{}", notice(&app));
    assert!(!path.with_file_name("nowhere").exists(), "nothing was written through the link");
}

/// L4: after a conflict, Undo back to the text the tab first read is not "saved": that text is on
/// no disk now, so its draft is kept and the close asks.
#[test]
fn undo_back_to_the_old_text_after_a_conflict_is_not_saved() {
    let (app, path) = with_file("l4", "first\n");
    typed(&app, "mine\n");
    outside_writes(&app, &path, "theirs\n");
    press(&app, WindowAction::Undo);
    assert_eq!(text(&app), "first\n");
    assert!(modified(&app), "first\\n is on no disk");
    app.run_writes();
    assert!(draft_file(&app).exists());
    press(&app, WindowAction::Close);
    assert_eq!(state(&app)["dialog"], 3);
}

// The delta review of #939 (head 519a3e75).

/// N1: a re-read set aside for a save that then fails is taken up again: the clean tab follows
/// its file, and is never left showing an old copy as saved.
#[test]
fn a_reread_set_aside_for_a_save_that_fails_is_taken_up_again() {
    let (app, path) = with_file("n1", "first\n");
    let taken = path.with_file_name("other.txt");
    std::fs::write(&taken, "somebody else's\n").unwrap();
    std::fs::write(&path, "theirs\n").unwrap();
    app.look_at_files();
    assert_eq!(app.jobs_waiting(), 1, "the re-read is out");
    let refused = act(&app, "save_as", json!({ "path": taken.display().to_string() })).unwrap_err();
    assert!(refused.contains("already exists"), "{refused}");
    assert_eq!(text(&app), "theirs\n", "the tab follows its file: {}", notice(&app));
    assert!(!modified(&app));
    assert!(notice(&app).contains("was changed outside the editor"), "{}", notice(&app));
}

/// N2: something changed while a decided close waits on its answers is asked about, not closed
/// away.
#[test]
fn a_change_made_while_a_decided_close_waits_is_asked_about() {
    let (app, path) = with_file("n2", "v1\n");
    typed(&app, "v2\n");
    app.close_requested(WindowId::PRIMARY);
    let save = app.act("save", json!({})).unwrap();
    app.run_one_job();
    assert_eq!(file(&path), "v2\n", "the prompt's save landed");
    assert!(app.exit_reason().is_none(), "the close waits on the save's answer");
    let append = app.act("append", json!({ "text": "late\n" })).unwrap();
    settle(&app);
    assert!(save.settle().is_ok());
    assert!(append.settle().is_ok());
    assert!(app.exit_reason().is_none(), "not closed over the late text");
    assert_eq!(state(&app)["dialog"], 3);
    assert_eq!(state(&app)["dialog_is"], "Untitled has unsaved changes.");
}
