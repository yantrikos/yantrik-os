//! Minimal 5-field cron expression parser.
//!
//! Supports: `*`, `N`, `N-M`, `*/N`, comma-separated lists.
//! Fields: minute(0-59) hour(0-23) day(1-31) month(1-12) weekday(0-6, 0=Sun).

use std::collections::HashSet;

/// Compute the next fire time after `after_ts` (unix timestamp) for a 5-field cron expression.
///
/// Returns `None` if the expression is invalid or no match found within 366 days.
pub fn next_cron(expr: &str, after_ts: f64) -> Option<f64> {
    let fields: Vec<&str> = expr.split_whitespace().collect();
    if fields.len() != 5 {
        return None;
    }

    let minutes = parse_field(fields[0], 0, 59)?;
    let hours = parse_field(fields[1], 0, 23)?;
    let days = parse_field(fields[2], 1, 31)?;
    let months = parse_field(fields[3], 1, 12)?;
    let weekdays = parse_field(fields[4], 0, 6)?;

    // Start one minute after after_ts
    let start_secs = after_ts as i64 + 60;
    // Round to start of next minute
    let start_min = start_secs - (start_secs % 60);

    // Search up to 366 days * 24 hours * 60 minutes = 527040 iterations
    let max_minutes = 366 * 24 * 60;

    for i in 0..max_minutes {
        let ts = start_min + i * 60;

        // Decompose unix timestamp to UTC time components
        let (minute, hour, day, month, weekday) = decompose_ts(ts);

        if minutes.contains(&minute)
            && hours.contains(&hour)
            && days.contains(&day)
            && months.contains(&month)
            && weekdays.contains(&weekday)
        {
            return Some(ts as f64);
        }
    }

    None
}

/// Parse a single cron field into a set of valid values.
fn parse_field(field: &str, min: u32, max: u32) -> Option<HashSet<u32>> {
    let mut result = HashSet::new();

    for part in field.split(',') {
        let part = part.trim();
        if part == "*" {
            // Every value
            for v in min..=max {
                result.insert(v);
            }
        } else if let Some(step) = part.strip_prefix("*/") {
            // Step: */N
            let n: u32 = step.parse().ok()?;
            if n == 0 {
                return None;
            }
            let mut v = min;
            while v <= max {
                result.insert(v);
                v += n;
            }
        } else if part.contains('-') {
            // Range: N-M
            let mut parts = part.splitn(2, '-');
            let from: u32 = parts.next()?.parse().ok()?;
            let to: u32 = parts.next()?.parse().ok()?;
            if from > max || to > max || from > to {
                return None;
            }
            for v in from..=to {
                result.insert(v);
            }
        } else {
            // Single value
            let v: u32 = part.parse().ok()?;
            if v < min || v > max {
                return None;
            }
            result.insert(v);
        }
    }

    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

/// Decompose a unix timestamp into (minute, hour, day, month, weekday).
/// weekday: 0=Sunday, 1=Monday, ..., 6=Saturday.
fn decompose_ts(ts: i64) -> (u32, u32, u32, u32, u32) {
    // seconds since epoch
    let minute = ((ts / 60) % 60) as u32;
    let hour = ((ts / 3600) % 24) as u32;

    // Days since epoch (Jan 1, 1970 = Thursday = weekday 4)
    let days_since_epoch = ts.div_euclid(86400);
    let weekday = ((days_since_epoch + 4) % 7) as u32; // 0=Sun

    // Compute year/month/day from days since epoch
    let (year, month, day) = days_to_ymd(days_since_epoch);
    let _ = year; // unused but computed

    (minute, hour, day as u32, month as u32, weekday)
}

/// Convert days since Unix epoch to (year, month, day).
fn days_to_ymd(days: i64) -> (i64, i64, i64) {
    // Algorithm from http://howardhinnant.github.io/date_algorithms.html
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_every_minute() {
        // "* * * * *" — should fire on the next minute
        let now = 1709510400.0; // some timestamp
        let next = next_cron("* * * * *", now).unwrap();
        assert!(next > now);
        assert!(next - now <= 120.0); // within 2 minutes
    }

    #[test]
    fn test_specific_minute() {
        // "30 * * * *" — minute 30 of every hour
        let now = 1709510400.0; // 2024-03-04 00:00:00 UTC
        let next = next_cron("30 * * * *", now).unwrap();
        let (min, _, _, _, _) = decompose_ts(next as i64);
        assert_eq!(min, 30);
    }

    #[test]
    fn test_daily_9am() {
        // "0 9 * * *" — 9:00 AM UTC every day
        let now = 1709510400.0;
        let next = next_cron("0 9 * * *", now).unwrap();
        let (min, hour, _, _, _) = decompose_ts(next as i64);
        assert_eq!(min, 0);
        assert_eq!(hour, 9);
    }

    #[test]
    fn test_invalid() {
        assert!(next_cron("bad", 0.0).is_none());
        assert!(next_cron("60 * * * *", 0.0).is_none()); // minute > 59
        assert!(next_cron("* 25 * * *", 0.0).is_none()); // hour > 23
    }

    #[test]
    fn test_step() {
        // "*/15 * * * *" — every 15 minutes
        let now = 1709510400.0;
        let next = next_cron("*/15 * * * *", now).unwrap();
        let (min, _, _, _, _) = decompose_ts(next as i64);
        assert!(min % 15 == 0);
    }

    #[test]
    fn test_range() {
        // "0 9-17 * * *" — every hour 9am-5pm
        let now = 1709510400.0;
        let next = next_cron("0 9-17 * * *", now).unwrap();
        let (min, hour, _, _, _) = decompose_ts(next as i64);
        assert_eq!(min, 0);
        assert!(hour >= 9 && hour <= 17);
    }
}
