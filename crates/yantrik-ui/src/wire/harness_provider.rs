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

pub(crate) fn home_dir() -> PathBuf {
    home()
}

fn home() -> PathBuf {
    std::env::var("HOME").map(PathBuf::from).unwrap_or_default()
}

/// The model a harness on Yantrik models answers with (what `"model": "picked"` resolves to at
/// each call): the one the person picked in the one picker, else the first it may use, on an
/// account not set to Off for private context (every mind carries it: YantrikDB memory).
pub(crate) fn starting_model(harness: &str) -> Result<String, String> {
    let catalogue = crate::ai_accounts::catalogue();
    if catalogue.models().next().is_none() {
        return Err("There are no models yet: add an account under AI & Intelligence (or switch on a free one), then press Use Yantrik models again.".into());
    }
    if let Some(picked) = crate::picker::remembered_model(harness).filter(|m| catalogue.find(m).is_some()) {
        return Ok(picked);
    }
    let accounts = crate::ai_accounts::accounts();
    let allowed = crate::ai_accounts::consent::load(&crate::ai_accounts::consent::path());
    let usable = catalogue
        .models()
        .find(|m| accounts.iter().any(|a| a.id == m.account && crate::gateway::sharing(a, &allowed) != crate::ai_accounts::consent::Sharing::Off))
        .map(|m| m.id.clone());
    usable.ok_or_else(|| {
        format!(
            "{harness} remembers things about you, and no account may hear them yet. Set up a private lane, or allow sharing in AI & Intelligence → AI accounts, then try again."
        )
    })
}

/// What install-time wiring offers a harness: a token minted now, `picked` resolved at each call
/// (so it is wired even before any account exists), and the catalogue for a harness that lists
/// models (Pi, OpenClaw).
fn install_offer(harness: &str) -> Result<provider_handoff::Offer, String> {
    Ok(provider_handoff::Offer {
        token: provider_handoff::Token(yantrik_gateway::tokens::mint()?),
        model: starting_model(harness).unwrap_or_else(|_| yantrik_gateway::PICKED.to_string()),
        models: crate::ai_accounts::catalogue().models().cloned().collect(),
    })
}

/// The model step of installing a harness, with no question (the install-time default): its own
/// config pointed at the gateway with a token and `"model": "picked"`, its own file kept for
/// Revert. Idempotent; a harness the person gave a saved provider or "Use its own models" is left
/// alone. What `yantrik-ui --wire-model <harness>` and Settings → Harnesses → Install call.
pub(crate) fn wire_installed(harness: &str) -> Result<provider_handoff::Wired, String> {
    // Only the shell's own host knows what an attached Mind announced; another process (the
    // installer's `--wire-model`) reads false, and the Mind is asked first, as before.
    let needs_model = crate::wire::harness::host().is_some_and(|h| h.needs_model(harness));
    provider_handoff::wire_at_install(&home(), harness, needs_model, || install_offer(harness))
}

/// `yantrik-ui --wire-model <harness>`: [`wire_installed`] for an installer (the ISO build, the
/// installer, harnesses/lib/install's `wire_model`), with no TTY. One line on stdout; exit 0 when
/// the harness is (or already was) on the person's model, or was left as the person chose; 1 with
/// the reason on stderr otherwise.
pub fn wire_model_cli(args: &[String]) -> i32 {
    let [harness] = args else {
        eprintln!("usage: yantrik-ui --wire-model <harness>");
        return 2;
    };
    // The person's own files, written as the person. As root (a `sudo` that kept $HOME) every
    // write would follow directories the person controls and leave root-owned files the shell
    // and the harness cannot read, so it is refused.
    // SAFETY: geteuid cannot fail.
    if unsafe { libc::geteuid() } == 0 {
        eprintln!("{harness}: run this as the person whose harness it is, not as root");
        return 1;
    }
    match wire_installed(harness) {
        Ok(provider_handoff::Wired::Now) => println!("{harness} now uses your model (Revert in Settings → Harnesses puts its own file back)"),
        Ok(provider_handoff::Wired::Already) => println!("{harness} already uses your model"),
        Ok(provider_handoff::Wired::OwnModels) => println!("{harness} uses its own models, as you chose"),
        Ok(provider_handoff::Wired::SavedProvider(p)) => println!("{harness} uses {p}, as you chose"),
        Ok(provider_handoff::Wired::AsksFirst) => println!("{harness} keeps its own provider until you choose Use Yantrik models on its card"),
        Err(e) => {
            eprintln!("{harness}: {e}");
            return 1;
        }
    }
    0
}

/// "Use its own models" on a harness's row: its own file back and no install wires it again.
pub(crate) fn use_own_models(harness: &str) -> Result<(), String> {
    provider_handoff::set_own_models(&home(), harness, true)
}

thread_local! {
    /// The window, for the one toast a wiring after onboarding shows. Set by [`wire`].
    static UI: std::cell::RefCell<Option<slint::Weak<App>>> = const { std::cell::RefCell::new(None) };
}

/// The toast's id prefix; its one action is "undo".
pub(crate) const WIRED_TOAST: &str = "local:wired:";

