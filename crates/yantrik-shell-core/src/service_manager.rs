//! Service Lifecycle Manager — starts, stops, and monitors service processes.
//!
//! The shell uses this to manage standalone service binaries (weather-service,
//! system-monitor-service, etc.) as child processes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::{Arc, Mutex};

/// Status of a managed service.
#[derive(Debug, Clone, PartialEq)]
pub enum ServiceStatus {
    Stopped,
    Starting,
    Running,
    Failed(String),
}

/// Info about a registered service.
#[derive(Debug, Clone)]
pub struct ServiceEntry {
    pub id: String,
    pub binary: PathBuf,
    pub autostart: bool,
    pub status: ServiceStatus,
}

/// Manages service process lifecycles.
#[derive(Clone)]
pub struct ServiceManager {
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    services: HashMap<String, ServiceEntry>,
    processes: HashMap<String, Child>,
    services_dir: PathBuf,
}

impl ServiceManager {
    /// Create a new service manager.
    /// `services_dir` is the directory containing service binaries.
    pub fn new(services_dir: PathBuf) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                services: HashMap::new(),
                processes: HashMap::new(),
                services_dir,
            })),
        }
    }

    /// Register a service for management.
    pub fn register(&self, id: &str, binary_name: &str, autostart: bool) {
        let mut inner = self.inner.lock().unwrap();
        let binary = inner.services_dir.join(binary_name);
        inner.services.insert(
            id.to_string(),
            ServiceEntry {
                id: id.to_string(),
                binary,
                autostart,
                status: ServiceStatus::Stopped,
            },
        );
        tracing::info!(service = id, "Service registered");
    }

    /// Start all services marked as autostart.
    pub fn start_autostart(&self) {
        let ids: Vec<String> = {
            let inner = self.inner.lock().unwrap();
            inner
                .services
                .values()
                .filter(|s| s.autostart)
                .map(|s| s.id.clone())
                .collect()
        };

        for id in ids {
            if let Err(e) = self.start(&id) {
                tracing::error!(service = %id, error = %e, "Failed to autostart service");
            }
        }
    }

    /// Start a service by ID.
    pub fn start(&self, id: &str) -> Result<(), String> {
        let mut inner = self.inner.lock().unwrap();

        let entry = inner
            .services
            .get(id)
            .ok_or_else(|| format!("Unknown service: {id}"))?;

        if entry.status == ServiceStatus::Running {
            return Ok(());
        }

        if !entry.binary.exists() {
            let msg = format!("Binary not found: {}", entry.binary.display());
            inner.services.get_mut(id).unwrap().status =
                ServiceStatus::Failed(msg.clone());
            return Err(msg);
        }

        let binary = entry.binary.clone();
        inner.services.get_mut(id).unwrap().status = ServiceStatus::Starting;
        tracing::info!(service = id, binary = %binary.display(), "Starting service");

        match Command::new(&binary).spawn() {
            Ok(child) => {
                inner.services.get_mut(id).unwrap().status = ServiceStatus::Running;
                inner.processes.insert(id.to_string(), child);
                tracing::info!(service = id, "Service started");
                Ok(())
            }
            Err(e) => {
                let msg = format!("Failed to start {id}: {e}");
                inner.services.get_mut(id).unwrap().status =
                    ServiceStatus::Failed(msg.clone());
                Err(msg)
            }
        }
    }

    /// Stop a service by ID.
    pub fn stop(&self, id: &str) -> Result<(), String> {
        let mut inner = self.inner.lock().unwrap();

        if let Some(mut child) = inner.processes.remove(id) {
            tracing::info!(service = id, "Stopping service");
            let _ = child.kill();
            let _ = child.wait();
        }

        if let Some(entry) = inner.services.get_mut(id) {
            entry.status = ServiceStatus::Stopped;
        }
        Ok(())
    }

    /// Get the status of a service.
    pub fn status(&self, id: &str) -> Option<ServiceStatus> {
        let mut inner = self.inner.lock().unwrap();

        // Check if process is still alive
        if let Some(child) = inner.processes.get_mut(id) {
            match child.try_wait() {
                Ok(Some(status)) => {
                    // Process exited
                    inner.processes.remove(id);
                    if let Some(entry) = inner.services.get_mut(id) {
                        if status.success() {
                            entry.status = ServiceStatus::Stopped;
                        } else {
                            entry.status = ServiceStatus::Failed(
                                format!("Exited with code: {:?}", status.code()),
                            );
                        }
                    }
                }
                Ok(None) => {
                    // Still running
                }
                Err(e) => {
                    tracing::warn!(service = id, error = %e, "Error checking service status");
                }
            }
        }

        inner.services.get(id).map(|s| s.status.clone())
    }

    /// List all registered services.
    pub fn list(&self) -> Vec<ServiceEntry> {
        let inner = self.inner.lock().unwrap();
        inner.services.values().cloned().collect()
    }

    /// Stop all running services.
    pub fn stop_all(&self) {
        let ids: Vec<String> = {
            let inner = self.inner.lock().unwrap();
            inner.processes.keys().cloned().collect()
        };
        for id in ids {
            let _ = self.stop(&id);
        }
    }

    /// Scan a directory for service manifests (yantrik.toml) and register them.
    pub fn scan_and_register(&self, manifests_dir: &Path) {
        let entries = match std::fs::read_dir(manifests_dir) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(path = %manifests_dir.display(), error = %e, "Cannot scan service dir");
                return;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let manifest_path = path.join("yantrik.toml");
            if !manifest_path.exists() {
                continue;
            }
            // Simple TOML parsing for service registration
            if let Ok(content) = std::fs::read_to_string(&manifest_path) {
                let id = extract_toml_value(&content, "id").unwrap_or_default();
                let binary = extract_toml_value(&content, "binary").unwrap_or_default();
                let autostart = extract_toml_value(&content, "autostart")
                    .map(|v| v == "true")
                    .unwrap_or(false);

                if !id.is_empty() && !binary.is_empty() {
                    self.register(&id, &binary, autostart);
                }
            }
        }
    }
}

/// Simple TOML value extractor (avoids pulling in toml crate for shell-core).
fn extract_toml_value(content: &str, key: &str) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with(key) && trimmed.contains('=') {
            let val = trimmed.split('=').nth(1)?.trim();
            let val = val.trim_matches('"').trim_matches('\'');
            return Some(val.to_string());
        }
    }
    None
}

impl Drop for ServiceManager {
    fn drop(&mut self) {
        self.stop_all();
    }
}
