//! A shell's output reaching the window, and what is read from `/proc` about each shell.
//!
//! A shell's worker wakes the app through the hub (`hub.rs`), which posts one `Output` command
//! until the frame after it has been taken. That frame comes at most a sixteenth of a second later
//! and draws only what moved: the tab strip if a title or a shell's life changed, the screen if
//! the active session's revision did. Nothing runs while every shell is quiet.
//!
//! Where a shell is and what runs in it come from `/proc`, which is read on a worker (a probe).
//! A shell is probed when something happened in it (output, a key, the window coming forward) and
//! again every tenth of a second while a job runs in it, so a command that prints nothing is
//! still seen to end. A probe that finds nothing new draws nothing.

use yantrik_app::effects::Effects;

use super::{find, life, FRAME, FRAME_AFTER, PROBE, PROBE_AFTER};
use crate::command::Command;
use crate::model::{title_of, Facts, Model, PART_NOTICE, PART_SCREEN, PART_TABS};
use crate::probe;
use crate::Terminal;

type Fx = Effects<Terminal>;

/// A shell has something new. While the app is leaving it is taken at once, because the end waits
/// for the shells' last word; otherwise it waits for the frame.
pub fn output(m: &mut Model, fx: &mut Fx) {
    if m.leaving() {
        return frame(m, fx);
    }
    if !m.frame_armed {
        m.frame_armed = true;
        fx.after(FRAME, FRAME_AFTER, Command::Frame);
    }
}

/// The frame: the hub is released first, so output after this posts again, then the window
/// catches up with the shells.
pub fn frame(m: &mut Model, fx: &mut Fx) {
    m.frame_armed = false;
    m.hub.release();
    catch_up(m, fx);
    life::settle(m, fx);
}

/// Draw what moved since the window last looked, and note which shells are due a probe.
pub fn catch_up(m: &mut Model, fx: &mut Fx) {
    m.dying.retain(|s| s.alive());
    let signature: Vec<(String, bool, bool)> = m
        .tabs
        .iter()
        .enumerate()
        .map(|(i, t)| (title_of(&t.facts.cwd), i == m.active, t.session.alive()))
        .collect();
    if signature != m.signature {
        m.signature = signature;
        fx.changed(PART_TABS);
    }
    let key = m.tab().map(|t| (t.id, t.session.revision()));
    if key != m.shown {
        m.shown = key;
        fx.changed(PART_SCREEN);
        // Keep Find coherent as a running job appends or replaces visible text. This runs only
        // on changed frames and only while a query is open.
        find::refresh(m, fx);
        if let Some(error) = m.session().and_then(|s| s.error()) {
            if m.notice != error {
                m.notice = error;
                fx.changed(PART_NOTICE);
            }
        }
    }
    let mut any = false;
    for tab in &mut m.tabs {
        let revision = tab.session.revision();
        if revision != tab.revision {
            tab.revision = revision;
            tab.due = true;
        }
        any |= tab.due && !tab.probing;
    }
    if any {
        arm(m, fx);
    }
}

fn arm(m: &mut Model, fx: &mut Fx) {
    if !m.probe_armed {
        m.probe_armed = true;
        fx.after(PROBE, PROBE_AFTER, Command::Probe);
    }
}

/// Something happened in the active shell that the screen has not shown yet (a key went in).
pub fn mark_due(m: &mut Model, fx: &mut Fx) {
    if let Some(tab) = m.tabs.get_mut(m.active) {
        tab.due = true;
        arm(m, fx);
    }
}

/// The window came forward: every shell is looked at again.
pub fn mark_all_due(m: &mut Model, fx: &mut Fx) {
    for tab in &mut m.tabs {
        tab.due = true;
    }
    if !m.tabs.is_empty() {
        arm(m, fx);
    }
}

/// The probe timer: every shell that is due is read on a worker, one probe each at a time.
pub fn probe(m: &mut Model, fx: &mut Fx) {
    m.probe_armed = false;
    for tab in m.tabs.iter_mut().filter(|t| t.due && !t.probing) {
        tab.due = false;
        tab.probing = true;
        let (id, session) = (tab.id, tab.session.clone());
        fx.spawn_or(Command::ProbeLost { tab: id }, move |_| Command::Probed {
            tab: id,
            facts: probe::read(&session),
        });
    }
}

/// A probe landed. A shell with a job in it is looked at again soon; one at rest is not.
pub fn probed(m: &mut Model, fx: &mut Fx, id: u64, facts: Facts) {
    let Some(tab) = m.tabs.iter_mut().find(|t| t.id == id) else {
        return;
    };
    tab.probing = false;
    let moved = tab.facts != facts;
    tab.due |= facts.busy;
    let again = tab.due;
    tab.facts = facts;
    if moved {
        fx.changed(PART_SCREEN);
        catch_up(m, fx);
    }
    if again {
        arm(m, fx);
    }
}

pub fn probe_lost(m: &mut Model, id: u64) {
    if let Some(tab) = m.tabs.iter_mut().find(|t| t.id == id) {
        tab.probing = false;
    }
}
