//! Claude: which plan an account is on, and today's tokens.
//!
//! **The sign-in is never opened.** Anthropic does not allow another program to collect or
//! intermediate a Claude.ai token, so the sign-in file is only looked for (`vendors`), and the
//! panel draws no meter Claude Code does not hand it.
//!
//! **The plan** is in Claude Code's settings file, `.claude.json`, which holds the account's
//! profile and no secret: `oauthAccount.organizationType` (`claude_max`) and
//! `organizationRateLimitTier` (`default_claude_max_20x`). It is read into a struct with only
//! those two fields. For the first account the file is `~/.claude.json`, beside the directory;
//! with `CLAUDE_CONFIG_DIR` it is inside the directory.
//!
//! **Today's tokens** are counted from Claude Code's own transcripts, `projects/<dir>/*.jsonl`: each
//! assistant message carries its `usage`. A message can be written twice (a resumed session copies
//! its history), so each is counted once by its message and request ids, as ccusage does. What is
//! counted is what was new — input, output and cache writes — not the cache reads, which are the
//! same context read again on every turn.

use std::collections::HashSet;
use std::path::Path;
use std::time::{Duration, SystemTime};

use serde::Deserialize;

use super::logs::{self, Tails};

#[derive(Deserialize)]
struct Settings {
    #[serde(rename = "oauthAccount")]
    account: Option<PlanFields>,
}

#[derive(Deserialize)]
struct PlanFields {
    #[serde(rename = "organizationType")]
    organization: Option<String>,
    #[serde(rename = "organizationRateLimitTier")]
    tier: Option<String>,
}

/// The largest settings file that is read at all: it keeps per-project history, and grows.
const SETTINGS_MOST: u64 = 32 * 1024 * 1024;

/// Where the settings file of the account in `dir` is: beside the vendor's own directory for the
/// first account, inside the directory for the others.
pub fn settings_path(dir: &Path, primary: bool) -> std::path::PathBuf {
    if primary {
        dir.with_file_name(".claude.json")
    } else {
        dir.join(".claude.json")
    }
}

/// What `.claude.json` said last time, keyed by the file's length and time: it is up to tens of
/// megabytes, and read again only when it changed.
#[derive(Default)]
pub struct PlanCache {
    seen: std::collections::HashMap<std::path::PathBuf, (u64, Option<std::time::SystemTime>, Option<String>)>,
}

/// The plan the account in `dir` is on — `Max 20x`, `Pro` — or `None` when it cannot be told.
/// Opened as the logs are (`logs::open_own`): never through a link or a second name.
pub fn plan(dir: &Path, primary: bool, cache: &mut PlanCache) -> Option<String> {
    use std::io::Read;
    let path = settings_path(dir, primary);
    let (f, len) = logs::open_own(&path)?;
    if len > SETTINGS_MOST {
        return None;
    }
    let modified = f.metadata().ok().and_then(|m| m.modified().ok());
    if let Some((l, m, plan)) = cache.seen.get(&path) {
        if *l == len && *m == modified {
            return plan.clone();
        }
    }
    let reader = std::io::BufReader::new(f.take(SETTINGS_MOST));
    let plan = serde_json::from_reader::<_, Settings>(reader)
        .ok()
        .and_then(|s| s.account)
        .and_then(|a| plan_name(a.organization.as_deref().map(|o| o.strip_prefix("claude_").unwrap_or(o)), a.tier.as_deref()));
    cache.seen.insert(path, (len, modified, plan.clone()));
    plan
}

/// `("max", "default_claude_max_20x")` → `Max 20x`.
pub fn plan_name(subscription: Option<&str>, tier: Option<&str>) -> Option<String> {
    let multiple = tier.and_then(|t| {
        let n = t.rsplit('_').next()?;
        let digits = n.strip_suffix('x')?;
        (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())).then(|| n.to_string())
    });
    let base = match subscription? {
        "max" => "Max",
        "pro" => "Pro",
        "team" => "Team",
        "enterprise" => "Enterprise",
        "free" => "Free",
        _ => return None,
    };
    Some(match multiple {
        Some(m) if base == "Max" => format!("{base} {m}"),
        _ => base.to_string(),
    })
}

#[derive(Deserialize)]
struct Line {
    #[serde(rename = "type")]
    kind: Option<String>,
    timestamp: Option<String>,
    #[serde(rename = "requestId")]
    request_id: Option<String>,
    message: Option<Message>,
}

#[derive(Deserialize)]
struct Message {
    id: Option<String>,
    usage: Option<Usage>,
}

#[derive(Deserialize, Default)]
struct Usage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,
}

/// Today's tokens for one Claude directory.
#[derive(Default)]
pub struct Counter {
    day: i64,
    tails: Tails,
    seen: HashSet<String>,
    tokens: u64,
    /// The count stopped at `MOST_MESSAGES` distinct messages: a day's real work is a few
    /// thousand, and a set that grew without end would be the shell's memory.
    pub full: bool,
}

/// The most distinct messages counted in a day.
pub const MOST_MESSAGES: usize = 200_000;

