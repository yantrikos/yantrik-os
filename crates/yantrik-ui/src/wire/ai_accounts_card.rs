//! Settings → AI & Intelligence → AI accounts: the catalogue drawn (components/ai_accounts_card.slint).
//! What each row says is made here, from the catalogue, the accounts and the consent file; no key
//! is ever read for it.

use std::sync::Arc;

use slint::{ComponentHandle, ModelRc, VecModel};

use crate::ai_accounts::{self, account::Account, consent::Consent, models::Listing, store::Catalogue};
use crate::app_context::AppContext;
use crate::{AiAccountRow, AiAccountsState, App};

/// One row, as words.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub id: String,
    pub name: String,
    pub summary: String,
    pub state: &'static str,
    pub detail: String,
    pub local: bool,
    pub private_ok: bool,
    pub private_words: String,
    pub switchable: bool,
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The card's summary and rows.
pub fn rows(catalogue: &Catalogue, accounts: &[Account], consent: &Consent) -> (String, Vec<Row>) {
    let mut out = Vec::new();
    for a in accounts {
        let listed = catalogue.accounts.iter().find(|m| m.account == a.id);
        let models = listed.map(|m| m.models.as_slice()).unwrap_or_default();
        let thinks = models.iter().filter(|m| m.caps.thinks()).count();
        let sees = models.iter().filter(|m| m.caps.vision).count();
        let mut summary = plural(models.len(), "model", "models");
        if thinks > 0 {
            summary.push_str(&format!(" · {}", plural(thinks, "thinks", "think")));
        }
        if sees > 0 {
            summary.push_str(&format!(" · {}", plural(sees, "sees images", "see images")));
        }
        let (state, detail) = match listed.map(|m| &m.listing) {
            Some(Listing::Provider) => ("ready", format!("Listed by {} itself.", a.name)),
            Some(Listing::Curated) => ("ready", "It has no model list of its own; these are the ones its documentation names.".to_string()),
            Some(Listing::Failed(why)) => ("failed", format!("Its models could not be listed: {why}")),
            Some(Listing::Pending) | None => ("pending", "Not listed yet.".to_string()),
        };
        let private_ok = crate::gateway::private_ok(a, consent);
        let private_words = if a.local {
            "Private context: stays on this network".to_string()
        } else if private_ok {
            "Private context: allowed for minds that send it".to_string()
        } else if a.trains_on_prompts {
            "Private context: never (this free tier may train on prompts)".to_string()
        } else {
            "Private context: not allowed".to_string()
        };
        out.push(Row {
            id: a.id.clone(),
            name: a.name.clone(),
            summary,
            state,
            detail,
            local: a.local,
            private_ok,
            private_words,
            switchable: !a.local && !a.trains_on_prompts,
        });
    }
    let total: usize = catalogue.models().count();
    let summary = format!("{} · {}", plural(accounts.len(), "account", "accounts"), plural(total, "model", "models"));
    (summary, out)
}

fn render(ui: &App) {
    let (summary, rows) = rows(&ai_accounts::catalogue(), &ai_accounts::accounts(), &ai_accounts::consent::load(&ai_accounts::consent::path()));
    let g = ui.global::<AiAccountsState>();
    let items: Vec<AiAccountRow> = rows
        .into_iter()
        .map(|r| AiAccountRow {
            id: r.id.into(),
            name: r.name.into(),
            summary: r.summary.into(),
            state: r.state.into(),
            detail: r.detail.into(),
            local: r.local,
            private_ok: r.private_ok,
            private_words: r.private_words.into(),
            switchable: r.switchable,
        })
        .collect();
    g.set_rows(ModelRc::new(VecModel::from(items)));
    g.set_summary(summary.into());
}

pub fn wire(ui: &App, ctx: &AppContext) {
    let bridge: Arc<_> = ctx.bridge.clone();
    let g = ui.global::<AiAccountsState>();
    let weak = ui.as_weak();
    ai_accounts::on_change(move || {
        let weak = weak.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = weak.upgrade() {
                ui.global::<AiAccountsState>().set_refreshing(false);
                render(&ui);
            }
        });
    });
    let weak = ui.as_weak();
    let b = bridge.clone();
    g.on_refresh(move || {
        if let Some(ui) = weak.upgrade() {
            ui.global::<AiAccountsState>().set_refreshing(true);
        }
        ai_accounts::refresh(b.clone());
    });
    let weak = ui.as_weak();
    g.on_toggle_private(move |id| {
        let Some(ui) = weak.upgrade() else { return };
        // A free tier that may train on prompts is never sent anything private: no switch.
        if ai_accounts::accounts().iter().any(|a| a.id == id.as_str() && (a.trains_on_prompts || a.local)) {
            return;
        }
        if let Err(e) = ai_accounts::consent::toggle(&ai_accounts::consent::path(), &id) {
            tracing::warn!(error = %e, "private context switch was not saved");
        }
        render(&ui);
    });
    render(ui);
    ai_accounts::refresh(bridge);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_accounts::account::{KeyRef, Source};
    use crate::ai_accounts::models::{build, AccountModels};

    fn account(id: &str, local: bool, trains: bool) -> Account {
        Account {
            id: id.into(),
            name: id.into(),
            provider_type: "openai".into(),
            base_url: "https://api.openai.com/v1".into(),
            key: KeyRef::None,
            local,
            source: Source::Saved,
            trains_on_prompts: trains,
            saved_model: String::new(),
        }
    }

    #[test]
    fn each_account_says_its_models_what_they_can_do_and_whether_private_context_may_go() {
        let a = account("openai", false, false);
        let listed = vec![
            crate::wire::provider_models::ListedModel { id: "o4-mini".into(), name: "o4-mini".into(), size_bytes: 0 },
            crate::wire::provider_models::ListedModel { id: "gpt-4o".into(), name: "gpt-4o".into(), size_bytes: 0 },
        ];
        let failed = account("broken", false, true);
        let catalogue = Catalogue {
            accounts: vec![
                build(&a, &[], Some(Ok(listed))),
                AccountModels { account: "broken".into(), name: "broken".into(), listing: Listing::Failed("The key was refused.".into()), models: vec![] },
            ],
            at: 0,
        };
        let home = account("home", true, false);
        let (summary, drawn) = rows(&catalogue, &[a, failed, home], &Consent::default());
        assert_eq!(summary, "3 accounts · 2 models");
        assert_eq!(drawn[0].summary, "2 models · 1 thinks · 1 sees images");
        assert_eq!(drawn[0].state, "ready");
        assert!(!drawn[0].private_ok && drawn[0].private_words.ends_with("not allowed"));
        assert_eq!(drawn[1].state, "failed");
        assert!(drawn[1].detail.contains("refused"));
        assert!(drawn[1].private_words.contains("may train"));
        assert!(!drawn[1].switchable && drawn[0].switchable && !drawn[2].switchable);
        assert!(drawn[2].private_ok && drawn[2].local, "a local account keeps private context on the network");
        let consent = Consent { private_context: ["openai".to_string()].into_iter().collect() };
        let (_, allowed) = rows(&catalogue, &[account("openai", false, false)], &consent);
        assert!(allowed[0].private_ok);
    }
}
