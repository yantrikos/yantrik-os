//! What runs on what, in plain words, with the exact facts kept for Details.
//!
//! `runs_on()` and `source()` are exact: "provider not reported · picked", "This network ·
//! 127.0.0.1:7460", "set in /opt/yantrik/config.yaml". Settings → AI & Intelligence and the Minds
//! cards showed them as their first lines, and the sweep of 17 October
//! (design/ui-sweep-2026-10-17.md, root cause 3) read them as what they are to a person: an
//! address, a path and a code. Every face now leads with these words — the map, the Minds cards
//! (`runs_on_card::row_line`) and the Minds quick panel (`plain_answering`, which names a custom
//! service's host because it tells the person where their messages go) — and the exact lines
//! live only in the collapsed Details (`details`), with the address shown without its sign-in
//! details, query or fragment. The chat composer's line is `destination()`, in mod.rs; the mind
//! picker (`picker::companion_runs_on`) still uses the exact `runs_on()`.

use super::identity::{is_loopback, ProviderRef};
use super::{Backend, RunsOn, SetIn, Via};

/// What a mind on the desktop's model service asks for when it means "whatever the person picked".
const PICKED: &str = "picked";

impl RunsOn {
    /// "Ollama Cloud · deepseek-v4.1-flash"; "The model you pick in the ask bar"; "A model server
    /// on this computer · qwen3.5:9b"; "It does not say what it runs on". A model's name is its
    /// name and stays; an address never leads.
    pub fn plain_runs_on(&self) -> String {
        self.plain(false)
    }

    /// `plain_runs_on`, but a service someone set up is named by its host: "A service you set up
    /// at aig.example · qwen3.5:27b". The Minds quick panel's line for the answering mind, which
    /// tells the person where their messages go, so it says where.
    pub fn plain_answering(&self) -> String {
        self.plain(true)
    }

    fn plain(&self, name_host: bool) -> String {
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
            Some(Via::Sends { lane: true }) => return format!("{}, on your private lane", plain_or_unsaid(self, model, name_host)),
            _ => {}
        }
        plain_or_unsaid(self, model, name_host)
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
        if !self.base_url.trim().is_empty() {
            // Never the URL as configured: it can carry a key (`user:key@`, `?api-key=`), and
            // these Details are drawn on screen and screenshotted (review of #844).
            match yantrik_app_runtime::place::address_of(&self.base_url) {
                shown if shown.is_empty() => lines.push("Address: not shown, as it may hold sign-in details".to_string()),
                shown => lines.push(format!("Address: {shown}")),
            }
        }
        if let Some(said) = &self.said {
            // A mind's own words can hold an address with its key; it is drawn as Address is.
            lines.push(format!("It said: {}", yantrik_app_runtime::place::without_sign_in(said)));
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

fn plain_or_unsaid(r: &RunsOn, model: Option<&str>, name_host: bool) -> String {
    let provider = match &r.provider {
        // The host is the drawable one (`identity::shown_host`); when that cannot be told, the
        // service is not named by any part of its address.
        ProviderRef::Custom(host) if name_host && !host.is_empty() => Some(format!("A service you set up at {host}")),
        other => provider_words(other),
    };
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

    /// The Details are drawn on screen (and screenshotted): a configured URL is shown without
    /// the key it can carry in its sign-in details, query or fragment (review of #844).
    #[test]
    fn details_never_show_a_key_in_the_address() {
        let companion = || MindFact { id: "companion".into(), name: "Yantrik Companion".into(), detail: None, answering: false, builtin: true, gateway: None };
        let at = |url: &str| {
            let c = CompanionFact { base_url: url.into(), model: "qwen3.5:27b".into(), source: "config.yaml".into(), ..CompanionFact::default() };
            resolve(&[companion()], Some(&c))[0].details()
        };
        let details = at("https://user:pw@aig.example/v1?api-key=sk-secret#frag");
        assert!(details.contains("Address: https://aig.example/v1"), "{details}");
        for secret in ["pw@", "user:", "api-key", "sk-secret", "#frag", "frag"] {
            assert!(!details.contains(secret), "Details show {secret}: {details}");
        }
        let odd = at("https://user:p/w@aig.example/v1");
        assert!(odd.contains("Address: not shown") && !odd.contains("p/w"), "{odd}");
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
