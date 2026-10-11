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

use crate::agents::catalog::Catalog;
use crate::app_context::AppContext;
use crate::bridge::CompanionHandle;
use crate::{
    App, RecipeRoleData, RecipeRowData, RecipeSeatData, RecipeStageData, RecipeStepData, RecipeTabData,
    RecipesState,
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

/// The seats a person has filled on the Recipes screen, before starting: `(recipe id, seat name)`
/// to the chosen role's catalog id. Kept here — not in the companion — so it lives only as long as
/// the screen holds it, and Start sends it as the run's inputs. It is UI-local because the rows are
/// updated in place, so a seat filled must survive another recipe moving on.
pub type SeatPicks = HashMap<(String, String), String>;

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
    /// The seats the person has filled for a formation's definition (#194).
    seat_picks: SeatPicks,
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
        seat_picks: SeatPicks::default(),
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
            let picks = state.borrow().seat_picks.clone();
            let catalog = Catalog::load();
            match start_request(&crate::recipes::snapshot().views, &id, &text, &catalog, &picks) {
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
    // A seat filled from the catalog (#194): recorded against the recipe and its row redrawn, so
    // the seat shows the role chosen. Start then sends it as that seat's input.
    g.on_choose_seat({
        let (weak, state) = (weak.clone(), state.clone());
        move |id, seat, role| {
            let Some(ui) = weak.upgrade() else { return };
            let mut st = state.borrow_mut();
            st.seat_picks.insert((id.to_string(), seat.to_string()), role.to_string());
            st.local += 1;
            draw(&ui, &mut *st);
        }
    });
    g.on_dismiss_notice({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                ui.global::<RecipesState>().set_notice("".into());
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

/// What the Recipes screen's Start asks the worker for: the formation's definition, its one
/// input given as `text`, and every seat the person filled from the catalog (#194) as that seat's
/// input. A seat left alone keeps the template's own role. Refused here for anything the screen
/// cannot start.
pub fn start_request(
    views: &[RecipeView],
    id: &str,
    text: &str,
    catalog: &Catalog,
    picks: &SeatPicks,
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
    // The seats the person filled: the chosen role travels as that seat's input, so the run seats
    // who they picked. A seat they left alone is not sent, and keeps the template's role. A pick
    // the catalog no longer resolves (a role file removed since) is not sent either, so the seat
    // falls back to the template's role — the same fallback seats_of shows on screen.
    for seat in view.inputs.iter().filter(|i| i.default.is_some()) {
        if let Some(role_id) = picks.get(&(id.to_string(), seat.name.clone())) {
            if catalog.find(role_id).is_some() {
                inputs.insert(seat.name.clone(), serde_json::Value::String(role_id.clone()));
            }
        }
    }
    Ok((view.id.clone(), inputs))
}

/// A formation's built-in definition with exactly one input a run must be given: the screen asks
/// for it and starts it.
fn can_start(v: &RecipeView) -> bool {
    v.template && v.formation && v.inputs.iter().filter(|i| i.default.is_none()).count() == 1
}

/// A startable formation's seats (#194): each input whose value names a catalog role, offered for
/// the person to re-seat before starting. The role shown is the one they picked, else the
/// template's default. An input that names no catalog role (a writers' room's cast of voices) is
/// not a seat, and is left to the template.
pub fn seats_of(v: &RecipeView, catalog: &Catalog, picks: &SeatPicks) -> Vec<RecipeSeatData> {
    if !can_start(v) {
        return Vec::new();
    }
    v.inputs
        .iter()
        .filter_map(|input| {
            let default = input.default.as_deref()?;
            // The person's pick, else the template's seat; a pick the catalog has lost (a role
            // file removed since they made it) falls back to the template's, never to no role.
            let picked = picks.get(&(v.id.clone(), input.name.clone())).map(String::as_str);
            let role = picked.and_then(|p| catalog.find(p)).or_else(|| catalog.find(default))?;
            Some(RecipeSeatData {
                name: input.name.as_str().into(),
                label: seat_label(&input.name).into(),
                role: role.name.as_str().into(),
                role_id: role.id.as_str().into(),
            })
        })
        .collect()
}

/// A seat's name as a person reads it: "seat_1" becomes "Seat 1", "chair" becomes "Chair".
fn seat_label(name: &str) -> String {
    name.split('_').filter(|p| !p.is_empty()).map(capitalize).collect::<Vec<_>>().join(" ")
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// The catalog's roles, as the choices every seat offers (#194).
fn roles_of(catalog: &Catalog) -> Vec<RecipeRoleData> {
    catalog.roles.iter().map(|r| RecipeRoleData { id: r.id.as_str().into(), name: r.name.as_str().into() }).collect()
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
    let mut rows: Vec<RecipeRowData> = shown.iter().map(|v| row_of(v, now)).collect();
    // A startable formation offers its seats as choices from the catalog (#194). Read the catalog
    // once, and only when such a formation is actually shown; keep each model that did not change,
    // so an opened chooser is not folded shut by another recipe taking a step.
    if rows.iter().any(|r| r.can_start) {
        let catalog = Catalog::load();
        if let Some(model) = crate::models::changed(g.get_roles(), roles_of(&catalog)) {
            g.set_roles(model);
        }
        for (row, v) in rows.iter_mut().zip(&shown) {
            if row.can_start {
                if let Some(model) = crate::models::changed(row.seats.clone(), seats_of(v, &catalog, &st.seat_picks)) {
                    row.seats = model;
                }
            }
        }
    }
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
        g.set_notice(outcome.text.into());
        g.set_notice_ok(outcome.ok);
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
        // Filled in draw() for a startable formation, from the catalog and the person's picks
        // (#194); empty for every other row.
        seats: ModelRc::default(),
        needs_you: v.needs_you.is_some(),
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
    }
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

/// The row's line for inputs nothing sets: what the recipe needs, said as a precondition.
///
/// It read "{{person}}, {{date}} have no value", drawn like an error. Nothing stops a recipe
/// with an unset input from running — the placeholder reaches the tool as the literal
/// `{{name}}` (#88) — so the line says that too, rather than promising a run is held back.
fn unbound_note(names: &[String]) -> String {
    let mut shown: Vec<String> = names.iter().take(3).map(|n| format!("{{{{{n}}}}}")).collect();
    let more = names.len().saturating_sub(3);
    if more > 0 {
        shown.push(format!("{more} more"));
    }
    let list = match shown.split_last() {
        None => return String::new(),
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
    };
    format!("Needs {list} \u{2014} sent as written until set")
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

    /// Unset inputs read as what the recipe needs, never as a failure, and never as a promise
    /// that it will not run: it does run, with the placeholder as written (#88).
    #[test]
    fn unset_inputs_read_as_a_precondition() {
        let names = |n: &[&str]| n.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(unbound_note(&[]), "");
        assert_eq!(unbound_note(&names(&["person", "date"])), "Needs {{person}} and {{date}} — sent as written until set");
        assert_eq!(
            unbound_note(&names(&["a", "b", "c", "d", "e"])),
            "Needs {{a}}, {{b}}, {{c}} and 2 more — sent as written until set"
        );
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
        assert_eq!(row.unbound, "Needs {{subfolder}} — sent as written until set");
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
        let catalog = Catalog::from_layers(&crate::agents::catalog::SHIPPED, &[]);
        let no_picks = SeatPicks::default();
        let (id, inputs) = start_request(&views, formations::COUNCIL, "  Should we ship?  ", &catalog, &no_picks).unwrap();
        assert_eq!((id.as_str(), inputs["question"].as_str()), (formations::COUNCIL, Some("Should we ship?")));
        assert!(start_request(&views, formations::COUNCIL, " ", &catalog, &no_picks).unwrap_err().contains("needs the question"));
        assert!(start_request(&views, "rcp_council", "x", &catalog, &no_picks).unwrap_err().contains("is not started from here"));
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

    /// A startable formation's definition offers its seats as choices from the catalog (#194):
    /// the Council's four seats draw with the roles that would sit in them, a role pressed into a
    /// seat is what Start sends — and only for that seat — and an input that names no catalog
    /// role (the writers' room's cast of voices) is no seat.
    #[test]
    fn a_formation_definition_offers_its_seats_as_catalog_choices() {
        use yantrik_companion::recipe_templates::{self, formations};

        let catalog = Catalog::from_layers(&crate::agents::catalog::SHIPPED, &[]);
        let definition = |id: &str| -> RecipeView {
            let template = recipe_templates::get_template(id).unwrap();
            let recipe = Recipe {
                id: id.into(),
                name: template.name.into(),
                description: String::new(),
                status: RecipeStatus::Pending,
                current_step: 0,
                created_at: 0.0,
                updated_at: 0.0,
                enabled: true,
                error_message: None,
            };
            let steps: Vec<StoredStep> = (template.steps)()
                .into_iter()
                .enumerate()
                .map(|(i, step)| StoredStep { step_index: i, step, status: "pending".into(), result: None })
                .collect();
            recipe_view::view(&recipe, &steps, &Default::default())
        };
        let council = definition(formations::COUNCIL);

        let seats: Vec<(String, String, String)> = seats_of(&council, &catalog, &SeatPicks::default())
            .iter()
            .map(|s| (s.label.to_string(), s.role.to_string(), s.role_id.to_string()))
            .collect();
        assert_eq!(
            seats,
            [
                ("Seat 1".into(), "Researcher".into(), "researcher".into()),
                ("Seat 2".into(), "Red team".into(), "red-team".into()),
                ("Seat 3".into(), "Planner".into(), "planner".into()),
                ("Chair".into(), "Chair".into(), "chair".into())
            ]
        );
        let offered: Vec<String> = roles_of(&catalog).iter().map(|r| r.id.to_string()).collect();
        assert!(
            offered.iter().any(|id| id == "coder") && offered.iter().any(|id| id == "writer"),
            "every catalog role is a choice: {offered:?}"
        );

        // A role pressed into a seat: the seat shows it, and Start sends it — only that seat, so
        // the seats left alone keep the template's own roles.
        let mut picks = SeatPicks::default();
        picks.insert((formations::COUNCIL.to_string(), "seat_2".to_string()), "coder".to_string());
        assert_eq!(seats_of(&council, &catalog, &picks)[1].role, "Coder");
        let views = vec![council.clone()];
        let (_, inputs) = start_request(&views, formations::COUNCIL, "Ship?", &catalog, &picks).unwrap();
        assert_eq!(inputs["question"].as_str(), Some("Ship?"));
        assert_eq!(inputs["seat_2"].as_str(), Some("coder"), "the seat the person filled");
        assert!(
            inputs.get("seat_1").is_none() && inputs.get("chair").is_none(),
            "a seat left alone stays the template's"
        );

        // A pick the catalog has lost falls back to the template's seat, never to no role — on
        // screen and in what Start sends, so the two never disagree.
        let mut stale = SeatPicks::default();
        stale.insert((formations::COUNCIL.to_string(), "seat_1".to_string()), "no-such-role".to_string());
        assert_eq!(seats_of(&council, &catalog, &stale)[0].role_id, "researcher");
        let (_, stale_inputs) = start_request(&views, formations::COUNCIL, "Ship?", &catalog, &stale).unwrap();
        assert!(stale_inputs.get("seat_1").is_none(), "a stale pick is not sent; the seat keeps the template's role");

        // The writers' room's cast is three names, not a catalog role: no seat is offered, and
        // Start leaves the cast to the template.
        let room = definition(formations::WRITERS_ROOM);
        assert!(seats_of(&room, &catalog, &SeatPicks::default()).is_empty());
        let (_, inputs) = start_request(&[room], formations::WRITERS_ROOM, "The larder, at midnight", &catalog, &SeatPicks::default()).unwrap();
        assert!(inputs.get("voice_1").is_none());

        // The screen draws the chooser: the seats on the definition's row, the catalog's roles as
        // the choices, and a press carrying the seat and the role back to the shell.
        let slint = read("../yantrik-ui-slint/ui/recipes.slint");
        assert!(slint.contains("for seat in root.recipe.seats : SeatRow"), "the definition draws its seats");
        assert!(slint.contains("for role in RecipesState.roles : YButton"), "an opened seat offers the catalog's roles");
        assert!(slint.contains("RecipesState.choose-seat(root.recipe-id, root.seat.name, role.id);"), "a role pressed is the seat's pick");
    }
}
