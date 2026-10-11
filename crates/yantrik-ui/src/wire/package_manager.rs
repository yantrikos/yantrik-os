//! Package Manager wire module — list, search, info, install, remove, upgrade.
//!
//! Two sources, one list. Debian packages come through `wire::apt`, and apps from Flathub
//! through `wire::flatpak` (#399), because the apps people ask for first on a new machine —
//! VS Code, Spotify, Discord, Steam — are not in Debian's archive. Every row says which source
//! it is from, and every action goes to that source's own commands: a Debian package through
//! the root helper, a Flatpak per-user with no root at all.
//!
//! It used to shell out to `apk` directly, which is Alpine's package manager. This OS is
//! Debian — `apk` is not installed — so the one screen whose purpose is installing software
//! could not install software, and the failure was invisible because every call site treated
//! "could not run apk" as a non-fatal empty result.
//!
//! All heavy operations run in background threads. UI is updated via Slint Timers polling
//! channels, and each kind of operation has its own timer: they used to share one, so clicking
//! a row while an install ran replaced the install's timer, and its result was never heard —
//! the screen said "Installing…" forever. A Flathub install can take minutes, so that was no
//! longer a rare click.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use slint::{ComponentHandle, ModelRc, Timer, TimerMode, VecModel};

use crate::app_context::AppContext;
use crate::{App, PackageData};

/// Where a row comes from, which decides every command it is given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    Debian,
    Flathub,
}

impl Source {
    /// The word the UI and its callbacks carry.
    fn key(self) -> &'static str {
        match self {
            Source::Debian => "debian",
            Source::Flathub => "flathub",
        }
    }

    fn from_key(key: &str) -> Self {
        if key == "flathub" {
            Source::Flathub
        } else {
            Source::Debian
        }
    }

    /// What a person reads on the row's badge.
    fn label(self) -> &'static str {
        match self {
            Source::Debian => "Debian",
            Source::Flathub => "Flathub",
        }
    }
}

/// One row of the package list, as the screen models it.
#[derive(Clone, Debug, PartialEq)]
struct PkgEntry {
    /// What the commands take: the package name for Debian, the app id for a Flatpak.
    id: String,
    /// What the list shows. The same as `id` for a Debian package; `Visual Studio Code` for
    /// `com.visualstudio.code`.
    name: String,
    version: String,
    description: String,
    installed: bool,
    upgradable: bool,
    size_text: String,
    repo: String,
    source: Source,
}

impl PkgEntry {
    fn from_apt(p: crate::wire::apt::Package, upgradable: bool) -> Self {
        PkgEntry {
            id: p.name.clone(),
            name: p.name,
            version: p.version,
            description: p.description,
            installed: p.installed,
            upgradable,
            size_text: p.size_text,
            repo: p.repo,
            source: Source::Debian,
        }
    }

    fn from_flatpak(a: crate::wire::flatpak::App) -> Self {
        PkgEntry {
            id: a.id,
            name: a.name,
            version: a.version,
            description: a.description,
            installed: a.installed,
            upgradable: a.upgradable,
            size_text: a.size_text,
            // The badge already says Flathub; the branch is the one thing left to say.
            repo: a.branch,
            source: Source::Flathub,
        }
    }

    fn same_package(&self, other: &PkgEntry) -> bool {
        self.source == other.source && self.id == other.id
    }
}

/// Everything the detail pane shows about one package.
#[derive(Clone, Debug, Default)]
struct PkgDetail {
    description: String,
    maintainer: String,
    dependencies: String,
    size: String,
    repo: String,
}

/// What the screen holds between callbacks.
#[derive(Default)]
struct State {
    /// Everything installed, from both sources.
    installed: Vec<PkgEntry>,
    /// What the last search found beyond the installed list.
    found: Vec<PkgEntry>,
    /// Said beside the results when a source could not be searched.
    found_note: String,
    /// The rows on screen, in order, so a click on row N is row N of what the person saw.
    /// Selection used to re-derive the list from the filter alone and ignore the search, so
    /// clicking the first result of a search opened the first package of the whole list.
    shown: Vec<PkgEntry>,
}

/// The filters, as the chips number them.
const FILTER_INSTALLED: i32 = 1;
const FILTER_UPGRADABLE: i32 = 2;
const FILTER_AVAILABLE: i32 = 3;

/// A search asks the sources only after typing pauses this long, and only for this much text:
/// `flatpak search` reads Flathub's whole catalogue each time, which is not a per-keystroke job.
const SEARCH_PAUSE: Duration = Duration::from_millis(450);
const SEARCH_MIN_CHARS: usize = 2;

