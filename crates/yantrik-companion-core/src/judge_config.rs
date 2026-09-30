//! `judge:` — a System One model that makes the companion's small decisions instead of the
//! chat model.
//!
//! A judge answers a typed question (which of these tools fits this request?) with a probability
//! for each option, in one pass and without writing text. Asking it which tool a request needs lets
//! the chat model see that one tool's schema instead of a shortlist of twenty, which saves the
//! tokens of every schema left out and stops the chat model picking the wrong one. Anything the
//! judge is unsure of, or any failure to reach it, falls back to the ordinary selection.
//!
//! The judge reads each request and the person's last few messages, never the assistant's
//! replies, with anything shaped like a credential redacted. A cloud judge therefore receives
//! that text; in incognito the companion asks no judge at all.
//!
//! Choosing tools is one use of it. Every use is listed in `JUDGE_USES`, with what it sends,
//! and has its own switch in Settings (`uses:`); a new use is a row there and its caller.
//!
//! Any server speaking `/v1/systemone` works:
//!
//! ```yaml
//! judge:                                   # TypeSafe's cloud
//!   endpoint: "https://api.typesafe.ai"
//!   model: "jev-latest"
//!   api_key_env: "JEV_API_KEY"
//!
//! judge:                                   # Kev on this machine (kev.serve)
//!   endpoint: "http://127.0.0.1:8009"
//!   model: "kev-latest"
//! ```

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JudgeConfig {
    /// Which decision model answers: `off`, `jev`, `kev`, `laya`, `jeff`, `systemone` (any other
    /// `/v1/systemone` server) or `chat_model` (the configured chat model answers the same typed
    /// questions). Empty is the older form of this section: a System One server when `endpoint`
    /// is set, otherwise off.
    #[serde(default)]
    pub provider: String,
    /// The server (its `/v1/systemone` path is added). Empty: no judge, the default.
    #[serde(default)]
    pub endpoint: String,
    /// The model the server should answer with.
    #[serde(default = "default_model")]
    pub model: String,
    /// The environment variable that holds the key, for servers that need one. The key itself
    /// never goes in this file.
    #[serde(default)]
    pub api_key_env: String,
    /// How long a decision may take before the companion goes on without it.
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    /// How sure the judge must be of its pick for the companion to follow it. At 0.7 Kev-4B was
    /// right 98% of the time on the 243-tool catalogue, and that sure for 69% of requests.
    #[serde(default = "default_route_at")]
    pub route_at: f64,
    /// How many tools, closest by meaning, the judge chooses among.
    #[serde(default = "default_shortlist")]
    pub shortlist: usize,
    /// Which uses (`JUDGE_USES`) are on, by id. A use not named is on: the person turns uses off.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub uses: BTreeMap<String, bool>,
    /// The older spelling of `uses.route_tools`: read, kept until that use is switched again.
    #[serde(default, rename = "route_tools", skip_serializing_if = "Option::is_none")]
    pub legacy_route_tools: Option<bool>,
    /// The older spelling of `uses.browser_commitment`.
    #[serde(default, rename = "browser_commitments", skip_serializing_if = "Option::is_none")]
    pub legacy_browser_commitments: Option<bool>,
}

/// One use of the decision model: something in the OS that asks it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JudgeUse {
    /// Its id in `uses:` and, for a use with a door, the `purpose` a caller names.
    pub id: &'static str,
    /// What Settings says it does.
    pub label: &'static str,
    /// What it sends the model, in the person's words: what a cloud model would receive.
    pub sends: &'static str,
    /// Whether the shell's `decide` serves it, for surfaces in other processes. Tool choice has
    /// none: it is asked inside the companion.
    pub door: bool,
    /// Whether an agent may ask for it. Only `agent`, and only of a model on this machine or the
    /// home network (`yantrik_companion::decisions`).
    pub for_agents: bool,
}

/// Every use of the decision model.
pub const JUDGE_USES: &[JudgeUse] = &[
    JudgeUse {
        id: "route_tools",
        label: "Choose the tool a request needs",
        sends: "your request and your last few messages (never the assistant's replies)",
        door: false,
        for_agents: false,
    },
    JudgeUse {
        id: "browser_commitment",
        label: "Spot purchases, sends and deletes in the browser (adds a card, never removes one)",
        sends: "the button, its page's title and address, and the text around it",
        door: true,
        for_agents: false,
    },
    JudgeUse {
        id: "agent",
        label: "Answer agents' quick questions, such as the Mind's (only a model on this machine or your home network)",
        sends: "what the agent asks about",
        door: true,
        for_agents: true,
    },
];

/// The use named `id`.
pub fn judge_use(id: &str) -> Option<&'static JudgeUse> {
    JUDGE_USES.iter().find(|u| u.id == id)
}

