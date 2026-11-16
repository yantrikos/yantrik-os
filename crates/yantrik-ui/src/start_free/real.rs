//! The press's steps, each the function an existing path already calls: nothing here writes a
//! file of its own.

use std::cell::Cell;
use std::path::PathBuf;

use super::press::{Failed, Steps};
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
    /// The copy Apply keeps of the person's own file was already there (so it is not Apply's to remove).
    had_copy: bool,
    already_on_models: bool,
    /// What the person already chose for the companion's provider, when it is not Yantrik models (a saved
    /// provider, or "Use its own models"): Revert would put back the file from before it, or not put
    /// back the mark, so the press leaves it alone. One rule for "the person already chose".
    chose: Option<String>,
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
            had_copy: provider_handoff::backup_path(&companion::file(&home)).exists(),
            already_on_models: provider_handoff::uses_gateway(&home, companion::ID),
            chose: chosen_before(&home),
            gave: Cell::new(false),
            account,
            model,
            check,
        }
    }

    /// The companion's file back as it was (or gone), and what a failed apply left beside it; what
    /// could not be done, said.
    fn put_back_companion_file(&self, home: &std::path::Path) -> Vec<String> {
        let path = companion::file(home);
        let mut left = Vec::new();
        let put_back = match &self.companion_file {
            Some(bytes) => crate::private_file::write(&path, bytes, crate::private_file::Publish::Replace, || Ok(())),
            None => remove(&path),
        };
        if let Err(e) = put_back {
            left.push(format!("the companion's file was not put back ({e})"));
        }
        if !self.had_copy {
            if let Err(e) = remove(&provider_handoff::backup_path(&path)) {
                left.push(format!("the copy Apply made of it was not removed ({e})"));
            }
        }
        provider_handoff::abandon_attempt(home, companion::ID);
        left
    }
}

/// What the person chose for the companion before the press, if it is not Yantrik models.
fn chosen_before(home: &std::path::Path) -> Option<String> {
    if let Some(m) = provider_handoff::marker(home, companion::ID).filter(|m| m.provider_id != provider_handoff::GATEWAY_ID) {
        return Some(format!("the companion already uses {}, which you chose in Settings \u{2192} Minds; change that there first", m.provider_name));
    }
    provider_handoff::uses_own_models(home, companion::ID)
        .then(|| "you chose that Yantrik Companion uses its own models, in Settings \u{2192} Minds; change that there first".to_string())
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
    fn allow_sharing(&self) -> Result<(), Failed> {
        Ok(consent::set_for_account(&consent::path(), &self.account, Sharing::Share)?)
    }

    /// Settings → Minds, "Use Yantrik models" → Apply, on the companion's row. A companion that is on
    /// them already has them: nothing is applied, and so nothing is undone.
    fn give_models(&self) -> Result<(), Failed> {
        if let Some(chose) = &self.chose {
            return Err(chose.clone().into());
        }
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
            // It may have written the file and accepted a token before it failed.
            Err(why) => Err(Failed { why, left: self.put_back_companion_file(&home) }),
        }
    }

    fn pick_model(&self) -> Result<(), Failed> {
        if crate::ai_accounts::catalogue().find(&self.model).is_none() {
            return Err(format!("{} is not one of the listed models", self.model).into());
        }
        crate::picker::pick_model_for(companion::ID, &self.model);
        Ok(())
    }

    fn undo_sharing(&self) -> Result<(), String> {
        let path = consent::path();
        let put_back = match &self.sharing_file {
            Some(bytes) => crate::private_file::write(&path, bytes, crate::private_file::Publish::Replace, || Ok(())),
            None => remove(&path),
        };
        put_back.map_err(|e| format!("the sharing choice was not put back ({e})"))
    }

    /// "Revert" on the companion's row, only for what this press gave.
    fn undo_models(&self) -> Result<(), String> {
        if !self.gave.get() {
            return Ok(());
        }
        provider_handoff::revert(&home(), companion::ID)
            .map(|_| ())
            .map_err(|e| format!("the companion was not put back on its own models ({e})"))
    }

    fn check(&self) -> Result<String, String> {
        (self.check)()
    }
}
