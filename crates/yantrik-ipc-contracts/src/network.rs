//! Network manager service contract — connections, interfaces, WiFi.

use serde::{Deserialize, Serialize};
use crate::email::ServiceError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConnection {
    pub id: String,
    pub name: String,
    pub conn_type: ConnectionType,
    pub state: ConnectionState,
    pub device: String,
    pub ip_address: Option<String>,
    pub gateway: Option<String>,
    pub dns: Vec<String>,
    pub signal_strength: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConnectionType {
    Ethernet,
    Wifi,
    Vpn,
    Bridge,
    Other(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConnectionState {
    Connected,
    Disconnected,
    Connecting,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WifiNetwork {
    pub ssid: String,
    pub signal_strength: i32,
    pub security: String,
    pub is_saved: bool,
}

/// Network manager service operations.
pub trait NetworkService: Send + Sync {
    fn list_connections(&self) -> Result<Vec<NetworkConnection>, ServiceError>;
    fn connect(&self, connection_id: &str) -> Result<(), ServiceError>;
    fn disconnect(&self, connection_id: &str) -> Result<(), ServiceError>;
    fn scan_wifi(&self) -> Result<Vec<WifiNetwork>, ServiceError>;
    fn connect_wifi(&self, ssid: &str, password: &str) -> Result<(), ServiceError>;
}