/// What kind of decision model a configuration names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JudgeKind {
    Off,
    /// A `/v1/systemone` server; the name is its dialect (`jev`, `kev`, `laya`, `jeff`, `systemone`).
    SystemOne(&'static str),
    ChatModel,
}

/// The providers Settings offers, and what each fills in. `label` is what the person reads.
pub const PRESETS: &[(&str, &str, &str, &str, &str)] = &[
    // (provider, label, endpoint, model, key variable)
    ("off", "Off: no decision model", "", "", ""),
    ("kev", "Kev (on this machine or the home GPU box)", "http://127.0.0.1:8009", "kev-latest", ""),
    ("laya", "Laya (small, runs on this machine's CPU)", "http://127.0.0.1:8000", "laya", ""),
    ("jeff", "Jeff (on this machine)", "http://127.0.0.1:8765", "jeff-latest", ""),
    ("jev", "Jev (TypeSafe cloud: what is judged leaves this machine)", "https://api.typesafe.ai", "jev-latest", "JEV_API_KEY"),
    ("systemone", "Another System One server", "", "", ""),
    ("chat_model", "The chat model (slower, uncalibrated)", "", "", ""),
];

fn default_model() -> String { "kev-latest".to_string() }
fn default_timeout_ms() -> u64 { 2000 }
fn default_true() -> bool { true }
fn default_route_at() -> f64 { 0.7 }
fn default_shortlist() -> usize { 20 }

impl Default for JudgeConfig {
    fn default() -> Self {
        Self {
            endpoint: String::new(),
            model: default_model(),
            api_key_env: String::new(),
            timeout_ms: default_timeout_ms(),
            route_at: default_route_at(),
            shortlist: default_shortlist(),
            provider: String::new(),
            uses: BTreeMap::new(),
            legacy_route_tools: None,
            legacy_browser_commitments: None,
        }
    }
}

impl JudgeConfig {
    /// What kind of decision model this names. An unknown provider is off, never a guess.
    pub fn kind(&self) -> JudgeKind {
        let has_endpoint = !self.endpoint.trim().is_empty();
        match self.provider.trim().to_ascii_lowercase().as_str() {
            "" if has_endpoint => JudgeKind::SystemOne("systemone"),
            "" | "off" => JudgeKind::Off,
            "chat_model" => JudgeKind::ChatModel,
            p => match PRESETS.iter().find(|(name, ..)| *name == p) {
                Some((name, ..)) if has_endpoint && !matches!(*name, "off" | "chat_model") => JudgeKind::SystemOne(name),
                _ => JudgeKind::Off,
            },
        }
    }

    /// Whether a judge is configured at all.
    pub fn enabled(&self) -> bool {
        self.kind() != JudgeKind::Off
    }

    /// Whether the use `id` is on for a model that runs in the cloud (`cloud`) or in the house.
    /// What the person switched decides; unswitched, a use is on in the house, and in the cloud
    /// only tool choice is: a model somebody set up to choose tools does not quietly start
    /// receiving the pages they browse (security review, 29 Sep 2026).
    pub fn use_on_where(&self, id: &str, cloud: bool) -> bool {
        if judge_use(id).is_none() {
            return false;
        }
        if self.uses.contains_key(id) || self.legacy_of(id).is_some() {
            return self.use_on(id);
        }
        id == "route_tools" || !cloud
    }

    fn legacy_of(&self, id: &str) -> Option<bool> {
        match id {
            "route_tools" => self.legacy_route_tools,
            "browser_commitment" => self.legacy_browser_commitments,
            _ => None,
        }
    }

    /// Whether the use `id` is on, wherever the model runs: what the person switched, or on.
    /// An unknown use is off: nothing asks the model for a use the person was never shown.
    pub fn use_on(&self, id: &str) -> bool {
        if judge_use(id).is_none() {
            return false;
        }
        if let Some(on) = self.uses.get(id) {
            return *on;
        }
        let legacy = match id {
            "route_tools" => self.legacy_route_tools,
            "browser_commitment" => self.legacy_browser_commitments,
            _ => None,
        };
        legacy.unwrap_or(true)
    }

    /// Switch the use `id`, from Settings; its older spelling goes.
    pub fn set_use(&mut self, id: &str, on: bool) {
        if judge_use(id).is_none() {
            return;
        }
        match id {
            "route_tools" => self.legacy_route_tools = None,
            "browser_commitment" => self.legacy_browser_commitments = None,
            _ => {}
        }
        self.uses.insert(id.to_string(), on);
    }

