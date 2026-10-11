//! The Minds panel's wiring: a worker that reads the accounts, and the callbacks that act on them.
//!
//! Everything that reads a file or starts a process — the vendors' logs, `PATH`, a terminal,
//! `systemctl` — runs on one worker thread, so the panel never stalls the shell. The worker keeps
//! the log counters between reads, which is what lets each log be read from where it stopped.
//! It reads while the panel is open (on opening, then every few seconds, so a sign-in finished in
//! the terminal shows up without a click) and not otherwise.
//!
//! What the panel decides is in `crate::accounts`; this file only moves facts and rows between
//! the worker and Slint.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};

use crate::accounts::{self, act, Counters, Facts, Group};
use crate::app_context::AppContext;
use crate::{AccountMeter, AccountRow, AddChoice, App, ProviderGroup};

/// A refresh is waiting on the worker. At most one is: the ticks never queue behind a slow read,
/// and a press never waits behind a pile of them.
static REFRESH_WAITING: AtomicBool = AtomicBool::new(false);

fn refresh(tx: &mpsc::Sender<Job>) {
    if !REFRESH_WAITING.swap(true, Ordering::SeqCst) {
        let _ = tx.send(Job::Refresh);
    }
}

/// How often the open panel is read again. Each read walks the vendors' log directories; a
/// sign-in finished in the terminal shows up within this.
const TICK: Duration = Duration::from_secs(10);

enum Job {
    Refresh,
    Use(String),
    SignIn(String),
    Add(String),
}

/// What the worker hands back beside the rows.
enum After {
    Nothing,
    /// Keys live in Settings → AI.
    Settings,
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| PathBuf::from("/"))
}

fn today_bounds() -> (i64, i64) {
    let now = chrono::Local::now();
    let midnight = now
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .and_then(|t| t.and_local_timezone(chrono::Local).earliest())
        .map(|t| t.timestamp())
        .unwrap_or_else(|| now.timestamp() - 86_400);
    (now.timestamp(), midnight)
}

pub fn wire(ui: &App, ctx: &AppContext) {
    let (tx, rx) = mpsc::channel::<Job>();
    let weak = ui.as_weak();
    let companion_url = ctx.llm_base_url.clone();

    let spawned = std::thread::Builder::new().name("minds-panel".into()).spawn(move || {
        let home = home();
        // What the person chose last time is true again from the start: new programs and the
        // user services get the account each vendor answers with.
        act::apply(&home, &accounts::store::Store::load(&accounts::store::path_in(&home)));
        let mut counters = Counters::default();
        let mut facts = Facts::default();
        while let Ok(job) = rx.recv() {
            let (status, after) = match job {
                Job::Refresh => {
                    REFRESH_WAITING.store(false, Ordering::SeqCst);
                    (None, After::Nothing)
                }
                Job::Use(id) => match act::use_account(&home, &id) {
                    Ok(()) => (Some(used_line(&id)), After::Nothing),
                    Err(e) => (Some(e), After::Nothing),
                },
                Job::SignIn(id) => {
                    let missing = accounts::parse_id(&id).is_some_and(|(v, _)| {
                        matches!(v.sign_in, accounts::vendors::SignIn::Program { .. }) && !facts.installed.contains(&v.id)
                    });
                    let done = if missing {
                        accounts::parse_id(&id).map(|(v, _)| act::install(v)).unwrap_or(Err("not an account id".into()))
                    } else {
                        act::sign_in(&home, &id)
                    };
                    opened(done, missing)
                }
                Job::Add(vendor) => {
                    let installing = !facts.installed.contains(&vendor.as_str())
                        && accounts::vendors::by_id(&vendor).is_some_and(|v| !v.binary.is_empty());
                    opened(act::add(&home, &vendor, &facts), installing)
                }
            };
            let (now, midnight) = today_bounds();
            facts = accounts::observe(&home, &mut counters, companion_url.clone(), now, midnight);
            let groups = accounts::rows(&facts);
            let choices = accounts::choices(&facts);
            let keys = accounts::keys_line(&facts);
            let weak = weak.clone();
            let _ = slint::invoke_from_event_loop(move || {
                let Some(ui) = weak.upgrade() else { return };
                show(&ui, &groups, &choices, &keys, status);
                if let After::Settings = after {
                    ui.set_minds_panel_open(false);
                    ui.set_settings_category(1);
                    ui.set_current_screen(7);
                    ui.invoke_navigate(7);
                }
            });
        }
    });
    if let Err(e) = spawned {
        tracing::error!(error = %e, "the Minds panel's worker could not start; the panel will stay empty");
        return;
    }

    let send = move |tx: &mpsc::Sender<Job>, job: Job| {
        let _ = tx.send(job);
    };

    {
        let tx = tx.clone();
        ui.on_minds_panel_opened(move || refresh(&tx));
    }
    {
        let tx = tx.clone();
        ui.on_minds_use_account(move |id| send(&tx, Job::Use(id.to_string())));
    }
    {
        let tx = tx.clone();
        ui.on_minds_sign_in(move |id| send(&tx, Job::SignIn(id.to_string())));
    }
    {
        let tx = tx.clone();
        ui.on_minds_add(move |vendor| send(&tx, Job::Add(vendor.to_string())));
    }
    {
        let weak = ui.as_weak();
        ui.on_minds_make(move |kind| {
            let Some(ui) = weak.upgrade() else { return };
            make(&ui, kind.as_str());
        });
    }

    // While the panel or Settings → Accounts is open, read it again every few seconds: a sign-in
    // finished in the terminal, or a turn that moved a meter, shows up on its own.
    let timer = Box::leak(Box::new(Timer::default()));
    let weak = ui.as_weak();
    timer.start(TimerMode::Repeated, TICK, move || {
        if weak.upgrade().is_some_and(|ui| ui.get_minds_panel_open() || accounts_page_open(&ui)) {
            refresh(&tx);
        }
    });
}

