//! What the picker's menus list, from what is known. Pure: the catalogue, the accounts, the
//! consent, the minds and the choice are handed in, and every row and every reason comes out.

use yantrik_ml::model_caps::ModelCaps;

use crate::ai_accounts::{account::Account, consent::Consent, store::Catalogue};

/// A mind in the Mind menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MindRow {
    pub id: String,
    pub name: String,
    pub words: String,
    pub selectable: bool,
    pub current: bool,
}

/// A mind as the menu needs it: attached ones from the host, the rest from the Harnesses list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MindSeen {
    pub id: String,
    pub name: String,
    pub attached: bool,
    pub active: bool,
    /// The Harnesses row's state words when not attached: "not installed", "ready to start".
    pub state: String,
    /// Pointed at the gateway (Use Yantrik models).
    pub on_gateway: bool,
    /// Keeps a memory of the person, so it sends private context.
    pub private_context: bool,
}

pub fn mind_rows(minds: &[MindSeen]) -> Vec<MindRow> {
    let mut rows: Vec<MindRow> = minds
        .iter()
        .map(|m| MindRow {
            id: m.id.clone(),
            name: m.name.clone(),
            words: match (m.active, m.attached, m.on_gateway) {
                (true, _, true) => "answering · Yantrik models".into(),
                (true, _, false) => "answering · its own models".into(),
                (false, true, true) => "ready · Yantrik models".into(),
                (false, true, false) => "ready · its own models".into(),
                (false, false, _) => format!("{} · set it up in Settings", m.state.to_lowercase()),
            },
            selectable: m.attached,
            current: m.active,
        })
        .collect();
    // The ones that can answer first, the one answering first of all.
    rows.sort_by_key(|r| (!r.current, !r.selectable));
    rows
}

/// A row of the Model menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelRow {
    /// "header", "model" or "note".
    pub kind: &'static str,
    pub id: String,
    pub name: String,
    pub detail: String,
    pub disabled: bool,
    pub reason: String,
    pub current: bool,
}

impl ModelRow {
    fn header(name: &str) -> ModelRow {
        ModelRow { kind: "header", id: String::new(), name: name.to_uppercase(), detail: String::new(), disabled: false, reason: String::new(), current: false }
    }
    fn note(text: String) -> ModelRow {
        ModelRow { kind: "note", id: String::new(), name: text, detail: String::new(), disabled: false, reason: String::new(), current: false }
    }
}

/// "thinks · sees images · 131K context · no tools".
pub fn caps_words(caps: &ModelCaps) -> String {
    let mut parts = Vec::new();
    if caps.thinks() {
        parts.push("thinks".to_string());
    }
    if caps.vision {
        parts.push("sees images".to_string());
    }
    if caps.context >= 1_000_000 {
        parts.push(format!("{}M context", caps.context / 1_000_000));
    } else if caps.context >= 1_000 {
        parts.push(format!("{}K context", caps.context / 1_000));
    }
    if !caps.tools {
        parts.push("no tools".to_string());
    }
    parts.join(" · ")
}

/// Why `mind` cannot use a model of `account`, or `None` when it can.
pub fn reason(mind: &MindSeen, account: &Account, consent: &Consent, private_mode: bool) -> Option<String> {
    if !mind.on_gateway {
        return Some(format!("{} uses its own models. Use Yantrik models (Settings → Harnesses) to pick here.", mind.name));
    }
    if private_mode && !account.local {
        return Some("Private mode: only models on this machine or network.".into());
    }
    if mind.private_context && !crate::gateway::private_ok(account, consent) {
        return Some(if account.trains_on_prompts {
            format!("{} keeps a memory of you, and this free tier may train on what it is sent.", mind.name)
        } else {
            format!("{} keeps a memory of you; allow private context for {} under AI accounts to use it.", mind.name, account.name)
        });
    }
    None
}

/// Whether a model matches what was typed in the search: every word, in its name, id or account.
fn matches(query: &str, haystack: &str) -> bool {
    let hay = haystack.to_lowercase();
    query.split_whitespace().all(|w| hay.contains(&w.to_lowercase()))
}

/// The Model menu: recent picks first, then each account's models under its name; models the
/// mind cannot use disabled with the reason. `query` narrows it.
#[allow(clippy::too_many_arguments)]
pub fn model_rows(
    mind: &MindSeen,
    catalogue: &Catalogue,
    accounts: &[Account],
    consent: &Consent,
    private_mode: bool,
    current: &str,
    recent: &[String],
    query: &str,
) -> Vec<ModelRow> {
    let mut out = Vec::new();
    if catalogue.models().next().is_none() {
        out.push(ModelRow::note("No models yet. Add an account under Settings → AI & Intelligence, or switch on a free one.".into()));
        return out;
    }
    if !mind.on_gateway {
        out.push(ModelRow::note(format!("{} uses its own models. Use Yantrik models on its row in Settings → Harnesses to pick from these.", mind.name)));
    }
    let row = |m: &crate::ai_accounts::models::CatalogueModel, a: &Account| {
        let why = reason(mind, a, consent, private_mode);
        ModelRow {
            kind: "model",
            id: m.id.clone(),
            name: m.name.clone(),
            detail: [a.name.clone(), caps_words(&m.caps)].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · "),
            disabled: why.is_some(),
            reason: why.unwrap_or_default(),
            current: m.id == current,
        }
    };
    let find = |id: &str| {
        let m = catalogue.find(id)?;
        let a = accounts.iter().find(|a| a.id == m.account)?;
        Some((m, a))
    };
    let searching = !query.trim().is_empty();
    if !searching {
        let recent: Vec<_> = recent.iter().filter_map(|id| find(id)).take(super::choices::RECENT).collect();
        if !recent.is_empty() {
            out.push(ModelRow::header("Recent"));
            out.extend(recent.into_iter().map(|(m, a)| row(m, a)));
        }
    }
    for group in &catalogue.accounts {
        let Some(a) = accounts.iter().find(|a| a.id == group.account) else { continue };
        let models: Vec<_> = group
            .models
            .iter()
            .filter(|m| !searching || matches(query, &format!("{} {} {}", m.name, m.id, a.name)))
            .collect();
        if models.is_empty() {
            continue;
        }
        out.push(ModelRow::header(&a.name));
        out.extend(models.into_iter().map(|m| row(m, a)));
    }
    if searching && !out.iter().any(|r| r.kind == "model") {
        out.push(ModelRow::note(format!("No model matches \u{201c}{}\u{201d}.", query.trim())));
    }
    out
}
