//! Yantrik Calendar, on YantrikApp (design/app-framework-2026-10-21.md, §11 step 9: behaviour
//! unchanged).
//!
//! The calendar's events live in calendar-service, not here: the app keeps no files of its own.
//! It is a window onto the store — the month, the week and the day, the agenda for the day picked,
//! a form for a new event — and the surface a mind reads and changes the calendar through.
//!
//! # How it is put together
//!
//! - `model.rs` and `command.rs`: what the app is, and every change to it.
//! - `update/`: the only code that changes the model. Navigation (`navigate`), the form and
//!   `add_event` (`form`), the deletes (`remove`), the updates (`change`), the agent layer (`ai`).
//! - `work.rs`: what the jobs do off the UI thread, over the service (`backend/`).
//! - `view.rs`: the window, bound and drawn. `describe.rs` and `follow.rs`: what the Mind reads.
//! - `intents/`: the eleven actions, as they were published. `asks.rs`: their arguments, checked.
//! - `events.rs` and `views/`: everything derived from the events in hand, pure.
//! - `today.rs`, `notice.rs`, `ownership.rs`, `who.rs`, `rail.rs`: one concern each.
//!
//! The person's button and the Mind's action are one command wherever they do the same thing, so
//! neither can report an outcome the other would not. Every read and write of the store is a job;
//! a write is read back from the store before it is reported.

use std::time::Duration;

use yantrik_app::app::{App, Boot, Changed, Event, Startup, WindowId};
use yantrik_app::control::View;
use yantrik_app::cx::Cx;
use yantrik_app::effects::Effects;
use yantrik_app::intents::Intents;
use yantrik_app::rail::RailCommand;
use yantrik_app::timers::TimerId;
use yantrik_app::Refused;

mod answers;
mod asks;
mod backend;
mod command;
mod describe;
mod events;
mod follow;
mod intents;
mod model;
mod notice;
mod ownership;
mod rail;
mod today;
mod update;
mod view;
/// The week and day views' arithmetic, and the rest of what is derived from dates: pure, with no
/// Slint in it.
pub mod views;
mod who;
mod work;
#[cfg(test)]
mod tests;

pub use command::Command;
pub use model::Model;

slint::include_modules!();
yantrik_app::window!(CalendarApp);
yantrik_app::rail_window!(CalendarApp);

/// The "now" line on the week and day grids moves once a minute.
pub const CLOCK: TimerId = TimerId("calendar.clock");
pub const CLOCK_EVERY: Duration = Duration::from_secs(60);

/// How often an open window asks the store whether anything has changed under it.
///
/// Twenty seconds. This is not a poll of the calendar — it is a poll of two numbers
/// (`calendar.revision`), and the month is re-listed only when they move. A calendar is written
/// at human speed by a person and at tool speed by a mind, and both of those already redraw this
/// window through the path that wrote; the timer is for the third writer, which is somebody
/// else's process, and twenty seconds of lag on an appointment nobody in this window made is not
/// worth a second of CPU. It never starts the calendar service (`backend::calls::revision`).
pub const STORE_WATCH: TimerId = TimerId("calendar.store_watch");
pub const STORE_WATCH_EVERY: Duration = Duration::from_secs(20);

pub struct Calendar;

impl App for Calendar {
    type Ui = CalendarApp;
    type Model = Model;
    type Command = Command;
    type Settings = ();
    const ID: &'static str = "calendar";

    fn model(boot: &Boot<()>) -> Result<Model, Startup> {
        Ok(Model::new(backend::for_app(&boot.paths), boot.mind.clone()))
    }

    fn wire(cx: &Cx<Self>) {
        view::wire(cx);
    }

    fn start(m: &Model, fx: &mut Effects<Self>) {
        fx.every(CLOCK, CLOCK_EVERY, Command::Minute);
        fx.every(STORE_WATCH, STORE_WATCH_EVERY, Command::Watch);
        update::arm_midnight(m, fx);
        // The first read. A `describe` that arrives before it lands reads the store itself
        // (`follow.rs`), so the first answer reports real events.
        fx.then(Command::Refresh);
    }

    fn update(m: &mut Model, command: Command, fx: &mut Effects<Self>) -> Result<(), Refused> {
        update::update(m, command, fx)
    }

    fn render(ui: &CalendarApp, _: WindowId, m: &Model, changed: &Changed) {
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
            // Brought back to the front: the date may have turned while the person was elsewhere.
            Event::Activated(_) => Some(Command::Activated),
            // A logout (a signal, the display going) or a person's quit: a write already sent to
            // the store is let land first (`update`). The framework still ends the process at its
            // own deadline after a signal, whatever is in flight.
            Event::ExitRequested(_) => Some(Command::Ending),
            // The last window closed while creates, deletes or updates are still in flight: it
            // closes, and the app lingers until they land (`Command::Closing`). With none, the
            // framework closes it and ends, as before.
            Event::CloseRequested(window) if m.writes > 0 => Some(Command::Closing(window)),
            // A second launch opened the window again while that close waits: the close is
            // cancelled. Raised once the window is shown, whatever the window manager does next.
            Event::Presented(_) if m.closing => Some(Command::Reopened),
            _ => None,
        }
    }

    fn rail(command: RailCommand) -> Option<Command> {
        Some(Command::Rail(command))
    }
}
