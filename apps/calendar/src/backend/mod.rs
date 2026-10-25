//! What this app talks to: calendar-service, who is calling, the clock, and the week's first day.
//!
//! `frameworks/yantrik-services` has no calendar client, so the calls stay what they always were —
//! `service::client("calendar")`, which starts the service when it is down, and a plain client for
//! the one question a window never starts the service for — and only move: they run in jobs now,
//! never on the UI thread, except the revision check before `describe` (`follow.rs`). They sit
//! behind [`CalendarApi`] so the tests drive the app against an in-memory store (`fake`) instead of
//! a socket.

use std::sync::Arc;

use serde_json::Value;
use yantrik_ipc_transport::{service, SyncRpcClient};

use crate::today::{Clock, LocalClock};
use crate::views::WeekStart;
use crate::who::{self, Who};

pub mod calls;
#[cfg(test)]
pub mod fake;

/// The service's id, its socket's name.
const SERVICE: &str = "calendar";

/// The code a call is given when the service could not be reached or started at all. Anything
/// but the store's own "no such event" (`-32602`), so a failed read is never taken for "gone".
pub const NOT_REACHED: i32 = -32000;

/// How the service said no: its error code and its sentence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpcFailure {
    pub code: i32,
    pub message: String,
}

/// calendar-service, as this app calls it. Blocking: from a job, or from `describe`'s one check.
pub trait CalendarApi: Send + Sync {
    /// One call, the service started first if it is down.
    fn call(&self, method: &str, params: Value) -> Result<Value, RpcFailure>;

    /// Whether the service is listening right now.
    fn is_up(&self) -> bool;

    /// One call to a service that is already listening; it is never started for this.
    fn call_running(&self, method: &str, params: Value) -> Result<Value, RpcFailure>;
}

/// The live service, over the socket bus. (A test's app talks to `fake` instead.)
#[derive(Debug, Clone, Copy, Default)]
#[cfg_attr(test, allow(dead_code))]
pub struct Service;

impl CalendarApi for Service {
    fn call(&self, method: &str, params: Value) -> Result<Value, RpcFailure> {
        let client = service::client(SERVICE).map_err(|message| RpcFailure { code: NOT_REACHED, message })?;
        client.call(method, params).map_err(|e| RpcFailure { code: e.code, message: e.message })
    }

    fn is_up(&self) -> bool {
        service::is_up(SERVICE)
    }

    fn call_running(&self, method: &str, params: Value) -> Result<Value, RpcFailure> {
        SyncRpcClient::for_service(SERVICE)
            .call(method, params)
            .map_err(|e| RpcFailure { code: e.code, message: e.message })
    }
}

/// Everything outside the app it reads: the store, who is calling, the clock, and the first day
/// of the week. One value, so a test gives the app its fakes in one place.
#[derive(Clone)]
pub struct Machine {
    pub api: Arc<dyn CalendarApi>,
    pub who: Arc<dyn Who>,
    pub clock: Arc<dyn Clock>,
    /// Read once at start: the shell settings file's `week_start`, else the locale. The month
    /// grid, the week view and `show_date` all read this one value.
    pub week_start: WeekStart,
}

impl Machine {
    /// This machine: the live service, the kernel's account of callers, the local clock.
    #[cfg_attr(test, allow(dead_code))]
    pub fn live() -> Machine {
        Machine {
            api: Arc::new(Service),
            who: Arc::new(who::Machine),
            clock: Arc::new(LocalClock),
            week_start: week_start_setting(),
        }
    }
}

/// The week's first day on this machine: `week_start: monday | sunday` in the shell's settings
/// file, else the locale's (`LC_ALL`, `LC_TIME`, `LANG`), else Sunday.
#[cfg_attr(test, allow(dead_code))]
fn week_start_setting() -> WeekStart {
    let text = std::fs::read_to_string(yantrik_app::theme::settings_path()).unwrap_or_default();
    WeekStart::system(&text)
}

/// What the app started at these paths talks to: this machine, or under a test the fakes that
/// test installed for these paths.
#[cfg(not(test))]
pub fn for_app(_paths: &yantrik_app::Paths) -> Machine {
    Machine::live()
}

#[cfg(test)]
pub use fake::for_app;
