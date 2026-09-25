//! The shell's side of the one notification store.
//!
//! Polls `notifications.since(revision)` about once a second, raises a toast for anything new
//! and takes one down for anything the store says was dismissed, keeps the unread badge and the
//! notification centre (screen 9) fed, and sends the shell's own notifications — an update
//! waiting, a mind waiting for an answer — through the same door every app uses.
//!
//! ## Why a poll and not a subscription
//!
//! The service serves one request per connection over a unix socket; there is no push. A poll
//! that asks "what changed since revision N" is a few hundred bytes and almost always answers
//! with an empty list, so a second is cheap — and it recovers by itself when the service is
//! restarted under a running shell, which a long-lived subscription would not.
//!
//! The poll runs on a thread of its own and hands its answer to the UI thread with
//! `upgrade_in_event_loop`. Nothing here touches a socket from the UI thread: the shell's own
//! control surface gives an action three seconds, and a notification must not spend any of it.
//!
//! ## Do Not Disturb
//!
//! Decided here, not in the store. Under DND nothing pops except `critical`; everything is still
//! stored, still counted in the badge, and still in the notification centre. The same goes for
//! focus mode. A store that dropped notifications under DND would destroy messages for the hours
//! somebody was concentrating, which is the opposite of what the setting is for.

use std::cell::RefCell;
use std::time::{Duration, Instant};

use slint::{ComponentHandle, ModelRc, VecModel};
use yantrik_app_runtime::notify;
use yantrik_ipc_contracts::notifications::{Notification, Since, Urgency, MARK_READ, SINCE};
use yantrik_ipc_transport::SyncRpcClient;

use crate::app_context::AppContext;
use crate::{notifications, App, ToastActionData, ToastData};

/// The service, as its socket is named.
const SERVICE: &str = "notifications";

/// How often the shell asks what changed.
const POLL: Duration = Duration::from_secs(1);

/// How long one poll may take. The store is a mutex and a file read; past this the service is in
/// trouble and the next tick will try again.
const POLL_TIMEOUT: Duration = Duration::from_millis(1500);

/// How long to wait before asking the shell to start the service again after it was found down.
/// Restarting a service on a one-second loop would hide whatever is killing it.
const RESTART_BACKOFF: Duration = Duration::from_secs(20);

/// Screens where a toast must not appear: lock, login, boot, onboarding. The same list the
/// approval card uses, and for the same reason — what is on a locked screen is readable by
/// whoever is standing in front of it.
const SILENT_SCREENS: [i32; 4] = [0, 2, 3, 32];

thread_local! {
    /// The shell's mirror of the store, installed by [`wire`]. A thread_local because the poll
    /// answer arrives through `upgrade_in_event_loop`, whose closure must be `Send` and so
    /// cannot carry an `Rc`.
    static MIRROR: RefCell<Option<notifications::SharedStore>> = const { RefCell::new(None) };
    /// The system observer, for the same reason — see [`relay_to_features`].
    static OBSERVER: RefCell<Option<std::rc::Rc<yantrik_os::SystemObserver>>> =
        const { RefCell::new(None) };
}

/// Read the mirror on the UI thread.
fn with_mirror<T>(f: impl FnOnce(&mut notifications::NotificationMirror) -> T) -> Option<T> {
    MIRROR.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|shared| f(&mut shared.borrow_mut()))
    })
}

/// Wire the notification centre, the toasts, the poll, and the shell's own senders.
pub fn wire(ui: &App, ctx: &AppContext) {
    MIRROR.with(|slot| *slot.borrow_mut() = Some(ctx.notification_store.clone()));
    OBSERVER.with(|slot| *slot.borrow_mut() = Some(ctx.observer.clone()));

    wire_centre(ui);
    wire_toasts(ui);
    start_poll(ui);
    watch_for_updates();
}

// ── The notification centre (screen 9) ──────────────────────────────────────────────────────

fn wire_centre(ui: &App) {
    let weak = ui.as_weak();
    ui.on_notification_clear_all(move || {
        // Everything showing, not everything ever: the store keeps a dismissed notification for
        // a week so history survives pressing this.
        act("notifications.dismiss_all", serde_json::json!({}));
        if let Some(ui) = weak.upgrade() {
            crate::wire::toast::clear(&ui);
            let ids: Vec<String> = with_mirror(|m| {
                m.showing().into_iter().map(|n| n.id.clone()).collect()
            })
            .unwrap_or_default();
            with_mirror(|m| {
                for id in &ids {
                    m.dismiss_locally(id);
                }
            });
            resync(&ui);
        }
    });

    let weak = ui.as_weak();
    ui.on_notification_mark_all_read(move || {
        act(MARK_READ, serde_json::json!({}));
        if let Some(ui) = weak.upgrade() {
            let ids: Vec<String> = with_mirror(|m| {
                m.showing().into_iter().map(|n| n.id.clone()).collect()
            })
            .unwrap_or_default();
            with_mirror(|m| {
                for id in &ids {
                    m.mark_read_locally(id);
                }
            });
            resync(&ui);
        }
    });

    // Tapping a row: it is read, and if we know which of our own apps sent it, that app comes
    // to the front. A notification from Chromium opens nothing — we have no honest way to put
    // somebody else's window in front from here, and pretending would be a dead control.
    let weak = ui.as_weak();
    ui.on_notification_tapped(move |id| {
        let id = id.to_string();
        act(MARK_READ, serde_json::json!({ "id": id }));
        with_mirror(|m| m.mark_read_locally(&id));
        if let Some(ui) = weak.upgrade() {
            resync(&ui);
            open_sender(&ui, &id);
        }
    });

    let weak = ui.as_weak();
    ui.on_notification_dismissed(move |id| {
        let id = id.to_string();
        act("notifications.dismiss", serde_json::json!({ "id": id }));
        with_mirror(|m| m.dismiss_locally(&id));
        if let Some(ui) = weak.upgrade() {
            crate::wire::toast::remove(&ui, &id);
            resync(&ui);
        }
    });

    let weak = ui.as_weak();
    ui.on_notification_action(move |id, action_id| {
        invoke_action(&weak, &id.to_string(), &action_id.to_string());
    });

    // "Clear all from this app" — one call per notification, because the store is addressed by
    // id and inventing a bulk method for a button nobody holds down is more surface than this
    // needs.
    let weak = ui.as_weak();
    ui.on_notification_clear_group(move |app_name| {
        let app = app_name.to_string().to_lowercase();
        let ids: Vec<String> = with_mirror(|m| {
            m.showing()
                .into_iter()
                .filter(|n| n.app.to_lowercase() == app)
                .map(|n| n.id.clone())
                .collect()
        })
        .unwrap_or_default();
        for id in &ids {
            act("notifications.dismiss", serde_json::json!({ "id": id }));
        }
        with_mirror(|m| {
            for id in &ids {
                m.dismiss_locally(id);
            }
        });
        if let Some(ui) = weak.upgrade() {
            for id in &ids {
                crate::wire::toast::remove(&ui, id);
            }
            resync(&ui);
        }
    });
}

/// Mark everything currently showing as read, and redraw.
///
/// Called when the notification centre opens. Public because `wire::navigate` is what knows a
/// screen was entered.
pub fn mark_showing_read(weak: &slint::Weak<App>) {
    let unread: Vec<String> = with_mirror(|m| {
        m.showing()
            .into_iter()
            .filter(|n| !n.read)
            .map(|n| n.id.clone())
            .collect()
    })
    .unwrap_or_default();
    if unread.is_empty() {
        return;
    }
    act(MARK_READ, serde_json::json!({}));
    with_mirror(|m| {
        for id in &unread {
            m.mark_read_locally(id);
        }
    });
    if let Some(ui) = weak.upgrade() {
        resync(&ui);
    }
}

