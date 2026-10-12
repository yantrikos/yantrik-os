//! Where a request goes: the person's private-context preference for the account it names, and
//! Private mode. Pure, and decided before any key is read.
//!
//! Private context (what a mind knows about the person: its memory, the desktop's context, a
//! handover) is a preference, not a lock (design/onboarding-one-model-one-memory-2026-10-10.md,
//! "Private context is a preference"). Per account the person chose one of three ([`Sharing`]):
//!
//! - **Share:** a call carrying it goes to the account unchanged.
//! - **Private lane:** a call carrying it goes to the private lane, a model on this machine or
//!   network, instead ([`Decision::Lane`]); a call without it still goes to the account.
//! - **Off:** only then is a call carrying it refused, and the refusal names the fix.
//!
//! Private mode is the person's "nothing leaves this machine": every call, from every harness, to
//! an account that is not local is refused.

use yantrik_ml::model_caps::{Effort, Reasoning};

use crate::tokens::Grant;

/// What the person chose for an account and private context.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sharing {
    /// "Share what your minds know about you with <provider>".
    Share,
    /// "Keep personal things on this computer": calls carrying it go to the private lane.
    Lane,
    /// "Don't share": a call carrying it is refused, with the fix named.
    #[default]
    Off,
}

impl Sharing {
    pub fn as_str(self) -> &'static str {
        match self {
            Sharing::Share => "share",
            Sharing::Lane => "lane",
            Sharing::Off => "off",
        }
    }

    pub fn parse(s: &str) -> Option<Sharing> {
        match s.trim() {
            "share" => Some(Sharing::Share),
            "lane" => Some(Sharing::Lane),
            "off" => Some(Sharing::Off),
            _ => None,
        }
    }
}

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
    /// What the person chose for this account and private context. A local account is always
    /// [`Sharing::Share`]: nothing leaves.
    pub sharing: Sharing,
    /// The account's name as the person reads it ("OpenAI"), for a refusal's fix.
    pub provider: String,
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
            .field("sharing", &self.sharing)
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

/// Where a permitted request goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// To the account it names.
    Send,
    /// To the private lane instead: it carries private context and the person keeps that on this
    /// computer for this account.
    Lane,
}

/// The refusal for a private-context call to an account set to Off (or to Private lane with no
/// lane set up): it names the fix, never fails silently.
pub fn not_shared(provider: &str) -> Refusal {
    Refusal::new(
        403,
        "private_context_not_shared",
        format!("Set up a private lane, or allow sharing with {provider} in AI accounts (Settings → AI & Intelligence → AI accounts)."),
    )
}

/// Where `grant`'s harness's request to `target` goes, or why it is refused. `private_mode`: the
/// desktop is in Private mode, when nothing goes off this machine's network.
pub fn decide(grant: &Grant, target: &Target, private_mode: bool) -> Result<Decision, Refusal> {
    if private_mode && !target.local {
        return Err(Refusal::new(
            403,
            "private_mode",
            format!("Private mode is on: {} is not on this machine or network, so nothing is sent to it.", target.account),
        ));
    }
    if !grant.private_context || target.local {
        return Ok(Decision::Send);
    }
    match target.sharing {
        Sharing::Share => Ok(Decision::Send),
        Sharing::Lane => Ok(Decision::Lane),
        Sharing::Off => Err(not_shared(&target.provider)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(local: bool, sharing: Sharing) -> Target {
        Target {
            account: "ollama-cloud".into(),
            model: "m".into(),
            base_url: "https://ollama.com/v1".into(),
            key: Some("sk-secret".into()),
            reasoning: Reasoning::None,
            efforts: vec![],
            local,
            sharing,
            provider: "Ollama Cloud".into(),
        }
    }

    fn grant(private_context: bool) -> Grant {
        Grant { harness: "mind".into(), private_context }
    }

    #[test]
    fn a_call_with_private_context_follows_the_persons_choice_for_the_account() {
        use Sharing::*;
        assert_eq!(decide(&grant(true), &target(false, Share), false), Ok(Decision::Send), "shared: unchanged");
        assert_eq!(decide(&grant(true), &target(false, Lane), false), Ok(Decision::Lane), "kept on this computer");
        let refused = decide(&grant(true), &target(false, Off), false).unwrap_err();
        assert_eq!((refused.status, refused.code), (403, "private_context_not_shared"));
        assert_eq!(
            refused.message,
            "Set up a private lane, or allow sharing with Ollama Cloud in AI accounts (Settings → AI & Intelligence → AI accounts)."
        );
        for s in [Share, Lane, Off] {
            assert_eq!(decide(&grant(true), &target(true, s), false), Ok(Decision::Send), "a local account: nothing leaves");
            assert_eq!(decide(&grant(false), &target(false, s), false), Ok(Decision::Send), "nothing private is sent");
        }
    }

    #[test]
    fn private_mode_keeps_every_harness_on_this_network() {
        for g in [grant(true), grant(false)] {
            for s in [Sharing::Share, Sharing::Lane, Sharing::Off] {
                assert_eq!(decide(&g, &target(false, s), true).unwrap_err().code, "private_mode");
                assert!(decide(&g, &target(true, s), true).is_ok());
            }
        }
    }

    #[test]
    fn sharing_reads_back_what_it_writes() {
        for s in [Sharing::Share, Sharing::Lane, Sharing::Off] {
            assert_eq!(Sharing::parse(s.as_str()), Some(s));
        }
        assert_eq!(Sharing::parse("allow"), None);
    }

    #[test]
    fn a_target_never_prints_its_key() {
        assert!(!format!("{:?}", target(false, Sharing::Off)).contains("sk-secret"));
    }
}
