//! A service's way to the shell's credential vault: the client of the shell's credential door
//! (`yantrik_ipc_contracts::credentials`).
//!
//! A service that keeps a secret holds a [`ShellVault`] where the shell holds its own vault, and
//! both are a [`KeyVault`]: the same `keep`, the same read-back, the same shut-vault error. The
//! secret travels once, over a unix socket that is mode 0600 in a 0700 directory, to a process the
//! kernel says is the shell (`expecting_peer`), and is never written to disk by the service.

use std::time::Duration;

use yantrik_foundation::vault::{KeyVault, VaultError};
use yantrik_ipc_contracts::credentials::{method, ERR_LOCKED, SERVICE_ID};
use yantrik_ipc_transport::protocol::RpcError;
use yantrik_ipc_transport::server::RpcServer;
use yantrik_ipc_transport::sync_client::{PeerRule, SyncRpcClient};

/// The vault answers within the shell's own ten-second bound for the worker that holds it; a
/// little longer than that here, so the door's refusal arrives rather than this client's timeout.
const TIMEOUT: Duration = Duration::from_secs(12);

/// The shell's vault, reached through its credential door.
pub struct ShellVault {
    address: String,
    shell: PeerRule,
}

impl ShellVault {
    /// The door at its usual address, answered only by the desktop's own shell.
    pub fn new() -> Self {
        Self::at(RpcServer::default_address(SERVICE_ID), yantrik_ipc_transport::owner::must_be_the_shell)
    }

    /// A door at `address`, whose answerer must satisfy `shell`. For a test's own.
    pub fn at(address: String, shell: PeerRule) -> Self {
        Self { address, shell }
    }

    fn call(&self, method: &str, params: serde_json::Value) -> Result<serde_json::Value, VaultError> {
        SyncRpcClient::new(&self.address)
            .with_timeout(TIMEOUT)
            .expecting_peer(self.shell)
            .call(method, params)
            .map_err(|e| failure(&e))
    }
}

impl Default for ShellVault {
    fn default() -> Self {
        Self::new()
    }
}

/// What a refusal from the door means: a shut vault is waited out, anything else is not.
pub fn failure(e: &RpcError) -> VaultError {
    if e.code == ERR_LOCKED {
        VaultError::Locked
    } else {
        VaultError::Failed
    }
}

impl KeyVault for ShellVault {
    fn store(&self, id: &str, value: &str) -> Result<(), VaultError> {
        self.call(method::STORE, serde_json::json!({ "id": id, "value": value })).map(|_| ())
    }

    fn value(&self, id: &str) -> Result<Option<String>, VaultError> {
        let answer = self.call(method::VALUE, serde_json::json!({ "id": id }))?;
        match &answer["value"] {
            serde_json::Value::Null => Ok(None),
            serde_json::Value::String(value) => Ok(Some(value.clone())),
            _ => Err(VaultError::Failed),
        }
    }

    fn remove(&self, id: &str) -> Result<(), VaultError> {
        self.call(method::REMOVE, serde_json::json!({ "id": id })).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refusal(code: i32) -> RpcError {
        RpcError { code, message: "no value here".into(), data: None }
    }

    #[test]
    fn only_a_shut_vault_is_waited_out() {
        assert_eq!(failure(&refusal(ERR_LOCKED)), VaultError::Locked);
        for code in [yantrik_ipc_contracts::credentials::ERR_UNAVAILABLE, yantrik_ipc_contracts::credentials::ERR_REFUSED, -32000, -1] {
            assert_eq!(failure(&refusal(code)), VaultError::Failed, "{code}");
        }
    }

    #[test]
    fn nothing_listening_is_a_failure_and_not_a_shut_vault() {
        let nowhere = std::env::temp_dir().join(format!("yantrik-no-door-{}.sock", std::process::id()));
        let vault = ShellVault::at(nowhere.display().to_string(), |_| Ok(()));
        assert_eq!(vault.value("mail:a"), Err(VaultError::Failed));
        assert_eq!(vault.store("mail:a", "fake-secret-0001"), Err(VaultError::Failed));
    }
}
