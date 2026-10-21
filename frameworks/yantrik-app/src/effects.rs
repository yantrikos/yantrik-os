//! What a command asks the framework to do (design/app-framework-2026-10-21.md, §4).
//!
//! `update` changes the model and *requests* effects; it performs none. The requests are staged
//! here and committed by the runtime only when `update` returns `Ok`, so a refused command runs
//! nothing. In a test, `Effects::recording()` captures them as data.

use std::time::Duration;

use crate::app::{App, Changed, WindowId};
use crate::intents::{Invocation, InvocationId, Origin};
use crate::jobs::{Cancel, Concurrency, Job, JobId, JobKey};
use crate::refused::Refused;
use crate::timers::TimerId;

/// One request, as the runtime applies it after the command commits.
pub enum Request<A: App> {
    /// A command to apply after this one, in order. Not a transaction: it is judged on its own.
    Then(A::Command),
    /// Work off the UI thread. The job returns the command that carries its result; `failed` is
    /// applied instead if it ends without one (a panic, a pool refusal, a key already running).
    Spawn {
        id: JobId,
        key: Option<JobKey>,
        mode: Concurrency,
        failed: Option<A::Command>,
        job: Job<A::Command>,
    },
    Cancel(JobId),
    /// A command every `period`; arming the same id again restarts it.
    Every {
        id: TimerId,
        period: Duration,
        command: A::Command,
    },
    /// A command once, after `delay`; arming the same id again restarts it (a debounce).
    After {
        id: TimerId,
        delay: Duration,
        command: A::Command,
    },
    StopTimer(TimerId),
    /// A failure the app survived, filed as a problem record.
    Report {
        what: &'static str,
        error: String,
    },
    /// The invocation this command runs under has reached its outcome (`Settle::Explicit`).
    Settle(Result<(), Refused>),
    /// A new value for the settings, written through the persistence lane. With
    /// `replace_unusable`, a file in recovery is replaced by it.
    SaveSettings {
        value: A::Settings,
        replace_unusable: bool,
    },
    /// A draft's bytes, written through the lane after `delay` unless a newer save of the same
    /// draft comes first.
    SaveDraft {
        name: String,
        bytes: Vec<u8>,
        delay: Duration,
    },
    /// The draft's file removed.
    DiscardDraft(String),
    /// A document's file watched under `key`, from what is known to be there.
    Watch {
        key: String,
        path: std::path::PathBuf,
        seen: Option<crate::documents::Observed>,
    },
    /// No longer watched.
    Unwatch(String),
    /// One line to the person through the notifications service, from this app.
    Notify(yantrik_notifications::Notification),
    /// The agent rail applying a proposal: one of the app's intents, run as an agent through
    /// the admitted path with the authority already read.
    ApplyAsAgent {
        apply: crate::rail::Apply,
        authority: yantrik_ipc_transport::gate::Authority,
    },
    /// Another window, under this id: made, wired by `App::wire_window`, drawn and shown.
    OpenWindow(WindowId),
    Close(WindowId),
    /// Keep running once this command's closes leave no window, until `fx.exit()` or the
    /// backstop: `crate::cx`'s `linger`.
    Linger(Duration),
    Exit,
    /// A person asked the app to quit: raised as `Event::ExitRequested(ExitReason::Person)`.
    RequestQuit,
}

/// The effects one command requests.
pub struct Effects<A: App> {
    requests: Vec<Request<A>>,
    changed: Changed,
    next_job: u64,
    invocation: Option<Invocation>,
}

impl<A: App> Effects<A> {
    /// For the runtime: job ids continue from `next_job`; `invocation` is what the command runs
    /// under, if it came through the surface. Every part counts as changed until `changed`
    /// narrows it (§3), so an `update` that forgets to say never leaves the window stale.
    pub(crate) fn new(next_job: u64, invocation: Option<Invocation>) -> Effects<A> {
        Effects {
            requests: Vec::new(),
            changed: Changed::all(),
            next_job,
            invocation,
        }
    }

