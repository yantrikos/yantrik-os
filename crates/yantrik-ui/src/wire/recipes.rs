//! The Recipes screen (35): every recipe the companion holds, each drawn as its stages left to
//! right, and the selected one opened below its row with every step.
//!
//! It reads `crate::recipes` — the worker's published copy — and nothing else, so drawing never
//! waits on the companion. A timer looks once a second whether that copy has moved and redraws if
//! it has; while the screen is up it also asks the worker to read the store again every few
//! seconds (once at a time), which catches a change made by a path that does not publish. Presses
//! go to the worker through `CompanionHandle::recipe` and come back as a notice.
//!
//! The rows are updated in place when the same recipes are shown, so an answer half-typed into a
//! question's box survives another recipe moving on.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::{ComponentHandle, Model, ModelRc, SharedString, Timer, TimerMode, VecModel};
use yantrik_companion::recipe::Leave;
use yantrik_companion::recipe_view::{self, RecipeOp, RecipeView, StepView};

use crate::agents::model::{Card, CallState};
use crate::agents::{AgentId, Store};
use crate::app_context::AppContext;
use crate::bridge::CompanionHandle;
use crate::{
    AgentsState, App, RecipeAnswerBlock, RecipeLinkData, RecipeRoleData, RecipeRowData, RecipeSeatData, RecipeStageData,
    RecipeStepData, RecipeTabData, RecipesState,
};

/// The screen id `app.slint` draws the Recipes screen at.
pub const SCREEN: i32 = 35;

/// How often the published copy is looked at. Looking is reading a counter; nothing is polled
/// faster than this.
const TICK: Duration = Duration::from_secs(1);

/// While the screen is up, how often the worker is asked to read the store again.
const REFRESH_EVERY: Duration = Duration::from_secs(5);

/// The screen's filters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    /// Running, waiting or paused.
    Active,
    /// Done, failed or cancelled.
    Finished,
    /// Never started, the built-in definitions among them.
    NotRun,
    All,
}

impl Tab {
    pub const EVERY: [Tab; 4] = [Tab::Active, Tab::Finished, Tab::NotRun, Tab::All];

    pub fn key(self) -> &'static str {
        match self {
            Tab::Active => "active",
            Tab::Finished => "finished",
            Tab::NotRun => "not_run",
            Tab::All => "all",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Tab::Active => "Active",
            Tab::Finished => "Finished",
            Tab::NotRun => "Not run",
            Tab::All => "All",
        }
    }

    pub fn from_key(key: &str) -> Tab {
        Tab::EVERY.into_iter().find(|t| t.key() == key).unwrap_or(Tab::All)
    }

    pub fn holds(self, v: &RecipeView) -> bool {
        match self {
            Tab::Active => recipe_view::is_in_flight(v),
            Tab::Finished => matches!(v.status.as_str(), "done" | "failed" | "cancelled"),
            Tab::NotRun => v.status == "pending",
            Tab::All => true,
        }
    }

    fn empty(self) -> &'static str {
        match self {
            Tab::Active => "Nothing is running. A recipe shows here while it works, its stages lit as it goes.",
            Tab::Finished => "Nothing has finished yet.",
            Tab::NotRun => "Every recipe has run.",
            Tab::All => "No recipes yet. A mind makes one with create_recipe, or starts a built-in one with run_recipe.",
        }
    }
}

struct Screen {
    tab: Tab,
    selected: Option<String>,
    /// The published generation and the local change drawn last.
    drawn: Option<(u64, u64)>,
    local: u64,
    /// Whether the screen was up at the last tick, to ask for a refresh the moment it opens.
    was_up: bool,
    last_refresh: Option<Instant>,
    /// The last outcome shown as a notice.
    notice_serial: u64,
    rows: Rc<VecModel<RecipeRowData>>,
    row_ids: Vec<String>,
    steps: Rc<VecModel<RecipeStepData>>,
    /// The seats chosen on a formation's definition before its Start: (template, seat) → role id.
    seats: HashMap<(String, String), String>,
    /// The seat whose roles are offered now: (template, seat).
    picking: Option<(String, String)>,
    /// The catalog's roles, (id, name), as last read: what a seat offers.
    roles: Vec<(String, String)>,
    /// The run the notice on screen is about, and what it said: once that run finishes, the
    /// notice says so instead (#194: "its agents are at work" stayed over a finished Council).
    notice_about: Option<(String, String)>,
}

type Shared = Rc<RefCell<Screen>>;

