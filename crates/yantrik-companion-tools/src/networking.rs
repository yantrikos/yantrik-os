//! General networking tools — interfaces, resolvers, reachability, ports, VPN.
//!
//! # One owner per domain
//!
//! `services/network-service` owns this machine's networking: interfaces, connectivity,
//! resolvers, Wi-Fi and the firewall reading. Commit d189ac1 gave it a typed contract
//! (`yantrik_ipc_contracts::network`) and the write half the Network Manager app had been calling
//! into a wall for months. These tools used to be a second, worse implementation of the same
//! domain, shelling out on their own:
//!
//! - `network_interfaces` ran `ip -br addr` and `ip -br link` and stitched the two together.
//! - `network_dns` read `/etc/resolv.conf` by hand.
//! - `network_dns_set` **wrote** `/etc/resolv.conf` with `std::fs::write`, left a `.bak` beside
//!   it, and answered "DNS set to 1.1.1.1". It is the sharpest of the three and is dealt with at
//!   [`NetworkDnsSetTool`].
//! - `network_diagnose` read `/etc/resolv.conf` for a nameserver and ran `iw dev` for Wi-Fi —
//!   `iw` is in neither of this OS's package lists, so that line was empty on every machine it
//!   builds.
//!
//! They are callers now, through [`backend::NetworkBackend`], the same shape
//! `crates/yantrik-companion/src/calendar/backend.rs` took when the calendar stopped being two
//! calendars. The trait exists so a tool's failure path can be *run* rather than assumed:
//! `backend::fake::FakeNetwork` (test builds only) answers the same questions the service does,
//! including the
//! refusals, and there is no hardware anywhere near it. `wifi.rs` and `firewall.rs` speak the
//! same backend and share that fake.
//!
//! # What is still local, and why
//!
//! `network_ping`, `network_traceroute`, `network_ports` and `network_vpn_status` have no
//! counterpart in the service and are not given one. `design/network-2026-09-20.md` took ping and
//! traceroute *off* the Network Manager app on the grounds that the companion already does them
//! properly on a worker thread, and it took the VPN tab off because nothing in this OS installs a
//! VPN. Adding them back here as service methods would be the second path to the same thing that
//! this whole pass exists to remove. They read; they change nothing; they stay.

use std::sync::Arc;

use super::{PermissionLevel, Tool, ToolContext, ToolRegistry};

use backend::NetworkBackend;
use yantrik_ipc_contracts::network::{DnsConfig, DnsSetParams, NetworkInterfaceInfo};

/// The one network, as the mind's tools see it.
///
/// Deliberately the service's own shape — the typed parameters and responses out of
/// `yantrik_ipc_contracts::network`, never a hand-written JSON key. Hand-written keys are what
/// left the Network Manager app calling five method names the service had never heard of, with
/// three of the five results dropped into a `let _ =` so the buttons looked like they worked.
pub mod backend {
    use yantrik_ipc_contracts::network::{
        method, DnsConfig, DnsSetParams, DnsSetResult, FirewallState, NetworkInterfaceInfo,
        ScannedNetwork, WifiConnectParams, WifiRadioParams, WifiScanParams, WifiState,
    };

    /// What a network owner has to be able to answer.
    ///
    /// Every method hands back the service's own sentence on failure. A reason thrown away here
    /// becomes a tool that says "failed to connect" and means nothing by it — which is what the
    /// old `wifi_connect` said whether the adapter was missing, the daemon was down or the access
    /// point had refused the password.
    pub trait NetworkBackend: Send + Sync {
        fn interfaces(&self) -> Result<Vec<NetworkInterfaceInfo>, String>;
        fn dns(&self) -> Result<DnsConfig, String>;
        fn dns_set(&self, params: &DnsSetParams) -> Result<DnsSetResult, String>;
        fn wifi_state(&self) -> Result<WifiState, String>;
        fn wifi_scan(&self, params: &WifiScanParams) -> Result<Vec<ScannedNetwork>, String>;
        fn wifi_radio(&self, params: &WifiRadioParams) -> Result<WifiState, String>;
        fn wifi_connect(&self, params: &WifiConnectParams) -> Result<WifiState, String>;
        fn wifi_disconnect(&self) -> Result<WifiState, String>;
        fn firewall(&self) -> Result<FirewallState, String>;
    }

    /// The real one: `network-service` over its socket.
    ///
    /// `yantrik_ipc_transport::service::client` is the function the Calendar app and the
    /// companion's calendar tools both use. It starts the service if it is down — inside the
    /// shell, through the shell's own ServiceManager without leaving the process — and gives the
    /// whole start two seconds before saying so. `network-service` is `autostart = true`, so the
    /// start is the unusual path rather than the usual one; it is here because "the service is
    /// not running" has to come back as those words and not as a connection refused.
    ///
    /// There is no private fallback. A tool whose service is down says the service is down. The
    /// alternative — shelling out to nmcli when the socket does not answer — is how this machine
    /// came to have two implementations of its own network in the first place.
    pub struct ServiceNetwork;

    impl ServiceNetwork {
        const SERVICE: &'static str = "network";

