//! Words and dates the store hands to the screen: the clock, "3 days ago", a plain date, and the
//! language a file's name implies. Plain functions, std only.

use std::path::Path;

/// Unix seconds now. Zero if the clock is before the epoch, which is not a reason to fail a save.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The language a file extension implies, or `Other`.
///
/// Used by the command line and by `import`: a file named on the command line arrives with no
/// language, and guessing from the extension is better than filing every one of them as `Other`.
pub fn language_for(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "rs" => "Rust",
        "py" | "pyw" => "Python",
        "js" | "mjs" | "cjs" | "jsx" => "JavaScript",
        "ts" | "tsx" => "TypeScript",
        "go" => "Go",
        "c" => "C",
        "cc" | "cpp" | "cxx" | "h" | "hpp" => "C++",
        "sh" | "bash" | "zsh" => "Shell",
        "sql" => "SQL",
        "html" | "htm" => "HTML",
        "css" | "scss" => "CSS",
        "toml" => "TOML",
        "yaml" | "yml" => "YAML",
        "json" => "JSON",
        _ => "Other",
    }
}

/// "just now", "12 minutes ago", "3 days ago" (yantrik_foundation::time), and a plain date once
/// that stops being useful, after a week.
pub fn relative(then: u64, now_secs: u64) -> String {
    if then == 0 {
        return String::new();
    }
    let delta = now_secs.saturating_sub(then);
    if delta < 7 * 86_400 {
        yantrik_foundation::time::ago_long(std::time::Duration::from_secs(delta))
    } else {
        date(then)
    }
}

/// `YYYY-MM-DD` from unix seconds, without pulling a date library into a module that is
/// otherwise std and serde only.
///
/// The civil-from-days arithmetic is Howard Hinnant's, shifting the epoch to 1 March so the leap
/// day lands at the end of the cycle and no month table is needed.
pub fn date(secs: u64) -> String {
    let days = (secs / 86400) as i64;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}
