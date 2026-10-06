//! The models of providers whose API has no model list, so the picker has something true to show.
//!
//! Every other provider is asked (`/v1/models`, or Ollama's `/api/tags`): its own list is the only
//! one that stays current. These answer 404 there, so their lists are kept by hand, short, and
//! only of models their own documentation names. A provider missing here and missing a list shows
//! its saved model alone.

/// One hand-kept model: its id, and the context its documentation states (0: not stated).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CuratedModel {
    pub id: &'static str,
    pub context: u32,
}

const fn m(id: &'static str, context: u32) -> CuratedModel {
    CuratedModel { id, context }
}

/// Providers with no model-list endpoint, and their models.
pub const CURATED: &[(&str, &[CuratedModel])] = &[
    ("perplexity", &[m("sonar", 127_072), m("sonar-pro", 200_000), m("sonar-reasoning-pro", 127_072), m("sonar-deep-research", 127_072)]),
    ("minimax", &[m("MiniMax-M3", 1_000_000), m("MiniMax-M2", 204_800)]),
    ("baidu", &[m("ernie-4.5-turbo-128k", 131_072), m("ernie-x1-turbo-32k", 32_768)]),
];

/// The hand-kept list for this provider, when it has no list of its own.
pub fn curated(provider_type: &str) -> Option<&'static [CuratedModel]> {
    CURATED.iter().find(|(id, _)| *id == provider_type).map(|(_, list)| *list)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_provider_without_a_list_has_a_short_true_one_and_others_have_none() {
        let p = curated("perplexity").unwrap();
        assert!(p.iter().any(|m| m.id == "sonar"));
        assert!(curated("openai").is_none(), "a provider with a list is asked, not remembered");
        for (_, list) in CURATED {
            assert!(!list.is_empty() && list.len() <= 8);
            let mut ids: Vec<_> = list.iter().map(|m| m.id).collect();
            ids.sort();
            ids.dedup();
            assert_eq!(ids.len(), list.len(), "no model twice");
        }
    }
}
