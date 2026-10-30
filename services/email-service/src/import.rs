//! `email.import_account`: the service's side of moving the companion's `config.yaml` mail
//! accounts here (#1022).
//!
//! The decision about an account that is already here lives in [`crate::store::AccountStore::import`];
//! this is the wire: parse the parameters without echoing them, call the store, and say what
//! happened in words that quote nothing from the request.

use yantrik_ipc_contracts::email::{ImportAccount, ServiceError};
use yantrik_service_sdk::prelude::tracing;

use crate::store::AccountStore;

pub fn handle(store: &AccountStore, params: &serde_json::Value) -> Result<serde_json::Value, ServiceError> {
    // The serde error can quote the text it could not read, and the text is a secret.
    let import: ImportAccount = serde_json::from_value(params.clone()).map_err(|_| ServiceError {
        code: -32602,
        message: "the account to import is not in the shape email.import_account takes".to_string(),
    })?;
    if !import.has_secret() {
        return Err(ServiceError {
            code: -32602,
            message: "the account to import has no password or sign-in to keep".to_string(),
        });
    }
    let result = store.import(&import).map_err(crate::store::StoreError::into_service_error)?;
    if result.created {
        tracing::info!(account = %result.account.id, "account imported");
    }
    Ok(serde_json::to_value(result).unwrap_or_default())
}
