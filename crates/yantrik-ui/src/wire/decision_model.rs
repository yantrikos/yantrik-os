//! Settings → AI & Intelligence → Decision model: which model makes the desktop's quick
//! judgements, or none.
//!
//! The choice is the companion's `judge:` configuration, switched at runtime through the bridge
//! and saved to its config, so there is one place it lives. Every model answers in the same
//! verdict (`yantrik_ml::judge::Verdict`), which is why switching here changes nothing else.
//!
//! What this screen never takes is a key: only the name of the environment variable that holds
//! one. A value that looks like a key is refused, so it is not written into a config file.

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use yantrik_companion::config::{JudgeConfig, JudgeKind, JUDGE_PRESETS as PRESETS, JUDGE_USES};
use yantrik_ml::judge::{Answer, Locality};

use crate::app_context::AppContext;
use crate::{App, DecisionPreset, DecisionUse};

/// The provider id Settings shows as chosen for a configuration.
fn provider_of(config: &JudgeConfig) -> &'static str {
    match config.kind() {
        JudgeKind::Off => "off",
        JudgeKind::ChatModel => "chat_model",
        JudgeKind::SystemOne(dialect) => PRESETS.iter().find(|(id, ..)| *id == dialect).map(|(id, ..)| *id).unwrap_or("systemone"),
    }
}

/// Where the chosen model runs: a server's address, or for `chat_model` wherever the chat model
/// runs (`chat_at`, published by the companion). Off is nowhere.
fn locality_of(config: &JudgeConfig, chat_at: Locality) -> Locality {
    match config.kind() {
        JudgeKind::Off => Locality::Nowhere,
        JudgeKind::ChatModel => chat_at,
        JudgeKind::SystemOne(_) => Locality::of_endpoint(&config.endpoint),
    }
}

/// Where the chosen model runs, in the person's words, and whether what it judges leaves the
/// house.
fn where_note(config: &JudgeConfig, chat_at: Locality) -> (String, bool) {
    let chat = config.kind() == JudgeKind::ChatModel;
    let tail = if chat { " Slower, and its numbers are not calibrated." } else { "" };
    match locality_of(config, chat_at) {
        Locality::Nowhere => (String::new(), false),
        Locality::ThisMachine if chat => (format!("Uses your chat model, which runs on this machine: nothing it judges leaves it.{tail}"), false),
        Locality::ThisMachine => ("Runs on this machine: nothing it judges leaves it.".into(), false),
        Locality::Home if chat => (format!("Uses your chat model, which runs on a machine on your home network.{tail}"), false),
        Locality::Home => ("Runs on a machine on your home network.".into(), false),
        Locality::Cloud if chat => (format!("Uses your chat model, which runs in the cloud: what it judges is sent there.{tail}"), true),
        Locality::Cloud => ("Runs in the cloud: what it judges (a request, a button and its page) is sent there.".into(), true),
    }
}

/// Whether a key-variable field holds the NAME of a variable, as it must, and not a key.
fn is_variable_name(text: &str) -> bool {
    let t = text.trim();
    t.is_empty()
        || (t.len() <= 64
            && t.chars().next().is_some_and(|c| c.is_ascii_uppercase() || c == '_')
            && t.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))
}

fn push(ui: &App, config: &JudgeConfig, chat_at: Locality) {
    let presets: Vec<DecisionPreset> = PRESETS
        .iter()
        .map(|(id, label, ..)| DecisionPreset { id: SharedString::from(*id), label: SharedString::from(*label) })
        .collect();
    ui.set_settings_decision_presets(ModelRc::new(VecModel::from(presets)));
    ui.set_settings_decision_provider(provider_of(config).into());
    ui.set_settings_decision_endpoint(config.endpoint.as_str().into());
    ui.set_settings_decision_model(config.model.as_str().into());
    ui.set_settings_decision_key_env(config.api_key_env.as_str().into());
    let cloud = locality_of(config, chat_at) == Locality::Cloud;
    let uses: Vec<DecisionUse> = JUDGE_USES
        .iter()
        .map(|u| DecisionUse { id: u.id.into(), label: u.label.into(), sends: u.sends.into(), on: config.use_on_where(u.id, cloud) })
        .collect();
    ui.set_settings_decision_uses(ModelRc::new(VecModel::from(uses)));
    let (note, leaves) = where_note(config, chat_at);
    ui.set_settings_decision_where(note.into());
    ui.set_settings_decision_leaves_machine(leaves);
}

fn status(ui: &App, text: &str, good: bool) {
    ui.set_settings_decision_status(text.into());
    ui.set_settings_decision_status_good(good);
}

