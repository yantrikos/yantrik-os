//! The only code that changes the model: every key, button, action and landing from a shell or a
//! worker arrives here as a `Command`, and asks for its effects.
//!
//! The person's buttons and the Mind's actions are the same requests, so a person and an agent
//! cannot get different answers about whether something worked. Nothing slow runs here: a shell
//! is started, a folder read, `/proc` looked at and a shell waited for on a worker (`tabs.rs`,
//! `frame.rs`, `surface.rs`), and what they found comes back as a command.
//!
//! - `frame.rs`: a shell's output reaching the window, at most once a frame, and the probes.
//! - `tabs.rs`: starting, switching and ending tabs.
//! - `find.rs`: searching the scrollback.
//! - `surface.rs`: `run`, `send_input`, `new_tab` and `open_directory`.
//! - `life.rs`: the close and quit questions, the logout, and what each waits for.
//! - `explain.rs`: the assistant panel.
//! - this file: the window's own small state, and the dispatch.

use std::time::Duration;

use yantrik_app::effects::Effects;
use yantrik_app::timers::TimerId;
use yantrik_app::Refused;

use crate::answers;
use crate::command::{Command, Do, Why};
use crate::model::{Model, FONT_DEFAULT, FONT_MAX, FONT_MIN, PART_FIND, PART_FONT, PART_NOTICE};
use crate::Terminal;

mod explain;
mod find;
mod frame;
pub mod life;
mod surface;
mod tabs;

pub use life::CLOSING;
pub use tabs::first;

type Fx = Effects<Terminal>;

/// The frame after output arrives: one a sixteenth of a second at most, and none while idle.
pub const FRAME: TimerId = TimerId("terminal.frame");
pub const FRAME_AFTER: Duration = Duration::from_millis(16);
/// The look at `/proc` after something happened in a shell, and while a job runs in one.
pub const PROBE: TimerId = TimerId("terminal.probe");
pub const PROBE_AFTER: Duration = Duration::from_millis(100);
/// The backstop of a logout, long enough for a hangup, a short wait and a kill.
pub const END: TimerId = TimerId("terminal.end");
pub const END_AFTER: Duration = Duration::from_secs(3);

pub fn update(m: &mut Model, command: Command, fx: &mut Fx) -> Result<(), Refused> {
    // An answer is written only in the command that settles it (`answers.rs`); what an earlier
    // command wrote is never anyone else's. And a command draws only the parts it names.
    m.answer = serde_json::Value::Null;
    fx.unchanged();
    // In a Mind's copy nothing is typed, started, searched or explained: the screen is hidden.
    // The window says why, as well as the caller: a refused key or action is no silent log line.
    if let Err(refused) = yantrik_app::for_mind::refuse_if(m.for_mind, reads_or_feeds_a_shell(&command)) {
        refuse(m, fx, refused.sentence);
        return Ok(());
    }
    if m.leaving() && starts_work(&command) {
        return Err(Refused::new("closing", CLOSING));
    }
    match command {
        Command::Do(d) => button(m, fx, d),
        Command::SelectTab(index) => tabs::select(m, fx, index),
        Command::CloseTab(index) => life::close_tab(m, fx, index),
        Command::Resized(rows, cols) => {
            m.rows = rows.clamp(2, 160) as u16;
            m.cols = cols.clamp(8, 400) as u16;
            if let Some(session) = m.session() {
                let _ = session.resize(m.rows, m.cols);
            }
        }
        Command::Scroll(lines) => {
            if let Some(session) = m.session() {
                session.scroll(lines);
            }
            m.match_row = -1;
            fx.changed(PART_FIND);
        }
        Command::Search(query) => find::search(m, fx, &query),
        Command::Paste(text) => {
            if let Some(session) = m.session().cloned() {
                session.set_scrollback(0);
                if let Err(e) = session.paste(&text) {
                    say(m, fx, e);
                }
                frame::mark_due(m, fx);
            }
        }
        Command::Input(bytes) => {
            if let Some(session) = m.session().cloned() {
                session.set_scrollback(0);
                m.match_row = -1;
                fx.changed(PART_FIND);
                if let Err(e) = session.write(&bytes) {
                    say(m, fx, e);
                }
                frame::mark_due(m, fx);
            }
        }
        Command::Run(line) => surface::run(m, fx, line),
        Command::SendInput(text) => surface::send_input(m, fx, text),
        Command::NewTab => tabs::new_tab(m, fx, Why::NewTab),
        Command::OpenDirectory(given) => surface::open_directory(m, fx, given),
        Command::Spawned(spawned) => tabs::spawned(m, fx, *spawned),
        Command::SpawnLost => tabs::spawn_lost(m, fx),
        Command::Answered(reply) => {
            let answer = answers::landed(m, *reply);
            answers::settle(m, fx, answer);
        }
        Command::AnswerLost => surface::lost(m, fx),
        Command::Probed { tab, facts } => frame::probed(m, fx, tab, facts),
        Command::ProbeLost { tab } => frame::probe_lost(m, tab),
        Command::Checked { ticket, busy } => life::checked(m, fx, ticket, busy),
        Command::CheckLost { ticket } => life::check_lost(m, fx, ticket),
        Command::Explained { generation, answer } => explain::landed(m, fx, generation, answer),
        Command::Output => frame::output(m, fx),
        Command::Frame => frame::frame(m, fx),
        Command::Probe => frame::probe(m, fx),
        Command::EndNow => fx.exit(),
        Command::CloseRequested(window) => life::close_requested(m, fx, window),
        Command::QuitRequested => life::quit_requested(m, fx),
        Command::Reopened => m.closing = false,
        Command::Ending => life::ending(m, fx),
        Command::Activated => frame::mark_all_due(m, fx),
    }
    Ok(())
}

