//! The one picker for every mind (#673): [Mind ▾] [Model ▾] [Effort ▾] [📎] in the Lens composer
//! and the mind panel's header (crates/yantrik-ui-kit/slint/model_picker.slint), and what it sends
//! with each message as turn options.
//!
//! - `choices`: the model and effort per mind, and the recent models, kept across restarts.
//! - `menu`: the menus' rows, the reasons a model cannot be used, the search (pure).
//! - `attach`: files handed over, with their provenance.
//! - `status`: "connected · <model>" only after a real call or a probe (pure).
//!
//! The choice is per mind and travels with every message, so a harness may take a different
//! model each turn; picking another before sending is how one message goes to another model.

pub mod attach;
pub mod choices;
pub mod menu;
pub mod status;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use yantrik_harness::protocol::{Attachment, TurnOptions};

use crate::ai_accounts;
use crate::app_context::AppContext;
use crate::bridge::CompanionBridge;
use crate::streaming::Speaker;
use crate::{App, PickerFile, PickerMind, PickerModel, PickerState};
use choices::Choices;
use menu::MindSeen;
use status::{Fix, Outcome, Probe};

static CHOICES: LazyLock<Mutex<Choices>> = LazyLock::new(|| Mutex::new(choices::load(&choices::path())));
static QUERY: Mutex<String> = Mutex::new(String::new());
static FILES: Mutex<Vec<Attachment>> = Mutex::new(Vec::new());
static BROWSE: Mutex<Option<PathBuf>> = Mutex::new(None);
static PROBES: LazyLock<Mutex<HashMap<String, Probe>>> = LazyLock::new(Default::default);
static PROBING: LazyLock<Mutex<std::collections::HashSet<String>>> = LazyLock::new(Default::default);

/// Settings, and the two sections the picker's fixes open.
const SETTINGS_SCREEN: i32 = 7;
const AI_SECTION: i32 = 1;
const HARNESSES_SECTION: i32 = 8;

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/root"))
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// What the person picked for a mind.
pub fn choice_of(mind: &str) -> choices::MindChoice {
    lock(&CHOICES).of(mind)
}

/// The model a mind answers with through the gateway, as the person picked it.
pub fn remembered_model(mind: &str) -> Option<String> {
    Some(lock(&CHOICES).of(mind).model).filter(|m| !m.is_empty())
}

/// Every mind the menu knows: the attached ones from the host, the rest from the Harnesses list.
fn minds_seen() -> Vec<MindSeen> {
    let home = home();
    let entries = crate::wire::harness::host().map(|h| h.list()).unwrap_or_default();
    let seen = |id: &str, name: &str, attached: bool, active: bool, state: String| MindSeen {
        id: id.to_string(),
        name: name.to_string(),
        attached,
        active,
        state,
        on_gateway: crate::provider_handoff::uses_gateway(&home, id),
        private_context: crate::provider_handoff::adapter_for(id).map(|a| a.private_context()).unwrap_or(true),
    };
    let mut out: Vec<MindSeen> = entries.iter().map(|e| seen(&e.id, &e.name, true, e.active, String::new())).collect();
    for row in crate::wire::harness::catalogue_for_describe().as_array().into_iter().flatten() {
        let id = row["id"].as_str().unwrap_or_default();
        if id.is_empty() || out.iter().any(|m| m.id == id) {
            continue;
        }
        let state = row["state"].as_str().unwrap_or("not installed").replace(['_', '-'], " ");
        out.push(seen(id, row["name"].as_str().unwrap_or(id), false, false, state));
    }
    out
}

fn active(minds: &[MindSeen]) -> Option<&MindSeen> {
    minds.iter().find(|m| m.active)
}

