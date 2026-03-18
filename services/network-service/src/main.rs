//! Network service — reads /proc for interface stats, connectivity, and DNS.
//!
//! This service is Linux-only (reads /proc, /etc/resolv.conf). On non-Unix
//! platforms it compiles but returns stub data for development purposes.
//!
//! Methods:
//!   network.interfaces  {}  -> Vec<NetworkInterfaceInfo>
//!   network.status      {}  -> NetworkStatus
//!   network.dns         {}  -> DnsConfig

use serde::{Deserialize, Serialize};
use yantrik_ipc_contracts::network::*;
use yantrik_service_sdk::prelude::*;

// ── Response types (not in contracts yet) ──────────────────────────────

/// Extended interface info combining /proc/net/dev stats with ip-addr metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkInterfaceInfo {
    pub name: String,
    pub mac_address: String,
    pub ip_address: Option<String>,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub state: String,
    pub conn_type: ConnectionType,
}

/// Overall connectivity status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkStatus {
    pub connected: bool,
    #[serde(rename = "type")]
    pub conn_type: String,
    pub ssid: Option<String>,
    pub ip_address: Option<String>,
}

/// DNS resolver configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsConfig {
    pub nameservers: Vec<String>,
    pub search_domains: Vec<String>,
}

fn main() {
    ServiceBuilder::new("network")
        .handler(NetworkHandler)
        .run();
}

struct NetworkHandler;

impl ServiceHandler for NetworkHandler {
    fn service_id(&self) -> &str {
        "network"
    }

    fn handle(
        &self,
        method: &str,
        _params: serde_json::Value,
    ) -> Result<serde_json::Value, ServiceError> {
        match method {
            "network.interfaces" => {
                let ifaces = read_interfaces()?;
                Ok(serde_json::to_value(ifaces).unwrap())
            }
            "network.status" => {
                let status = read_status()?;
                Ok(serde_json::to_value(status).unwrap())
            }
            "network.dns" => {
                let dns = read_dns()?;
                Ok(serde_json::to_value(dns).unwrap())
            }
            _ => Err(ServiceError {
                code: -1,
                message: format!("Unknown method: {method}"),
            }),
        }
    }
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
                let type_str = match &iface.conn_type {
                    ConnectionType::Wifi => "wifi",
                    ConnectionType::Ethernet => "ethernet",
                    ConnectionType::Vpn => "vpn",
                    ConnectionType::Bridge => "bridge",
                    ConnectionType::Other(_) => "other",
                };

                let ssid = if matches!(iface.conn_type, ConnectionType::Wifi) {
                    read_wifi_ssid(&iface.name)
                } else {
                    None
                };

                return Ok(NetworkStatus {
                    connected: true,
                    conn_type: type_str.to_string(),
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
            match content.trim() {
                "801" => return ConnectionType::Wifi,
                _ => {}
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

    /// Try to read the IP address for an interface from /proc/net/fib_trie or
    /// by parsing the output format of ip-addr. We use /proc/net/if_inet6 and
    /// a simpler /proc-based approach to avoid shelling out.
    fn read_interface_ip(name: &str) -> Option<String> {
        // Try reading from /proc/net/fib_trie — parse is complex, so we use
        // a simpler approach: read the route table for interface-specific IPs.
        let content = std::fs::read_to_string("/proc/net/fib_trie").ok()?;

        // The fib_trie format is complex; use a simpler fallback:
        // Read /proc/net/route to find the interface, then try to get its
        // configured address from the ioctl-less /sys approach.
        // Actually, the cleanest /proc-only approach is to parse
        // /proc/net/if_inet6 for IPv6 or use SIOCGIFADDR via libc.
        drop(content);

        // Use libc ioctl to get IPv4 address without shelling out
        get_ipv4_addr(name)
    }

    /// Get IPv4 address for an interface using libc ioctl.
    fn get_ipv4_addr(iface_name: &str) -> Option<String> {
        use std::ffi::CString;
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
        if addr.sa_family != libc::AF_INET as u16 {
            return None;
        }

        let sin: libc::sockaddr_in = unsafe { mem::transmute(addr) };
        let ip = sin.sin_addr.s_addr.to_ne_bytes();
        Some(format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]))
    }

    /// Try to read the current WiFi SSID from /proc/net/wireless or iwconfig.
    fn read_wifi_ssid(iface: &str) -> Option<String> {
        // Try reading from /proc — limited info available without iw/iwconfig.
        // As a fallback, try to read the wireless essid via ioctl.
        // For now, return None; a future version can use nl80211.
        let _ = iface;
        None
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
        Ok(NetworkStatus {
            connected: false,
            conn_type: "none".to_string(),
            ssid: None,
            ip_address: None,
        })
    }

    pub fn read_dns() -> Result<DnsConfig, ServiceError> {
        Ok(DnsConfig {
            nameservers: Vec::new(),
            search_domains: Vec::new(),
        })
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
