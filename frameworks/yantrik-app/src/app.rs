//! What an app is to the framework: one trait, and the runner drives it
//! (design/app-framework-2026-10-21.md, §2).
//!
//! There is no base struct to inherit from. The app implements [`App`]; the runner makes the
//! window, keeps the model on the UI thread, applies every command through [`App::update`],
//! draws the result through [`App::render`], and ends the process in order. Step 2 of the
//! design: the model, commands, effects, jobs and timers. The surface's intents (step 3), the
//! manifest, paths and settings (step 4) and the clients (step 5) are added beside these.

use std::ffi::OsString;

use crate::control::View;
use crate::cx::Cx;
use crate::drafts::Drafts;
use crate::effects::Effects;
use crate::intents::Intents;
use crate::launch::Launch;
use crate::paths::Paths;
use crate::refused::Refused;
use crate::settings::{Health, Settings, Store};
use crate::window::Window;

/// One window of the app. `PRIMARY` is the one the runner makes at start; the design allows
/// more, and nothing here assumes there is only one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowId(pub u32);

impl WindowId {
    pub const PRIMARY: WindowId = WindowId(0);
}

/// Why the process was asked to end. As `Event::ExitRequested`, it says who asked, and so
/// whether anyone is there to answer a question (`crate::quit`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitReason {
    /// The last window closed, or a linger after it ran past its backstop (`fx.linger`).
    LastWindowClosed,
    /// The app ended itself (`fx.exit()`): the reason recorded, never raised as an event by the
    /// framework.
    App,
    /// The session or the system is ending the process: SIGTERM (a shutdown, a logout, a kill),
    /// SIGHUP (the session hung up), or the display went (the compositor exited). No one is there
    /// to answer, and the process is killed after a grace period whatever the app does: save
    /// without asking, then `fx.exit()`. A bounded chance to save, not a completed interactive
    /// save; after a signal the framework ends the process itself if the app has not within
    /// [`crate::quit::SIGNAL_DEADLINE`]. Hand the save over in the command that answers (a draft
    /// checkpointed, the settings saved): at the end, a `then` or a job's result still queued is
    /// not applied.
    Logout,
    /// A person asked the app to quit: the app's `fx.request_quit()`, from Ctrl+Q or a Quit
    /// menu. Someone is there to answer: the app may ask about unsaved work, once over every
    /// window, and the person may cancel, which keeps the app running.
    Person,
}

/// Something the framework saw that the app may turn into a command. `S` is the app's
/// [`App::Settings`].
#[derive(Debug, Clone, PartialEq)]
pub enum Event<S> {
    /// The first launch, once, after `start`; and every launch handed over by a second copy. The
    /// files to open or the deep link's intent are in it. `None` opens nothing.
    ///
    /// The intent was admitted before it got here (`intents::admit`): its name is one this app
    /// declares, its arguments pass the declared parameters' check, and one that came by
    /// hand-over is graded standard or below. The app runs it as the person's own command.
    Launch(Launch),
    /// The person asked a window to close. `None` from [`App::event`] closes it now; a command
    /// keeps it open until something runs `fx.close(window)`, so "Save changes?" is a flow. The
    /// command that closes the last window may also run `fx.linger`, to finish work still in
    /// flight with no window before it ends.
    CloseRequested(WindowId),
    /// The process is asked to end: by a person (`ExitReason::Person`) or by the session or the
    /// system (`ExitReason::Logout`). `None` ends it now; a command may save first and then
    /// `fx.exit()`, or, for a person's quit, ask and keep running when the person cancels.
    ExitRequested(ExitReason),
    /// A window became the active one: the window manager gave it the keyboard (winit's
    /// `Focused(true)`), because the person clicked into it, switched to it or brought it back,
    /// or because it was just shown. Raised once per activation: a window that is already active
    /// raises nothing more, so it fires again only after the window lost the activation to
    /// another window, this app's or another's. Never raised when a window loses the activation,
    /// for a window that is closed, or where there is no window manager (the headless platform;
    /// a test raises it with `TestApp::activate`). The window manager's hook only queues it; it
    /// is applied in the drain right after, and one queued behind a command that closed its
    /// window is dropped. `None` does nothing.
    Activated(WindowId),
    /// A second launch brought this window to the person: the desktop's `present`, or a launch
    /// handed over (`launch`, after its `Event::Launch`). Raised once the window is shown, every
    /// time, whether or not the window manager then gives it the keyboard, so it is the reliable
    /// word that the person is back; `Activated` follows only if the window manager says so.
    ///
    /// During a linger (`fx.linger`) no window is open, and the first window is opened again,
    /// drawn whole from the model: the linger is over, the app runs on as before it closed, and
    /// this is where it cancels the exit it was waiting to make. `None` does nothing.
    Presented(WindowId),
    /// The last `fx.save_settings` reached the disk. `None` does nothing.
    SettingsSaved,
    /// A save did not reach the disk; the value is kept in memory and will be tried again with
    /// the next save. `None` files a problem record and does nothing else; an app with a notice
    /// line shows the sentence.
    SettingsNotSaved(String),
    /// The settings file was edited outside this app and nothing of this app's was pending: the
    /// value is the file's now. `None` leaves the model as it was, so an app that keeps its
    /// settings in its model answers this with the command that adopts them.
    SettingsChangedOutside(S),
    /// The settings file was edited outside this app while a save of this app's was pending.
    /// `None` keeps this app's value and writes it, and files a problem record saying an outside
    /// edit lost (never silently, §6); an app that prefers the file answers with a command that
    /// saves `outside`. A file repaired by hand while the app was in recovery is not a conflict:
    /// the repair is the person's choice and arrives as `SettingsChangedOutside`.
    SettingsConflict { outside: S, mine: S },
    /// The last `fx.save_draft` of this name reached the disk. `None` does nothing.
    /// A discard raises nothing: this means the draft is on disk at its latest revision.
    DraftSaved(String),
    /// A draft did not reach the disk. `None` files a problem record.
    DraftNotSaved { name: String, why: String },
    /// Whether the Mind can answer changed (the shell came or went, a model answered or did not),
    /// as the housekeeping thread heard it. An app with an AI control shows `reach.hint()` in its
    /// place, or offers it again. `None` does nothing.
    ReachChanged(yantrik_mind::Reach),
    /// A watched document's file changed from what was last seen: another program wrote it,
    /// or it was moved or deleted (`fx.watch`). Our own saves are seen too; the document's
    /// [`observe`](crate::documents::Document::observe) tells them apart. `None` does nothing.
    FileChanged {
        key: String,
        seen: crate::documents::Observed,
    },
    /// A window `fx.open_window` returned the id of could not be made; nothing will be drawn for
    /// it, and the app drops what it attached to the id. `None` does nothing more: the failure is
    /// already logged and filed as a problem record.
    WindowNotOpened { window: WindowId, why: String },
}

