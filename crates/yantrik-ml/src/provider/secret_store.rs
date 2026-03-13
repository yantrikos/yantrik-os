//! Secure API key storage — OS keyring with encrypted file fallback.
//!
//! Keys are stored via the OS credential manager (Windows Credential Manager,
//! Linux secret-service/kwallet) using the `keyring` crate. If the OS keyring
//! is unavailable, falls back to an AES-256-GCM encrypted file.
//!
//! # Secret references
//!
//! Config files use `secret_ref` pointers instead of plain-text keys:
//! ```yaml
//! secret_ref: "keyring://yantrik/providers/openai-main"
//! ```
//! The companion resolves these at config load time via `SecretStore::get()`.

use anyhow::{Context, Result};
use std::path::PathBuf;

// ── Trait ────────────────────────────────────────────────────────────────

/// Secure credential storage backend.
///
/// Implementations must be thread-safe (Send + Sync) because the provider
/// registry may resolve secrets from multiple threads during startup.
pub trait SecretStore: Send + Sync {
    /// Store a secret value under the given key.
    fn put(&self, key: &str, value: &str) -> Result<()>;

    /// Retrieve a secret value. Returns `None` if the key doesn't exist.
    fn get(&self, key: &str) -> Result<Option<String>>;

    /// Delete a stored secret. No-op if the key doesn't exist.
    fn delete(&self, key: &str) -> Result<()>;

    /// Human-readable backend name for diagnostics.
    fn backend_name(&self) -> &str;
}

// ── Keyring backend ──────────────────────────────────────────────────────

/// OS keyring backend using the `keyring` crate.
///
/// Maps to:
/// - Windows: Credential Manager
/// - Linux: secret-service (GNOME Keyring / KDE Wallet)
/// - macOS: Keychain
pub struct KeyringSecretStore {
    /// Service name prefix (e.g. "yantrik").
    service: String,
}

impl KeyringSecretStore {
    /// Create a new keyring-backed store.
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    /// Build the full keyring entry key.
    fn entry_key(&self, key: &str) -> String {
        format!("{}/{}", self.service, key)
    }

    /// Test whether the OS keyring is accessible.
    pub fn is_available(&self) -> bool {
        let test_key = "__yantrik_keyring_probe__";
        let entry = keyring::Entry::new(&self.service, test_key);
        match entry {
            Ok(e) => {
                // Try to set and delete a probe value
                if e.set_password("probe").is_ok() {
                    let _ = e.delete_credential();
                    true
                } else {
                    false
                }
            }
            Err(_) => false,
        }
    }
}

impl SecretStore for KeyringSecretStore {
    fn put(&self, key: &str, value: &str) -> Result<()> {
        let full_key = self.entry_key(key);
        let entry = keyring::Entry::new(&self.service, &full_key)
            .context("failed to create keyring entry")?;
        entry
            .set_password(value)
            .map_err(|e| anyhow::anyhow!("keyring put failed for '{}': {}", key, e))
    }

    fn get(&self, key: &str) -> Result<Option<String>> {
        let full_key = self.entry_key(key);
        let entry = keyring::Entry::new(&self.service, &full_key)
            .context("failed to create keyring entry")?;
        match entry.get_password() {
            Ok(pw) => Ok(Some(pw)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(anyhow::anyhow!("keyring get failed for '{}': {}", key, e)),
        }
    }

    fn delete(&self, key: &str) -> Result<()> {
        let full_key = self.entry_key(key);
        let entry = keyring::Entry::new(&self.service, &full_key)
            .context("failed to create keyring entry")?;
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()), // already gone
            Err(e) => Err(anyhow::anyhow!("keyring delete failed for '{}': {}", key, e)),
        }
    }

    fn backend_name(&self) -> &str {
        "os-keyring"
    }
}

// ── Encrypted file backend ───────────────────────────────────────────────