// ── Toasts ──────────────────────────────────────────────────────────────────────────────────

fn wire_toasts(ui: &App) {
    let weak = ui.as_weak();
    ui.on_toast_clicked(move |id| {
        let Some(ui) = weak.upgrade() else { return };
        // Toasts draw over every screen — the banner has no screen condition — so a toast
        // still standing when the screen locked is clickable from the lock screen, and this
        // handler walks straight to the notifications screen or launches the sender's app
        // (#203). While the desktop waits for the person the click is dropped; the toast stays
        // and can be clicked once they are back.
        if crate::control::locked_screen(ui.get_current_screen()) {
            tracing::debug!("Toast click dropped — the desktop is waiting for the person to sign in");
            return;
        }
        let id = id.to_string();
        // The "+N more" row sends an empty id: it is not a notification, it is a way in.
        if id.is_empty() {
            crate::wire::toast::clear(&ui);
            ui.set_current_screen(9);
            ui.invoke_navigate(9);
            return;
        }
        act(MARK_READ, serde_json::json!({ "id": id }));
        with_mirror(|m| m.mark_read_locally(&id));
        crate::wire::toast::remove(&ui, &id);
        resync(&ui);
        // Clicking the body of a freedesktop notification is that spec's `default` action, and
        // the sender is waiting to hear about it.
        let has_default = with_mirror(|m| {
            m.get(&id)
                .map(|n| n.actions.iter().any(|a| a.id == "default"))
                .unwrap_or(false)
        })
        .unwrap_or(false);
        if has_default {
            act(
                "notifications.action",
                serde_json::json!({ "id": id, "action_id": "default" }),
            );
        }
        open_sender(&ui, &id);
    });

    let weak = ui.as_weak();
    ui.on_toast_dismissed(move |id| {
        let Some(ui) = weak.upgrade() else { return };
        let id = id.to_string();
        crate::wire::toast::remove(&ui, &id);
        // Closing a toast is closing the notification. It stays in the centre for a week — the
        // store keeps a dismissed notification — so nothing is destroyed by a stray click.
        if !id.starts_with("local:") {
            act("notifications.dismiss", serde_json::json!({ "id": id }));
            with_mirror(|m| m.dismiss_locally(&id));
            resync(&ui);
        }
    });

    let weak = ui.as_weak();
    ui.on_toast_action(move |id, action_id| {
        invoke_action(&weak, &id.to_string(), &action_id.to_string());
    });
}

/// Press a button on a notification, wherever it was drawn.
///
/// Two different things happen depending on where the notification came from, because the two
/// kinds of sender have different ways of being told:
///
/// * **freedesktop** — the service emits `ActionInvoked` on the session bus and the program that
///   raised the notification does the work itself. That is the whole mechanism the spec gives,
///   and it is enough.
/// * **ours** — there is no such signal, and the app may not even be running. So the shell makes
///   the call the button stands for, on the app's own control surface, with the arguments the
///   notification carried. It is the same call the button inside that app's window makes.
fn invoke_action(weak: &slint::Weak<App>, id: &str, action_id: &str) {
    // A toast's buttons draw on every screen, the lock screen included, and pressing one
    // forwards to the sender's own control surface — starting the app if it is closed — so
    // this is a path that must not act while the desktop waits for the person (#203). The
    // press is dropped; the notification itself stays filed in the centre.
    if let Some(ui) = weak.upgrade() {
        if crate::control::locked_screen(ui.get_current_screen()) {
            tracing::debug!(id, action_id, "Notification action dropped — the desktop is waiting for the person to sign in");
            return;
        }
    }
    act(
        yantrik_ipc_contracts::notifications::ACTION,
        serde_json::json!({ "id": id, "action_id": action_id }),
    );

    let ours = with_mirror(|m| {
        m.get(id).and_then(|n| {
            if n.source != yantrik_ipc_contracts::notifications::Source::Yantrik {
                return None;
            }
            n.actions
                .iter()
                .find(|a| a.id == action_id)
                .map(|a| (n.app.clone(), a.args.clone()))
        })
    })
    .flatten();

    // The service dismisses a notification whose button was pressed, and the next poll will say
    // so — but the button is gone from the screen now, because a person who pressed it is done.
    with_mirror(|m| m.dismiss_locally(id));
    if let Some(ui) = weak.upgrade() {
        crate::wire::toast::remove(&ui, id);
        resync(&ui);
        if let Some((app, args)) = ours {
            forward_to_app(&ui, &app, action_id, args);
        }
    }
    tracing::info!(id, action_id, "notification action pressed");
}

/// Make the call one of our own notifications' buttons stands for.
///
/// The app is started first if it is not running: a notification outlives the process that
/// raised it, and "the download finished" is exactly the case where Download Manager has since
/// been closed. If the app is not one of ours, or its surface never answers, that is logged and
/// nothing else happens — the alternative is a button that reports success it did not observe.
fn forward_to_app(ui: &App, app: &str, action: &str, args: Option<serde_json::Value>) {
    let installed = crate::apps::Catalogue::shared().get();
    let Some((surface, opens_as)) = button_route(app, &installed) else {
        tracing::warn!(app, action, "a notification button named an app this desktop does not open");
        return;
    };
    let address = format!("app-{surface}");
    if !yantrik_app_runtime::service::is_up(&address) {
        match opens_as {
            Some(name) => ui.invoke_launch_app(name.into()),
            None => tracing::warn!(app, action, "the app a notification button belongs to is closed and cannot be opened"),
        }
    }

    let action = action.to_string();
    let args = args.unwrap_or_else(|| serde_json::json!({}));
    let _ = std::thread::Builder::new()
        .name("yos-notification-action".into())
        .spawn(move || {
            // Up to five seconds for a cold app to open its socket. This is a worker thread;
            // the person has already seen the toast go away.
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline && !yantrik_app_runtime::service::is_up(&address) {
                std::thread::sleep(Duration::from_millis(150));
            }
            if !yantrik_app_runtime::service::is_up(&address) {
                tracing::warn!(address, action, "the app a notification button belongs to did not open");
                return;
            }
            SyncRpcClient::clear_breaker(&yantrik_ipc_transport::RpcServer::default_address(&address));
            match SyncRpcClient::for_service(&address)
                .with_timeout(Duration::from_secs(4))
                .call("app.act", serde_json::json!({ "action": action, "args": args }))
            {
                Ok(reply) => tracing::info!(
                    address,
                    action,
                    accepted = reply["accepted"].as_bool().unwrap_or(false),
                    settled = reply["settled"].as_bool().unwrap_or(false),
                    "a notification button was carried out by the app that raised it"
                ),
                Err(e) => tracing::warn!(address, action, error = %e.message,
                    "the app refused the action a notification button named"),
            }
        });
}

/// Where one of our notifications' buttons goes: the surface the call is made on, and the name to
/// open the app by first when it is closed (`None` when there is nothing to open).
///
/// Through the same catalogue `open_app` and `describe shell` read, so any app that declares a
/// surface in its `.desktop` file gets its buttons carried out — a third-party app exactly as
/// ours. It used to be the dock's route table alone, so only this OS's own apps could have a
/// working button; and before that the published listing, which names each app once, so a
/// notification whose `app` was `container-manager` — the name that app carries everywhere but on
/// its socket — lost its buttons to "an app this desktop does not open". "Downloads", the name
/// Download Manager sends under, is its `Name=` and an alias.
///
/// The shell sends under `Yantrik`, which is the desktop's own surface: without that, "See what it
/// did" on a lapsed bypass would be a control that does nothing. Nothing opens the desktop, so
/// there is no name to launch.
fn button_route(
    app: &str,
    installed: &[crate::apps::DesktopEntry],
) -> Option<(String, Option<String>)> {
    let surface = crate::wire::dock::surface_for(app, installed)?;
    Some((surface, launch_name_for(app, installed)))
}