/// At most this many results from each source. `apt-cache search code` alone is hundreds of
/// libraries, and a person scrolling past them to find an editor has been given nothing.
const RESULTS_PER_SOURCE: usize = 40;

/// The rows for this query and filter: installed packages that match, then what a search found
/// that is not already installed.
fn visible(installed: &[PkgEntry], found: &[PkgEntry], query: &str, filter: i32) -> Vec<PkgEntry> {
    let q = query.trim().to_lowercase();
    let matches = |p: &PkgEntry| {
        q.is_empty()
            || p.name.to_lowercase().contains(&q)
            || p.id.to_lowercase().contains(&q)
            || p.description.to_lowercase().contains(&q)
    };
    let local = installed.iter().filter(|p| matches(p));
    let remote = found
        .iter()
        .filter(|f| !installed.iter().any(|p| p.same_package(f)));
    local
        .chain(remote)
        .filter(|p| match filter {
            FILTER_INSTALLED => p.installed,
            FILTER_UPGRADABLE => p.upgradable,
            FILTER_AVAILABLE => !p.installed,
            _ => true,
        })
        .cloned()
        .collect()
}

/// The timers that poll background work, one per kind so none replaces another's.
#[derive(Clone, Default)]
struct Timers {
    load: Rc<RefCell<Option<Timer>>>,
    detail: Rc<RefCell<Option<Timer>>>,
    action: Rc<RefCell<Option<Timer>>>,
    search: Rc<RefCell<Option<Timer>>>,
    pause: Rc<Timer>,
}