pub fn wire(ui: &App, ctx: &AppContext) {
    let companion = ctx.bridge.handle();
    let state: Shared = Rc::new(RefCell::new(Screen {
        tab: Tab::All,
        selected: None,
        drawn: None,
        local: 0,
        was_up: false,
        last_refresh: None,
        notice_serial: 0,
        rows: Rc::new(VecModel::default()),
        row_ids: Vec::new(),
        steps: Rc::new(VecModel::default()),
        seats: HashMap::new(),
        picking: None,
        roles: catalog_roles(),
        notice_about: None,
    }));
    let g = ui.global::<RecipesState>();
    g.set_rows(ModelRc::from(state.borrow().rows.clone()));
    g.set_steps(ModelRc::from(state.borrow().steps.clone()));
    g.set_tab(Tab::All.key().into());
    g.set_empty("Waiting for the companion to read its recipes…".into());

    let weak = ui.as_weak();
    let local = |f: fn(&mut Screen, String)| {
        let (weak, state) = (weak.clone(), state.clone());
        move |arg: SharedString| {
            let Some(ui) = weak.upgrade() else { return };
            let mut st = state.borrow_mut();
            f(&mut *st, arg.to_string());
            st.local += 1;
            draw(&ui, &mut *st);
        }
    };
    g.on_select_tab(local(|st, key| {
        st.tab = Tab::from_key(&key);
    }));
    g.on_select(local(|st, id| {
        st.selected = if st.selected.as_deref() == Some(id.as_str()) { None } else { Some(id) };
    }));
    g.on_show(local(|st, id| {
        let snap = crate::recipes::snapshot();
        if let Some(view) = snap.views.iter().find(|v| v.id == id) {
            if !st.tab.holds(view) {
                st.tab = Tab::All;
            }
        }
        st.selected = Some(id);
    }));

    // A seat of a formation's definition: its roles offered, then one chosen.
    g.on_choose_seat({
        let (weak, state) = (weak.clone(), state.clone());
        move |recipe, seat| {
            let Some(ui) = weak.upgrade() else { return };
            let mut st = state.borrow_mut();
            let key = (recipe.to_string(), seat.to_string());
            st.picking = if st.picking.as_ref() == Some(&key) { None } else { Some(key) };
            // Read again as it is offered: a role added under ~/.config/yantrik/agents shows.
            st.roles = catalog_roles();
            st.local += 1;
            draw(&ui, &mut st);
        }
    });
    g.on_pick_seat({
        let (weak, state) = (weak.clone(), state.clone());
        move |recipe, seat, role| {
            let Some(ui) = weak.upgrade() else { return };
            let mut st = state.borrow_mut();
            st.seats.insert((recipe.to_string(), seat.to_string()), role.to_string());
            st.picking = None;
            st.local += 1;
            draw(&ui, &mut st);
        }
    });
    // An agent's whole session — its answer in full, every call — one click away: the Agents
    // screen with that agent selected.
    g.on_open_agent({
        let weak = weak.clone();
        move |agent| {
            if let Some(ui) = weak.upgrade() {
                ui.global::<AgentsState>().invoke_show_agent(agent);
            }
        }
    });

    let press = |op: fn(String) -> RecipeOp| {
        let (weak, companion) = (weak.clone(), companion.clone());
        move |id: SharedString, arg: String| {
            if let Some(ui) = weak.upgrade() {
                send(&ui.global::<RecipesState>(), &companion, id.to_string(), op(arg));
            }
        }
    };
    {
        let answer = press(RecipeOp::Answer);
        g.on_answer(move |id, text| answer(id, text.to_string()));
        let pause = press(|_| RecipeOp::Pause);
        g.on_pause(move |id| pause(id, String::new()));
        let resume = press(|_| RecipeOp::Resume);
        g.on_resume(move |id| resume(id, String::new()));
        let cancel = press(|_| RecipeOp::Cancel);
        g.on_cancel(move |id| cancel(id, String::new()));
    }
    // Start, on a formation's definition: the person's own press, and so their leave for its
    // agents. The run is the worker's to make; what came of it comes back as the notice.
    g.on_start({
        let (weak, companion, state) = (weak.clone(), companion.clone(), state.clone());
        move |id, text| {
            let Some(ui) = weak.upgrade() else { return };
            let g = ui.global::<RecipesState>();
            let chosen = chosen_seats(&state.borrow().seats, &id);
            match start_request(&crate::recipes::snapshot().views, &id, &text, &chosen) {
                Ok((recipe, inputs)) => {
                    let leave = Leave::new("the person, from the Recipes screen", None);
                    match companion.start_recipe(recipe, inputs, Some(leave)) {
                        Ok(()) => {
                            g.set_notice("Starting…".into());
                            g.set_notice_ok(true);
                        }
                        Err(why) => {
                            g.set_notice(why.into());
                            g.set_notice_ok(false);
                        }
                    }
                }
                Err(why) => {
                    g.set_notice(why.into());
                    g.set_notice_ok(false);
                }
            }
        }
    });
    g.on_dismiss_notice({
        let (weak, state) = (weak.clone(), state.clone());
        move || {
            if let Some(ui) = weak.upgrade() {
                ui.global::<RecipesState>().set_notice("".into());
                state.borrow_mut().notice_about = None;
            }
        }
    });

    let timer = Timer::default();
    timer.start(TimerMode::Repeated, TICK, move || {
        let Some(ui) = weak.upgrade() else { return };
        let mut st = state.borrow_mut();
        let up = ui.get_current_screen() == SCREEN;
        if up && (!st.was_up || st.last_refresh.is_none_or(|t| t.elapsed() >= REFRESH_EVERY)) {
            crate::recipes::request_refresh(&companion);
            st.last_refresh = Some(Instant::now());
        }
        st.was_up = up;
        // Drawn whether or not the screen is up — only when something changed, which is rare —
        // so opening it shows the recipes as they are, not as they were.
        draw(&ui, &mut *st);
    });
    // The timer lives as long as the shell, the idiom every wire module uses.
    std::mem::forget(timer);
}

/// What the Recipes screen's Start asks the worker for: the formation's definition, its one input
/// given as `text`, and each seat the person chose (`seats`, seat → role id) — the rest take their
/// defaults. Refused here for anything the screen cannot start, and for a seat the definition does
/// not have.
pub fn start_request(
    views: &[RecipeView],
    id: &str,
    text: &str,
    seats: &HashMap<String, String>,
) -> Result<(String, serde_json::Map<String, serde_json::Value>), String> {
    let view = views.iter().find(|v| v.id == id).ok_or_else(|| format!("no recipe `{id}` any more"))?;
    if !can_start(view) {
        return Err(format!("'{}' is not started from here; a mind starts it with run_recipe", view.name));
    }
    let input = view.inputs.iter().find(|i| i.default.is_none()).expect("can_start: one input with no default");
    let text = text.trim();
    if text.is_empty() {
        return Err(format!("'{}' needs {} to start", view.name, input.describe.to_lowercase()));
    }
    let mut inputs = serde_json::Map::new();
    inputs.insert(input.name.clone(), serde_json::Value::String(text.to_string()));
    for (seat, role) in seats {
        if !view.inputs.iter().any(|i| i.seat && &i.name == seat) {
            return Err(format!("'{}' has no seat `{seat}`", view.name));
        }
        inputs.insert(seat.clone(), serde_json::Value::String(role.clone()));
    }
    Ok((view.id.clone(), inputs))
}

/// The seats chosen for one definition, seat → role id.
fn chosen_seats(all: &HashMap<(String, String), String>, recipe: &str) -> HashMap<String, String> {
    all.iter().filter(|((r, _), _)| r == recipe).map(|((_, seat), role)| (seat.clone(), role.clone())).collect()
}

/// The catalog's roles as a seat offers them: (id, name).
fn catalog_roles() -> Vec<(String, String)> {
    crate::agents::catalog::Catalog::load().roles.into_iter().map(|r| (r.id, r.name)).collect()
}

/// A formation's seats as its definition's row offers them: each seat, the role in it — the one
/// chosen, or its default — and that role's name from the catalog.
pub fn seats_of(v: &RecipeView, chosen: &HashMap<String, String>, roles: &[(String, String)]) -> Vec<RecipeSeatData> {
    if !can_start(v) {
        return Vec::new();
    }
    v.inputs
        .iter()
        .filter(|i| i.seat)
        .map(|i| {
            let role = chosen.get(&i.name).cloned().or_else(|| i.default.clone()).unwrap_or_default();
            let name = roles
                .iter()
                .find(|(id, _)| *id == role)
                .map(|(_, name)| name.clone())
                .unwrap_or_else(|| yantrik_companion::recipe::role_display(&role));
            RecipeSeatData {
                name: i.name.as_str().into(),
                label: seat_label(&i.name).into(),
                role: role.as_str().into(),
                role_name: name.into(),
                changed: chosen.contains_key(&i.name),
            }
        })
        .collect()
}

/// "seat_2" → "Seat 2", "chair" → "Chair".
fn seat_label(name: &str) -> String {
    yantrik_companion::recipe::role_display(name)
}