/// The name `launch_app` takes for this sender, if the shell can open it at all: one of the
/// shell's own routes, or an app that declares a surface. Not any program with a .desktop file —
/// a foreign app that did not declare a surface did not route this notification, and opening a
/// second copy of it would be a guess.
fn launch_name_for(app: &str, installed: &[crate::apps::DesktopEntry]) -> Option<String> {
    let key = app.trim().to_lowercase();
    let ours = crate::wire::dock::route(&key).is_some()
        || crate::surfaces::find(&key, installed).is_some();
    let opens = !matches!(
        crate::wire::dock::resolve(&key, installed),
        crate::wire::dock::Resolved::Unknown | crate::wire::dock::Resolved::Shelved(_)
    );
    (ours && opens).then_some(key)
}

/// Bring the app that sent a notification to the front, when it is one this desktop routes — one
/// of its own screens, or an app that declares a surface — and it is installed. Anything else does
/// nothing, and says nothing, because there is nothing honest to do: we cannot raise a foreign
/// window from a notification we did not route.
fn open_sender(ui: &App, id: &str) {
    let Some(app) = with_mirror(|m| m.get(id).map(|n| n.app.clone())).flatten() else {
        return;
    };
    if let Some(name) = launch_name_for(&app, &crate::apps::Catalogue::shared().get()) {
        ui.invoke_launch_app(name.clone().into());
        tracing::debug!(app = %name, "opened the app a notification came from");
    }
}

/// Put the mirror on screen: the list, the badge, and whether the service is answering.
fn resync(ui: &App) {
    MIRROR.with(|slot| {
        if let Some(shared) = slot.borrow().as_ref() {
            notifications::sync_to_ui(&shared.borrow(), &ui.as_weak());
        }
    });
}

/// Raise a toast for one notification, if the screen is allowed to show one right now.
fn maybe_toast(ui: &App, n: &Notification) {
    let critical = n.urgency == Urgency::Critical;

    // A request for approval is stored and counted, and never popped. The card it announces is
    // already on this screen — over every screen, or in the conversation when that is open — and
    // the conversation's copy of the card sits in the bottom right, which is where toasts go: the
    // notice saying "allow or deny it on the card" came up over the card's Allow and Deny.
    // The notification is for whoever is NOT looking at this screen, and they read the centre.
    if n.app == "Yantrik" && n.title.contains(ASKING) {
        return;
    }

    if SILENT_SCREENS.contains(&ui.get_current_screen()) {
        return;
    }
    // Do Not Disturb and focus mode hold everything but critical. Nothing is lost: it is in the
    // store, in the badge and in the notification centre either way.
    if (ui.get_dnd_mode() || ui.get_focus_mode()) && !critical {
        return;
    }

    let actions: Vec<ToastActionData> = n
        .actions
        .iter()
        .filter(|a| a.id != "default")
        .map(|a| ToastActionData {
            id: a.id.clone().into(),
            label: a.label.clone().into(),
        })
        .collect();

    crate::wire::toast::push(
        ui,
        ToastData {
            id: n.id.clone().into(),
            app_name: n.app.clone().into(),
            summary: n.title.clone().into(),
            body: super::toast::brief(&n.body).into(),
            urgency: notifications::urgency_int(n.urgency),
            icon_char: n
                .app
                .chars()
                .next()
                .unwrap_or('N')
                .to_uppercase()
                .to_string()
                .into(),
            actions: ModelRc::new(VecModel::from(actions)),
        },
        notifications::urgency_int(n.urgency),
    );
}

// ── The poll ────────────────────────────────────────────────────────────────────────────────

/// One tick's outcome, as the poll thread hands it to the UI thread.
enum Tick {
    Changed(Since),
    Down(String),
}

fn start_poll(ui: &App) {
    let weak = ui.as_weak();
    let spawned = std::thread::Builder::new()
        .name("yos-notification-poll".into())
        .spawn(move || {
            let mut revision: u64 = 0;
            let mut last_start_attempt: Option<Instant> = None;
            let mut said_down = false;
            loop {
                std::thread::sleep(POLL);

                let tick = match poll_once(revision) {
                    Ok(since) => {
                        revision = since.revision;
                        if said_down {
                            tracing::info!("the notifications service is answering again");
                            said_down = false;
                        }
                        Tick::Changed(since)
                    }
                    Err(why) => {
                        // Ask the shell to start it, but not on every tick: a service that dies
                        // on start would be respawned once a second and its real failure would
                        // be buried under the restarts.
                        let due = last_start_attempt
                            .map(|at| at.elapsed() >= RESTART_BACKOFF)
                            .unwrap_or(true);
                        if due {
                            last_start_attempt = Some(Instant::now());
                            if let Err(e) = yantrik_app_runtime::service::ensure(SERVICE) {
                                tracing::warn!(error = %e, "could not start the notifications service");
                            }
                        }
                        if !said_down {
                            tracing::warn!(
                                error = %why,
                                "the notifications service is not answering; the notification \
                                 centre will say so"
                            );
                            said_down = true;
                        }
                        Tick::Down(why)
                    }
                };

                // `Err` means the event loop has gone — the desktop is shutting down and this
                // thread should go with it rather than spin against a dead window.
                if weak
                    .upgrade_in_event_loop(move |ui| apply_tick(&ui, tick))
                    .is_err()
                {
                    return;
                }
            }
        });

    if let Err(e) = spawned {
        tracing::error!(error = %e, "could not start the notification poll; the desktop will \
                                     show no toasts and an empty notification centre");
    }
}

fn poll_once(revision: u64) -> Result<Since, String> {
    let answer = SyncRpcClient::for_service(SERVICE)
        .with_timeout(POLL_TIMEOUT)
        .call(SINCE, serde_json::json!({ "revision": revision }))
        .map_err(|e| e.message)?;
    serde_json::from_value(answer).map_err(|e| format!("the service answered something else: {e}"))
}

/// On the UI thread: fold the answer into the mirror, raise toasts, redraw.
fn apply_tick(ui: &App, tick: Tick) {
    // One place, once a second, on the UI thread: the companion worker cannot read a Slint
    // property and needs to know whether the conversation is on screen. See `situation_now`.
    LENS_OPEN.store(ui.get_lens_open(), std::sync::atomic::Ordering::Relaxed);
    withdraw_answered_questions(ui);
    match tick {
        Tick::Changed(since) => {
            let applied = with_mirror(|m| m.apply(since)).unwrap_or_default();
            // Dismissed somewhere this file did not see — `yos act notifications dismiss_all`,
            // a mind, a sender closing its own notification — so the toast follows the store
            // off the screen. The shell's own buttons take the toast down on the click and this
            // takes it down again a second later, which is a no-op; for a dismissal that did
            // not start here it is the only way down, and critical toasts have no other.
            for id in &applied.gone {
                crate::wire::toast::remove(ui, id);
            }
            for n in &applied.fresh {
                maybe_toast(ui, n);
            }
            if !applied.fresh.is_empty() {
                relay_to_features(&applied.fresh);
            }
            resync(ui);
        }
        Tick::Down(why) => {
            with_mirror(|m| m.unreachable(why));
            resync(ui);
        }
    }
}

/// Hand each new notification to the shell's own feature pipeline.
///
/// The shell used to learn about notifications from its own D-Bus daemon, and
/// `features::notification_relay` turned each one into an urge — which is how a mind came to
/// hear "Thunderbird says you have mail" without being asked. That daemon is gone: the service
/// owns the bus name now, because two processes cannot. This puts the same event back on the
/// same channel, from the one store, so the relay, the activity feed and the system context all
/// keep working and now see *every* notification rather than the half that reached whichever
/// daemon won the name that boot.
fn relay_to_features(fresh: &[Notification]) {
    OBSERVER.with(|slot| {
        let Some(observer) = slot.borrow().clone() else { return };
        for n in fresh {
            observer.inject(yantrik_os::SystemEvent::NotificationReceived {
                app: n.app.clone(),
                summary: n.title.clone(),
                body: n.body.clone(),
                urgency: notifications::urgency_int(n.urgency) as u8,
            });
        }
    });
}