/// Wire package manager callbacks.
pub fn wire(ui: &App, ctx: &AppContext) {
    let state: Rc<RefCell<State>> = Rc::new(RefCell::new(State::default()));
    let timers = Timers::default();
    // Which search is current. A result that arrives for an older one is dropped, so a slow
    // search for "sp" cannot land on top of the answer for "spotify".
    let search_gen: Rc<Cell<u64>> = Rc::new(Cell::new(0));

    // ── Search: the installed list at once, the sources once typing pauses ──
    {
        let ui_weak = ui.as_weak();
        let state = state.clone();
        let timers = timers.clone();
        let search_gen = search_gen.clone();
        ui.on_pkg_search(move |query| {
            let query = query.to_string();
            let generation = search_gen.get() + 1;
            search_gen.set(generation);
            timers.pause.stop();

            let Some(ui) = ui_weak.upgrade() else { return };
            if query.trim().chars().count() < SEARCH_MIN_CHARS {
                let mut s = state.borrow_mut();
                s.found.clear();
                s.found_note.clear();
                drop(s);
                render(&ui, &state);
                return;
            }
            render(&ui, &state);

            let ui_weak = ui_weak.clone();
            let state = state.clone();
            let slot = timers.search.clone();
            let search_gen = search_gen.clone();
            timers.pause.start(TimerMode::SingleShot, SEARCH_PAUSE, move || {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_pkg_is_loading(true);
                }
                let (tx, rx) = mpsc::channel::<(Vec<PkgEntry>, String)>();
                let q = query.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(search_sources(&q));
                });
                let ui_weak = ui_weak.clone();
                let state = state.clone();
                let search_gen = search_gen.clone();
                when_done(&slot, rx, move |(found, note)| {
                    let Some(ui) = ui_weak.upgrade() else { return };
                    ui.set_pkg_is_loading(false);
                    if search_gen.get() != generation {
                        return;
                    }
                    {
                        let mut s = state.borrow_mut();
                        s.found = found;
                        s.found_note = note;
                    }
                    render(&ui, &state);
                });
            });
        });
    }

    // ── Filter changed ──
    {
        let ui_weak = ui.as_weak();
        let state = state.clone();
        ui.on_pkg_filter_changed(move |_| {
            if let Some(ui) = ui_weak.upgrade() {
                render(&ui, &state);
            }
        });
    }

    // ── Refresh — the index, then everything installed, in the background ──
    {
        let ui_weak = ui.as_weak();
        let state = state.clone();
        let timers = timers.clone();
        ui.on_pkg_refresh(move || {
            load(&ui_weak, &state, &timers, true, None);
        });
    }

    // ── Select package ──
    {
        let ui_weak = ui.as_weak();
        let state = state.clone();
        let timers = timers.clone();
        ui.on_pkg_select_package(move |idx| {
            let Some(ui) = ui_weak.upgrade() else { return };
            let Some(pkg) = usize::try_from(idx).ok().and_then(|i| state.borrow().shown.get(i).cloned())
            else {
                return;
            };

            show_detail(&ui, &pkg);

            // A Flatpak's row already holds everything its detail pane shows. A Debian
            // package's description, maintainer and dependencies come from `apt-cache show`.
            if pkg.source == Source::Flathub {
                return;
            }
            let (tx, rx) = mpsc::channel::<PkgDetail>();
            let name = pkg.id.clone();
            std::thread::spawn(move || {
                let _ = tx.send(fetch_pkg_detail(&name, pkg.installed, pkg.upgradable));
            });
            let weak = ui_weak.clone();
            let asked_for = pkg.id.clone();
            when_done(&timers.detail, rx, move |detail| {
                let Some(ui) = weak.upgrade() else { return };
                // The person may have clicked another row while this was fetched.
                if ui.get_pkg_detail_id().as_str() != asked_for {
                    return;
                }
                ui.set_pkg_detail_description(detail.description.into());
                ui.set_pkg_detail_maintainer(detail.maintainer.into());
                ui.set_pkg_detail_dependencies(detail.dependencies.into());
                if !detail.size.is_empty() {
                    ui.set_pkg_detail_size(detail.size.into());
                }
                if !detail.repo.is_empty() {
                    ui.set_pkg_detail_repo(detail.repo.into());
                }
            });
        });
    }

    // ── Install, remove, upgrade one ──
    //
    // The callbacks carry the id; which source it belongs to is the detail pane's, since that
    // is the only place the buttons are.
    for verb in [Verb::Install, Verb::Remove, Verb::Upgrade] {
        let ui_weak = ui.as_weak();
        let state = state.clone();
        let timers = timers.clone();
        let handler = move |id: slint::SharedString| {
            let Some(ui) = ui_weak.upgrade() else { return };
            let id = id.to_string();
            let source = Source::from_key(ui.get_pkg_detail_source().as_str());
            let label = match ui.get_pkg_detail_name().to_string() {
                name if name.is_empty() => id.clone(),
                name => name,
            };
            let job = match job_for(verb, source, &id) {
                Ok(job) => job,
                Err(e) => {
                    ui.set_pkg_error_text(e.into());
                    return;
                }
            };
            run_pkg_action(&ui_weak, &state, &timers, label, source, verb, job);
        };
        match verb {
            Verb::Install => ui.on_pkg_install_package(handler),
            Verb::Remove => ui.on_pkg_remove_package(handler),
            Verb::Upgrade => ui.on_pkg_upgrade_package(handler),
        }
    }

    // ── Upgrade all: Debian, then every Flatpak ──
    {
        let ui_weak = ui.as_weak();
        let state = state.clone();
        let timers = timers.clone();
        ui.on_pkg_upgrade_all(move || {
            let job: Job = Box::new(|| {
                // Both run whatever the other did: a Debian upgrade that failed is no reason
                // to leave the Flatpaks behind, and the person is told about each failure.
                let mut failures = Vec::new();
                if let Err(e) = run_apt(&crate::wire::apt::upgrade_all_command(), "all packages") {
                    failures.push(format!("Debian: {e}"));
                }
                if crate::wire::flatpak::available() {
                    if let Err(e) = run_flatpak(&crate::wire::flatpak::update_all_command()) {
                        failures.push(format!("Flathub: {e}"));
                    }
                }
                if failures.is_empty() {
                    Ok(())
                } else {
                    Err(failures.join(" · "))
                }
            });
            run_pkg_action(
                &ui_weak,
                &state,
                &timers,
                "all packages".to_string(),
                Source::Debian,
                Verb::Upgrade,
                job,
            );
        });
    }

    // ── Apply changes (currently not batched — direct actions) ──
    {
        ui.on_pkg_apply_changes(move || {
            // Actions are applied immediately in this implementation
        });
    }

    // ── Cancel changes ──
    {
        ui.on_pkg_cancel_changes(move || {
            // No-op in direct-action mode
        });
    }

    // ── AI Explain callback (explain selected package) ──
    let bridge = ctx.bridge.clone();
    let ai_state = super::ai_assist::AiAssistState::new();
    let ui_weak = ui.as_weak();
    let ai_st = ai_state.clone();
    ui.on_pkg_ai_explain(move || {
        let Some(ui) = ui_weak.upgrade() else { return };
        let name = ui.get_pkg_detail_name().to_string();
        if name.is_empty() { return; }

        let version = ui.get_pkg_detail_version().to_string();
        let desc = ui.get_pkg_detail_description().to_string();
        let deps = ui.get_pkg_detail_dependencies().to_string();
        let size = ui.get_pkg_detail_size().to_string();
        let source = Source::from_key(ui.get_pkg_detail_source().as_str()).label();

        let info = format!(
            "Source: {}\nVersion: {}\nDescription: {}\nSize: {}\nDependencies: {}",
            source, version, desc, size, deps
        );
        let prompt = super::ai_assist::package_explain_prompt(&name, &info);

        super::ai_assist::ai_request(
            &ui.as_weak(),
            &bridge,
            &ai_st,
            super::ai_assist::AiAssistRequest {
                prompt,
                timeout_secs: 30,
                set_working: Box::new(|ui, v| ui.set_pkg_ai_is_working(v)),
                set_response: Box::new(|ui, s| ui.set_pkg_ai_response(s.into())),
                get_response: Box::new(|ui| ui.get_pkg_ai_response().to_string()),
            },
        );
    });

    // ── AI Intent callback (natural language → package suggestion) ──
    let bridge2 = ctx.bridge.clone();
    let ai_st2 = ai_state.clone();
    let ui_weak = ui.as_weak();
    ui.on_pkg_ai_intent(move |query| {
        let intent = query.to_string();
        if intent.is_empty() { return; }

        let prompt = super::ai_assist::intent_to_package_prompt(&intent);

        super::ai_assist::ai_request(
            &ui_weak,
            &bridge2,
            &ai_st2,
            super::ai_assist::AiAssistRequest {
                prompt,
                timeout_secs: 30,
                set_working: Box::new(|ui, v| ui.set_pkg_ai_is_working(v)),
                set_response: Box::new(|ui, s| ui.set_pkg_ai_response(s.into())),
                get_response: Box::new(|ui| ui.get_pkg_ai_response().to_string()),
            },
        );
    });

    // ── AI Dismiss ──
    let ui_weak = ui.as_weak();
    ui.on_pkg_ai_dismiss(move || {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_pkg_ai_panel_open(false);
        }
    });
}

