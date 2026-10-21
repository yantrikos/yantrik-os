//! How the core answers a framework event: the app's command for it (`App::event`), or the
//! default. Part of [`Core`]; here so the core's own file stays readable.

use crate::app::{App, Event};
use crate::problems;
use crate::rail::RailCommand;

use super::Core;

impl<A: App> Core<A> {
    /// An event: the app's command for it, or the default. The runner raises the first
    /// `Event::Launch` through this; a test may raise any.
    pub fn event(&self, event: Event<A::Settings>) {
        // An activation queued behind a command that closed its window: said of no window.
        if matches!(&event, Event::Activated(w) if !self.windows_open().contains(w)) {
            return;
        }
        // An app with a rail: the rail hears of the Mind's reach whatever the app makes of it.
        if let Event::ReachChanged(reach) = &event {
            if let Some(command) = A::rail(RailCommand::Reach(*reach)) {
                self.run(command);
            }
        }
        let command = A::event(event.clone(), &self.model.borrow());
        match (command, event) {
            (Some(command), _) => self.run(command),
            (None, Event::Launch(_)) => {}
            (None, Event::CloseRequested(window)) => {
                self.close_window(window);
                self.exit_if_no_window();
            }
            (None, Event::ExitRequested(reason)) => self.request_exit(reason),
            (None, Event::Activated(_) | Event::Presented(_)) => {}
            (None, Event::SettingsSaved) => {}
            (None, Event::SettingsChangedOutside(_)) => {}
            (None, Event::SettingsNotSaved(sentence)) => {
                problems::record_failure(self.id, "settings", &sentence);
            }
            // The app's own value stands and the file is brought back to it; never silently
            // (§6): the outside edit that lost is written down, so it can be found.
            (None, Event::SettingsConflict { mine, .. }) => {
                let why = "the settings file was edited outside the app while a save was \
                           pending; the app's value stands and was written over the edit";
                tracing::warn!(app = self.id, why);
                problems::record_failure(self.id, "settings_conflict", why);
                self.resolve_settings(mine);
            }
            (None, Event::DraftSaved(_)) => {}
            (None, Event::DraftNotSaved { name, why }) => {
                problems::record_failure(self.id, "draft", &format!("{name}: {why}"));
            }
            (None, Event::ReachChanged(_)) => {}
            (None, Event::FileChanged { .. }) => {}
            // Already logged and filed by the window table.
            (None, Event::WindowNotOpened { .. }) => {}
        }
    }
}