/// What the notice about a run says once it has finished (#194), and whether it reads as good
/// news: "Council finished — verdict from the Chair." None while it is still in flight.
pub fn finished_notice(v: &RecipeView) -> Option<(String, bool)> {
    match v.status.as_str() {
        "done" => {
            let last = v.steps.last();
            let by = last
                .filter(|s| s.kind == "agent")
                .and_then(|s| v.agents.iter().find(|a| a.step == s.index))
                .map(|a| a.role.clone());
            let kept = last.and_then(|s| s.store_as.clone()).map(|k| k.replace('_', " "));
            let line = match (by, kept) {
                (Some(role), Some(kept)) => format!("{} finished — {kept} from the {role}. It is under the recipe's last step.", v.name),
                _ => format!("{} finished.", v.name),
            };
            Some((line, true))
        }
        "failed" => Some((
            format!("{} failed: {}", v.name, recipe_view::head(v.error.as_deref().unwrap_or("no error recorded"), 160)),
            false,
        )),
        "cancelled" => Some((format!("{} was cancelled.", v.name), true)),
        _ => None,
    }
}

/// What a finished formation's agents opened on the desktop — an app, a screen, a browser tab —
/// each with the agent that opened it, from their sessions' calls (#194).
///
/// Listed, not closed. The shell cannot tell a window the agent opened from one the person has
/// since turned to (the same app, the same tab brought to the front), a browser tab is not a
/// window the shell can close by itself — it would need the tab's DevTools id, which the agent's
/// call does not carry — and a screen is where the person may now be. Closing any of those under
/// the person would be worse than leaving them; naming them lets the person close what they are
/// done with.
pub fn opened_by_agents(v: &RecipeView, store: &Store) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for a in &v.agents {
        let Some(agent) = store.agent(&AgentId(a.agent.clone())) else { continue };
        for card in agent.cards() {
            if let Some(what) = opened(card) {
                let line = format!("{what} (the {})", a.role);
                if !out.contains(&line) {
                    out.push(line);
                }
            }
        }
    }
    out
}

/// What one call opened, if it opened something and did not fail: `os_act shell.open_app`,
/// `open_app`, `show_screen`, or a call that opened a web address.
fn opened(card: &Card) -> Option<String> {
    if matches!(card.state, CallState::Failed | CallState::Interrupted) {
        return None;
    }
    let args = &card.args;
    let inner = args.get("args").filter(|a| a.is_object()).unwrap_or(args);
    let field = |k: &str| inner.get(k).or_else(|| args.get(k)).and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty());
    let said = format!(
        "{} {} {}.{}",
        card.name,
        card.target,
        args.get("app").and_then(|v| v.as_str()).unwrap_or_default(),
        args.get("action").and_then(|v| v.as_str()).unwrap_or_default()
    )
    .to_lowercase();
    if said.contains("open_app") {
        return field("name").or_else(|| field("app_id")).map(str::to_string);
    }
    if said.contains("show_screen") {
        return field("screen").map(|screen| format!("the {screen} screen"));
    }
    let web = ["navigate", "open_url", "new_tab", "open_tab", "browse"].iter().any(|w| said.contains(w));
    match field("url") {
        Some(url) if web => Some(format!("a browser tab at {url}")),
        _ => None,
    }
}

/// The row's line for them: empty unless the formation has finished and its agents opened any.
fn opened_line(v: &RecipeView, store: &Store) -> String {
    if !v.formation || !matches!(v.status.as_str(), "done" | "failed" | "cancelled") {
        return String::new();
    }
    let opened = opened_by_agents(v, store);
    if opened.is_empty() {
        return String::new();
    }
    format!(
        "Its agents opened {}. They are left as they are — close what you are done with.",
        opened.join(", ")
    )
}

/// A formation's built-in definition with exactly one input a run must be given: the screen asks
/// for it and starts it.
fn can_start(v: &RecipeView) -> bool {
    v.template && v.formation && v.inputs.iter().filter(|i| i.default.is_none()).count() == 1
}

/// Hand a press to the worker. What it made of it comes back as the notice.
fn send(g: &RecipesState, companion: &CompanionHandle, id: String, op: RecipeOp) {
    let doing = match op {
        RecipeOp::Answer(_) => "Answering…",
        RecipeOp::Pause => "Pausing…",
        RecipeOp::Resume => "Resuming…",
        RecipeOp::Cancel => "Cancelling…",
    };
    match companion.recipe(id, op) {
        Ok(()) => {
            g.set_notice(doing.into());
            g.set_notice_ok(true);
        }
        Err(why) => {
            g.set_notice(why.into());
            g.set_notice_ok(false);
        }
    }
}

/// Redraw from the published copy, if it or the screen's own state moved since the last draw.
fn draw(ui: &App, st: &mut Screen) {
    let snap = crate::recipes::snapshot();
    let key = (snap.generation, st.local);
    if st.drawn == Some(key) {
        return;
    }
    st.drawn = Some(key);
    let g = ui.global::<RecipesState>();
    g.set_loaded(snap.loaded);
    g.set_tab(st.tab.key().into());
    g.set_tabs(ModelRc::new(VecModel::from(tabs(&snap.views))));

    if st.selected.as_ref().is_some_and(|id| !snap.views.iter().any(|v| &v.id == id)) {
        st.selected = None;
    }
    let shown: Vec<&RecipeView> = snap.views.iter().filter(|v| st.tab.holds(v)).collect();
    let now = unix_now();
    let picking = st.picking.clone().map(|(recipe, seat)| format!("{recipe}/{seat}")).unwrap_or_default();
    let rows: Vec<RecipeRowData> = crate::agents::store().read(|store| {
        shown
            .iter()
            .map(|v| {
                let mut row = row_of(v, now);
                let seats = seats_of(v, &chosen_seats(&st.seats, &v.id), &st.roles);
                if let Some((_, seat)) = st.picking.as_ref().filter(|(recipe, _)| *recipe == v.id) {
                    row.picking = seat.as_str().into();
                    row.picking_role = seats.iter().find(|s| s.name.as_str() == seat).map(|s| s.role.clone()).unwrap_or_default();
                }
                row.seats = ModelRc::new(VecModel::from(seats));
                row.opened = opened_line(v, store).into();
                row
            })
            .collect()
    });
    g.set_picking(picking.into());
    g.set_roles(ModelRc::new(VecModel::from(
        st.roles.iter().map(|(id, name)| RecipeRoleData { id: id.as_str().into(), name: name.as_str().into() }).collect::<Vec<_>>(),
    )));
    let ids: Vec<String> = shown.iter().map(|v| v.id.clone()).collect();
    if ids == st.row_ids {
        // The same recipes in the same order: updated in place, so what is typed into a
        // question's box is not thrown away because another recipe took a step.
        for (i, row) in rows.into_iter().enumerate() {
            st.rows.set_row_data(i, row);
        }
    } else {
        st.rows.set_vec(rows);
        st.row_ids = ids;
    }

    let selected = st.selected.clone().unwrap_or_default();
    g.set_selected(selected.clone().into());
    let steps: Vec<RecipeStepData> = shown
        .iter()
        .find(|v| v.id == selected)
        .map(|v| v.steps.iter().map(step_of).collect())
        .unwrap_or_default();
    st.steps.set_vec(steps);

    g.set_empty(if snap.loaded { st.tab.empty() } else { "Waiting for the companion to read its recipes…" }.into());
    if let Some(outcome) = snap.outcome.filter(|o| o.serial > st.notice_serial) {
        st.notice_serial = outcome.serial;
        g.set_notice(outcome.text.as_str().into());
        g.set_notice_ok(outcome.ok);
        st.notice_about = outcome.ok.then(|| (outcome.recipe.clone(), outcome.text.clone()));
    }
    // The notice about a run that has since finished says it finished — never "its agents are at
    // work" over a done Council (#194). Only while that notice is still the one on screen.
    if let Some((run, said)) = st.notice_about.clone() {
        if let Some((line, ok)) = snap.views.iter().find(|v| v.id == run).and_then(finished_notice) {
            if g.get_notice().as_str() == said {
                g.set_notice(line.into());
                g.set_notice_ok(ok);
            }
            st.notice_about = None;
        }
    }
}

