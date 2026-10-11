//! The shell's view of the AI provider catalogue.
//!
//! `yantrik_ml::KNOWN_PROVIDERS` is the one list of providers. Settings' preset
//! grid, first-boot onboarding and the installer all read it through here, so
//! none of them keeps a list of its own. Before, each did, and they disagreed:
//! onboarding offered "google", which no other list knew, so a person who
//! picked Google at first boot had nothing saved.

use slint::{ModelRc, VecModel};
use yantrik_ml::{ProviderDescriptor, ProviderKind};

use crate::{App, ProviderPresetData, ProviderPresetGroup, ProviderPresetRow};

/// Cells in one row of Settings' preset grid.
const ROW_WIDTH: usize = 5;

/// What a model is called when the catalogue names none: the companion's own
/// placeholder for "whatever the endpoint serves by default".
const NO_MODEL: &str = "default";

/// Settings' grid sections, in the order they are shown.
const GROUPS: [(ProviderKind, &str); 3] = [
    (ProviderKind::Cloud, "CLOUD"),
    (ProviderKind::Aggregator, "AGGREGATORS & BUDGET"),
    (ProviderKind::Local, "LOCAL INFERENCE"),
];

/// A known provider's display name and OpenAI-compatible base URL, or
/// `("Custom", "")` for anything else. The shell talks chat completions to
/// every provider, so the URL is the OpenAI-compatible one.
pub(crate) fn provider_preset(id: &str) -> (&'static str, &'static str) {
    match ProviderDescriptor::by_id(id) {
        Some(p) => (p.display_name, p.openai_base_url()),
        None => ("Custom", ""),
    }
}

/// The model a new setup of this provider starts on.
pub(crate) fn default_model_for(id: &str) -> &'static str {
    ProviderDescriptor::by_id(id)
        .map(|p| p.default_model)
        .filter(|m| !m.is_empty())
        .unwrap_or(NO_MODEL)
}

/// The auth type providers.yaml records, for the OpenAI-compatible endpoint
/// the shell uses. Anthropic's compatibility layer still takes `x-api-key`;
/// Gemini's takes a bearer token, unlike its native API's `?key=`. Unknown
/// (custom) providers are assumed bearer.
pub(crate) fn auth_type_for(id: &str) -> &'static str {
    use yantrik_ml::AuthScheme;
    match ProviderDescriptor::by_id(id).map(|p| p.auth_scheme) {
        Some(AuthScheme::XApiKey) => "x-api-key",
        Some(AuthScheme::None) => "none",
        Some(AuthScheme::Bearer | AuthScheme::QueryParam) | None => "bearer",
    }
}

/// Providers served by a runtime the user hosts, whose preset endpoint is a
/// localhost guess rather than a fixed vendor URL.
pub(crate) fn is_local_runtime(id: &str) -> bool {
    ProviderDescriptor::by_id(id).is_some_and(|p| p.kind == ProviderKind::Local)
}

fn group_of(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::Cloud => "cloud",
        ProviderKind::Aggregator => "aggregator",
        ProviderKind::Local => "local",
    }
}

fn preset_data(p: &ProviderDescriptor) -> ProviderPresetData {
    ProviderPresetData {
        id: p.id.into(),
        name: p.display_name.into(),
        label: p.short_name.into(),
        url: p.openai_base_url().into(),
        placeholder: p.key_placeholder.into(),
        group: group_of(p.kind).into(),
    }
}

/// Settings' preset grid: each kind's providers in catalogue order, five to a
/// row, the last row padded with blank cells so every button keeps its width.
pub(crate) fn settings_preset_groups() -> Vec<ProviderPresetGroup> {
    GROUPS
        .iter()
        .filter_map(|(kind, title)| {
            let cells: Vec<ProviderPresetData> = ProviderDescriptor::settings_providers()
                .filter(|p| p.kind == *kind)
                .map(preset_data)
                .collect();
            if cells.is_empty() {
                return None;
            }
            let rows: Vec<ProviderPresetRow> = cells
                .chunks(ROW_WIDTH)
                .map(|chunk| {
                    let mut items = chunk.to_vec();
                    items.resize(ROW_WIDTH, ProviderPresetData::default());
                    ProviderPresetRow { items: ModelRc::new(VecModel::from(items)) }
                })
                .collect();
            Some(ProviderPresetGroup {
                title: (*title).into(),
                rows: ModelRc::new(VecModel::from(rows)),
            })
        })
        .collect()
}

