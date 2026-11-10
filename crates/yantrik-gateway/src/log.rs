//! Every call, without its content: which harness, which account and model, the effort, how it
//! ended, the tokens and the time. For the honest status (`connected · <model>` only after a real
//! success) and the audit view. JSON lines at `~/.local/state/yantrik/gateway-calls.jsonl`, mode
//! 600, cut to the newest half when it passes [`MAX_BYTES`]; the last few hundred also in memory.

use std::collections::VecDeque;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

pub const MAX_BYTES: u64 = 2 * 1024 * 1024;
const KEEP_IN_MEMORY: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Call {
    /// Seconds since the epoch.
    pub at: i64,
    /// The call's own key, set by the gateway ([`new_rid`]) and never reused: what a turn is tied to
    /// when it ends, so a past answer names the call that answered it and no other. 0 on a line
    /// written before there was one.
    #[serde(default)]
    pub rid: u64,
    pub harness: String,
    pub account: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// The HTTP status the harness was given.
    pub status: u16,
    /// Why, when it was refused or failed: a code, never content (`private_context_not_shared`,
    /// `upstream_401`, `unreachable`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub outcome: String,
    #[serde(default)]
    pub prompt_tokens: u64,
    #[serde(default)]
    pub completion_tokens: u64,
    pub ms: u64,
    /// When the private lane answered (`account`/`model` are then the lane's): the
    /// `<account>/<model>` the call named, which was kept from private context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lane_for: Option<String>,
    /// Parameters the gateway took out of the call's body because the model refuses them
    /// (`temperature`): their names, never their values.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dropped: Vec<String>,
}

/// A key no other call has had, across restarts: the clock in nanoseconds, or one more than the last
/// key handed out when two calls land in the same tick.
pub fn new_rid() -> u64 {
    static LAST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
    let mut last = LAST.load(std::sync::atomic::Ordering::SeqCst);
    loop {
        let next = now.max(last + 1);
        match LAST.compare_exchange(last, next, std::sync::atomic::Ordering::SeqCst, std::sync::atomic::Ordering::SeqCst) {
            Ok(_) => return next,
            Err(seen) => last = seen,
        }
    }
}

impl Call {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

pub struct CallLog {
    path: Option<PathBuf>,
    recent: Mutex<VecDeque<Call>>,
}

impl CallLog {
    pub fn in_memory() -> CallLog {
        CallLog { path: None, recent: Mutex::new(VecDeque::new()) }
    }

    /// The log at `path`, with its newest calls read back in: what was answered before the shell
    /// started is still what answered.
    pub fn at(path: PathBuf) -> CallLog {
        let recent = std::fs::read_to_string(&path).map(|text| newest_calls(&text)).unwrap_or_default();
        CallLog { path: Some(path), recent: Mutex::new(recent) }
    }

    pub fn record(&self, call: Call) {
        tracing::info!(
            harness = %call.harness, account = %call.account, model = %call.model, status = call.status,
            outcome = %call.outcome, prompt_tokens = call.prompt_tokens, completion_tokens = call.completion_tokens,
            ms = call.ms, "gateway call"
        );
        if let Some(path) = &self.path {
            if let Err(e) = append(path, &call) {
                tracing::warn!(error = %e, "the gateway's call log was not written");
            }
        }
        let mut r = self.recent.lock().unwrap_or_else(|e| e.into_inner());
        r.push_back(call);
        while r.len() > KEEP_IN_MEMORY {
            r.pop_front();
        }
    }

    /// The newest calls, oldest first.
    pub fn recent(&self) -> Vec<Call> {
        self.recent.lock().unwrap_or_else(|e| e.into_inner()).iter().cloned().collect()
    }

    /// The newest call by this harness.
    pub fn last_for(&self, harness: &str) -> Option<Call> {
        self.recent.lock().unwrap_or_else(|e| e.into_inner()).iter().rev().find(|c| c.harness == harness).cloned()
    }

    /// The newest call by this harness that was answered: the one that shows which model really
    /// answered it.
    pub fn last_ok_for(&self, harness: &str) -> Option<Call> {
        self.recent.lock().unwrap_or_else(|e| e.into_inner()).iter().rev().find(|c| c.harness == harness && c.ok()).cloned()
    }
}

/// The last [`KEEP_IN_MEMORY`] calls a log's text holds, oldest first; a line that is not a call is skipped.
fn newest_calls(text: &str) -> VecDeque<Call> {
    let mut calls: VecDeque<Call> = text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
    while calls.len() > KEEP_IN_MEMORY {
        calls.pop_front();
    }
    calls
}

fn append(path: &std::path::Path, call: &Call) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    if std::fs::symlink_metadata(path).is_ok_and(|m| m.len() > MAX_BYTES) {
        crate::tokens::write_private(path, newest_half(&std::fs::read(path).unwrap_or_default()))?;
    }
    let line = serde_json::to_string(call).map_err(|e| e.to_string())? + "\n";
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .custom_flags(libc_nofollow())
        .open(path)
        .map_err(|e| e.to_string())?;
    f.write_all(line.as_bytes()).map_err(|e| e.to_string())
}