        fn call<P: serde::Serialize, R: serde::de::DeserializeOwned>(
            method_name: &str,
            params: &P,
        ) -> Result<R, String> {
            let client = yantrik_ipc_transport::service::client(Self::SERVICE)?;
            client.call_typed(method_name, params).map_err(|e| e.message)
        }
    }

    impl NetworkBackend for ServiceNetwork {
        fn interfaces(&self) -> Result<Vec<NetworkInterfaceInfo>, String> {
            Self::call(method::INTERFACES, &serde_json::json!({}))
        }

        fn dns(&self) -> Result<DnsConfig, String> {
            Self::call(method::DNS, &serde_json::json!({}))
        }

        fn dns_set(&self, params: &DnsSetParams) -> Result<DnsSetResult, String> {
            Self::call(method::DNS_SET, params)
        }

        fn wifi_state(&self) -> Result<WifiState, String> {
            Self::call(method::WIFI_STATE, &serde_json::json!({}))
        }

        fn wifi_scan(&self, params: &WifiScanParams) -> Result<Vec<ScannedNetwork>, String> {
            Self::call(method::WIFI_SCAN, params)
        }

        fn wifi_radio(&self, params: &WifiRadioParams) -> Result<WifiState, String> {
            Self::call(method::WIFI_RADIO, params)
        }

        fn wifi_connect(&self, params: &WifiConnectParams) -> Result<WifiState, String> {
            Self::call(method::WIFI_CONNECT, params)
        }

        fn wifi_disconnect(&self) -> Result<WifiState, String> {
            Self::call(method::WIFI_DISCONNECT, &serde_json::json!({}))
        }

        fn firewall(&self) -> Result<FirewallState, String> {
            Self::call(method::FIREWALL, &serde_json::json!({}))
        }
    }

    /// A network that answers from a script instead of from hardware.
    ///
    /// This is what the trait is for. None of the interesting cases can be produced on demand on
    /// the machines this is built and run on — they have no Wi-Fi adapter and no firewall — and
    /// the two that matter most are a service that is down and a change that did not take.
    /// Shared by the tests in `wifi`, `networking` and `firewall`, which all speak this one
    /// backend.
    #[cfg(test)]
    pub mod fake {
        use super::*;
        use std::sync::Mutex;

        #[derive(Default)]
        pub struct FakeNetwork {
            pub interfaces: Option<Result<Vec<NetworkInterfaceInfo>, String>>,
            pub dns: Option<Result<DnsConfig, String>>,
            pub dns_set: Option<Result<DnsSetResult, String>>,
            pub wifi_state: Option<Result<WifiState, String>>,
            pub wifi_scan: Option<Result<Vec<ScannedNetwork>, String>>,
            pub wifi_radio: Option<Result<WifiState, String>>,
            pub wifi_connect: Option<Result<WifiState, String>>,
            pub wifi_disconnect: Option<Result<WifiState, String>>,
            pub firewall: Option<Result<FirewallState, String>>,
            /// What crossed the wire, so a test can assert on the request and not only on the
            /// answer. The password one matters: nothing may echo it back.
            pub dns_set_seen: Mutex<Vec<DnsSetParams>>,
            pub connect_seen: Mutex<Vec<WifiConnectParams>>,
            pub radio_seen: Mutex<Vec<bool>>,
            pub scan_seen: Mutex<Vec<bool>>,
        }

        fn unscripted<T>(what: &str) -> Result<T, String> {
            panic!("this test did not script {what}, so the tool asked something it should not")
        }

        impl NetworkBackend for FakeNetwork {
            fn interfaces(&self) -> Result<Vec<NetworkInterfaceInfo>, String> {
                self.interfaces.clone().unwrap_or_else(|| unscripted("interfaces"))
            }
            fn dns(&self) -> Result<DnsConfig, String> {
                self.dns.clone().unwrap_or_else(|| unscripted("dns"))
            }
            fn dns_set(&self, params: &DnsSetParams) -> Result<DnsSetResult, String> {
                self.dns_set_seen.lock().unwrap().push(params.clone());
                self.dns_set.clone().unwrap_or_else(|| unscripted("dns_set"))
            }
            fn wifi_state(&self) -> Result<WifiState, String> {
                self.wifi_state.clone().unwrap_or_else(|| unscripted("wifi_state"))
            }
            fn wifi_scan(&self, params: &WifiScanParams) -> Result<Vec<ScannedNetwork>, String> {
                self.scan_seen.lock().unwrap().push(params.rescan);
                self.wifi_scan.clone().unwrap_or_else(|| unscripted("wifi_scan"))
            }
            fn wifi_radio(&self, params: &WifiRadioParams) -> Result<WifiState, String> {
                self.radio_seen.lock().unwrap().push(params.enabled);
                self.wifi_radio.clone().unwrap_or_else(|| unscripted("wifi_radio"))
            }
            fn wifi_connect(&self, params: &WifiConnectParams) -> Result<WifiState, String> {
                self.connect_seen.lock().unwrap().push(params.clone());
                self.wifi_connect.clone().unwrap_or_else(|| unscripted("wifi_connect"))
            }
            fn wifi_disconnect(&self) -> Result<WifiState, String> {
                self.wifi_disconnect.clone().unwrap_or_else(|| unscripted("wifi_disconnect"))
            }
            fn firewall(&self) -> Result<FirewallState, String> {
                self.firewall.clone().unwrap_or_else(|| unscripted("firewall"))
            }
        }
    }
}