// ── An update is waiting ────────────────────────────────────────────────────────────────────

/// How long after start to look for an update. Long enough that the check is not competing with
/// everything else a desktop does in its first seconds, short enough that somebody who logs in
/// to read one thing still hears about it.
const UPDATE_CHECK_DELAY: Duration = Duration::from_secs(45);

/// Where the last version we told the person about is remembered.
fn update_marker() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/state")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("yantrik").join("update-notified")
}

/// Tell the person once, per version, that an update is available.
///
/// The About screen could already check, but only when somebody pressed the button on it — so a
/// machine that was never taken to Settings never learned there was anything to install. The
/// rule that matters is **once per new version, not once per boot**: an update the person has
/// decided not to install yet must not re-announce itself every morning, which is how people
/// learn to dismiss update notifications without reading them. The version that was announced
/// is written to a marker file, so this survives a restart of the shell as well as of the
/// machine.
///
/// A check that cannot run — no updater on this machine, no network, an unreadable channel —
/// says nothing at all. It is not news that something could not be checked.
fn watch_for_updates() {
    let _ = std::thread::Builder::new()
        .name("yos-update-notice".into())
        .spawn(|| {
            std::thread::sleep(UPDATE_CHECK_DELAY);
            let crate::control_update::CheckOutcome::UpdateAvailable { from, to, .. } =
                crate::control_update::check_now(None)
            else {
                return;
            };

            let marker = update_marker();
            if std::fs::read_to_string(&marker)
                .map(|seen| seen.trim() == to)
                .unwrap_or(false)
            {
                tracing::debug!(version = %to, "an update is available and was already announced");
                return;
            }

            notify::send(
                notify::Notification::new("Yantrik", format!("Update available — {to}"))
                    .body(format!(
                        "This machine is on {from}. Install it from Settings › About; the \
                         desktop restarts into the new build and rolls back by itself if it \
                         does not start."
                    ))
                    .urgency(Urgency::Low),
            );

            if let Some(dir) = marker.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Err(e) = std::fs::write(&marker, &to) {
                // Not fatal, but worth saying: without the marker this becomes once per boot,
                // which is the thing it exists to avoid.
                tracing::warn!(path = %marker.display(), error = %e,
                    "could not record which version was announced; the update notice may repeat");
            }
            tracing::info!(from = %from, to = %to, "told the person an update is waiting");
        });
}

// ── Sending ─────────────────────────────────────────────────────────────────────────────────

/// Fire one JSON-RPC call at the service and forget it.
///
/// Off the UI thread, always. Every caller here is a Slint callback — a button the person just
/// pressed — and the screen has already been updated optimistically, so there is nothing to wait
/// for. A failure is logged and corrected by the next poll, which re-reads the store.
fn act(method: &'static str, params: serde_json::Value) {
    let _ = std::thread::Builder::new()
        .name("yos-notification-call".into())
        .spawn(move || {
            if let Err(e) = yantrik_app_runtime::service::ensure(SERVICE) {
                tracing::warn!(method, error = %e, "the notifications service is not reachable");
                return;
            }
            if let Err(e) = SyncRpcClient::for_service(SERVICE)
                .with_timeout(Duration::from_secs(2))
                .call(method, params)
            {
                tracing::warn!(method, error = %e.message, "a notification call was refused");
            }
        });
}

/// What marks a notification as "somebody is waiting for an answer". In the title, between who
/// and what, because the title is the one field both the sender and the withdrawal below hold.
const ASKING: &str = " is asking to ";

/// Take the question down once nobody is asking it.
///
/// `approval_waiting` raises its notice as `critical` so that it "stays on screen until it is
/// answered" — and nothing ever took it down when it was. Seventeen minutes after a request had
/// been denied, the desktop still said "Allow or deny it on the card at the top right" about a
/// card that no longer existed, and because a critical toast does not time out it sat over the
/// conversation's text box for as long as the session lasted.
///
/// Reconciled rather than signalled: every way a request can end — allowed, denied, expired,
/// the asking program gone — ends with `approvals::pending()` empty, and this runs on the tick
/// that already runs once a second. While another request is still pending the notices stay;
/// they come down together when the last one is answered.
fn withdraw_answered_questions(ui: &App) {
    if !crate::approvals::pending().is_empty() {
        return;
    }
    let stale: Vec<String> = with_mirror(|m| {
        m.showing()
            .iter()
            .filter(|n| n.app == "Yantrik" && n.title.contains(ASKING))
            .map(|n| n.id.clone())
            .collect()
    })
    .unwrap_or_default();
    if stale.is_empty() {
        return;
    }
    for id in &stale {
        act("notifications.dismiss", serde_json::json!({ "id": id }));
        with_mirror(|m| m.dismiss_locally(id));
        crate::wire::toast::remove(ui, id);
    }
    resync(ui);
}

/// A mind is waiting for an answer.
///
/// Called from `control_approvals` when a request arrives. The card itself is drawn over every
/// screen and raises the shell to the front, but the person may be at another machine, in
/// another room, or looking at a phone — and the request expires unanswered after two minutes,
/// which from their side is indistinguishable from the machine ignoring them.
///
/// `critical`, so it survives Do Not Disturb and stays on screen until it is answered: a
/// question with a deadline is exactly what that level is for.
pub fn approval_waiting(requester: &str, app: &str, action: &str) {
    notify::send(
        notify::Notification::new("Yantrik", format!("{requester}{ASKING}{action}"))
            .body(format!(
                "In {app}. Allow or deny it on the card on this machine's screen — it expires on \
                 its own if nobody answers."
            ))
            .urgency(Urgency::Critical),
    );
}

/// A bypass ran out on its own.
///
/// Only when it LAPSED. A person who pressed `Ask` themselves has just watched the chip change
/// and needs telling nothing; the two people this is for are the one who chose "1 hour" and
/// walked away, and the one sitting in front of the machine when the mind suddenly starts asking
/// again. Both otherwise discover it by being surprised — and the first of them may never learn
/// what the hour bought at all, because the chip is the only thing that changed and they were
/// not looking at it. `mind_mode::take_lapse_notice` is what makes sure this is said once.
///
/// `normal`, not `critical`: it is news, not a question. Critical stays on screen until it is
/// dismissed and survives Do Not Disturb, and a machine that had just become STRICTER holding
/// somebody's screen for it would be the wrong way round.
///
/// The button is dropped when nothing ran, because "See what it did" under "Nothing ran without
/// asking" is a control that contradicts the sentence above it.
pub fn bypass_ended(ended: crate::mind_mode::BypassEnded) {
    let mut notification = notify::Notification::new("Yantrik", "Bypass ended")
        .body(crate::mind_mode::bypass_ended_body(&ended))
        .urgency(Urgency::Normal);
    if ended.unasked > 0 {
        // Routed the way Download Manager's "Open folder" is: the shell presses the named
        // action on the sender's own control surface, and here the sender is the shell.
        // `control_approvals` publishes `show_mind_audit` for exactly this, and
        // `mind_mode_the_tightening_action_is_published` fails the build if it disappears.
        notification = notification.action("show_mind_audit", "See what it did");
    }
    notify::send(notification);
}