fn tabs(views: &[RecipeView]) -> Vec<RecipeTabData> {
    Tab::EVERY
        .iter()
        .map(|t| RecipeTabData {
            id: t.key().into(),
            label: t.label().into(),
            count: views.iter().filter(|v| t.holds(v)).count() as i32,
        })
        .collect()
}

/// One recipe's row: its name, where it stands, its stages, and what it waits on.
pub fn row_of(v: &RecipeView, now: f64) -> RecipeRowData {
    let total = v.steps.len();
    let focus = crate::recipes::focus(v);
    let at = focus.map(|s| s.index + 1);
    let status_label = if v.needs_you.is_some() {
        "waiting for you".to_string()
    } else if v.template && v.formation {
        "formation, never run".to_string()
    } else if v.template {
        "built-in, never run".to_string()
    } else {
        match v.status.as_str() {
            "pending" => "not started".to_string(),
            "waiting" if v.question.is_some() => "waiting for you".to_string(),
            other => other.to_string(),
        }
    };
    let progress = match (v.status.as_str(), at) {
        ("running" | "waiting" | "paused", Some(k)) => format!("step {k} of {total}"),
        ("failed" | "cancelled", Some(k)) => format!("stopped at step {k} of {total}"),
        _ => count(total, "step"),
    };
    let when = if v.template {
        String::new()
    } else if v.status == "pending" {
        format!("created {}", clock(v.created_at, now))
    } else {
        format!("updated {}", clock(v.updated_at, now))
    };
    let error = match (v.status.as_str(), v.error.as_deref()) {
        ("cancelled", _) => format!("Cancelled{}.", at.map(|k| format!(" at step {k}")).unwrap_or_default()),
        ("failed", Some(e)) => match at {
            Some(k) => format!("Step {k} failed: {}", recipe_view::head(e, 200)),
            None => format!("Failed: {}", recipe_view::head(e, 200)),
        },
        ("failed", None) => "Failed, with no error recorded.".to_string(),
        _ => String::new(),
    };
    let waiting_for = match (v.status.as_str(), v.waiting_for.as_deref()) {
        ("paused", Some(w)) => format!("Paused while waiting for {w}. Resume and it waits again."),
        ("paused", None) => format!("Paused before step {}.", v.current_step + 1),
        (_, Some(w)) => format!("Waiting for {w}"),
        _ => String::new(),
    };
    let unbound: Vec<String> = {
        let mut names: Vec<String> = Vec::new();
        for s in &v.steps {
            for n in &s.unbound {
                if !names.contains(n) {
                    names.push(n.clone());
                }
            }
        }
        names
    };
    let stages: Vec<RecipeStageData> = v
        .steps
        .iter()
        .map(|s| RecipeStageData {
            number: (s.index + 1).to_string().into(),
            kind: s.kind.as_str().into(),
            label: s.label.as_str().into(),
            state: s.state.as_str().into(),
        })
        .collect();
    let (question, choices) = match &v.question {
        Some(q) => (q.text.clone(), q.choices.iter().map(|c| SharedString::from(c.as_str())).collect()),
        None => (String::new(), Vec::new()),
    };
    let start_hint = if can_start(v) {
        v.inputs.iter().find(|i| i.default.is_none()).map(|i| format!("{}…", i.describe)).unwrap_or_default()
    } else {
        String::new()
    };
    RecipeRowData {
        id: v.id.as_str().into(),
        name: v.name.as_str().into(),
        status: v.status.as_str().into(),
        status_label: status_label.into(),
        progress: progress.into(),
        when: when.into(),
        error: error.into(),
        waiting_for: waiting_for.into(),
        question: question.into(),
        choices: ModelRc::new(VecModel::from(choices)),
        template: v.template,
        unbound: unbound_note(&unbound).into(),
        can_answer: v.can.answer,
        can_pause: v.can.pause,
        can_resume: v.can.resume,
        can_cancel: v.can.cancel,
        stages: ModelRc::new(VecModel::from(stages)),
        formation: v.formation,
        can_start: can_start(v),
        start_hint: start_hint.into(),
        needs_you: v.needs_you.is_some(),
        seats: ModelRc::default(),
        picking: Default::default(),
        picking_role: Default::default(),
        opened: Default::default(),
        // A run a trigger started: nobody at the desk did, so its agents ask on a card.
        started_by: v
            .started_by
            .as_deref()
            .map(|by| format!("Started on its own by {by} — nobody at the desk, so its agents ask before they start."))
            .unwrap_or_default()
            .into(),
        triggers: if v.triggers.is_empty() { String::new() } else { format!("Starts on its own {}.", v.triggers.join("; ")) }.into(),
    }
}

/// One step, opened.
pub fn step_of(s: &StepView) -> RecipeStepData {
    let ran = matches!(s.state.as_str(), "done" | "failed" | "skipped" | "waiting");
    let names = s.unbound.iter().map(|n| format!("{{{{{n}}}}}")).collect::<Vec<_>>().join(", ");
    let unbound = match (s.unbound.len(), ran) {
        (0, _) => String::new(),
        (_, true) => format!("{names} had no value when it ran"),
        (1, false) => format!("{names} has no value, and no step before this one sets it"),
        (_, false) => format!("{names} have no value, and no step before this one sets them"),
    };
    RecipeStepData {
        number: (s.index + 1).to_string().into(),
        kind: s.kind.as_str().into(),
        label: s.label.as_str().into(),
        summary: s.summary.as_str().into(),
        state: s.state.as_str().into(),
        state_label: state_label(&s.state, &s.kind).into(),
        result: s.result.clone().unwrap_or_default().into(),
        path: s.path.clone().unwrap_or_default().into(),
        unbound: unbound.into(),
        agent: s.agent.clone().unwrap_or_default().into(),
        detail: s.detail.join("\n").into(),
        agent_id: s.agent_id.clone().unwrap_or_default().into(),
        answer: ModelRc::new(VecModel::from(answer_blocks(s.answer.as_deref().unwrap_or_default()))),
        reads: ModelRc::new(VecModel::from(
            s.reads
                .iter()
                .map(|l| RecipeLinkData { label: l.label.as_str().into(), agent: l.agent.as_str().into() })
                .collect::<Vec<_>>(),
        )),
    }
}