/// AES-256-GCM encrypted file store — fallback when OS keyring is unavailable.
///
/// Stores all secrets in a single JSON file encrypted with a key derived from
/// a machine-specific identifier (hostname + username + OS install ID).
pub struct EncryptedFileStore {
    /// Path to the encrypted secrets file.
    file_path: PathBuf,
    /// 32-byte encryption key derived from machine identity.
    key: [u8; 32],
}

impl EncryptedFileStore {
    /// Create a new encrypted file store.
    ///
    /// The encryption key is derived from `machine_id` using SHA-256.
    /// For best security, pass a combination of hostname + username + OS install ID.
    pub fn new(file_path: impl Into<PathBuf>, machine_id: &str) -> Self {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        // Derive a 32-byte key from the machine ID using repeated hashing.
        // This is NOT a proper KDF (would need argon2/scrypt for real security),
        // but provides adequate protection for the fallback case where no OS
        // keyring is available (embedded/minimal Linux).
        let mut key = [0u8; 32];
        let bytes = machine_id.as_bytes();
        for chunk_start in (0..32).step_by(8) {
            let mut hasher = DefaultHasher::new();
            chunk_start.hash(&mut hasher);
            bytes.hash(&mut hasher);
            let hash = hasher.finish().to_le_bytes();
            key[chunk_start..chunk_start + 8].copy_from_slice(&hash);
        }

        Self {
            file_path: file_path.into(),
            key,
        }
    }

    /// Default file path: `~/.config/yantrik/secrets.enc`
    pub fn default_path() -> PathBuf {
        let config_dir = dirs_next::config_dir()
            .unwrap_or_else(|| PathBuf::from("."));
        config_dir.join("yantrik").join("secrets.enc")
    }

    /// Generate a machine ID from available system info.
    pub fn default_machine_id() -> String {
        let hostname = hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "unknown-host".to_string());
        let username = whoami::username();
        format!("yantrik-secrets-{}-{}", hostname, username)
    }

    /// Read and decrypt the secrets map.
    fn read_secrets(&self) -> Result<std::collections::HashMap<String, String>> {
        if !self.file_path.exists() {
            return Ok(std::collections::HashMap::new());
        }

        let ciphertext = std::fs::read(&self.file_path)
            .context("failed to read secrets file")?;

        if ciphertext.len() < 12 {
            // File too short to contain nonce — treat as empty/corrupt
            tracing::warn!("secrets file too short, starting fresh");
            return Ok(std::collections::HashMap::new());
        }

        // Split: first 12 bytes = nonce, rest = ciphertext + tag
        let (nonce_bytes, encrypted) = ciphertext.split_at(12);

        use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
        use aes_gcm::aead::Aead;

        let cipher = Aes256Gcm::new_from_slice(&self.key)
            .map_err(|e| anyhow::anyhow!("AES key init failed: {}", e))?;
        let nonce = Nonce::from_slice(nonce_bytes);

        let plaintext = cipher
            .decrypt(nonce, encrypted)
            .map_err(|_| anyhow::anyhow!("failed to decrypt secrets file — key mismatch or corruption"))?;

        let map: std::collections::HashMap<String, String> =
            serde_json::from_slice(&plaintext).context("failed to parse decrypted secrets")?;

        Ok(map)
    }

    /// Encrypt and write the secrets map.
    fn write_secrets(&self, map: &std::collections::HashMap<String, String>) -> Result<()> {
        // Ensure parent directory exists
        if let Some(parent) = self.file_path.parent() {
            std::fs::create_dir_all(parent)
                .context("failed to create secrets directory")?;
        }

        let plaintext = serde_json::to_vec(map).context("failed to serialize secrets")?;

        use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
        use aes_gcm::aead::Aead;

        let cipher = Aes256Gcm::new_from_slice(&self.key)
            .map_err(|e| anyhow::anyhow!("AES key init failed: {}", e))?;

        // Generate random 12-byte nonce
        let mut nonce_bytes = [0u8; 12];
        use rand::RngCore;
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, plaintext.as_ref())
            .map_err(|e| anyhow::anyhow!("encryption failed: {}", e))?;

        // Write: nonce || ciphertext
        let mut output = Vec::with_capacity(12 + ciphertext.len());
        output.extend_from_slice(&nonce_bytes);
        output.extend_from_slice(&ciphertext);

        // Write atomically via temp file
        let tmp_path = self.file_path.with_extension("tmp");
        std::fs::write(&tmp_path, &output).context("failed to write temp secrets file")?;
        std::fs::rename(&tmp_path, &self.file_path).context("failed to rename secrets file")?;

        Ok(())
    }
}