impl Counter {
    pub fn update(&mut self, dir: &Path, midnight: i64, budget: &mut logs::Budget) {
        if midnight != self.day {
            *self = Counter { day: midnight, ..Counter::default() };
        }
        let since = SystemTime::UNIX_EPOCH + Duration::from_secs(midnight.max(0) as u64);
        let root = dir.join("projects");
        let (seen, tokens, full) = (&mut self.seen, &mut self.tokens, &mut self.full);
        for (path, _) in logs::written_since(&root, since) {
            self.tails.read(&path, "\"usage\"", budget, |text| {
                if *full {
                    return;
                }
                let Ok(line) = serde_json::from_str::<Line>(text) else { return };
                if line.kind.as_deref() != Some("assistant") {
                    return;
                }
                if line.timestamp.as_deref().and_then(logs::unix_of).is_none_or(|t| t < midnight) {
                    return;
                }
                let Some(m) = line.message else { return };
                let Some(u) = m.usage else { return };
                if let Some(id) = m.id {
                    if seen.len() >= MOST_MESSAGES {
                        *full = true;
                        return;
                    }
                    if !seen.insert(format!("{id}:{}", line.request_id.unwrap_or_default())) {
                        return;
                    }
                }
                // What was new that turn. A cache read is the same context read again, every
                // turn; counted, a day of work reads as hundreds of millions (found on a real day:
                // 457M with the reads, which are nearly all of it).
                *tokens += u.input_tokens + u.output_tokens + u.cache_creation_input_tokens;
            });
        }
    }

    pub fn tokens_today(&self) -> u64 {
        self.tokens
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("yantrik-claude-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("projects/-home-p-code")).unwrap();
        d
    }

    /// Nothing in this file reads the sign-in.
    #[test]
    fn the_sign_in_file_is_never_named_here() {
        let src = include_str!("claude.rs");
        let name = [".credentials", ".json"].concat();
        assert!(!src.contains(&name));
    }

    #[test]
    fn plan_names() {
        assert_eq!(plan_name(Some("max"), Some("default_claude_max_20x")).as_deref(), Some("Max 20x"));
        assert_eq!(plan_name(Some("max"), Some("default_claude_max_5x")).as_deref(), Some("Max 5x"));
        assert_eq!(plan_name(Some("pro"), Some("default_claude_ai")).as_deref(), Some("Pro"));
        assert_eq!(plan_name(Some("max"), None).as_deref(), Some("Max"));
        assert_eq!(plan_name(None, Some("default_claude_max_20x")), None);
        assert_eq!(plan_name(Some("<script>"), None), None);
    }

    #[test]
    fn the_plan_comes_from_the_settings_file_beside_the_first_account_and_inside_the_others() {
        let home = tmp("plan");
        let dir = home.join(".claude");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            home.join(".claude.json"),
            r#"{"numStartups":3,"oauthAccount":{"emailAddress":"p@example.com","organizationType":"claude_max","organizationRateLimitTier":"default_claude_max_20x"},"projects":{}}"#,
        )
        .unwrap();
        let mut cache = PlanCache::default();
        assert_eq!(plan(&dir, true, &mut cache).as_deref(), Some("Max 20x"));
        assert_eq!(plan(&dir, false, &mut cache), None, "an extra account's file is inside its own directory");
        std::fs::write(dir.join(".claude.json"), r#"{"oauthAccount":{"organizationType":"claude_pro"}}"#).unwrap();
        assert_eq!(plan(&dir, false, &mut cache).as_deref(), Some("Pro"));
        std::fs::write(dir.join(".claude.json"), r#"{"oauthAccount":{"organizationType":"claude_team"}}"#).unwrap();
        assert_eq!(plan(&dir, false, &mut cache).as_deref(), Some("Team"), "a changed file is read again");
    }

    #[test]
    fn todays_tokens_are_counted_once_each() {
        let d = tmp("count");
        let midnight = logs::unix_of("2026-09-29T05:00:00Z").unwrap();
        let msg = |ts: &str, id: &str, req: &str| {
            format!(
                r#"{{"type":"assistant","timestamp":"{ts}","requestId":"{req}","message":{{"id":"{id}","usage":{{"input_tokens":2,"output_tokens":300,"cache_creation_input_tokens":1000,"cache_read_input_tokens":20000}}}}}}"#
            )
        };
        let lines = [
            msg("2026-09-29T04:59:00Z", "msg_old", "req_0"), // yesterday
            msg("2026-09-29T06:00:00Z", "msg_a", "req_1"),
            msg("2026-09-29T06:00:00Z", "msg_a", "req_1"), // the same message again
            r#"{"type":"user","timestamp":"2026-09-29T06:01:00Z","message":{"content":"what is my usage"}}"#.to_string(),
            msg("2026-09-29T06:02:00Z", "msg_b", "req_2"),
        ];
        std::fs::write(d.join("projects/-home-p-code/s.jsonl"), lines.join("\n") + "\n").unwrap();
        // A resumed session repeats msg_b in another file.
        std::fs::write(d.join("projects/-home-p-code/t.jsonl"), msg("2026-09-29T06:02:00Z", "msg_b", "req_2") + "\n").unwrap();
        let mut c = Counter::default();
        c.update(&d, midnight, &mut logs::Budget::tick());
        assert_eq!(c.tokens_today(), 2 * 1_302, "cache reads are not new tokens");
        c.update(&d, midnight, &mut logs::Budget::tick());
        assert_eq!(c.tokens_today(), 2 * 1_302, "a second read adds nothing");
    }
}