pub fn register(reg: &mut ToolRegistry) {
    register_with(reg, Arc::new(backend::ServiceNetwork));
}

/// Register against a given backend. The tests use it; `register` is the machine's own.
pub fn register_with(reg: &mut ToolRegistry, net: Arc<dyn NetworkBackend>) {
    reg.register(Box::new(NetworkInterfacesTool { net: net.clone() }));
    reg.register(Box::new(NetworkPingTool));
    reg.register(Box::new(NetworkTracerouteTool));
    reg.register(Box::new(NetworkPortsTool));
    reg.register(Box::new(NetworkDnsTool { net: net.clone() }));
    reg.register(Box::new(NetworkDnsSetTool { net: net.clone() }));
    reg.register(Box::new(NetworkVpnStatusTool));
    reg.register(Box::new(NetworkDiagnoseTool { net }));
}

/// Validate a hostname or IP (no shell metacharacters).
fn validate_host(host: &str) -> Result<(), String> {
    if host.is_empty() {
        return Err("host is required".to_string());
    }
    if host.len() > 253 {
        return Err("hostname too long".to_string());
    }
    // The host goes to ping and traceroute as an argument; one beginning with `-` would be read
    // as one of their options (`-f` floods, `-p` sets a payload) instead of a host.
    if host.starts_with('-') {
        return Err("host cannot begin with '-'".to_string());
    }
    if host.contains(|c: char| c == '`' || c == '$' || c == ';' || c == '|' || c == '&' || c == ' ' || c == '\'' || c == '"') {
        return Err("host contains invalid characters".to_string());
    }
    Ok(())
}

/// How a failure from the service reads to a model.
///
/// Prefixed with the thing that failed rather than handed over bare, because the service's own
/// sentences — "this machine has no Wi-Fi adapter", "NetworkManager is not running" — are about
/// the machine and say nothing about which tool was asking.
fn refused(what: &str, why: &str) -> String {
    format!("Could not {what}: {why}")
}

// ── Network Interfaces ──

pub struct NetworkInterfacesTool {
    net: Arc<dyn NetworkBackend>,
}

/// The interface list as the service read it out of `/proc` and `/sys`.
///
/// The tool this replaces ran `ip -br addr`, then ran `ip -br link` a second time and printed a
/// separate "MAC addresses" block, so a reader had to join two lists by eye. One reading, one row
/// per interface. An interface with no address says so rather than being given an empty column.
fn format_interfaces(interfaces: &[NetworkInterfaceInfo]) -> String {
    if interfaces.is_empty() {
        return "This machine reports no network interfaces at all, not even loopback — which is \
                itself the finding."
            .to_string();
    }
    let mut out = String::from("Network interfaces:\n");
    for iface in interfaces {
        let address = iface.ip_address.as_deref().unwrap_or("no address");
        out.push_str(&format!(
            "  {} [{}] {} — {}, mac {}, rx {} B / tx {} B\n",
            iface.name,
            iface.state,
            iface.conn_type.as_str(),
            address,
            if iface.mac_address.is_empty() {
                "unknown"
            } else {
                &iface.mac_address
            },
            iface.rx_bytes,
            iface.tx_bytes,
        ));
    }
    out
}

impl Tool for NetworkInterfacesTool {
    fn name(&self) -> &'static str { "network_interfaces" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "networking" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "network_interfaces",
                "description": "List network adapters, their link state and addresses",
                "parameters": { "type": "object", "properties": {} }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, _args: &serde_json::Value) -> String {
        match self.net.interfaces() {
            Ok(interfaces) => format_interfaces(&interfaces),
            Err(why) => refused("read this machine's network interfaces", &why),
        }
    }
}

// ── Ping ──

pub struct NetworkPingTool;

impl Tool for NetworkPingTool {
    fn name(&self) -> &'static str { "network_ping" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "networking" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "network_ping",
                "description": "Ping host for reachability only",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "host": {"type": "string", "description": "Hostname or IP address to ping"},
                        "count": {"type": "integer", "description": "Number of packets (default: 4, max: 10)"}
                    },
                    "required": ["host"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let host = args.get("host").and_then(|v| v.as_str()).unwrap_or_default();
        let count = args.get("count").and_then(|v| v.as_i64()).unwrap_or(4).clamp(1, 10);

        if let Err(e) = validate_host(host) {
            return format!("Error: {e}");
        }

        match std::process::Command::new("ping")
            .args(["-c", &count.to_string(), "-W", "5", host])
            .output()
        {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                // Extract the summary lines (last 2-3 lines)
                let lines: Vec<&str> = stdout.lines().collect();
                let mut result = Vec::new();

                // First line (PING header)
                if let Some(first) = lines.first() {
                    result.push(first.to_string());
                }

                // Stats lines (usually last 2 lines)
                for line in lines.iter().rev().take(3).collect::<Vec<_>>().into_iter().rev() {
                    if line.contains("packets") || line.contains("rtt") || line.contains("round-trip") {
                        result.push(line.to_string());
                    }
                }

                if result.is_empty() {
                    if output.status.success() {
                        stdout.to_string()
                    } else {
                        format!("Host {} is unreachable.", host)
                    }
                } else {
                    result.join("\n")
                }
            }
            // Not "unreachable": the host was never asked. `ping` is not part of every build of
            // this OS, and a mind told a host is down will go and act on that.
            Err(e) => format!(
                "Could not ping {host}: the `ping` program is not installed on this machine ({e}). \
                 Nothing was sent, so this says nothing about whether {host} is reachable; \
                 network_diagnose checks connectivity without it."
            ),
        }
    }
}

