//! What a provider's response says about what is left: its rate-limit headers, in whichever of
//! the shapes it uses (Groq and Scaleway: `x-ratelimit-*`; OVHcloud: the IETF draft's
//! `ratelimit-*`; Mistral: `x-ratelimit-remaining-req-minute`; nearly all: `retry-after`).

/// What the headers said. `None` is "not said", never zero.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Observed {
    pub remaining_requests: Option<u64>,
    pub remaining_tokens: Option<u64>,
    /// Seconds until the request window is full again.
    pub reset_requests: Option<f64>,
    pub reset_tokens: Option<f64>,
    /// Seconds to wait before asking again (on a refusal).
    pub retry_after: Option<f64>,
}

/// Read the headers of one response. `get` looks a header up by name, case-insensitively.
pub fn observe(get: impl Fn(&str) -> Option<String>) -> Observed {
    let num = |names: &[&str]| names.iter().find_map(|n| get(n)).and_then(|v| v.trim().parse::<f64>().ok()).map(|f| f.max(0.0) as u64);
    let dur = |names: &[&str]| names.iter().find_map(|n| get(n)).and_then(|v| duration(&v));
    Observed {
        remaining_requests: num(&[
            "x-ratelimit-remaining-requests",
            "x-ratelimit-remaining-req-minute",
            "x-ratelimit-remaining-minute",
            "ratelimit-remaining",
        ]),
        remaining_tokens: num(&["x-ratelimit-remaining-tokens"]),
        reset_requests: dur(&["x-ratelimit-reset-requests", "ratelimit-reset"]),
        reset_tokens: dur(&["x-ratelimit-reset-tokens"]),
        retry_after: get("retry-after").and_then(|v| duration(&v)),
    }
}

/// A duration as providers write it: "7", "1.5", "6s", "120ms", "2m59.56s", "1h2m3s". `None`
/// for anything else (an HTTP date in `retry-after` is not worth a calendar: the caller backs off).
pub fn duration(text: &str) -> Option<f64> {
    duration_unbounded(text).filter(|s| s.is_finite() && *s >= 0.0).map(|s| s.min(MAX_WAIT_SECS))
}

/// The longest wait a provider's header can impose: a day. Longer, or `inf`, or `1e308`, is a
/// broken or hostile header, and believing it would rest a model for years or overflow the clock
/// into no wait at all (security review of #535, 1 Oct 2026).
pub const MAX_WAIT_SECS: f64 = 86_400.0;

fn duration_unbounded(text: &str) -> Option<f64> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    if let Ok(secs) = t.parse::<f64>() {
        return (secs >= 0.0).then_some(secs);
    }
    let mut total = 0.0;
    let mut number = String::new();
    let mut chars = t.chars().peekable();
    let mut any = false;
    while let Some(c) = chars.next() {
        if c.is_ascii_digit() || c == '.' {
            number.push(c);
            continue;
        }
        let value: f64 = number.parse().ok()?;
        number.clear();
        let unit = match (c, chars.peek()) {
            ('m', Some('s')) => {
                chars.next();
                0.001
            }
            ('h', _) => 3600.0,
            ('m', _) => 60.0,
            ('s', _) => 1.0,
            _ => return None,
        };
        total += value * unit;
        any = true;
    }
    if !number.is_empty() {
        return None; // a number with no unit after other parts: not a duration we know
    }
    any.then_some(total)
}
