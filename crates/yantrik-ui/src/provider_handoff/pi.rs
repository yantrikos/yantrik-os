//! Pi (harnesses/pi): its provider and model in ~/.config/yantrik/pi.json, which the harness turns
//! into `pi --provider --model`, and its custom providers in Pi's own ~/.pi/agent/models.json. Using
//! Yantrik models is one provider there, `yantrik`, at the gateway with Pi's own token and every
//! model of the catalogue; every other provider the person keeps there is left as it was.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::{Handoff, Offer, Plan, Write};
use crate::wire::settings::ProviderStoreEntry;

pub(crate) struct Pi;

const UNIT: &str = "yantrik-pi.service";
/// The provider's name in Pi's models.json.
pub(crate) const PROVIDER: &str = "yantrik";

fn models_json(home: &Path) -> PathBuf {
    home.join(".pi/agent/models.json")
}

fn pi_json(home: &Path) -> PathBuf {
    home.join(".config/yantrik/pi.json")
}

impl Handoff for Pi {
    fn harness(&self) -> &'static str {
        "pi"
    }

    fn name(&self) -> &'static str {
        "Pi"
    }

    fn files(&self, home: &Path) -> Vec<PathBuf> {
        vec![pi_json(home), models_json(home)]
    }

    fn unit(&self) -> Option<&'static str> {
        Some(UNIT)
    }

    fn plan(&self, _home: &Path, _provider: &ProviderStoreEntry) -> Result<Plan, String> {
        Err("Pi keeps its providers itself; give it Yantrik models instead, or set it up with its own `pi` login".into())
    }

    fn plan_gateway(&self, home: &Path, offer: &Offer) -> Result<Plan, String> {
        let mut config = super::read_json_object(&pi_json(home))?;
        config.insert("provider".into(), Value::String(PROVIDER.into()));
        config.insert("model".into(), Value::String(yantrik_gateway::PICKED.into()));
        let pi = serde_json::to_string_pretty(&Value::Object(config)).map_err(|e| e.to_string())? + "\n";

        let mut models = super::read_json_object(&models_json(home))?;
        let providers = models.entry("providers").or_insert_with(|| json!({}));
        let Some(providers) = providers.as_object_mut() else {
            return Err(format!("{}: `providers` is not an object, so it is left alone", models_json(home).display()));
        };
        // `picked` first: the model the person picks in the ask bar, resolved by the gateway at
        // each call; then every model, for Pi's own /model.
        let picked = json!({
            "id": yantrik_gateway::PICKED,
            "name": "The model picked in the ask bar",
            "reasoning": true,
            "input": ["text", "image"],
            "cost": { "input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0 },
            "contextWindow": 128_000,
            "maxTokens": 32_768,
        });
        let list: Vec<Value> = std::iter::once(picked).chain(offer
            .models
            .iter()
            .map(|m| {
                json!({
                    "id": m.id,
                    "name": format!("{} · {}", m.name, m.account),
                    "reasoning": m.caps.thinks(),
                    "input": if m.caps.vision { json!(["text", "image"]) } else { json!(["text"]) },
                    "cost": { "input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0 },
                    "contextWindow": if m.caps.context > 0 { m.caps.context } else { 128_000 },
                    "maxTokens": 32_768,
                })
            }))
            .collect();
        providers.insert(
            PROVIDER.into(),
            json!({ "baseUrl": offer.base_url(), "api": "openai-completions", "apiKey": offer.token.0, "models": list }),
        );
        let models_text = serde_json::to_string_pretty(&Value::Object(models)).map_err(|e| e.to_string())? + "\n";
        Ok(super::gateway_plan(
            self,
            offer,
            vec![
                Write { path: pi_json(home), content: pi, what: "which provider and model Pi uses".into() },
                Write {
                    path: models_json(home),
                    content: models_text,
                    what: "Pi's own provider list, with Yantrik models and its gateway token added".into(),
                },
            ],
        ))
    }

    /// A coding agent on the folder it is given; it keeps no memory of the person.
    fn private_context(&self) -> bool {
        false
    }
}
