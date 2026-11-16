//! The press's steps, each the function an existing path already calls: nothing here writes a
//! file of its own.

use std::path::PathBuf;

use super::press::Steps;
use crate::ai_accounts::account::Account;
use crate::ai_accounts::consent::{self, Sharing};
use crate::picker::choices::Choices;
use crate::provider_handoff;

/// What each step would have to put back, taken before the first one runs.
pub struct Real {
    account: Account,
    /// `<account>/<model>`.
    model: String,
    sharing_file: Option<Vec<u8>>,
    picks: Choices,
    check: Box<dyn Fn() -> Result<String, String> + Send>,
}

impl Real {
    pub fn new(account: Account, model: String, check: Box<dyn Fn() -> Result<String, String> + Send>) -> Real {
        Real { sharing_file: std::fs::read(consent::path()).ok(), picks: crate::picker::picks(), account, model, check }
    }
}

fn home() -> PathBuf {
    crate::wire::harness_provider::home_dir()
}

impl Steps for Real {
    fn allow_sharing(&self) -> Result<(), String> {
        consent::set_for_account(&consent::path(), &self.account, Sharing::Share)
    }

    /// Settings → Minds, "Use Yantrik models" → Apply, on the companion's row.
    fn give_models(&self) -> Result<(), String> {
        let id = provider_handoff::companion::ID;
        let plan = crate::wire::harness_provider::plan(id, provider_handoff::GATEWAY_ID)?;
        provider_handoff::apply(&home(), &plan).map(|_| ())
    }

    fn pick_model(&self) -> Result<(), String> {
        if crate::ai_accounts::catalogue().find(&self.model).is_none() {
            return Err(format!("{} is not one of the listed models", self.model));
        }
        crate::picker::pick_model_for(provider_handoff::companion::ID, &self.model);
        Ok(())
    }

    fn undo_sharing(&self) {
        let path = consent::path();
        let put_back = match &self.sharing_file {
            Some(bytes) => crate::private_file::write(&path, bytes, crate::private_file::Publish::Replace, || Ok(())),
            None => match std::fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
                _ => Ok(()),
            },
        };
        if let Err(e) = put_back {
            tracing::warn!(error = %e, "the sharing choice was not put back");
        }
    }

    /// "Revert" on the companion's row.
    fn undo_models(&self) {
        if let Err(e) = provider_handoff::revert(&home(), provider_handoff::companion::ID) {
            tracing::warn!(error = %e, "the companion was not put back on its own models");
        }
    }

    fn undo_pick(&self) {
        crate::picker::restore_picks(self.picks.clone());
    }

    fn check(&self) -> Result<String, String> {
        (self.check)()
    }
}
