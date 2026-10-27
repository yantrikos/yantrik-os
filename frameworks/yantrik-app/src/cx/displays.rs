//! Whose each window is (`crate::displays`): the person's windows and a Mind's, in the one core.
//! Part of [`Core`]; here so the core's own file stays readable.
//!
//! A launch from the person's display reaches [`Core::present`], which only ever brings forward
//! or opens again a window of the person's. A launch from a Mind's display reaches
//! [`Core::present_for_mind`], which only ever opens or raises that Mind's window, drawn for its
//! view helper. Neither is handed to the other's window, so the person's launch never lands in a
//! Mind's masked window, and a Mind never drives the person's.
//!
//! A Mind's window is the framework's own: nothing in it is the person's to save, so its close
//! (from its title bar, its helper, or its helper going away) is not asked of the app. The app
//! hears `Event::Opened` before it is first drawn and `Event::Closed` once it is gone.

use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use slint::ComponentHandle as _;

use super::{Core, Cx};
use crate::app::{App, Changed, Event, WindowId};
use crate::displays::link::{self, Heard, Link};
use crate::displays::streamed::{self, Sink};
use crate::displays::wire::ToHelper;
use crate::displays::platform;
use crate::displays::refused_beside_the_person;
use crate::problems;
use crate::theme;
use crate::window::Window as _;

/// A Mind's window: the display it is for, and the link to the helper that shows it there (none
/// under a test, which draws it headless).
pub(crate) struct MindWindow {
    pub(crate) display: Option<String>,
    link: Option<Rc<Link>>,
}

impl<A: App> Core<A> {
    /// The person's windows open now, the first one first.
    pub fn person_windows(&self) -> Vec<WindowId> {
        self.windows_open().into_iter().filter(|w| !w.for_mind()).collect()
    }

    /// A Mind's windows open now, with the display each is for.
    pub fn mind_windows(&self) -> Vec<(WindowId, Option<String>)> {
        let windows = self.windows.borrow();
        windows.minds.iter().map(|(id, w)| (*id, w.display.clone())).collect()
    }

    /// A launch from a Mind on `display`: that Mind's window, raised when it is open, else opened
    /// and drawn for the helper listening at `view`, which must be the process `asker`. Answers
    /// as `present` does, with `view` saying whether this launch's helper now shows the window.
    ///
    /// Without a helper the window could be seen by no one, so one is required wherever the
    /// window would be streamed; a test's headless window needs none.
    pub fn present_for_mind(
        &self,
        display: Option<String>,
        view: Option<&Path>,
        asker: Option<i32>,
    ) -> Result<serde_json::Value, String> {
        if self.exit.get().is_some() {
            return Err("the app is ending, so it opens no window; open it again once it has".into());
        }
        // H3: a Mind watches the person's live state only through a window that masks it.
        if !A::MASKS_FOR_MIND && !self.person_windows().is_empty() {
            return Err(refused_beside_the_person(A::ID));
        }
        let open = self.mind_windows().into_iter().find(|(_, d)| *d == display).map(|(id, _)| id);
        let (window, streamed) = match open {
            Some(window) => {
                self.raise_mind(window);
                (window, false)
            }
            None => {
                let link = match view {
                    Some(path) => Some(Rc::new(link::connect(path, asker)?)),
                    None if platform::installed() => {
                        return Err("a Mind's window is drawn for its launch from the Mind's display, \
                                    and this request came with no view to draw it in"
                            .into())
                    }
                    None => None,
                };
                let streamed = link.is_some();
                (self.open_mind_window(display.clone(), link)?, streamed)
            }
        };
        Ok(serde_json::json!({
            "window": true,
            "presented": true,
            "display": display,
            "elsewhere": false,
            "view": streamed,
            "mind_window": window.0,
        }))
    }

    /// The person's window is about to be shown: an app that does not mask for a Mind closes every
    /// Mind's window first, so none of them goes on to draw what the person does (#972 H3).
    pub(super) fn before_the_persons_window(&self) {
        if A::MASKS_FOR_MIND {
            return;
        }
        for (window, _) in self.mind_windows() {
            tracing::info!(app = self.id, window = window.0, "the person's window opens: a Mind's window of an app that does not mask closes");
            self.close_window(window);
            self.event(Event::Closed(window));
        }
    }

    /// Writes whose windows are open where the shell reads it (`displays::report`), when the runner
    /// asked for a report.
    pub(super) fn report_windows(&self) {
        let windows = self.windows.borrow();
        let Some(reporter) = &windows.reporter else { return };
        let person = u32::from(windows.first_open) + windows.more.keys().filter(|w| !w.for_mind()).count() as u32;
        let mut minds: Vec<String> = windows.minds.values().map(|w| w.display.clone().unwrap_or_else(|| "unnamed".into())).collect();
        minds.sort();
        minds.dedup();
        reporter.write(person, minds);
    }