/// Who says a reply that begins now: the answering mind, and the model it answers with when the
/// desktop knows it (the one picked, for a mind on Yantrik models; what it reports, otherwise).
pub fn speaker() -> Speaker {
    let Some(host) = crate::wire::harness::host() else { return Speaker::default() };
    let entries = host.list();
    let Some(e) = entries.iter().find(|e| e.active) else { return Speaker::default() };
    let model = if crate::provider_handoff::uses_gateway(&home(), &e.id) {
        remembered_model(&e.id).map(|m| m.split_once('/').map(|(_, rest)| rest.to_string()).unwrap_or(m)).unwrap_or_default()
    } else {
        String::new()
    };
    Speaker { mind: e.name.clone(), model }
}

/// What travels with the next message to `mind`: the model (for a mind on Yantrik models), the
/// effort, and the files handed over, which are taken. `None` when nothing was chosen.
pub fn turn_options(mind: &str, via: &str) -> Option<TurnOptions> {
    let choice = lock(&CHOICES).of(mind);
    let on_gateway = crate::provider_handoff::uses_gateway(&home(), mind);
    let model = if on_gateway { choice.model.clone() } else { String::new() };
    // Effort only for a model that thinks, at a level it has; a mind with its own models is sent
    // the word to map as it can.
    let effort = match ai_accounts::catalogue().find(&choice.model) {
        Some(m) if on_gateway => m.caps.efforts.iter().find(|e| e.as_str() == choice.effort).map(|e| e.as_str().to_string()).unwrap_or_default(),
        _ if on_gateway => String::new(),
        _ => choice.effort.clone(),
    };
    let mut attachments: Vec<Attachment> = std::mem::take(&mut *lock(&FILES));
    for a in &mut attachments {
        a.via = via.to_string();
    }
    let options = TurnOptions { model, effort, attachments };
    (!options.is_empty()).then_some(options)
}

/// The chips above the words, after a message took the files: none left.
pub fn files_sent(ui: &slint::Weak<App>) {
    if let Some(ui) = ui.upgrade() {
        ui.global::<PickerState>().set_files(ModelRc::new(VecModel::from(files_rows())));
    }
}

/// A file into the next message. A sentence when it cannot be.
fn add_file(path: &std::path::Path) -> Result<(), String> {
    let mut files = lock(&FILES);
    if files.len() >= attach::MAX_FILES {
        return Err(format!("At most {} files go with one message.", attach::MAX_FILES));
    }
    let spent: u64 = files.iter().filter(|f| !f.content_b64.is_empty()).map(|f| f.size).sum();
    let mut left = attach::INLINE_TOTAL.saturating_sub(spent);
    let a = attach::read(path, "lens", &chrono::Utc::now().to_rfc3339(), &mut left)?;
    if files.iter().any(|f| f.path == a.path) {
        return Ok(());
    }
    tracing::info!(size = a.size, inline = !a.content_b64.is_empty(), "a file was handed over for the next message");
    files.push(a);
    Ok(())
}

/// Ask the model's account whether it answers, off the UI thread, at most one at a time per
/// account; the answer stands for a few minutes (status::PROBE_FRESH_SECS).
fn probe(bridge: &Arc<CompanionBridge>, model: &str) {
    let Some(m) = ai_accounts::catalogue().find(model).cloned() else { return };
    let account = m.account.clone();
    if !lock(&PROBING).insert(account.clone()) {
        return;
    }
    let bridge = bridge.clone();
    let _ = std::thread::Builder::new().name("model-probe".into()).spawn(move || {
        let accounts = ai_accounts::accounts();
        let outcome = match accounts.iter().find(|a| a.id == account) {
            None => Outcome::KeyMissing,
            Some(a) => {
                let saved = crate::wire::settings::ProviderStore::load().entries;
                let vault = |id: &str| ai_accounts::vault_value(&bridge, id);
                match ai_accounts::keys::resolve(a, &saved, &vault) {
                    Err(_) => Outcome::KeyMissing,
                    Ok(r) if ai_accounts::models::needs_no_listing(a, yantrik_ml::provider::pool::tiers::FREE_TIERS) => {
                        // No list to ask: the smallest request that needs the key, one token.
                        match crate::wire::provider_models::check_key(&r.base_url, r.key.as_deref().unwrap_or(""), &m.model) {
                            crate::wire::provider_models::KeyCheck::Works => Outcome::Ok,
                            crate::wire::provider_models::KeyCheck::Refused => Outcome::KeyRefused,
                            crate::wire::provider_models::KeyCheck::Unconfirmed(why) => Outcome::Unreachable(why),
                        }
                    }
                    Ok(r) => match crate::wire::provider_models::list_models(&r.base_url, r.key.as_deref(), if r.key.is_some() { "bearer" } else { "none" }) {
                        Ok(_) => Outcome::Ok,
                        Err(crate::wire::provider_models::ListError::KeyRefused) => Outcome::KeyRefused,
                        Err(e) => Outcome::Unreachable(e.to_string()),
                    },
                }
            }
        };
        lock(&PROBES).insert(account.clone(), Probe { at: now(), outcome });
        lock(&PROBING).remove(&account);
    });
}

