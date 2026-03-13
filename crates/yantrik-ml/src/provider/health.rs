//! Provider health tracking — status, latency, and staleness.

use std::time::Instant;

use serde::{Deserialize, Serialize};

/// Health status for a provider endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HealthStatus {
    /// Provider is reachable and responding normally.
    Healthy,
    /// Provider is reachable but slow or partially degraded.
    Degraded,
    /// Provider is unreachable (network error, timeout).
    Unavailable,
    /// Provider rejected authentication (invalid/expired API key).
    AuthError,
    /// Provider is rate-limiting requests.
    RateLimited,
}

impl HealthStatus {
    /// Whether the provider is usable (possibly degraded but functional).
    pub fn is_usable(&self) -> bool {
        matches!(self, HealthStatus::Healthy | HealthStatus::Degraded)
    }
}

/// Tracks the health of a single provider.
#[derive(Debug, Clone)]
pub struct ProviderHealth {
    /// Current health status.
    pub status: HealthStatus,
    /// When the last health check was performed.
    pub last_check: Option<Instant>,
    /// Latency of the last successful probe (milliseconds).
    pub latency_ms: Option<u64>,
    /// Number of consecutive failures since last success.
    pub consecutive_failures: u32,
    /// Human-readable error from the last failure (if any).
    pub last_error: Option<String>,
}

impl Default for ProviderHealth {
    fn default() -> Self {
        Self {
            status: HealthStatus::Healthy,
            last_check: None,
            latency_ms: None,
            consecutive_failures: 0,
            last_error: None,
        }
    }
}

impl ProviderHealth {
    /// Record a successful health check.
    pub fn record_success(&mut self, latency_ms: u64) {
        self.status = if latency_ms > 5000 {
            HealthStatus::Degraded
        } else {
            HealthStatus::Healthy
        };
        self.last_check = Some(Instant::now());
        self.latency_ms = Some(latency_ms);
        self.consecutive_failures = 0;
        self.last_error = None;
    }

    /// Record a failed health check.
    pub fn record_failure(&mut self, error: &str) {
        self.consecutive_failures += 1;
        self.last_check = Some(Instant::now());
        self.last_error = Some(error.to_string());

        // Classify failure type
        let err_lower = error.to_lowercase();
        if err_lower.contains("401") || err_lower.contains("403") || err_lower.contains("unauthorized") || err_lower.contains("forbidden") {
            self.status = HealthStatus::AuthError;
        } else if err_lower.contains("429") || err_lower.contains("rate limit") {
            self.status = HealthStatus::RateLimited;
        } else {
            self.status = HealthStatus::Unavailable;
        }
    }

    /// Whether the health data is stale (older than the given duration).
    pub fn is_stale(&self, max_age: std::time::Duration) -> bool {
        match self.last_check {
            None => true,
            Some(t) => t.elapsed() > max_age,
        }
    }
}
