//! OpenClaw (harnesses/openclaw): its providers are OpenClaw's own, in ~/.openclaw/openclaw.json
//! under `models.providers`, and the model this desktop asks it for is `model` in
//! ~/.config/yantrik/openclaw.json (`--model`, or `x-openclaw-model` on the gateway route). Using
//! Yantrik models adds one provider there, `yantrik`, with OpenClaw's own gateway token, and
//! asks for `yantrik/<account>/<model>`. An openclaw.json with comments (JSON5) is left alone, and
//! the refusal says so: the person adds the provider by hand.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::{Handoff, Offer, Plan, Write};
use crate::wire::settings::ProviderStoreEntry;

pub(crate) struct OpenClaw;

const UNIT: &str = "yantrik-openclaw.service";
const PROVIDER: &str = "yantrik";

fn own(home: &Path) -> PathBuf {
    home.join(".openclaw/openclaw.json")
}

fn ours(home: &Path) -> PathBuf {
    home.join(".config/yantrik/openclaw.json")
}

impl Handoff for OpenClaw {
    fn harness(&self) -> &'static str {
        "openclaw"
    }

    fn name(&self) -> &'static str {
        "OpenClaw"
    }

    fn files(&self, home: &Path) -> Vec<PathBuf> {
        vec![ours(home), own(home)]
    }

    fn unit(&self) -> Option<&'static str> {
        Some(UNIT)
    }

    fn plan(&self, _home: &Path, _provider: &ProviderStoreEntry) -> Result<Plan, String> {
        Err("OpenClaw keeps its providers itself (Set up OpenClaw); give it Yantrik models instead".into())
    }

    fn plan_gateway(&self, home: &Path, offer: &Offer) -> Result<Plan, String> {
        let mut config = super::read_json_object(&ours(home))?;
        config.insert("model".into(), Value::String(format!("{PROVIDER}/{}", yantrik_gateway::PICKED)));
        let ours_text = serde_json::to_string_pretty(&Value::Object(config)).map_err(|e| e.to_string())? + "\n";

        let mut openclaw = super::read_json_object(&own(home))?;
        let models = openclaw.entry("models").or_insert_with(|| json!({}));
        let providers = models
            .as_object_mut()
            .map(|m| m.entry("providers").or_insert_with(|| json!({})))
            .and_then(Value::as_object_mut)
            .ok_or_else(|| format!("{}: `models.providers` is not an object, so it is left alone", own(home).display()))?;
        let picked = json!({ "id": yantrik_gateway::PICKED, "name": "The model picked in the ask bar", "reasoning": true });
        let list: Vec<Value> = std::iter::once(picked)
            .chain(offer.models.iter().map(|m| {
                json!({ "id": m.id, "name": format!("{} · {}", m.name, m.account), "reasoning": m.caps.thinks(), "contextWindow": m.caps.context })
            }))
            .collect();
        providers.insert(
            PROVIDER.into(),
            json!({ "baseUrl": offer.base_url(), "apiKey": offer.token.0, "api": "openai-completions", "models": list }),
        );
        let own_text = serde_json::to_string_pretty(&Value::Object(openclaw)).map_err(|e| e.to_string())? + "\n";
        Ok(super::gateway_plan(
            self,
            offer,
            vec![
                Write { path: ours(home), content: ours_text, what: "the model this desktop asks OpenClaw for".into() },
                Write {
                    path: own(home),
                    content: own_text,
                    what: "OpenClaw's own providers, with Yantrik models and its gateway token added".into(),
                },
            ],
        ))
    }
}