/// Settings → Accounts (screen 7, category 4), which draws the same list as the panel.
fn accounts_page_open(ui: &App) -> bool {
    ui.get_current_screen() == 7 && ui.get_settings_category() == 4
}

fn opened(done: Result<act::Opened, String>, installing: bool) -> (Option<String>, After) {
    match done {
        Ok(act::Opened::Terminal) if installing => {
            (Some("Installing in a terminal — sign in once it is done".into()), After::Nothing)
        }
        Ok(act::Opened::Terminal) => (Some("Sign in in the terminal that opened; this panel updates by itself".into()), After::Nothing),
        Ok(act::Opened::Settings) => (None, After::Settings),
        Err(e) => (Some(e), After::Nothing),
    }
}

fn used_line(id: &str) -> String {
    match accounts::parse_id(id) {
        Some((v, l)) => format!("{} {} answers for programs started from now on", v.name, accounts::label_name(l)),
        None => String::new(),
    }
}

/// The starter sentence each "make something" tile leaves in the Lens, for the person to finish.
pub fn starter(kind: &str) -> Option<&'static str> {
    match kind {
        "app" => Some("Make me an app that "),
        "theme" => Some("Make me a desktop theme that "),
        "recipe" => Some("Make me a recipe that "),
        _ => None,
    }
}

/// Open the Lens with the tile's starter in the field, the way `open_lens` does: the text is left
/// for the person to finish and send, never sent for them.
fn make(ui: &App, kind: &str) {
    let Some(text) = starter(kind) else { return };
    if ui.get_current_screen() != 1 {
        ui.set_current_screen(1);
        ui.invoke_navigate(1);
    }
    ui.set_lens_input_text(text.into());
    ui.set_lens_open(true);
    ui.invoke_open_lens();
}

/// The groups as Slint draws them.
fn slint_groups(groups: &[Group]) -> Vec<ProviderGroup> {
    groups
        .iter()
        .map(|g| ProviderGroup {
            id: g.vendor.id.into(),
            name: g.vendor.name.into(),
            accounts: ModelRc::new(VecModel::from(
                g.rows
                    .iter()
                    .map(|r| AccountRow {
                        id: r.id.as_str().into(),
                        label: r.label.as_str().into(),
                        plan: r.plan.as_str().into(),
                        state: r.state.word().into(),
                        program: g.vendor.program.into(),
                        used_by: r.used_by.as_str().into(),
                        note: r.note.as_str().into(),
                        meters: ModelRc::new(VecModel::from(
                            r.meters
                                .iter()
                                .map(|m| AccountMeter {
                                    name: m.name.as_str().into(),
                                    used: m.used.unwrap_or(0.0),
                                    known: m.used.is_some(),
                                    value: m.value.as_str().into(),
                                })
                                .collect::<Vec<_>>(),
                        )),
                    })
                    .collect::<Vec<_>>(),
            )),
        })
        .collect()
}

