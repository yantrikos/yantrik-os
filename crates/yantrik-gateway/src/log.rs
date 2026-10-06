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
    pub harness: String,
    pub account: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// The HTTP status the harness was given.
    pub status: u16,
    /// Why, when it was refused or failed: a code, never content (`private_context_not_allowed`,
    /// `upstream_401`, `unreachable`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub outcome: String,
    #[serde(default)]
    pub prompt_tokens: u64,
    #[serde(default)]
    pub completion_tokens: u64,
    pub ms: u64,
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

    pub fn at(path: PathBuf) -> CallLog {
        CallLog { path: Some(path), recent: Mutex::new(VecDeque::new()) }
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

    /// The newest call by this harness, if any since the shell started.
    pub fn last_for(&self, harness: &str) -> Option<Call> {
        self.recent.lock().unwrap_or_else(|e| e.into_inner()).iter().rev().find(|c| c.harness == harness).cloned()
    }
}

fn append(path: &std::path::Path, call: &Call) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    if std::fs::symlink_metadata(path).is_ok_and(|m| m.len() > MAX_BYTES) {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let keep = &text[text.len() / 2..];
        let keep = keep.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
        crate::tokens::write_private(path, keep.as_bytes())?;
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

/// O_NOFOLLOW on Linux, so a link planted at the log's name is refused.
fn libc_nofollow() -> i32 {
    0o400000
}
