//! ProviderManager — spawns and monitors provider threads.
//!
//! One OS thread per active provider. Each thread runs the provider's
//! poll() or WebSocket loop and pushes InboundEvents to the router.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use crossbeam_channel::Sender;

use crate::model::*;
use crate::provider::*;
use crate::router::RouterEvent;

/// Status of a managed provider thread.
#[derive(Debug)]
pub struct ProviderStatus {
    pub id: String,
    pub health: ProviderHealth,
    pub last_event_at: Option<Instant>,
    pub error_count: u32,
    pub events_received: u64,
}

/// Manages provider lifecycle: connect, run, reconnect, health.
pub struct ProviderManager {
    /// Running provider threads.
    threads: HashMap<String, JoinHandle<()>>,
    /// Shared status map.
    statuses: Arc<Mutex<HashMap<String, ProviderStatus>>>,
    /// Channel to push events to the router.
    event_tx: Sender<(String, InboundEvent)>,
    /// Optional channel for router events (health changes).
    router_event_tx: Option<Sender<RouterEvent>>,
}

impl ProviderManager {
    pub fn new(
        event_tx: Sender<(String, InboundEvent)>,
        router_event_tx: Option<Sender<RouterEvent>>,
    ) -> Self {
        Self {
            threads: HashMap::new(),
            statuses: Arc::new(Mutex::new(HashMap::new())),
            event_tx,
            router_event_tx,
        }
    }

    /// Start a provider in its own thread.
    /// Takes ownership of the provider.
    pub fn start_provider(&mut self, mut provider: Box<dyn ChatProvider>) {
        let id = provider.id().to_string();
        let tx = self.event_tx.clone();
        let statuses = Arc::clone(&self.statuses);
        let router_tx = self.router_event_tx.clone();
        let provider_id = id.clone();

        // Initialize status
        if let Ok(mut map) = statuses.lock() {
            map.insert(id.clone(), ProviderStatus {
                id: id.clone(),
                health: ProviderHealth::Connecting,
                last_event_at: None,
                error_count: 0,
                events_received: 0,
            });
        }

        let handle = thread::Builder::new()
            .name(format!("chat-{}", id))
            .spawn(move || {
                provider_loop(&provider_id, &mut *provider, &tx, &statuses, &router_tx);
            })
            .expect("Failed to spawn provider thread");

        tracing::info!(provider = %id, "Chat provider thread started");
        self.threads.insert(id, handle);
    }

    /// Get current status of all providers.
    pub fn statuses(&self) -> Vec<ProviderStatus> {
        if let Ok(map) = self.statuses.lock() {
            map.values().map(|s| ProviderStatus {
                id: s.id.clone(),
                health: s.health,
                last_event_at: s.last_event_at,
                error_count: s.error_count,
                events_received: s.events_received,
            }).collect()
        } else {
            vec![]
        }
    }

    /// Check if a provider thread is still running.
    pub fn is_running(&self, id: &str) -> bool {
        self.threads.get(id).map(|h| !h.is_finished()).unwrap_or(false)
    }

    /// Stop all providers.
    pub fn stop_all(&mut self) {
        // Signal threads to stop by dropping the event channel
        // (they'll get a send error and exit)
        self.threads.clear();
        tracing::info!("All chat provider threads stopped");
    }
}

/// Main loop for a single provider thread.
fn provider_loop(
    id: &str,
    provider: &mut dyn ChatProvider,
    tx: &Sender<(String, InboundEvent)>,
    statuses: &Arc<Mutex<HashMap<String, ProviderStatus>>>,
    router_tx: &Option<Sender<RouterEvent>>,
) {
    let mut backoff = Duration::from_secs(1);
    let max_backoff = Duration::from_secs(60);
    let mut consecutive_errors: u32 = 0;

    loop {
        // Connect
        update_health(id, ProviderHealth::Connecting, statuses, router_tx);
        match provider.connect() {
            Ok(()) => {
                tracing::info!(provider = id, "Connected");
                update_health(id, ProviderHealth::Connected, statuses, router_tx);
                backoff = Duration::from_secs(1);
                consecutive_errors = 0;
            }
            Err(e) => {
                tracing::error!(provider = id, error = %e, "Connection failed, retrying");
                update_health(id, ProviderHealth::Error, statuses, router_tx);
                increment_errors(id, statuses);
                thread::sleep(backoff);
                backoff = (backoff * 2).min(max_backoff);
                continue;
            }
        }

        // Poll loop
        loop {
            match provider.poll() {
                Ok(events) => {
                    consecutive_errors = 0;
                    for event in events {
                        if tx.send((id.to_string(), event)).is_err() {
                            // Router channel closed — exit thread
                            tracing::info!(provider = id, "Router channel closed, exiting");
                            let _ = provider.disconnect();
                            return;
                        }
                        increment_events(id, statuses);
                    }
                }
                Err(ChatError::RateLimited(secs)) => {
                    tracing::warn!(provider = id, secs, "Rate limited");
                    thread::sleep(Duration::from_secs(secs));
                }
                Err(ChatError::Network(msg)) => {
                    tracing::warn!(provider = id, error = msg, "Network error, reconnecting");
                    consecutive_errors += 1;
                    increment_errors(id, statuses);
                    if consecutive_errors > 10 {
                        update_health(id, ProviderHealth::Error, statuses, router_tx);
                    }
                    break; // Break inner loop → reconnect
                }
                Err(e) => {
                    tracing::error!(provider = id, error = %e, "Poll error");
                    consecutive_errors += 1;
                    increment_errors(id, statuses);
                    if consecutive_errors > 5 {
                        break; // Reconnect after too many errors
                    }
                    thread::sleep(Duration::from_secs(2));
                }
            }
        }

        // Disconnect before reconnecting
        let _ = provider.disconnect();
        update_health(id, ProviderHealth::Disconnected, statuses, router_tx);

        // Backoff before reconnect
        thread::sleep(backoff);
        backoff = (backoff * 2).min(max_backoff);
    }
}

fn update_health(
    id: &str,
    health: ProviderHealth,
    statuses: &Arc<Mutex<HashMap<String, ProviderStatus>>>,
    router_tx: &Option<Sender<RouterEvent>>,
) {
    if let Ok(mut map) = statuses.lock() {
        if let Some(status) = map.get_mut(id) {
            status.health = health;
        }
    }
    if let Some(tx) = router_tx {
        let _ = tx.send(RouterEvent::ProviderStatus {
            provider: id.to_string(),
            health,
        });
    }
}

fn increment_errors(id: &str, statuses: &Arc<Mutex<HashMap<String, ProviderStatus>>>) {
    if let Ok(mut map) = statuses.lock() {
        if let Some(status) = map.get_mut(id) {
            status.error_count += 1;
        }
    }
}

fn increment_events(id: &str, statuses: &Arc<Mutex<HashMap<String, ProviderStatus>>>) {
    if let Ok(mut map) = statuses.lock() {
        if let Some(status) = map.get_mut(id) {
            status.events_received += 1;
            status.last_event_at = Some(Instant::now());
        }
    }
}