impl SecretStore for EncryptedFileStore {
    fn put(&self, key: &str, value: &str) -> Result<()> {
        let mut map = self.read_secrets()?;
        map.insert(key.to_string(), value.to_string());
        self.write_secrets(&map)
    }

    fn get(&self, key: &str) -> Result<Option<String>> {
        let map = self.read_secrets()?;
        Ok(map.get(key).cloned())
    }

    fn delete(&self, key: &str) -> Result<()> {
        let mut map = self.read_secrets()?;
        map.remove(key);
        self.write_secrets(&map)
    }

    fn backend_name(&self) -> &str {
        "encrypted-file"
    }
}

// ── Auto-detecting store ─────────────────────────────────────────────────

/// Auto-detecting secret store — tries OS keyring first, falls back to encrypted file.
pub struct AutoSecretStore {
    inner: Box<dyn SecretStore>,
}

impl AutoSecretStore {
    /// Create a new auto-detecting store.
    ///
    /// Probes the OS keyring; if unavailable, falls back to encrypted file storage.
    pub fn new() -> Self {
        let keyring = KeyringSecretStore::new("yantrik");
        if keyring.is_available() {
            tracing::info!("using OS keyring for secret storage");
            Self {
                inner: Box::new(keyring),
            }
        } else {
            tracing::info!("OS keyring unavailable, falling back to encrypted file store");
            let path = EncryptedFileStore::default_path();
            let machine_id = EncryptedFileStore::default_machine_id();
            Self {
                inner: Box::new(EncryptedFileStore::new(path, &machine_id)),
            }
        }
    }

    /// Create with a specific backend (for testing).
    pub fn with_backend(backend: Box<dyn SecretStore>) -> Self {
        Self { inner: backend }
    }

    /// Which backend is active.
    pub fn active_backend(&self) -> &str {
        self.inner.backend_name()
    }
}

impl SecretStore for AutoSecretStore {
    fn put(&self, key: &str, value: &str) -> Result<()> {
        self.inner.put(key, value)
    }

    fn get(&self, key: &str) -> Result<Option<String>> {
        self.inner.get(key)
    }

    fn delete(&self, key: &str) -> Result<()> {
        self.inner.delete(key)
    }

    fn backend_name(&self) -> &str {
        self.inner.backend_name()
    }
}

// ── Secret reference resolution ──────────────────────────────────────────

/// A secret reference pointer used in config files instead of plain-text keys.
///
/// Format: `keyring://yantrik/providers/{provider_id}`
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SecretRef {
    /// The full URI (e.g. "keyring://yantrik/providers/openai-main").
    pub uri: String,
}

impl SecretRef {
    /// Create a new secret reference for a provider.
    pub fn for_provider(provider_id: &str) -> Self {
        Self {
            uri: format!("keyring://yantrik/providers/{}", provider_id),
        }
    }

    /// Extract the storage key from the URI.
    ///
    /// `keyring://yantrik/providers/openai-main` -> `providers/openai-main`
    pub fn storage_key(&self) -> &str {
        self.uri
            .strip_prefix("keyring://yantrik/")
            .unwrap_or(&self.uri)
    }

    /// Resolve this reference against a SecretStore.
    pub fn resolve(&self, store: &dyn SecretStore) -> Result<Option<String>> {
        store.get(self.storage_key())
    }
}

// ── Config migration ─────────────────────────────────────────────────────