/// Poll `rx` from the UI thread and hand its one value to `done`.
fn when_done<T: 'static>(
    slot: &Rc<RefCell<Option<Timer>>>,
    rx: mpsc::Receiver<T>,
    done: impl FnOnce(T) + 'static,
) {
    let mut done = Some(done);
    let handle = slot.clone();
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_millis(50), move || match rx.try_recv() {
        Ok(value) => {
            // Out of the slot BEFORE `done` runs, and dropped after it: `done` may start the next
            // piece of work in this same slot (a refresh lists what is installed, then asks the
            // mirrors), and clearing the slot afterwards would stop that new timer unseen.
            let _this = handle.borrow_mut().take();
            if let Some(done) = done.take() {
                done(value);
            }
        }
        // The worker died without answering. Nothing will arrive; stop asking.
        Err(mpsc::TryRecvError::Disconnected) => *handle.borrow_mut() = None,
        Err(mpsc::TryRecvError::Empty) => {}
    });
    *slot.borrow_mut() = Some(timer);
}

/// Put the list on screen for the current query and filter, keeping the detail pane's package
/// selected if it is still in view, and its buttons true to what is now installed.
fn render(ui: &App, state: &Rc<RefCell<State>>) {
    let query = ui.get_pkg_search_query().to_string();
    let filter = ui.get_pkg_active_filter();
    let mut guard = state.borrow_mut();
    let s = &mut *guard;
    s.shown = visible(&s.installed, &s.found, &query, filter);

    let items: Vec<PackageData> = s.shown.iter().map(pkg_to_model).collect();
    ui.set_pkg_packages(ModelRc::new(VecModel::from(items)));

    let detail_id = ui.get_pkg_detail_id().to_string();
    let detail_source = Source::from_key(ui.get_pkg_detail_source().as_str());
    let selected = s
        .shown
        .iter()
        .position(|p| !detail_id.is_empty() && p.id == detail_id && p.source == detail_source);
    ui.set_pkg_selected_index(selected.map(|i| i as i32).unwrap_or(-1));
    if let Some(p) = selected.map(|i| &s.shown[i]) {
        ui.set_pkg_detail_installed(p.installed);
        ui.set_pkg_detail_upgradable(p.upgradable);
    }

    let upgradable = s.installed.iter().filter(|p| p.upgradable).count();
    ui.set_pkg_upgradable_count(upgradable as i32);
    ui.set_pkg_status_text(status_line(s, &query, filter).into());
}