// ── Traceroute ──

pub struct NetworkTracerouteTool;

impl Tool for NetworkTracerouteTool {
    fn name(&self) -> &'static str { "network_traceroute" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "networking" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "network_traceroute",
                "description": "Trace path packets take to a host",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "host": {"type": "string", "description": "Hostname or IP to trace route to"},
                        "max_hops": {"type": "integer", "description": "Maximum hops (default: 15, max: 30)"}
                    },
                    "required": ["host"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let host = args.get("host").and_then(|v| v.as_str()).unwrap_or_default();
        let max_hops = args.get("max_hops").and_then(|v| v.as_i64()).unwrap_or(15).clamp(1, 30);

        if let Err(e) = validate_host(host) {
            return format!("Error: {e}");
        }

        // Try traceroute, fallback to tracepath (common on minimal installs)
        let has_traceroute = std::process::Command::new("traceroute")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        let cmd = if has_traceroute { "traceroute" } else { "tracepath" };
        let hops_str = max_hops.to_string();
        let cmd_args: Vec<&str> = if has_traceroute {
            vec!["-m", &hops_str, "-w", "3", host]
        } else {
            vec!["-m", &hops_str, host]
        };

        match std::process::Command::new(cmd)
            .args(&cmd_args)
            .output()
        {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if stdout.trim().is_empty() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    format!("Traceroute failed: {}", stderr.trim())
                } else if stdout.len() > 3000 {
                    format!("{}\n... (truncated)", &stdout[..stdout.floor_char_boundary(3000)])
                } else {
                    stdout.to_string()
                }
            }
            // Said accurately rather than hopefully. The old message was "Install with: apk add
            // traceroute" — Alpine's package manager, on an OS whose two build recipes are Debian.
            // Neither recipe installs `traceroute` or `iputils-tracepath`, which is the same
            // reading that took the traceroute button off the Network Manager app.
            Err(_) => "Neither traceroute nor tracepath is installed on this machine, and neither \
                       is in this OS's build recipes, so this is expected rather than a fault. \
                       `apt install traceroute` would add it."
                .to_string(),
        }
    }
}

// ── Open Ports ──

pub struct NetworkPortsTool;

impl Tool for NetworkPortsTool {
    fn name(&self) -> &'static str { "network_ports" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "networking" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "network_ports",
                "description": "List open or listening local ports",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "state": {"type": "string", "enum": ["listening", "established", "all"], "description": "Filter by connection state (default: listening)"}
                    }
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let state = args.get("state").and_then(|v| v.as_str()).unwrap_or("listening");

        // Use `ss` (socket statistics) — available on all modern Linux
        let filter = match state {
            "listening" => vec!["-tlnp"],
            "established" => vec!["-tnp", "state", "established"],
            "all" => vec!["-tanp"],
            _ => vec!["-tlnp"],
        };

        match std::process::Command::new("ss").args(&filter).output() {
            Ok(output) if output.status.success() => {
                let text = String::from_utf8_lossy(&output.stdout);
                if text.trim().is_empty() {
                    format!("No {} ports found.", state)
                } else if text.len() > 3000 {
                    format!("{}\n... (truncated)", &text[..text.floor_char_boundary(3000)])
                } else {
                    text.to_string()
                }
            }
            Ok(output) => {
                // Fallback to netstat
                let text = String::from_utf8_lossy(&output.stderr);
                match std::process::Command::new("netstat").args(["-tlnp"]).output() {
                    Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
                    _ => format!("ss failed: {}", text.trim()),
                }
            }
            Err(e) => format!("Error (ss not available): {e}"),
        }
    }
}

// ── DNS Info ──

pub struct NetworkDnsTool {
    net: Arc<dyn NetworkBackend>,
}

fn format_dns(dns: &DnsConfig) -> String {
    let mut out = String::new();
    if dns.nameservers.is_empty() {
        out.push_str(
            "This machine has no DNS server configured, so nothing resolves by name.\n",
        );
    } else {
        out.push_str(&format!("DNS servers: {}\n", dns.nameservers.join(", ")));
    }
    if !dns.search_domains.is_empty() {
        out.push_str(&format!("Search domains: {}\n", dns.search_domains.join(", ")));
    }
    out
}

