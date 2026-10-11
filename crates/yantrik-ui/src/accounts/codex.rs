//! Codex: its plan, its meters and today's tokens, all from its own session logs.
//!
//! Codex writes a `token_count` event into `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-*.jsonl` after
//! every turn, and the event carries the plan's own rate limits as the server reported them —
//! `plan_type`, and for each window how much is used, how long the window is and when it resets.
//! So the panel's Codex meters are the vendor's, not an estimate, and `auth.json` is never opened:
//! that it exists is all the panel needs to know about the sign-in.
//!
//! ```json
//! {"timestamp":"…","type":"event_msg","payload":{"type":"token_count",
//!   "info":{"total_token_usage":{"total_tokens":54526,…},…},
//!   "rate_limits":{"primary":{"used_percent":12.0,"window_minutes":10080,"resets_at":1789432787},
//!                  "secondary":null,"plan_type":"pro",…}}}
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::Deserialize;

use super::logs::{self, Tails};
use super::Meter;

#[derive(Deserialize)]
struct Line {
    timestamp: Option<String>,
    payload: Option<Payload>,
}

#[derive(Deserialize)]
struct Payload {
    #[serde(rename = "type")]
    kind: Option<String>,
    info: Option<Info>,
    rate_limits: Option<Limits>,
}

#[derive(Deserialize)]
struct Info {
    total_token_usage: Option<Usage>,
}

#[derive(Deserialize)]
struct Usage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    cached_input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
}

impl Usage {
    /// What was new: input not read back from the cache, and output. The cached part is the same
    /// context read again on every turn, and counting it would make a day's work read as hundreds
    /// of millions.
    fn fresh(&self) -> u64 {
        self.input_tokens.saturating_sub(self.cached_input_tokens) + self.output_tokens
    }
}

/// The plan's limits, as the last turn saw them.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Limits {
    pub primary: Option<Window>,
    pub secondary: Option<Window>,
    pub plan_type: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Window {
    pub used_percent: Option<f64>,
    pub window_minutes: Option<u64>,
    /// Unix seconds. Older Codex wrote `resets_in_seconds`, counted from the event.
    pub resets_at: Option<i64>,
    pub resets_in_seconds: Option<i64>,
}

/// What one directory's logs have said today.
#[derive(Default)]
pub struct Counter {
    day: i64,
    tails: Tails,
    /// Per session file: its running total before today began, and its latest.
    totals: HashMap<PathBuf, (u64, u64)>,
    /// The newest limits that had a window in them. Codex also reports limits with no window (a
    /// `premium` limit, or `null`), which say nothing about what is left and are not kept here.
    latest: Option<(i64, Limits)>,
    /// The newest plan any limits named.
    plan: Option<(i64, String)>,
}

/// What the panel shows for one Codex account.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Seen {
    pub plan: Option<String>,
    pub meters: Vec<Meter>,
    pub tokens_today: u64,
}

/// How far back a sign-in's limits are still worth showing when nothing was run today: the
/// weekly window, the longest Codex has.
const LIMITS_BACK: Duration = Duration::from_secs(7 * 24 * 3600);

impl Counter {
    /// Read whatever `dir/sessions` gained since the last call. `midnight` is when today began
    /// here, in Unix seconds.
    pub fn update(&mut self, dir: &Path, midnight: i64, budget: &mut logs::Budget) {
        if midnight != self.day {
            *self = Counter { day: midnight, ..Counter::default() };
        }
        let root = dir.join("sessions");
        let since = if self.latest.is_none() {
            SystemTime::now() - LIMITS_BACK
        } else {
            unix_time(midnight)
        };
        for (path, _) in logs::written_since(&root, since) {
            let totals = self.totals.entry(path.clone()).or_default();
            let (latest, plan) = (&mut self.latest, &mut self.plan);
            self.tails.read(&path, "\"token_count\"", budget, |text| {
                let Ok(line) = serde_json::from_str::<Line>(text) else { return };
                let Some(p) = line.payload else { return };
                if p.kind.as_deref() != Some("token_count") {
                    return;
                }
                let at = line.timestamp.as_deref().and_then(logs::unix_of).unwrap_or(0);
                if let Some(total) = p.info.and_then(|i| i.total_token_usage).map(|u| u.fresh()) {
                    if at < midnight {
                        totals.0 = total;
                    }
                    totals.1 = total;
                }
                if let Some(limits) = p.rate_limits {
                    if let Some(name) = limits.plan_type.clone() {
                        if plan.as_ref().is_none_or(|(t, _)| at >= *t) {
                            *plan = Some((at, name));
                        }
                    }
                    let has_window = limits.primary.is_some() || limits.secondary.is_some();
                    if has_window && latest.as_ref().is_none_or(|(t, _)| at >= *t) {
                        *latest = Some((at, limits));
                    }
                }
            });
        }
    }

