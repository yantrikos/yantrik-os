//! Whether a harness may use an account for this request: the private-context consent, and
//! Private mode. Pure, and decided before any key is read.

use yantrik_ml::model_caps::{Effort, Reasoning};

use crate::tokens::Grant;

/// The account a request goes to, as the OS resolved it at the moment of the call. Holds the key,
/// so it never prints one.
#[derive(Clone, PartialEq)]
pub struct Target {
    pub account: String,
    /// The upstream model, as the provider names it.
    pub model: String,
    /// The OpenAI-compatible base URL (`…/v1`), filled in.
    pub base_url: String,
    pub key: Option<String>,
    pub reasoning: Reasoning,
    /// The levels this model is offered at; empty when unknown or none.
    pub efforts: Vec<Effort>,
    /// On this machine or the local network.
    pub local: bool,
    /// The person allowed this account to receive private context.
    pub private_ok: bool,
}

impl std::fmt::Debug for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Target")
            .field("account", &self.account)
            .field("model", &self.model)
            .field("base_url", &self.base_url)
            .field("key", &self.key.as_ref().map(|_| "<redacted>"))
            .field("reasoning", &self.reasoning)
            .field("local", &self.local)
            .field("private_ok", &self.private_ok)
            .finish()
    }
}

/// Why a request was refused, with its HTTP status and a sentence for the harness to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub status: u16,
    pub code: &'static str,
    pub message: String,
}

impl Refusal {
    pub fn new(status: u16, code: &'static str, message: impl Into<String>) -> Refusal {
        Refusal { status, code, message: message.into() }
    }
}

/// May `grant`'s harness send this request to `target`? `private_mode`: the desktop is in Private
/// mode, when nothing goes off this machine's network.
pub fn decide(grant: &Grant, target: &Target, private_mode: bool) -> Result<(), Refusal> {
    if private_mode && !target.local {
        return Err(Refusal::new(
            403,
            "private_mode",
            format!("Private mode is on: {} is not on this machine or network, so nothing is sent to it.", target.account),
        ));
    }
    if grant.private_context && !target.local && !target.private_ok {
        return Err(Refusal::new(
            403,
            "private_context_not_allowed",
            format!(
                "{} sends your private context, and you have not allowed {} to receive it. Allow it in Settings → AI & Intelligence → AI accounts, or pick another model.",
                grant.harness, target.account
            ),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(local: bool, private_ok: bool) -> Target {
        Target {
            account: "ollama-cloud".into(),
            model: "m".into(),
            base_url: "https://ollama.com/v1".into(),
            key: Some("sk-secret".into()),
            reasoning: Reasoning::None,
            efforts: vec![],
            local,
            private_ok,
        }
    }

    fn grant(private_context: bool) -> Grant {
        Grant { harness: "mind".into(), private_context }
    }

    #[test]
    fn a_harness_with_private_context_uses_only_accounts_allowed_for_it() {
        let refused = decide(&grant(true), &target(false, false), false).unwrap_err();
        assert_eq!((refused.status, refused.code), (403, "private_context_not_allowed"));
        assert!(refused.message.contains("ollama-cloud") && refused.message.contains("Allow it"));
        assert!(decide(&grant(true), &target(false, true), false).is_ok(), "allowed by the person");
        assert!(decide(&grant(true), &target(true, false), false).is_ok(), "nothing leaves the network");
        assert!(decide(&grant(false), &target(false, false), false).is_ok(), "nothing private is sent");
    }

    #[test]
    fn private_mode_keeps_every_harness_on_this_network() {
        for g in [grant(true), grant(false)] {
            assert_eq!(decide(&g, &target(false, true), true).unwrap_err().code, "private_mode");
            assert!(decide(&g, &target(true, false), true).is_ok());
        }
    }

    #[test]
    fn a_target_never_prints_its_key() {
        assert!(!format!("{:?}", target(false, false)).contains("sk-secret"));
    }
}