/// The mind finished saying something while the Lens was closed.
///
/// The bridge used to raise a private toast for this, which nothing kept: closing it lost the
/// message, and the notification centre never had it. Now it is a notification like any other,
/// and only when the Lens is shut — with it open the answer is already on screen, and a toast
/// over it would be the same sentence twice.
pub fn companion_said(ui: &App, title: &str, text: &str) {
    if ui.get_lens_open() {
        return;
    }
    notify::send(
        notify::Notification::new("Yantrik", title)
            .body(text.chars().take(200).collect::<String>()),
    );
}

/// The built-in companion said something *unprompted* while the Lens was closed, so the person
/// would otherwise never see it.
///
/// This is the proactive sibling of [`companion_said`], and unlike it this one is gated: a thought
/// nobody asked for becomes a notification only when the companion's rule allows it and today's cap
/// is not reached (issue #216, when the companion filed 35 chatty notifications in a day). A
/// result the person *did* ask for — a finished task, an agent that completed — goes through
/// `companion_said` instead and is never gated; news somebody is waiting on is not chatter.
pub fn companion_thought(ui: &App, text: &str) {
    if ui.get_lens_open() {
        return;
    }
    notify_companion_thought(text);
}

// ── Where a proactive message goes ──────────────────────────────────────────────────────────
//
// Observed live, and the reason this section exists. The desktop's answering mind was Hermes.
// A person asked it "Is this machine online, and what is its IP address?" and, while Hermes was
// working, the BUILT-IN companion's serendipity instinct pushed
//
//     Something came to mind — you once said: "User is interested in: technology"
//
// into the same transcript, as an `assistant` message. It reads as the answer to the question.
// A task grader took it as the answer and failed the task; a person would have been just as
// confused. Two faults, and they are independent:
//
// 1. **A mind that is not answering was writing into the conversation.** The transcript belongs
//    to whoever is answering. When that is not the built-in companion, the companion's proactive
//    output is not part of the conversation at all — it is a notification, and it goes to the
//    one store like everything else.
//
// 2. **The Synthesis Gate thought nobody was talking.** `conversation_active` was computed from
//    a timestamp bumped only inside the built-in companion's own `SendMessage` arm, so a message
//    routed to a harness never counted — the gate saw an idle user through a live conversation
//    and let a message through mid-answer. Every message now passes through `dispatch` in
//    `wire::chat`, whichever mind it goes to, and that is where the clock is bumped.
//
// Separately, and not fixed here because the instincts are not this lane: the quoted "memory" is
// an auto-extracted profile stub. "User is interested in: technology" is not worth saying to
// anybody, in a transcript or a toast, and the instinct that chose it
// (`yantrik-companion-instincts/src/serendipity.rs:59`) is choosing badly. Routing it correctly
// makes it quiet; it does not make it worth reading.

/// How long after a person speaks the conversation still counts as live, for the gate.
const CONVERSATION_WINDOW: f64 = 300.0;

/// How long an answer may be in flight before this stops believing it is.
///
/// A ceiling, not a timeout: nothing is cancelled. The stream pump signals its own end, but a
/// harness whose channel is dropped without a `__DONE__` never gets there, and a flag that could
/// stick on would silence every proactive message for the rest of the session.
const ANSWER_CEILING: f64 = 180.0;

static LAST_USER_MESSAGE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static ANSWER_STARTED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static ANSWER_ENDED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static LENS_OPEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// A person just said something to a mind — any mind.
///
/// Called from `wire::chat::dispatch`, which is the one join every typed message passes through
/// on its way to whichever mind is answering. It used to be bumped inside the companion worker's
/// own `SendMessage` arm, which a harness-bound message never reaches.
pub fn note_user_message() {
    LAST_USER_MESSAGE.store(now_secs(), std::sync::atomic::Ordering::Relaxed);
}

/// A mind started producing an answer.
pub fn note_answer_started() {
    ANSWER_STARTED.store(now_secs(), std::sync::atomic::Ordering::Relaxed);
}

/// That answer is finished.
pub fn note_answer_ended() {
    ANSWER_ENDED.store(now_secs(), std::sync::atomic::Ordering::Relaxed);
}

/// Is somebody in the middle of a conversation right now? The Synthesis Gate's question.
pub fn conversation_active() -> bool {
    let last = LAST_USER_MESSAGE.load(std::sync::atomic::Ordering::Relaxed);
    last > 0 && (now_secs().saturating_sub(last) as f64) < CONVERSATION_WINDOW
}

/// Seconds since a person last said anything to any mind. `None` if they never have.
pub fn seconds_since_user_message() -> Option<f64> {
    let last = LAST_USER_MESSAGE.load(std::sync::atomic::Ordering::Relaxed);
    (last > 0).then(|| now_secs().saturating_sub(last) as f64)
}

/// Is a mind working on an answer at this moment?
pub fn answer_in_flight() -> bool {
    let started = ANSWER_STARTED.load(std::sync::atomic::Ordering::Relaxed);
    let ended = ANSWER_ENDED.load(std::sync::atomic::Ordering::Relaxed);
    started > ended && (now_secs().saturating_sub(started) as f64) < ANSWER_CEILING
}

/// Everything that decides where an unprompted message from the built-in companion goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProactiveSituation {
    /// Is the built-in companion the mind currently answering the person?
    pub builtin_is_answering_mind: bool,
    /// Is any mind mid-answer right now?
    pub answer_in_flight: bool,
    /// Is the Lens — the conversation — on screen?
    pub lens_open: bool,
}

/// Where it goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProactiveDelivery {
    /// Into the conversation. `notify` is true when a notification goes out as well, because
    /// the Lens is closed and nobody would otherwise see it.
    Transcript { notify: bool },
    /// A notification and nothing else. The transcript is not ours to write in.
    NotifyOnly,
    /// Nowhere, yet. A mind is mid-answer.
    Hold,
}

/// The rule, as one pure function.
pub fn route_proactive(s: ProactiveSituation) -> ProactiveDelivery {
    // Nothing goes anywhere while a question is being answered — not even a notification. A
    // toast popping over a half-written answer is the same interruption in a different corner,
    // and the instinct that produced this will offer it again.
    if s.answer_in_flight {
        return ProactiveDelivery::Hold;
    }
    // Somebody else's conversation. The companion may still speak, but not in there.
    if !s.builtin_is_answering_mind {
        return ProactiveDelivery::NotifyOnly;
    }
    ProactiveDelivery::Transcript {
        notify: !s.lens_open,
    }
}

/// The situation as it stands. Readable from any thread: the answering mind comes from the
/// harness host's own lock, the rest from atomics, so the companion worker can decide where a
/// message is going before it records having sent it.
pub fn situation_now() -> ProactiveSituation {
    ProactiveSituation {
        builtin_is_answering_mind: crate::wire::harness::host()
            // No host yet means very early boot, when the built-in is the only thing that could
            // be answering.
            .map(|h| h.active_id() == crate::wire::harness::BUILTIN_ID)
            .unwrap_or(true),
        answer_in_flight: answer_in_flight(),
        // A second stale at worst — it is refreshed by the notification poll, which runs on the
        // UI thread once a second. It only ever decides whether a notification goes out
        // *alongside* a transcript message, and `companion_said` re-reads the real property on
        // the UI thread before it does, so the exact answer always wins.
        lens_open: LENS_OPEN.load(std::sync::atomic::Ordering::Relaxed),
    }
}

/// The same rule for something the person actually asked for, which must never be held.
///
/// A finished background task is a result, not a musing: dropping it because a mind happened to
/// be mid-sentence would lose work somebody is waiting on. It still must not be written into
/// another mind's transcript, so `Hold` becomes a notification rather than silence.
pub fn route_result(s: ProactiveSituation) -> ProactiveDelivery {
    match route_proactive(s) {
        ProactiveDelivery::Hold => ProactiveDelivery::NotifyOnly,
        other => other,
    }
}