    /// A preset's configuration, keeping this one's policy fields (thresholds, which uses are on).
    pub fn with_preset(&self, provider: &str) -> JudgeConfig {
        let mut out = self.clone();
        if let Some((name, _, endpoint, model, key)) = PRESETS.iter().find(|(name, ..)| *name == provider) {
            out.provider = name.to_string();
            out.endpoint = endpoint.to_string();
            out.model = model.to_string();
            out.api_key_env = key.to_string();
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_provider_names_the_kind_and_an_unknown_one_is_off() {
        let kev = JudgeConfig::default().with_preset("kev");
        assert_eq!(kev.kind(), JudgeKind::SystemOne("kev"));
        assert_eq!(kev.endpoint, "http://127.0.0.1:8009");
        assert_eq!(JudgeConfig::default().with_preset("chat_model").kind(), JudgeKind::ChatModel);
        assert_eq!(JudgeConfig::default().with_preset("off").kind(), JudgeKind::Off);
        let odd = JudgeConfig { provider: "gpt-judge-9000".into(), endpoint: "http://x".into(), ..JudgeConfig::default() };
        assert_eq!(odd.kind(), JudgeKind::Off);
        let no_endpoint = JudgeConfig { provider: "kev".into(), ..JudgeConfig::default() };
        assert_eq!(no_endpoint.kind(), JudgeKind::Off, "a server with no address is not a judge");
        let jev = JudgeConfig::default().with_preset("jev");
        assert_eq!(jev.api_key_env, "JEV_API_KEY");
    }

    #[test]
    fn a_preset_keeps_the_policy_it_was_chosen_under() {
        let mut c = JudgeConfig::default();
        c.route_at = 0.9;
        c.set_use("browser_commitment", false);
        let laya = c.with_preset("laya");
        assert_eq!((laya.route_at, laya.use_on("browser_commitment")), (0.9, false));
    }

    #[test]
    fn every_use_is_on_until_the_person_turns_it_off() {
        let mut c = JudgeConfig::default();
        assert!(JUDGE_USES.iter().all(|u| c.use_on(u.id)));
        assert!(!c.use_on("mail_everyone"), "a use nobody was shown is off");
        c.set_use("mail_everyone", true);
        assert!(!c.use_on("mail_everyone") && c.uses.is_empty(), "and cannot be switched on");
        c.set_use("agent", false);
        assert!(!c.use_on("agent") && c.use_on("route_tools"));
    }

    #[test]
    fn a_cloud_model_gets_only_tool_choice_until_the_person_says_otherwise() {
        let mut c = JudgeConfig::default();
        assert!(c.use_on_where("route_tools", true));
        assert!(!c.use_on_where("browser_commitment", true) && !c.use_on_where("agent", true));
        assert!(c.use_on_where("browser_commitment", false) && c.use_on_where("agent", false));
        c.set_use("browser_commitment", true);
        assert!(c.use_on_where("browser_commitment", true), "switched on, it is on in the cloud too");
        c.set_use("route_tools", false);
        assert!(!c.use_on_where("route_tools", true) && !c.use_on_where("route_tools", false));
    }

    #[test]
    fn the_older_switches_are_read_and_kept_until_switched_again() {
        let c: JudgeConfig = serde_yaml::from_str("provider: kev\nroute_tools: false\nbrowser_commitments: false").unwrap();
        assert!(!c.use_on("route_tools") && !c.use_on("browser_commitment") && c.use_on("agent"));
        let again: JudgeConfig = serde_yaml::from_str(&serde_yaml::to_string(&c).unwrap()).unwrap();
        assert!(!again.use_on("route_tools"), "saving does not turn an old switch back on");
        let mut c = again;
        c.set_use("route_tools", true);
        let yaml = serde_yaml::to_string(&c).unwrap();
        assert!(!yaml.contains("route_tools: false") && c.use_on("route_tools"), "{yaml}");
    }

    #[test]
    fn only_agent_is_for_agents_and_tool_choice_has_no_door() {
        assert_eq!(JUDGE_USES.iter().filter(|u| u.for_agents).map(|u| u.id).collect::<Vec<_>>(), ["agent"]);
        assert!(!judge_use("route_tools").unwrap().door);
        assert!(JUDGE_USES.iter().all(|u| !u.sends.is_empty() && !u.label.is_empty()));
    }

    #[test]
    fn no_judge_unless_an_endpoint_is_named() {
        assert!(!JudgeConfig::default().enabled());
        let c: JudgeConfig = serde_yaml::from_str("endpoint: \"http://127.0.0.1:8009\"").unwrap();
        assert!(c.enabled());
        assert_eq!(c.model, "kev-latest");
        assert!(c.use_on("route_tools"));
        assert_eq!(c.route_at, 0.7);
    }
}
