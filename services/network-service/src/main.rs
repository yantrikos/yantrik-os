//! Network service — interfaces, connectivity and resolvers from `/proc`, Wi-Fi and the firewall
//! from the tools this OS ships.
//!
//! Methods, all of them named by `yantrik_ipc_contracts::network::method` rather than spelled out
//! here, because a list in a doc comment is exactly what the app's five calls disagreed with:
//!
//! ```text
//!   network.interfaces     {}                        -> Vec<NetworkInterfaceInfo>
//!   network.status         {}                        -> NetworkStatus
//!   network.dns            {}                        -> DnsConfig
//!   network.wifi_state     {}                        -> WifiState
//!   network.wifi_known     {}                        -> Vec<KnownNetwork>
//!   network.firewall       {}                        -> FirewallState
//!   network.dns_set        { servers }               -> DnsSetResult   (re-read)
//!   network.wifi_radio     { enabled }               -> WifiState      (re-read)
//!   network.wifi_scan      { rescan }                -> Vec<ScannedNetwork>
//!   network.wifi_connect   { ssid, password? }       -> WifiState      (re-read)
//!   network.wifi_disconnect{}                        -> WifiState      (re-read)
//!   network.wifi_forget    { ssid }                  -> WifiForgetResult (re-read)
//! ```
//!
//! # The half that was never written
//!
//! `apps/network-manager` called `network.wifi_toggle`, `network.wifi_scan`,
//! `network.wifi_connect`, `network.wifi_disconnect` and `network.wifi_forget`. This service
//! implemented `network.interfaces`, `network.status` and `network.dns` and answered everything
//! else `Unknown method`. Not one name in common. The read half was repaired in an earlier pass —
//! the window used to show "Not connected" on a machine with a routable address because nothing
//! ever asked — and the write half was left exactly as it was, five verbs into a wall.
//!
//! The six write and read methods below are that half. Every one of them re-reads the machine
//! after it acts and answers with what it then saw, and returns nmcli's own first line of stderr
//! when it failed. An action that cannot be verified is an error, not a success: `wifi_radio`
//! with the radio still off afterwards fails, rather than reporting the request back as a result.
//!
//! # No secret reaches a log line
//!
//! The one method that takes a password hands it to nmcli on stdin (see `nmcli::run_with_secret`)
//! and never puts it in `argv`, in an error, or in a trace. `WifiConnectParams` has a hand-written
//! `Debug` so that a `{:?}` cannot leak it either.

mod firewall;
mod nmcli;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use yantrik_ipc_contracts::network::{
    method, ConnectionType, DnsConfig, DnsSetParams, DnsSetResult, FirewallState, KnownNetwork,
    NetworkInterfaceInfo, NetworkStatus, RadioState, ScannedNetwork, WifiConnectParams,
    WifiForgetParams, WifiForgetResult, WifiRadioParams, WifiScanParams, WifiState,
};
#[cfg(test)]
use yantrik_service_sdk::gate::{self, Authority};
use yantrik_service_sdk::prelude::*;
use yantrik_service_sdk::{Action, Param, PeerCred, Surface, View};

use nmcli::Trouble;

/// The id this surface publishes, and the app a grant for one of its actions is bound to.
const APP: &str = "network";

fn main() {
    ServiceBuilder::new("network")
        .handler(NetworkHandler::default())
        .run();
}

struct NetworkHandler {
    /// `app.describe` and `app.act`, dispatched as an app window's are.
    surface: Surface,
    /// The minimum rescan interval, shared by the raw `network.wifi_scan` method and the
    /// surface's `wifi_scan` action: one verb with two doors, one gate between them (#332).
    rescans: Arc<RescanGate>,
}

impl Default for NetworkHandler {
    fn default() -> Self {
        let rescans = Arc::new(RescanGate::new(RESCAN_INTERVAL));
        NetworkHandler { surface: network_surface(rescans.clone()), rescans }
    }
}

/// Read the parameters of one method, naming the method when they do not fit.
///
/// The same helper calendar-service grew for the same reason: a caller that sends the wrong shape
/// hears which method rejected it, instead of the request quietly deserialising to a default.
fn params_for<T: serde::de::DeserializeOwned>(
    method_name: &str,
    params: serde_json::Value,
) -> Result<T, ServiceError> {
    serde_json::from_value(params).map_err(|e| ServiceError {
        code: -32602,
        message: format!("{method_name}: {e}"),
    })
}

impl ServiceHandler for NetworkHandler {
    fn service_id(&self) -> &str {
        "network"
    }

    fn handle(
        &self,
        method_name: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, ServiceError> {
        self.handle_from(method_name, params, None)
    }

    fn handle_from(
        &self,
        method_name: &str,
        params: serde_json::Value,
        peer: Option<PeerCred>,
    ) -> Result<serde_json::Value, ServiceError> {
        // The agent-facing surface: connectivity in one line and a small state object, plus the
        // two Wi-Fi reads that are this socket's to answer. The ceiling and the mode are read
        // per call from the files the shell writes, as an app window's dispatch reads them.
        if let Some(answer) = self.surface.answer(method_name, &params, peer) {
            return answer;
        }
        // The raw methods are the desktop's own plumbing (#332): the Network Manager window
        // calls them by name, and until #43 decides what each one is worth, nobody else gets
        // them at all — `wifi_connect` from here takes a password and `dns_set` rewrites this
        // machine's resolvers, with no grade, no card and no ceiling on any of it. The gate is
        // the kernel's account of the calling process; the graded actions on `app.act` above
        // answer any caller, under the ceiling, the mode and the grant, exactly as before. A
        // method that is not one of this service's is left to the unknown-method answer below,
        // whoever asks: the protocol's `-32601` for a name nobody serves is the checker's
        // (`yos check`) and every caller's to get.
        if method_name.starts_with("network.") {
            if let Err(why) = own_process_only(peer) {
                return Err(ServiceError { code: -32035, message: why });
            }
        }
        match method_name {
            method::INTERFACES => Ok(serde_json::to_value(read_interfaces()?).unwrap()),
            method::STATUS => Ok(serde_json::to_value(read_status()?).unwrap()),
            method::DNS => Ok(serde_json::to_value(read_dns()?).unwrap()),
            method::WIFI_STATE => Ok(serde_json::to_value(wifi_state()).unwrap()),
            method::WIFI_KNOWN => Ok(serde_json::to_value(wifi_known()?).unwrap()),
            method::FIREWALL => Ok(serde_json::to_value(firewall_state()).unwrap()),

            method::WIFI_RADIO => {
                let p: WifiRadioParams = params_for(method_name, params)?;
                Ok(serde_json::to_value(wifi_radio(p.enabled)?).unwrap())
            }
            method::WIFI_SCAN => {
                let p: WifiScanParams = params_for(method_name, params)?;
                Ok(serde_json::to_value(wifi_scan(&self.rescans, p.rescan)?).unwrap())
            }
            method::WIFI_CONNECT => {
                let p: WifiConnectParams = params_for(method_name, params)?;
                Ok(serde_json::to_value(wifi_connect(&p)?).unwrap())
            }
            method::DNS_SET => {
                let p: DnsSetParams = params_for(method_name, params)?;
                Ok(serde_json::to_value(dns_set(&p)?).unwrap())
            }
            method::WIFI_DISCONNECT => Ok(serde_json::to_value(wifi_disconnect()?).unwrap()),
            method::WIFI_FORGET => {
                let p: WifiForgetParams = params_for(method_name, params)?;
                Ok(serde_json::to_value(wifi_forget(&p.ssid)?).unwrap())
            }

            _ => Err(ServiceError {
                code: -1,
                message: format!("Unknown method: {method_name}"),
            }),
        }
    }
}

impl NetworkHandler {
    /// `app.act` under a pinned authority, as the socket's dispatch would run it. The tests' door.
    #[cfg(test)]
    fn act(
        &self,
        params: &serde_json::Value,
        authority: Authority,
    ) -> Result<serde_json::Value, ServiceError> {
        self.surface.act(params, None, authority)
    }
}

// ══════════════════════════════════════════════════════════════════════
// Failures, as codes a caller can branch on
// ══════════════════════════════════════════════════════════════════════

/// One code per distinguishable trouble, so a caller does not have to read English to tell a
/// missing adapter from a wrong password. The sentence is nmcli's wherever nmcli wrote one.
fn service_error(trouble: &Trouble) -> ServiceError {
    let code = match trouble {
        Trouble::NmcliMissing => -32020,
        Trouble::NmcliUnstartable(_) => -32021,
        Trouble::NetworkManagerDown => -32022,
        Trouble::NoWifiAdapter => -32023,
        Trouble::PermissionDenied(_) => -32024,
        Trouble::WrongPassword(_) => -32025,
        Trouble::SsidNotFound(_) => -32026,
        Trouble::TimedOut(_) => -32027,
        Trouble::Said(_) => -32028,
    };
    ServiceError {
        code,
        message: trouble.message(),
    }
}

/// A change that ran without error and did not do what it was asked.
///
/// `docker rm` exiting zero on a container that is still listed is the same shape of bug, and it
/// is the one this whole pass exists to remove: an action must not report what it asked for as
/// what happened.
fn disagreed(what: &str) -> ServiceError {
    ServiceError {
        code: -32029,
        message: what.to_string(),
    }
}

// ══════════════════════════════════════════════════════════════════════
// Wi-Fi
// ══════════════════════════════════════════════════════════════════════

/// Whether this machine has a Wi-Fi adapter, read from the kernel rather than from nmcli.
///
/// `/sys/class/net/<iface>/wireless` exists for a wireless interface and for nothing else. Asked
/// here rather than of NetworkManager because it answers on a machine where NetworkManager is not
/// running, is not installed, or has the device marked unmanaged — and "there is no adapter" and
/// "I could not ask" are the two answers that must never be confused. It is also exactly the test
/// `tests/conformance/probes/network-manager.py` uses for its own ground truth, so the app and the
/// probe are reading the same thing.
fn wifi_adapter_name() -> Option<String> {
    let entries = std::fs::read_dir("/sys/class/net").ok()?;
    let mut found: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let wireless = e.path().join("wireless");
            wireless.exists().then_some(name)
        })
        .collect();
    found.sort();
    found.into_iter().next()
}

