//! "What runs on what", on screen: the map at the top of Settings → AI & Intelligence, the line
//! under "Answering" in the Minds panel, each harness row's "Runs on", and the line under the chat
//! composer. All of them come from one `runs_on::resolve`, so they cannot disagree.
//!
//! Published whenever the minds change (wire::harness::publish) and whenever the companion's own
//! address is resolved again (wire::ai_status::refresh), and whenever the pick changes in the
//! picker: a mind on the gateway runs on the pick (picker::wire).

use std::sync::Mutex;

use crate::runs_on::{self, GatewayFact, MindFact, RunsOn};
use crate::App;
use yantrik_ui_slint::{InUseRow, RunsOnRow};

/// The addresses of the providers saved in Settings, kept up to date by push_providers_to_ui, so
/// the Providers list can tell what is in use but not saved without reading the file again.
static SAVED: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// What every mind runs on, right now.
pub(crate) fn current() -> Vec<RunsOn> {
    let minds: Vec<MindFact> = super::harness::host()
        .map(|h| h.list())
        .unwrap_or_default()
        .into_iter()
        .map(|e| MindFact { gateway: gateway_fact(&e.id, e.detail.as_deref()), id: e.id, name: e.name, detail: e.detail, answering: e.active, builtin: e.builtin })
        .collect();
    let companion = super::ai_status::companion();
    runs_on::resolve(&minds, companion.as_ref())
}

/// For a mind wired to this desktop's gateway (its handoff marker names the gateway, and its own
/// config still points there): where the gateway sends its next call, by the gateway's own routing
/// over the model that config asks for — `picked`, or one set there since (`gateway::route_for`).
/// `None` for a mind with its own provider, or whose config was pointed elsewhere after the marker.
fn gateway_fact(harness: &str, detail: Option<&str>) -> Option<GatewayFact> {
    let home = super::harness_provider::home_dir();
    let marker = crate::provider_handoff::marker(&home, harness)?;
    if marker.provider_id != crate::provider_handoff::GATEWAY_ID || !crate::provider_handoff::still_on_gateway(harness, detail) {
        return None;
    }
    let asked = crate::provider_handoff::adapter_for(harness)?.asks_gateway_for(&home)?;
    Some(match crate::gateway::route_for(harness, &asked) {
        Ok(routed) => {
            let t = routed.target;
            let provider_kind =
                crate::ai_accounts::accounts().into_iter().find(|a| a.id == t.account).map(|a| a.provider_type).unwrap_or_default();
            GatewayFact::Sends { base_url: t.base_url, provider_kind, model: t.model, lane: routed.lane_for.is_some() }
        }
        Err(r) if r.message == yantrik_gateway::server::NO_PICK => GatewayFact::NoPick,
        Err(r) => GatewayFact::Refused { code: r.code.to_string(), message: r.message },
    })
}

/// The saved providers changed: remember their addresses and show the lists again.
pub(crate) fn set_saved(ui: &App, urls: Vec<String>) {
    *SAVED.lock().unwrap_or_else(|e| e.into_inner()) = urls;
    publish(ui);
}

/// The Minds rows' line, in the map's plain words: "Runs on Ollama Cloud · deepseek-v4.1-flash".
/// Empty for a mind that is not attached, which says nothing about itself. The exact line is the
/// row's Details ([`row_details`]).
pub(crate) fn row_line(rows: &[RunsOn], id: &str) -> String {
    rows.iter().find(|r| r.id == id && !r.builtin).map(|r| format!("Runs on {}", r.plain_runs_on())).unwrap_or_default()
}

/// The exact facts behind [`row_line`], for the row's collapsed Details.
pub(crate) fn row_details(rows: &[RunsOn], id: &str) -> String {
    rows.iter().find(|r| r.id == id && !r.builtin).map(|r| r.details()).unwrap_or_default()
}

pub(crate) fn publish(ui: &App) {
    let rows = current();
    let answering = rows
        .iter()
        .find(|r| r.answering)
        .map(|r| r.plain_runs_on())
        .unwrap_or_default();
    ui.set_runs_on_summary(runs_on::summary(&rows).into());
    ui.set_runs_on_answering(answering.into());
    // The chat composer's line: where words typed to the answering mind go, or "" when that is
    // not known (the composer then says nothing rather than guess).
    let destination = rows.iter().find(|r| r.answering).map(|r| r.destination()).unwrap_or_default();
    if ui.get_lens_destination() != destination.as_str() {
        ui.set_lens_destination(destination.into());
    }
    let model: Vec<RunsOnRow> = rows
        .iter()
        .map(|r| RunsOnRow {
            name: r.name.as_str().into(),
            state: r.state().into(),
            answering: r.answering,
            runs_on: r.plain_runs_on().into(),
            source: r.plain_source().into(),
            details: r.details().into(),
        })
        .collect();
    if let Some(m) = crate::models::changed(ui.get_runs_on_rows(), model) {
        ui.set_runs_on_rows(m);
    }

    let saved = SAVED.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let companion = super::ai_status::companion();
    let in_use: Vec<InUseRow> = runs_on::in_use_not_saved(&rows, companion.as_ref(), &saved)
        .into_iter()
        .map(|u| InUseRow {
            label: u.label.into(),
            used_by: format!("Used by {}", u.used_by.join(", ")).into(),
            where_set: u.where_set.into(),
            models: u.models.join(", ").into(),
            preset: u.preset.into(),
            base_url: u.base_url.into(),
            model: u.model.into(),
        })
        .collect();
    if let Some(m) = crate::models::changed(ui.get_in_use_providers(), in_use) {
        ui.set_in_use_providers(m);
    }
}
