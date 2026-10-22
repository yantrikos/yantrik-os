//! The runner: the lifecycle every app gets, in a fixed order
//! (design/app-framework-2026-10-21.md, §2.2). An app's `main` is
//! `yantrik_app::run::<MyApp>()` and nothing else.

use std::path::Path;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use crate::app::{App, Boot, Event, Startup, WindowId};
use crate::cx::{Core, Cx, Inbound, Parts};
use crate::instance::Held;
use crate::intents::{self, Intents};
use crate::jobs::Threads;
use crate::paths::Paths;
use crate::persist::Threaded;
use crate::present::Presented;
use crate::timers::SlintTimers;
use crate::watch::Watch;
use crate::window::Window;
use crate::{boot, instance, launch, problems, theme};

/// How long shutdown waits for the job pool before exiting without it.
pub const GRACE: Duration = Duration::from_secs(2);
/// The job pool: threads running at once, and jobs that may wait for one.
const JOB_THREADS: usize = 4;
const JOBS_WAITING: usize = 64;

thread_local! {
    /// How the wake reaches the core: a closure installed on the UI thread, so the wake itself
    /// captures nothing and is `Send`.
    static DRAIN: std::cell::RefCell<Option<Box<dyn Fn()>>> = const { std::cell::RefCell::new(None) };
}

pub(crate) fn drain_on_ui_thread() {
    DRAIN.with(|cell| {
        if let Some(drain) = cell.borrow().as_ref() {
            drain();
        }
    });
}

/// Runs the app to its end. The exit code says how it ended: 0 for a close, a display that went
/// away or a launch handed to a running copy, 1 for a failure, with a problem record.
pub fn run<A: App>() -> ExitCode {
    boot::init_tracing_for(A::ID, Some(boot::crate_of::<A>()));
    boot::choose_backend();
    let paths = Paths::for_app(A::ID);
    // Every record from here on, a panic's included, goes where this run's other files go.
    problems::set_dir(paths.problems_dir());
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    // The launch is parsed before the claim: a command line this app does not take is said and
    // refused here, whichever copy would have run it. Opening the settings and drafts writes
    // nothing a copy that hands over would regret: the one write, settings brought across from
    // the old shared place (`settings::former`), is made only while the app's own file is
    // missing, from the same old file, so two copies make the same.
    let (mut boot, store) = match Boot::<A::Settings>::open(
        paths.clone(),
        args,
        yantrik_mind::live(),
        crate::services::Services::live(),
    ) {
        Ok(opened) => opened,
        Err(e) => return startup_failed(A::ID, "launch", &e.sentence),
    };
    // The intents are declared, not read from the model, so a deep link's intent is admitted
    // before the claim: a link this app cannot take is refused here, bothering no running copy.
    let mut table = Intents::new();
    A::intents(&mut table);
    if let Some(intent) = boot.launch.intent.take() {
        match intents::admit(&table.specs(), intent, false) {
            Ok(admitted) => boot.launch.intent = Some(admitted),
            Err(why) => return startup_failed(A::ID, "launch", &why),
        }
    }

    // One instance per app and profile: a second launch hands what it was asked to open to the
    // running copy, which shows its window, and exits.
    let instance = paths.instance_name();
    // What the running copy did not answer, kept for the end: a copy that is alive and silent
    // twice has lost the launch, and the person hears so rather than nothing.
    let unanswered = std::cell::RefCell::new(None::<String>);
    let claimed = instance::claim_or_hand_over(&instance, |_| {
        match launch::hand_over(&instance, &boot.launch) {
            Presented::Unanswered(why) => {
                tracing::warn!(app = A::ID, "the running copy did not answer the hand-over: {why}");
                *unanswered.borrow_mut() = Presented::Unanswered(why).sentence(A::ID);
                Held::OwnerGone
            }
            answered => {
                match answered.sentence(A::ID) {
                    None => tracing::info!(app = A::ID, "Already running; handed the launch to it"),
                    Some(said) => {
                        tracing::warn!(app = A::ID, answer = ?answered, "{said}");
                        eprintln!("{said}");
                    }
                }
                *unanswered.borrow_mut() = None;
                Held::HandedOver
            }
        }
    });
    let Some(_instance) = claimed else {
        return match unanswered.into_inner() {
            Some(said) => startup_failed(A::ID, "launch", &said),
            None => ExitCode::SUCCESS,
        };
    };

    if let crate::settings::Health::Recovery(e) = &boot.settings_health {
        tracing::warn!(app = A::ID, "{e}; running on the defaults");
        problems::record_failure(A::ID, "settings", &e.to_string());
    }
    let model = match A::model(&boot) {
        Ok(model) => model,
        Err(declined) => return model_declined(A::ID, &declined, &problems::dir()),
    };
    let ui = match A::Ui::make() {
        Ok(ui) => ui,
        Err(e) => return startup_failed(A::ID, "window", &e.to_string()),
    };
    let theme_now = theme::load();
    ui.apply_theme(&theme_now);
    ui.connect_chrome();

    let wake: crate::queue::Wake = Arc::new(|| {
        let _ = slint::invoke_from_event_loop(drain_on_ui_thread);
    });
    let core = Core::<A>::new(
        model,
        ui,
        Parts {
            executor: Box::new(Threads::new(A::ID, JOB_THREADS, JOBS_WAITING)),
            timers: Box::new(|sink| Box::new(SlintTimers::new(sink))),
            lane: Box::new(|report| Box::new(Threaded::new(A::ID, report))),
            settings: store,
            paths: boot.paths.clone(),
            wake,
        },
    );
    install(&core);
    core.set_theme(theme_now);
    // A file of an older schema is rewritten at this one, once.
    core.flush_settings();
    // The settings file (an outside edit), the shell's theme and the documents' files, watched
    // until the end.
    let _watch = Watch::start(
        A::ID,
        boot.paths.config_file().to_path_buf(),
        theme_now,
        core.watched(),
        core.notice(),
    );

    let cx = Cx::new(core.clone());
    A::wire(&cx);
    core.render_all();
    core.start();
    // The launch is applied once, here (§2.2 step 11): the files to open, or the deep link.
    core.event(Event::Launch(boot.launch.clone()));

    // The surface: the app's intents, `present` for the desktop's own handover, and `launch`
    // for a second copy's. Served only now that commands can be applied, on the instance's own
    // socket (`app-<id>[-<profile>].sock`).
    let registry = intents::registry(&core, table);
    let registry = crate::control::App::from_registry(registry).serve_as(&instance);
    core.attach_registry(registry);

    // A second launch's `present` goes through the core, which knows which windows are open.
    let presenting = Rc::downgrade(&core);
    crate::present::install(move || presenting.upgrade().ok_or("the app is closing")?.present());
    // A logout or a shutdown (SIGTERM, SIGHUP) asks the app, so it can save, rather than
    // killing it (`quit`). Without the handler the signals keep their default.
    if let Err(e) = crate::quit::listen::<A>(core.post(), crate::quit::SIGNAL_DEADLINE) {
        tracing::warn!(app = A::ID, "SIGTERM and SIGHUP will end the app unasked: {e}");
    }
    // An exit asked for in `start` or for the launch ends here, with no loop: Slint drops a quit
    // posted before its loop runs. The first window is shown only if it is still open.
    let first_open = core.windows_open().contains(&WindowId::PRIMARY);
    let ending = match crate::event_loop::before_loop(core.exit_reason().is_some(), first_open) {
        crate::event_loop::BeforeLoop::Skip => crate::event_loop::Ending::Closed,
        crate::event_loop::BeforeLoop::Run { show_first } => {
            crate::event_loop::run_loop(core.ui(), A::ID, show_first)
        }
    };
    // The compositor went (a logout, or a shutdown that signals the whole session at once): no
    // one is there, and the app is owed its logout before the end, as it is for a signal.
    if ending == crate::event_loop::Ending::DisplayGone {
        core.display_gone();
    }
    core.shutdown(GRACE);
    DRAIN.with(|cell| *cell.borrow_mut() = None);
    // A close, an `fx.exit()` and a logout are all 0; only a loop that failed with the display
    // still there is 1, and that one has its problem record.
    match ending {
        crate::event_loop::Ending::Failure if core.exit_reason().is_none() => ExitCode::from(1),
        _ => ExitCode::SUCCESS,
    }
}

