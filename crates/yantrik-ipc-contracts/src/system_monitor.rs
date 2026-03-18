//! System monitor service contract — CPU, memory, disk, network, processes.

use serde::{Deserialize, Serialize};
use crate::email::ServiceError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpuInfo {
    pub overall_percent: f64,
    pub cores: Vec<CpuCore>,
    pub load_avg_1: f64,
    pub load_avg_5: f64,
    pub load_avg_15: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpuCore {
    pub id: u32,
    pub usage_percent: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryInfo {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub usage_percent: f64,
    pub swap_total_bytes: u64,
    pub swap_used_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskInfo {
    pub mount_point: String,
    pub device: String,
    pub filesystem: String,
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub usage_percent: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkInterface {
    pub name: String,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub rx_rate_bps: u64,
    pub tx_rate_bps: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub cpu_percent: f64,
    pub mem_percent: f64,
    pub mem_bytes: u64,
    pub state: String,
    pub user: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemSnapshot {
    pub cpu: CpuInfo,
    pub memory: MemoryInfo,
    pub disks: Vec<DiskInfo>,
    pub networks: Vec<NetworkInterface>,
    pub uptime_secs: u64,
}

/// System monitor service operations.
pub trait SystemMonitorService: Send + Sync {
    fn snapshot(&self) -> Result<SystemSnapshot, ServiceError>;
    fn processes(&self, sort_by: &str, limit: u32) -> Result<Vec<ProcessInfo>, ServiceError>;
    fn kill_process(&self, pid: u32) -> Result<(), ServiceError>;
}