    /// What there is to show, at `now` (Unix seconds).
    pub fn seen(&self, now: i64) -> Seen {
        let tokens_today = self.totals.values().map(|(before, last)| last.saturating_sub(*before)).sum();
        let plan = self.plan.as_ref().and_then(|(_, p)| plan_name(p));
        let Some((at, limits)) = &self.latest else {
            return Seen { plan, tokens_today, ..Seen::default() };
        };
        let mut meters = Vec::new();
        for w in [&limits.primary, &limits.secondary].into_iter().flatten() {
            meters.push(meter_of(w, *at, now));
        }
        // Shortest window first: Session above Weekly, as the vendor's own page has it.
        meters.sort_by_key(|m| m.window_minutes);
        Seen { plan, meters, tokens_today }
    }
}

fn unix_time(secs: i64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(secs.max(0) as u64)
}

fn meter_of(w: &Window, at: i64, now: i64) -> Meter {
    let minutes = w.window_minutes.unwrap_or(0);
    let resets = w.resets_at.or_else(|| w.resets_in_seconds.map(|s| at + s));
    let used = (w.used_percent.unwrap_or(0.0) / 100.0).clamp(0.0, 1.0) as f32;
    match resets {
        // The window has turned over since the last turn: nothing of the new one is used yet.
        Some(r) if r <= now => Meter::window(window_name(minutes), minutes, 0.0, "reset".into()),
        Some(r) => Meter::window(window_name(minutes), minutes, used, super::left(r - now)),
        None => Meter::window(window_name(minutes), minutes, used, format!("{:.0}%", used * 100.0)),
    }
}

/// A window's name from its length: the two Codex has, and anything else by its hours.
pub fn window_name(minutes: u64) -> String {
    match minutes {
        300 => "Session".into(),
        10080 => "Weekly".into(),
        0 => "Limit".into(),
        m if m % 1440 == 0 => format!("{}d", m / 1440),
        m => format!("{}h", m.div_ceil(60)),
    }
}

