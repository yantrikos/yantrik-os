//! Talking to network-service: the one raw call this app makes, and the typed reading of what
//! comes back.
//!
//! `frameworks/yantrik-services` has no network client, so the call stays what it always was —
//! `SyncRpcClient::for_service("network")`, with its default timeout — and only moves: it runs in
//! a job now, never on the UI thread. It sits behind [`NetworkApi`] so the tests drive the app
//! against a scripted service ([`fake`]) instead of a socket.

use serde::de::DeserializeOwned;
use yantrik_ipc_contracts::network::{method, ScannedNetwork, WifiConnectParams, WifiScanParams, WifiState};
use yantrik_ipc_transport::SyncRpcClient;

use crate::connect;

/// How the service said no: its error code and its sentence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpcFailure {
    pub code: i32,
    pub message: String,
}

/// What this machine says about its network: network-service, and the kernel's route table.
/// Blocking: from a job only.
pub trait NetworkApi: Send + Sync {
    /// One call to network-service.
    fn call(&self, method_name: &str, params: serde_json::Value) -> Result<serde_json::Value, RpcFailure>;

    /// The kernel's route table, as text; empty when it cannot be read. Read here rather than
    /// through network-service, which reports interfaces and their addresses but not routes.
    fn route_table(&self) -> String {
        std::fs::read_to_string(crate::opening::ROUTE_TABLE).unwrap_or_default()
    }
}

/// The live service, over the socket bus. (A test's app talks to `fake` instead.)
#[derive(Debug, Clone, Copy, Default)]
#[cfg_attr(test, allow(dead_code))]
pub struct Service;

impl NetworkApi for Service {
    fn call(&self, method_name: &str, params: serde_json::Value) -> Result<serde_json::Value, RpcFailure> {
        SyncRpcClient::for_service("network")
            .call(method_name, params)
            .map_err(|e| RpcFailure {
                code: e.code,
                message: e.message,
            })
    }
}

/// One call, with the answer parsed into the type the contract says it is.
///
/// The parse is the point. The old reader did
/// `serde_json::from_value(v).unwrap_or_default()` over the interface list, so a shape change on
/// the service side produced an empty ethernet pane and said nothing — the window reported "No
/// interface" on a machine with eth0 up. A mismatch is an error the person sees now. It cannot
/// happen while both ends build the same struct, and leaving the trap in place because it is
/// currently unreachable is how it becomes reachable again.
pub fn call<T: DeserializeOwned>(
    api: &dyn NetworkApi,
    method_name: &str,
    params: serde_json::Value,
) -> Result<T, String> {
    let value = api
        .call(method_name, params)
        .map_err(|e| format!("{method_name}: {}", e.message))?;
    parse(method_name, value)
}

fn parse<T: DeserializeOwned>(method_name: &str, value: serde_json::Value) -> Result<T, String> {
    serde_json::from_value(value)
        .map_err(|e| format!("{method_name} answered in a shape this app does not understand: {e}"))
}

/// What network-service answers a rescan asked for within ten seconds of the last (#332).
pub const RESCAN_TOO_SOON: i32 = -32034;

/// Sweep the band and read the list back; if the radio was swept moments ago, read the list
/// that sweep left instead.
///
/// The service runs one rescan per ten seconds for everyone on its socket, so a mind that just
/// rescanned used to turn the person's own Scan press into a refusal — and the refusal into an
/// empty list, the one outcome that says "no networks here" when there are some. A list that
/// is seconds old is as current as the sweep the press asked for.
pub fn scan_now(api: &dyn NetworkApi) -> Result<Vec<ScannedNetwork>, String> {
    let sweep = to_params(WifiScanParams { rescan: true });
    match api.call(method::WIFI_SCAN, sweep) {
        Err(e) if e.code == RESCAN_TOO_SOON => {
            call(api, method::WIFI_SCAN, to_params(WifiScanParams { rescan: false }))
        }
        Err(e) => Err(format!("{}: {}", method::WIFI_SCAN, e.message)),
        Ok(value) => parse(method::WIFI_SCAN, value),
    }
}

/// A request struct as call parameters. The contract's structs always serialize.
pub fn to_params(params: impl serde::Serialize) -> serde_json::Value {
    serde_json::to_value(params).unwrap_or_else(|_| serde_json::json!({}))
}

/// The connect backend `connect::submit` is handed: the socket call to network-service.
/// `join_request` has already decided what secret, if any, travels; the service hands it to
/// nmcli over stdin with `--ask`, never through a command line, and its errors never name it.
pub struct Joiner<'a>(pub &'a dyn NetworkApi);

impl connect::Backend for Joiner<'_> {
    fn wifi_connect(&self, request: &WifiConnectParams) -> Result<WifiState, String> {
        call::<WifiState>(self.0, method::WIFI_CONNECT, to_params(request))
    }
}

/// The service this app talks to: the live one, or under a test the fake that test installed
/// for these paths (an unreachable one when it installed none).
#[cfg(not(test))]
pub fn for_app(_paths: &yantrik_app::Paths) -> std::sync::Arc<dyn NetworkApi> {
    std::sync::Arc::new(Service)
}

#[cfg(test)]
pub use fake::for_app;

#[cfg(test)]
pub mod fake {
    //! A scripted network-service: what each method answers, and what was asked of it.