/// Deliver a proactive message from the built-in companion, wherever it belongs.
///
/// `push_to_transcript` is handed in rather than done here because the transcript lives in a
/// Slint model on the UI thread and this is called from the companion worker; the caller already
/// knows how to make that hop. It runs only for [`ProactiveDelivery::Transcript`].
pub fn deliver_proactive(
    text: &str,
    push_to_transcript: impl FnOnce(bool),
) -> ProactiveDelivery {
    let route = route_proactive(situation_now());
    match route {
        ProactiveDelivery::Hold => {
            tracing::info!(
                text,
                "held a proactive message: a mind is in the middle of an answer"
            );
        }
        // The transcript is not ours to write in, so this thought is a notification or nothing.
        // Issue #216: it becomes one only when the companion's rule allows it and today's cap is
        // not reached. Small talk, machinery and empty findings are dropped here rather than
        // filling the one interruptive channel with chatter.
        ProactiveDelivery::NotifyOnly => {
            notify_companion_thought(text);
        }
        // The Lens is ours: the thought is written into it either way, and the callback decides
        // whether it also raises a notification (see `companion_thought`).
        ProactiveDelivery::Transcript { notify } => push_to_transcript(notify),
    }
    route
}

/// The same, for a result the person is waiting on — see [`route_result`].
pub fn deliver_result(text: &str, push_to_transcript: impl FnOnce(bool)) -> ProactiveDelivery {
    deliver(text, route_result(situation_now()), push_to_transcript)
}

fn deliver(
    text: &str,
    route: ProactiveDelivery,
    push_to_transcript: impl FnOnce(bool),
) -> ProactiveDelivery {
    match route {
        ProactiveDelivery::Hold => {
            tracing::info!(
                text,
                "held a proactive message: a mind is in the middle of an answer"
            );
        }
        ProactiveDelivery::NotifyOnly => {
            tracing::info!(
                text,
                "the answering mind is not the built-in companion, so its proactive message is \
                 a notification and not part of the conversation"
            );
            let (title, body) = headline_and_rest(text);
            notify::send(
                notify::Notification::new("Yantrik Companion", title).body(body).urgency(Urgency::Low),
            );
        }
        ProactiveDelivery::Transcript { notify } => push_to_transcript(notify),
    }
    route
}

// ── The gate on an unprompted companion notification ────────────────────────────────────────
//
// Issue #216. The one door every proactive companion notification goes through, so the rule and
// the daily cap are applied once and cannot be bypassed by one delivery path or the other. The
// content rule itself — actionable, or small talk, or machinery — is a pure function in the
// companion crate (`yantrik_companion::proactive::judge_proactive`), which is where the proactive
// path lives and where it is tested against the messages that were really filed. What is here is
// the two things that belong to the store's side: the cap, which is state, and the send.

/// How many unprompted companion notifications may interrupt in a day. The issue suggested 3; the
/// companion filed 35. Results, approvals and bypass notices never come through here, so the one
/// that matters is not competing with the cap — it only limits the companion's own chatter.
const COMPANION_DAILY_CAP: u32 = 3;

static COMPANION_NOTIF_COUNT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static COMPANION_NOTIF_DAY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Count a companion notification against today's cap. True when it is within the cap (and now
/// counted), false once the cap is reached. Resets itself when the day rolls over.
fn companion_cap_record() -> bool {
    use std::sync::atomic::Ordering::Relaxed;
    let today = now_secs() / 86400;
    if COMPANION_NOTIF_DAY.swap(today, Relaxed) != today {
        COMPANION_NOTIF_COUNT.store(0, Relaxed);
    }
    let mut count = COMPANION_NOTIF_COUNT.load(Relaxed);
    while count < COMPANION_DAILY_CAP {
        match COMPANION_NOTIF_COUNT.compare_exchange_weak(count, count + 1, Relaxed, Relaxed) {
            Ok(_) => return true,
            Err(actual) => count = actual,
        }
    }
    false
}

/// The text of a proactive companion notification, if this thought may become one — cleaned of the
/// markers and emoji a persona adds. `None` when the rule refuses it: small talk, a raw tool call,
/// an internal marker it cannot strip, or idle thinking that found nothing. Pure; the daily cap is
/// the caller's, because the cap is state.
fn companion_notification_text(text: &str) -> Option<String> {
    use yantrik_companion::proactive::{judge_proactive, NotificationVerdict};
    match judge_proactive(text) {
        NotificationVerdict::Notify(cleaned) => Some(cleaned),
        NotificationVerdict::LensOnly | NotificationVerdict::Refuse(_) => None,
    }
}

/// File an unprompted companion thought as a notification, if it may become one. Returns whether a
/// notification was filed. Both delivery routes — the notification-only route and the Lens-closed
/// transcript route — come through here, so the rule and the cap are applied exactly once.
fn notify_companion_thought(text: &str) -> bool {
    let Some(cleaned) = companion_notification_text(text) else {
        tracing::info!(
            text,
            "an unprompted companion thought may not become a notification; leaving it to the Lens"
        );
        return false;
    };
    if !companion_cap_record() {
        tracing::info!(
            cap = COMPANION_DAILY_CAP,
            "companion notification cap reached for today; not filing another"
        );
        return false;
    }
    let (title, body) = headline_and_rest(&cleaned);
    notify::send(
        notify::Notification::new("Yantrik Companion", title)
            .body(body)
            .urgency(Urgency::Low),
    );
    true
}

/// How long a headline may be before it is cut at a word.
const HEADLINE_LEN: usize = 80;

/// How long a *complete first sentence* may be and still stand as the headline whole. Cutting
/// ten characters off the end of a sentence to obey [`HEADLINE_LEN`] reads worse than the
/// sentence does, and the store keeps a title of 200 (`MAX_TITLE`), so there is room.
const SENTENCE_LEN: usize = 120;

/// How much of the rest is kept. The store's bound is 2,000 (`MAX_BODY`); an unprompted thought
/// is a paragraph, and this is room for one without turning a notification into a log dump.
const BODY_LEN: usize = 600;

/// A message written for a conversation, cut to fit a notification: a headline and the rest.
///
/// The first 120 characters used to go in as the title, whatever they were. A companion writes
/// markdown, so a toast read `Ran the check. Here's the read:\n\n**2,114 memories… | Bucket | Count`
/// — asterisks, a table's pipes and two newlines in a one-line title. The headline is the first
/// line that says something, with the markup taken off; what follows it is the body, flattened
/// the same way and cut at a word.
///
/// Then a person found the second half of that fix: a thought the companion wrote as ONE
/// paragraph has no second line, so everything past the cut went nowhere. Notification 61 on
/// 22 September was a 293-character sentence stored as an 89-character title ending in "…" and
/// an empty body; the whole of it survived only in `yantrik-os.log`. The remainder of the first
/// line is now the start of the body — nothing the companion said is dropped on the way in.
fn headline_and_rest(text: &str) -> (String, String) {
    let lines: Vec<String> = text.lines().map(plain_line).filter(|l| !l.is_empty()).collect();
    let Some(first) = lines.first() else {
        return (String::new(), String::new());
    };
    let (title, mut rest) = split_headline(first);
    for line in lines.iter().skip(1) {
        if !rest.is_empty() {
            rest.push(' ');
        }
        rest.push_str(line);
    }
    (title, clip_at_word(&rest, BODY_LEN))
}

/// Split one line into the headline and whatever is left of it.
///
/// Three rules, in order, and the last two both keep the remainder:
///
/// * A line that already fits is the headline. A line break is the writer saying where a thought
///   stops, so `Ran the check. Here's the unvarnished read:` stays in one piece.
/// * Otherwise the first sentence, when it is one line's worth of sentence.
/// * Otherwise as much of it as fits, cut at a word, with an ellipsis to say so.
fn split_headline(line: &str) -> (String, String) {
    if line.chars().count() <= HEADLINE_LEN {
        return (line.to_string(), String::new());
    }
    if let Some(end) = first_sentence_end(line) {
        if line[..end].chars().count() <= SENTENCE_LEN {
            return (line[..end].trim().to_string(), line[end..].trim().to_string());
        }
    }
    let cut = word_cut(line, HEADLINE_LEN);
    let head = line[..cut].trim_end_matches([',', ';', ':', '.', ' ']);
    (format!("{head}…"), line[cut..].trim().to_string())
}

