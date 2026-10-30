//! Yantrik Mind desk, on YantrikApp (#1023, phase 1): the bars around Mind View's apps.
//!
//! Mind View is a second labwc, nested, in which the apps a Mind opens are drawn. This app runs
//! inside it, started by the shell itself (not by the compositor's startup script), and draws two
//! windows the nested compositor keeps above and below the apps (config/labwc-mind/rc.xml): above,
//! the Mind's name and state, the tool it is running, Stop… and Open in Agents, and the tool's
//! mark and the counts of its run; below, the apps open, how the Mind is housed (its own account or
//! the person's), and the model it says it runs.
//!
//! # Why two windows
//!
//! One window tall enough to hold both bars would sit over the apps and take their clicks. Two
//! leave the middle to them, and each reserves its own margin in the compositor.
//!
//! # What it is told, and what it may do
//!
//! A Mind acting in Mind View can read this window and press its buttons, so the desk is built
//! for that:
//!
//! - It is told display-safe fields only (`mind_desk`), never what a Mind says it is doing, which
//!   is withheld from every other Mind and this is a window they all read. It waits on a socket
//!   for a change and never on a timer.
//! - It stops nothing. Stop… asks the shell, which asks the *person* on their own desktop, and
//!   only the person's Allow there stops the Mind. Open in Agents asks the shell to put up the
//!   person's screen, which it does only while Mind View is in front, and rarely.
//! - The shell answers all three only to this one process, which it started and knows by pid and
//!   start time. There is no way here to approve, deny, grant, take over or hand back, and
//!   `yantrik_services::mind_desk::MindDeskApi` is where that is held to a list.
//!
//! # How it is put together
//!
//! - `words.rs`: every sentence and colour, as plain functions. `chrome.rs`: the model as the
//!   two windows' words. `view.rs`: the only code that writes a window. `describe.rs`: what a
//!   Mind reads of it.
//! - `model.rs` and `update.rs`: the state, and the only code that changes it.
//! - `metrics.rs`: the numbers the nested compositor's rules agree with.

use yantrik_app::app::{App, Boot, Changed, Event, Startup, WindowId};
use yantrik_app::control::View;
use yantrik_app::cx::Cx;
use yantrik_app::effects::Effects;
use yantrik_app::Refused;

mod chrome;
mod describe;
pub mod metrics;
mod model;
mod update;
mod view;
pub mod words;
#[cfg(test)]
mod tests;

pub use model::{Command, Link, Model, Stop};

slint::include_modules!();
yantrik_app::window!(MindDeskApp);

pub struct MindDesk;

impl App for MindDesk {
    type Ui = MindDeskApp;
    type Model = Model;
    type Command = Command;
    type Settings = ();
    const ID: &'static str = "mind-desk";

    /// An app that is drawn on a Mind's display, and on no other: the framework starts it there
    /// and refuses to start it on the person's.
    const ON_MINDS_DISPLAY: bool = true;

    fn model(boot: &Boot<()>) -> Result<Model, Startup> {
        Ok(Model {
            desk: boot.services.mind_desk.clone(),
            link: Link::Connecting,
            snapshot: None,
            stop: Stop::Resting,
            bottom: None,
            asking: false,
            asked_at: None,
            retry: model::RETRY_FROM,
            note: String::new(),
        })
    }

    fn wire(cx: &Cx<Self>) {
        let ui = cx.ui();
        view::wire(cx, &ui);
    }

    fn wire_window(cx: &Cx<Self>, ui: &MindDeskApp, _: WindowId) {
        view::wire(cx, ui);
    }

    fn start(_: &Model, fx: &mut Effects<Self>) {
        fx.then(Command::OpenFooter);
        fx.then(Command::Ask);
    }

    fn update(m: &mut Model, command: Command, fx: &mut Effects<Self>) -> Result<(), Refused> {
        update::update(m, command, fx)
    }

    fn render(ui: &MindDeskApp, window: WindowId, m: &Model, _: &Changed) {
        view::render(ui, window, m);
    }

    fn describe(m: &Model) -> View {
        describe::view(m)
    }

    fn event(event: Event<()>, _: &Model) -> Option<Command> {
        match event {
            // The bars are the room's walls, not windows a person closes. A close asked of one
            // (Alt+F4 is bound in the nested compositor) changes nothing.
            Event::CloseRequested(_) => Some(Command::CloseAsked),
            Event::WindowNotOpened { .. } => Some(Command::FooterNotOpened),
            _ => None,
        }
    }
}