/// The device list, or the trouble that stopped it being read.
fn devices() -> Result<Vec<nmcli::Device>, Trouble> {
    let exit = nmcli::run(
        nmcli::QUICK_WAIT_SECS,
        &["-t", "-f", nmcli::DEVICE_FIELDS, "device", "status"],
    );
    nmcli::outcome(&exit).map(|text| nmcli::parse_devices(&text))
}

/// Everything the window and `describe` say about Wi-Fi, gathered once.
///
/// Never returns an error. The absence of an adapter, an absent nmcli and a stopped
/// NetworkManager are all *states of this machine* worth reporting, and turning them into a
/// failed read would put the app back where it started: a blank pane and a default drawn as a
/// fact.
fn wifi_state() -> WifiState {
    let adapter = wifi_adapter_name();
    let Some(device_name) = adapter else {
        return WifiState {
            adapter_present: false,
            reason: Some(Trouble::NoWifiAdapter.message()),
            ..WifiState::default()
        };
    };

    let mut state = WifiState {
        adapter_present: true,
        device: Some(device_name.clone()),
        radio: RadioState::Unknown,
        ..WifiState::default()
    };

    let device_rows = match devices() {
        Ok(rows) => rows,
        Err(trouble) => {
            // The adapter is in the machine and its state could not be read. Both halves are
            // said: `adapter_present` stays true, and the radio stays `unknown` rather than
            // becoming the `false` this app used to draw.
            state.reason = Some(trouble.message());
            return state;
        }
    };

    state.radio = match nmcli::outcome(&nmcli::run(nmcli::QUICK_WAIT_SECS, &["-t", "radio", "wifi"]))
    {
        Ok(text) => match nmcli::parse_radio(&text) {
            Some(true) => RadioState::On,
            Some(false) => RadioState::Off,
            None => RadioState::Unknown,
        },
        Err(trouble) => {
            state.reason = Some(trouble.message());
            RadioState::Unknown
        }
    };

    let device = nmcli::wifi_device(&device_rows);
    // The scan list is read from NetworkManager's cache — no rescan — because this runs on every
    // three-second refresh and a rescan every three seconds would keep the radio off the air.
    let scanned = read_scan_cached(&[]).unwrap_or_default();
    state.connected_ssid = nmcli::connected_ssid(&scanned, device);
    if let Some(row) = scanned.iter().find(|n| n.is_connected) {
        state.signal = Some(row.signal);
        if !row.rate.is_empty() {
            state.rate = Some(row.rate.clone());
        }
    }

    if let Ok(text) = nmcli::outcome(&nmcli::run(
        nmcli::QUICK_WAIT_SECS,
        &["-t", "-f", nmcli::DEVICE_SHOW_FIELDS, "device", "show", device_name.as_str()],
    )) {
        let detail = nmcli::parse_device_show(&text);
        state.ip_address = detail.address;
        state.gateway = detail.gateway;
        state.subnet = detail.prefix.and_then(nmcli::prefix_to_mask);
    }

    state
}

/// The saved SSIDs, as plain strings, for marking the scan list.
fn saved_ssids() -> Vec<String> {
    wifi_known()
        .unwrap_or_default()
        .into_iter()
        .map(|k| k.ssid)
        .collect()
}

/// NetworkManager's cached access-point list, with no rescan.
///
/// `saved` is passed in rather than read here so the caller decides whether the extra
/// `nmcli connection show` is worth it. It is, for the list the window draws, which marks the
/// rows that will be joined without a password; it is not for [`wifi_state`], which runs on every
/// three-second refresh and wants only the in-use row.
fn read_scan_cached(saved: &[String]) -> Result<Vec<ScannedNetwork>, Trouble> {
    let exit = nmcli::run(
        nmcli::QUICK_WAIT_SECS,
        &["-t", "-f", nmcli::SCAN_FIELDS, "device", "wifi", "list"],
    );
    let text = nmcli::outcome(&exit)?;
    Ok(nmcli::parse_scan(&text, saved))
}

/// The SSID one Wi-Fi device is joined to, in as few calls as it takes.
///
/// [`read_status`] is on the same three-second path and needs nothing else about the radio, so it
/// asks this rather than building a whole [`WifiState`]. What it replaces returned `None`
/// unconditionally under a comment saying a future version could use nl80211 — so the header said
/// "WiFi" and never which network, on the one screen whose job is to say which network.
fn connected_ssid_for(device: &str) -> Option<String> {
    let rows = read_scan_cached(&[]).unwrap_or_default();
    if let Some(row) = rows.iter().find(|n| n.is_connected) {
        if !row.ssid.is_empty() {
            return Some(row.ssid.clone());
        }
    }
    // No in-use row in the scan cache: fall back to the profile name on the device, which is the
    // SSID for a profile NetworkManager made and is not for one somebody renamed.
    let text = nmcli::outcome(&nmcli::run(
        nmcli::QUICK_WAIT_SECS,
        &["-t", "-f", "GENERAL.CONNECTION", "device", "show", device],
    ))
    .ok()?;
    nmcli::parse_device_show(&text).connection
}