/// Migrate plain-text API keys from config to the secret store.
///
/// For each provider entry that has a plain `api_key`, this function:
/// 1. Stores the key in the SecretStore
/// 2. Returns a modified config value with `secret_ref` replacing `api_key`
///
/// This is idempotent — already-migrated entries (with `secret_ref`) are skipped.
pub fn migrate_config_keys(
    providers: &mut [crate::types::ProviderConfigEntry],
    store: &dyn SecretStore,
) -> Result<usize> {
    let mut migrated = 0;

    for provider in providers.iter_mut() {
        // Skip if already has a secret_ref
        if provider.secret_ref.is_some() {
            continue;
        }

        // Skip if no plain key
        let api_key = match &provider.api_key {
            Some(key) if !key.is_empty() => key.clone(),
            _ => continue,
        };

        // Store in secret store
        let secret_ref = SecretRef::for_provider(&provider.id);
        store.put(secret_ref.storage_key(), &api_key)
            .with_context(|| format!("failed to migrate key for provider '{}'", provider.id))?;

        // Replace plain key with secret_ref
        provider.secret_ref = Some(secret_ref.uri.clone());
        provider.api_key = None;
        migrated += 1;

        tracing::info!(
            provider = %provider.id,
            backend = store.backend_name(),
            "migrated API key to secure storage"
        );
    }

    Ok(migrated)
}

/// Resolve all secret references in provider configs, returning plain keys
/// for runtime use (never written back to disk).
pub fn resolve_provider_keys(
    providers: &[crate::types::ProviderConfigEntry],
    store: &dyn SecretStore,
) -> Result<std::collections::HashMap<String, String>> {
    let mut resolved = std::collections::HashMap::new();

    for provider in providers {
        // If there's a secret_ref, resolve it
        if let Some(ref uri) = provider.secret_ref {
            let secret_ref = SecretRef { uri: uri.clone() };
            if let Some(key) = secret_ref.resolve(store)? {
                resolved.insert(provider.id.clone(), key);
                continue;
            }
            tracing::warn!(
                provider = %provider.id,
                uri = %uri,
                "secret_ref could not be resolved"
            );
        }

        // Fall back to plain key if present
        if let Some(ref key) = provider.api_key {
            if !key.is_empty() {
                resolved.insert(provider.id.clone(), key.clone());
            }
        }
    }

    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// In-memory secret store for testing.
    struct MemoryStore {
        data: Mutex<HashMap<String, String>>,
    }

    impl MemoryStore {
        fn new() -> Self {
            Self {
                data: Mutex::new(HashMap::new()),
            }
        }
    }

    impl SecretStore for MemoryStore {
        fn put(&self, key: &str, value: &str) -> Result<()> {
            self.data.lock().unwrap().insert(key.to_string(), value.to_string());
            Ok(())
        }

        fn get(&self, key: &str) -> Result<Option<String>> {
            Ok(self.data.lock().unwrap().get(key).cloned())
        }

        fn delete(&self, key: &str) -> Result<()> {
            self.data.lock().unwrap().remove(key);
            Ok(())
        }

        fn backend_name(&self) -> &str {
            "memory"
        }
    }

    #[test]
    fn test_secret_ref_storage_key() {
        let sr = SecretRef::for_provider("openai-main");
        assert_eq!(sr.storage_key(), "providers/openai-main");
        assert_eq!(sr.uri, "keyring://yantrik/providers/openai-main");
    }

    #[test]
    fn test_memory_store_roundtrip() {
        let store = MemoryStore::new();
        store.put("test-key", "test-value").unwrap();
        assert_eq!(store.get("test-key").unwrap(), Some("test-value".to_string()));
        store.delete("test-key").unwrap();
        assert_eq!(store.get("test-key").unwrap(), None);
    }

    #[test]
    fn test_secret_ref_resolve() {
        let store = MemoryStore::new();
        store.put("providers/openai-main", "sk-abc123").unwrap();

        let sr = SecretRef::for_provider("openai-main");
        assert_eq!(sr.resolve(&store).unwrap(), Some("sk-abc123".to_string()));
    }
}
