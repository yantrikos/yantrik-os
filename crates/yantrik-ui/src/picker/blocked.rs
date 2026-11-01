//! When every model in the Model menu is held back for the same reason (#1042), the menu says it
//! once at the top, in plain words, with the one press that fixes it, instead of greying out every
//! row with no way to tell why or what to do. With mixed reasons there is no header and each row
//! keeps its own. Pure: what is known is handed in, and the rows come out.

use super::menu::{why, MindSeen, ModelRow, Why};
use crate::ai_accounts::{account::Account, consent::{self, Consent}, store::Catalogue};

/// The header's presses (`ModelRow::id` of an "action" row). Pressing one travels the path a
/// disabled row's press does (`fix-model` with the reason, `picker::section_for_reason`), and the
/// reason an action carries names AI accounts: the one place that holds the sharing choice and the private lane. No second consent path.
pub const ALLOW_SHARING: &str = "allow-sharing";
pub const PRIVATE_LANE: &str = "private-lane";
/// What both presses carry as their reason; it names AI accounts, so `section_for_reason` opens it.
pub const OPENS_AI_ACCOUNTS: &str = "Opens AI accounts, where sharing and the private lane are set.";

/// What holds back every model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Blocker {
    PrivateMode,
    /// Sharing is off for these accounts (names, in menu order); `lane_missing`: no private lane.
    SharingOff { accounts: Vec<String>, lane_missing: bool },
}

/// The one reason every model is disabled, when there is one and the menu can act on it.
pub fn blocker(mind: &MindSeen, catalogue: &Catalogue, accounts: &[Account], consent: &Consent, private_mode: bool) -> Option<Blocker> {
    let mut kind: Option<Why> = None;
    let mut names: Vec<String> = Vec::new();
    for m in catalogue.models() {
        let a = accounts.iter().find(|a| a.id == m.account)?;
        let w = why(mind, a, consent, private_mode)?;
        if *kind.get_or_insert(w) != w {
            return None;
        }
        if !names.contains(&a.name) {
            names.push(a.name.clone());
        }
    }
    match kind? {
        // The menu already opens with its own note for a mind that keeps its own models.
        Why::OwnModels => None,
        Why::PrivateMode => Some(Blocker::PrivateMode),
        Why::SharingOff => Some(Blocker::SharingOff { accounts: names, lane_missing: consent::lane(consent, accounts).is_none() }),
    }
}

impl Blocker {
    /// The header: the reason once, then its presses.
    pub fn rows(&self) -> Vec<ModelRow> {
        match self {
            Blocker::PrivateMode => vec![ModelRow::banner("Private mode is on: only models on this machine or network can be used, and none is set up.".into())],
            Blocker::SharingOff { accounts, lane_missing } => {
                let whom = match accounts.as_slice() {
                    [one] => one.clone(),
                    _ => "every account".into(),
                };
                let mut rows = vec![ModelRow::banner(format!(
                    "Sharing is off for {whom}, so no model can be used yet. Your Mind sends what it knows about you only where you allow it."
                ))];
                let label = match accounts.as_slice() {
                    [one] => format!("Allow sharing with {one}\u{2026}"),
                    _ => "Allow sharing\u{2026}".into(),
                };
                rows.push(ModelRow::action(ALLOW_SHARING, label, OPENS_AI_ACCOUNTS.into()));
                if *lane_missing {
                    rows.push(ModelRow::action(PRIVATE_LANE, "Set up a private lane".into(), OPENS_AI_ACCOUNTS.into()));
                }
                rows
            }
        }
    }
}