fn wifi_known() -> Result<Vec<KnownNetwork>, ServiceError> {
    let exit = nmcli::run(
        nmcli::QUICK_WAIT_SECS,
        &["-t", "-f", nmcli::CONNECTION_FIELDS, "connection", "show"],
    );
    nmcli::outcome(&exit)
        .map(|text| nmcli::parse_known(&text))
        .map_err(|t| service_error(&t))
}

/// The adapter, or a refusal naming its absence.
///
/// Every mutation starts here. A machine with no Wi-Fi hardware refuses before nmcli is run at
/// all, which is both faster and the only way to give the caller the one sentence that is
/// actually true about it.
fn require_adapter() -> Result<String, ServiceError> {
    wifi_adapter_name().ok_or_else(|| service_error(&Trouble::NoWifiAdapter))
}

fn wifi_scan(gate: &RescanGate, rescan: bool) -> Result<Vec<ScannedNetwork>, ServiceError> {
    // The interval is taken before the adapter is even asked: what it counts is rescan attempts
    // on this socket, and a loop that only ever reached a missing adapter would still be a loop
    // (#332). `rescan=false` never touches the gate — the cached list costs the radio nothing.
    if rescan {
        gate.take()?;
    }
    require_adapter()?;
    if rescan {
        // A rescan that fails is reported and the cached list is not returned in its place: a
        // stale list presented as the result of a scan is a small version of the same lie.
        let exit = nmcli::run(nmcli::SCAN_WAIT_SECS, &["device", "wifi", "rescan"]);
        nmcli::outcome(&exit).map_err(|t| service_error(&t))?;
    }
    // The saved list is read here and not in `wifi_state`: the window's network list marks the
    // rows that need no password, and this is the one call that wants it.
    let saved = saved_ssids();
    read_scan_cached(&saved).map_err(|t| service_error(&t))
}

// ── The minimum rescan interval (#332) ───────────────────────────────
//
// A rescan puts the radio off the air while it sweeps the bands, so a loop of
// `wifi_scan rescan=true` — on the raw method or on the surface's action — is a loop of
// knocking Wi-Fi off: the person's call drops, their music stops, for as long as the caller
// keeps asking. Ten seconds is the issue's example and the adapter's own sweep bound
// (`SCAN_WAIT_SECS`), which is also what the Network Manager window's scan action defers for;
// a person clicking as fast as they can stays inside it.

/// The minimum time between two rescans this socket runs.
const RESCAN_INTERVAL: Duration = Duration::from_secs(nmcli::SCAN_WAIT_SECS as u64);

/// When the last rescan attempt went out, and how close together attempts may be.
struct RescanGate {
    interval: Duration,
    last: Mutex<Option<Instant>>,
}

impl RescanGate {
    fn new(interval: Duration) -> Self {
        RescanGate { interval, last: Mutex::new(None) }
    }

    /// Take the rescan slot at `now`, or refuse with how long the caller has to wait and the
    /// way around it (`rescan=false`, the cached list). Checked and recorded in one lock, so
    /// two callers racing cannot both take the slot. The refusal is a `ServiceError` with its
    /// own code because it leaves through two doors — the raw method answers it whole, and
    /// the surface answers its sentence — and a caller may branch on either.
    fn take_at(&self, now: Instant) -> Result<(), ServiceError> {
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(previous) = *last {
            let elapsed = now.saturating_duration_since(previous);
            if elapsed < self.interval {
                let wait = (self.interval - elapsed).as_secs() + 1;
                return Err(ServiceError {
                    code: -32034,
                    message: format!(
                        "the radio was asked to rescan {ago} seconds ago and a rescan takes \
                         Wi-Fi off the air while it sweeps: this socket runs one at most every \
                         {interval} seconds. Wait {wait} seconds, or ask with `rescan=false` \
                         and get NetworkManager's cached list now",
                        ago = elapsed.as_secs(),
                        interval = self.interval.as_secs(),
                    ),
                });
            }
        }
        *last = Some(now);
        Ok(())
    }

    fn take(&self) -> Result<(), ServiceError> {
        self.take_at(Instant::now())
    }
}

// ── The raw methods answer the desktop's own programs (#332) ────────

/// Whether `/proc` says this executable is one of this OS's own binaries: `yantrik` itself —
/// the CLI whose `ask` and `serve` run the companion, calendar and network tools included — and
/// every `yantrik-*` program beside it. A binary replaced mid-run reads as
/// `/path/yantrik-x (deleted)`, which still passes — right for a service restarted while a
/// caller holds the socket. A path that is not absolute is not the kernel's answer and is
/// refused like anything else.
fn is_own_binary(exe: &str) -> bool {
    let Some(base) = exe.strip_prefix('/').and_then(|path| path.rsplit('/').next()) else {
        return false;
    };
    let base = base.strip_suffix(" (deleted)").unwrap_or(base);
    base == "yantrik" || base.starts_with("yantrik-")
}

/// The peer check on the raw methods, and the refusals in sentences.
///
/// Same-uid limits stand (#154): this stops accidents and casual impersonation — a script,
/// another user's process, a caller that never says who it is — not hostile code running as
/// the same user, which can be the shell's own child and wear its name.
fn own_process_only(peer: Option<PeerCred>) -> Result<(), String> {
    let Some(peer) = peer else {
        return Err(
            "the kernel would not say which process is calling, and the raw network.* \
             methods answer the desktop's own programs; the graded actions on app.act \
             answer any caller"
                .to_string(),
        );
    };
    let exe = std::fs::read_link(format!("/proc/{}/exe", peer.pid))
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    if is_own_binary(&exe) {
        return Ok(());
    }
    Err(if exe.is_empty() {
        format!(
            "the process calling the raw network.* methods (pid {}) could not be identified \
             from /proc, and those methods answer the desktop's own programs; the graded \
             actions on app.act answer any caller",
            peer.pid
        )
    } else {
        format!(
            "the raw network.* methods answer the desktop's own programs and the process \
             calling is {exe} (pid {}); the graded actions on app.act answer any caller",
            peer.pid
        )
    })
}

fn wifi_radio(enabled: bool) -> Result<WifiState, ServiceError> {
    require_adapter()?;
    let word = if enabled { "on" } else { "off" };
    let exit = nmcli::run(nmcli::QUICK_WAIT_SECS, &["radio", "wifi", word]);
    nmcli::outcome(&exit).map_err(|t| service_error(&t))?;

    // What the machine says now, not what was asked for.
    let state = wifi_state();
    let want = if enabled { RadioState::On } else { RadioState::Off };
    if state.radio != want {
        return Err(disagreed(&format!(
            "nmcli accepted `radio wifi {word}` and the radio reads {} afterwards{}",
            state.radio.as_str(),
            state
                .reason
                .as_ref()
                .map(|r| format!(": {r}"))
                .unwrap_or_default()
        )));
    }
    Ok(state)
}

/// Join a network. The password, when there is one, never touches `argv`.
fn wifi_connect(params: &WifiConnectParams) -> Result<WifiState, ServiceError> {
    require_adapter()?;
    let ssid = params.ssid.trim();
    if ssid.is_empty() {
        return Err(ServiceError {
            code: -32602,
            message: "a network name is needed to connect".to_string(),
        });
    }

    let exit = match params.password.as_deref().filter(|p| !p.is_empty()) {
        // `--ask` plus the secret on stdin. The alternative, `… password <pw>`, leaves the
        // passphrase in /proc/<pid>/cmdline for the twenty-five seconds the connect may run,
        // readable by every other user on the machine.
        Some(secret) => nmcli::run_with_secret(
            nmcli::CONNECT_WAIT_SECS,
            &["device", "wifi", "connect", ssid],
            secret,
        ),
        // No secret: an open network, or one this machine already has credentials for.
        None => nmcli::run(
            nmcli::CONNECT_WAIT_SECS,
            &["device", "wifi", "connect", ssid],
        ),
    };
    nmcli::outcome(&exit).map_err(|t| service_error(&t))?;

    let state = wifi_state();
    match state.connected_ssid.as_deref() {
        Some(joined) if joined == ssid => Ok(state),
        Some(joined) => Err(disagreed(&format!(
            "nmcli reported success for \"{ssid}\" and this machine is on \"{joined}\""
        ))),
        None => Err(disagreed(&format!(
            "nmcli reported success for \"{ssid}\" and this machine is not joined to any network"
        ))),
    }
}