    /// For `App::start`, which reads the model and changes nothing: no redraw unless it says.
    pub(crate) fn starting(next_job: u64) -> Effects<A> {
        Effects {
            requests: Vec::new(),
            changed: Changed::none(),
            next_job,
            invocation: None,
        }
    }

    /// For a test of `update`: nothing is run, and [`Effects::requests`] says what was asked.
    pub fn recording() -> Effects<A> {
        Effects::new(1, None)
    }

    /// The same, as if the command came through the surface from `origin`.
    pub fn recording_from(origin: Origin) -> Effects<A> {
        Effects::new(
            1,
            Some(Invocation {
                id: InvocationId(1),
                origin,
            }),
        )
    }

    /// Where this command came from: a button or a timer here, or the surface.
    pub fn origin(&self) -> Origin {
        self.invocation.map_or(Origin::Local, |i| i.origin)
    }

    /// The invocation this command runs under, if it came through the surface.
    pub fn invocation(&self) -> Option<InvocationId> {
        self.invocation.map(|i| i.id)
    }

    /// Mark a part of the model changed, so `render` redraws it. The first call narrows the
    /// redraw from every part to this one; later calls add to it.
    pub fn changed(&mut self, part: &'static str) {
        match &mut self.changed {
            Changed::All => self.changed = Changed::Parts(vec![part]),
            parts => parts.add(part),
        }
    }

    pub fn then(&mut self, command: impl Into<A::Command>) {
        self.requests.push(Request::Then(command.into()));
    }

    /// Work off the UI thread, unkeyed: it runs beside anything else.
    pub fn spawn(&mut self, job: impl FnOnce(&Cancel) -> A::Command + Send + 'static) -> JobId {
        self.spawn_as(None, Concurrency::Parallel, None, job)
    }

    /// [`Effects::spawn`], with the command applied if the job ends without a result: a panic,
    /// or the pool at its bound. An `update` that set `loading` uses it to clear it.
    pub fn spawn_or(
        &mut self,
        failed: A::Command,
        job: impl FnOnce(&Cancel) -> A::Command + Send + 'static,
    ) -> JobId {
        self.spawn_as(None, Concurrency::Parallel, Some(failed), job)
    }

    /// Work off the UI thread under a key, so two of a kind cannot run at once:
    /// `ReplacePrevious` cancels the earlier one and drops its late result; `RejectIfRunning`
    /// drops this one.
    pub fn spawn_keyed(
        &mut self,
        key: JobKey,
        mode: Concurrency,
        job: impl FnOnce(&Cancel) -> A::Command + Send + 'static,
    ) -> JobId {
        self.spawn_as(Some(key), mode, None, job)
    }

    /// [`Effects::spawn_keyed`], with the command applied if the job ends without a result:
    /// a panic, the pool at its bound, or `RejectIfRunning` finding one of its kind running.
    pub fn spawn_keyed_or(
        &mut self,
        key: JobKey,
        mode: Concurrency,
        failed: A::Command,
        job: impl FnOnce(&Cancel) -> A::Command + Send + 'static,
    ) -> JobId {
        self.spawn_as(Some(key), mode, Some(failed), job)
    }

    fn spawn_as(
        &mut self,
        key: Option<JobKey>,
        mode: Concurrency,
        failed: Option<A::Command>,
        job: impl FnOnce(&Cancel) -> A::Command + Send + 'static,
    ) -> JobId {
        let id = JobId(self.next_job);
        self.next_job += 1;
        self.requests.push(Request::Spawn {
            id,
            key,
            mode,
            failed,
            job: Box::new(job),
        });
        id
    }

    pub fn cancel(&mut self, job: JobId) {
        self.requests.push(Request::Cancel(job));
    }

    pub fn every(&mut self, id: TimerId, period: Duration, command: A::Command) {
        self.requests.push(Request::Every {
            id,
            period,
            command,
        });
    }