    /// The runner's: report this instance's windows to the shell from now on.
    pub fn report_windows_to(&self, reporter: crate::displays::report::Reporter) {
        self.windows.borrow_mut().reporter = Some(reporter);
        self.report_windows();
    }

    /// Asks a Mind's window's helper to bring it forward on its display.
    fn raise_mind(&self, window: WindowId) {
        let link = self.windows.borrow().minds.get(&window).and_then(|w| w.link.clone());
        if let Some(link) = link {
            link.send(ToHelper::Raise);
        }
    }

    /// A Mind's window, made as a streamed window, themed, wired, drawn whole and shown.
    fn open_mind_window(&self, display: Option<String>, link: Option<Rc<Link>>) -> Result<WindowId, String> {
        let id = super::windows::next_mind_window_id();
        let (made, adapter) = streamed::make(A::Ui::make);
        let ui = made.map_err(|e| {
            let why = format!("a Mind's window could not be made: {e}");
            problems::record_failure(self.id, "window", &why);
            why
        })?;
        if platform::installed() && adapter.is_none() {
            return Err("a Mind's window was not made for its display; it is not shown here".into());
        }
        let theme = self.windows.borrow().theme.unwrap_or_else(theme::load);
        ui.apply_theme(&theme);
        ui.connect_chrome();
        // Its close is the framework's, outside the window's own callback.
        let core = self.me.clone();
        ui.window().on_close_requested(move || {
            let core = core.clone();
            slint::Timer::single_shot(Duration::ZERO, move || {
                if let Some(core) = core.upgrade() {
                    core.close_mind_window(id);
                }
            });
            slint::CloseRequestResponse::KeepWindowShown
        });
        if let (Some(adapter), Some(link)) = (&adapter, &link) {
            adapter.attach(link.clone() as Rc<dyn Sink>);
            let (core, adapter) = (self.me.clone(), Rc::downgrade(adapter));
            link::on_ui(
                link.id(),
                Box::new(move |heard| match heard {
                    Heard::Message(message) => {
                        if let Some(adapter) = adapter.upgrade() {
                            adapter.deliver(message);
                        }
                    }
                    Heard::Gone => {
                        if let Some(core) = core.upgrade() {
                            core.close_mind_window(id);
                        }
                    }
                    Heard::Redraw => {
                        if let Some(adapter) = adapter.upgrade() {
                            adapter.redraw_whole();
                        }
                    }
                }),
            );
        }
        {
            let mut windows = self.windows.borrow_mut();
            windows.more.insert(id, ui.clone_strong());
            windows.minds.insert(id, MindWindow { display: display.clone(), link });
        }
        if let Some(core) = self.me.upgrade() {
            A::wire_window(&Cx::new(core), &ui, id);
        }
        // A window open again: an instance waiting for its first one, or lingering, runs on.
        self.linger_ended();
        self.event(Event::Opened(id));
        A::render(&ui, id, &self.model.borrow(), &Changed::all());
        if let Err(e) = ui.show() {
            let why = format!("a Mind's window could not be shown: {e}");
            problems::record_failure(self.id, "window", &why);
            self.close_mind_window(id);
            return Err(why);
        }
        let on = display.as_deref().unwrap_or("unnamed");
        tracing::info!(app = self.id, window = id.0, display = on, "a Mind's window is open, on its own display");
        self.report_windows();
        Ok(id)
    }

    /// A Mind's window closed by the framework: its helper is told, the app hears
    /// `Event::Closed`, and the process ends if no window of anyone's is left.
    pub fn close_mind_window(&self, window: WindowId) {
        if !window.for_mind() || !self.windows_open().contains(&window) {
            return;
        }
        self.close_window(window);
        self.event(Event::Closed(window));
        self.exit_if_no_window();
    }

    /// Whether the call being handled now may run as a launch of a Mind's: what it asks to open is
    /// refused, since the app would apply it to the person's model (#972 M1).
    pub(crate) fn mind_launch_opens_nothing(launch: &crate::launch::Launch) -> Result<(), String> {
        if launch.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "A Mind's launch opens no file or link in {}: a Mind opens them through the app's actions. Nothing was opened.",
                A::ID
            ))
        }
    }

    /// The runner's start for an instance a Mind's first launch started ([`crate::displays::spawn`]):
    /// no window is shown, and the process waits up to `wait` for one, as a linger does.
    pub fn start_windowless(&self, wait: Duration) {
        let _ = self.ui.hide();
        self.windows.borrow_mut().first_open = false;
        self.report_windows();
        self.linger_for_first_window(wait);
    }
}
