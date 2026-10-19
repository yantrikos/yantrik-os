//! One token per harness: minted by the OS, written into that harness's own config, and kept here
//! as its SHA-256 only, so the gateway's own file cannot be used to call it.
//!
//! `ygw-` and 256 random bits as hex. A token names a harness and whether that harness sends the
//! person's private context (every one issued now does: every mind has YantrikDB memory): the
//! one fact [`crate::policy`] needs.
//!
//! The file is the one record: a harness wired at install by another process (`yantrik-ui
//! --wire-model`) writes it while the shell runs, so the set is read again whenever the file has
//! changed since it was last read, before a change and when a token is not found. Like the agent token (docs/harness.md), it stops confusion
//! and casual impersonation, not a hostile program running as the person, which can read the
//! harness's file as well as the harness can.

use std::collections::BTreeMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, RwLock};
use std::time::SystemTime;

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
    /// The file's modification time when it was last read or written.
    read_at: Mutex<Option<SystemTime>>,
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// The file as the gateway trusts it. Every grant read from it carries private context: every
/// token the OS issues now does, and the file is writable by anything running as the person, so a
/// grant there saying otherwise (from before, or written by a harness) must not let a call skip
/// the person's Share / Private lane / Off choice. Only [`Tokens::register`] in this process can
/// hold a grant without it.
fn read(path: &Path) -> Option<File> {
    let mut file: File = std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok())?;
    for g in file.tokens.values_mut() {
        g.private_context = true;
    }
    Some(file)
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
        Tokens { path: None, inner: RwLock::new(File::default()), read_at: Mutex::new(None) }
    }

    /// Tokens kept at `path` (`~/.config/yantrik/gateway/tokens.json`, mode 600): hashes only.
    pub fn open(path: &Path) -> Tokens {
        let at = modified(path);
        let file = read(path).unwrap_or_default();
        Tokens { path: Some(path.to_path_buf()), inner: RwLock::new(file), read_at: Mutex::new(at) }
    }

    /// Read the file again into `f` when another process changed it since it was last read.
    fn refresh(&self, f: &mut File) {
        let Some(path) = &self.path else { return };
        let now = modified(path);
        let mut read_at = self.read_at.lock().unwrap_or_else(|e| e.into_inner());
        if now == *read_at {
            return;
        }
        if let Some(file) = read(path) {
            *f = file;
        }
        *read_at = now;
    }

    /// The file changed since it was last read (one stat).
    fn stale(&self) -> bool {
        let Some(path) = &self.path else { return false };
        modified(path) != *self.read_at.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Accept `token` for `grant.harness`, replacing any token that harness had: one per harness.
    pub fn register(&self, token: &str, grant: Grant) -> Result<(), String> {
        if !token.starts_with(PREFIX) || token.len() != PREFIX.len() + 64 {
            return Err("not a gateway token".into());
        }
        let mut f = self.inner.write().unwrap_or_else(|e| e.into_inner());
        self.refresh(&mut f);
        f.tokens.retain(|_, g| g.harness != grant.harness);
        f.tokens.insert(digest(token), grant);
        self.save(&f)
    }

    /// Stop accepting the harness's token (Revert).
    pub fn revoke(&self, harness: &str) -> Result<bool, String> {
        let mut f = self.inner.write().unwrap_or_else(|e| e.into_inner());
        self.refresh(&mut f);
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
        let d = digest(token);
        // Wired or revoked by another process since it was read: one stat, and a read only if it
        // changed. Checked on every call, found or not, so a token withdrawn elsewhere (Revert,
        // "Use its own models", a re-wire) stops working at once rather than at the next miss.
        if self.stale() {
            let mut f = self.inner.write().unwrap_or_else(|e| e.into_inner());
            self.refresh(&mut f);
            return f.tokens.get(&d).cloned();
        }
        self.inner.read().unwrap_or_else(|e| e.into_inner()).tokens.get(&d).cloned()
    }

    /// Mark every token held as carrying private context, as every one issued now is (every mind
    /// has YantrikDB memory): a token issued before that, marked as carrying none, would otherwise
    /// skip the person's choice per account. Returns how many changed.
    pub fn mark_all_private(&self) -> Result<usize, String> {
        let mut f = self.inner.write().unwrap_or_else(|e| e.into_inner());
        self.refresh(&mut f);
        let mut changed = 0;
        for g in f.tokens.values_mut().filter(|g| !g.private_context) {
            g.private_context = true;
            changed += 1;
        }
        if changed > 0 {
            self.save(&f)?;
        }
        Ok(changed)
    }

    /// The harnesses that hold a token.
    pub fn harnesses(&self) -> Vec<String> {
        self.inner.read().unwrap_or_else(|e| e.into_inner()).tokens.values().map(|g| g.harness.clone()).collect()
    }

    fn save(&self, f: &File) -> Result<(), String> {
        let Some(path) = &self.path else { return Ok(()) };
        let text = serde_json::to_string_pretty(f).map_err(|e| e.to_string())?;
        write_private(path, text.as_bytes())?;
        *self.read_at.lock().unwrap_or_else(|e| e.into_inner()) = modified(path);
        Ok(())
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
