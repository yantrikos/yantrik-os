//! The unlock prompt: the one way a vault passphrase enters this machine.
//!
//! Three jobs, all of them small.
//!
//! **Seed the cache.** `describe shell` answers on the UI thread and the database lives on the
//! companion's worker, so the vault's state is read once at startup and kept in
//! `vault_unlock`'s statics from there. Without it the first `describe` after boot would have to
//! choose between blocking on a worker that might be mid-thought and guessing.
//!
//! **Notice when a mind wanted the vault and could not have it.** The tools set a latch
//! (`yantrik_companion::tools::vault::take_unlock_request`) rather than calling into the shell,
//! because they are a leaf crate the shell depends on and the call would be a cycle. This polls
//! it.
//!
//! **Take the keystrokes.** `on_vault_unlock_submit` is wired here and nowhere else. It is a
//! Slint callback, which means the only thing that can fire it is a person pressing Enter or
//! clicking a button in a window this shell drew — the same property the approval card's
//! `on_approval_allow` has, and for the same reason.

use std::time::Duration;

use slint::{ComponentHandle, Timer, TimerMode};

use crate::app_context::AppContext;
use crate::vault_unlock::{self, Op, Outcome};
use crate::{App, VaultUnlockRequest};

/// How long to give the worker. Argon2id at 19 MiB is a fraction of a second; the rest of this is
/// headroom for a companion in the middle of something.
const VAULT_TIMEOUT: Duration = Duration::from_secs(20);

pub fn wire(ui: &App, ctx: &AppContext) {
    wire_submit(ui, ctx);
    wire_dismiss(ui);
    wire_settings_button(ui);
    poll_for_requests(ui, ctx);
}

/// "Set a vault passphrase" in Settings > Privacy & Security.
///
/// Raises the same card everything else raises. Settings does not collect the passphrase itself,
/// even though it has a panel and could: `show_screen settings section=privacy` is a published
/// action, so an agent can put that panel in front of a person, and a passphrase field on a screen
/// something else can navigate to is a field something else can be standing in front of. One card,
/// in one place, drawn over everything — that is the thing a person can learn to recognise.
fn wire_settings_button(ui: &App) {
    let ui_weak = ui.as_weak();
    ui.on_vault_set_passphrase(move || {
        let first_time = !vault_unlock::cached_status().protected;
        vault_unlock::raise(
            if first_time {
                "you asked to lock the vault with a passphrase"
            } else {
                "you asked to change the vault's passphrase"
            },
            first_time,
        );
        if let Some(ui) = ui_weak.upgrade() {
            render(&ui);
        }
    });
}

/// A person typed a passphrase. This is the only function in the shell that receives one from
/// outside a verified login.
fn wire_submit(ui: &App, ctx: &AppContext) {
    let ui_weak = ui.as_weak();
    let bridge = ctx.bridge.clone();

    ui.on_vault_unlock_submit(move |typed| {
        let passphrase = typed.to_string();
        if passphrase.is_empty() {
            vault_unlock::set_prompt_error("Type the passphrase, then press Enter.");
            if let Some(ui) = ui_weak.upgrade() {
                render(&ui);
            }
            return;
        }

        let bridge = bridge.clone();
        let weak = ui_weak.clone();
        // On a thread, because the derivation is deliberately slow and the desktop must not
        // freeze while a person's password is being turned into a key.
        std::thread::spawn(move || {
            let result = bridge.vault(Op::Adopt(passphrase), VAULT_TIMEOUT);
            // `passphrase` was moved into the call and the reply carries nothing derived from it.
            // Everything below this line is a fixed sentence.
            let message = match &result {
                Ok(reply) => match &reply.outcome {
                    Some(outcome @ (Outcome::Protected | Outcome::Unlocked)) => {
                        tracing::info!(
                            first_time = matches!(outcome, Outcome::Protected),
                            "The vault was opened from the desktop's prompt"
                        );
                        crate::provider_vault::opened();
                        None
                    }
                    Some(other) => Some(other.message()),
                    None => None,
                },
                Err(e) => {
                    tracing::warn!(error = %e, "Could not reach the vault from the prompt");
                    Some("The vault did not answer. Try again in a moment.".to_string())
                }
            };

            match message {
                None => vault_unlock::dismiss(),
                Some(text) => vault_unlock::set_prompt_error(text),
            }

            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = weak.upgrade() {
                    render(&ui);
                }
            });
        });
    });
}

