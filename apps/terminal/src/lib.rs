//! Yantrik Terminal, on YantrikApp (design/app-framework-2026-10-21.md, section 11: behaviour
//! unchanged).
//!
//! Persistent PTY shells in tabs: bounded history, event-driven rendering, and the four actions a
//! mind uses to type into a shell. Each tab is a shell on its own PTY (`session.rs`); one worker
//! per session sleeps on the PTY and is the only thing that reads or writes it.
//!
//! # How it is put together
//!
//! - `session.rs`: a bounded PTY session. `hub.rs`: how its worker wakes the app, once a frame.
//! - `model.rs` and `command.rs`: what the app is, and every change to it. `update/`: the only
//!   code that changes the model. `view.rs`: the window. `keys.rs`: which keys are shortcuts and
//!   what bytes the rest become. `describe.rs`: what the Mind reads.
//! - `intents.rs`: the four actions, as they were published. `answers.rs` and `wait.rs`: what each
//!   is told, from what the shell did. `probe.rs`: what `/proc` says about a shell.
//!
//! Four rules shaped the code:
//!
//! - NOTHING SLOW ON THE UI THREAD. A shell is started, a folder read, `/proc` looked at and a
//!   shell waited for on a worker. The UI thread only queues keystrokes (a bounded channel and a
//!   one-byte wake-up) and reads the parser's screen in memory.
//! - OUTPUT IS BATCHED. A shell's worker posts at most one command until the frame after it has
//!   been taken, and a frame is at most a sixteenth of a second after output. A shell that prints
//!   without pause (`yes`) costs one command and one redraw a frame, and its memory is the
//!   session's scrollback, `HISTORY_LINES` (1000) lines plus the screen, whatever it prints.
//!   Nothing runs while every shell is quiet.
//! - ENDING IS A HANGUP. Closing a tab, closing the window, a quit and a logout each hang up the
//!   shells they cover as a terminal does: SIGHUP to every process of the shell's session,
//!   foreground and background, then up to 500 ms for SIGHUP handlers to finish, then SIGKILL for
//!   what is still running. A job that ignores SIGHUP (`nohup`, `trap '' HUP`) chose to outlive its
//!   terminal and is left running, and so is one that called `setsid`. The one exception: if the
//!   shell itself has not exited 500 ms after the hangup (the person ran `trap '' HUP` in it), the
//!   foreground group, a job that ignores SIGHUP included, is killed with it. A tab, a window or a quit
//!   with a job running asks first, one question at a time; a logout asks nothing.
//! - ANSWERS ARE THE SHELL'S OWN. An action is told what its own shell did (`wait.rs`), read by
//!   the worker that waited for it, never the tab the person happens to have in front of them.

use yantrik_app::app::{App, Boot, Changed, Event, ExitReason, Startup, WindowId};
use yantrik_app::control::View;
use yantrik_app::cx::Cx;
use yantrik_app::effects::Effects;
use yantrik_app::intents::Intents;
use yantrik_app::Refused;

mod answers;
mod child;
mod command;
mod describe;
mod hub;
mod intents;
mod keys;
mod model;
mod probe;
pub mod session;
mod update;
mod view;
mod wait;
#[cfg(test)]
mod tests;

pub use command::Command;
pub use model::{tabs_full, Model, MAX_TABS};
pub use session::Session;

slint::include_modules!();
yantrik_app::window!(TerminalApp);

pub struct Terminal;

impl App for Terminal {
    type Ui = TerminalApp;
    type Model = Model;
    type Command = Command;
    /// Terminal keeps nothing between runs, as before: the text size and the tabs are the
    /// window's for as long as it is open.
    type Settings = ();
    /// The control surface's id, as it always was: `yos act terminal ...`.
    const ID: &'static str = "terminal";

    fn model(boot: &Boot<()>) -> Result<Model, Startup> {
        let mut m = Model::new(hub::Hub::default(), boot.mind.clone(), boot.for_mind);
        // The first tab is on its way from the start (`start` asks the worker), so the cap and
        // the end both count it.
        m.spawning = 1;
        Ok(m)
    }

    fn wire(cx: &Cx<Self>) {
        view::wire(cx);
    }

    fn start(model: &Model, fx: &mut Effects<Self>) {
        update::first(model, fx);
    }

    fn update(m: &mut Model, command: Command, fx: &mut Effects<Self>) -> Result<(), Refused> {
        update::update(m, command, fx)
    }

    fn render(ui: &TerminalApp, _: WindowId, m: &Model, changed: &Changed) {
        view::render(ui, m, changed);
    }

    fn describe(m: &Model) -> View {
        describe::view(m)
    }

    fn intents(table: &mut Intents<Self>) {
        intents::table(table);
    }

    fn event(event: Event<()>, m: &Model) -> Option<Command> {
        match event {
            // The person closed the window: with a job running in a shell it asks first.
            Event::CloseRequested(window) => Some(Command::CloseRequested(window)),
            // A second launch opened the window again while a close waited for the shells.
            Event::Presented(_) if m.closing => Some(Command::Reopened),
            // A person's quit asks as a close does; a logout (a signal, the display going) asks
            // nothing and ends the shells.
            Event::ExitRequested(ExitReason::Person) => Some(Command::QuitRequested),
            Event::ExitRequested(_) => Some(Command::Ending),
            // Brought to the front: what runs in each shell is read again.
            Event::Activated(_) => Some(Command::Activated),
            // A launch opens the window and nothing more: a file named on the command line or a
            // link's intent is never typed into a shell.
            _ => None,
        }
    }
}