/// The newest half of the log, from the first whole line on. Cut in bytes at a newline, never in
/// the middle of a character: a model id a harness sent can be any text, and slicing a `str` there
/// panicked the connection's thread on every call once the log was full.
fn newest_half(bytes: &[u8]) -> &[u8] {
    let half = &bytes[bytes.len() / 2..];
    half.iter().position(|b| *b == b'\n').map_or(&[][..], |i| &half[i + 1..])
}

/// O_NOFOLLOW, so a link planted at the log's name is refused. From libc: it is not the same
/// number on every architecture (0o400000 on x86_64 is O_LARGEFILE on aarch64).
fn libc_nofollow() -> i32 {
    libc::O_NOFOLLOW
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(model: &str) -> Call {
        Call { at: 1, rid: 0, harness: "pi".into(), account: "a".into(), model: model.into(), effort: None, status: 200, outcome: String::new(), prompt_tokens: 0, completion_tokens: 0, ms: 1, lane_for: None, dropped: Vec::new() }
    }

    /// #1162 M1: each call has a key of its own, which survives the log's round trip.
    #[test]
    fn every_call_has_a_key_no_other_call_has() {
        let keys: std::collections::BTreeSet<u64> = (0..1000).map(|_| new_rid()).collect();
        assert_eq!(keys.len(), 1000);
        assert!(!keys.contains(&0));
        let c = Call { rid: new_rid(), ..call("m") };
        assert_eq!(serde_json::from_str::<Call>(&serde_json::to_string(&c).unwrap()).unwrap().rid, c.rid);
        let old = r#"{"at":1,"harness":"pi","account":"a","model":"m","status":200,"ms":1}"#;
        assert_eq!(serde_json::from_str::<Call>(old).unwrap().rid, 0, "a line from before keys is unkeyed");
    }

    #[test]
    fn the_newest_answered_call_survives_a_restart_and_skips_failures() {
        let dir = std::env::temp_dir().join(format!("gateway-log-restart-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("calls.jsonl");
        let failed = Call { status: 502, outcome: "unreachable".into(), ..call("newest-but-failed") };
        let lines = [call("answered"), failed].iter().map(|c| serde_json::to_string(c).unwrap() + "
").collect::<String>() + "not json
";
        std::fs::write(&path, lines).unwrap();
        let log = CallLog::at(path);
        assert_eq!(log.last_for("pi").unwrap().model, "newest-but-failed");
        assert_eq!(log.last_ok_for("pi").unwrap().model, "answered");
        assert!(log.last_ok_for("hermes").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_full_log_is_cut_at_a_line_whatever_characters_it_holds() {
        let dir = std::env::temp_dir().join(format!("gateway-log-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("calls.jsonl");
        // Three-byte characters throughout, so the middle of the file is inside one.
        let line = serde_json::to_string(&call(&"模".repeat(200))).unwrap() + "\n";
        let mut text = String::new();
        while (text.len() as u64) <= MAX_BYTES {
            text.push_str(&line);
        }
        while text.is_char_boundary(text.len() / 2) {
            text.insert(0, 'x');
        }
        std::fs::write(&path, &text).unwrap();
        append(&path, &call("after")).unwrap();
        let kept = std::fs::read_to_string(&path).unwrap();
        assert!((kept.len() as u64) < MAX_BYTES && kept.ends_with("\n"));
        for l in kept.lines() {
            serde_json::from_str::<Call>(l).unwrap();
        }
        assert!(kept.lines().last().unwrap().contains("after"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_link_at_the_logs_name_is_not_followed() {
        let dir = std::env::temp_dir().join(format!("gateway-log-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let elsewhere = dir.join("elsewhere");
        std::fs::write(&elsewhere, "").unwrap();
        std::os::unix::fs::symlink(&elsewhere, dir.join("calls.jsonl")).unwrap();
        assert!(append(&dir.join("calls.jsonl"), &call("m")).is_err());
        assert_eq!(std::fs::read_to_string(&elsewhere).unwrap(), "");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
