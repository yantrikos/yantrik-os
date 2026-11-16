//! The card drawn (components/start_free_card.slint) and what a press starts. The press is the
//! person's: it is a callback of the card's button, and the control surface refuses an agent's
//! call to it (`control_start_free`).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use slint::{ComponentHandle, ModelRc, VecModel};

use super::offers::{offers, Offer};
use super::press::{self, Outcome};
use super::real::Real;
use crate::app_context::AppContext;
use crate::wire::onboarding::{ask_the_mind, Proof};
use crate::{App, StartFreeOffer, StartFreeState};

/// One press at a time.
static RUNNING: AtomicBool = AtomicBool::new(false);
/// The provider of the card last pressed, for "Try again".
static LAST: Mutex<Option<Offer>> = Mutex::new(None);

/// "Can it answer?", asked of the companion itself, as the first run's own check asks it.
fn can_it_answer() -> Result<String, String> {
    match ask_the_mind(crate::wire::harness::BUILTIN_ID) {
        Proof::Passed { details } => Ok(details),
        Proof::Attention { reason, .. } => Err(reason),
    }
}

fn publish(ui: &App) {
    let shown: Vec<StartFreeOffer> = offers(&crate::ai_accounts::accounts(), &crate::ai_accounts::catalogue())
        .into_iter()
        .map(|o| StartFreeOffer {
            account: o.account.into(),
            provider: o.provider.into(),
            label: o.label.into(),
            sentence: o.sentence.into(),
            button: o.button.into(),
        })
        .collect();
    ui.global::<StartFreeState>().set_offers(ModelRc::new(VecModel::from(shown)));
}

/// Show what a press came to, and tell the rest of the shell what it changed.
fn show(weak: &slint::Weak<App>, bridge: &std::sync::Arc<crate::bridge::CompanionBridge>, account: &str, outcome: Outcome) {
    let (status, words) = match outcome {
        Outcome::Answered(w) => ("answered", w),
        Outcome::Unanswered(w) => ("unanswered", w),
        Outcome::Failed(w) => ("failed", w),
    };
    super::note_pressed(account, status);
    // The accounts card and the ask bar's line read the consent and the pick: list again.
    crate::ai_accounts::refresh(bridge.clone());
    let (weak, account) = (weak.clone(), account.to_string());
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = weak.upgrade() {
            let g = ui.global::<StartFreeState>();
            g.set_account(account.into());
            g.set_status(status.into());
            g.set_words(words.into());
            crate::wire::runs_on_card::publish(&ui);
        }
    });
}

fn begin(ui: &App, account: &str) {
    let g = ui.global::<StartFreeState>();
    g.set_account(account.into());
    g.set_words("".into());
    g.set_status("working".into());
}

pub fn wire(ui: &App, ctx: &AppContext) {
    let weak = ui.as_weak();
    crate::ai_accounts::on_change(move || {
        let weak = weak.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = weak.upgrade() {
                publish(&ui);
            }
        });
    });
    publish(ui);
    let g = ui.global::<StartFreeState>();
    let (weak, bridge) = (ui.as_weak(), ctx.bridge.clone());
    g.on_press(move |account| {
        let Some(ui) = weak.upgrade() else { return };
        // Only a card on screen, as offered now, can be pressed.
        let Some(offer) = offers(&crate::ai_accounts::accounts(), &crate::ai_accounts::catalogue()).into_iter().find(|o| o.account == account.as_str()) else {
            return;
        };
        let Some(chosen) = crate::ai_accounts::accounts().into_iter().find(|a| a.id == offer.account) else { return };
        if RUNNING.swap(true, Ordering::SeqCst) {
            return;
        }
        begin(&ui, &offer.account);
        super::note_pressed(&offer.account, "working");
        *LAST.lock().unwrap_or_else(|e| e.into_inner()) = Some(offer.clone());
        let (weak, bridge) = (weak.clone(), bridge.clone());
        let started = std::thread::Builder::new().name("start-free".into()).spawn(move || {
            let steps = Real::new(chosen, offer.model.clone(), Box::new(can_it_answer));
            let outcome = press::press(&steps, &offer.provider);
            RUNNING.store(false, Ordering::SeqCst);
            show(&weak, &bridge, &offer.account, outcome);
        });
        if started.is_err() {
            RUNNING.store(false, Ordering::SeqCst);
        }
    });
    let (weak, bridge) = (ui.as_weak(), ctx.bridge.clone());
    g.on_retry(move || {
        let Some(ui) = weak.upgrade() else { return };
        let Some(offer) = LAST.lock().unwrap_or_else(|e| e.into_inner()).clone() else { return };
        if RUNNING.swap(true, Ordering::SeqCst) {
            return;
        }
        begin(&ui, &offer.account);
        let (weak, bridge) = (weak.clone(), bridge.clone());
        let started = std::thread::Builder::new().name("start-free".into()).spawn(move || {
            let outcome = press::check(&offer.provider, &can_it_answer);
            RUNNING.store(false, Ordering::SeqCst);
            show(&weak, &bridge, &offer.account, outcome);
        });
        if started.is_err() {
            RUNNING.store(false, Ordering::SeqCst);
        }
    });
}