/// After an install (any thread): wire the harness, and once onboarding is over, say so in one
/// toast, "<Harness> now uses your model · Undo". During onboarding its summary says it instead.
pub(crate) fn wire_after_install(harness: &str) {
    match wire_installed(harness) {
        Ok(provider_handoff::Wired::Now) if crate::onboarding::marker_path().exists() => {
            let Some(name) = provider_handoff::adapter_for(harness).map(|a| a.name()) else { return };
            let id = harness.to_string();
            let _ = slint::invoke_from_event_loop(move || {
                let Some(ui) = UI.with(|u| u.borrow().as_ref().and_then(|w| w.upgrade())) else { return };
                crate::wire::toast::push(
                    &ui,
                    crate::ToastData {
                        id: format!("{WIRED_TOAST}{id}").into(),
                        app_name: name.into(),
                        summary: format!("{name} now uses your model").into(),
                        body: "".into(),
                        urgency: 1,
                        icon_char: name.chars().next().unwrap_or('M').to_string().into(),
                        actions: slint::ModelRc::new(slint::VecModel::from(vec![crate::ToastActionData { id: "undo".into(), label: "Undo".into() }])),
                        thumbnail: Default::default(),
                        has_thumbnail: false,
                    },
                    1,
                );
            });
        }
        Ok(w) => tracing::info!(harness, outcome = ?w, "model step at install"),
        Err(e) => {
            tracing::warn!(harness, error = %e, "the model step at install did not run");
            // The install itself stands; the person is told it still uses its own models, why, and
            // where to try again. A harness with no model step has nothing to tell.
            let Some(name) = provider_handoff::adapter_for(harness).map(|a| a.name().to_string()) else { return };
            let _ = slint::invoke_from_event_loop(move || {
                let Some(ui) = UI.with(|u| u.borrow().as_ref().and_then(|w| w.upgrade())) else { return };
                crate::wire::toast::push(&ui, not_wired_toast(&name, &e), 1);
            });
        }
    }
}

/// What the person is told when an install's model step did not run: the harness is installed and
/// keeps its own models, with the reason and where to try again. No action: there is nothing to undo.
pub(crate) fn not_wired_toast(name: &str, why: &str) -> crate::ToastData {
    crate::ToastData {
        id: format!("local:not-wired:{name}").into(),
        app_name: name.into(),
        summary: format!("{name} still uses its own models").into(),
        body: format!("{}. Settings → Harnesses → Use Yantrik models tries again.", why.trim_end_matches('.')).into(),
        urgency: 1,
        icon_char: name.chars().next().unwrap_or('M').to_string().into(),
        actions: slint::ModelRc::new(slint::VecModel::<crate::ToastActionData>::default()),
        thumbnail: Default::default(),
        has_thumbnail: false,
    }
}

/// The toast's Undo: the harness goes back to its own models and its own file.
pub(crate) fn undo_wired(harness: &str) {
    if let Err(e) = use_own_models(harness) {
        tracing::warn!(harness, error = %e, "undoing the model step did not work");
    }
}

/// The plan for pointing `harness` at the local model gateway, with a token minted for it now.
fn plan_gateway(harness: &str) -> Result<Plan, String> {
    let adapter = provider_handoff::adapter_for(harness)
        .ok_or_else(|| format!("{harness} cannot be pointed at Yantrik models from here yet — it keeps its own settings"))?;
    let model = starting_model(harness)?;
    let offer = provider_handoff::Offer {
        token: provider_handoff::Token(yantrik_gateway::tokens::mint()?),
        model,
        models: crate::ai_accounts::catalogue().models().cloned().collect(),
    };
    adapter.plan_gateway(&home(), &offer)
}

/// The plan for giving `harness` the saved provider `provider` — its id, or a name only one
/// saved provider has — or, for [`provider_handoff::GATEWAY_ID`], Yantrik models.
pub(crate) fn plan(harness: &str, provider: &str) -> Result<Plan, String> {
    if provider == provider_handoff::GATEWAY_ID {
        return plan_gateway(harness);
    }
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
    if provider_handoff::uses_gateway(&home(), harness) {
        if let Some(a) = provider_handoff::adapter_for(harness) {
            return format!(
                "Withdraws {}'s gateway token and puts back its own settings, as they were before it was given Yantrik models.",
                a.name()
            );
        }
    }
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
    UI.with(|u| *u.borrow_mut() = Some(ui.as_weak()));
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
    {
        let weak = ui.as_weak();
        ui.on_use_own_models(move |harness| {
            let Some(ui) = weak.upgrade() else { return };
            if let Err(e) = use_own_models(&harness) {
                ui.set_harness_error(e.into());
            }
        });
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_model_step_that_did_not_run_is_said_with_its_reason_and_where_to_try_again() {
        let t = super::not_wired_toast("Yantrik Mind", "The Mind did not take it (HTTP 422): it may not take Yantrik models this way yet.");
        assert_eq!(t.summary.as_str(), "Yantrik Mind still uses its own models");
        assert_eq!(
            t.body.as_str(),
            "The Mind did not take it (HTTP 422): it may not take Yantrik models this way yet. Settings → Harnesses → Use Yantrik models tries again."
        );
        assert_eq!(slint::Model::row_count(&t.actions), 0, "nothing to undo");
    }
}