/// Where the first sentence of a line ends, as a byte index just past its full stop.
///
/// A full stop is `.`, `!` or `?` followed by a space or the end of the line, together with
/// anything that closes with it — `?!`, a quote, a bracket. `3.5` and `v1.2` are not sentence
/// ends, because what follows them is not a space. That is as much sentence detection as a
/// notification title has any use for.
fn first_sentence_end(line: &str) -> Option<usize> {
    let mut chars = line.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if !matches!(c, '.' | '!' | '?') {
            continue;
        }
        let mut end = i + c.len_utf8();
        while let Some(&(j, next)) = chars.peek() {
            if matches!(next, '.' | '!' | '?' | '"' | '\'' | '\u{2019}' | '\u{201d}' | ')' | ']') {
                end = j + next.len_utf8();
                chars.next();
            } else {
                break;
            }
        }
        match chars.peek() {
            None => return Some(end),
            Some(&(_, next)) if next.is_whitespace() => return Some(end),
            _ => {}
        }
    }
    None
}

/// The byte index to cut a line at so the headline is about `max` characters and does not end
/// half way through a word.
fn word_cut(line: &str, max: usize) -> usize {
    let hard = line
        .char_indices()
        .nth(max)
        .map(|(i, _)| i)
        .unwrap_or(line.len());
    line[..hard].rfind(' ').unwrap_or(hard)
}

/// One line of markdown as plain words: emphasis, heading and list marks, table pipes and rules
/// removed. Not a markdown parser — a notification has no room for one to matter.
fn plain_line(line: &str) -> String {
    let line = line.trim().trim_start_matches(['#', '>', '-', '*', '|', ' ']);
    let cleaned: String = line
        .replace("**", "")
        .replace("__", "")
        .replace('`', "")
        .replace('|', " · ");
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let cleaned = cleaned.trim_matches([' ', '·']).to_string();
    // A table's rule line (`|---|---|`) is only punctuation once the pipes are gone.
    if cleaned.chars().all(|c| matches!(c, '-' | ':' | '·' | ' ')) {
        return String::new();
    }
    cleaned
}

fn clip_at_word(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max).collect();
    let cut = cut.rsplit_once(' ').map(|(head, _)| head).unwrap_or(&cut);
    format!("{}…", cut.trim_end_matches([',', ';', ':', '.', ' ']))
}

// ── For `describe shell` ────────────────────────────────────────────────────────────────────