/// What the runner knows before any window exists.
#[derive(Debug, Clone)]
pub struct Boot<S = ()> {
    /// The command line after the program name, as given.
    pub args: Vec<OsString>,
    /// What the command line asked for: files, or a deep link. Read here; applied once as
    /// `Event::Launch` after `start`.
    pub launch: Launch,
    /// Where this app's files are.
    pub paths: Paths,
    /// The drafts found at start, by name. The app reads back what it kept.
    pub drafts: Drafts,
    /// The settings as read from disk (migrated if older), or the defaults when there is no file
    /// or it could not be used.
    pub settings: S,
    /// Whether the settings file is this app's to write. In recovery the app runs on the
    /// defaults, and a save is held in memory until the person chooses to replace the file
    /// (`fx.replace_settings_file`).
    pub settings_health: Health,
    /// The Mind, as this process reaches it: the live one under the runner, a fake under a
    /// test. An app with an agent rail hands it to its `Rail`.
    pub mind: std::sync::Arc<dyn yantrik_mind::Mind>,
    /// The platform's services, typed: live under the runner, fakes under a test unless the
    /// test gives its own. An app's jobs call them; nothing here does.
    pub services: crate::services::Services,
}

impl<S: Settings> Boot<S> {
    /// Parses the launch, opens the settings and reads the drafts at `paths`, and makes the boot
    /// the app's `model` is given. An unusable settings file is `settings_health`, not an error;
    /// a command line the app does not take is.
    pub fn open(
        paths: Paths,
        args: Vec<OsString>,
        mind: std::sync::Arc<dyn yantrik_mind::Mind>,
        services: crate::services::Services,
    ) -> Result<(Boot<S>, Store<S>), Startup> {
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/"));
        let launch = Launch::parse(paths.id(), paths.profile(), &cwd, &args)?;
        let store = Store::<S>::open(paths.config_file().to_path_buf());
        let drafts = Drafts::read(paths.state_dir());
        let boot = Boot {
            args,
            launch,
            drafts,
            paths,
            settings: store.get().clone(),
            settings_health: store.health().clone(),
            mind,
            services,
        };
        Ok((boot, store))
    }
}

/// Why this copy of the app does not go on to run.
///
/// Most often it could not start: printed as a sentence, filed as a problem, exit 1. Or another
/// copy took this launch ([`Startup::handed_over`]): the start worked as meant, so it is logged
/// and exits 0 with nothing filed, as the framework's own hand-over to a running copy does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Startup {
    pub sentence: String,
    handed_over: bool,
}

impl Startup {
    /// The app could not start, and `sentence` says why.
    pub fn new(sentence: impl Into<String>) -> Startup {
        Startup {
            sentence: sentence.into(),
            handed_over: false,
        }
    }

