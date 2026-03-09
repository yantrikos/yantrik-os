//! Ambient Intelligence — the desktop breathes with you.
//!
//! Tracks sentiment (from user messages), cognitive load (from activity frequency),
//! and time of day to drive particle field visual responses.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;
use std::time::Instant;

/// Thread-safe ambient state shared between bridge worker and UI poll.
#[derive(Clone)]
pub struct AmbientState {
    /// Sentiment: -1000 to +1000 (stored as i32, divide by 1000 for float).
    /// Negative = frustrated, positive = happy.
    sentiment: Arc<AtomicI32>,
    /// Cognitive load: 0 to 1000 (stored as i32, divide by 1000 for float).
    cognitive_load: Arc<AtomicI32>,
    /// Message timestamps for cognitive load calculation.
    message_times: Arc<std::sync::Mutex<Vec<Instant>>>,
}

impl AmbientState {
    pub fn new() -> Self {
        Self {
            sentiment: Arc::new(AtomicI32::new(0)),
            cognitive_load: Arc::new(AtomicI32::new(0)),
            message_times: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    /// Update sentiment from a user message. Uses keyword-based fast analysis.
    /// Exponential moving average: sentiment = 0.8 * old + 0.2 * new_score.
    pub fn update_from_message(&self, user_text: &str) {
        let score = analyze_sentiment(user_text);
        let old = self.sentiment.load(Ordering::Relaxed) as f32 / 1000.0;
        let new_val = (0.8 * old + 0.2 * score).clamp(-1.0, 1.0);
        self.sentiment.store((new_val * 1000.0) as i32, Ordering::Relaxed);

        // Track message time for cognitive load
        let now = Instant::now();
        if let Ok(mut times) = self.message_times.lock() {
            times.push(now);
            // Keep only last 5 minutes
            let cutoff = now - std::time::Duration::from_secs(300);
            times.retain(|t| *t > cutoff);

            // Cognitive load = messages in 5 min / 10, clamped 0-1
            let load = (times.len() as f32 / 10.0).clamp(0.0, 1.0);
            self.cognitive_load.store((load * 1000.0) as i32, Ordering::Relaxed);
        }
    }

    /// Get current sentiment as f32 (-1.0 to 1.0).
    pub fn sentiment(&self) -> f32 {
        self.sentiment.load(Ordering::Relaxed) as f32 / 1000.0
    }

    /// Get current cognitive load as f32 (0.0 to 1.0).
    pub fn cognitive_load(&self) -> f32 {
        self.cognitive_load.load(Ordering::Relaxed) as f32 / 1000.0
    }

    /// Calculate time-of-day factor (0.0 at midnight, peaks ~0.5 at noon).
    /// Uses sine curve for natural day/night feel.
    pub fn time_of_day() -> f32 {
        let now = chrono::Local::now();
        let hour = now.hour() as f32 + now.minute() as f32 / 60.0;
        // Map 0-24 to sine: 0 at midnight, 1 at noon, 0 at midnight
        // sin((hour/24 - 0.25) * 2π) maps noon → peak, midnight → trough
        let normalized = (hour / 24.0 - 0.25) * std::f32::consts::TAU;
        (normalized.sin() * 0.5 + 0.5).clamp(0.0, 1.0)
    }
}

/// Fast keyword-based sentiment analysis. No LLM call needed.
/// Returns -1.0 to 1.0.
fn analyze_sentiment(text: &str) -> f32 {
    let lower = text.to_lowercase();
    let words: Vec<&str> = lower.split_whitespace().collect();

    let positive = [
        "thanks", "thank", "great", "awesome", "perfect", "love", "nice",
        "cool", "yes", "yeah", "good", "excellent", "amazing", "wonderful",
        "brilliant", "fantastic", "helpful", "exactly", "beautiful", "sweet",
    ];
    let negative = [
        "no", "wrong", "broken", "error", "fail", "failed", "hate", "ugh",
        "damn", "fix", "bug", "shit", "fuck", "terrible", "horrible",
        "annoying", "frustrated", "sucks", "bad", "worse", "worst",
    ];

    let mut score = 0.0f32;
    let mut count = 0.0f32;

    for word in &words {
        if positive.contains(word) {
            score += 1.0;
            count += 1.0;
        } else if negative.contains(word) {
            score -= 1.0;
            count += 1.0;
        }
    }

    if count == 0.0 {
        0.0 // neutral
    } else {
        (score / count).clamp(-1.0, 1.0)
    }
}

// Local time via libc::localtime_r (respects /etc/localtime)
mod chrono {
    pub struct Local;
    impl Local {
        pub fn now() -> DateTime {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            let mut tm: libc::tm = unsafe { std::mem::zeroed() };
            unsafe { libc::localtime_r(&ts as *const i64, &mut tm) };
            DateTime {
                hour: tm.tm_hour as u32,
                minute: tm.tm_min as u32,
            }
        }
    }

    pub struct DateTime {
        hour: u32,
        minute: u32,
    }

    impl DateTime {
        pub fn hour(&self) -> u32 {
            self.hour
        }
        pub fn minute(&self) -> u32 {
            self.minute
        }
    }
}
