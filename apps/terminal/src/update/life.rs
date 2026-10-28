//! The app's life: the questions on a close, the logout, and what each waits for.
//!
//! # The questions
//!
//! Closing a tab, closing the window and quitting each ask when a shell has something running in
//! it, and not otherwise. What is running is read on a worker (`/proc`), so a question has two
//! stages: checking, with no dialog yet, and asking, with the dialog up. There is one question at
//! a time: while one is open, another is refused in the notice line, and the dialog is drawn only
//! while the question behind it is asking (`Model::asking`), never without one.
//!
//! # The end
//!
//! A close the person confirmed hangs every shell up (SIGHUP to every process of its
//! session, a short wait for handlers, SIGKILL for what still runs and did not ignore the hangup,
//! the shell reaped: `session.rs`), closes the window
//! and lingers until the last shell is gone. A logout (SIGTERM, SIGHUP, or the display going)
//! asks nothing, because nobody is there to answer: it hangs the shells up and ends the app when
//! they are gone, or when a backstop passes. A person's quit that was confirmed ends the same
//! way. Either way no shell is left running with nobody to read it; a job that ignores SIGHUP chose to
//! outlive its terminal and stays.

use std::sync::Arc;
use std::time::Duration;

use yantrik_app::app::WindowId;
use yantrik_app::effects::Effects;

use super::{say, tabs, END, END_AFTER};
use crate::command::Command;
use crate::model::{Ask, Model, Stage, Subject, PART_DIALOG, PART_FOCUS};
use crate::session::Session;
use crate::Terminal;

type Fx = Effects<Terminal>;

/// How long a close waits for the shells before the framework ends the app anyway.
pub const CLOSE_BACKSTOP: Duration = Duration::from_secs(5);

/// What a command that would start something is told while the app is closing.
pub const CLOSING: &str = "Terminal is closing, so nothing more is started.";
/// What a second question is told while one is open, and while one is being prepared.
pub const ANSWER_FIRST: &str = "Answer the question on the screen first.";
pub const WAIT_FOR_CHECK: &str = "Please wait: Terminal is checking what is running.";
const CHECK_LOST: &str = "Terminal could not tell what is running, so nothing was closed.";

/// Why a new question cannot be asked now.
fn blocked(m: &Model) -> Option<&'static str> {
    m.ask.map(|ask| match ask.stage {
        Stage::Asking => ANSWER_FIRST,
        Stage::Checking => WAIT_FOR_CHECK,
    })
}

/// Close the tab at `index`, asking first if something runs in it.
pub fn close_tab(m: &mut Model, fx: &mut Fx, index: i32) {
    let Some(tab) = usize::try_from(index).ok().and_then(|i| m.tabs.get(i)) else {
        return;
    };
    if let Some(sentence) = blocked(m) {
        return say(m, fx, sentence.into());
    }
    let (id, session) = (tab.id, tab.session.clone());
    check(m, fx, Subject::Tab(id), vec![session]);
}

/// The person closed the window.
pub fn close_requested(m: &mut Model, fx: &mut Fx, window: WindowId) {
    if let Some(sentence) = blocked(m) {
        return say(m, fx, sentence.into());
    }
    check(m, fx, Subject::Window(window), sessions(m));
}

/// The person asked the app to quit.
pub fn quit_requested(m: &mut Model, fx: &mut Fx) {
    if let Some(sentence) = blocked(m) {
        return say(m, fx, sentence.into());
    }
    check(m, fx, Subject::Quit, sessions(m));
}

fn sessions(m: &Model) -> Vec<Arc<Session>> {
    m.tabs.iter().map(|t| t.session.clone()).collect()
}

/// Ask a worker how many of these shells have something running, and ask the person if any do.
fn check(m: &mut Model, fx: &mut Fx, subject: Subject, shells: Vec<Arc<Session>>) {
    m.next_ticket += 1;
    let ticket = m.next_ticket;
    m.ask = Some(Ask { subject, stage: Stage::Checking, ticket, busy: 0 });
    fx.spawn_or(Command::CheckLost { ticket }, move |_| Command::Checked {
        ticket,
        busy: shells.iter().filter(|s| s.has_children()).count(),
    });
}

/// What the check found. A check that is not the open question's (the app began to end, or the
/// question was answered) is not heard.
pub fn checked(m: &mut Model, fx: &mut Fx, ticket: u64, busy: usize) {
    let Some(ask) = m.ask.filter(|a| a.ticket == ticket && a.stage == Stage::Checking) else {
        return;
    };
    if busy == 0 {
        m.ask = None;
        return proceed(m, fx, ask.subject);
    }
    m.ask = Some(Ask { stage: Stage::Asking, busy, ..ask });
    fx.changed(PART_DIALOG);
}

pub fn check_lost(m: &mut Model, fx: &mut Fx, ticket: u64) {
    if m.ask.is_some_and(|a| a.ticket == ticket) {
        m.ask = None;
        say(m, fx, CHECK_LOST.into());
    }
}

/// The person said yes, close it.
pub fn confirm(m: &mut Model, fx: &mut Fx) {
    if let Some(ask) = m.ask.filter(|a| a.stage == Stage::Asking) {
        m.ask = None;
        fx.changed(PART_DIALOG);
        fx.changed(PART_FOCUS);
        m.focus += 1;
        proceed(m, fx, ask.subject);
    }
}

/// The person said keep it open.
pub fn cancel(m: &mut Model, fx: &mut Fx) {
    if m.ask.is_some_and(|a| a.stage == Stage::Asking) {
        m.ask = None;
        fx.changed(PART_DIALOG);
        fx.changed(PART_FOCUS);
        m.focus += 1;
    }
}

/// The question is settled, one way or the other, and the thing it was about goes.
fn proceed(m: &mut Model, fx: &mut Fx, subject: Subject) {
    match subject {
        Subject::Tab(id) => tabs::remove(m, fx, id),
        Subject::Window(window) => {
            // The window goes at once; the app runs on without it until every shell is gone.
            m.closing = true;
            hang_up_all(m);
            fx.close(window);
            fx.linger(CLOSE_BACKSTOP);
            settle(m, fx);
        }
        Subject::Quit => end(m, fx),
    }
}

/// The process is asked to end: a logout, a signal, the display going. Nobody is there to answer,
/// so nothing is asked, and a question that was open is dropped.
pub fn ending(m: &mut Model, fx: &mut Fx) {
    if m.ask.take().is_some() {
        fx.changed(PART_DIALOG);
    }
    end(m, fx);
}

fn end(m: &mut Model, fx: &mut Fx) {
    m.ending = true;
    hang_up_all(m);
    fx.after(END, END_AFTER, Command::EndNow);
    settle(m, fx);
}

fn hang_up_all(m: &Model) {
    for session in m.tabs.iter().map(|t| &t.session).chain(m.dying.iter()) {
        session.hang_up();
    }
}

/// The app is leaving and the last shell has gone: it ends. Called after every command that can
/// be the last word of a shell.
pub fn settle(m: &mut Model, fx: &mut Fx) {
    if m.leaving() && m.all_ended() {
        fx.exit();
    }
}