impl Tool for NetworkDnsTool {
    fn name(&self) -> &'static str { "network_dns" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "networking" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "network_dns",
                "description": "Show which DNS servers this machine uses, and optionally resolve a name",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "lookup": {"type": "string", "description": "Optional: a hostname to resolve through the machine's own resolver"}
                    }
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let lookup = args.get("lookup").and_then(|v| v.as_str()).unwrap_or("").trim();

        let mut out = match self.net.dns() {
            Ok(dns) => format_dns(&dns),
            Err(why) => refused("read this machine's DNS configuration", &why) + "\n",
        };

        if !lookup.is_empty() {
            if let Err(e) = validate_host(lookup) {
                return format!("Error: {e}");
            }
            // Resolved through the C library rather than by shelling out. `nslookup` is in
            // `dnsutils`, which neither of this OS's build recipes installs, so the old code's
            // first choice was "command not found" on every machine it builds and the answer came
            // from its `getent` fallback — one process deep, for something the standard library
            // does with the same resolver.
            use std::net::ToSocketAddrs;
            out.push_str(&format!("\nResolving \"{lookup}\":\n"));
            match (lookup, 0u16).to_socket_addrs() {
                Ok(addrs) => {
                    let found: Vec<String> =
                        addrs.map(|a| a.ip().to_string()).collect();
                    if found.is_empty() {
                        out.push_str("  the resolver answered with no addresses\n");
                    } else {
                        for address in found {
                            out.push_str(&format!("  {address}\n"));
                        }
                    }
                }
                Err(e) => out.push_str(&format!("  did not resolve: {e}\n")),
            }
        }

        out
    }
}

// ── Set DNS ──

pub struct NetworkDnsSetTool {
    net: Arc<dyn NetworkBackend>,
}

/// Change this machine's resolvers, through the service that owns them.
///
/// # What this used to do
///
/// ```text
/// let _ = std::fs::copy("/etc/resolv.conf", "/etc/resolv.conf.bak");
/// match std::fs::write("/etc/resolv.conf", &content) { Ok(()) => "DNS set to {primary}" … }
/// ```
///
/// Two things were wrong and both were silent. The write needs root, which a desktop session does
/// not have — so on an ordinary machine it failed and suggested "Try running as root", and on the
/// images `deploy/` builds, where the session can become root for anything through a blanket
/// `NOPASSWD: ALL`, it succeeded as an unscoped root write from a tool call. And even rooted it
/// does not last: NetworkManager owns `/etc/resolv.conf` and rewrites it from the active profile
/// on the next carrier change or DHCP renew. "DNS set to 1.1.1.1" was true for as long as nothing
/// happened.
///
/// It is `network.dns_set` now — `nmcli connection modify … ipv4.dns` on the profile carrying the
/// default route, `ipv4.ignore-auto-dns yes` so the router's servers do not stay in the list
/// behind the caller's, and a re-activation to apply it. The service re-reads `/etc/resolv.conf`
/// *and* NetworkManager's own view of the device afterwards and refuses to call it done unless
/// every server asked for is in one of them.
///
/// **Sensitive, not standard.** Applying the change re-activates the connection: the link goes
/// down and comes back with a fresh lease. Called down the connection it is changing — an ssh
/// session, a remote caller — that is a visible interruption. It is not `dangerous`: the link
/// comes back by itself, which is the line `wifi_disconnect` and `wifi_radio` are on the wrong
/// side of.
impl Tool for NetworkDnsSetTool {
    fn name(&self) -> &'static str { "network_dns_set" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Sensitive }
    fn category(&self) -> &'static str { "networking" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "network_dns_set",
                "description": "Point this machine's DNS at given servers. Re-activates the network connection to apply, which briefly interrupts it.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "servers": {
                            "type": "array",
                            "items": {"type": "string"},
                            "description": "DNS server IP addresses in order of preference, e.g. [\"1.1.1.1\", \"1.0.0.1\"]. At most 3 — the resolver reads no more than that."
                        }
                    },
                    "required": ["servers"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        dns_set_answer(&*self.net, args)
    }
}

/// The tool, without the `ToolContext` it does not read.
///
/// Separated so the test at the bottom can run it against a backend that refuses — a service that
/// is down, a change that did not take — which is the half that used to be guesswork.
fn dns_set_answer(net: &dyn NetworkBackend, args: &serde_json::Value) -> String {
    let servers: Vec<String> = match args.get("servers") {
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        // A single string is what a model reaches for when it wants one server, and refusing it
        // teaches nothing. Anything else is a shape this tool does not take.
        Some(serde_json::Value::String(one)) => vec![one.trim().to_string()],
        _ => Vec::new(),
    };

    if servers.is_empty() {
        return "Error: `servers` is required — a list of DNS server IP addresses, e.g. \
                [\"1.1.1.1\", \"1.0.0.1\"]."
            .to_string();
    }

    match net.dns_set(&DnsSetParams { servers: servers.clone() }) {
        Ok(result) => {
            let mut out = format!(
                "DNS changed on \"{}\" ({}).\n",
                result.connection, result.device
            );
            out.push_str(&format!(
                "  /etc/resolv.conf now: {}\n",
                if result.resolv_conf.nameservers.is_empty() {
                    "no nameserver lines".to_string()
                } else {
                    result.resolv_conf.nameservers.join(", ")
                }
            ));
            out.push_str(&format!(
                "  NetworkManager applied to {}: {}\n",
                result.device,
                if result.device_dns.is_empty() {
                    "nothing it reports".to_string()
                } else {
                    result.device_dns.join(", ")
                }
            ));
            out.push_str(
                "The connection was re-activated to apply this, so it dropped and came back.",
            );
            out
        }
        Err(why) => refused(&format!("set DNS to {}", servers.join(", ")), &why),
    }
}