fn wifi_disconnect() -> Result<WifiState, ServiceError> {
    let device = require_adapter()?;
    let before = wifi_state();
    if before.connected_ssid.is_none() {
        return Err(ServiceError {
            code: -32030,
            message: "this machine is not joined to a Wi-Fi network".to_string(),
        });
    }
    let exit = nmcli::run(
        nmcli::QUICK_WAIT_SECS,
        &["device", "disconnect", device.as_str()],
    );
    nmcli::outcome(&exit).map_err(|t| service_error(&t))?;

    let state = wifi_state();
    if let Some(still) = state.connected_ssid.as_deref() {
        return Err(disagreed(&format!(
            "nmcli accepted the disconnect and this machine is still joined to \"{still}\""
        )));
    }
    Ok(state)
}

/// Delete a saved network.
///
/// Refused for the network this machine is currently joined to. `nmcli connection delete` on the
/// active profile takes the link down as a side effect, which would make a `sensitive` action do
/// a `dangerous` thing without saying so. Disconnect first, deliberately, and then forget.
fn wifi_forget(ssid: &str) -> Result<WifiForgetResult, ServiceError> {
    require_adapter()?;
    let ssid = ssid.trim();
    let known = wifi_known()?;
    let Some(entry) = known.iter().find(|k| k.ssid == ssid) else {
        return Err(ServiceError {
            code: -32031,
            message: format!("this machine has no saved network called \"{ssid}\""),
        });
    };
    if entry.is_active {
        return Err(ServiceError {
            code: -32032,
            message: format!(
                "\"{ssid}\" is the network this machine is using; deleting it would take the \
                 connection down. Disconnect first, then forget it."
            ),
        });
    }

    let exit = nmcli::run(
        nmcli::QUICK_WAIT_SECS,
        &["connection", "delete", "uuid", entry.uuid.as_str()],
    );
    nmcli::outcome(&exit).map_err(|t| service_error(&t))?;

    let after = wifi_known()?;
    if after.iter().any(|k| k.ssid == ssid) {
        return Err(disagreed(&format!(
            "nmcli accepted the delete and \"{ssid}\" is still in the saved list"
        )));
    }
    Ok(WifiForgetResult {
        forgotten: ssid.to_string(),
        known: after,
    })
}

// ══════════════════════════════════════════════════════════════════════
// Resolvers
// ══════════════════════════════════════════════════════════════════════

/// glibc's resolver reads at most three `nameserver` lines out of `/etc/resolv.conf` — `MAXNS`
/// in `<resolv.h>`, and it has been 3 for as long as there has been a resolv.conf. A fourth
/// server accepted here would be written into the profile, appear in the readings, and never be
/// asked a question, which is a setting that looks applied and is not.
const MAX_RESOLVERS: usize = 3;

/// The profiles this machine currently has up.
fn active_connections() -> Result<Vec<nmcli::ActiveConnection>, Trouble> {
    let exit = nmcli::run(
        nmcli::QUICK_WAIT_SECS,
        &[
            "-t",
            "-f",
            nmcli::ACTIVE_FIELDS,
            "connection",
            "show",
            "--active",
        ],
    );
    nmcli::outcome(&exit).map(|text| nmcli::parse_active(&text))
}

/// The resolvers NetworkManager says it applied to one device.
fn device_dns(device: &str) -> Vec<String> {
    let exit = nmcli::run(
        nmcli::QUICK_WAIT_SECS,
        &[
            "-t",
            "-f",
            nmcli::DEVICE_DNS_FIELDS,
            "device",
            "show",
            device,
        ],
    );
    nmcli::outcome(&exit)
        .map(|text| nmcli::parse_device_dns(&text))
        .unwrap_or_default()
}

/// Which devices hold an IPv4 gateway. Asked of NetworkManager rather than of `/proc/net/route`
/// so that the answer and the profile it picks out come from the same place.
fn devices_with_gateway(active: &[nmcli::ActiveConnection]) -> Vec<String> {
    active
        .iter()
        .filter(|c| !c.device.is_empty())
        .filter(|c| {
            let exit = nmcli::run(
                nmcli::QUICK_WAIT_SECS,
                &[
                    "-t",
                    "-f",
                    nmcli::DEVICE_SHOW_FIELDS,
                    "device",
                    "show",
                    c.device.as_str(),
                ],
            );
            nmcli::outcome(&exit)
                .map(|text| nmcli::parse_device_show(&text).gateway.is_some())
                .unwrap_or(false)
        })
        .map(|c| c.device.clone())
        .collect()
}

