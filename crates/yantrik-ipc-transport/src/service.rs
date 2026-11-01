//! Reach a service that is only started when something needs it.
//!
//! The machine rail calls calendar, email and notes "on demand", and until recently that was a
//! label on a stopped process rather than a mechanism: nothing started them, so every call an app
//! made to one failed at connect. The calendar could not save an appointment on a freshly booted
//! machine, and said it had.
//!
//! The shell owns service lifetimes through its ServiceManager, so this asks the shell to start
//! one rather than spawning a binary behind its back — otherwise the rail would report "on
//! demand" for a process that is running, which is the same class of lie in the other direction.
//!
//! This lived in `yantrik-app-runtime` while only apps needed it. `yantrik-app-runtime` pulls in
//! Slint, and the built-in companion — which now reaches the calendar the same way an app does —
//! must not carry a toolkit to make a socket call. The logic needs nothing from this crate but
//! [`SyncRpcClient`] and [`RpcServer::default_address`], so it lives here and
//! `yantrik_app_runtime::service` re-exports it unchanged.
//!
//! ## Two ways to start a service, and when each is right
//!
//! An app is a separate process. It has no ServiceManager and cannot have one, so it asks the
//! shell over the shell's own control surface: `app.act start_service`. That is the default
//! below, and it is the path every app under `apps/` takes.
//!
//! The companion is not a separate process. It runs on a worker thread *inside* the shell, a few
//! frames away from the ServiceManager itself. For it, the socket round trip would leave the
//! process only to come back in, and `app.act` is dispatched onto the Slint event loop, so the
//! start would queue behind whatever the compositor thread is doing and is given three seconds
//! there before the caller is told the app did not answer. Neither hop buys anything: the manager
//! is right there, it is `Send + Sync`, and starting a service is a mutex and a `spawn`.
//!
//! So a process that owns the manager installs it here with [`set_local_starter`], and [`ensure`]
//! uses it in place of the round trip. The shell does this in `main` once the manager exists.
//! Nothing else changes: the same function starts the same service through the same manager, and
//! the rail still reads the one list.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use crate::{RpcServer, SyncRpcClient};

/// How long starting a service may take, in total: asking the shell, and then waiting for the
/// socket. A service is a local process opening a socket; if it has not managed that in this
/// long, something is wrong and the caller should hear so.
///
/// Two seconds, because this runs inside control-surface actions and the surface gives an action
/// three on the UI thread before telling the caller the app did not answer — while the work
/// carries on. At the five seconds this used to allow for each half, a slow start would have had
/// the calendar save an appointment and the caller told it had not: the fabricated outcome this
/// module exists to remove, pointing the other way.
const START_TIMEOUT: Duration = Duration::from_secs(2);

/// How often to look for the socket while waiting.
const POLL: Duration = Duration::from_millis(100);

/// Starts a service without leaving the process. Installed by whoever owns the ServiceManager.
type LocalStarter = Box<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

static LOCAL_STARTER: OnceLock<LocalStarter> = OnceLock::new();

/// Teach [`ensure`] to start services directly, for the one process that can.
///
/// Only the shell may call this, and only with its own ServiceManager behind it. Installing
/// anything else here would put service lifetimes in a second place, and the machine rail — which
/// reads the manager — would go back to describing running processes as stopped.
///
/// First call wins, and a second is ignored rather than fatal: a started service is not worth a
/// panic in a shell, and the second closure would have done the same thing as the first.
pub fn set_local_starter<F>(start: F)
where
    F: Fn(&str) -> Result<(), String> + Send + Sync + 'static,
{
    let _ = LOCAL_STARTER.set(Box::new(start));
}

/// True when the service's socket accepts a connection right now.
pub fn is_up(service_id: &str) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::net::UnixStream::connect(RpcServer::default_address(service_id)).is_ok()
    }
    #[cfg(not(unix))]
    {
        std::net::TcpStream::connect(RpcServer::default_address(service_id)).is_ok()
    }
}

/// Make sure `service_id` is running, starting it if it is not.
///
/// Returns `Ok(())` when the service is reachable. Callers that hold the UI thread should treat
/// the error as the answer to show: it names what could not be reached.
pub fn ensure(service_id: &str) -> Result<(), String> {
    if is_up(service_id) {
        return Ok(());
    }

    // One deadline for both halves, so the budget above is the budget.
    let deadline = Instant::now() + START_TIMEOUT;

    match LOCAL_STARTER.get() {
        Some(start) => start(service_id)
            .map_err(|e| format!("could not start the {service_id} service: {e}"))?,
        None => {
            SyncRpcClient::for_service("app-shell")
                .with_timeout(START_TIMEOUT / 2)
                .call(
                    "app.act",
                    serde_json::json!({
                        "action": "start_service",
                        "args": { "name": service_id },
                    }),
                )
                .map_err(|e| {
                    format!(
                        "could not ask the desktop to start the {service_id} service: {}",
                        e.message
                    )
                })?;
        }
    }

    // The manager returns when it has spawned the process; the socket appears a moment later.
    while Instant::now() < deadline {
        if is_up(service_id) {
            // The address failed a moment ago, when the service was genuinely down, and the
            // breaker is holding that verdict for a few seconds more. Drop it: the socket just
            // answered, and the call this start was for is about to be made.
            // Through the client a caller will use: in a mind's process that dials the door, whose
            // key is not the default address (#1026 review).
            SyncRpcClient::for_service(service_id).forget_breaker();
            return Ok(());
        }
        std::thread::sleep(POLL);
    }
    Err(format!("the {service_id} service did not come up"))
}

/// A client for a service, started first if it was not running.
pub fn client(service_id: &str) -> Result<SyncRpcClient, String> {
    ensure(service_id)?;
    Ok(SyncRpcClient::for_service(service_id))
}