/// A command that would show the screen's text, start a shell or type into one.
fn reads_or_feeds_a_shell(command: &Command) -> bool {
    starts_work(command) || matches!(command, Command::Search(_) | Command::Do(Do::Explain))
}

/// A command that would start a shell or type into one.
fn starts_work(command: &Command) -> bool {
    matches!(
        command,
        Command::Run(_)
            | Command::SendInput(_)
            | Command::NewTab
            | Command::OpenDirectory(_)
            | Command::Input(_)
            | Command::Paste(_)
            | Command::Do(Do::New | Do::Restart)
    )
}

/// Put `sentence` on the notice line.
pub fn say(m: &mut Model, fx: &mut Fx, sentence: String) {
    m.notice = sentence;
    fx.changed(PART_NOTICE);
}

/// Failure is said twice: to the caller, and on the notice line the person sees (and
/// `describe.notice` carries).
pub fn refuse(m: &mut Model, fx: &mut Fx, sentence: impl Into<String>) {
    let sentence = sentence.into();
    answers::fail(fx, sentence.clone());
    say(m, fx, sentence);
}

/// A header button, a shortcut or the prompt's answer.
fn button(m: &mut Model, fx: &mut Fx, d: Do) {
    match d {
        Do::New => tabs::new_tab(m, fx, Why::Person),
        Do::Close => life::close_tab(m, fx, m.active as i32),
        Do::ConfirmClose => life::confirm(m, fx),
        Do::CancelClose => life::cancel(m, fx),
        Do::Restart => tabs::restart(m, fx),
        Do::Live => {
            if let Some(session) = m.session() {
                session.set_scrollback(0);
            }
            m.match_row = -1;
            fx.changed(PART_FIND);
        }
        Do::Dismiss => {
            if let Some(session) = m.session() {
                session.clear_error();
            }
            say(m, fx, String::new());
        }
        Do::ZoomIn => zoom(m, fx, (m.font_size + 1).min(FONT_MAX)),
        Do::ZoomOut => zoom(m, fx, (m.font_size - 1).max(FONT_MIN)),
        Do::ZoomReset => zoom(m, fx, FONT_DEFAULT),
        Do::NextTab => tabs::step(m, fx, true),
        Do::PreviousTab => tabs::step(m, fx, false),
        Do::NextMatch => find::step(m, fx, true),
        Do::PreviousMatch => find::step(m, fx, false),
        Do::Explain => explain::ask(m, fx),
    }
}

fn zoom(m: &mut Model, fx: &mut Fx, size: i32) {
    if m.font_size != size {
        m.font_size = size;
        fx.changed(PART_FONT);
    }
}
