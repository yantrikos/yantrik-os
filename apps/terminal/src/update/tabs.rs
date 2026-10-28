//! Tabs: starting a shell, switching, and taking one away.
//!
//! A shell is started on a worker (a PTY, a fork and an exec, and the folder it opens in read
//! first), and the tab appears when it lands. The cap counts the tabs being started as well as
//! those open, so a burst of requests cannot go past it.

use std::path::PathBuf;
use std::sync::Arc;

use yantrik_app::effects::Effects;

use super::{frame, life, refuse};
use crate::answers;
use crate::command::{Command, Reply, Spawned, Why};
use crate::model::{
    tabs_full, Facts, Model, Tab, PART_DIALOG, PART_EXPLAIN, PART_FIND, PART_FOCUS, PART_NOTICE,
    PART_RAISE, PART_SCREEN,
};
use crate::probe;
use crate::session::Session;
use crate::Terminal;

type Fx = Effects<Terminal>;

/// The sentence for a shell that never answered.
pub const NOT_STARTED: &str = "Terminal could not start the shell. Try again.";

/// Start a shell beside the active one, in the folder it is sitting in.
pub fn new_tab(m: &mut Model, fx: &mut Fx, why: Why) {
    if let Some(full) = tabs_full(m.tabs.len() + m.spawning) {
        return refuse(m, fx, full);
    }
    start(m, fx, why, None);
}

/// Start a shell, in `dir` if one is given, else where the active shell is.
pub fn start(m: &mut Model, fx: &mut Fx, why: Why, dir: Option<PathBuf>) {
    m.spawning += 1;
    let (rows, cols, notify) = (m.rows, m.cols, m.hub.notifier());
    let active = m.session().cloned();
    fx.spawn_or(Command::SpawnLost, move |_| {
        Command::Spawned(Box::new(Spawned { why, result: spawn(dir, active, rows, cols, notify) }))
    });
}

/// The first tab, at start: the model counted it already, so only the worker is asked for.
pub fn first(m: &Model, fx: &mut Fx) {
    let (rows, cols, notify): (u16, u16, Arc<dyn Fn() + Send + Sync>) = (m.rows, m.cols, m.hub.notifier());
    fx.spawn_or(Command::SpawnLost, move |_| {
        Command::Spawned(Box::new(Spawned { why: Why::First, result: spawn(None, None, rows, cols, notify) }))
    });
}

/// On a worker: the folder is read and the shell started.
fn spawn(
    dir: Option<PathBuf>,
    active: Option<Arc<Session>>,
    rows: u16,
    cols: u16,
    notify: Arc<dyn Fn() + Send + Sync>,
) -> Result<(Arc<Session>, Facts), String> {
    let dir = dir.unwrap_or_else(|| probe::start_dir(active.as_deref()));
    let dir = std::fs::canonicalize(dir).map_err(|e| format!("Could not open folder: {e}"))?;
    if !dir.is_dir() {
        return Err("The requested path is not a folder.".into());
    }
    let session =
        Session::spawn(&dir, rows, cols, notify).map_err(|e| format!("Could not start the shell: {e}"))?;
    let facts = probe::read(&session);
    Ok((Arc::new(session), facts))
}

/// A shell has started, or could not. A tab started for a person is the one in front; for a
/// caller, the answer is that shell's own.
pub fn spawned(m: &mut Model, fx: &mut Fx, spawned: Spawned) {
    m.spawning = m.spawning.saturating_sub(1);
    let Spawned { why, result } = spawned;
    let (session, facts) = match result {
        Ok(started) => started,
        Err(sentence) => return refuse(m, fx, sentence),
    };
    // Started, and then the app was told to end: it is ended with the rest, and not shown.
    if m.leaving() {
        session.hang_up();
        m.dying.push(session);
        answers::fail(fx, super::CLOSING.to_string());
        return life::settle(m, fx);
    }
    if let Some(full) = tabs_full(m.tabs.len()) {
        session.hang_up();
        m.dying.push(session);
        return refuse(m, fx, full);
    }
    m.serial += 1;
    let id = m.serial;
    let revision = session.revision();
    m.tabs.push(Tab { id, session: session.clone(), facts: facts.clone(), probing: false, due: false, revision });
    m.active = m.tabs.len() - 1;
    reset_view(m, fx);
    m.notice.clear();
    fx.changed(PART_NOTICE);
    m.focus += 1;
    fx.changed(PART_FOCUS);
    frame::catch_up(m, fx);
    if let Why::Directory(_) = &why {
        m.raises += 1;
        fx.changed(PART_RAISE);
    }
    if answers::owed(fx) {
        let reply = Reply { tab: id, body: answers::shell_body(&session, &facts) };
        let mut body = answers::landed(m, reply);
        if let Why::Directory(given) = &why {
            body["asked_for"] = serde_json::json!(given);
        }
        answers::settle(m, fx, body);
    }
}

/// A shell was being started and the worker never answered.
pub fn spawn_lost(m: &mut Model, fx: &mut Fx) {
    m.spawning = m.spawning.saturating_sub(1);
    refuse(m, fx, NOT_STARTED);
    life::settle(m, fx);
}

/// Whatever the view held about the last tab is not about this one.
pub fn reset_view(m: &mut Model, fx: &mut Fx) {
    m.shown = None;
    m.matches.clear();
    m.match_index = 0;
    m.query.clear();
    m.match_row = -1;
    m.explain.generation += 1;
    m.explain.working = false;
    m.explain.text.clear();
    if let Some(session) = m.session() {
        let _ = session.resize(m.rows, m.cols);
    }
    fx.changed(PART_FIND);
    fx.changed(PART_EXPLAIN);
    // With no tab left there is no key to differ from, and the screen still has to be emptied.
    fx.changed(PART_SCREEN);
}

pub fn select(m: &mut Model, fx: &mut Fx, index: i32) {
    if index >= 0 && (index as usize) < m.tabs.len() {
        m.active = index as usize;
        reset_view(m, fx);
        frame::catch_up(m, fx);
    }
}

/// The next or previous tab, wrapping at the ends.
pub fn step(m: &mut Model, fx: &mut Fx, forward: bool) {
    let n = m.tabs.len();
    if n > 0 {
        m.active = (m.active + n + if forward { 1 } else { n - 1 }) % n;
        reset_view(m, fx);
        frame::catch_up(m, fx);
    }
}

/// Take the tab with this id away, ending its shell.
pub fn remove(m: &mut Model, fx: &mut Fx, id: u64) {
    let Some(index) = m.index_of(id) else {
        return;
    };
    let tab = m.tabs.remove(index);
    // Ended now, whoever else still holds the session (a worker waiting on it), and remembered
    // until it is gone so that an end of the app waits for it.
    tab.session.hang_up();
    m.dying.push(tab.session);
    if index < m.active {
        m.active -= 1;
    }
    m.active = m.active.min(m.tabs.len().saturating_sub(1));
    fx.changed(PART_DIALOG);
    reset_view(m, fx);
    frame::catch_up(m, fx);
}

/// Restart on an exited shell: its tab goes and a new one takes its place.
pub fn restart(m: &mut Model, fx: &mut Fx) {
    if m.session().is_some_and(|s| s.alive()) {
        return;
    }
    if let Some(id) = m.tab().map(|t| t.id) {
        remove(m, fx, id);
    }
    new_tab(m, fx, Why::Restart);
}