    pub fn after(&mut self, id: TimerId, delay: Duration, command: A::Command) {
        self.requests.push(Request::After { id, delay, command });
    }

    pub fn stop_timer(&mut self, id: TimerId) {
        self.requests.push(Request::StopTimer(id));
    }

    pub fn report(&mut self, what: &'static str, error: &dyn std::error::Error) {
        self.requests.push(Request::Report {
            what,
            error: error.to_string(),
        });
    }

    /// The invocation this command runs under has reached its outcome: the Mind is answered
    /// from the committed model, or with the refusal. Nothing happens for a command that did
    /// not come through the surface, or whose intent settles on return.
    pub fn settle(&mut self, result: Result<(), Refused>) {
        self.requests.push(Request::Settle(result));
    }

    /// Save the settings: the value is the app's at once, the write is ordered behind earlier
    /// ones, and `Event::SettingsSaved` or `Event::SettingsNotSaved` says how it went. Over a
    /// file in recovery the value is held in memory and `SettingsNotSaved` says why.
    pub fn save_settings(&mut self, value: A::Settings) {
        self.requests.push(Request::SaveSettings {
            value,
            replace_unusable: false,
        });
    }

    /// The person chose to replace a settings file this app could not read with `value`.
    pub fn replace_settings_file(&mut self, value: A::Settings) {
        self.requests.push(Request::SaveSettings {
            value,
            replace_unusable: true,
        });
    }

    /// Keep a draft: written after [`crate::drafts::DEBOUNCE`], or sooner if the app closes; a
    /// newer save of the same name meanwhile takes its place, so typing is not a write per key.
    /// `Event::DraftSaved` or `Event::DraftNotSaved` says how it went. The app owns the bytes'
    /// schema and reads them back from `Boot::drafts`.
    pub fn save_draft(&mut self, name: &str, bytes: Vec<u8>) {
        self.requests.push(Request::SaveDraft {
            name: name.into(),
            bytes,
            delay: crate::drafts::DEBOUNCE,
        });
    }

    /// A draft written now: a meaningful transition (a document switched, a send pressed).
    pub fn checkpoint_draft(&mut self, name: &str, bytes: Vec<u8>) {
        self.requests.push(Request::SaveDraft {
            name: name.into(),
            bytes,
            delay: Duration::ZERO,
        });
    }

    /// The draft is no longer needed: its file is removed.
    pub fn discard_draft(&mut self, name: &str) {
        self.requests.push(Request::DiscardDraft(name.into()));
    }

    /// Watches a document's file: a change from `seen` arrives as `Event::FileChanged` under
    /// `key`. Watching the same key again replaces the entry. Pass
    /// [`Document::watch_entry`](crate::documents::Document::watch_entry).
    pub fn watch(&mut self, key: &str, path: std::path::PathBuf, seen: Option<crate::documents::Observed>) {
        self.requests.push(Request::Watch { key: key.into(), path, seen });
    }

    pub fn unwatch(&mut self, key: &str) {
        self.requests.push(Request::Unwatch(key.into()));
    }

    /// Tell the person something, in one line, through the notifications service. Never blocks;
    /// a button on it names one of this app's intents, which the shell calls when it is pressed.
    /// `Notification::titled` leaves the app's name for the framework to fill in.
    pub fn notify(&mut self, notification: yantrik_notifications::Notification) {
        self.requests.push(Request::Notify(notification));
    }

    /// The rail's: apply a proposal through the app's own intents, as an agent. What came of it
    /// returns as `RailCommand::Applied`.
    pub(crate) fn apply_as_agent(
        &mut self,
        apply: crate::rail::Apply,
        authority: yantrik_ipc_transport::gate::Authority,
    ) {
        self.requests.push(Request::ApplyAsAgent { apply, authority });
    }

    /// Ask the desktop for something (open a web address or a file, copy text) off the UI
    /// thread; `then` turns what came of it into the command that carries it back.
    pub fn desktop(
        &mut self,
        request: crate::desktop::DesktopRequest,
        then: impl FnOnce(crate::desktop::DesktopOutcome) -> A::Command + Send + 'static,
    ) -> JobId {
        self.spawn(move |_| then(crate::desktop::perform(request)))
    }

