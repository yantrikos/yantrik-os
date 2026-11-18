//! Yantrik Images, on YantrikApp (design/app-framework-2026-10-21.md, step 10: behaviour
//! unchanged).
//!
//! It once could not open a picture: no argument handling, no control surface, no file path
//! anywhere, and a 24 MB binary nothing could reach (design/images-2026-09-20.md). Now it opens
//! the file it is given and the folder around it, a second launch hands its file to the window
//! already open (`Event::Launch`), and it answers on a control surface so Files, the launcher
//! and a mind reach the same window. Reading a picture and turning it are jobs; the screen
//! changes when the newest one lands. The eight actions are intents, graded as they were; the
//! agent rail is the framework's; and `main.rs` is three lines.

use yantrik_app::app::{App, Boot, Changed, Event, Startup, WindowId};
use yantrik_app::control::View;
use yantrik_app::cx::Cx;
use yantrik_app::effects::Effects;
use yantrik_app::intents::Intents;
use yantrik_app::rail::RailCommand;
use yantrik_app::Refused;

mod agent_rule;
mod ai;
mod describe;
mod intents;
mod model;
mod open;
mod orient;
mod picture;
mod rail;
mod update;
mod view;
#[cfg(test)]
mod tests;

pub use model::{Command, Model, ViewState, NO_PICTURE};
pub use open::Loaded;
pub use orient::Orientation;

slint::include_modules!();
yantrik_app::window!(ImageViewerApp);

pub struct ImageViewer;

impl App for ImageViewer {
    type Ui = ImageViewerApp;
    type Model = Model;
    type Command = Command;
    type Settings = ();
    const ID: &'static str = "image-viewer";

    fn model(boot: &Boot<()>) -> Result<Model, Startup> {
        Ok(Model::new(boot.mind.clone()))
    }

    fn wire(cx: &Cx<Self>) {
        view::wire(cx);
    }

    fn update(m: &mut Model, command: Command, fx: &mut Effects<Self>) -> Result<(), Refused> {
        update::update(m, command, fx)
    }

    fn render(ui: &ImageViewerApp, _: WindowId, m: &Model, changed: &Changed) {
        view::render(ui, m, changed);
    }

    fn describe(m: &Model) -> View {
        describe::view(m)
    }

    fn intents(table: &mut Intents<Self>) {
        intents::table(table);
    }

    fn event(event: Event<()>, _: &Model) -> Option<Command> {
        match event {
            // Every launch, the first one included, so the first is told from a hand-over. A
            // deep link's intent is not something this app has ever taken; it is left alone.
            Event::Launch(launch) => Some(Command::Launch(launch.files)),
            _ => None,
        }
    }

    fn rail(command: RailCommand) -> Option<Command> {
        Some(Command::Rail(command))
    }
}
