//! One token per harness: minted by the OS, written into that harness's own config, and kept here
//! as its SHA-256 only, so the gateway's own file cannot be used to call it.
//!
//! `ygw-` and 256 random bits as hex. A token names a harness and whether that harness sends the
//! person's private context (the Mind does; a coding agent pointed at a repository may not): the
//! one fact [`crate::policy`] needs. Like the agent token (docs/harness.md), it stops confusion
//! and casual impersonation, not a hostile program running as the person, which can read the
//! harness's file as well as the harness can.

use std::collections::BTreeMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const PREFIX: &str = "ygw-";

/// Who a valid token belongs to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    pub harness: String,
    /// The harness sends the person's private context with its requests.
    pub private_context: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct File {
    /// SHA-256 of the token, as hex → whose it is.
    #[serde(default)]
    tokens: BTreeMap<String, Grant>,
}

/// The tokens the gateway accepts.
pub struct Tokens {
    path: Option<PathBuf>,
    inner: RwLock<File>,
}

fn digest(token: &str) -> String {
    Sha256::digest(token.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// A new token: `ygw-` and 64 hex digits from the kernel's random source.
pub fn mint() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(|e| format!("no random source for a gateway token: {e}"))?;
    Ok(format!("{PREFIX}{}", bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()))
}

impl Tokens {
    /// Tokens kept in memory only (tests).
    pub fn in_memory() -> Tokens {
        Tokens { path: None, inner: RwLock::new(File::default()) }
    }

    /// Tokens kept at `path` (`~/.config/yantrik/gateway/tokens.json`, mode 600): hashes only.
    pub fn open(path: &Path) -> Tokens {
        let file = std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        Tokens { path: Some(path.to_path_buf()), inner: RwLock::new(file) }
    }

    /// Accept `token` for `grant.harness`, replacing any token that harness had: one per harness.
    pub fn register(&self, token: &str, grant: Grant) -> Result<(), String> {
        if !token.starts_with(PREFIX) || token.len() != PREFIX.len() + 64 {
            return Err("not a gateway token".into());
        }
        let mut f = self.inner.write().unwrap_or_else(|e| e.into_inner());
        f.tokens.retain(|_, g| g.harness != grant.harness);
        f.tokens.insert(digest(token), grant);
        self.save(&f)
    }

    /// Stop accepting the harness's token (Revert).
    pub fn revoke(&self, harness: &str) -> Result<bool, String> {
        let mut f = self.inner.write().unwrap_or_else(|e| e.into_inner());
        let before = f.tokens.len();
        f.tokens.retain(|_, g| g.harness != harness);
        let gone = f.tokens.len() != before;
        self.save(&f)?;
        Ok(gone)
    }

    /// Whose token this is, if it is one.
    pub fn verify(&self, token: &str) -> Option<Grant> {
        if !token.starts_with(PREFIX) {
            return None;
        }
        self.inner.read().unwrap_or_else(|e| e.into_inner()).tokens.get(&digest(token)).cloned()
    }

    /// The harnesses that hold a token.
    pub fn harnesses(&self) -> Vec<String> {
        self.inner.read().unwrap_or_else(|e| e.into_inner()).tokens.values().map(|g| g.harness.clone()).collect()
    }

    fn save(&self, f: &File) -> Result<(), String> {
        let Some(path) = &self.path else { return Ok(()) };
        let text = serde_json::to_string_pretty(f).map_err(|e| e.to_string())?;
        write_private(path, text.as_bytes())
    }
}

/// Write at mode 600 through a fresh temporary file, then rename: never through a link.
pub(crate) fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt;
    let dir = path.parent().ok_or("no directory")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("could not make {}: {e}", dir.display()))?;
    let temp = dir.join(format!(".{}.{}.tmp", path.file_name().and_then(|n| n.to_str()).unwrap_or("f"), std::process::id()));
    let _ = std::fs::remove_file(&temp);
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp)
        .map_err(|e| format!("could not write {}: {e}", path.display()))?;
    f.write_all(bytes).and_then(|_| f.sync_all()).map_err(|e| format!("could not write {}: {e}", path.display()))?;
    std::fs::rename(&temp, path).map_err(|e| format!("could not replace {}: {e}", path.display()))
}
