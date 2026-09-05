//! Synchronous JSON-RPC client — for use in non-async contexts (UI wire modules).
//!
//! Uses std::net TCP / Unix streams. No tokio dependency.
//!
//! ## Why this file is defensive about time
//!
//! Every caller of this client is a Slint callback, and a Slint callback runs on the thread that
//! draws the screen. A blocking round-trip here is therefore a blocking round-trip in the
//! compositor: while it waits, the desktop does not paint, does not scroll, and does not respond
//! to the mouse. That is acceptable when the answer arrives in a millisecond over a local socket
//! and catastrophic when it does not.
//!
//! The original version used a thirty-second read timeout and no memory of failure. A service that
//! had crashed, or one wedged mid-request, would freeze the whole shell for thirty seconds — and
//! then freeze it again on the next click, because nothing recorded that the last attempt had just
//! timed out. A single folder click that fans out to three calls paid ninety seconds.
//!
//! Two rules fix that, and the second is the one that matters:
//!
//! **A bounded default.** `DEFAULT_TIMEOUT` is two seconds, not thirty. A local service that
//! cannot answer a UI query in two seconds is not going to become useful in twenty-eight more;
//! the caller wants a failure it can render, not a longer wait. Genuinely slow operations — a
//! mailbox sync, sending a message — opt into a longer budget explicitly via
//! [`SyncRpcClient::with_timeout`], which makes the cost visible at the call site instead of
//! hiding it in a default.
//!
//! **A circuit breaker.** After a connect or read failure the address is marked unreachable for
//! [`BREAKER_COOLDOWN`], and calls during that window fail instantly instead of each re-paying the
//! timeout. This is what turns a dead service from "the desktop is frozen" into "this panel says
//! it is unavailable". The breaker is keyed by address and shared process-wide, because the point
//! is precisely that a *different* call site must not have to rediscover the outage.
//!
//! Neither rule makes a slow call fast. They bound how much of the user's session one can eat.
//! Work that is inherently slow still belongs on a worker thread, not in a callback.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::protocol::{RpcError, RpcRequest, RpcResponse};
use crate::server::RpcServer;

/// How long a UI-thread call may block before it is treated as a failure.
///
/// Two seconds is already far past the point where the desktop feels broken; it exists to absorb a
/// cold service touching disk, not to wait out a network.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(2);

/// How long an address stays marked unreachable after a failure.
///
/// Long enough that a burst of calls from one interaction costs a single timeout rather than one
/// per call; short enough that a service restarting is picked up without the user restarting
/// anything.
pub const BREAKER_COOLDOWN: Duration = Duration::from_secs(5);

/// Addresses known to be failing, and the instant each becomes worth retrying.
fn breakers() -> &'static Mutex<HashMap<String, Instant>> {
    static BREAKERS: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
    BREAKERS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// True if this address is inside its cooldown window and should fail without being contacted.
fn breaker_is_open(address: &str) -> bool {
    let mut map = match breakers().lock() {
        Ok(m) => m,
        // A poisoned lock means some other thread panicked mid-update. That is not a reason to
        // start blocking the UI again, but it is also not a reason to refuse every call forever,
        // so treat it as "no opinion" and let the request through.
        Err(_) => return false,
    };
    match map.get(address) {
        Some(retry_at) if Instant::now() < *retry_at => true,
        // Cooldown elapsed — drop the entry so a recovered service is not re-probed via the map.
        Some(_) => {
            map.remove(address);
            false
        }
        None => false,
    }
}

/// Record that this address just failed, so the next caller does not pay the timeout again.
fn breaker_trip(address: &str) {
    if let Ok(mut map) = breakers().lock() {
        map.insert(address.to_string(), Instant::now() + BREAKER_COOLDOWN);
    }
}

/// Record that this address answered, clearing any cooldown.
fn breaker_reset(address: &str) {
    if let Ok(mut map) = breakers().lock() {
        map.remove(address);
    }
}

/// Blocking RPC client for calling services from synchronous code.
pub struct SyncRpcClient {
    address: String,
    next_id: std::sync::atomic::AtomicU64,
    timeout: Duration,
}

impl SyncRpcClient {
    /// Create a client targeting the given address, with the default UI-safe timeout.
    pub fn new(address: &str) -> Self {
        Self {
            address: address.to_string(),
            next_id: std::sync::atomic::AtomicU64::new(1),
            timeout: DEFAULT_TIMEOUT,
        }
    }