/// The drain, and the first window's close handler and activation, on the UI thread.
fn install<A: App>(core: &Rc<Core<A>>) {
    let for_drain = core.clone();
    DRAIN.with(|cell| *cell.borrow_mut() = Some(Box::new(move || for_drain.drain())));
    core.hear_activation(WindowId::PRIMARY, core.ui());
    let post = core.post();
    use slint::ComponentHandle as _;
    core.ui().window().on_close_requested(move || {
        // The app decides (`App::event`): a close now, or a flow that ends in `fx.close`.
        let _ = post.post(Inbound::Event(Event::CloseRequested(WindowId::PRIMARY)));
        drain_on_ui_thread();
        slint::CloseRequestResponse::KeepWindowShown
    });
}

fn startup_failed(id: &str, what: &str, sentence: &str) -> ExitCode {
    failed_into(&problems::dir(), id, what, sentence)
}

/// Ends a start the app's model declined, filing into `records` when it is a failure. A copy the
/// app found another copy for (`Startup::handed_over`) did what a second launch is for, exactly
/// as the framework's own hand-over above: logged at info, exit 0, nothing filed.
fn model_declined(id: &str, declined: &Startup, records: &Path) -> ExitCode {
    if declined.is_handed_over() {
        tracing::info!(app = id, "{}", declined.sentence);
        return ExitCode::SUCCESS;
    }
    failed_into(records, id, "model", &declined.sentence)
}

fn failed_into(records: &Path, id: &str, what: &str, sentence: &str) -> ExitCode {
    eprintln!("{id}: {sentence}");
    tracing::error!(app = id, what, "{sentence}");
    problems::record_failure_in(records, id, what, sentence);
    ExitCode::from(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn records(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("yantrik-runner-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d.join("problems")
    }

    #[test]
    fn a_start_another_copy_took_exits_0_and_files_nothing() {
        let dir = records("handed-over");
        let declined = Startup::handed_over("Already running on this library.");
        assert_eq!(model_declined("runner-test", &declined, &dir), ExitCode::SUCCESS);
        assert!(problems::list_in(&dir).is_empty(), "a hand-over was filed as a problem");
    }

    #[test]
    fn a_model_that_cannot_start_exits_1_and_is_filed() {
        let dir = records("refused");
        let declined = Startup::new("The library is a newer format.");
        assert_eq!(model_declined("runner-test", &declined, &dir), ExitCode::from(1));
        let filed = problems::list_in(&dir);
        assert_eq!(filed.len(), 1, "a failed start left no record");
        assert_eq!(filed[0].1.program, "runner-test");
        assert_eq!(filed[0].1.message, "model: The library is a newer format.");
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }
}
