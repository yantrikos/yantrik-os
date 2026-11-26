//! The dock, kept as current as the compositor's stream (#1423).
//!
//! The dock used to be published only by the three-second poll, from the window list, while the
//! minimized state came from the foreign-toplevel stream the moment a `done` arrived. So between
//! polls the screen and `describe shell` showed the window list of one event ago: minimize Notes
//! and `.dock` still said `focused`, restore it and `.dock` said `minimized`.
//!
//! Now one function, [`refresh`], builds the dock and the window list it is made from, and it
//! runs on the UI thread at three moments: the poll, every `done` (and close) of the stream, and
//! each time `describe shell` reads the dock. The models the screen is drawn from are the models
//! `describe` reads, so they cannot disagree.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use slint::ComponentHandle;

use crate::app_context::AppContext;
use crate::apps::Catalogue;
use crate::App;

static CATALOGUE: OnceLock<Catalogue> = OnceLock::new();
static HOOK: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

/// Wakes the UI thread at most once for any number of events that arrive before it runs.
#[derive(Default)]
pub(crate) struct Coalesce(AtomicBool);

impl Coalesce {
    /// `true` for the caller that must schedule a run: nobody has since the last [`Coalesce::ran`].
    pub(crate) fn want(&self) -> bool {
        !self.0.swap(true, Ordering::AcqRel)
    }
    /// The scheduled run has started, so it will read everything that has happened until now.
    pub(crate) fn ran(&self) {
        self.0.store(false, Ordering::Release);
    }
}

/// Called by the stream thread after it has taken in a `done` or a close: the shell's own account
/// of the windows has changed, so the dock must be drawn again from it.
pub fn stream_changed() {
    if let Some(hook) = HOOK.get() {
        hook();
    }
}

pub fn wire(ui: &App, ctx: &AppContext) {
    let _ = CATALOGUE.set(ctx.installed_apps.clone());
    let weak = ui.as_weak();
    static PENDING: std::sync::LazyLock<Coalesce> = std::sync::LazyLock::new(Coalesce::default);
    let _ = HOOK.set(Box::new(move || {
        if PENDING.want() {
            let _ = weak.upgrade_in_event_loop(|ui| {
                PENDING.ran();
                refresh(&ui, true);
            });
        }
    }));
}

