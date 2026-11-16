//! The press's steps, each the function an existing path already calls: nothing here writes a
//! file of its own.

use std::cell::Cell;
use std::path::PathBuf;

use super::press::Steps;
use crate::ai_accounts::account::Account;
use crate::ai_accounts::consent::{self, Sharing};
use crate::provider_handoff::{self, companion};

/// What each step would have to put back, taken before the first one runs.
pub struct Real {
    account: Account,
    /// `<account>/<model>`.
    model: String,
    sharing_file: Option<Vec<u8>>,
    /// The companion's file as it was, and whether it was on Yantrik models already: then there is
    /// nothing to give, and nothing of the person's to take back.
    companion_file: Option<Vec<u8>>,
    already_on_models: bool,
    /// This press gave the companion Yantrik models (so only this press may take them back).
    gave: Cell<bool>,
    check: Box<dyn Fn() -> Result<String, String> + Send>,
}

impl Real {
    pub fn new(account: Account, model: String, check: Box<dyn Fn() -> Result<String, String> + Send>) -> Real {
        let home = home();
        Real {
            sharing_file: std::fs::read(consent::path()).ok(),
            companion_file: std::fs::read(companion::file(&home)).ok(),
            already_on_models: provider_handoff::uses_gateway(&home, companion::ID),
            gave: Cell::new(false),
            account,
            model,
            check,
        }
    }

    /// The companion's file back as it was (or gone), and what a failed apply left beside it.
    fn put_back_companion_file(&self, home: &std::path::Path) {
        let path = companion::file(home);
        let put_back = match &self.companion_file {
            Some(bytes) => crate::private_file::write(&path, bytes, crate::private_file::Publish::Replace, || Ok(())),
            None => remove(&path),
        };
        if let Err(e) = put_back {
            tracing::warn!(error = %e, "the companion's file was not put back");
        }
        provider_handoff::abandon_attempt(home, companion::ID);
    }
}

fn home() -> PathBuf {
    crate::wire::harness_provider::home_dir()
}

fn remove(path: &std::path::Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
        _ => Ok(()),
    }
}

impl Steps for Real {
    fn allow_sharing(&self) -> Result<(), String> {
        consent::set_for_account(&consent::path(), &self.account, Sharing::Share)
    }

    /// Settings → Minds, "Use Yantrik models" → Apply, on the companion's row. A companion that is on
    /// them already has them: nothing is applied, and so nothing is undone.
    fn give_models(&self) -> Result<(), String> {
        if self.already_on_models {
            return Ok(());
        }
        let home = home();
        let plan = crate::wire::harness_provider::plan(companion::ID, provider_handoff::GATEWAY_ID)?;
        match provider_handoff::apply(&home, &plan) {
            Ok(_) => {
                self.gave.set(true);
                Ok(())
            }
            Err(why) => {
                // It may have written the file and accepted a token before it failed.
                self.put_back_companion_file(&home);
                Err(why)
            }
        }
    }

    fn pick_model(&self) -> Result<(), String> {
        if crate::ai_accounts::catalogue().find(&self.model).is_none() {
            return Err(format!("{} is not one of the listed models", self.model));
        }
        crate::picker::pick_model_for(companion::ID, &self.model);
        Ok(())
    }

    fn undo_sharing(&self) {
        let path = consent::path();
        let put_back = match &self.sharing_file {
            Some(bytes) => crate::private_file::write(&path, bytes, crate::private_file::Publish::Replace, || Ok(())),
            None => remove(&path),
        };
        if let Err(e) = put_back {
            tracing::warn!(error = %e, "the sharing choice was not put back");
        }
    }

    /// "Revert" on the companion's row, only for what this press gave.
    fn undo_models(&self) {
        if !self.gave.get() {
            return;
        }
        if let Err(e) = provider_handoff::revert(&home(), companion::ID) {
            tracing::warn!(error = %e, "the companion was not put back on its own models");
        }
    }

    fn check(&self) -> Result<String, String> {
        (self.check)()
    }
}