pub fn wire(ui: &App, ctx: &AppContext) {
    push(ui, &ctx.bridge.judge_config(), ctx.bridge.decisions().chat_locality());

    let bridge = ctx.bridge.clone();
    let weak = ui.as_weak();
    ui.on_decision_choose(move |id| {
        let config = bridge.judge_config().with_preset(id.as_str());
        bridge.set_judge(config.clone());
        if let Some(ui) = weak.upgrade() {
            push(&ui, &config, bridge.decisions().chat_locality());
            status(&ui, "", false);
        }
    });

    let bridge = ctx.bridge.clone();
    let weak = ui.as_weak();
    ui.on_decision_save(move |endpoint, model, key_env| {
        let Some(ui) = weak.upgrade() else { return };
        if !is_variable_name(&key_env) {
            status(&ui, "Enter the name of the environment variable that holds the key (like JEV_API_KEY), not the key itself. Nothing was saved.", false);
            return;
        }
        let mut config = bridge.judge_config();
        config.endpoint = endpoint.trim().to_string();
        config.model = model.trim().to_string();
        config.api_key_env = key_env.trim().to_string();
        bridge.set_judge(config.clone());
        push(&ui, &config, bridge.decisions().chat_locality());
        status(&ui, "Saved.", true);
    });

    let bridge = ctx.bridge.clone();
    let weak = ui.as_weak();
    ui.on_decision_test(move || {
        if let Some(ui) = weak.upgrade() {
            status(&ui, "testing", false);
        }
        let bridge = bridge.clone();
        let weak = weak.clone();
        std::thread::spawn(move || {
            let (text, good) = match bridge.test_judge() {
                Err(why) => (format!("No answer: {why}"), false),
                Ok(v) => match v.get("commit") {
                    Some(Answer::Noul(p)) => (
                        format!("Answered in {} ms ({} {}): \"Place your order\" is a commitment, p = {:.2}{}",
                                v.latency_ms, v.by.provider, v.by.model, p,
                                if *p < 0.5 { ". That should be close to 1: check the model." } else { "." }),
                        *p >= 0.5,
                    ),
                    _ => ("It answered, but not with a yes/no.".to_string(), false),
                },
            };
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = weak.upgrade() {
                    status(&ui, &text, good);
                }
            });
        });
    });

    let bridge = ctx.bridge.clone();
    let weak = ui.as_weak();
    ui.on_decision_toggle_use(move |id| {
        let mut config = bridge.judge_config();
        let chat_at = bridge.decisions().chat_locality();
        // What the switch showed, which for an unswitched use depends on where the model runs.
        let on = config.use_on_where(&id, locality_of(&config, chat_at) == Locality::Cloud);
        config.set_use(&id, !on);
        bridge.set_judge(config.clone());
        if let Some(ui) = weak.upgrade() {
            push(&ui, &config, chat_at);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_variable_name_is_taken_never_a_key() {
        for ok in ["", "JEV_API_KEY", "_MY_KEY2"] {
            assert!(is_variable_name(ok), "{ok}");
        }
        for key in ["sk-ab12cd34", "jev_live_9f8e7d6c5b4a", "Bearer x", "my key", "JEV-API-KEY"] {
            assert!(!is_variable_name(key), "{key}");
        }
    }

    #[test]
    fn the_person_is_told_where_the_model_runs() {
        let here = Locality::ThisMachine;
        let cloud = JudgeConfig::default().with_preset("jev");
        assert!(where_note(&cloud, here).1, "Jev is in the cloud");
        let local = JudgeConfig::default().with_preset("kev");
        assert!(!where_note(&local, here).1);
        let home = JudgeConfig { endpoint: "http://192.168.4.20:8009".into(), ..JudgeConfig::default().with_preset("kev") };
        assert!(where_note(&home, here).0.contains("home network"));
        assert_eq!(where_note(&JudgeConfig::default(), here).0, "", "off says nothing");
        let chat = JudgeConfig::default().with_preset("chat_model");
        assert!(where_note(&chat, Locality::Cloud).1, "a chat model in the cloud says so");
        assert!(!where_note(&chat, here).1 && where_note(&chat, here).0.contains("this machine"));
    }

    #[test]
    fn the_chosen_provider_is_the_one_shown() {
        assert_eq!(provider_of(&JudgeConfig::default()), "off");
        assert_eq!(provider_of(&JudgeConfig::default().with_preset("laya")), "laya");
        let legacy = JudgeConfig { endpoint: "http://127.0.0.1:8009".into(), ..JudgeConfig::default() };
        assert_eq!(provider_of(&legacy), "systemone", "an older judge: section still reads as a server");
    }
}
