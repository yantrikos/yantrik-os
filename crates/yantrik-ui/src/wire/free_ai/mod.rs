//! Settings → AI → Free AI accounts: the card that walks a person through the free tiers' sign-up
//! pages and takes each key into the vault (components/free_ai_card.slint).
//!
//! - `rows`: what the card shows, from what is known (pure, tested).
//! - `intake`: a key from the clipboard, shaped, checked with its provider, kept (tested).
//! - `store`: the keys on the companion's worker, reached only from the UI's own bridge.
//! - `choices`: what the person chose, kept across restarts (no key in it).
//!
//! The card draws from a cache of which values are kept, refreshed off the UI thread: the worker
//! can be in the middle of an answer for tens of seconds, and a click must not wait for it.

pub mod choices;
pub mod intake;
pub mod rows;
pub mod store;

use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::app_context::AppContext;
use crate::bridge::CompanionBridge;
use crate::clipboard::SharedHistory;
use crate::{App, FreeAiRow, FreeAiState, FreeAiValue};
use choices::Choices;
use intake::Step;
use rows::Session;
use store::{Op, Reply};

const WORKER_TIMEOUT: Duration = Duration::from_secs(30);

static SESSION: LazyLock<Mutex<Session>> = LazyLock::new(Default::default);
/// Which values are kept, with their tails, as last read from the vault.
static KEPT: LazyLock<Mutex<BTreeMap<String, String>>> = LazyLock::new(Default::default);
static CHOICES: LazyLock<Mutex<Choices>> = LazyLock::new(|| Mutex::new(choices::load(&choices::path())));

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Clone)]
struct Wiring {
    ui: slint::Weak<App>,
    bridge: Arc<CompanionBridge>,
    history: SharedHistory,
}

pub fn wire(ui: &App, ctx: &AppContext) {
    let w = Wiring { ui: ui.as_weak(), bridge: ctx.bridge.clone(), history: ctx.clip_history.clone() };
    let g = ui.global::<FreeAiState>();

    let x = w.clone();
    g.on_start(move |id| x.start(&id));
    let x = w.clone();
    g.on_have_key(move |id| x.open_keys(&id));
    let x = w.clone();
    g.on_open_keys(move |id| x.open_keys(&id));
    let x = w.clone();
    g.on_replace(move |id| x.open_keys(&id));
    let x = w.clone();
    g.on_paste(move |value_id| x.paste(value_id.to_string()));
    let x = w.clone();
    g.on_skip(move |id| x.choose(|c| { c.skipped.insert(id.to_string()); c.stage.remove(id.as_str()); }));
    let x = w.clone();
    g.on_unskip(move |id| x.choose(|c| { c.skipped.remove(id.as_str()); }));
    let x = w.clone();
    g.on_toggle(move |id| x.choose(|c| { if !c.off.remove(id.as_str()) { c.off.insert(id.to_string()); } }));
    let x = w.clone();
    g.on_remove(move |id| { lock(&SESSION).confirm_remove = Some(id.to_string()); x.render(); });
    let x = w.clone();
    g.on_cancel_remove(move |_| { lock(&SESSION).confirm_remove = None; x.render(); });
    let x = w.clone();
    g.on_confirm_remove(move |id| x.remove(id.to_string()));
    let x = w.clone();
    g.on_set_expanded(move |open| { if let Some(ui) = x.ui.upgrade() { ui.global::<FreeAiState>().set_expanded(open); } });
    let x = w.clone();
    g.on_chip_pressed(move || x.chip());

    w.render();
    w.refresh();
}

impl Wiring {
    /// Draw from what is known now.
    fn render(&self) {
        let card = rows::card(&lock(&CHOICES), &lock(&KEPT), &lock(&SESSION));
        // The clipboard is held (history paused, the model's clipboard tools refused) while a key
        // is awaited or being checked. Not after a refusal: a refused key that was key-shaped has
        // already been cleared from the clipboard, and holding on would keep history paused and
        // the clipboard tools refused for as long as the row is left (seen on VM 520, 1 Oct 2026).
        let held = card.rows.iter().any(|r| matches!(r.state, "waiting" | "checking"));
        crate::clipboard::hold_for_a_key(held);
        let status = crate::vault_unlock::cached_status();
        let Some(ui) = self.ui.upgrade() else { return };
        let g = ui.global::<FreeAiState>();
        let rows: Vec<FreeAiRow> = card.rows.iter().map(to_slint).collect();
        let (trains, clean): (Vec<FreeAiRow>, Vec<FreeAiRow>) = rows.iter().cloned().partition(|r| r.trains);
        g.set_clean_rows(ModelRc::new(VecModel::from(clean)));
        g.set_trains_rows(ModelRc::new(VecModel::from(trains)));
        g.set_rows(ModelRc::new(VecModel::from(rows)));
        g.set_summary(card.summary.into());
        g.set_next_line(card.next_line.into());
        g.set_next_id(card.next_id.into());
        g.set_next_name(card.next_name.into());
        g.set_chip_label(card.chip_label.into());
        g.set_chip_value_id(card.chip_value_id.into());
        g.set_vault_locked(status.protected && !status.unlocked);
    }