/// The status bar's sentence for what is on screen.
fn status_line(s: &State, query: &str, filter: i32) -> String {
    let count = s.shown.len();
    let mut line = if !query.trim().is_empty() {
        format!("{count} matching '{}'", query.trim())
    } else if filter == FILTER_AVAILABLE {
        "Search to find Flathub apps and Debian packages to install".to_string()
    } else {
        let flatpaks = s.installed.iter().filter(|p| p.source == Source::Flathub).count();
        let upgradable = s.installed.iter().filter(|p| p.upgradable).count();
        format!(
            "{} installed ({} Debian, {} Flathub), {} upgradable",
            s.installed.len(),
            s.installed.len() - flatpaks,
            flatpaks,
            upgradable
        )
    };
    if !s.found_note.is_empty() && !query.trim().is_empty() {
        line.push_str(" · ");
        line.push_str(&s.found_note);
    }
    line
}

/// Ask both sources for `query`: Flathub first, since what people search a store for is
/// usually an app, then Debian's archive. A source that cannot be searched is said in the note,
/// and the other's results still come back.
fn search_sources(query: &str) -> (Vec<PkgEntry>, String) {
    let mut found = Vec::new();
    let mut notes = Vec::new();

    match crate::wire::flatpak::search(query, RESULTS_PER_SOURCE) {
        Ok(apps) => found.extend(apps.into_iter().map(PkgEntry::from_flatpak)),
        Err(e) => notes.push(e),
    }
    // Leading dashes would reach apt-cache as options. Nothing a package is called starts
    // with one.
    let q = query.trim().trim_start_matches('-');
    if !q.is_empty() {
        found.extend(
            crate::wire::apt::search(q)
                .into_iter()
                .take(RESULTS_PER_SOURCE)
                .map(|p| PkgEntry::from_apt(p, false)),
        );
    }
    (found, notes.join(" · "))
}

/// Load everything installed, from both sources, off the UI thread.
///
/// `update_index` refreshes Debian's package index first. The Refresh button asks for that; a
/// reload after an install does not, since what changed is on this machine, not the mirror.
///
/// `then` is said in the status bar once the list is back — an action's "Successfully
/// installed", which the list's own count would otherwise replace before anyone read it.
///
/// A refresh that updates the index lists what is installed FIRST, from this machine alone, and
/// only then asks the mirrors. VM 520 sweep, 4 October: the screen sat blank for as long as
/// "Updating package database…" took, because the index update (network-bound, often the
/// slow part) ran before anything was read, though the installed list was there to show the
/// whole time.
fn load(
    ui_weak: &slint::Weak<App>,
    state: &Rc<RefCell<State>>,
    timers: &Timers,
    update_index: bool,
    then: Option<String>,
) {
    if !update_index {
        return load_once(ui_weak, state, timers, false, then, None);
    }
    let (weak, st, tm) = (ui_weak.clone(), state.clone(), timers.clone());
    let index_next: Box<dyn FnOnce()> = Box::new(move || load_once(&weak, &st, &tm, true, then, None));
    load_once(ui_weak, state, timers, false, None, Some(index_next));
}