    /// Opens another window of the app and returns its id. It is wired by `App::wire_window`,
    /// drawn whole by `render` with this id, and shown; every later render reaches it too. Its
    /// close button raises `Event::CloseRequested` with this id. It may not appear: a window
    /// that cannot be made raises `Event::WindowNotOpened` with this id instead.
    pub fn open_window(&mut self) -> WindowId {
        let id = crate::cx::next_window_id();
        self.requests.push(Request::OpenWindow(id));
        id
    }

    /// Close a window. A `CloseRequested` flow ends here once any saving is done. Closing the
    /// last open window ends the process, judged after all of this command's effects, so a close
    /// and an open in one command replace a window.
    pub fn close(&mut self, window: WindowId) {
        self.requests.push(Request::Close(window));
    }

    /// Keep the process running after this command closes the last window, so work still in
    /// flight (a write to a service, say) can land and its outcome be known, until the app runs
    /// `fx.exit()` or `backstop` passes, when the framework ends it and logs that it did. The
    /// backstop is capped at [`crate::cx::MAX_LINGER`].
    ///
    /// Only in the command that closes the last open window (`fx.close`), as the answer to its
    /// `CloseRequested` or the end of the close flow it started; anywhere else it is ignored,
    /// with a warning, and the usual rule holds: the last window closing ends the process.
    ///
    /// While it lingers there is no window; the dock still shows the app running. A second
    /// launch opens the first window again and raises `Event::Presented`, where the app cancels
    /// the exit it was waiting to make. A logout still ends it, within the signal deadline
    /// (`crate::quit::SIGNAL_DEADLINE`). `Event::Presented` documents what the app sees.
    pub fn linger(&mut self, backstop: Duration) {
        self.requests.push(Request::Linger(backstop));
    }

    /// End the process, in order.
    pub fn exit(&mut self) {
        self.requests.push(Request::Exit);
    }

    /// A person asked the app to quit (Ctrl+Q, a Quit menu): raises
    /// `Event::ExitRequested(ExitReason::Person)` once this command's effects are applied, so the
    /// app's own close flow runs over every window. It ends the process only when that flow
    /// does (`fx.exit()`), or at once for an app that takes the default; the person cancelling
    /// keeps it running. See `crate::quit`.
    pub fn request_quit(&mut self) {
        self.requests.push(Request::RequestQuit);
    }

    /// What was asked, in order. For tests and the runtime.
    pub fn requests(&self) -> &[Request<A>] {
        &self.requests
    }

    pub fn changed_parts(&self) -> &Changed {
        &self.changed
    }

    pub(crate) fn into_parts(self) -> (Vec<Request<A>>, Changed, u64) {
        (self.requests, self.changed, self.next_job)
    }
}

impl<A: App> Request<A> {
    /// The request's name, for a test that asserts on what was asked.
    pub fn name(&self) -> &'static str {
        match self {
            Request::Then(_) => "then",
            Request::Spawn { .. } => "spawn",
            Request::Cancel(_) => "cancel",
            Request::Every { .. } => "every",
            Request::After { .. } => "after",
            Request::StopTimer(_) => "stop_timer",
            Request::Report { .. } => "report",
            Request::Settle(_) => "settle",
            Request::SaveSettings { .. } => "save_settings",
            Request::SaveDraft { .. } => "save_draft",
            Request::DiscardDraft(_) => "discard_draft",
            Request::Watch { .. } => "watch",
            Request::Unwatch(_) => "unwatch",
            Request::Notify(_) => "notify",
            Request::ApplyAsAgent { .. } => "apply_as_agent",
            Request::OpenWindow(_) => "open_window",
            Request::Close(_) => "close",
            Request::Linger(_) => "linger",
            Request::Exit => "exit",
            Request::RequestQuit => "request_quit",
        }
    }
}