// ── VPN Status ──

pub struct NetworkVpnStatusTool;

impl Tool for NetworkVpnStatusTool {
    fn name(&self) -> &'static str { "network_vpn_status" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "networking" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "network_vpn_status",
                "description": "Check whether VPN is connected",
                "parameters": { "type": "object", "properties": {} }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, _args: &serde_json::Value) -> String {
        let mut info = Vec::new();

        // Check WireGuard
        match std::process::Command::new("wg").arg("show").output() {
            Ok(o) if o.status.success() => {
                let text = String::from_utf8_lossy(&o.stdout);
                if text.trim().is_empty() {
                    info.push("WireGuard: no active tunnels".to_string());
                } else {
                    info.push("WireGuard: active".to_string());
                    for line in text.lines().take(10) {
                        info.push(format!("  {}", line.trim()));
                    }
                }
            }
            _ => {
                info.push("WireGuard: not installed".to_string());
            }
        }

        // Check OpenVPN
        match std::process::Command::new("pgrep")
            .args(["-a", "openvpn"])
            .output()
        {
            Ok(o) if o.status.success() => {
                let text = String::from_utf8_lossy(&o.stdout);
                if !text.trim().is_empty() {
                    info.push("OpenVPN: running".to_string());
                    for line in text.lines().take(3) {
                        info.push(format!("  {}", line.trim()));
                    }
                } else {
                    info.push("OpenVPN: not running".to_string());
                }
            }
            _ => {
                info.push("OpenVPN: not detected".to_string());
            }
        }

        // Check nmcli VPN connections
        if let Ok(o) = std::process::Command::new("nmcli")
            .args(["-t", "-f", "NAME,TYPE,DEVICE", "connection", "show", "--active"])
            .output()
        {
            if o.status.success() {
                let text = String::from_utf8_lossy(&o.stdout);
                for line in text.lines() {
                    if line.contains("vpn") || line.contains("wireguard") || line.contains("tun") {
                        info.push(format!("NM VPN: {}", line.replace(':', " | ")));
                    }
                }
            }
        }

        // Check tun/tap interfaces
        if let Ok(o) = std::process::Command::new("ip")
            .args(["-br", "link", "show", "type", "tun"])
            .output()
        {
            if o.status.success() {
                let text = String::from_utf8_lossy(&o.stdout);
                if !text.trim().is_empty() {
                    info.push(format!("TUN interfaces: {}", text.trim()));
                }
            }
        }

        if info.is_empty() {
            "No VPN connections detected.".to_string()
        } else {
            info.join("\n")
        }
    }
}

// ── Network Diagnose ──

pub struct NetworkDiagnoseTool {
    net: Arc<dyn NetworkBackend>,
}