/// An answer's head as the opened step draws it: a block per paragraph, heading, list or code, read
/// by the Lens's own parser and drawn as the Agents pane draws them (`crate::markdown`, #197) — not
/// the raw `## Verdict` and `**1.**` it came as (#194).
pub fn answer_blocks(answer: &str) -> Vec<RecipeAnswerBlock> {
    crate::markdown::parse_blocks(answer)
        .iter()
        .map(|block| RecipeAnswerBlock {
            block: match block.block_type {
                kind @ ("heading" | "bullet" | "code") => kind,
                _ => "text",
            }
            .into(),
            text: block.text.as_str().into(),
            styled: crate::markdown::styled(block),
        })
        .collect()
}

fn state_label(state: &str, kind: &str) -> String {
    match state {
        "done" => "done",
        "current" => "running now",
        "waiting" if kind == "ask_user" => "waiting for you",
        "waiting" if kind == "agent" => "working",
        "waiting" => "waiting",
        "failed" => "failed",
        "skipped" => "skipped",
        "pending" => "to come",
        "not_taken" => "not taken",
        "paused" => "paused here",
        "stopped" => "stopped here",
        "unknown" => "not recorded",
        other => other,
    }
    .to_string()
}

fn unbound_note(names: &[String]) -> String {
    let shown: Vec<String> = names.iter().take(3).map(|n| format!("{{{{{n}}}}}")).collect();
    let more = names.len().saturating_sub(3);
    let list = if more > 0 { format!("{} and {more} more", shown.join(", ")) } else { shown.join(", ") };
    match names.len() {
        0 => String::new(),
        1 => format!("{list} has no value"),
        _ => format!("{list} have no value"),
    }
}

fn count(n: usize, what: &str) -> String {
    if n == 1 { format!("1 {what}") } else { format!("{n} {what}s") }
}