/// "Not now". Takes the card down and changes nothing.
///
/// Whatever provoked it is not retried and is not queued: a mind that wanted a credential gets its
/// `VAULT_LOCKED` answer either way, and a person who said not now should not find the same box
/// back a second later. It comes back the next time something actually needs the vault.
fn wire_dismiss(ui: &App) {
    let ui_weak = ui.as_weak();
    ui.on_vault_unlock_dismiss(move || {
        vault_unlock::dismiss();
        if let Some(ui) = ui_weak.upgrade() {
            render(&ui);
        }
        tracing::info!("The vault unlock prompt was dismissed; the vault stays as it was");
    });
}

/// Read the vault once at startup, then watch for a mind that wanted it and could not have it.
fn poll_for_requests(ui: &App, ctx: &AppContext) {
    let ui_weak = ui.as_weak();
    let bridge = ctx.bridge.clone();

    // The one read, off the UI thread. The worker is still starting when `wire_all` runs, so this
    // retries rather than giving up on the first refusal — a `protection_known()` of false makes
    // the screen-unlock path skip the vault entirely, and it should not skip it forever because
    // the shell asked a beat too early.
    {
        let bridge = bridge.clone();
        std::thread::spawn(move || {
            for attempt in 0..10 {
                if bridge.vault(Op::Read, VAULT_TIMEOUT).is_ok() {
                    let status = vault_unlock::cached_status();
                    tracing::info!(
                        protected = status.protected,
                        unlocked = status.unlocked,
                        why = status.why.as_deref().unwrap_or(""),
                        "Vault state at startup"
                    );
                    return;
                }
                std::thread::sleep(Duration::from_secs(1 + attempt));
            }
            tracing::warn!(
                "Could not read the vault's state at startup; the desktop will report it as \
                 unprotected until something opens it"
            );
        });
    }

    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_secs(1), move || {
        let Some(ui) = ui_weak.upgrade() else { return };

        // `first_time` comes from the tool that raised this, not from the cache here. The tool
        // read the vault a moment ago; the cache may not have been filled yet on a shell that has
        // only just started, and a wrong answer asks a person to invent a new passphrase for a
        // vault that already has one.
        if let Some((reason, first_time)) = yantrik_companion::tools::vault::take_unlock_request() {
            vault_unlock::raise(reason, first_time);
        }
        render(&ui);
    });

    // The timer is dropped at the end of this function unless it is kept, and a dropped Slint
    // timer stops. Same pattern as the other polls in `wire`.
    std::mem::forget(timer);
}

/// Put the prompt's current state on the window, or take it off — and keep Settings honest.
fn render(ui: &App) {
    // The Settings row reads from the same cached status `describe shell` does, so the panel and
    // the control surface cannot tell a person two different things about their own machine.
    let status = vault_unlock::cached_status();
    ui.set_vault_protected(status.protected);
    ui.set_vault_unlocked(status.unlocked);
    ui.set_vault_why(status.why.unwrap_or_default().into());

    let request = match vault_unlock::pending() {
        Some(prompt) => VaultUnlockRequest {
            reason: prompt.reason.into(),
            error: prompt.error.into(),
            first_time: prompt.first_time,
        },
        // An empty reason is what the card checks; there is no separate "visible" flag to get out
        // of step with it.
        None => VaultUnlockRequest {
            reason: String::new().into(),
            error: String::new().into(),
            first_time: false,
        },
    };
    ui.set_vault_unlock(request);
}
