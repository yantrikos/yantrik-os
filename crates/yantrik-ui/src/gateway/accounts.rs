//! What the gateway asks the shell, answered from the AI accounts catalogue. The key is read at
//! the moment of the call, from the vault or the saved provider, and handed to the gateway alone.

use std::sync::Arc;

use yantrik_gateway::{ModelInfo, Refusal, Target};
use yantrik_ml::model_caps::caps_for;

use crate::ai_accounts::{self, account::Account, consent, keys};
use crate::bridge::CompanionBridge;
use crate::wire::settings::ProviderStore;

pub(super) struct ShellAccounts {
    pub bridge: Arc<CompanionBridge>,
}

/// Whether the person allowed private context for this account. A free tier that may train on
/// prompts never is: the free card promises nothing of the person's goes there.
pub(crate) fn private_ok(account: &Account, allowed: &consent::Consent) -> bool {
    account.local || (!account.trains_on_prompts && allowed.private_context.contains(&account.id))
}

impl yantrik_gateway::Accounts for ShellAccounts {
    fn models(&self) -> Vec<ModelInfo> {
        let accounts = ai_accounts::accounts();
        let allowed = consent::load(&consent::path());
        ai_accounts::catalogue()
            .models()
            .filter_map(|m| {
                let a = accounts.iter().find(|a| a.id == m.account)?;
                Some(ModelInfo {
                    id: m.id.clone(),
                    account: m.account.clone(),
                    name: m.name.clone(),
                    caps: m.caps.clone(),
                    local: a.local,
                    private_ok: private_ok(a, &allowed),
                })
            })
            .collect()
    }

    fn target(&self, account: &str, model: &str) -> Result<Target, Refusal> {
        let accounts = ai_accounts::accounts();
        let a = accounts.iter().find(|a| a.id == account).ok_or_else(|| {
            Refusal::new(404, "account_not_found", format!("There is no account `{account}`. Settings → AI & Intelligence lists them."))
        })?;
        let caps = ai_accounts::catalogue()
            .find(&ai_accounts::models::gateway_id(account, model))
            .map(|m| m.caps.clone())
            .unwrap_or_else(|| caps_for(&a.provider_type, model, None, None));
        let saved = ProviderStore::load().entries;
        let vault = |id: &str| ai_accounts::vault_value(&self.bridge, id);
        let resolved = keys::resolve(a, &saved, &vault).map_err(|why| {
            let code = match why {
                keys::Missing::Key => "key_missing",
                keys::Missing::Vault => "vault_locked",
                keys::Missing::Address => "address_incomplete",
            };
            Refusal::new(503, code, format!("{} cannot be used now: {why}.", a.name))
        })?;
        let allowed = consent::load(&consent::path());
        Ok(Target {
            account: a.id.clone(),
            model: model.to_string(),
            base_url: resolved.base_url,
            key: resolved.key,
            reasoning: caps.reasoning,
            efforts: caps.efforts,
            local: a.local,
            private_ok: private_ok(a, &allowed),
        })
    }

    fn private_mode(&self) -> bool {
        crate::private_mode::is_on()
    }

    fn picked_model(&self, harness: &str) -> Option<String> {
        let private_context = crate::provider_handoff::adapter_for(harness).map(|a| a.private_context()).unwrap_or(true);
        crate::wire::harness_provider::starting_model(harness, private_context).ok()
    }

    fn effort_for(&self, harness: &str, model: &str) -> Option<yantrik_ml::model_caps::Effort> {
        let choice = crate::picker::choice_of(harness);
        (choice.model == model).then(|| yantrik_ml::model_caps::Effort::parse(&choice.effort)).flatten()
    }
}