    /// Connect to the default address for a service.
    pub fn for_service(service_id: &str) -> Self {
        Self::new(&RpcServer::default_address(service_id))
    }

    /// Give this client a longer budget than [`DEFAULT_TIMEOUT`].
    ///
    /// Use for operations that are legitimately slow — a mailbox sync, sending mail — and prefer a
    /// worker thread over a large value here: this raises the ceiling on how long the UI thread can
    /// stall, it does not move the work off it.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// The timeout this client will apply to each call.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    fn next_id(&self) -> u64 {
        self.next_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }

    fn unreachable(&self) -> RpcError {
        RpcError {
            code: -32000,
            message: format!(
                "Service at {} is unreachable (retrying in up to {}s)",
                self.address,
                BREAKER_COOLDOWN.as_secs()
            ),
            data: None,
        }
    }

    /// Call a method synchronously and return the result.
    ///
    /// Fails immediately, without touching the socket, if this address failed recently.
    pub fn call(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, RpcError> {
        if breaker_is_open(&self.address) {
            return Err(self.unreachable());
        }

        let id = self.next_id();
        let req = RpcRequest::new(method, params, id);

        let mut req_json = serde_json::to_string(&req).map_err(|e| RpcError {
            code: -32000,
            message: format!("Serialize error: {e}"),
            data: None,
        })?;
        req_json.push('\n');

        // Transport failures trip the breaker; a well-formed error response does not. A service
        // that says "no such folder" is healthy and answering — penalising it would take the panel
        // offline for a routine application error.
        let resp_line = match self.send_receive(&req_json) {
            Ok(line) => {
                breaker_reset(&self.address);
                line
            }
            Err(e) => {
                breaker_trip(&self.address);
                return Err(e);
            }
        };

        let resp: RpcResponse = serde_json::from_str(&resp_line).map_err(|e| RpcError {
            code: -32000,
            message: format!("Response parse error: {e}"),
            data: None,
        })?;

        if let Some(err) = resp.error {
            Err(err)
        } else {
            Ok(resp.result.unwrap_or(serde_json::Value::Null))
        }
    }

    /// Convenience: call with typed params and typed result.
    pub fn call_typed<P: serde::Serialize, R: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        params: &P,
    ) -> Result<R, RpcError> {
        let params_value = serde_json::to_value(params).map_err(|e| RpcError {
            code: -32000,
            message: format!("Param serialize error: {e}"),
            data: None,
        })?;
        let result = self.call(method, params_value)?;
        serde_json::from_value(result).map_err(|e| RpcError {
            code: -32000,
            message: format!("Result deserialize error: {e}"),
            data: None,
        })
    }

    /// Check if the service is reachable.
    pub fn ping(&self) -> bool {
        self.call("rpc.ping", serde_json::Value::Null).is_ok()
    }

    /// Exchange one line with the service, applying `self.timeout` to both directions.
    ///
    /// Read AND write are bounded. A write timeout looks unnecessary — the request is small — but a
    /// peer that has stopped reading fills the socket buffer and blocks the writer just as
    /// effectively as a silent peer blocks the reader.
    #[cfg(unix)]
    fn send_receive(&self, req_json: &str) -> Result<String, RpcError> {
        use std::os::unix::net::UnixStream;

        // A missing socket file fails here immediately with ENOENT, which is the common
        // "service not running" case and costs nothing.
        let stream = UnixStream::connect(&self.address).map_err(|e| RpcError {
            code: -32000,
            message: format!("Connection failed ({}): {e}", self.address),
            data: None,
        })?;

        self.exchange(stream, req_json)
    }

    #[cfg(windows)]
    fn send_receive(&self, req_json: &str) -> Result<String, RpcError> {
        use std::net::{TcpStream, ToSocketAddrs};

        // Unlike a Unix socket path, a TCP connect to a dropped host hangs until the OS gives up,
        // so the connect itself has to be bounded rather than relying on the read timeout.
        let addr = self
            .address
            .to_socket_addrs()
            .map_err(|e| RpcError {
                code: -32000,
                message: format!("Bad address ({}): {e}", self.address),
                data: None,
            })?
            .next()
            .ok_or_else(|| RpcError {
                code: -32000,
                message: format!("Address resolved to nothing ({})", self.address),
                data: None,
            })?;

        let stream = TcpStream::connect_timeout(&addr, self.timeout).map_err(|e| RpcError {
            code: -32000,
            message: format!("Connection failed ({}): {e}", self.address),
            data: None,
        })?;

        self.exchange(stream, req_json)
    }

    /// Write the request and read one response line, with both halves bounded by `self.timeout`.
    ///
    /// Generic over the stream type so the Unix and Windows paths differ only in how they connect.
    fn exchange<S>(&self, mut stream: S, req_json: &str) -> Result<String, RpcError>
    where
        S: Read + Write + Timeouts,
    {
        stream.set_read_timeout(Some(self.timeout)).ok();
        stream.set_write_timeout(Some(self.timeout)).ok();

        stream.write_all(req_json.as_bytes()).map_err(|e| RpcError {
            code: -32000,
            message: format!("Write error ({}): {e}", self.address),
            data: None,
        })?;

        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).map_err(|e| RpcError {
            code: -32000,
            message: format!("Read error ({}): {e}", self.address),
            data: None,
        })?;

        if line.is_empty() {
            return Err(RpcError {
                code: -32000,
                message: "Connection closed before response".into(),
                data: None,
            });
        }

        Ok(line)
    }
}