/// The status of the mind answering now.
fn status_now(minds: &[MindSeen]) -> status::Status {
    let Some(mind) = active(minds) else {
        return status::Status { state: "not-set-up", words: "no mind is answering".into(), fix: Fix::None };
    };
    let model = remembered_model(&mind.id).unwrap_or_default();
    let account = model.split_once('/').map(|(a, _)| a.to_string()).unwrap_or_default();
    let own = crate::agents::store().read(|s| {
        let a = s.agent(&crate::agents::feed::main_agent(&mind.id))?;
        let t = a.turns.iter().rev().find(|t| t.ended.is_some() && t.ok == Some(true))?;
        Some((t.ended.unwrap_or_default() as i64, a.meta.model.clone()))
    });
    let input = status::Input {
        now: now(),
        on_gateway: mind.on_gateway,
        model,
        last_call: crate::gateway::log().last_for(&mind.id),
        probe: lock(&PROBES).get(&account).cloned(),
        gateway_down: crate::gateway::state().err(),
        own_answered_at: own.as_ref().map(|(at, _)| *at),
        own_model: own.map(|(_, m)| m).unwrap_or_default(),
    };
    status::judge(&input)
}

/// The status for the mind panel's Now line: (state, words).
pub fn panel_status() -> (String, String) {
    let s = status_now(&minds_seen());
    (s.state.to_string(), s.words)
}

fn files_rows() -> Vec<PickerFile> {
    lock(&FILES)
        .iter()
        .map(|f| PickerFile { path: f.path.as_str().into(), name: f.name.as_str().into(), detail: attach::size_words(f.size).into(), folder: false })
        .collect()
}

/// Draw everything from what is known now. Cheap enough for a timer: no network, no file listing.
fn render(ui: &App, bridge: &Arc<CompanionBridge>) {
    let g = ui.global::<PickerState>();
    let minds = minds_seen();
    let rows: Vec<PickerMind> = menu::mind_rows(&minds)
        .into_iter()
        .map(|r| PickerMind { id: r.id.into(), name: r.name.into(), words: r.words.into(), selectable: r.selectable, current: r.current })
        .collect();
    g.set_minds(ModelRc::new(VecModel::from(rows)));
    let Some(mind) = active(&minds).cloned() else {
        g.set_mind_label("No mind".into());
        g.set_model_label("—".into());
        g.set_model_enabled(false);
        return;
    };
    g.set_mind_label(mind.name.as_str().into());
    let catalogue = ai_accounts::catalogue();
    let accounts = ai_accounts::accounts();
    let consent = ai_accounts::consent::load(&ai_accounts::consent::path());
    let choice = lock(&CHOICES).of(&mind.id);
    let picked = catalogue.find(&choice.model).filter(|_| mind.on_gateway);
    g.set_model_label(match (mind.on_gateway, picked) {
        (false, _) => "its own models".into(),
        (true, Some(m)) => {
            let account = accounts.iter().find(|a| a.id == m.account).map(|a| a.name.clone()).unwrap_or_default();
            format!("{} · {}", m.name, account).into()
        }
        (true, None) => "Pick a model".into(),
    });
    g.set_model_enabled(true);
    let efforts: Vec<SharedString> = match picked {
        Some(m) => m.caps.efforts.iter().map(|e| SharedString::from(e.as_str())).collect(),
        // Its own models: the word is sent for it to map as it can.
        None if !mind.on_gateway => Vec::new(),
        None => Vec::new(),
    };
    g.set_efforts(ModelRc::new(VecModel::from(efforts)));
    g.set_effort(choice.effort.as_str().into());
    let query = lock(&QUERY).clone();
    let recent = lock(&CHOICES).recent.clone();
    let rows: Vec<PickerModel> = menu::model_rows(&mind, &catalogue, &accounts, &consent, crate::private_mode::is_on(), &choice.model, &recent, &query)
        .into_iter()
        .map(|r| PickerModel {
            kind: r.kind.into(),
            id: r.id.into(),
            name: r.name.into(),
            detail: r.detail.into(),
            disabled: r.disabled,
            reason: r.reason.into(),
            current: r.current,
        })
        .collect();
    g.set_models(ModelRc::new(VecModel::from(rows)));
    g.set_files(ModelRc::new(VecModel::from(files_rows())));
    let s = status_now(&minds);
    if s.state == "checking" {
        probe(bridge, &choice.model);
    }
    g.set_status_state(s.state.into());
    g.set_status_words(s.words.into());
    g.set_fix_label(s.fix.label().into());
}

