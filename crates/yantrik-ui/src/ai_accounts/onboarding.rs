//! What the first-run AI step calls (design/onboarding-one-model-one-memory-2026-10-10.md, "The
//! flow"; design/experience-final-2026-10-10.md, "First run"). The screens live with the
//! onboarding UI; these are the decisions they record, each saved at once and each the same one
//! Settings → AI accounts makes later.
//!
//! 1. "Who should do the thinking?": the provider is saved as now (`wire::ai_onboarding`), then
//!    [`choose_model`] makes the model the one pick for every mind.
//! 2. "What your minds share": [`share_line`] is the pre-checked line; [`agree_to_share`] records
//!    the box. "Keep personal things on this computer instead": [`private_lane_choices`] lists
//!    the local models and [`keep_personal_things_on`] makes one the private lane.
//! 3. "Getting your minds ready": [`wire_mind`] is each harness's model step (no question).
// Called by the onboarding screens, which are built on a branch stacked on this one.
#![allow(dead_code)]

use super::consent;
use super::models::CatalogueModel;

/// The pick for every mind, `<account>/<model>` (an id from the catalogue): what `"model":
/// "picked"` in every harness's config resolves to from now on.
pub fn choose_model(model: &str) -> Result<(), String> {
    if super::catalogue().find(model).is_none() {
        return Err(format!("{model} is not one of the listed models"));
    }
    crate::picker::pick_for_every_mind(model)
}

/// The sharing line, pre-checked, in the person's words: "Your minds will send what they remember
/// about you, and where you are, to <provider> when they ask it something."
pub fn share_line(provider: &str) -> String {
    format!("Your minds will send what they remember about you, and where you are, to {provider} when they ask it something.")
}

/// The box under [`share_line`]: checked, every account with no choice of its own is Share;
/// unchecked, Off (a call carrying personal things is then answered by the private lane, or
/// refused with the fix named).
pub fn agree_to_share(checked: bool) -> Result<(), String> {
    consent::agree_at_onboarding(&consent::path(), checked)
}

/// The models on this computer or the local network that can keep personal things here.
pub fn private_lane_choices() -> Vec<CatalogueModel> {
    let local: Vec<String> = super::accounts().into_iter().filter(|a| a.local).map(|a| a.id).collect();
    super::catalogue().models().filter(|m| local.contains(&m.account)).cloned().collect()
}

/// "Keep personal things on this computer instead": `model` (one of [`private_lane_choices`])
/// becomes the private lane, and every account the person has not chosen for otherwise keeps
/// personal things on it.
pub fn keep_personal_things_on(model: &str) -> Result<(), String> {
    if !private_lane_choices().iter().any(|m| m.id == model) {
        return Err(format!("{model} is not on this computer or your network"));
    }
    let path = consent::path();
    consent::set_lane(&path, Some(model))?;
    let mut c = consent::load(&path);
    if c.default != Some(consent::Sharing::Share) {
        c.default = Some(consent::Sharing::Lane);
        consent::save(&path, &c)?;
    }
    Ok(())
}

/// One mind's model step: its own config pointed at the person's model, with no question.
pub(crate) fn wire_mind(harness: &str) -> Result<crate::provider_handoff::Wired, String> {
    crate::wire::harness_provider::wire_installed(harness)
}