impl Tool for NetworkDiagnoseTool {
    fn name(&self) -> &'static str { "network_diagnose" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "networking" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "network_diagnose",
                "description": "Run full network health check",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, _args: &serde_json::Value) -> String {
        let mut report = Vec::new();

        // 1. Which resolver, asked of the service rather than scraped out of /etc/resolv.conf by
        //    a second reader of the same file. "unknown" here is now "the service could not be
        //    reached", said in those words, instead of a word that also means "no nameserver
        //    line".
        let dns_server = match self.net.dns() {
            Ok(dns) => dns
                .nameservers
                .first()
                .cloned()
                .unwrap_or_else(|| "none configured".to_string()),
            Err(why) => format!("not readable ({why})"),
        };

        // 2. DNS latency, through the C library's resolver — the same one everything else on this
        //    machine uses.
        let dns_start = std::time::Instant::now();
        let dns_ok = {
            use std::net::ToSocketAddrs;
            ("example.com", 0u16).to_socket_addrs().is_ok()
        };
        let dns_ms = dns_start.elapsed().as_millis();

        if dns_ok {
            let assessment = if dns_ms > 500 {
                format!("SLOW ({}ms via {}). Consider switching to 1.1.1.1 or 8.8.8.8.", dns_ms, dns_server)
            } else {
                format!("OK ({}ms via {}).", dns_ms, dns_server)
            };
            report.push(format!("DNS: {}", assessment));
        } else {
            report.push(format!("DNS: FAILED via {}. Name resolution broken.", dns_server));
        }

        // 3 and 4. The gateway and the internet, without `ip` or `ping`.
        //
        // These two lines ran `ip route` and `ping`, and neither program is on the machine this
        // OS builds. So on a machine with a default route and a working connection the mind was
        // told "No default route found. Network not configured." and "Internet: UNREACHABLE" —
        // a missing tool reported as a broken network, to the one reader that would act on it.
        // The kernel publishes the route and the neighbour table as files, and reaching the
        // internet is a TCP connect, so neither question needs a program to be installed.
        let route = std::fs::read_to_string("/proc/net/route").unwrap_or_default();
        match default_route(&route) {
            Some((gateway, iface)) => {
                let arp = std::fs::read_to_string("/proc/net/arp").unwrap_or_default();
                report.push(match neighbour_resolved(&arp, &gateway) {
                    Some(true) => format!("Gateway ({gateway} via {iface}): OK (it answers on the link)."),
                    Some(false) => format!(
                        "Gateway ({gateway} via {iface}): NOT ANSWERING on the link. Router may be down or the cable unplugged."
                    ),
                    None => format!(
                        "Gateway ({gateway} via {iface}): route present; nothing has been sent to it recently, so whether it answers is not known."
                    ),
                });
            }
            None => report.push("Gateway: No default route found. Network not configured.".to_string()),
        }

        let started = std::time::Instant::now();
        let target: std::net::SocketAddr = ([1, 1, 1, 1], 443).into();
        match std::net::TcpStream::connect_timeout(&target, std::time::Duration::from_secs(4)) {
            Ok(_) => report.push(format!(
                "Internet: OK ({}ms to open a connection to 1.1.1.1:443).",
                started.elapsed().as_millis()
            )),
            Err(e) => report.push(format!("Internet: UNREACHABLE. Could not connect to 1.1.1.1:443 ({e}).")),
        }

        // 5. Wi-Fi, from the service. This used to be `sh -c "iw dev | grep …"`; `iw` is in
        //    neither of this OS's package lists, so the line was simply absent on every machine
        //    it builds, and a machine with no adapter and a machine with no `iw` looked identical.
        match self.net.wifi_state() {
            Ok(wifi) if !wifi.adapter_present => {
                report.push(format!(
                    "Wi-Fi: this machine has no Wi-Fi adapter{}.",
                    wifi.reason
                        .filter(|r| !r.to_lowercase().contains("no wi-fi adapter"))
                        .map(|r| format!(" ({r})"))
                        .unwrap_or_default()
                ));
            }
            Ok(wifi) => {
                let device = wifi.device.unwrap_or_else(|| "?".to_string());
                match wifi.connected_ssid {
                    Some(ssid) => report.push(format!(
                        "Wi-Fi ({device}): radio {}, joined to \"{ssid}\"{}.",
                        wifi.radio.as_str(),
                        wifi.signal
                            .map(|s| format!(", signal {s}%"))
                            .unwrap_or_default()
                    )),
                    None => report.push(format!(
                        "Wi-Fi ({device}): radio {}, not joined to any network.",
                        wifi.radio.as_str()
                    )),
                }
            }
            Err(why) => report.push(format!("Wi-Fi: not readable ({why}).")),
        }

        report.join("\n")
    }
}


/// The default route out of `/proc/net/route`: `(gateway, interface)`.
///
/// The file is a table of hex fields in host byte order; the default route is the row whose
/// destination and mask are both zero and whose flags carry RTF_GATEWAY (0x2).
fn default_route(proc_net_route: &str) -> Option<(String, String)> {
    proc_net_route.lines().skip(1).find_map(|line| {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 8 {
            return None;
        }
        let flags = u32::from_str_radix(f[3], 16).ok()?;
        if f[1] != "00000000" || f[7] != "00000000" || flags & 0x2 == 0 {
            return None;
        }
        let raw = u32::from_str_radix(f[2], 16).ok()?;
        // Little-endian on every machine this OS runs on: the first octet is the low byte.
        let ip = std::net::Ipv4Addr::from(raw.to_le_bytes());
        Some((ip.to_string(), f[0].to_string()))
    })
}

/// Whether the kernel has a completed neighbour entry for `ip` in `/proc/net/arp`.
/// `None` when it has no entry at all, which is "not known", not "down".
fn neighbour_resolved(proc_net_arp: &str, ip: &str) -> Option<bool> {
    proc_net_arp.lines().skip(1).find_map(|line| {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 4 || f[0] != ip {
            return None;
        }
        let flags = u32::from_str_radix(f[2].trim_start_matches("0x"), 16).ok()?;
        // ATF_COM (0x2): the entry is complete — the address answered an ARP request.
        Some(flags & 0x2 != 0 && f[3] != "00:00:00:00:00:00")
    })
}

#[cfg(test)]
mod route_tests {
    use super::{default_route, neighbour_resolved};

    const ROUTE: &str = "Iface	Destination	Gateway 	Flags	RefCnt	Use	Metric	Mask		MTU	Window	IRTT
ens18	00000000	0104A8C0	0003	0	0	100	00000000	0	0	0
ens18	0004A8C0	00000000	0001	0	0	100	00FFFFFF	0	0	0
";

    #[test]
    fn the_default_route_is_read_from_the_kernels_table() {
        assert_eq!(default_route(ROUTE), Some(("192.168.4.1".into(), "ens18".into())));
    }

    #[test]
    fn a_table_with_only_a_subnet_route_has_no_default() {
        let only_subnet = "Iface	Destination	Gateway	Flags	RefCnt	Use	Metric	Mask
ens18	0004A8C0	00000000	0001	0	0	100	00FFFFFF
";
        assert_eq!(default_route(only_subnet), None);
        assert_eq!(default_route(""), None);
    }