    /// Another copy of the app has taken this launch: an app's own claim beyond the framework's
    /// (one copy per library, say) found the copy that holds it. Not a failure.
    pub fn handed_over(sentence: impl Into<String>) -> Startup {
        Startup {
            sentence: sentence.into(),
            handed_over: true,
        }
    }

    pub fn is_handed_over(&self) -> bool {
        self.handed_over
    }
}

impl std::fmt::Display for Startup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.sentence)
    }
}

/// Which parts of the model a command changed, so `render` redraws those. A part is a feature's
/// name (`forecast::PART`). All parts unless an `update` narrowed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Changed {
    All,
    Parts(Vec<&'static str>),
}

impl Changed {
    pub fn all() -> Changed {
        Changed::All
    }

    pub fn none() -> Changed {
        Changed::Parts(Vec::new())
    }

    pub fn has(&self, part: &str) -> bool {
        match self {
            Changed::All => true,
            Changed::Parts(parts) => parts.contains(&part),
        }
    }

    pub fn is_empty(&self) -> bool {
        matches!(self, Changed::Parts(p) if p.is_empty())
    }

    pub(crate) fn add(&mut self, part: &'static str) {
        if let Changed::Parts(parts) = self {
            if !parts.contains(&part) {
                parts.push(part);
            }
        }
    }
}

/// An app, as the runner drives it.
pub trait App: Sized + 'static {
    /// The primary window's generated component. `yantrik_app::window!(X)` makes it a `Window`.
    type Ui: Window;
    /// Plain Rust, composed of feature models. Never holds a Slint handle or a list model.
    type Model: 'static;
    /// Plain data, `Send` and `Clone`: it crosses from worker threads, timers (which make one
    /// each time they fire) and the surface's thread.
    type Command: Send + Clone + 'static;
    /// What the app remembers between runs: typed, with a schema number, kept at
    /// `Paths::config_file`. `()` for an app with nothing to remember.
    type Settings: Settings;

    /// The app's one id: the instance claim, the socket, the tracing target. The manifest
    /// (step 4) supplies it from `yantrik.toml`.
    const ID: &'static str;

    /// Build the model before any window exists. No UI. `boot.settings` is what was saved.
    /// `Err(Startup::new(..))` when the app cannot start; `Err(Startup::handed_over(..))` when
    /// another copy has taken this launch, which ends the process quietly.
    fn model(boot: &Boot<Self::Settings>) -> Result<Self::Model, Startup>;

    /// Bind the window's callbacks to commands. Once, on the UI thread, after the window is made
    /// and themed.
    fn wire(cx: &Cx<Self>);

    /// Bind a window opened later (`fx.open_window`) to commands, as `wire` does the first.
    /// The id is the window's, so a command can say which window it came from. The default
    /// binds nothing.
    fn wire_window(cx: &Cx<Self>, ui: &Self::Ui, window: WindowId) {
        let _ = (cx, ui, window);
    }

    /// The first effects: an initial fetch, timers. After `wire` and the first `render`.
    fn start(model: &Self::Model, fx: &mut Effects<Self>) {
        let _ = (model, fx);
    }

    /// The only function that changes the model. Validate before changing anything: a
    /// `Refused` must leave the model as it was, and no effect it requested is run.
    fn update(
        model: &mut Self::Model,
        command: Self::Command,
        fx: &mut Effects<Self>,
    ) -> Result<(), Refused>;

    /// The only function that writes Slint properties. After every applied command, with the
    /// parts that changed; once at start with `Changed::All`.
    fn render(ui: &Self::Ui, window: WindowId, model: &Self::Model, changed: &Changed);

    /// The Mind's view of the app: a projection of the committed model, never the model itself
    /// (no caches, no credentials). Runs on every `describe` and around every `act`.
    fn describe(model: &Self::Model) -> View {
        let _ = model;
        View::new(Self::ID)
    }

    /// The commands the Mind, keys and menus may raise by name (`intents::Intent`).
    fn intents(table: &mut Intents<Self>) {
        let _ = table;
    }

    /// A framework event the app may turn into a command. `None` takes the default.
    fn event(event: Event<Self::Settings>, model: &Self::Model) -> Option<Self::Command> {
        let _ = (event, model);
        None
    }

    /// The agent rail's commands as this app's own (`Some(Command::Rail(command))` for an app
    /// with a rail, see `rail`). `None` means the app has no rail: the framework arms no refresh
    /// for it and routes nothing to it.
    fn rail(command: crate::rail::RailCommand) -> Option<Self::Command> {
        let _ = command;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changed_parts_accumulate_and_all_covers_everything() {
        let mut c = Changed::none();
        assert!(c.is_empty());
        c.add("forecast");
        c.add("forecast");
        assert_eq!(c, Changed::Parts(vec!["forecast"]));
        assert!(c.has("forecast") && !c.has("places"));
        let mut all = Changed::all();
        all.add("x");
        assert!(all.has("anything"));
    }
}