/// Point this machine's resolvers somewhere else, through the profile that owns them.
///
/// # Why this is not a write to `/etc/resolv.conf`
///
/// The companion's `network_dns_set` did `std::fs::write("/etc/resolv.conf", …)` with a `.bak`
/// beside it and answered "DNS set to 1.1.1.1". Two things were wrong with that and both are
/// silent. The first is privilege: a desktop session cannot write that file, so the tool returned
/// its own `Try running as root` and the resolvers were unchanged — or, on the images `deploy/`
/// builds, where the session can become root for anything, it *did* write it, unscoped. The
/// second is ownership, and it survives being root: NetworkManager writes `/etc/resolv.conf`
/// itself from the active profile and rewrites it on the next carrier change, DHCP renew or
/// re-activation. So the good case was a setting with a half-life, and nothing said so.
///
/// Resolvers are a property of the connection profile. `ipv4.dns` on the profile plus
/// `ipv4.ignore-auto-dns yes` is what "use these servers, not the ones the router handed us"
/// means — without the second, NetworkManager keeps the DHCP servers in the list and the caller's
/// choice is merely first, so a resolver the caller thought it had removed still answers.
///
/// # What this costs
///
/// `nmcli connection up` re-activates the profile to apply the change. On the connection carrying
/// the default route that is a brief interruption: the link goes down and comes back with a new
/// lease. That is why the tool in front of this is graded `sensitive` rather than `standard`, and
/// why this is a bad thing to call down the connection you are calling over. `nmcli device
/// reapply` would apply the change in place and is the gentler instrument, but which properties
/// it picks up varies by NetworkManager version and nothing here has run against a live one; `up`
/// is the unambiguous one and the verification below is written against it.
fn dns_set(params: &DnsSetParams) -> Result<DnsSetResult, ServiceError> {
    use std::net::IpAddr;

    let bad = |message: String| ServiceError {
        code: -32602,
        message,
    };

    if params.servers.is_empty() {
        return Err(bad(
            "no DNS servers were given. An empty list is not read as \"clear the resolvers\": \
             leaving the connection carrying the default route with no resolver at all is not \
             something to ask for by omission"
                .to_string(),
        ));
    }
    if params.servers.len() > MAX_RESOLVERS {
        return Err(bad(format!(
            "{} DNS servers were given and glibc's resolver reads at most {MAX_RESOLVERS}; the \
             rest would be stored and never asked",
            params.servers.len()
        )));
    }

    // Parsed as addresses, not pattern-matched. The tool this replaces checked that every
    // character was a digit, a dot or a colon, which accepts `...`, `999.999.999.999` and `:`.
    let mut v4: Vec<String> = Vec::new();
    let mut v6: Vec<String> = Vec::new();
    for server in &params.servers {
        let server = server.trim();
        match server.parse::<IpAddr>() {
            Ok(IpAddr::V4(a)) => v4.push(a.to_string()),
            Ok(IpAddr::V6(a)) => v6.push(a.to_string()),
            Err(_) => {
                return Err(bad(format!(
                    "\"{server}\" is not an IP address. A DNS server is named by address here, \
                     not by hostname — a hostname would have to be resolved by the resolver this \
                     call is about to change"
                )))
            }
        }
    }

    let active = active_connections().map_err(|t| service_error(&t))?;
    let gateways = devices_with_gateway(&active);
    let Some(target) = nmcli::resolver_connection(&active, &gateways) else {
        return Err(ServiceError {
            code: -32033,
            message: "this machine has no active network connection to set resolvers on"
                .to_string(),
        });
    };

    // `modify uuid <uuid>`, never `modify <name>`: a Wi-Fi profile is named after its SSID and an
    // SSID may begin with a dash or contain a space.
    let joined_v4 = v4.join(" ");
    let joined_v6 = v6.join(" ");
    let mut argv: Vec<&str> = vec!["connection", "modify", "uuid", target.uuid.as_str()];
    if !v4.is_empty() {
        argv.extend_from_slice(&["ipv4.dns", joined_v4.as_str(), "ipv4.ignore-auto-dns", "yes"]);
    }
    if !v6.is_empty() {
        argv.extend_from_slice(&["ipv6.dns", joined_v6.as_str(), "ipv6.ignore-auto-dns", "yes"]);
    }
    let exit = nmcli::run(nmcli::QUICK_WAIT_SECS, &argv);
    nmcli::outcome(&exit).map_err(|t| service_error(&t))?;

    // Stored is not applied. The profile now says so on disk and the running link does not.
    let exit = nmcli::run(
        nmcli::APPLY_WAIT_SECS,
        &["connection", "up", "uuid", target.uuid.as_str()],
    );
    nmcli::outcome(&exit).map_err(|t| service_error(&t))?;

    let resolv_conf = read_dns()?;
    let applied = device_dns(&target.device);

    // Every server asked for has to turn up in one of the two readings, or this failed. A
    // `connection up` that exits zero having re-activated the profile without the new resolvers —
    // because the connection is shared with another setting, because a stub resolver sits in
    // front, because NetworkManager kept an old applied connection — is a success message over an
    // unchanged machine, which is the shape of bug this whole pass exists to remove.
    let missing: Vec<&String> = params
        .servers
        .iter()
        .filter(|wanted| {
            let wanted = wanted.trim();
            !applied.iter().any(|s| s == wanted)
                && !resolv_conf.nameservers.iter().any(|s| s == wanted)
        })
        .collect();
    if !missing.is_empty() {
        return Err(disagreed(&format!(
            "nmcli accepted the change and {} is not among this machine's resolvers afterwards. \
             /etc/resolv.conf says [{}]; NetworkManager says the {} device has [{}]",
            missing
                .iter()
                .map(|s| format!("\"{s}\""))
                .collect::<Vec<_>>()
                .join(", "),
            resolv_conf.nameservers.join(", "),
            target.device,
            applied.join(", ")
        )));
    }

    Ok(DnsSetResult {
        connection: target.name.clone(),
        device: target.device.clone(),
        resolv_conf,
        device_dns: applied,
    })
}

// ══════════════════════════════════════════════════════════════════════
// Firewall
// ══════════════════════════════════════════════════════════════════════

/// Which firewall is on this machine and what it is doing — read, never assumed.
///
/// Like [`wifi_state`] this never fails: "there is no firewall tool here" and "I was not allowed
/// to read the ruleset" are both answers, and the one thing that must not come back is a
/// confident `false`.
fn firewall_state() -> FirewallState {
    for (kind, binary) in firewall::CANDIDATES {
        let exit = match kind {
            "nftables" => firewall::run(binary, &["list", "ruleset"]),
            "ufw" => firewall::run(binary, &["status", "verbose"]),
            _ => firewall::run(binary, &["--state"]),
        };
        // A tool that is not installed is not this machine's firewall; try the next one.
        if matches!(exit, nmcli::Exit::Missing) {
            continue;
        }
        return match kind {
            "nftables" => firewall::read_nftables(&exit),
            "ufw" => firewall::read_ufw(&exit),
            _ => firewall::read_firewalld(&exit),
        };
    }
    // None of the three is installed. `absent`, with the list of what was looked for, comes back
    // from any of the three readers given a `Missing`; nftables is asked for the sentence.
    firewall::read_nftables(&nmcli::Exit::Missing)
}

// ══════════════════════════════════════════════════════════════════════
// Control surface (app.describe / app.act)
// ══════════════════════════════════════════════════════════════════════

/// The surface this socket answers `app.describe` and `app.act` with.
///
/// The two Wi-Fi reads are published because this service already does them, exactly as the
/// window's buttons do — a caller with no window open (`yos`, a mind) gets the same scan and the
/// same saved list. The verbs that change the machine's network state stay on the Network
/// Manager app's own surface (`app-network`), and not only for the reason that comment used to
/// give (#179's defect was publishing a surface with nothing true on it): disconnecting and
/// radio-off cannot be undone over the channel they close, so they are graded `dangerous` and
/// belong where the person whose Wi-Fi goes off is looking at the card (`docs/sdk/grades.md`).
fn network_surface(rescans: Arc<RescanGate>) -> Surface {
    Surface::new(APP)
        .socket_name("network")
        .describe(|| {
            describe_view().unwrap_or_else(|e| {
                View::new(format!(
                    "Network — could not read this machine's connectivity ({})",
                    e.message
                ))
                .with("error", e.message)
            })
        })
        .action(wifi_scan_action(), move |args| {
            let rescan = args["rescan"].as_bool().unwrap_or(false);
            wifi_scan(&rescans, rescan)
                .map(|networks| serde_json::to_value(networks).unwrap())
                .map_err(|e| e.message)
        })
        .action(wifi_known_action(), |_| {
            wifi_known()
                .map(|networks| serde_json::to_value(networks).unwrap())
                .map_err(|e| e.message)
        })
}

/// See the note on [`describe_view`] for why exactly these two and not the others.
#[cfg(test)]
fn network_actions() -> Vec<Action> {
    vec![wifi_scan_action(), wifi_known_action()]
}

/// The grade this surface publishes for `action`, from the same table `describe` hands out, so
/// the grade a caller is shown and the grade that is enforced cannot come apart.
#[cfg(test)]
fn published_grade(action: &str) -> Option<&'static str> {
    network_actions().into_iter().find(|a| a.name == action).map(|a| a.permission)
}

/// The networks around this machine, optionally asking the radio for a fresh scan first.
///
/// `standard` — the floor for anything that reaches outside the process (nmcli, NetworkManager),
/// and the same grade the Network Manager window publishes this verb at: a scan puts the radio
/// off the air for a few seconds and changes nothing (`docs/sdk/grades.md`).
fn wifi_scan_action() -> Action {
    Action::new("wifi_scan", "List the Wi-Fi networks this machine can see")
        .arg(
            Param::flag("rescan")
                .describe("Ask the radio for a fresh list first, instead of answering from \
                           NetworkManager's cache. A rescan takes Wi-Fi off the air while it \
                           sweeps, so this socket runs one at most every ten seconds and \
                           refuses a sooner one; the cached list is always there (#332)")
                .optional(),
        )
        .expected_seconds(10)
}

/// The networks saved on this machine — the ones joining needs no key for.
///
/// `safe`: one `nmcli connection show`, a local read that does not touch the radio at all.
fn wifi_known_action() -> Action {
    Action::new("wifi_known", "List the Wi-Fi networks saved on this machine").risk("safe")
}

