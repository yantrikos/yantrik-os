//! What runs on what, in plain words, with the exact facts kept for Details.
//!
//! `runs_on()` and `source()` are exact: "provider not reported · picked", "This network ·
//! 127.0.0.1:7460", "set in /opt/yantrik/config.yaml". They stay what the harness rows' long line
//! and the chat composer are built from. Settings → AI & Intelligence showed them as the map's
//! first lines, and the sweep of 17 October (design/ui-sweep-2026-10-17.md, root cause 3) read
//! them as what they are to a person: an address, a path and a code. The map now leads with
//! these words, and puts the exact ones in its collapsed Details.

use super::identity::{is_loopback, ProviderRef};
use super::{Backend, RunsOn, SetIn, Via};

/// What a mind on the desktop's model service asks for when it means "whatever the person picked".
const PICKED: &str = "picked";

impl RunsOn {
    /// "Ollama Cloud · deepseek-v4.1-flash"; "The model you pick in the ask bar"; "A model server
    /// on this computer · qwen3.5:9b"; "It does not say what it runs on". A model's name is its
    /// name and stays; an address never leads.
    pub fn plain_runs_on(&self) -> String {
        let model = self.model.as_deref().map(str::trim).filter(|m| !m.is_empty());
        let with = |what: &str| match model {
            Some(m) if m == PICKED => format!("{what}: the model you pick in the ask bar"),
            Some(m) => format!("{what} \u{b7} {m}"),
            None => what.to_string(),
        };
        match self.backend {
            Backend::ClaudeCli => return with("Anthropic\u{2019}s Claude"),
            Backend::InProcess => return with("This computer"),
            Backend::Api => {}
        }
        if self.set_in == SetIn::Nowhere {
            return "Nothing set up yet".to_string();
        }
        match &self.via_gateway {
            Some(Via::NoPick) => return "No model picked yet".to_string(),
            Some(Via::Refused { .. }) => return "Nothing is sent: this desktop holds it back".to_string(),
            Some(Via::Sends { lane: true }) => return format!("{}, on your private lane", plain_or_unsaid(self, model)),
            _ => {}
        }
        plain_or_unsaid(self, model)
    }

    /// Where it is set, as a sentence: "Set in its own settings; this is what it says".
    pub fn plain_source(&self) -> String {
        let place = match &self.set_in {
            SetIn::ConfigYaml => "Set in this computer\u{2019}s setup file".to_string(),
            SetIn::SavedProvider(name) => format!("Uses your saved provider \u{201c}{name}\u{201d}"),
            SetIn::OwnSettings => "Set in its own settings".to_string(),
            SetIn::Gateway => "Chosen on this desktop".to_string(),
            SetIn::Nowhere => "Add an AI account below to give it one".to_string(),
        };
        if self.reported {
            format!("{place}; this is what it says")
        } else {
            place
        }
    }

    /// The exact facts, one per line, for the collapsed Details: the address, the file, the
    /// mind's own words, and the full technical line. Empty when there is nothing more to say.
    pub fn details(&self) -> String {
        let mut lines = Vec::new();
        lines.push(format!("{}: {} \u{b7} {}", self.name, self.runs_on(), self.source()));
        if !self.base_url.is_empty() {
            lines.push(format!("Address: {}", self.base_url));
        }
        if let Some(said) = &self.said {
            lines.push(format!("It said: {said}"));
        }
        if let Some(fb) = &self.fallback {
            lines.push(format!("Falls back to: {fb}"));
        }
        if let Some(Via::Refused { code, message }) = &self.via_gateway {
            lines.push(format!("Refused ({code}): {message}"));
        }
        lines.join("\n")
    }
}

/// A provider in words, never by its address: "Ollama Cloud", "A service you set up", "A model
/// server on this computer". `None` when nothing said which.
pub(super) fn provider_words(provider: &ProviderRef) -> Option<String> {
    match provider {
        ProviderRef::Known(p) => Some(p.display_name.to_string()),
        ProviderRef::Custom(_) => Some("A service you set up".to_string()),
        ProviderRef::Local(host) if is_loopback(host) => Some("A model server on this computer".to_string()),
        ProviderRef::Local(_) => Some("A model server on your network".to_string()),
        ProviderRef::NotReported => None,
    }
}

