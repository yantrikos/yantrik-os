//! ScreenWatcher — periodic screen analysis via computer vision.
//!
//! Every 5 minutes when the user is idle, captures a screenshot,
//! sends it to the vision model, and tracks activity changes.
//! Generates urges when significant activity shifts are detected.

use super::{FeatureContext, Outcome, ProactiveFeature, Urge, UrgeCategory};
use yantrik_os::SystemEvent;

/// How often to check the screen (in ticks, at 3s per tick = ~5 min).
const CHECK_INTERVAL_TICKS: u32 = 100;
/// Minimum idle time before checking (seconds).
const MIN_IDLE_SECS: u64 = 120;

pub struct ScreenWatcher {
    tick_count: u32,
    last_activity: String,
    last_check_ts: f64,
}

impl ScreenWatcher {
    pub fn new() -> Self {
        Self {
            tick_count: 0,
            last_activity: String::new(),
            last_check_ts: 0.0,
        }
    }
}

impl ProactiveFeature for ScreenWatcher {
    fn name(&self) -> &str { "ScreenWatcher" }

    fn on_event(&mut self, _event: &SystemEvent, _ctx: &FeatureContext) -> Vec<Urge> {
        Vec::new()
    }

    fn on_tick(&mut self, ctx: &FeatureContext) -> Vec<Urge> {
        self.tick_count += 1;
        if self.tick_count < CHECK_INTERVAL_TICKS {
            return Vec::new();
        }
        self.tick_count = 0;

        // Only check when user is idle
        if !ctx.system.user_idle || ctx.system.idle_seconds < MIN_IDLE_SECS {
            return Vec::new();
        }

        // Avoid checking too frequently
        let now = ctx.clock.duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default().as_secs_f64();
        if now - self.last_check_ts < 240.0 {
            return Vec::new();
        }
        self.last_check_ts = now;

        // Capture and analyze in background (non-blocking would be better,
        // but for now we do it synchronously since tick runs on UI thread timer)
        let activity = match quick_screen_check() {
            Some(desc) => desc,
            None => return Vec::new(),
        };

        let mut urges = Vec::new();

        // If activity changed significantly, generate an urge
        if !self.last_activity.is_empty() && activity != self.last_activity {
            // Only notify on interesting transitions
            let idle_mins = ctx.system.idle_seconds / 60;
            if idle_mins > 5 {
                urges.push(Urge {
                    id: format!("screen-watch-{}", now as u64),
                    source: "ScreenWatcher".to_string(),
                    title: "Activity noticed".to_string(),
                    body: format!("You've been idle for {}m. Screen shows: {}", idle_mins, activity),
                    urgency: 0.25,
                    confidence: 0.6,
                    category: UrgeCategory::Vision,
                });
            }
        }

        self.last_activity = activity;
        urges
    }

    fn on_feedback(&mut self, _urge_id: &str, _outcome: Outcome) {}
}

/// Quick screen check — captures screenshot and gets a brief description.
/// Returns None if capture or analysis fails (non-fatal).
fn quick_screen_check() -> Option<String> {
    // Capture screenshot
    let output = std::process::Command::new("grim")
        .args(["-t", "png", "/tmp/yantrik-screen-check.png"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    // For now, just note that we captured — full vision analysis would
    // require the Ollama URL which isn't available in the feature context.
    // The ScreenWatcher detects THAT something is on screen; the vision
    // tools let the user/LLM ask WHAT is on screen.
    //
    // In future: pass ollama_base via FeatureContext or shared config.
    // For now, return a basic description based on grim success.
    Some("screen captured".to_string())
}