/// Connectivity as data: "am I online, and how", plus interfaces, resolvers, Wi-Fi and firewall.
fn describe_view() -> Result<View, ServiceError> {
    let status = read_status()?;
    let ifaces = read_interfaces().unwrap_or_default();
    let dns = read_dns().ok();
    let wifi = wifi_state();
    let fw = firewall_state();

    let summary = if status.connected {
        let where_ = status.ssid.clone().unwrap_or_else(|| status.conn_type.clone());
        let ip = status.ip_address.clone().unwrap_or_else(|| "no address".to_string());
        format!("Network — online via {where_}, {ip}")
    } else {
        "Network — offline".to_string()
    };

    let interfaces: Vec<serde_json::Value> = ifaces
        .iter()
        // Loopback is never the answer to "how am I connected"; drop it from the glance.
        .filter(|i| i.name != "lo")
        .map(|i| {
            serde_json::json!({
                "name": i.name,
                "type": i.conn_type.as_str(),
                "state": i.state,
                "ip": i.ip_address,
                "mac": i.mac_address,
            })
        })
        .collect();

    let mut view = View::new(summary)
        .with("connected", status.connected)
        .with("type", status.conn_type)
        .with("ssid", status.ssid.map(serde_json::Value::String).unwrap_or(serde_json::Value::Null))
        .with("ip_address", status.ip_address.map(serde_json::Value::String).unwrap_or(serde_json::Value::Null))
        .with("interfaces", serde_json::Value::Array(interfaces))
        .with("wifi", serde_json::to_value(&wifi).unwrap_or(serde_json::Value::Null))
        .with("firewall", serde_json::to_value(&fw).unwrap_or(serde_json::Value::Null));
    if let Some(dns) = dns {
        view = view
            .with("nameservers", serde_json::json!(dns.nameservers))
            .with("search_domains", serde_json::json!(dns.search_domains));
    }
    Ok(view)
}

// ══════════════════════════════════════════════════════════════════════
// Linux implementation (reads /proc, /sys, /etc)
// ══════════════════════════════════════════════════════════════════════

#[cfg(unix)]
mod platform {
    use super::*;

    /// Read network interfaces from /proc/net/dev and enrich with /sys metadata.
    pub fn read_interfaces() -> Result<Vec<NetworkInterfaceInfo>, ServiceError> {
        let content = std::fs::read_to_string("/proc/net/dev").map_err(|e| ServiceError {
            code: -32000,
            message: format!("Cannot read /proc/net/dev: {e}"),
        })?;

        let mut interfaces = Vec::new();

        for line in content.lines().skip(2) {
            let line = line.trim();
            let (name, rest) = match line.split_once(':') {
                Some(pair) => pair,
                None => continue,
            };
            let name = name.trim();
            if name == "lo" {
                continue;
            }

            let values: Vec<u64> = rest
                .split_whitespace()
                .filter_map(|s| s.parse().ok())
                .collect();
            if values.len() < 10 {
                continue;
            }

            let rx_bytes = values[0];
            let tx_bytes = values[8];

            let mac_address = read_sys_attr(name, "address");
            let operstate = read_sys_attr(name, "operstate");
            let state = if operstate.is_empty() {
                "unknown".to_string()
            } else {
                operstate
            };

            let conn_type = detect_interface_type(name);
            let ip_address = read_interface_ip(name);

            interfaces.push(NetworkInterfaceInfo {
                name: name.to_string(),
                mac_address,
                ip_address,
                rx_bytes,
                tx_bytes,
                state,
                conn_type,
            });
        }

        Ok(interfaces)
    }

    /// Determine overall connectivity status.
    pub fn read_status() -> Result<NetworkStatus, ServiceError> {
        let interfaces = read_interfaces()?;

        for iface in &interfaces {
            if iface.state == "up" && iface.ip_address.is_some() {
                // Read through NetworkManager now, rather than the `None` the stub this replaces
                // returned for every machine. See `connected_ssid_for`.
                let ssid = if matches!(iface.conn_type, ConnectionType::Wifi) {
                    super::connected_ssid_for(&iface.name)
                } else {
                    None
                };

                return Ok(NetworkStatus {
                    connected: true,
                    conn_type: iface.conn_type.as_str().to_string(),
                    ssid,
                    ip_address: iface.ip_address.clone(),
                });
            }
        }

        Ok(NetworkStatus {
            connected: false,
            conn_type: "none".to_string(),
            ssid: None,
            ip_address: None,
        })
    }

    /// Read DNS configuration from /etc/resolv.conf.
    pub fn read_dns() -> Result<DnsConfig, ServiceError> {
        let content = std::fs::read_to_string("/etc/resolv.conf").unwrap_or_default();

        let mut nameservers = Vec::new();
        let mut search_domains = Vec::new();

        for line in content.lines() {
            let line = line.trim();
            if line.starts_with('#') || line.is_empty() {
                continue;
            }

            if let Some(rest) = line.strip_prefix("nameserver") {
                let ns = rest.trim();
                if !ns.is_empty() {
                    nameservers.push(ns.to_string());
                }
            } else if let Some(rest) = line.strip_prefix("search") {
                for domain in rest.split_whitespace() {
                    search_domains.push(domain.to_string());
                }
            }
        }

        Ok(DnsConfig {
            nameservers,
            search_domains,
        })
    }

    /// Read a sysfs attribute for a network interface.
    fn read_sys_attr(iface: &str, attr: &str) -> String {
        std::fs::read_to_string(format!("/sys/class/net/{iface}/{attr}"))
            .map(|s| s.trim().to_string())
            .unwrap_or_default()
    }

    /// Detect interface type from name conventions and sysfs.
    fn detect_interface_type(name: &str) -> ConnectionType {
        // Check sysfs type field (1 = ethernet, 801 = wifi, etc.)
        if let Ok(content) = std::fs::read_to_string(format!("/sys/class/net/{name}/type")) {
            if content.trim() == "801" {
                return ConnectionType::Wifi;
            }
        }

        // Check if wireless directory exists
        if std::path::Path::new(&format!("/sys/class/net/{name}/wireless")).exists() {
            return ConnectionType::Wifi;
        }

        // Fall back to name-based heuristics
        if name.starts_with("wl") || name.starts_with("wlan") {
            ConnectionType::Wifi
        } else if name.starts_with("eth")
            || name.starts_with("en")
            || name.starts_with("eno")
            || name.starts_with("ens")
        {
            ConnectionType::Ethernet
        } else if name.starts_with("tun") || name.starts_with("tap") || name.starts_with("wg") {
            ConnectionType::Vpn
        } else if name.starts_with("br") || name.starts_with("docker") || name.starts_with("virbr")
        {
            ConnectionType::Bridge
        } else {
            ConnectionType::Other(name.to_string())
        }
    }

    /// The interface's IPv4 address, via `SIOCGIFADDR`, without shelling out.
    fn read_interface_ip(name: &str) -> Option<String> {
        get_ipv4_addr(name)
    }

    /// Get IPv4 address for an interface using libc ioctl.
    fn get_ipv4_addr(iface_name: &str) -> Option<String> {
        use std::mem;
        use std::os::unix::io::RawFd;

        let sock: RawFd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0) };
        if sock < 0 {
            return None;
        }

        let mut ifr: libc::ifreq = unsafe { mem::zeroed() };
        let name_bytes = iface_name.as_bytes();
        let copy_len = name_bytes.len().min(libc::IFNAMSIZ - 1);
        unsafe {
            std::ptr::copy_nonoverlapping(
                name_bytes.as_ptr(),
                ifr.ifr_name.as_mut_ptr() as *mut u8,
                copy_len,
            );
        }

        let result = unsafe { libc::ioctl(sock, libc::SIOCGIFADDR as _, &mut ifr) };
        unsafe {
            libc::close(sock);
        }

        if result < 0 {
            return None;
        }

        let addr = unsafe { ifr.ifr_ifru.ifru_addr };
        // `sa_family` is `u8` on macOS/BSD and `u16` on Linux — compare via `u32`.
        if addr.sa_family as u32 != libc::AF_INET as u32 {
            return None;
        }

        let sin: libc::sockaddr_in = unsafe { mem::transmute(addr) };
        let ip = sin.sin_addr.s_addr.to_ne_bytes();
        Some(format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]))
    }
}