fn browse(ui: &App, asked: &str) {
    let dir = attach::resolve_dir(asked, &home());
    let entries: Vec<PickerFile> = attach::list(&dir)
        .unwrap_or_default()
        .into_iter()
        .map(|e| PickerFile {
            path: e.path.display().to_string().into(),
            name: e.name.into(),
            detail: if e.folder { "folder".into() } else { attach::size_words(e.size).into() },
            folder: e.folder,
        })
        .collect();
    let g = ui.global::<PickerState>();
    g.set_browse_path(dir.display().to_string().into());
    g.set_browse_entries(ModelRc::new(VecModel::from(entries)));
    *lock(&BROWSE) = Some(dir);
}

fn open_settings(ui: &App, section: i32) {
    ui.set_lens_open(false);
    ui.set_settings_category(section);
    ui.set_current_screen(SETTINGS_SCREEN);
    ui.invoke_navigate(SETTINGS_SCREEN);
}

fn save_choices() {
    if let Err(e) = choices::save(&choices::path(), &lock(&CHOICES)) {
        tracing::warn!(error = %e, "the picker's choices were not saved");
    }
}

pub fn wire(ui: &App, ctx: &AppContext) {
    let bridge = ctx.bridge.clone();
    let g = ui.global::<PickerState>();

    let (w, b) = (ui.as_weak(), bridge.clone());
    g.on_opened(move |_| {
        if let Some(ui) = w.upgrade() {
            *lock(&QUERY) = String::new();
            render(&ui, &b);
        }
    });
    let (w, b) = (ui.as_weak(), bridge.clone());
    g.on_search(move |q| {
        *lock(&QUERY) = q.to_string();
        if let Some(ui) = w.upgrade() {
            render(&ui, &b);
        }
    });
    let (w, b) = (ui.as_weak(), bridge.clone());
    g.on_choose_mind(move |id| {
        let Some(ui) = w.upgrade() else { return };
        let attached = crate::wire::harness::host().is_some_and(|h| h.list().iter().any(|e| e.id == id.as_str()));
        if attached {
            // The same path every other place that chooses a mind takes: remembered, published.
            ui.invoke_use_harness(id);
        } else {
            // Not here yet: the row that installs or starts it is the one press that helps.
            open_settings(&ui, HARNESSES_SECTION);
        }
        render(&ui, &b);
    });
    let (w, b) = (ui.as_weak(), bridge.clone());
    g.on_choose_model(move |id| {
        let Some(ui) = w.upgrade() else { return };
        let Some(mind) = crate::wire::harness::host().map(|h| h.active_id()) else { return };
        {
            let mut c = lock(&CHOICES);
            c.pick_model(&mind, &id);
            // A level the new model does not have is dropped rather than sent.
            let offered = ai_accounts::catalogue().find(&id).map(|m| m.caps.efforts.clone()).unwrap_or_default();
            if !offered.iter().any(|e| e.as_str() == c.of(&mind).effort) {
                let first = offered.get(1).or(offered.first()).map(|e| e.as_str()).unwrap_or("");
                c.pick_effort(&mind, first);
            }
        }
        // A mind on Yantrik models asks for `picked`, so this is its next call's model.
        save_choices();
        render(&ui, &b);
    });
    let (w, b) = (ui.as_weak(), bridge.clone());
    g.on_choose_effort(move |e| {
        let Some(ui) = w.upgrade() else { return };
        if let Some(mind) = crate::wire::harness::host().map(|h| h.active_id()) {
            lock(&CHOICES).pick_effort(&mind, &e);
            save_choices();
        }
        render(&ui, &b);
    });
    let w = ui.as_weak();
    g.on_browse(move |p| {
        if let Some(ui) = w.upgrade() {
            browse(&ui, &p);
        }
    });
    let (w, b) = (ui.as_weak(), bridge.clone());
    g.on_attach(move |p| {
        let Some(ui) = w.upgrade() else { return };
        if let Err(e) = add_file(std::path::Path::new(p.as_str())) {
            crate::streaming::say(&ui.as_weak(), None, "desktop", &format!("That file was not handed over: {e}"));
        }
        render(&ui, &b);
    });
    let (w, b) = (ui.as_weak(), bridge.clone());
    g.on_remove_file(move |p| {
        lock(&FILES).retain(|f| f.path != p.as_str());
        if let Some(ui) = w.upgrade() {
            render(&ui, &b);
        }
    });
    let (w, b) = (ui.as_weak(), bridge.clone());
    g.on_fix(move || {
        let Some(ui) = w.upgrade() else { return };
        let s = status_now(&minds_seen());
        match s.fix {
            Fix::SetUp => {
                let mind = crate::wire::harness::host().map(|h| h.active_id()).unwrap_or_default();
                open_settings(&ui, HARNESSES_SECTION);
                ui.invoke_plan_provider(mind.into(), crate::provider_handoff::GATEWAY_ID.into());
            }
            Fix::AddKey | Fix::Allow => open_settings(&ui, AI_SECTION),
            Fix::Retry => lock(&PROBES).clear(),
            Fix::None => {}
        }
        render(&ui, &b);
    });

    // A file dragged from another program onto the window: the ask bar takes it.
    {
        use slint::winit_030::{winit::event::WindowEvent, EventResult, WinitWindowAccessor};
        let (w, b) = (ui.as_weak(), bridge.clone());
        ui.window().on_winit_window_event(move |_, event| {
            let Some(ui) = w.upgrade() else { return EventResult::Propagate };
            match event {
                WindowEvent::HoveredFile(_) => ui.global::<PickerState>().set_drop_hover(true),
                WindowEvent::HoveredFileCancelled => ui.global::<PickerState>().set_drop_hover(false),
                WindowEvent::DroppedFile(path) => {
                    ui.global::<PickerState>().set_drop_hover(false);
                    if let Err(e) = add_file(path) {
                        tracing::info!(error = %e, "a dropped file was not handed over");
                    }
                    ui.set_lens_open(true);
                    render(&ui, &b);
                }
                _ => {}
            }
            EventResult::Propagate
        });
    }

    // The status moves on its own (a probe answers, a call ends): drawn again every few seconds.
    let (w, b) = (ui.as_weak(), bridge.clone());
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, Duration::from_secs(3), move || {
        if let Some(ui) = w.upgrade() {
            render(&ui, &b);
        }
    });
    std::mem::forget(timer);
    let w = ui.as_weak();
    ai_accounts::on_change(move || {
        let w = w.clone();
        let b = bridge.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = w.upgrade() {
                render(&ui, &b);
            }
        });
    });
    browse(ui, "");
    render(ui, &ctx.bridge);
}