/// How many vendors the Minds panel draws before "N more on the Accounts page". The panel is a
/// glance under the status bar and has to stay on the screen; the page lists every one.
const PANEL_VENDORS: usize = 3;

fn show(ui: &App, groups: &[Group], choices: &[accounts::Choice], keys: &str, status: Option<String>) {
    let rows = slint_groups(groups);
    ui.set_minds_today(accounts::today_line(groups).into());
    let (panel, more) = panel_view(groups);
    ui.set_minds_panel_groups(ModelRc::new(VecModel::from(slint_groups(&panel))));
    ui.set_minds_panel_more(more as i32);
    ui.set_minds_groups(ModelRc::new(VecModel::from(rows)));
    let model = |keep: &dyn Fn(&accounts::Choice) -> bool| {
        ModelRc::new(VecModel::from(
            choices
                .iter()
                .filter(|c| keep(c))
                .map(|c| AddChoice { vendor: c.vendor.id.into(), name: c.vendor.name.into(), what: SharedString::from(c.what) })
                .collect::<Vec<_>>(),
        ))
    };
    ui.set_minds_choices(model(&|c| !c.by_key()));
    ui.set_minds_panel_choices(model(&|c| c.in_panel()));
    ui.set_minds_key_choices(model(&|c| c.by_key()));
    ui.set_minds_keys_line(keys.into());
    if let Some(s) = status {
        ui.set_minds_status(s.into());
    }
}

/// The panel's view: the first vendors, each account on one line, and only the account that
/// answers with a meter, its tightest one (the window nearest full, else today's count). The page
/// keeps every meter and note. Returns the vendors left for the page.
fn panel_view(all: &[Group]) -> (Vec<Group>, usize) {
    let shown = all.len().min(PANEL_VENDORS);
    let groups = all[..shown]
        .iter()
        .map(|g| Group {
            vendor: g.vendor,
            rows: g
                .rows
                .iter()
                .map(|r| {
                    let mut r = r.clone();
                    if r.state == accounts::State::Active {
                        let tightest = r
                            .meters
                            .iter()
                            .filter(|m| m.used.is_some())
                            .max_by(|a, b| a.used.partial_cmp(&b.used).unwrap_or(std::cmp::Ordering::Equal))
                            .or_else(|| r.meters.last())
                            .cloned();
                        r.meters = tightest.into_iter().collect();
                    } else {
                        r.meters.clear();
                        r.note.clear();
                    }
                    r
                })
                .collect(),
        })
        .collect();
    (groups, all.len() - shown)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(state: accounts::State, meters: Vec<accounts::Meter>) -> accounts::Row {
        accounts::Row { id: "claude:primary".into(), label: "Main".into(), plan: String::new(), state, note: "a note".into(), meters, tokens_today: 0, used_by: String::new() }
    }

    #[test]
    fn the_panel_shows_three_vendors_one_line_each_and_the_tightest_meter() {
        use accounts::{Meter, State};
        let claude = accounts::vendors::by_id("claude").unwrap();
        let active = row(State::Active, vec![
            Meter::window("Session".into(), 300, 0.2, "2h".into()),
            Meter::window("Weekly".into(), 10080, 0.8, "4d".into()),
            Meter::count("Today", "1.2M tokens".into()),
        ]);
        let ready = row(State::Ready, vec![Meter::count("Today", "84K tokens".into())]);
        let g = Group { vendor: claude, rows: vec![active, ready] };
        let (shown, more) = panel_view(&vec![g.clone(); 5]);
        assert_eq!((shown.len(), more), (3, 2));
        let rows = &shown[0].rows;
        assert_eq!(rows[0].meters.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), ["Weekly"], "the window nearest full");
        assert!(rows[1].meters.is_empty() && rows[1].note.is_empty(), "an account that does not answer is one line");
        let counts_only = Group { vendor: claude, rows: vec![row(State::Active, vec![Meter::count("Today", "9 tokens".into())])] };
        assert_eq!(panel_view(&[counts_only]).0[0].rows[0].meters[0].name, "Today", "no window: today's count");
        assert_eq!(panel_view(&g.rows.iter().map(|_| g.clone()).collect::<Vec<_>>()).1, 0);
    }

    #[test]
    fn every_tile_has_a_starter_and_nothing_else_does() {
        for kind in ["app", "theme", "recipe"] {
            assert!(starter(kind).is_some_and(|s| s.ends_with(' ')), "{kind}: left open for the person to finish");
        }
        assert_eq!(starter("rm -rf"), None);
    }
}