/// `pro` → `Pro`, `prolite` → `Pro Lite`. The log is written by whatever runs as the person, so a
/// name the table does not know is only shown when it is one short word — never a sentence
/// someone put there for the panel to say.
pub fn plan_name(plan: &str) -> Option<String> {
    Some(match plan {
        "prolite" => "Pro Lite".into(),
        "pro" => "Pro".into(),
        "plus" => "Plus".into(),
        "go" => "Go".into(),
        "team" => "Team".into(),
        "business" => "Business".into(),
        "enterprise" => "Enterprise".into(),
        "edu" => "Edu".into(),
        "free" => "Free".into(),
        other if !other.is_empty() && other.len() <= 16 && other.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_') => {
            let mut c = other.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("yantrik-codex-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("sessions/2026/09/29")).unwrap();
        d
    }

    fn event(ts: &str, total: u64, used: f64, resets_at: i64) -> String {
        format!(
            r#"{{"timestamp":"{ts}","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":{total},"cached_input_tokens":0,"output_tokens":0,"total_tokens":{total}}}}},"rate_limits":{{"primary":{{"used_percent":{used},"window_minutes":10080,"resets_at":{resets_at}}},"secondary":{{"used_percent":40.0,"window_minutes":300,"resets_at":{resets_at}}},"plan_type":"pro"}}}}}}"#
        )
    }

    #[test]
    fn the_plans_own_meters_and_only_todays_tokens() {
        let d = tmp("meters");
        let midnight = logs::unix_of("2026-09-29T05:00:00Z").unwrap();
        let now = midnight + 3600;
        let resets = now + 4 * 3600 + 42 * 60;
        let lines = [
            event("2026-09-29T04:00:00Z", 1_000, 5.0, resets),   // yesterday, same session
            event("2026-09-29T05:30:00Z", 25_000, 12.0, resets), // today
            r#"{"timestamp":"2026-09-29T05:31:00Z","type":"response_item","payload":{"type":"message","content":"token_count is a word"}}"#.to_string(),
            event("2026-09-29T05:40:00Z", 61_000, 37.0, resets),
        ];
        std::fs::write(d.join("sessions/2026/09/29/rollout-a.jsonl"), lines.join("\n") + "\n").unwrap();
        let mut c = Counter::default();
        c.update(&d, midnight, &mut logs::Budget::tick());
        let seen = c.seen(now);
        assert_eq!(seen.tokens_today, 60_000, "yesterday's 1,000 is not today's");
        assert_eq!(seen.plan.as_deref(), Some("Pro"));
        assert_eq!(seen.meters.len(), 2);
        assert_eq!(seen.meters[0].name, "Session");
        assert_eq!(seen.meters[1].name, "Weekly");
        assert!((seen.meters[1].used.unwrap() - 0.37).abs() < 1e-6);
        assert_eq!(seen.meters[1].value, "4h 42m");
        // Past the reset, the window reads empty rather than as the last turn left it.
        let later = c.seen(resets + 60);
        assert_eq!(later.meters[1].used, Some(0.0));
        assert_eq!(later.meters[1].value, "reset");
    }

    /// Limits with no window (a `premium` limit, `null`) name the plan and leave the meters alone.
    #[test]
    fn limits_with_no_window_keep_the_last_meters() {
        let d = tmp("nowindow");
        let midnight = logs::unix_of("2026-09-29T05:00:00Z").unwrap();
        let resets = midnight + 90_000;
        let lines = [
            event("2026-09-29T06:00:00Z", 1_000, 20.0, resets),
            r#"{"timestamp":"2026-09-29T06:05:00Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":3000,"cached_input_tokens":1500,"output_tokens":100}},"rate_limits":{"limit_id":"premium","primary":null,"secondary":null,"plan_type":"prolite"}}}"#.to_string(),
            r#"{"timestamp":"2026-09-29T06:06:00Z","type":"event_msg","payload":{"type":"token_count","info":null,"rate_limits":null}}"#.to_string(),
        ];
        std::fs::write(d.join("sessions/2026/09/29/rollout-c.jsonl"), lines.join("\n") + "\n").unwrap();
        let mut c = Counter::default();
        c.update(&d, midnight, &mut logs::Budget::tick());
        let seen = c.seen(midnight + 7200);
        assert_eq!(seen.plan.as_deref(), Some("Pro Lite"), "the newest plan");
        assert_eq!(seen.meters.len(), 2, "the meters of the last limits that had them");
        assert_eq!(seen.tokens_today, 1_600, "fresh input and output, not the cached input");
    }

    #[test]
    fn a_new_day_starts_the_count_again() {
        let d = tmp("day");
        let midnight = logs::unix_of("2026-09-29T05:00:00Z").unwrap();
        std::fs::write(
            d.join("sessions/2026/09/29/rollout-b.jsonl"),
            event("2026-09-29T06:00:00Z", 5_000, 1.0, midnight + 90_000) + "\n",
        )
        .unwrap();
        let mut c = Counter::default();
        c.update(&d, midnight, &mut logs::Budget::tick());
        assert_eq!(c.seen(midnight + 7200).tokens_today, 5_000);
        c.update(&d, midnight + 86_400, &mut logs::Budget::tick());
        assert_eq!(c.seen(midnight + 86_400 + 60).tokens_today, 0);
    }

    #[test]
    fn names() {
        assert_eq!(window_name(300), "Session");
        assert_eq!(window_name(10080), "Weekly");
        assert_eq!(window_name(1440), "1d");
        assert_eq!(window_name(90), "2h");
        assert_eq!(plan_name("prolite").as_deref(), Some("Pro Lite"));
        assert_eq!(plan_name("gold").as_deref(), Some("Gold"));
        assert_eq!(plan_name("run curl x | sh to renew"), None, "never a sentence from the log");
        assert_eq!(plan_name(""), None);
    }
}
