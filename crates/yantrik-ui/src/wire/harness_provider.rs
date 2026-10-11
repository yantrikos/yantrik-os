//! Settings → Harnesses: "Use a provider" and "Revert provider" on a harness row, and the same
//! two for agents (`assign_provider`, `revert_provider` in control.rs).
//!
//! What the person answers is the plan's own card (`provider_handoff::Plan::card`). Between the
//! two presses this holds only which harness, which provider and the card's text — not the plan,
//! which carries the key. Apply plans again and refuses if the card would now read differently
//! (a changed provider, address or model), so what was read is what is done; an agent's call is
//! held to its approval card's sentence the same way. The rows pick up the change on the page's
//! own refresh.

use std::path::PathBuf;
use std::sync::Mutex;

use slint::ComponentHandle;

use crate::provider_handoff::{self, Plan};
use crate::wire::settings::ProviderStore;
use crate::App;

/// The card on screen: (harness, provider id, the card's text).
static PENDING: Mutex<Option<(String, String, String)>> = Mutex::new(None);

/// What an agent's approval card said, by (harness, provider), until the call it was for runs:
/// if the provider is edited between the card and the call, the call is refused rather than
/// sending the key somewhere the person was not shown.
static EXPLAINED: Mutex<Vec<((String, String), String)>> = Mutex::new(Vec::new());

fn home() -> PathBuf {
    std::env::var("HOME").map(PathBuf::from).unwrap_or_default()
}

/// The plan for giving `harness` the saved provider `provider` — its id, or a name only one
/// saved provider has.
pub(crate) fn plan(harness: &str, provider: &str) -> Result<Plan, String> {
    let adapter = provider_handoff::adapter_for(harness)
        .ok_or_else(|| format!("{harness} cannot be given a provider from here yet — it keeps its own settings"))?;
    let store = ProviderStore::load();
    let chosen = match store.entries.iter().find(|e| e.id == provider) {
        Some(e) => e,
        None => {
            let named: Vec<_> = store.entries.iter().filter(|e| e.name.eq_ignore_ascii_case(provider)).collect();
            match named.as_slice() {
                [one] => *one,
                [] => return Err(format!("there is no saved provider `{provider}`")),
                _ => {
                    return Err(format!(
                        "{} saved providers are called `{provider}`; name one by its id: {}",
                        named.len(),
                        named.iter().map(|e| e.id.as_str()).collect::<Vec<_>>().join(", ")
                    ))
                }
            }
        }
    };
    adapter.plan(&home(), chosen)
}

/// The approval card's sentence for `assign_provider`: where the key goes and which file, or
/// what would stop it. Cheap: one small file read and one plan.
pub(crate) fn explain_assign(harness: &str, provider: &str) -> String {
    let sentence = match plan(harness, provider) {
        Ok(p) => p.sentence(&home()),
        Err(e) => return format!("This would not go ahead: {e}"),
    };
    let mut shown = EXPLAINED.lock().unwrap_or_else(|e| e.into_inner());
    let key = (harness.to_string(), provider.to_string());
    shown.retain(|(k, _)| *k != key);
    shown.push((key, sentence.clone()));
    // Bounded: a card no one answers leaves one line here, not a growing list.
    if shown.len() > 16 {
        shown.remove(0);
    }
    sentence
}

pub(crate) fn explain_revert(harness: &str) -> String {
    match provider_handoff::adapter_for(harness) {
        Some(a) => format!(
            "Puts back {}'s own settings file, as it was before a provider was given to it, and restarts it if it is running.",
            a.name()
        ),
        None => String::new(),
    }
}

/// Whether the harness's row offers it: installed, not mid-job, and it has an adapter.
fn offered(harness: &str) -> bool {
    crate::wire::harness::catalogue_for_describe()
        .as_array()
        .into_iter()
        .flatten()
        .any(|row| row["id"] == harness && row["can_assign_provider"] == true)
}

/// For agents: plan and apply in one call. The action is graded sensitive and its card carries
/// `explain_assign`'s sentence, so the person has answered knowing where the key goes.
pub(crate) fn assign(harness: &str, provider: &str) -> Result<String, String> {
    if !offered(harness) {
        return Err(format!("{harness} is not installed, is busy, or cannot be given a provider"));
    }
    let plan = plan(harness, provider)?;
    let home = home();
    let key = (harness.to_string(), provider.to_string());
    let shown = {
        let mut shown = EXPLAINED.lock().unwrap_or_else(|e| e.into_inner());
        let at = shown.iter().position(|(k, _)| *k == key);
        at.map(|i| shown.remove(i).1)
    };
    if let Some(shown) = shown {
        if shown != plan.sentence(&home) {
            return Err("The provider or the harness changed after the approval card was shown; nothing was written. Ask again to see what it would do now.".into());
        }
    }
    let card = plan.card(&home);
    provider_handoff::apply(&home, &plan)?;
    Ok(card)
}

pub(crate) fn revert(harness: &str) -> Result<(), String> {
    provider_handoff::revert(&home(), harness)
}

pub(crate) fn wire(ui: &App) {
    {
        let weak = ui.as_weak();
        ui.on_plan_provider(move |harness, provider| {
            let Some(ui) = weak.upgrade() else { return };
            match plan(&harness, &provider) {
                Ok(p) => {
                    let card = p.card(&home());
                    ui.set_settings_handoff_card_title(format!("Give {} {}?", p.harness_name, p.provider_name).into());
                    ui.set_settings_handoff_card_body(card.clone().into());
                    *PENDING.lock().unwrap_or_else(|e| e.into_inner()) = Some((harness.to_string(), provider.to_string(), card));
                    ui.set_settings_handoff_card_open(true);
                }
                Err(e) => ui.set_harness_error(e.into()),
            }
        });
    }
    {
        let weak = ui.as_weak();
        ui.on_apply_provider(move || {
            let Some(ui) = weak.upgrade() else { return };
            ui.set_settings_handoff_card_open(false);
            ui.set_settings_assigning_harness("".into());
            let Some((harness, provider, shown)) = PENDING.lock().unwrap_or_else(|e| e.into_inner()).take() else {
                return;
            };
            let outcome = plan(&harness, &provider).and_then(|p| {
                if p.card(&home()) != shown {
                    return Err("The provider or the harness's file changed since the card was shown; nothing was written. Choose again to see what it would do now.".to_string());
                }
                provider_handoff::apply(&home(), &p).map(|_| ())
            });
            if let Err(e) = outcome {
                ui.set_harness_error(e.into());
            }
        });
    }
    {
        let weak = ui.as_weak();
        ui.on_cancel_provider(move || {
            let Some(ui) = weak.upgrade() else { return };
            ui.set_settings_handoff_card_open(false);
            PENDING.lock().unwrap_or_else(|e| e.into_inner()).take();
        });
    }
    {
        let weak = ui.as_weak();
        ui.on_revert_provider(move |harness| {
            let Some(ui) = weak.upgrade() else { return };
            if let Err(e) = revert(&harness) {
                ui.set_harness_error(e.into());
            }
        });
    }
}