/// Onboarding's cloud cards: the first-boot tier, minus local runtimes (the
/// local path has its own screen).
pub(crate) fn onboarding_cloud_presets() -> Vec<ProviderPresetData> {
    ProviderDescriptor::onboarding_providers()
        .into_iter()
        .filter(|p| p.kind != ProviderKind::Local)
        .map(preset_data)
        .collect()
}

/// Push both preset models to the UI.
pub fn push_presets(ui: &App) {
    ui.set_settings_provider_presets(ModelRc::new(VecModel::from(settings_preset_groups())));
    ui.set_onboard_ai_cloud_presets(ModelRc::new(VecModel::from(onboarding_cloud_presets())));
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::Model;
    use yantrik_ml::{SetupTier, KNOWN_PROVIDERS};

    fn grid_ids() -> Vec<String> {
        let mut ids = Vec::new();
        for group in settings_preset_groups() {
            for row in group.rows.iter() {
                assert_eq!(row.items.row_count(), ROW_WIDTH, "{}: rows are padded", group.title);
                ids.extend(row.items.iter().map(|p| p.id.to_string()).filter(|id| !id.is_empty()));
            }
        }
        ids
    }

    #[test]
    fn the_settings_grid_has_one_cell_per_shown_provider() {
        let ids = grid_ids();
        let shown: Vec<&str> = KNOWN_PROVIDERS
            .iter()
            .filter(|p| p.setup_tier != SetupTier::Expert)
            .map(|p| p.id)
            .collect();
        assert_eq!(ids.len(), shown.len(), "{ids:?}");
        for id in shown {
            assert_eq!(ids.iter().filter(|i| *i == id).count(), 1, "{id}");
        }
        assert!(ids.iter().any(|i| i == "nvidia-nim"));
    }

    #[test]
    fn the_grid_keeps_its_sections_in_order() {
        let titles: Vec<String> = settings_preset_groups().iter().map(|g| g.title.to_string()).collect();
        assert_eq!(titles, ["CLOUD", "AGGREGATORS & BUDGET", "LOCAL INFERENCE"]);
    }

    #[test]
    fn onboarding_offers_the_first_boot_clouds_with_their_key_hints() {
        let cards = onboarding_cloud_presets();
        let ids: Vec<&str> = cards.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["openai", "anthropic", "gemini", "deepseek", "nvidia-nim", "openrouter"]);
        let nim = cards.iter().find(|c| c.id == "nvidia-nim").unwrap();
        assert_eq!(nim.url, "https://integrate.api.nvidia.com/v1");
        assert_eq!(nim.placeholder, "nvapi-…");
        // Every card is one the rest of the shell can save.
        for c in &cards {
            assert_ne!(provider_preset(&c.id).1, "", "{}", c.id);
        }
    }

    #[test]
    fn presets_resolve_through_the_catalogue() {
        assert_eq!(provider_preset("gemini"), ("Google Gemini", "https://generativelanguage.googleapis.com/v1beta/openai"));
        assert_eq!(provider_preset("anthropic"), ("Anthropic", "https://api.anthropic.com/v1"));
        assert_eq!(provider_preset("nvidia-nim"), ("NVIDIA NIM", "https://integrate.api.nvidia.com/v1"));
        assert_eq!(provider_preset("custom"), ("Custom", ""));
        assert_eq!(provider_preset("google"), ("Custom", ""), "Gemini's id is gemini");
    }

    #[test]
    fn auth_and_models_come_from_the_catalogue() {
        assert_eq!(auth_type_for("anthropic"), "x-api-key");
        assert_eq!(auth_type_for("gemini"), "bearer");
        assert_eq!(auth_type_for("nvidia-nim"), "bearer");
        assert_eq!(auth_type_for("ollama-cloud"), "bearer");
        for local in ["ollama", "llamacpp", "lmstudio", "vllm"] {
            assert_eq!(auth_type_for(local), "none", "{local}");
            assert!(is_local_runtime(local), "{local}");
        }
        assert!(!is_local_runtime("ollama-cloud"));
        assert_eq!(auth_type_for("custom"), "bearer");
        assert_eq!(default_model_for("anthropic"), "claude-sonnet-5-5");
        assert_eq!(default_model_for("baidu"), NO_MODEL);
        assert_eq!(default_model_for("custom"), NO_MODEL);
    }
}