fn unix_now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// Local time today, the date before today.
fn clock(unix: f64, now: f64) -> String {
    use chrono::TimeZone;
    let (Some(at), Some(today)) = (
        chrono::Local.timestamp_opt(unix as i64, 0).single(),
        chrono::Local.timestamp_opt(now as i64, 0).single(),
    ) else {
        return String::new();
    };
    if at.date_naive() == today.date_naive() {
        at.format("%H:%M").to_string()
    } else {
        at.format("%b %-d, %H:%M").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use yantrik_companion::recipe::{Recipe, RecipeStatus, RecipeStep, StoredStep};

    fn read(rel: &str) -> String {
        std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)).unwrap()
    }

    fn fixture(status: RecipeStatus, current_step: usize, statuses: &[&str], error: Option<&str>) -> RecipeView {
        let recipe = Recipe {
            id: "rcp_tidy".into(),
            name: "Tidy downloads".into(),
            description: String::new(),
            status,
            current_step,
            created_at: 1_790_000_000.0,
            updated_at: 1_790_000_060.0,
            enabled: true,
            error_message: error.map(str::to_string),
        };
        let steps = vec![
            RecipeStep::Tool {
                tool_name: "list_dir".into(),
                args: json!({"path": "~/Downloads/{{subfolder}}"}),
                store_as: "files".into(),
                on_error: Default::default(),
            },
            RecipeStep::AskUser { question: "Move them where?".into(), store_as: "folder".into(), choices: Some(vec!["Archive".into(), "Trash".into()]) },
            RecipeStep::Notify { message: "Moved to {{folder}}".into() },
        ];
        let stored: Vec<StoredStep> = steps
            .into_iter()
            .enumerate()
            .map(|(i, step)| StoredStep { step_index: i, step, status: statuses.get(i).unwrap_or(&"pending").to_string(), result: None })
            .collect();
        recipe_view::view(&recipe, &stored, &Default::default())
    }

    /// A recipe waiting on the person: its question and choices in the row, its stages ticked,
    /// waited on and to come, and the placeholder nothing set called out.
    #[test]
    fn a_row_draws_its_stages_and_the_question_it_waits_on() {
        let v = fixture(RecipeStatus::Waiting, 2, &["done", "done"], None);
        let row = row_of(&v, 1_790_000_100.0);
        let stages: Vec<(String, String)> =
            row.stages.iter().map(|s| (s.label.to_string(), s.state.to_string())).collect();
        assert_eq!(
            stages,
            [("list_dir".into(), "done".into()), ("Ask you".into(), "waiting".into()), ("Notify".into(), "pending".into())]
        );
        assert_eq!(row.status_label, "waiting for you");
        assert_eq!(row.progress, "step 2 of 3");
        assert!(row.can_answer && row.can_pause && row.can_cancel && !row.can_resume);
        assert_eq!(row.question, "Move them where?");
        assert_eq!(row.choices.iter().map(|c| c.to_string()).collect::<Vec<_>>(), ["Archive", "Trash"]);
        assert_eq!(row.waiting_for, "Waiting for your answer");
        assert_eq!(row.unbound, "{{subfolder}} has no value");
        assert!(row.when.starts_with("updated "), "{}", row.when);

        let step = step_of(&v.steps[0]);
        assert_eq!(step.unbound, "{{subfolder}} had no value when it ran");
        assert_eq!(step.summary, r#"list_dir path="~/Downloads/{{subfolder}}""#);
        assert_eq!(step_of(&v.steps[1]).state_label, "waiting for you");
        assert_eq!(step_of(&v.steps[2]).state_label, "to come");
        assert!(step_of(&v.steps[0]).detail.contains("keeps it as: files"));
    }

    #[test]
    fn a_failed_row_is_red_with_its_error_and_a_cancelled_one_says_so() {
        let failed = row_of(&fixture(RecipeStatus::Failed, 0, &["failed"], Some("Unknown tool: list_dir")), 0.0);
        assert_eq!(failed.status, "failed");
        assert_eq!(failed.error, "Step 1 failed: Unknown tool: list_dir");
        assert_eq!(failed.progress, "stopped at step 1 of 3");
        assert!(!failed.can_pause && !failed.can_cancel);
        let states: Vec<String> = failed.stages.iter().map(|s| s.state.to_string()).collect();
        assert_eq!(states, ["failed", "not_taken", "not_taken"]);

        let cancelled = row_of(&fixture(RecipeStatus::Failed, 1, &["done"], Some(yantrik_companion::recipe::CANCELLED)), 0.0);
        assert_eq!(cancelled.status, "cancelled");
        assert_eq!(cancelled.error, "Cancelled at step 2.");

        let paused = row_of(&fixture(RecipeStatus::Paused, 1, &["done"], None), 0.0);
        assert_eq!(paused.waiting_for, "Paused before step 2.");
        assert!(paused.can_resume && !paused.can_pause);
    }

    /// A formation mid-flight, as its row draws it: each Agent stage called by its role and mind,
    /// working ones waiting, the Chair to come; the step opened says who is at work. Its built-in
    /// definition asks for its one input, and Start hands the worker that input and the id.
    #[test]
    fn a_formation_draws_its_agents_and_its_definition_starts_with_its_input() {
        use yantrik_companion::recipe::{AgentRun, AGENTS_VAR};
        use yantrik_companion::recipe_templates::{self, formations};
        let template = recipe_templates::get_template(formations::COUNCIL).unwrap();
        let steps: Vec<StoredStep> = (template.steps)()
            .into_iter()
            .enumerate()
            .map(|(i, step)| StoredStep { step_index: i, step, status: if i == 0 { "done" } else { "pending" }.into(), result: None })
            .collect();
        let run = |role: &str, mind: &str, n: u32, state: &str| AgentRun {
            role: role.into(),
            role_name: yantrik_companion::recipe::role_display(role),
            mind: mind.into(),
            agent: format!("{mind}:c-00{n}"),
            store_as: format!("answer_{n}"),
            since: 0.0,
            until: 9e9,
            state: state.into(),
            needs_you: None,
        };
        let agents: serde_json::Map<String, serde_json::Value> = [
            ("0", run("researcher", "deepseek", 1, "answered")),
            ("1", run("red-team", "pi", 2, "working")),
            ("2", run("planner", "deepseek", 3, "working")),
        ]
        .into_iter()
        .map(|(k, r)| (k.to_string(), serde_json::to_value(r).unwrap()))
        .collect();
        let vars = std::collections::HashMap::from([
            ("question".to_string(), json!("Should we ship on Friday?")),
            ("seat_1".to_string(), json!("researcher")),
            ("seat_2".to_string(), json!("red-team")),
            ("seat_3".to_string(), json!("planner")),
            ("chair".to_string(), json!("chair")),
            ("answer_1".to_string(), json!("Yes: the tests pass.")),
            (AGENTS_VAR.to_string(), serde_json::Value::Object(agents)),
            ("_wait".to_string(), json!({"step": 3, "since": 0.0, "agents": true})),
        ]);
        let recipe = Recipe {
            id: "rcp_council".into(),
            name: "Council".into(),
            description: String::new(),
            status: RecipeStatus::Waiting,
            current_step: 3,
            created_at: 1_790_000_000.0,
            updated_at: 1_790_000_060.0,
            enabled: true,
            error_message: None,
        };
        let v = recipe_view::view(&recipe, &steps, &vars);
        let row = row_of(&v, 1_790_000_100.0);
        let stages: Vec<(String, String)> = row.stages.iter().map(|s| (s.label.to_string(), s.state.to_string())).collect();
        assert_eq!(
            stages,
            [
                ("Researcher · deepseek".into(), "done".into()),
                ("Red team · pi".into(), "waiting".into()),
                ("Planner · deepseek".into(), "waiting".into()),
                ("Chair".into(), "pending".into())
            ]
        );
        assert!(row.formation && !row.can_start);
        assert_eq!(row.waiting_for, "Waiting for answers from the Red team (pi) and the Planner (deepseek)");
        assert_eq!(row.progress, "step 2 of 4", "lit at the first agent still at work");
        let working = step_of(&v.steps[1]);
        assert_eq!((working.state_label.as_str(), working.agent.as_str()), ("working", "the Red team on pi (pi:c-002)"));
        assert_eq!(step_of(&v.steps[0]).result, "Yes: the tests pass.");
        assert_eq!(step_of(&v.steps[3]).agent, "Chair");

        // The definition: its input asked for, and Start's request.
        let mut def = recipe.clone();
        def.id = formations::COUNCIL.into();
        def.status = RecipeStatus::Pending;
        def.current_step = 0;
        let fresh: Vec<StoredStep> = steps.iter().cloned().map(|mut s| { s.status = "pending".into(); s }).collect();
        let dv = recipe_view::view(&def, &fresh, &Default::default());
        let drow = row_of(&dv, 0.0);
        assert!(drow.can_start && drow.template);
        assert_eq!(drow.status_label, "formation, never run");
        assert_eq!(drow.start_hint, "The question the council is to answer…");
        let views = vec![dv.clone(), v.clone()];
        let none = HashMap::new();
        let (id, inputs) = start_request(&views, formations::COUNCIL, "  Should we ship?  ", &none).unwrap();
        assert_eq!((id.as_str(), inputs["question"].as_str()), (formations::COUNCIL, Some("Should we ship?")));
        assert!(start_request(&views, formations::COUNCIL, " ", &none).unwrap_err().contains("needs the question"));
        assert!(start_request(&views, "rcp_council", "x", &none).unwrap_err().contains("is not started from here"));
        let recipes_slint = read("../yantrik-ui-slint/ui/recipes.slint");
        assert!(recipes_slint.contains("RecipesState.start(root.recipe.id, start-input.value);"), "Start sends the input");
        assert!(recipes_slint.contains("root.kind == \"agent\" ? Icons.people"), "a working agent's stage wears the agents mark");

        // When its agents need the person — a card to answer, a place to free — the row says so,
        // and so does the mind panel, which counts it among what needs the person.
        let mut stuck = v.clone();
        stuck.needs_you = Some("a place: the desktop is running 6 agents, the most it runs at once".into());
        let row = row_of(&stuck, 1_790_000_100.0);
        assert!(row.needs_you);
        assert_eq!((row.status.as_str(), row.status_label.as_str()), ("waiting", "waiting for you"));
        let line = crate::mind_panel::recipe_line(&stuck);
        assert!(line.needs_you && line.step.starts_with("needs you: a place: the desktop is running 6 agents"), "{line:?}");
        assert!(!crate::mind_panel::recipe_line(&v).needs_you);
        assert_eq!(crate::mind_panel::recipe_line(&v).step, "step 2 of 4 · Red team · pi at work");
    }

    /// The Council's definition as its row offers it (#194): each seat with the role in it — the
    /// default until one is chosen — and Start hands the worker the seats chosen with the question.
    /// A seat the definition does not have is refused.
    #[test]
    fn a_formations_seats_are_chosen_on_the_screen_and_start_with_it() {
        use yantrik_companion::recipe_templates::{self, formations};
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        yantrik_companion::recipe::RecipeStore::ensure_tables(&conn);
        recipe_templates::register_all(&conn);
        let views = recipe_view::list(&conn);
        let council = views.iter().find(|v| v.id == formations::COUNCIL).unwrap();
        let roles = vec![("researcher".to_string(), "Researcher".to_string()), ("reviewer".to_string(), "Reviewer".to_string())];
        let shown = |chosen: &HashMap<String, String>| -> Vec<(String, String, String, bool)> {
            seats_of(council, chosen, &roles)
                .into_iter()
                .map(|s| (s.label.to_string(), s.role.to_string(), s.role_name.to_string(), s.changed))
                .collect()
        };
        assert_eq!(
            shown(&HashMap::new()),
            [
                ("Seat 1".to_string(), "researcher".to_string(), "Researcher".to_string(), false),
                ("Seat 2".to_string(), "red-team".to_string(), "Red team".to_string(), false),
                ("Seat 3".to_string(), "planner".to_string(), "Planner".to_string(), false),
                ("Chair".to_string(), "chair".to_string(), "Chair".to_string(), false),
            ]
        );
        let chosen = HashMap::from([("seat_2".to_string(), "reviewer".to_string())]);
        assert_eq!(shown(&chosen)[1], ("Seat 2".to_string(), "reviewer".to_string(), "Reviewer".to_string(), true));
        let (id, inputs) = start_request(&views, formations::COUNCIL, "Ship on Friday?", &chosen).unwrap();
        assert_eq!(id, formations::COUNCIL);
        assert_eq!(inputs.get("seat_2").and_then(|v| v.as_str()), Some("reviewer"));
        assert_eq!(inputs.get("seat_1"), None, "the rest take their defaults");
        let wrong = HashMap::from([("question".to_string(), "reviewer".to_string())]);
        assert!(start_request(&views, formations::COUNCIL, "q", &wrong).unwrap_err().contains("has no seat `question`"));
        // Only a formation's definition offers seats; a Writers' room's cast is not one.
        let room = views.iter().find(|v| v.id == formations::WRITERS_ROOM).unwrap();
        assert!(seats_of(room, &HashMap::new(), &roles).is_empty());
        let slint = read("../yantrik-ui-slint/ui/recipes.slint");
        assert!(slint.contains("RecipesState.pick-seat(root.recipe.id, root.recipe.picking, role.id);"), "a role is chosen for its seat");
    }

    /// A finished run's banner says it finished (#194), and the answer head is drawn from its
    /// markdown (#194), and the agent whose session holds it all is a click away.
    #[test]
    fn a_finished_council_says_so_and_draws_its_answer() {
        use yantrik_companion::recipe::{AgentRun, AGENTS_VAR};
        let mut v = fixture(RecipeStatus::Done, 3, &["done", "done", "done"], None);
        assert_eq!(finished_notice(&v), Some(("Tidy downloads finished.".to_string(), true)));
        v.status = "failed".into();
        v.error = Some("Unknown tool: list_dir".into());
        assert_eq!(finished_notice(&v), Some(("Tidy downloads failed: Unknown tool: list_dir".to_string(), false)));
        v.status = "running".into();
        assert_eq!(finished_notice(&v), None, "still in flight: the notice stands");

        // A Council done: the Chair's verdict.
        let template = yantrik_companion::recipe_templates::get_template(yantrik_companion::recipe_templates::formations::COUNCIL).unwrap();
        let steps: Vec<StoredStep> = (template.steps)()
            .into_iter()
            .enumerate()
            .map(|(i, step)| StoredStep { step_index: i, step, status: "done".into(), result: None })
            .collect();
        let run = |role: &str, n: u32, store_as: &str| AgentRun {
            role: role.into(),
            role_name: yantrik_companion::recipe::role_display(role),
            mind: "deepseek".into(),
            agent: format!("deepseek:c-00{n}"),
            store_as: store_as.into(),
            since: 0.0,
            until: 9e9,
            state: "answered".into(),
            needs_you: None,
        };
        let agents: serde_json::Map<String, serde_json::Value> = [
            ("0", run("researcher", 1, "answer_1")),
            ("1", run("red-team", 2, "answer_2")),
            ("2", run("planner", 3, "answer_3")),
            ("3", run("chair", 4, "verdict")),
        ]
        .into_iter()
        .map(|(k, r)| (k.to_string(), serde_json::to_value(r).unwrap()))
        .collect();
        let verdict = "## Verdict\n\nPublish, **but only** the build without the mind.\n\n- the check is optional\n- say so in the notes";
        let vars = std::collections::HashMap::from([
            ("question".to_string(), json!("Publish the nightly?")),
            ("seat_1".to_string(), json!("researcher")),
            ("seat_2".to_string(), json!("red-team")),
            ("seat_3".to_string(), json!("planner")),
            ("chair".to_string(), json!("chair")),
            ("answer_1".to_string(), json!("Yes.")),
            ("answer_2".to_string(), json!("No.")),
            ("answer_3".to_string(), json!("Monday.")),
            ("verdict".to_string(), json!(verdict)),
            (AGENTS_VAR.to_string(), serde_json::Value::Object(agents)),
        ]);
        let recipe = Recipe {
            id: "rcp_council_done".into(),
            name: "Council".into(),
            description: String::new(),
            status: RecipeStatus::Done,
            current_step: 4,
            created_at: 0.0,
            updated_at: 0.0,
            enabled: true,
            error_message: None,
        };
        let v = recipe_view::view(&recipe, &steps, &vars);
        assert_eq!(finished_notice(&v).unwrap().0, "Council finished — verdict from the Chair. It is under the recipe's last step.");
        let chair = step_of(&v.steps[3]);
        let blocks: Vec<(String, String)> = chair.answer.iter().map(|b| (b.block.to_string(), b.text.to_string())).collect();
        assert_eq!(
            blocks,
            [
                ("heading".to_string(), "Verdict".to_string()),
                ("text".to_string(), "Publish, but only the build without the mind.".to_string()),
                ("bullet".to_string(), "\u{2022} the check is optional\n\u{2022} say so in the notes".to_string()),
            ]
        );
        assert!(format!("{:?}", chair.answer.row_data(1).unwrap().styled).contains("Strong"), "drawn bold, not with asterisks");
        assert_eq!(chair.agent_id, "deepseek:c-004", "its session, a click away");
        let links: Vec<(String, String)> = chair.reads.iter().map(|l| (l.label.to_string(), l.agent.to_string())).collect();
        assert_eq!(links[0], ("Researcher · step 1".to_string(), "deepseek:c-001".to_string()));
        assert!(chair.detail.contains("[step 1: the Researcher's answer]") && !chair.detail.contains("{{"), "{}", chair.detail);
        assert!(step_of(&v.steps[0]).detail.contains("The question: Publish the nightly?"));
        let slint = read("../yantrik-ui-slint/ui/recipes.slint");
        assert!(slint.contains("RecipesState.open-agent(root.step.agent-id);"), "Open its session");
        assert!(slint.contains("for block in root.step.answer : AnswerBlock"), "the answer's blocks are drawn");
    }

    /// When a formation finishes, what its agents opened is listed under its row — an app, a
    /// screen, a browser tab — with who opened it; a call that failed opened nothing (#194).
    #[test]
    fn what_a_formations_agents_opened_is_listed_when_it_ends() {
        use crate::agents::{AgentMeta, Event, Provenance, RecipeOrigin};
        let mut store = Store::new();
        let researcher = AgentId("deepseek:c-opener".into());
        let mut meta = AgentMeta::new(researcher.clone(), "deepseek");
        meta.recipe = Some(RecipeOrigin { id: "rcp_opened".into(), name: "Council".into() });
        store.upsert_agent(meta);
        store.open_turn(&researcher, "Publish the nightly?");
        let mut n = 0;
        let mut call = |store: &mut Store, name: &str, args: serde_json::Value, ok: bool| {
            n += 1;
            let id = format!("c{n}");
            store.event(&researcher, &Event::ToolStart { call: id.clone(), name: name.into(), target: String::new(), args }, Provenance::Reported);
            store.event(&researcher, &Event::ToolEnd { call: id, ok, summary: String::new(), exit_code: None }, Provenance::Reported);
        };
        call(&mut store, "os_act", json!({"app": "shell", "action": "open_app", "args": {"name": "chromium"}}), true);
        call(&mut store, "os_act", json!({"app": "shell", "action": "show_screen", "args": {"screen": "problems"}}), true);
        call(&mut store, "browser_navigate", json!({"url": "https://example.org/releases"}), true);
        call(&mut store, "os_act", json!({"app": "shell", "action": "open_app", "args": {"name": "terminal"}}), false);
        call(&mut store, "os_act", json!({"app": "notes", "action": "list"}), true);
        store.close_turn(&researcher, true);

        let mut v = fixture(RecipeStatus::Done, 3, &["done", "done", "done"], None);
        v.formation = true;
        v.agents = vec![yantrik_companion::recipe_view::AgentView {
            step: 0,
            role: "Researcher".into(),
            mind: "deepseek".into(),
            agent: researcher.0.clone(),
            state: "answered".into(),
            needs_you: None,
        }];
        assert_eq!(
            opened_by_agents(&v, &store),
            [
                "chromium (the Researcher)",
                "the problems screen (the Researcher)",
                "a browser tab at https://example.org/releases (the Researcher)",
            ]
        );
        assert!(opened_line(&v, &store).starts_with("Its agents opened chromium (the Researcher), the problems screen"));
        v.status = "running".into();
        assert_eq!(opened_line(&v, &store), "", "listed when it ends, not while it works");
    }

    #[test]
    fn tabs_hold_what_they_say() {
        let views = vec![
            fixture(RecipeStatus::Waiting, 2, &["done", "done"], None),
            fixture(RecipeStatus::Running, 0, &[], None),
            fixture(RecipeStatus::Done, 3, &["done", "done", "done"], None),
            fixture(RecipeStatus::Pending, 0, &[], None),
        ];
        let counts: Vec<(String, i32)> = tabs(&views).into_iter().map(|t| (t.id.to_string(), t.count)).collect();
        assert_eq!(
            counts,
            [("active".into(), 2), ("finished".into(), 1), ("not_run".into(), 1), ("all".into(), 4)]
        );
        assert_eq!(Tab::from_key("finished"), Tab::Finished);
        assert_eq!(Tab::from_key("nonsense"), Tab::All);
    }

    /// The screen is reachable every way a screen must be: `show_screen recipes`, `open_app
    /// recipes`, a launcher tile with an icon, the taskbar on it, a title, and `describe shell`.
    #[test]
    fn the_recipes_screen_is_registered_everywhere_a_screen_must_be() {
        let app = read("../yantrik-ui-slint/ui/app.slint");
        let branch = format!("if current-screen == {SCREEN} : WindowFrame");
        let at = app.find(&branch).expect("app.slint draws the Recipes screen at SCREEN");
        let block: Vec<&str> = app[at..].lines().take(40).collect();
        assert!(block.iter().any(|l| l.trim_start().starts_with("RecipesScreen {")), "screen {SCREEN} draws RecipesScreen");
        assert!(block.iter().any(|l| l.contains("title: Tr.title-recipes;")), "it is titled");
        assert!(block.iter().any(|l| l.contains("root.minimized-app-id = \"recipes\";")), "it minimises as recipes");
        let taskbar = app.lines().find(|l| l.contains(": Rectangle") && l.contains("current-screen == 1 ||")).expect("the taskbar's condition");
        assert!(taskbar.contains(&format!("current-screen == {SCREEN}")), "the taskbar shows on the Recipes screen: {taskbar}");
        assert!(app.contains("export { RecipesState"), "RecipesState is exported from app.slint");
        assert!(read("../yantrik-ui-slint/ui/translations.slint").contains("title-recipes: \"Recipes\""));

        assert_eq!(crate::control::screen_name(SCREEN), "recipes");
        let control = read("src/control.rs");
        let control = control.split("#[cfg(test)]").next().unwrap();
        assert!(control.contains(".with(\"recipes\", crate::recipes::for_describe())"), "describe shell lists the recipes");
        assert!(control.contains("agents, recipes"), "show_screen's description offers recipes");

        use crate::wire::dock::{availability, route, Availability, Launch};
        assert_eq!(route("recipes"), Some(Launch::Screen(SCREEN)));
        assert_eq!(availability("recipes", &[]), Availability::Ready);
        let listed = crate::wire::dock::openable();
        let entry = listed.iter().find(|a| a["name"] == "recipes").expect("open_app lists recipes");
        assert!(entry["for"].as_str().is_some_and(|f| f.contains("recipe")), "{entry}");

        assert!(crate::apps::builtin_apps().iter().any(|e| e.app_id == "recipes" && e.name == "Recipes"));
        assert!(read("../yantrik-ui-kit/slint/icon.slint").contains("id == \"recipes\""), "Icons.app knows recipes");
        assert!(read("../yantrik-ui-kit/slint/app_color.slint").contains("id == \"recipes\""), "and it has a colour of its own");

        // The mind panel: drawn on it, its room kept, and its recipe rows open this screen.
        assert!(crate::mind_panel::shown_on(SCREEN));
        assert!(
            block.iter().any(|l| l.contains("width: root.window-maximized ? parent.width - root.mind-panel-reserve")),
            "maximized, the screen stops short of the mind panel"
        );
        let panel = &app[app.find("if root.mind-panel-shown : MindPanel {").expect("the panel")..];
        assert!(panel.contains(&format!("recipes-screen: {SCREEN};")), "the panel's recipe rows are links to this screen");
        assert!(panel.contains("RecipesState.show(id);"), "and open the recipe they name");
    }
}
