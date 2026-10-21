//! A toy app with a close flow shaped as Notes' is (apps/notes/src/update/closing.rs): a quit
//! with unsaved work asks once, *Cancel* keeps it running, *Don't save* ends it; a logout saves
//! without asking (a draft, written now) and ends once that save lands, or at once with nothing
//! unsaved. Shared by `tests/quit.rs`, `tests/logout.rs` and `tests/signals.rs`, the last its own
//! binary because it raises SIGTERM at its own process.

#![allow(dead_code)]

use std::thread::ThreadId;

use yantrik_app::app::{App, Boot, Changed, Event, ExitReason, Startup, WindowId};
use yantrik_app::cx::Cx;
use yantrik_app::effects::Effects;
use yantrik_app::refused::Refused;
use yantrik_testing::TestApp;

slint::slint! {
    export component QuitterWin inherits Window {
        in-out property <bool> asking: false;
        callback quit-pressed();
    }
}

impl yantrik_app::Window for QuitterWin {
    fn make() -> Result<Self, slint::PlatformError> {
        QuitterWin::new()
    }
    fn apply_theme(&self, _: &yantrik_app::theme::ThemeSettings) {}
    fn connect_chrome(&self) {}
}

#[derive(Default)]
pub struct Model {
    pub unsaved: bool,
    /// The prompt is up.
    pub asking: bool,
    /// How many times the prompt was put up.
    pub asked: u32,
    /// How many logouts the app heard, and on which thread the last was applied.
    pub logouts: u32,
    pub logout_thread: Option<ThreadId>,
    /// A logout's quiet save is under way.
    pub saving: bool,
    pub opened: Vec<WindowId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Cancel,
    DontSave,
}

#[derive(Debug, Clone)]
pub enum Command {
    Edit,
    OpenWindow,
    /// Ctrl+Q, as a key binding raises it.
    QuitPressed,
    /// `Event::ExitRequested` from a person: the close flow over every window.
    Quit,
    Answer(Answer),
    /// `Event::ExitRequested(Logout)`: no one to ask.
    LoggingOut,
    /// The logout's quiet save landed.
    Saved,
}

pub struct Quitter;

/// The draft a logout keeps of the unsaved text.
pub const DRAFT: &str = "unsaved";
pub const DRAFT_TEXT: &[u8] = b"the text not yet saved";

impl App for Quitter {
    type Ui = QuitterWin;
    type Model = Model;
    type Command = Command;
    type Settings = ();
    const ID: &'static str = "quitter-test";

    fn model(_: &Boot) -> Result<Model, Startup> {
        Ok(Model::default())
    }

    fn wire(cx: &Cx<Self>) {
        cx.ui().on_quit_pressed(cx.cmd(|| Command::QuitPressed));
    }

    fn wire_window(cx: &Cx<Self>, ui: &QuitterWin, _: WindowId) {
        ui.on_quit_pressed(cx.cmd(|| Command::QuitPressed));
    }

    fn update(m: &mut Model, command: Command, fx: &mut Effects<Self>) -> Result<(), Refused> {
        match command {
            Command::Edit => m.unsaved = true,
            Command::OpenWindow => m.opened.push(fx.open_window()),
            Command::QuitPressed => fx.request_quit(),
            Command::Quit if !m.unsaved => fx.exit(),
            // One prompt at a time: a second quit while it is up asks nothing more.
            Command::Quit if m.asking => {}
            Command::Quit => {
                m.asking = true;
                m.asked += 1;
            }
            Command::Answer(Answer::Cancel) => m.asking = false,
            Command::Answer(Answer::DontSave) => fx.exit(),
            Command::LoggingOut => {
                m.logouts += 1;
                m.logout_thread = Some(std::thread::current().id());
                // Nothing unsaved: the end now, as Notes' logout ends.
                if !m.unsaved {
                    fx.exit();
                    return Ok(());
                }
                // Saved without asking: the unsaved text as a draft, written now.
                fx.checkpoint_draft(DRAFT, DRAFT_TEXT.to_vec());
                m.asking = false;
                m.saving = true;
            }
            Command::Saved => {
                m.saving = false;
                m.unsaved = false;
                fx.exit();
            }
        }
        Ok(())
    }

    fn render(ui: &QuitterWin, _: WindowId, m: &Model, _: &Changed) {
        ui.set_asking(m.asking);
    }

    fn event(event: Event<()>, _: &Model) -> Option<Command> {
        match event {
            Event::ExitRequested(ExitReason::Logout) => Some(Command::LoggingOut),
            Event::ExitRequested(_) => Some(Command::Quit),
            _ => None,
        }
    }
}

/// The thread the app's commands are applied on.
pub fn ui_thread(app: &TestApp<Quitter>) -> ThreadId {
    app.model(|_| std::thread::current().id())
}