use std::io::Read;

/// The one thing `UnixStream` and `TcpStream` share that [`SyncRpcClient::exchange`] needs and the
/// standard traits do not cover.
trait Timeouts {
    fn set_read_timeout(&self, dur: Option<Duration>) -> std::io::Result<()>;
    fn set_write_timeout(&self, dur: Option<Duration>) -> std::io::Result<()>;
}

#[cfg(unix)]
impl Timeouts for std::os::unix::net::UnixStream {
    fn set_read_timeout(&self, dur: Option<Duration>) -> std::io::Result<()> {
        std::os::unix::net::UnixStream::set_read_timeout(self, dur)
    }
    fn set_write_timeout(&self, dur: Option<Duration>) -> std::io::Result<()> {
        std::os::unix::net::UnixStream::set_write_timeout(self, dur)
    }
}

impl Timeouts for std::net::TcpStream {
    fn set_read_timeout(&self, dur: Option<Duration>) -> std::io::Result<()> {
        std::net::TcpStream::set_read_timeout(self, dur)
    }
    fn set_write_timeout(&self, dur: Option<Duration>) -> std::io::Result<()> {
        std::net::TcpStream::set_write_timeout(self, dur)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An address that cannot exist, so the first call is guaranteed to fail the transport.
    fn dead_address() -> String {
        #[cfg(unix)]
        {
            "/tmp/yantrik-nonexistent-service-for-tests.sock".to_string()
        }
        #[cfg(windows)]
        {
            // Port 1 is reserved and nothing will be listening on it.
            "127.0.0.1:1".to_string()
        }
    }

    #[test]
    fn default_timeout_is_ui_safe() {
        let c = SyncRpcClient::new(&dead_address());
        assert_eq!(c.timeout(), DEFAULT_TIMEOUT);
        assert!(
            c.timeout() <= Duration::from_secs(2),
            "a UI-thread default above two seconds is a visible freeze"
        );
    }

    #[test]
    fn with_timeout_overrides_the_default() {
        let c = SyncRpcClient::new(&dead_address()).with_timeout(Duration::from_secs(30));
        assert_eq!(c.timeout(), Duration::from_secs(30));
    }

    #[test]
    fn breaker_opens_after_failure_and_short_circuits() {
        // Unique per test run so a shared breaker map cannot leak between tests.
        let addr = format!("{}-{:?}", dead_address(), std::thread::current().id());
        let c = SyncRpcClient::new(&addr);

        assert!(!breaker_is_open(&addr), "breaker starts closed");

        let first = c.call("rpc.ping", serde_json::Value::Null);
        assert!(first.is_err(), "no service is listening");
        assert!(breaker_is_open(&addr), "a transport failure must trip the breaker");

        // The second call must not touch the socket at all — it should be refused by the breaker.
        let started = Instant::now();
        let second = c.call("rpc.ping", serde_json::Value::Null);
        assert!(second.is_err());
        assert!(
            started.elapsed() < Duration::from_millis(100),
            "a call inside the cooldown must fail instantly, took {:?}",
            started.elapsed()
        );

        breaker_reset(&addr);
        assert!(!breaker_is_open(&addr));
    }

    #[test]
    fn breaker_reset_reopens_the_address() {
        let addr = format!("reset-probe-{:?}", std::thread::current().id());
        breaker_trip(&addr);
        assert!(breaker_is_open(&addr));
        breaker_reset(&addr);
        assert!(!breaker_is_open(&addr), "a recovered service must be reachable again");
    }
}