/// What the machine is trying to tell the person, for the shell's control surface.
///
/// Read from the mirror rather than the service, because `describe` runs on the UI thread and
/// must not make a socket call there. The mirror is at most a second behind.
pub fn describe_summary() -> serde_json::Value {
    with_mirror(|m| {
        serde_json::json!({
            "unread": m.unread_count(),
            "showing": m.showing().len(),
            "latest": m.latest_for_describe(3),
            // A caller that sees zero unread needs to know whether that is "nothing to say" or
            // "nobody asked".
            "service": match m.notice() {
                Some(why) => serde_json::Value::String(why.to_string()),
                None => serde_json::Value::String("up".to_string()),
            },
        })
    })
    .unwrap_or_else(|| serde_json::json!({ "service": "not wired" }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn situation(builtin: bool, in_flight: bool, lens: bool) -> ProactiveSituation {
        ProactiveSituation {
            builtin_is_answering_mind: builtin,
            answer_in_flight: in_flight,
            lens_open: lens,
        }
    }

    #[test]
    fn a_markdown_message_becomes_a_headline_and_a_body() {
        // The toast that was on screen on 21 September, verbatim.
        let (title, body) = headline_and_rest(
            "Ran the check. Here's the unvarnished read:\n\n**2,114 memories. About 2,091 of them \
             are garbage.**\n\n| Bucket | Count | Verdict |\n|---|---|---|\n| Noise | 2,091 | drop |",
        );
        assert_eq!(title, "Ran the check. Here's the unvarnished read:");
        assert!(body.starts_with("2,114 memories. About 2,091 of them are garbage."), "{body}");
        assert!(!body.contains('*') && !body.contains('|') && !body.contains("---"), "{body}");
        assert!(body.contains("Bucket · Count · Verdict"), "{body}");

        let (title, body) = headline_and_rest("One line.");
        assert_eq!((title.as_str(), body.as_str()), ("One line.", ""));
    }

    #[test]
    fn a_thought_written_as_one_paragraph_keeps_everything_past_the_headline() {
        // Notification 61, verbatim: 293 characters, no line break in it. It was stored as an
        // 89-character title ending in "…" and an empty body — the rest of the sentence existed
        // only in the log.
        let text = "One thing that stood out: your memory graph shows you've set up both a \
                    morning brief and a preference for warm, concise end-of-day reflections \
                    without exclamation marks — so you're quietly building yourself a daily \
                    bookend ritual, which is a more thoughtful habit than most people admit to \
                    having.";
        let (title, body) = headline_and_rest(text);

        assert!(title.chars().count() <= 81 && title.ends_with('…'), "{title}");
        assert!(title.starts_with("One thing that stood out:"), "{title}");
        assert!(!body.is_empty(), "the rest of the sentence went nowhere");
        assert!(body.ends_with("most people admit to having."), "{body}");
        // And between them they hold the whole of it: the body picks up at the word the
        // headline was cut before, so nothing falls down the seam.
        assert_eq!(
            format!("{} {}", title.trim_end_matches('…'), body),
            text,
            "the two halves must add back up to what the companion said"
        );
    }

    #[test]
    fn a_first_sentence_that_reads_as_one_line_becomes_the_headline_whole() {
        // Notification 59 opened with a 264-character sentence — too long for any title — and
        // a second sentence after it.
        let (title, body) = headline_and_rest(
            "Fun one: you've got a \"morning_brief\" notification preference on one side and a \
             \"warm, concise end-of-day reflections, no exclamation marks\" preference on the \
             other — your whole day is bookended by two short briefings. Pretty deliberate \
             rhythm for someone who's into tech.",
        );
        assert!(title.ends_with('…'), "{title}");
        assert!(body.ends_with("someone who's into tech."), "{body}");

        // A sentence a person would read as one line stays in one piece, and what follows it
        // is the body rather than a casualty.
        let (title, body) = headline_and_rest(
            "The backup finished and it took nine minutes, which is about twice as long as \
             usual. Two of the three disks were busy the whole time.",
        );
        assert_eq!(
            title,
            "The backup finished and it took nine minutes, which is about twice as long as usual."
        );
        assert_eq!(body, "Two of the three disks were busy the whole time.");

        // A version number is not a full stop.
        let (title, body) = headline_and_rest(
            "This machine is on 0.1.0-289-gf529880 and the build waiting for it is newer, which \
             is worth a look when there is a moment for it.",
        );
        assert!(title.contains("0.1.0-289-gf529880"), "{title}");
        assert!(!body.is_empty(), "{body}");
    }

    #[test]
    fn a_line_with_no_sentence_in_it_is_cut_at_a_word_and_the_rest_kept() {
        // Every word is the same eight letters, so a cut in the middle of one is visible.
        let long = "alphabet ".repeat(40);
        let (title, body) = headline_and_rest(&long);
        assert!(title.chars().count() <= 81 && title.ends_with('…'), "{title}");
        assert!(title.ends_with("alphabet…"), "cut mid-word: {title}");
        assert!(body.starts_with("alphabet "), "the body starts mid-word: {body}");
        // 359 characters in, 359 characters out, give or take the ellipsis and the space the
        // cut fell on.
        assert!(
            title.chars().count() + body.chars().count() >= long.trim_end().chars().count() - 1,
            "title {} + body {} lost text",
            title.chars().count(),
            body.chars().count()
        );
    }

    #[test]
    fn a_mind_that_is_not_answering_never_writes_in_the_conversation() {
        // The live fault: Hermes was answering "what is this machine's IP address" and the
        // built-in companion's serendipity instinct put `Something came to mind — you once
        // said: "User is interested in: technology"` into the same transcript as an assistant
        // message. It read as the answer. A grader took it as the answer and failed the task.
        assert_eq!(
            route_proactive(situation(false, false, true)),
            ProactiveDelivery::NotifyOnly
        );
        assert_eq!(
            route_proactive(situation(false, false, false)),
            ProactiveDelivery::NotifyOnly
        );
    }

    #[test]
    fn nothing_is_delivered_while_a_mind_is_writing_an_answer() {
        // Not even a notification: a toast over a half-written answer is the same interruption
        // in a different corner. This holds whichever mind is answering.
        for builtin in [true, false] {
            for lens in [true, false] {
                assert_eq!(
                    route_proactive(situation(builtin, true, lens)),
                    ProactiveDelivery::Hold,
                    "builtin={builtin} lens={lens}"
                );
            }
        }
    }

    #[test]
    fn the_built_in_companion_keeps_the_conversation_when_it_is_the_one_answering() {
        // With the Lens open the message is already on screen, so there is nothing to notify
        // about; with it closed the person would otherwise never see it.
        assert_eq!(
            route_proactive(situation(true, false, true)),
            ProactiveDelivery::Transcript { notify: false }
        );
        assert_eq!(
            route_proactive(situation(true, false, false)),
            ProactiveDelivery::Transcript { notify: true }
        );
    }

    #[test]
    fn a_message_to_any_mind_counts_as_conversation_activity() {
        // The Synthesis Gate read a timestamp that only the built-in companion's own message
        // arm bumped, so a conversation with a harness mind looked like an idle user and the
        // gate let proactive messages through mid-answer. Every typed message now passes
        // through `wire::chat::dispatch`, whichever mind it is bound for, and that is what
        // calls this.
        LAST_USER_MESSAGE.store(0, std::sync::atomic::Ordering::Relaxed);
        assert!(!conversation_active(), "nobody has said anything yet");
        assert_eq!(seconds_since_user_message(), None);

        note_user_message();
        assert!(conversation_active());
        assert!(seconds_since_user_message().is_some_and(|s| s < 5.0));

        // And it goes quiet again on its own.
        LAST_USER_MESSAGE.store(
            now_secs() - (CONVERSATION_WINDOW as u64) - 1,
            std::sync::atomic::Ordering::Relaxed,
        );
        assert!(!conversation_active());
    }

    #[test]
    fn an_answer_that_never_reported_its_end_does_not_silence_the_machine_forever() {
        // A harness whose channel is dropped without a `__DONE__` never reaches the pump's end
        // branch. A flag that could stick on would hold every proactive message for the rest of
        // the session, which is worse than the fault it prevents.
        ANSWER_ENDED.store(0, std::sync::atomic::Ordering::Relaxed);
        note_answer_started();
        assert!(answer_in_flight());
        note_answer_ended();
        assert!(!answer_in_flight());

        ANSWER_ENDED.store(0, std::sync::atomic::Ordering::Relaxed);
        ANSWER_STARTED.store(
            now_secs() - (ANSWER_CEILING as u64) - 1,
            std::sync::atomic::Ordering::Relaxed,
        );
        assert!(!answer_in_flight(), "an answer older than the ceiling is not in flight");
    }

    #[test]
    fn a_raw_tool_call_or_an_empty_finding_never_becomes_a_notification() {
        // Issue #216, verbatim: the companion filed its own tool call, and a toast that said
        // there was nothing to say. The rule refuses both, so nothing reaches the store.
        assert_eq!(
            companion_notification_text(
                "recall(query=\"interesting memory connection past conversation event\") → \
                 Nothing to surface right now."
            ),
            None
        );
        assert_eq!(
            companion_notification_text("Nothing actionable right now — no pending tasks."),
            None
        );
        // Small talk is refused too — it belongs in the Lens, not the one interruptive channel.
        assert_eq!(
            companion_notification_text("Hey — how's your day going? Coffee still in hand?"),
            None
        );
    }

    #[test]
    fn an_actionable_notification_arrives_without_markers_or_emoji() {
        // The tool-call shape is refused; an actionable sentence is kept, cleaned of the internal
        // marker the firewall adds and the emoji the persona adds.
        assert_eq!(
            companion_notification_text("The nightly backup failed [unverified]. ☕"),
            Some("The nightly backup failed.".to_string())
        );
        assert_eq!(
            companion_notification_text("Your meeting starts in 15 minutes ☕"),
            Some("Your meeting starts in 15 minutes".to_string())
        );
    }

    #[test]
    fn companion_notifications_stop_at_the_daily_cap() {
        // 35 chatty notifications in one day was the fault; the cap is the backstop on volume, and
        // it counts only the companion's own unprompted thoughts, never a result or an approval.
        COMPANION_NOTIF_COUNT.store(0, std::sync::atomic::Ordering::Relaxed);
        COMPANION_NOTIF_DAY.store(now_secs() / 86400, std::sync::atomic::Ordering::Relaxed);
        assert!(companion_cap_record());
        assert!(companion_cap_record());
        assert!(companion_cap_record());
        assert!(
            !companion_cap_record(),
            "the fourth companion notification of the day is over the cap of {COMPANION_DAILY_CAP}"
        );
    }
}

/// Where a notification's button goes, for our apps and anybody else's alike.
#[cfg(test)]
mod button_route_tests {
    use super::button_route;

    fn installed() -> Vec<crate::apps::DesktopEntry> {
        let mut installed = crate::surfaces::shipped_catalogue();
        installed.push(
            crate::apps::parse_desktop_text(
                "org.example.Mailer",
                "[Desktop Entry]\nType=Application\nName=Mailer\nExec=/usr/bin/mailer\n\
                 X-Yantrik-Surface=mailer\nX-Yantrik-Aliases=post\n",
            )
            .unwrap(),
        );
        installed.push(
            crate::apps::parse_desktop_text(
                "firefox",
                "[Desktop Entry]\nType=Application\nName=Firefox\nExec=/usr/bin/firefox %u\n",
            )
            .unwrap(),
        );
        installed
    }

    #[test]
    fn a_button_reaches_any_app_that_declares_a_surface() {
        let installed = installed();
        let route = |app: &str| button_route(app, &installed);
        // Ours, under the names they send as.
        assert_eq!(route("Downloads"), Some(("download-manager".into(), Some("downloads".into()))));
        assert_eq!(route("Calendar"), Some(("calendar".into(), Some("calendar".into()))));
        assert_eq!(route("container-manager"), Some(("containers".into(), Some("container-manager".into()))));
        // Somebody else's, by its Name and by its alias, opened by that name when it is closed.
        assert_eq!(route("Mailer"), Some(("mailer".into(), Some("mailer".into()))));
        assert_eq!(route("post"), Some(("mailer".into(), Some("post".into()))));
        // The desktop's own buttons reach the desktop, which nothing opens.
        assert_eq!(route("Yantrik"), Some(("shell".into(), None)));
        // A program that declared nothing did not route this notification: no surface to call.
        assert_eq!(route("Firefox"), None);
        assert_eq!(route("browser"), None, "the browser publishes nothing");
        assert_eq!(route("no-such-app"), None);
    }
}
