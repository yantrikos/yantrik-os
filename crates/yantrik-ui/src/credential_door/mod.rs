//! The shell's credential door: the mail service keeps its accounts' passwords and OAuth tokens in
//! the shell's vault through this, and nowhere else.
//!
//! The vault lives in the companion's database, in this process. The mail service is another one
//! and must not link that database (`design/email-2026-09-20.md`), so this publishes a small door
//! on the service bus under `credentials` (`yantrik_ipc_contracts::credentials`). It is the shape
//! the saved AI providers already use, with a socket in the middle: the same vault, the same
//! worker, the same read-back rule (`yantrik_foundation::vault::keep`), the same waiting for an
//! unlock (`opened`).
//!
//! Who gets in, and what they get:
//!
//! - the caller must be `email-service` and run as this user, as the kernel says
//!   (`SO_PEERCRED`, `/proc/<pid>/exe`); the socket is 0600 in a 0700 directory besides;
//! - the id must be a mail id (`mail:<account>`): the door cannot read a saved provider's key, a
//!   free tier's, or anything else the vault holds;
//! - a shut vault answers `ERR_LOCKED` and the mail service pauses; nothing is cached here;
//! - no value is logged, formatted into an error or printed: this file never puts one in a string
//!   (`no_value_reaches_a_string`, in the tests, reads this file to keep it so).
//!
//! The door also tells the mail service when the vault opens (`announce_openings`), so a service
//! that paused for the shut vault resumes at the same hook the providers' migration uses.

use std::sync::Arc;

use yantrik_foundation::vault::opening;
use yantrik_foundation::vault::{KeyVault, VaultError};
use yantrik_ipc_contracts::credentials::{
    is_mail_id, method, ERR_LOCKED, ERR_REFUSED, ERR_UNAVAILABLE, SERVICE_ID,
};
use yantrik_ipc_contracts::email::ServiceError;
use yantrik_ipc_transport::owner;
use yantrik_ipc_transport::server::{PeerCred, RpcServer, ServiceHandler};

use crate::bridge::CompanionBridge;

#[cfg(test)]
mod tests;

/// Who may use the door.
type Caller = fn(Option<PeerCred>) -> Result<(), String>;

/// The vault in use now, if the shell has one yet.
type Vault = Arc<dyn Fn() -> Option<Arc<dyn KeyVault>> + Send + Sync>;

/// The mail service, as the kernel names it.
fn mail_service(peer: Option<PeerCred>) -> Result<(), String> {
    owner::must_be_program(peer, "email-service")
}

struct Door {
    vault: Vault,
    caller: Caller,
}

fn refused(message: impl Into<String>) -> ServiceError {
    ServiceError { code: ERR_REFUSED, message: message.into() }
}

fn why(e: VaultError) -> ServiceError {
    match e {
        VaultError::Locked => ServiceError { code: ERR_LOCKED, message: "the credential vault is locked".into() },
        VaultError::Failed => {
            ServiceError { code: ERR_UNAVAILABLE, message: "the credential vault did not answer".into() }
        }
    }
}

fn text<'a>(params: &'a serde_json::Value, key: &str) -> Result<&'a str, ServiceError> {
    params[key].as_str().ok_or_else(|| ServiceError { code: -32602, message: format!("missing `{key}`") })
}

impl Door {
    fn call(&self, method: &str, params: &serde_json::Value) -> Result<serde_json::Value, ServiceError> {
        let id = text(params, "id")?;
        if !is_mail_id(id) {
            // The id is the caller's to choose, so it is not quoted back.
            return Err(refused("the credential door keeps mail accounts' secrets only"));
        }
        let vault = (self.vault)().ok_or_else(|| why(VaultError::Failed))?;
        match method {
            method::STORE => {
                let value = text(params, "value")?;
                vault.store(id, value).map_err(why)?;
                Ok(serde_json::json!({}))
            }
            method::VALUE => {
                let value = vault.value(id).map_err(why)?;
                Ok(serde_json::json!({ "value": value }))
            }
            method::REMOVE => {
                vault.remove(id).map_err(why)?;
                Ok(serde_json::json!({}))
            }
            _ => Err(ServiceError { code: -1, message: format!("Unknown method: {method}") }),
        }
    }
}

impl ServiceHandler for Door {
    fn service_id(&self) -> &str {
        SERVICE_ID
    }

    /// Without a peer there is nobody to vouch for, so nobody is answered.
    fn handle(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, ServiceError> {
        Err(refused("the credential door answers the mail service, and could not tell who is calling"))
    }

    fn handle_from(
        &self,
        method: &str,
        params: serde_json::Value,
        peer: Option<PeerCred>,
    ) -> Result<serde_json::Value, ServiceError> {
        (self.caller)(peer).map_err(refused)?;
        self.call(method, &params)
    }
}

/// Publish the door, and tell the mail service whenever the vault opens. Called once at start,
/// beside the companion's own socket.
pub fn serve(bridge: &Arc<CompanionBridge>) {
    crate::provider_vault::install_for(bridge);
    announce_openings();
    let door = Arc::new(Door { vault: Arc::new(crate::provider_vault::current), caller: mail_service });
    let spawned = std::thread::Builder::new().name("credential-door".into()).spawn(move || {
        let runtime = match tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build() {
            Ok(rt) => rt,
            Err(e) => {
                tracing::error!(error = %e, "Credential door: no runtime, the mail service cannot keep its passwords");
                return;
            }
        };
        runtime.block_on(async {
            let address = RpcServer::default_address(SERVICE_ID);
            tracing::info!(address = %address, "Credential door listening");
            if let Err(e) = RpcServer::new(&address).serve(door).await {
                tracing::error!(error = %e, "Credential door stopped");
            }
        });
    });
    if spawned.is_err() {
        tracing::error!("no thread for the credential door; the mail service cannot keep its passwords");
    }
}

/// Tell the mail service the vault has just opened, if it is running. It is never started for
/// this: a service that is not running has nothing waiting. On a thread of its own, because the
/// vault is opened from the screen's thread and a call is a wait.
fn announce_openings() {
    opening::on_open(|| {
        let spawned = std::thread::Builder::new().name("mail-vault-opened".into()).spawn(|| {
            if !yantrik_ipc_transport::service::is_up("email") {
                return;
            }
            let told = yantrik_ipc_transport::SyncRpcClient::for_service("email")
                .with_timeout(std::time::Duration::from_secs(5))
                .call(yantrik_ipc_contracts::email::method::VAULT_OPENED, serde_json::json!({}));
            if let Err(e) = told {
                tracing::warn!(code = e.code, "the mail service was not told the vault opened");
            }
        });
        let _ = spawned;
    });
}