/// One pass of `load`: optionally update the index, then list both sources. `after` runs once
/// the list is on screen (or has failed to be read), whichever it was.
fn load_once(
    ui_weak: &slint::Weak<App>,
    state: &Rc<RefCell<State>>,
    timers: &Timers,
    update_index: bool,
    then: Option<String>,
    after: Option<Box<dyn FnOnce()>>,
) {
    if let Some(ui) = ui_weak.upgrade() {
        ui.set_pkg_is_loading(true);
        ui.set_pkg_status_text(
            if update_index { "Updating package database..." } else { "Loading packages..." }.into(),
        );
        ui.set_pkg_error_text("".into());
    }

    let (tx, rx) = mpsc::channel::<Result<(Vec<PkgEntry>, String), String>>();
    std::thread::spawn(move || {
        if update_index {
            // Best-effort: it needs the network, and a machine that is offline should still be
            // able to see and remove what it already has.
            let update = crate::wire::apt::update_command();
            if let Some((bin, args)) = update.split_first() {
                let _ = std::process::Command::new(bin).args(args).output();
            }
        }

        let installed = match crate::wire::apt::list_installed() {
            Ok(list) => list,
            Err(e) => {
                let _ = tx.send(Err(e));
                return;
            }
        };
        // Which of them have something newer waiting. Not fatal if it fails: a machine that
        // has never refreshed its index simply has nothing to report.
        let upgradable = crate::wire::apt::list_upgradable();
        let mut packages: Vec<PkgEntry> = installed
            .into_iter()
            .map(|p| {
                let up = upgradable.iter().any(|u| *u == p.name);
                PkgEntry::from_apt(p, up)
            })
            .collect();

        // Flatpaks next. Their failure is said, not fatal: the Debian half is still true.
        let note = match crate::wire::flatpak::list_installed() {
            Ok(apps) => {
                packages.extend(apps.into_iter().map(PkgEntry::from_flatpak));
                String::new()
            }
            Err(e) => format!("Could not list Flatpak apps: {e}"),
        };

        packages.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        let _ = tx.send(Ok((packages, note)));
    });

    let weak = ui_weak.clone();
    let state = state.clone();
    when_done(&timers.load, rx, move |result| {
        let Some(ui) = weak.upgrade() else { return };
        ui.set_pkg_is_loading(false);
        match result {
            Ok((packages, note)) => {
                state.borrow_mut().installed = packages;
                render(&ui, &state);
                if let Some(then) = then {
                    ui.set_pkg_status_text(then.into());
                }
                if !note.is_empty() {
                    ui.set_pkg_error_text(note.into());
                }
            }
            Err(err) => {
                ui.set_pkg_error_text(err.into());
                ui.set_pkg_status_text("Error loading packages".into());
            }
        }
        if let Some(after) = after {
            after();
        }
    });
}

/// Fill the detail pane from a row.
fn show_detail(ui: &App, pkg: &PkgEntry) {
    ui.set_pkg_detail_id(pkg.id.clone().into());
    ui.set_pkg_detail_source(pkg.source.key().into());
    ui.set_pkg_detail_name(pkg.name.clone().into());
    ui.set_pkg_detail_version(pkg.version.clone().into());
    ui.set_pkg_detail_description(pkg.description.clone().into());
    ui.set_pkg_detail_installed(pkg.installed);
    ui.set_pkg_detail_upgradable(pkg.upgradable);
    ui.set_pkg_detail_size(pkg.size_text.clone().into());
    let repo = match pkg.source {
        Source::Debian => pkg.repo.clone(),
        Source::Flathub if pkg.repo.is_empty() => "Flathub".to_string(),
        Source::Flathub => format!("Flathub ({})", pkg.repo),
    };
    ui.set_pkg_detail_repo(repo.into());
    ui.set_pkg_detail_maintainer("".into());
    ui.set_pkg_detail_dependencies("".into());
}

/// What a button asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Verb {
    Install,
    Remove,
    Upgrade,
}

impl Verb {
    fn doing(self) -> &'static str {
        match self {
            Verb::Install => "Installing",
            Verb::Remove => "Removing",
            Verb::Upgrade => "Upgrading",
        }
    }

    fn done(self) -> &'static str {
        match self {
            Verb::Install => "installed",
            Verb::Remove => "removed",
            Verb::Upgrade => "upgraded",
        }
    }
}

/// The work an action does, run on a worker thread.
type Job = Box<dyn FnOnce() -> Result<(), String> + Send>;

/// The work for one button on one package. A Flatpak id that is not one is refused here, before
/// any thread or command exists.
fn job_for(verb: Verb, source: Source, id: &str) -> Result<Job, String> {
    use crate::wire::{apt, flatpak};
    let id_owned = id.to_string();
    let job: Job = match source {
        Source::Debian => {
            let cmd = match verb {
                Verb::Install => apt::install_command(id),
                Verb::Remove => apt::remove_command(id),
                Verb::Upgrade => apt::upgrade_one_command(id),
            };
            Box::new(move || run_apt(&cmd, &id_owned))
        }
        Source::Flathub => {
            let cmd = match verb {
                Verb::Install => flatpak::install_command(id)?,
                Verb::Remove => flatpak::uninstall_command(id)?,
                Verb::Upgrade => flatpak::update_one_command(id)?,
            };
            Box::new(move || {
                // Flathub is added the first time something is installed from it, per-user;
                // removing or updating needs only what is already there.
                if verb == Verb::Install {
                    flatpak::ensure_flathub()?;
                } else if let Some(why) = flatpak::unavailable_reason() {
                    return Err(why);
                }
                run_flatpak(&cmd)
            })
        }
    };
    Ok(job)
}

