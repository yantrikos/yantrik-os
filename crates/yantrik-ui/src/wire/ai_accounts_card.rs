//! Settings → AI & Intelligence → AI accounts: the catalogue drawn (components/ai_accounts_card.slint).
//! What each row says is made here, from the catalogue, the accounts and the consent file; no key
//! is ever read for it.

use std::sync::Arc;

use slint::{ComponentHandle, ModelRc, VecModel};

use crate::ai_accounts::{self, account::Account, consent::{self, Consent, Sharing}, models::Listing, store::Catalogue};
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
    /// The person's choice as the gateway applies it: "share", "lane" or "off".
    pub sharing: &'static str,
    /// "Share what your minds know about you with OpenAI".
    pub share_words: String,
    /// What happens now, in one plain sentence.
    pub private_words: String,
    /// The three choices are offered: not local (nothing leaves it).
    pub switchable: bool,
    /// Share is among them: not a free tier that may train on what it is sent.
    pub offers_share: bool,
    /// A local account: the model it would keep personal things on, as the private lane
    /// (`<account>/<model>`), or "" when it has none listed.
    pub lane_model: String,
    /// It is the private lane now.
    pub is_lane: bool,
}

/// The private lane, in words: "Personal things stay on: On this computer (Qwen 3.5 4B) · qwen…".
pub fn lane_words(catalogue: &Catalogue, accounts: &[Account], consent: &Consent) -> String {
    match consent::lane(consent, accounts) {
        Some(id) => {
            let account = accounts.iter().find(|a| id.starts_with(&format!("{}/", a.id))).map(|a| a.name.clone()).unwrap_or_default();
            let model = catalogue.find(&id).map(|m| m.name.clone()).unwrap_or_else(|| id.split_once('/').map(|(_, m)| m.to_string()).unwrap_or_default());
            format!("Personal things are kept on this computer by {account} · {model}.")
        }
        None => "Keeping personal things on this computer needs a model here: add Ollama or another local one, then choose it below.".into(),
    }
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
        let sharing = crate::gateway::sharing(a, consent);
        let lane = consent::lane(consent, accounts);
        let private_words = if a.local {
            "On this computer or your network: what your minds know about you never leaves it.".to_string()
        } else {
            match sharing {
                Sharing::Share => format!("Your minds share what they know about you with {}.", a.name),
                Sharing::Lane if lane.is_some() => format!("Personal things stay on this computer; everything else goes to {}.", a.name),
                Sharing::Lane => "Personal things stay on this computer, but no model here is set up for them yet, so those questions are not answered.".to_string(),
                Sharing::Off => format!("Questions with personal things are not sent to {}.", a.name),
            }
        };
        let lane_model = if a.local {
            let first = listed.and_then(|m| m.models.iter().find(|m| m.name == a.saved_model).or(m.models.first())).map(|m| m.id.clone());
            first.unwrap_or_default()
        } else {
            String::new()
        };
        out.push(Row {
            id: a.id.clone(),
            name: a.name.clone(),
            summary,
            state,
            detail,
            local: a.local,
            sharing: sharing.as_str(),
            share_words: format!("Share what your minds know about you with {}", a.name),
            private_words,
            switchable: !a.local,
            offers_share: !a.local && !a.trains_on_prompts,
            is_lane: a.local && lane.as_deref().is_some_and(|l| l.starts_with(&format!("{}/", a.id))),
            lane_model,
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
            sharing: r.sharing.into(),
            share_words: r.share_words.into(),
            private_words: r.private_words.into(),
            switchable: r.switchable,
            offers_share: r.offers_share,
            lane_model: r.lane_model.into(),
            is_lane: r.is_lane,
        })
        .collect();
    g.set_rows(ModelRc::new(VecModel::from(items)));
    g.set_summary(summary.into());
    g.set_lane(lane_words(&ai_accounts::catalogue(), &ai_accounts::accounts(), &ai_accounts::consent::load(&ai_accounts::consent::path())).into());
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
    g.on_set_sharing(move |id, choice| {
        let Some(ui) = weak.upgrade() else { return };
        let Some(sharing) = Sharing::parse(&choice) else { return };
        let Some(a) = ai_accounts::accounts().into_iter().find(|a| a.id == id.as_str()) else { return };
        // Nothing to choose for a local account; a free tier that may train is never shared with.
        if a.local || (a.trains_on_prompts && sharing == Sharing::Share) {
            return;
        }
        if let Err(e) = consent::set(&consent::path(), &a.id, sharing) {
            tracing::warn!(error = %e, "the private-context choice was not saved");
        }
        render(&ui);
    });
    let weak = ui.as_weak();
    g.on_use_as_lane(move |model| {
        let Some(ui) = weak.upgrade() else { return };
        let local = model.split_once('/').is_some_and(|(acc, _)| ai_accounts::accounts().iter().any(|a| a.id == acc && a.local));
        if local {
            if let Err(e) = consent::set_lane(&consent::path(), Some(&model)) {
                tracing::warn!(error = %e, "the private lane was not saved");
            }
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
        assert_eq!(drawn[0].sharing, "off", "nothing is shared until the person says so");
        assert_eq!(drawn[0].private_words, "Questions with personal things are not sent to openai.");
        assert_eq!(drawn[0].share_words, "Share what your minds know about you with openai");
        assert_eq!(drawn[1].state, "failed");
        assert!(drawn[1].detail.contains("refused"));
        assert!(!drawn[1].offers_share && drawn[1].switchable, "a free tier that may train: keep on this computer, or don't share");
        assert!(drawn[0].offers_share && !drawn[2].switchable);
        assert_eq!(drawn[2].sharing, "share");
        assert!(drawn[2].local && drawn[2].private_words.contains("never leaves"), "a local account keeps private context on the network");
        let consent = Consent { sharing: [("openai".to_string(), Sharing::Share)].into_iter().collect(), ..Consent::default() };
        let (_, shared) = rows(&catalogue, &[account("openai", false, false)], &consent);
        assert_eq!((shared[0].sharing, shared[0].private_words.as_str()), ("share", "Your minds share what they know about you with openai."));
        let consent = Consent { sharing: [("openai".to_string(), Sharing::Lane)].into_iter().collect(), ..Consent::default() };
        let (_, laned) = rows(&catalogue, &[account("openai", false, false)], &consent);
        assert!(laned[0].private_words.contains("no model here is set up"), "a lane chosen with none set up says so");
    }

    #[test]
    fn a_local_account_can_be_made_the_private_lane_and_the_card_says_so() {
        let mut home = account("ollama", true, false);
        home.saved_model = "qwen3:8b".into();
        let listed = vec![crate::wire::provider_models::ListedModel { id: "qwen3:8b".into(), name: "qwen3:8b".into(), size_bytes: 0 }];
        let catalogue = Catalogue { accounts: vec![build(&home, &[], Some(Ok(listed)))], at: 0 };
        let none = Consent::default();
        let (_, drawn) = rows(&catalogue, std::slice::from_ref(&home), &none);
        assert_eq!(drawn[0].lane_model, "ollama/qwen3:8b");
        assert!(!drawn[0].is_lane);
        assert!(lane_words(&catalogue, std::slice::from_ref(&home), &none).starts_with("Keeping personal things on this computer needs"));
        let lane = Consent { lane: Some("ollama/qwen3:8b".into()), ..Consent::default() };
        let (_, drawn) = rows(&catalogue, std::slice::from_ref(&home), &lane);
        assert!(drawn[0].is_lane);
        assert_eq!(lane_words(&catalogue, std::slice::from_ref(&home), &lane), "Personal things are kept on this computer by ollama · qwen3:8b.");
    }
}
