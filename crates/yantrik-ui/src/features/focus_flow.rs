//! Focus Flow — detects deep work and manages interruptions.
//!
//! States: NORMAL → DEEP_WORK (after 20min no switch) → break reminder at 90min.
//! SCATTERED state (>5 switches/min for 3min) — currently just tracked.
//! In DEEP_WORK: suppresses low-urgency whispers via the UrgencyScorer.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use yantrik_os::SystemEvent;

use super::{FeatureContext, ProactiveFeature, Urge, UrgeCategory};

/// Focus states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FocusState {
    Normal,
    DeepWork { since: Instant },
    Scattered { since: Instant },
}

pub struct FocusFlow {
    state: FocusState,
    /// Timestamps of recent app switches (for scatter detection).
    switch_times: VecDeque<Instant>,
    /// Last time we reminded about a break.
    last_break_reminder: Option<Instant>,
    /// Time with no app switch to enter deep work.
    deep_work_threshold: Duration,
    /// Deep work duration before suggesting a break.
    break_reminder_duration: Duration,
    /// Last app switch time.
    last_switch: Instant,
    /// Current interruptibility (output for UrgencyScorer).
    interruptibility: f32,
}

impl FocusFlow {
    pub fn new() -> Self {
        Self {
            state: FocusState::Normal,
            switch_times: VecDeque::new(),
            last_break_reminder: None,
            deep_work_threshold: Duration::from_secs(20 * 60),  // 20 minutes
            break_reminder_duration: Duration::from_secs(90 * 60), // 90 minutes
            last_switch: Instant::now(),
            interruptibility: 1.0,
        }
    }

    /// Current interruptibility level (read by the main loop to set UrgencyScorer).
    pub fn interruptibility(&self) -> f32 {
        self.interruptibility
    }

    fn record_switch(&mut self) {
        let now = Instant::now();
        self.last_switch = now;
        self.switch_times.push_back(now);

        // Keep only last 3 minutes of switches
        let cutoff = now - Duration::from_secs(180);
        while self.switch_times.front().map_or(false, |t| *t < cutoff) {
            self.switch_times.pop_front();
        }
    }

    fn switches_per_minute(&self) -> f32 {
        if self.switch_times.len() < 2 {
            return 0.0;
        }
        let window = Instant::now()
            .duration_since(*self.switch_times.front().unwrap())
            .as_secs_f32();
        if window < 1.0 {
            return 0.0;
        }
        self.switch_times.len() as f32 / (window / 60.0)
    }
}

impl ProactiveFeature for FocusFlow {
    fn name(&self) -> &str {
        "focus_flow"
    }

    fn on_event(&mut self, event: &SystemEvent, _ctx: &FeatureContext) -> Vec<Urge> {
        match event {
            SystemEvent::ProcessStarted { .. } => {
                // Process starts often correlate with app switches
                self.record_switch();
            }
            SystemEvent::UserIdle { idle_seconds } => {
                // If user is idle for a while during deep work, that's fine — still deep work
                if *idle_seconds > 300 {
                    // 5 min idle — reset to normal
                    self.state = FocusState::Normal;
                    self.interruptibility = 1.0;
                }
            }
            SystemEvent::UserResumed => {
                // Coming back from idle — reset switch tracking
                self.switch_times.clear();
                self.last_switch = Instant::now();
            }
            _ => {}
        }

        Vec::new()
    }

    fn on_tick(&mut self, _ctx: &FeatureContext) -> Vec<Urge> {
        let mut urges = Vec::new();
        let now = Instant::now();
        let since_last_switch = now.duration_since(self.last_switch);

        // State transitions
        match &self.state {
            FocusState::Normal => {
                // Check for deep work entry
                if since_last_switch >= self.deep_work_threshold {
                    tracing::info!("Entering DEEP_WORK state");
                    self.state = FocusState::DeepWork { since: now };
                    self.interruptibility = 0.3; // Suppress most whispers
                }

                // Check for scattered state
                if self.switches_per_minute() > 5.0 {
                    self.state = FocusState::Scattered { since: now };
                    self.interruptibility = 1.0;
                }
            }

            FocusState::DeepWork { since } => {
                // Still deep? Check if user started switching again
                if since_last_switch < Duration::from_secs(60) && self.switches_per_minute() > 2.0 {
                    tracing::info!("Exiting DEEP_WORK (switching resumed)");
                    self.state = FocusState::Normal;
                    self.interruptibility = 1.0;
                    return urges;
                }

                // Break reminder
                let deep_duration = now.duration_since(*since);
                if deep_duration >= self.break_reminder_duration {
                    let should_remind = self
                        .last_break_reminder
                        .map_or(true, |t| now.duration_since(t) > Duration::from_secs(30 * 60));

                    if should_remind {
                        self.last_break_reminder = Some(now);
                        let mins = deep_duration.as_secs() / 60;
                        // V15: Bond-aware break reminders
                        let body = match _ctx.bond_level {
                            1 => format!(
                                "You've been focused for {} minutes. A short break is recommended.",
                                mins
                            ),
                            2 => format!(
                                "You've been focused for {} minutes. Stretch, hydrate, look away from the screen.",
                                mins
                            ),
                            3 => format!(
                                "Nice {} minute focus session. Time for a breather \u{2014} even machines need cooldown.",
                                mins
                            ),
                            4 => format!(
                                "{} minutes of deep work. Your eyes will thank you for a break.",
                                mins
                            ),
                            _ => format!(
                                "{} minutes straight. Get up. Stretch. That's an order.",
                                mins
                            ),
                        };
                        urges.push(Urge {
                            id: format!("ff:break:{}", mins),
                            source: "focus_flow".into(),
                            title: "Take a break".into(),
                            body,
                            urgency: 0.5,
                            confidence: 0.9,
                            category: UrgeCategory::Focus,
                        });
                    }
                }
            }

            FocusState::Scattered { since } => {
                // Exit scattered if switching slows down
                if self.switches_per_minute() < 3.0 {
                    let duration = now.duration_since(*since);
                    if duration > Duration::from_secs(60) {
                        self.state = FocusState::Normal;
                        self.interruptibility = 1.0;
                    }
                }
            }
        }

        urges
    }
}