    /// Read which values are kept from the vault, off the UI thread, then draw.
    fn refresh(&self) {
        let x = self.clone();
        let _ = std::thread::Builder::new().name("free-ai-refresh".into()).spawn(move || {
            if let Ok(Reply::Tails(tails)) = x.bridge.provider_keys(Op::Tails, WORKER_TIMEOUT) {
                *lock(&KEPT) = tails;
            }
            let y = x.clone();
            let _ = slint::invoke_from_event_loop(move || y.render());
        });
    }

    fn choose(&self, change: impl FnOnce(&mut Choices)) {
        {
            let mut c = lock(&CHOICES);
            change(&mut c);
            if let Err(e) = choices::save(&choices::path(), &c) {
                tracing::warn!(error = %e, "the free AI card's choices were not saved");
            }
        }
        self.render();
    }

    fn start(&self, id: &str) {
        let Some(s) = yantrik_ml::provider::pool::signup::signup(id) else { return };
        // Where the sign-up page is the key page too (Google's), there is nothing between them.
        let same = s.values.first().is_some_and(|v| v.url == s.signup_url);
        if let Err(e) = crate::open_url::open_apart(s.signup_url) {
            tracing::warn!(provider = id, error = %e, "the sign-up page could not be opened");
        }
        let stage = if same { "waiting" } else { "sign-up-opened" };
        self.choose(|c| { c.stage.insert(id.to_string(), stage.to_string()); c.skipped.remove(id); });
    }

    fn open_keys(&self, id: &str) {
        let Some(s) = yantrik_ml::provider::pool::signup::signup(id) else { return };
        let kept = lock(&KEPT).clone();
        let pending = lock(&SESSION).pending_account.is_some();
        let next = s.values.iter().find(|v| !kept.contains_key(v.id) && !(v.id == "cloudflare_account" && pending)).or(s.values.last());
        if let Some(v) = next {
            if let Err(e) = crate::open_url::open_apart(v.url) {
                tracing::warn!(provider = id, error = %e, "the key page could not be opened");
            }
        }
        lock(&SESSION).rejected.remove(id);
        self.choose(|c| { c.stage.insert(id.to_string(), "waiting".to_string()); c.skipped.remove(id); });
    }

    fn paste(&self, value_id: String) {
        let Some((s, value)) = intake::value_of(&value_id) else { return };
        let provider = s.id.to_string();
        {
            let mut session = lock(&SESSION);
            if session.checking.is_some() {
                return; // one at a time
            }
            session.checking = Some(provider.clone());
            session.rejected.remove(&provider);
        }
        self.render();
        let x = self.clone();
        let spawned = std::thread::Builder::new().name("free-ai-paste".into()).spawn(move || {
            // However this ends, a panic included, the row stops saying "checking".
            let _done = Checking(x.clone());
            let Some(pasted) = intake::read_clipboard() else {
                lock(&SESSION).rejected.insert(
                    provider.clone(),
                    "The clipboard holds more than a key, or did not answer in time. Copy the key again, then press Paste. Nothing was sent or saved.".into(),
                );
                return;
            };
            let pending = lock(&SESSION).pending_account.clone();
            let step = intake::decide(&value_id, &pasted, pending.as_deref(), intake::check, |id, value| {
                x.bridge
                    .provider_keys(Op::Store { id: id.to_string(), value: value.to_string() }, WORKER_TIMEOUT)
                    .unwrap_or(Reply::NoAnswer)
            });
            // What was a key, kept or not, leaves history and the clipboard: history exactly, and
            // the clipboard only if it still holds that key (the person may have copied since).
            let was_a_key = yantrik_ml::provider::pool::signup::shape(value, &pasted).is_ok();
            if was_a_key {
                if let Ok(mut h) = x.history.lock() {
                    h.forget(&pasted);
                }
                if intake::read_clipboard().is_some_and(|now| now.trim() == pasted.trim()) {
                    intake::clear_clipboard();
                }
            }
            drop(pasted);
            {
                let mut session = lock(&SESSION);
                match &step {
                    Step::Kept { resting } => {
                        session.pending_account = None;
                        if *resting {
                            tracing::info!(provider = %provider, "a free AI key was kept; its provider is at its limit right now");
                        }
                    }
                    Step::AccountHeld(account) => session.pending_account = Some(account.clone()),
                    Step::NotKept(why) => {
                        session.rejected.insert(provider.clone(), why.clone());
                    }
                }
            }
            if matches!(step, Step::Kept { .. }) {
                lock(&CHOICES).stage.remove(&provider);
                let _ = choices::save(&choices::path(), &lock(&CHOICES));
                tracing::info!(provider = %provider, "a free AI key was checked with its provider and kept in the vault");
            }
        });
        if spawned.is_err() {
            lock(&SESSION).checking = None;
            self.render();
        }
    }