// ══════════════════════════════════════════════════════════════════════
// Windows stub (for compilation only — service runs on Linux)
// ══════════════════════════════════════════════════════════════════════

#[cfg(not(unix))]
mod platform {
    use super::*;

    pub fn read_interfaces() -> Result<Vec<NetworkInterfaceInfo>, ServiceError> {
        Ok(Vec::new())
    }

    pub fn read_status() -> Result<NetworkStatus, ServiceError> {
        Ok(NetworkStatus::default())
    }

    pub fn read_dns() -> Result<DnsConfig, ServiceError> {
        Ok(DnsConfig::default())
    }
}

fn read_interfaces() -> Result<Vec<NetworkInterfaceInfo>, ServiceError> {
    platform::read_interfaces()
}

fn read_status() -> Result<NetworkStatus, ServiceError> {
    platform::read_status()
}

fn read_dns() -> Result<DnsConfig, ServiceError> {
    platform::read_dns()
}

// The service only answers Wi-Fi on Linux (`platform` says so on Windows), so the surface tests
// that would call a handler run only there; the gate mechanics they pin are the same everywhere.
#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// A machine at `ceiling`, in `mode`, with no grant spent — the authority pinned per case
    /// rather than inherited from whatever files the machine running the tests has.
    fn at(ceiling: &str, mode: &str) -> Authority {
        Authority { ceiling: ceiling.into(), mode: gate::Mode::named(mode), granted: false }
    }

    /// `app.act` for `wifi_scan` with `args`, and a grant id beside it when the test says one was
    /// spent for the call.
    fn act_wifi_scan(args: serde_json::Value, grant: Option<&str>) -> serde_json::Value {
        let mut params = serde_json::json!({ "action": "wifi_scan", "args": args });
        if let Some(grant) = grant {
            params["grant"] = grant.into();
        }
        params
    }

    /// Grants the stand-in shell spent. `ok-*` holds, anything else is refused in its words.
    static SPENT: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    /// What each grant was spent against, as the shell would have been handed it.
    static SPENT_AGAINST: std::sync::Mutex<Vec<(String, String)>> = std::sync::Mutex::new(Vec::new());

    fn spend_through_a_stand_in_shell() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            gate::spend_grants_with(|id, _app, _action, args| {
                if !id.starts_with("ok-") {
                    return Err(format!("no approval request `{id}`."));
                }
                SPENT.lock().unwrap_or_else(|e| e.into_inner()).push(id.to_string());
                SPENT_AGAINST.lock().unwrap_or_else(|e| e.into_inner()).push((id.to_string(), args.to_string()));
                Ok(())
            });
        });
    }

    /// `wifi_scan` is `standard`, and `standard` needs no grant in any mode, plan included: the
    /// only mode-independent proof is that the gate lets it through, since what the handler
    /// answers next is this machine's Wi-Fi, not a sentence the test can pin. `wifi_known` is
    /// `safe` — under every ceiling there is — so the ceiling test uses `wifi_scan`.
    #[test]
    fn the_published_grades_are_what_describe_shows() {
        assert_eq!(published_grade("wifi_scan"), Some("standard"));
        assert_eq!(published_grade("wifi_known"), Some("safe"));

        let described = NetworkHandler::default()
            .handle("app.describe", serde_json::json!({}))
            .expect("describe");
        let actions = described["actions"].as_array().expect("actions");
        assert_eq!(actions.len(), network_actions().len());
        for a in actions {
            let name = a["name"].as_str().unwrap();
            assert_eq!(a["permission"].as_str(), published_grade(name), "{name}");
        }
    }

    /// The ceiling binds this door as it binds every app's: a machine set to `safe` refuses
    /// `wifi_scan` on the grade alone, grant or none — and a grant it refused was never offered
    /// to the shell (#154). The handler is never reached, so no nmcli runs either way.
    #[test]
    fn a_ceiling_of_safe_refuses_wifi_scan_whatever_the_grant() {
        spend_through_a_stand_in_shell();
        for grant in [None, Some("ok-179-network")] {
            let err = NetworkHandler::default()
                .act(&act_wifi_scan(serde_json::json!({}), grant), at("safe", "bypass"))
                .unwrap_err();
            assert!(
                err.message.starts_with("CEILING: network.wifi_scan is graded `standard`"),
                "grant={grant:?}: {}",
                err.message
            );
            assert_eq!(err.code, -32602);
        }
        let spent = SPENT.lock().unwrap_or_else(|e| e.into_inner());
        assert!(!spent.iter().any(|id| id == "ok-179-network"), "spent above the ceiling: {spent:?}");
    }

    /// As on a window: a grant that rides on a call is spent past the ceiling, and one the shell
    /// refuses ends the call in the shell's words — before the handler, so no scan runs.
    #[test]
    fn a_grant_that_does_not_hold_ends_the_call() {
        spend_through_a_stand_in_shell();
        let err = NetworkHandler::default()
            .act(&act_wifi_scan(serde_json::json!({}), Some("made-up")), at("sensitive", "ask"))
            .unwrap_err();
        assert!(
            err.message.starts_with("GRANT: `made-up` does not authorise network.wifi_scan"),
            "{}",
            err.message
        );
    }

    /// An agent token travels beside `args`, never among them: one a caller put among them is
    /// taken out before the grant is spent, so the shell is handed the arguments alone. The stale
    /// revision is here to stop the call at the guard, before any scan, whatever the machine has.
    #[test]
    fn an_agent_token_among_the_arguments_is_not_what_a_grant_is_bound_to() {
        spend_through_a_stand_in_shell();
        let params = serde_json::json!({
            "action": "wifi_scan",
            "args": { "agent_token": "smuggled" },
            "agent_token": "tok-7f3a",
            "grant": "ok-179-token",
            "expect_revision": "0000000000000000",
        });
        let err = NetworkHandler::default().act(&params, at("sensitive", "ask")).unwrap_err();
        assert!(err.message.starts_with("STALE: this app is at revision "), "{}", err.message);
        let against = SPENT_AGAINST.lock().unwrap_or_else(|e| e.into_inner());
        let (_, args) = against.iter().find(|(id, _)| id == "ok-179-token").expect("the grant was spent");
        assert_eq!(args, "{}");
    }

    /// Every action `describe` offers reaches a handler past the gate, and an action it does not
    /// offer is answered as that before any grant is looked at — as an app window answers it:
    /// -32602, in the dispatch's words. What a handler then answers is the machine's (`nmcli`
    /// missing, no adapter, a real list), so the test only pins that the gate let it through.
    #[test]
    fn every_published_action_reaches_a_handler() {
        for spec in network_actions() {
            let outcome = NetworkHandler::default().act(
                &serde_json::json!({ "action": spec.name, "args": {} }),
                at("dangerous", "bypass"),
            );
            if let Err(err) = outcome {
                for prefix in ["CEILING:", "GRANT:", "STALE:", "unknown action"] {
                    assert!(!err.message.starts_with(prefix), "{}: {}", spec.name, err.message);
                }
            }
        }
        let err = NetworkHandler::default()
            .act(
                &serde_json::json!({ "action": "wifi_connect", "args": {}, "grant": "made-up" }),
                at("dangerous", "ask"),
            )
            .unwrap_err();
        assert_eq!(err.code, -32602);
        assert_eq!(err.message, "unknown action `wifi_connect`; this app offers: wifi_scan, wifi_known");
    }

    /// Arguments are checked on this door as on every app's — by name and by declared type, in
    /// the dispatch's words, before the handler runs.
    #[test]
    fn the_arguments_are_checked_as_an_apps_are() {
        let handler = NetworkHandler::default();

        let err = handler
            .act(&serde_json::json!({ "action": "wifi_known", "args": { "ssid": "x" } }), at("sensitive", "ask"))
            .unwrap_err();
        assert_eq!(err.code, -32602);
        assert_eq!(err.message, "`wifi_known` takes no arguments, but `ssid` was given");

        let err = handler
            .act(&serde_json::json!({ "action": "wifi_scan", "args": { "city": "Dallas" } }), at("sensitive", "ask"))
            .unwrap_err();
        assert_eq!(err.message, "`wifi_scan` has no argument `city`; it takes: rescan");

        let err = handler
            .act(&act_wifi_scan(serde_json::json!({ "rescan": "yes" }), None), at("sensitive", "ask"))
            .unwrap_err();
        assert_eq!(err.message, "`wifi_scan` argument `rescan` must be a boolean, and a string arrived");
    }

    /// Stale is refused before the handler, as on a window — and in every mode, since the guard
    /// sits behind the gate: this is also the proof that a call under the ceiling reaches the
    /// guard unasked from plan to bypass. `wifi_known` is asked with nothing, so no nmcli runs and no
    /// scan happens on the machine the tests build on, whichever answer the guard gives.
    #[test]
    fn an_act_decided_on_an_old_revision_scans_nothing() {
        for mode in ["plan", "ask", "auto", "bypass"] {
            let err = NetworkHandler::default()
                .act(
                    &serde_json::json!({
                        "action": "wifi_known",
                        "args": {},
                        "expect_revision": "0000000000000000",
                    }),
                    at("dangerous", mode),
                )
                .unwrap_err();
            assert!(err.message.starts_with("STALE: this app is at revision "), "{mode}: {}", err.message);
        }
    }

    /// The surface declares nothing the dispatch cannot check.
    #[test]
    fn the_surface_is_declared_soundly() {
        assert!(NetworkHandler::default().surface.registry().problems().is_empty());
    }

    /// The interval's own counting, on constructed instants: a second rescan inside the
    /// interval is refused with its own code, a sentence naming the wait and the cached-list
    /// way round, and once the interval has passed the slot opens again.
    #[test]
    fn the_rescan_gate_counts_its_interval() {
        let gate = RescanGate::new(Duration::from_secs(10));
        let t0 = Instant::now();
        assert!(gate.take_at(t0).is_ok());
        let err = gate.take_at(t0 + Duration::from_secs(3)).unwrap_err();
        assert_eq!(err.code, -32034);
        assert!(err.message.contains("at most every 10 seconds"), "{}", err.message);
        assert!(err.message.contains("Wait 8 seconds"), "{}", err.message);
        assert!(err.message.contains("`rescan=false`"), "{}", err.message);
        assert!(gate.take_at(t0 + Duration::from_secs(10)).is_ok(), "the interval passed");
    }

    /// The loop item 4 of #332 named, on the door itself: two `wifi_scan rescan=true` acts back
    /// to back, and the second is the gate's refusal whatever this machine's Wi-Fi is — the
    /// slot is taken before the adapter is asked, so the refusal holds on a machine with no
    /// adapter as firmly as on one with. The first act's answer is the machine's (a list, or
    /// the trouble that stopped it) and is not pinned here. Through the surface the refusal
    /// arrives as every handler sentence does (-32602, the envelope's one code); the gate's own
    /// code, -32034, is what the raw method answers with, and the pure test above pins it.
    #[test]
    fn a_loop_of_rescans_cannot_keep_knocking_wifi_off() {
        let handler = NetworkHandler::default();
        let _first = handler.act(&act_wifi_scan(serde_json::json!({ "rescan": true }), None), at("sensitive", "ask"));
        let err = handler
            .act(&act_wifi_scan(serde_json::json!({ "rescan": true }), None), at("sensitive", "ask"))
            .unwrap_err();
        assert!(err.message.contains("at most every 10 seconds"), "{}", err.message);
        assert!(err.message.contains("`rescan=false`"), "{}", err.message);

        // The cached list is the way round, and it needs no slot: `rescan=false` is answered
        // (here, on a machine whose Wi-Fi the test must not touch, by whatever the machine
        // says — but never by the gate).
        let cached = handler.act(&act_wifi_scan(serde_json::json!({}), None), at("sensitive", "ask"));
        if let Err(err) = cached {
            assert!(!err.message.contains("at most every"), "a cached read hit the rescan gate: {}", err.message);
        }
    }

    /// The raw `network.*` methods are the desktop's own plumbing (#332): a caller the kernel
    /// cannot place, or a process that is not a Yantrik binary, is refused before any method is
    /// looked at — `wifi_connect` from here takes a password and `dns_set` rewrites this
    /// machine's resolvers with no grade and no card on either. The graded doors are untouched
    /// by the check: `app.describe` still answers any caller at all.
    #[test]
    fn the_raw_methods_answer_only_the_desktops_own_programs() {
        let handler = NetworkHandler::default();
        // No peer credentials at all: refused, and the sentence says where the open door is.
        let err = handler.handle(method::STATUS, serde_json::json!({})).unwrap_err();
        assert_eq!(err.code, -32035);
        assert!(err.message.contains("would not say which process is calling"), "{}", err.message);
        assert!(err.message.contains("app.act"), "{}", err.message);

        // A real process that is not a Yantrik binary — this test runner itself.
        let pid = std::process::id();
        let peer = PeerCred { pid: pid as i32, uid: 0, gid: 0 };
        let err = handler
            .handle_from(method::WIFI_SCAN, serde_json::json!({ "rescan": false }), Some(peer))
            .unwrap_err();
        assert_eq!(err.code, -32035);
        assert!(err.message.contains("the process calling is"), "{}", err.message);
        assert!(err.message.contains(&format!("(pid {pid})")), "{}", err.message);

        // The gate itself never sees the check: `app.describe` answers with no peer.
        let described = handler.handle("app.describe", serde_json::json!({})).expect("describe answers any caller");
        assert_eq!(described["app"], "network");

        // And a name no service serves keeps the protocol's own answer for a caller the raw
        // methods refuse: `yos check` probes with one and expects -32601, which the socket
        // layer maps from this -1.
        let err = handler.handle("app.yos_check_no_such_method", serde_json::json!({})).unwrap_err();
        assert_eq!(err.code, -1);
    }

    /// What counts as one of this OS's own binaries: an absolute path whose last segment is a
    /// `yantrik-` name — including a binary replaced mid-run, which the kernel reports with a
    /// " (deleted)" suffix — and nothing else. A relative path is not the kernel's answer.
    #[test]
    fn a_yantrik_binary_passes_the_peer_check_and_anything_else_does_not() {
        assert!(is_own_binary("/opt/yantrik/bin/yantrik-network-manager"));
        assert!(is_own_binary("/opt/yantrik/bin/yantrik-ui (deleted)"), "a service restarted mid-call");
        assert!(is_own_binary("/opt/yantrik/bin/yantrik"), "the CLI: `yantrik ask` runs the companion's tools");
        assert!(is_own_binary("/opt/yantrik/bin/yantrik (deleted)"));
        assert!(!is_own_binary("/opt/yantrik/bin/yantrikish"), "a name that only starts like ours");
        assert!(!is_own_binary("/usr/bin/python3"));
        assert!(!is_own_binary("yantrik-ui"), "not an absolute path");
        assert!(!is_own_binary(""));
    }
}