    #[test]
    fn a_gateway_that_answered_is_told_from_one_that_did_not_and_one_never_asked() {
        let arp = "IP address       HW type     Flags       HW address            Mask     Device
192.168.4.1      0x1         0x2         aa:bb:cc:dd:ee:ff     *        ens18
192.168.4.9      0x1         0x0         00:00:00:00:00:00     *        ens18
";
        assert_eq!(neighbour_resolved(arp, "192.168.4.1"), Some(true));
        assert_eq!(neighbour_resolved(arp, "192.168.4.9"), Some(false));
        assert_eq!(neighbour_resolved(arp, "192.168.4.77"), None);
    }
}

#[cfg(test)]
mod tests {
    use super::backend::fake::FakeNetwork;
    use super::*;
    use yantrik_ipc_contracts::network::{ConnectionType, DnsSetResult};

    #[test]
    fn a_host_is_never_read_as_an_option() {
        assert!(validate_host("-f").is_err());
        assert!(validate_host("--help").is_err());
        assert!(validate_host("example.com").is_ok());
        assert!(validate_host("my-host.lan").is_ok(), "a '-' inside a name is fine");
    }

    #[test]
    fn an_interface_with_no_address_says_so_rather_than_showing_a_blank() {
        let text = format_interfaces(&[NetworkInterfaceInfo {
            name: "eth0".into(),
            mac_address: "52:54:00:12:34:56".into(),
            ip_address: None,
            rx_bytes: 10,
            tx_bytes: 20,
            state: "down".into(),
            conn_type: ConnectionType::Ethernet,
            ..Default::default()
        }]);
        assert!(text.contains("no address"), "{text}");
        assert!(text.contains("eth0 [down]"), "{text}");
        // One row per interface. The tool this replaces ran `ip -br link` a second time and
        // printed a separate "MAC addresses" block, so a reader had to join two lists by eye.
        assert_eq!(text.lines().count(), 2, "{text}");
    }

    #[test]
    fn no_interfaces_at_all_is_reported_as_the_finding_it_is() {
        let text = format_interfaces(&[]);
        assert!(text.contains("no network interfaces"), "{text}");
    }

    #[test]
    fn a_service_that_will_not_answer_is_named_rather_than_drawn_as_an_empty_list() {
        let net = FakeNetwork {
            interfaces: Some(Err("the network service did not come up".into())),
            ..Default::default()
        };
        let text = match net.interfaces() {
            Ok(list) => format_interfaces(&list),
            Err(why) => refused("read this machine's network interfaces", &why),
        };
        assert!(text.starts_with("Could not read"), "{text}");
        assert!(text.contains("did not come up"), "{text}");
    }

    #[test]
    fn an_empty_nameserver_list_is_not_reported_as_a_working_resolver() {
        let text = format_dns(&DnsConfig::default());
        assert!(text.contains("no DNS server configured"), "{text}");
        assert!(!text.contains("DNS servers:"), "{text}");
    }

    #[test]
    fn a_service_that_is_down_is_named_rather_than_called_a_failure_to_set_dns() {
        let net = FakeNetwork {
            dns_set: Some(Err(
                "could not start the network service: no such file or directory".into(),
            )),
            ..Default::default()
        };
        let text = dns_set_answer(&net, &serde_json::json!({ "servers": ["1.1.1.1"] }));
        assert!(text.starts_with("Could not set DNS to 1.1.1.1"), "{text}");
        assert!(text.contains("network service"), "{text}");
        // And what was asked for still went out unmangled.
        assert_eq!(net.dns_set_seen.lock().unwrap()[0].servers, vec!["1.1.1.1"]);
    }

    #[test]
    fn setting_dns_reports_what_the_machine_reads_back_not_what_was_asked_for() {
        let net = FakeNetwork {
            dns_set: Some(Ok(DnsSetResult {
                connection: "Wired connection 1".into(),
                device: "enp0s3".into(),
                resolv_conf: DnsConfig {
                    nameservers: vec!["1.1.1.1".into(), "1.0.0.1".into()],
                    search_domains: vec!["lan".into()],
                },
                device_dns: vec!["1.1.1.1".into(), "1.0.0.1".into()],
            })),
            ..Default::default()
        };
        let text = dns_set_answer(&net, &serde_json::json!({ "servers": ["1.1.1.1", "1.0.0.1"] }));
        assert!(text.contains("Wired connection 1"), "{text}");
        assert!(text.contains("enp0s3"), "{text}");
        assert!(text.contains("/etc/resolv.conf now: 1.1.1.1, 1.0.0.1"), "{text}");
        // The interruption is stated, because the caller may be on the connection being changed.
        assert!(text.contains("re-activated"), "{text}");
    }

    #[test]
    fn a_dns_set_with_no_servers_is_refused_before_the_service_is_asked() {
        let net = FakeNetwork::default();
        let text = dns_set_answer(&net, &serde_json::json!({}));
        assert!(text.starts_with("Error:"), "{text}");
        assert!(net.dns_set_seen.lock().unwrap().is_empty());
    }

    #[test]
    fn one_server_given_as_a_bare_string_is_taken_as_a_list_of_one() {
        let net = FakeNetwork {
            dns_set: Some(Err("refused".into())),
            ..Default::default()
        };
        let _ = dns_set_answer(&net, &serde_json::json!({ "servers": "9.9.9.9" }));
        assert_eq!(net.dns_set_seen.lock().unwrap()[0].servers, vec!["9.9.9.9"]);
    }
}