/// Run one apt helper command, and turn its failure into something a person can act on.
fn run_apt(cmd: &[String], pkg_name: &str) -> Result<(), String> {
    let (bin, args) = cmd.split_first().ok_or("No command")?;
    let output = std::process::Command::new(bin)
        .args(args)
        .output()
        .map_err(|e| format!("Failed to execute: {e}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    // The helper is the only way this account runs apt without a password (#397), and a
    // machine that has not updated since may not have it, or its sudo rule, yet.
    Err(if stderr.contains("a password is required") {
        "This machine does not yet let the desktop manage packages without a password. \
         Install the latest Yantrik update, which sets that up, then try again."
            .to_string()
    } else if stderr.contains(crate::wire::apt::PKG_HELPER) && stderr.contains("not found") {
        "The package helper is not installed yet. Install the latest Yantrik update, \
         which puts it in place, then try again."
            .to_string()
    } else if stderr.contains("is not a package name from the repositories") {
        format!("`{pkg_name}` is not a package this machine installs by name.")
    } else if stderr.contains("Permission denied") || stderr.contains("not permitted") {
        "Requires root privileges, and this machine did not grant them.".to_string()
    } else {
        format!("{} {}", stdout.trim(), stderr.trim())
    })
}

/// Run one flatpak command. Its own last error line is the useful part: flatpak prints the
/// transaction's progress first and `error: …` at the end.
fn run_flatpak(cmd: &[String]) -> Result<(), String> {
    let (bin, args) = cmd.split_first().ok_or("No command")?;
    let output = std::process::Command::new(bin)
        .args(args)
        .output()
        .map_err(|e| format!("Failed to run flatpak: {e}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    Err(flatpak_failure(&stderr, &stdout))
}

/// The sentence a failed flatpak run is reported with.
fn flatpak_failure(stderr: &str, stdout: &str) -> String {
    let last = |text: &str| {
        text.lines()
            .map(str::trim)
            .rev()
            .find(|l| !l.is_empty())
            .map(str::to_string)
    };
    last(stderr)
        .or_else(|| last(stdout))
        .map(|l| l.strip_prefix("error: ").map(str::to_string).unwrap_or(l))
        .unwrap_or_else(|| "flatpak failed and said nothing".to_string())
}

/// Run a package action in a background thread, one at a time.
fn run_pkg_action(
    ui_weak: &slint::Weak<App>,
    state: &Rc<RefCell<State>>,
    timers: &Timers,
    label: String,
    source: Source,
    verb: Verb,
    job: Job,
) {
    let Some(ui) = ui_weak.upgrade() else { return };
    // One change at a time: apt and flatpak each hold a lock of their own, and the second
    // would fail on it with an error that says nothing about why.
    if ui.get_pkg_is_applying() {
        ui.set_pkg_error_text("Another change is still running. Try again when it finishes.".into());
        return;
    }
    ui.set_pkg_is_applying(true);
    let from = if source == Source::Flathub && verb == Verb::Install {
        // The first app from Flathub also brings the runtime it is built on, which can be
        // larger than the app. Said, so a long wait reads as work rather than a hang.
        " from Flathub (the first app can take several minutes)"
    } else {
        ""
    };
    ui.set_pkg_status_text(format!("{} {}{}...", verb.doing(), label, from).into());
    ui.set_pkg_error_text("".into());

    let (tx, rx) = mpsc::channel::<Result<(), String>>();
    std::thread::spawn(move || {
        let _ = tx.send(job());
    });

    let weak = ui_weak.clone();
    let state = state.clone();
    let reload_timers = timers.clone();
    when_done(&timers.action, rx, move |result| {
        let Some(ui) = weak.upgrade() else { return };
        ui.set_pkg_is_applying(false);
        match result {
            Ok(()) => {
                // What is installed changed on this machine, not on a mirror: reload the list
                // without refreshing the index again.
                let said = format!("Successfully {} {}", verb.done(), label);
                ui.set_pkg_status_text(said.clone().into());
                load(&weak, &state, &reload_timers, false, Some(said));
            }
            Err(err) => {
                ui.set_pkg_error_text(err.into());
                ui.set_pkg_status_text("Operation failed".into());
            }
        }
    });
}

/// Detail for one package, from `apt-cache show`.
///
/// The apk version of this walked a bespoke sectioned format looking for lines ending in
/// "description:" and "depends:". Debian's is RFC822, which `wire::apt::parse_detail` handles
/// and has tests for — including the lone `.` that means a blank line inside a description,
/// and the fact that `apt-cache show` prints every available version and only the first is the
/// one being described.
fn fetch_pkg_detail(name: &str, installed: bool, upgradable: bool) -> PkgDetail {
    let d = crate::wire::apt::detail(name, installed, upgradable);
    PkgDetail {
        description: d.description,
        maintainer: d.maintainer,
        dependencies: d.dependencies,
        size: d.size,
        repo: d.repo,
    }
}

/// Convert a single PkgEntry to a Slint PackageData.
fn pkg_to_model(p: &PkgEntry) -> PackageData {
    PackageData {
        id: p.id.clone().into(),
        name: p.name.clone().into(),
        version: p.version.clone().into(),
        description: p.description.clone().into(),
        installed: p.installed,
        upgradable: p.upgradable,
        size_text: p.size_text.clone().into(),
        repo: p.repo.clone().into(),
        source: p.source.key().into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deb(name: &str, installed: bool, upgradable: bool) -> PkgEntry {
        PkgEntry {
            id: name.into(),
            name: name.into(),
            version: "1".into(),
            description: format!("{name} from Debian"),
            installed,
            upgradable,
            size_text: String::new(),
            repo: "utils".into(),
            source: Source::Debian,
        }
    }

    fn app(id: &str, name: &str, installed: bool) -> PkgEntry {
        PkgEntry {
            id: id.into(),
            name: name.into(),
            version: "1".into(),
            description: String::new(),
            installed,
            upgradable: false,
            size_text: String::new(),
            repo: "stable".into(),
            source: Source::Flathub,
        }
    }

    fn ids(rows: &[PkgEntry]) -> Vec<&str> {
        rows.iter().map(|p| p.id.as_str()).collect()
    }

    #[test]
    fn a_search_shows_matching_installed_rows_then_what_it_found() {
        let installed = vec![deb("vim", true, false), app("com.visualstudio.code", "Visual Studio Code", true)];
        let found = vec![app("com.vscodium.codium", "VSCodium", false), deb("code-aster", false, false)];
        // "code" matches the installed Flatpak by its id and name, not vim.
        assert_eq!(
            ids(&visible(&installed, &found, "code", 0)),
            ["com.visualstudio.code", "com.vscodium.codium", "code-aster"]
        );
    }

    #[test]
    fn a_found_package_that_is_installed_is_shown_once_as_installed() {
        let installed = vec![app("com.spotify.Client", "Spotify", true)];
        let found = vec![app("com.spotify.Client", "Spotify", false)];
        let rows = visible(&installed, &found, "spotify", 0);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].installed);
    }

    #[test]
    fn the_same_name_in_two_sources_is_two_packages() {
        // A Debian package and a Flatpak are different things to install even when named alike.
        let installed = vec![deb("steam", true, false)];
        let found = vec![app("steam", "Steam", false)];
        assert_eq!(visible(&installed, &found, "steam", 0).len(), 2);
    }

    #[test]
    fn the_filters_split_installed_upgradable_and_available() {
        let installed = vec![deb("vim", true, true), deb("nano", true, false)];
        let found = vec![app("com.spotify.Client", "Spotify", false)];
        assert_eq!(ids(&visible(&installed, &found, "", FILTER_INSTALLED)), ["vim", "nano"]);
        assert_eq!(ids(&visible(&installed, &found, "", FILTER_UPGRADABLE)), ["vim"]);
        assert_eq!(ids(&visible(&installed, &found, "", FILTER_AVAILABLE)), ["com.spotify.Client"]);
        assert_eq!(visible(&installed, &found, "", 0).len(), 3);
    }

    #[test]
    fn a_flatpak_id_that_is_not_one_never_becomes_a_job() {
        assert!(job_for(Verb::Install, Source::Flathub, "--system").is_err());
        assert!(job_for(Verb::Remove, Source::Flathub, "spotify").is_err());
        assert!(job_for(Verb::Install, Source::Flathub, "com.spotify.Client").is_ok());
    }

    #[test]
    fn a_flatpak_failure_is_reported_by_its_last_error_line() {
        let stderr = "Looking for matches…\nerror: Unable to load summary from remote flathub: Could not resolve hostname\n";
        assert_eq!(
            flatpak_failure(stderr, ""),
            "Unable to load summary from remote flathub: Could not resolve hostname"
        );
        assert_eq!(flatpak_failure("", ""), "flatpak failed and said nothing");
    }

    #[test]
    fn sources_travel_through_the_ui_as_words() {
        for source in [Source::Debian, Source::Flathub] {
            assert_eq!(Source::from_key(source.key()), source);
        }
        assert_eq!(Source::from_key(""), Source::Debian, "a row with no source is the old kind");
    }
}