    fn remove(&self, id: String) {
        lock(&SESSION).confirm_remove = None;
        let x = self.clone();
        let _ = std::thread::Builder::new().name("free-ai-remove".into()).spawn(move || {
            let ids: Vec<&str> = yantrik_ml::provider::pool::signup::signup(&id).map(|s| s.values.iter().map(|v| v.id).collect()).unwrap_or_default();
            let mut failed = None;
            for value_id in ids {
                match x.bridge.provider_keys(Op::Remove { id: value_id.to_string() }, WORKER_TIMEOUT) {
                    Ok(Reply::Done) => {}
                    Ok(Reply::Locked) => failed = Some("The key was not removed: the vault is locked. Unlock it, then remove it again."),
                    _ => failed = Some("The key may not have been removed: the vault did not answer. This row says Ready while it is still kept."),
                }
            }
            match failed {
                None => tracing::info!(provider = %id, "a free AI key was removed from the vault"),
                Some(why) => {
                    tracing::warn!(provider = %id, "a free AI key could not be removed");
                    lock(&SESSION).rejected.insert(id.clone(), why.to_string());
                }
            }
            x.refresh();
        });
        self.render();
    }

    /// The status bar's chip: bring the card forward and paste what it named.
    fn chip(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let value_id = ui.global::<FreeAiState>().get_chip_value_id().to_string();
        if let Err(e) = crate::windows::raise_shell() {
            tracing::debug!(error = %e, "the shell could not be raised for the paste chip");
        }
        ui.set_settings_category(1);
        ui.set_current_screen(7);
        ui.invoke_navigate(7);
        if !value_id.is_empty() {
            self.paste(value_id);
        }
    }
}

/// Clears "checking" when a paste ends, however it ends, and redraws.
struct Checking(Wiring);

impl Drop for Checking {
    fn drop(&mut self) {
        lock(&SESSION).checking = None;
        self.0.refresh();
    }
}

fn to_slint(r: &rows::Row) -> FreeAiRow {
    let s = |t: &str| SharedString::from(t);
    FreeAiRow {
        id: s(&r.id),
        name: s(&r.name),
        state: s(r.state),
        state_words: s(&r.state_words),
        offer: s(&r.offer),
        needs: s(&r.needs),
        detail: s(&r.detail),
        trains: r.trains,
        on: r.on,
        values: ModelRc::new(VecModel::from(
            r.values.iter().map(|v| FreeAiValue { id: s(&v.id), label: s(&v.label), r#where: s(&v.where_), done: v.done }).collect::<Vec<_>>(),
        )),
        opt_out: s(&r.opt_out),
    }
}


#[cfg(test)]
mod card_source_tests {
    const CARD: &str = include_str!("../../../../yantrik-ui-slint/ui/components/free_ai_card.slint");

    /// A key never enters the card: every callback carries one id (or a bool) and nothing else,
    /// and no row field is a key.
    #[test]
    fn no_callback_on_the_card_can_carry_a_key() {
        let callbacks: Vec<&str> = CARD.lines().map(str::trim).filter(|l| l.starts_with("callback ")).collect();
        assert!(callbacks.len() >= 12, "{callbacks:?}");
        for c in callbacks {
            let args = c.split_once('(').and_then(|(_, rest)| rest.split_once(')')).map_or("", |(a, _)| a);
            assert!(matches!(args, "" | "string" | "bool"), "{c} carries more than an id");
        }
        let row = CARD.split("export struct FreeAiRow").nth(1).and_then(|s| s.split('}').next()).unwrap();
        for line in row.lines().map(str::trim).filter(|l| !l.starts_with("//") && l.contains(':')) {
            let field = line.split(':').next().unwrap().trim();
            assert!(!field.contains("key") && !field.contains("secret") && !field.contains("token"), "FreeAiRow has a field {field}");
        }
    }
}