fn plain_or_unsaid(r: &RunsOn, model: Option<&str>) -> String {
    let provider = provider_words(&r.provider);
    match (provider, model) {
        (Some(p), Some(m)) if m == PICKED => format!("{p}: the model you pick in the ask bar"),
        (Some(p), Some(m)) => format!("{p} \u{b7} {m}"),
        (Some(p), None) => p,
        (None, Some(m)) if m == PICKED => "The model you pick in the ask bar".to_string(),
        (None, Some(m)) => format!("{m}; it does not say from where"),
        (None, None) => "It does not say what it runs on".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::{resolve, CompanionFact, MindFact};

    fn mind(detail: &str) -> MindFact {
        MindFact { id: "hermes".into(), name: "Hermes Agent".into(), detail: Some(detail.into()), answering: true, builtin: false, gateway: None }
    }

    #[test]
    fn the_map_leads_with_words_and_keeps_addresses_and_paths_for_details() {
        let picked = &resolve(&[mind("Hermes 0.21.5 \u{b7} picked")], None)[0];
        assert_eq!(picked.runs_on(), "provider not reported \u{b7} picked", "the exact line is unchanged");
        assert_eq!(picked.plain_runs_on(), "The model you pick in the ask bar");
        assert_eq!(picked.plain_source(), "Set in its own settings; this is what it says");
        assert!(picked.details().contains("provider not reported \u{b7} picked"), "the exact line is in Details");
        assert!(picked.details().contains("It said: Hermes 0.21.5 \u{b7} picked"));

        let local = &resolve(&[mind("picked \u{b7} 127.0.0.1:7460")], None)[0];
        assert_eq!(local.plain_runs_on(), "A model server on this computer: the model you pick in the ask bar");
        assert!(!local.plain_runs_on().contains("127.0.0.1"));

        let cloud = &resolve(&[mind("ollama-cloud:deepseek-v4.1-flash")], None)[0];
        assert_eq!(cloud.plain_runs_on(), "Ollama Cloud \u{b7} deepseek-v4.1-flash");

        let companion = MindFact { id: "companion".into(), name: "Yantrik Companion".into(), detail: None, answering: false, builtin: true, gateway: None };
        let c = CompanionFact { base_url: "http://127.0.0.1:8341/v1".into(), model: "qwen3.5:9b".into(), source: "config.yaml".into(), ..CompanionFact::default() };
        let row = &resolve(&[companion], Some(&c))[0];
        assert_eq!(row.plain_source(), "Set in this computer\u{2019}s setup file");
        assert!(!row.plain_runs_on().contains("127.0.0.1") && !row.plain_source().contains("/opt/"));
        assert!(row.details().contains("Address: http://127.0.0.1:8341/v1") && row.details().contains("/opt/yantrik/config.yaml"));
    }

    /// Settings names minds "Minds" (the vocabulary rule), and keeps addresses and paths off the
    /// face of the AI page and the Minds cards. Internal identifiers (category 8, `harness-rows`)
    /// stay as they are.
    #[test]
    fn settings_says_minds_and_keeps_paths_behind_details() {
        let settings = include_str!("../../../yantrik-ui-slint/ui/settings.slint");
        let code: String = settings.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
        for gone in ["label: \"Harnesses\"", "label:\"Harnesses\"", "\"HARNESSES\"", "handed to harnesses", "A harness brings", "set in /opt/yantrik/config.yaml", "/opt/yantrik/share/harnesses"] {
            assert!(!code.contains(gone), "settings.slint still shows {gone}");
        }
        assert!(code.contains("for r in root.runs-on-rows : Text {\n") || code.contains("text: r.details;"), "the map's exact facts are in its Details");
        let card = include_str!("../../../yantrik-ui-slint/ui/components/ai_accounts_card.slint");
        assert!(!card.lines().filter(|l| !l.trim_start().starts_with("//")).any(|l| l.contains("token")), "no token in the AI accounts card's words");
        let rust = include_str!("../wire/settings.rs");
        assert!(rust.contains("label: \"Minds\".into()") && !rust.contains("label: \"Harnesses\".into()"));
    }
}