/// Build the shell's window list and the dock from the one reading of the windows, and put them on
/// the screen. `event` is a window event (not the poll, not `describe`): it also tells the dock to
/// forget what the pointer was resting on, since the pointer may have left without a word.
pub fn refresh(ui: &App, event: bool) {
    let Some(catalogue) = CATALOGUE.get() else { return };
    let installed = catalogue.get();
    let wins = crate::windows::list_windows_throttled();
    super::pins::publish(ui, &installed);
    let items = super::system_poll::window_items(ui.get_current_screen(), &wins, crate::windows::shell_in_front());
    if let Some(model) = crate::models::changed(ui.get_window_list(), items) {
        ui.set_window_list(model);
    }
    super::dock_bar::publish(ui, &installed);
    if event {
        ui.set_dock_window_event(ui.get_dock_window_event().wrapping_add(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dock_model::{self, LaunchOrder, Pin, Win};
    use crate::toplevel_stack::{apps_put_away, set_minimized};
    use crate::wire::dock_bar::{describe_value, DescribeRow};

    /// One window as the stream has it after a `done`.
    struct Seen {
        title: &'static str,
        app: &'static str,
        minimized: bool,
        activated: bool,
    }

    /// What `describe shell` says for `.dock`, from the stream's windows: the same steps `refresh`
    /// takes (the stream's list, grouped by app, minimized from the stream), without a screen.
    fn describe_after_done(stream: &[Seen]) -> serde_json::Value {
        let by_app: Vec<(String, bool)> = stream.iter().map(|w| (w.app.to_string(), w.minimized)).collect();
        set_minimized(Vec::new(), apps_put_away(&by_app));
        let mut front: Vec<&Seen> = stream.iter().filter(|w| w.activated && !w.minimized).collect();
        front.extend(stream.iter().filter(|w| !(w.activated && !w.minimized)));
        let wins: Vec<Win> = front.iter().map(|w| Win { title: w.title.into(), app_id: w.app.into(), subtitle: String::new() }).collect();
        let pins = [Pin { app_id: "notes".into(), label: "Notes".into() }];
        let first = wins.first().map(|w| w.app_id.clone());
        let mut entries = dock_model::entries(&pins, &wins, first.as_deref(), &mut LaunchOrder::default(), |id, _| id.to_string());
        dock_model::apply_minimized(&mut entries, &crate::toplevel_stack::minimized_apps());
        let rows: Vec<DescribeRow> = entries
            .iter()
            .map(|e| DescribeRow { app: e.app_id.clone(), label: e.label.clone(), label_hidden: false, pinned: e.pinned, running: e.running(), windows: e.windows.len() as i32, focused: e.focused, minimized: e.minimized })
            .collect();
        describe_value(&rows, 8, 0, false)
    }

    fn notes(v: &serde_json::Value) -> (bool, bool) {
        let b = v["buttons"].as_array().unwrap().iter().find(|b| b["app"] == "notes").unwrap();
        (b["minimized"].as_bool().unwrap(), b["focused"].as_bool().unwrap())
    }

    /// The report: minimize Notes, then restore it. After each `done` the dock says what the
    /// compositor said in that `done`, not what it said before.
    #[test]
    fn describe_dock_follows_minimize_then_restore_one_done_at_a_time() {
        let up = |minimized, activated| [Seen { title: "Notes", app: "notes", minimized, activated }, Seen { title: "Files", app: "files", minimized: false, activated: false }];
        assert_eq!(notes(&describe_after_done(&up(false, true))), (false, true), "in front");
        assert_eq!(notes(&describe_after_done(&up(true, false))), (true, false), "minimized: the dock says so at once");
        assert_eq!(notes(&describe_after_done(&up(false, true))), (false, true), "restored: in front again, not minimized");
    }

    #[test]
    fn many_events_before_the_ui_runs_wake_it_once() {
        let c = Coalesce::default();
        assert!(c.want(), "the first event schedules a refresh");
        assert!(!c.want() && !c.want(), "the rest ride on it");
        c.ran();
        assert!(c.want(), "an event after the refresh started needs another");
    }

    /// The stream thread must tell the dock after it has taken in an event, and `describe` must
    /// draw the dock again before it reads it: the two places that closed the gap.
    #[test]
    fn the_stream_and_describe_both_refresh_the_dock() {
        let watch = include_str!("../toplevel_watch.rs");
        let done = &watch[watch.find("handle::Event::Done").unwrap()..];
        let done = &done[..done.find("handle::Event::Closed").unwrap()];
        assert!(done.contains("dock_live::stream_changed()"), "`done` must refresh the dock after the focus log has moved");
        let closed = &watch[watch.find("handle::Event::Closed").unwrap()..];
        assert!(closed.contains("dock_live::stream_changed()"), "a close must too");
        let bar: String = include_str!("dock_bar.rs").split("#[cfg(test)]").next().unwrap().to_string();
        let describe = &bar[bar.find("pub fn for_describe").unwrap()..];
        assert!(describe.contains("dock_live::refresh(ui, false)"), "describe reads the dock after drawing it again");
    }

    /// A dock hover label must not outlive the pointer (#1423): the dock forgets it on every window
    /// event, and on the pointer leaving.
    #[test]
    fn the_dock_forgets_its_hover_on_a_window_event() {
        let dock = include_str!("../../../yantrik-ui-slint/ui/components/grounded_dock.slint");
        let at = dock.find("changed window-event").expect("the dock reacts to window events");
        let body = &dock[at..dock[at..].find("\n    }").map_or(dock.len(), |e| at + e)];
        assert!(body.contains("over-id = \"\"") && body.contains("label-shown = false"), "{body}");
        let app = include_str!("../../../yantrik-ui-slint/ui/app.slint");
        assert!(app.contains("window-event: root.dock-window-event"));
        assert!(include_str!("dock_live.rs").contains("set_dock_window_event"));
    }
}