    use std::collections::{HashMap, VecDeque};
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex, OnceLock};

    use serde_json::{json, Value};
    use yantrik_app::Paths;

    use super::{NetworkApi, RpcFailure};

    type Answer = Result<Value, RpcFailure>;

    #[derive(Default)]
    struct Script {
        /// Answered every time, unless something is queued.
        always: HashMap<String, Answer>,
        /// Answered once each, first.
        once: HashMap<String, VecDeque<Answer>>,
        /// Methods whose call panics, as a parser bug would.
        panics: Vec<String>,
        /// The route table the machine has.
        route_table: String,
        asked: Vec<(String, Value)>,
    }

    /// The fake. Cloning shares it.
    #[derive(Clone, Default)]
    pub struct FakeNetwork(Arc<Mutex<Script>>);

    impl FakeNetwork {
        /// A service that answers nothing: every method is unreachable.
        pub fn unreachable() -> FakeNetwork {
            FakeNetwork::default()
        }

        /// A wired machine: eth0 up and carrying the traffic, a Wi-Fi adapter with its radio on
        /// and joined to nothing, one saved network, no firewall tool.
        pub fn wired() -> FakeNetwork {
            FakeNetwork::default()
                .answering("network.dns", json!({ "nameservers": ["192.168.4.1"], "search_domains": ["lan"] }))
                .answering(
                    "network.interfaces",
                    json!([{
                        "name": "eth0", "mac_address": "52:54:00:12:34:56", "ip_address": "192.168.4.20",
                        "rx_bytes": 1, "tx_bytes": 2, "state": "up", "conn_type": "Ethernet",
                        "speed_mbps": 1000, "subnet": "255.255.255.0", "gateway": "192.168.4.1",
                    }]),
                )
                .answering(
                    "network.status",
                    json!({ "connected": true, "type": "ethernet", "ip_address": "192.168.4.20" }),
                )
                .answering(
                    "network.wifi_state",
                    json!({ "adapter_present": true, "device": "wlan0", "radio": "on" }),
                )
                .answering(
                    "network.wifi_known",
                    json!([{ "ssid": "HomeNet", "uuid": "11111111-1111-1111-1111-111111111111", "is_active": false }]),
                )
                .answering(
                    "network.firewall",
                    json!({ "state": "absent", "rules": [], "reason": "none of nft, ufw or firewall-cmd is installed" }),
                )
        }

        /// The kernel's route table: the default route leaving by `iface`.
        pub fn routing_by(self, iface: &str) -> FakeNetwork {
            self.0.lock().unwrap().route_table = [
                "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT".to_string(),
                format!("{iface}\t00000000\t0104A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0"),
            ]
            .join("\n");
            self
        }

        /// `method` answers `value` from now on.
        pub fn answering(self, method: &str, value: Value) -> FakeNetwork {
            self.set(method, Ok(value));
            self
        }

        /// `method` fails with this code and sentence from now on.
        pub fn failing(self, method: &str, code: i32, message: &str) -> FakeNetwork {
            self.set(method, Err(RpcFailure { code, message: message.into() }));
            self
        }

        pub fn set(&self, method: &str, answer: Answer) {
            self.0.lock().unwrap().always.insert(method.into(), answer);
        }

        /// `method` answers this once, before what it always answers.
        pub fn once(&self, method: &str, answer: Answer) {
            self.0.lock().unwrap().once.entry(method.into()).or_default().push_back(answer);
        }

        pub fn panicking(self, method: &str) -> FakeNetwork {
            self.0.lock().unwrap().panics.push(method.into());
            self
        }

        /// Every call, in order: the method and its parameters.
        pub fn asked(&self) -> Vec<(String, Value)> {
            self.0.lock().unwrap().asked.clone()
        }

        pub fn asked_for(&self, method: &str) -> Vec<Value> {
            self.asked().into_iter().filter(|(m, _)| m == method).map(|(_, p)| p).collect()
        }
    }

    impl NetworkApi for FakeNetwork {
        fn call(&self, method: &str, params: Value) -> Result<Value, RpcFailure> {
            let mut script = self.0.lock().unwrap();
            script.asked.push((method.into(), params));
            if script.panics.iter().any(|m| m == method) {
                drop(script);
                panic!("the fake network service was told to panic on {method}");
            }
            if let Some(answer) = script.once.get_mut(method).and_then(VecDeque::pop_front) {
                return answer;
            }
            script.always.get(method).cloned().unwrap_or_else(|| {
                Err(RpcFailure {
                    code: -32000,
                    message: "Service at /run/user/1000/network.sock is unreachable (retrying in up to 5s)".into(),
                })
            })
        }

        fn route_table(&self) -> String {
            self.0.lock().unwrap().route_table.clone()
        }
    }

    fn installed() -> &'static Mutex<HashMap<PathBuf, FakeNetwork>> {
        static INSTALLED: OnceLock<Mutex<HashMap<PathBuf, FakeNetwork>>> = OnceLock::new();
        INSTALLED.get_or_init(Default::default)
    }

    /// The fake an app started at `paths` talks to. Each test's paths are its own.
    pub fn install(paths: &Paths, fake: &FakeNetwork) {
        installed().lock().unwrap().insert(paths.config_file().to_path_buf(), fake.clone());
    }

    pub fn for_app(paths: &Paths) -> Arc<dyn NetworkApi> {
        let found = installed().lock().unwrap().get(paths.config_file()).cloned();
        Arc::new(found.unwrap_or_else(FakeNetwork::unreachable))
    }
}
