//! Hermes (harnesses/hermes): its model is Hermes's own setting, in ~/.hermes/config.yaml under
//! `model` — `default`, `provider`, `base_url`, `api_key` for a custom OpenAI-compatible endpoint,
//! which is what `hermes model` writes for one. Using Yantrik models sets those four, with
//! Hermes's own gateway token as the key, and leaves the rest of the file's settings as they were.
//! Its comments are not kept in the new file (YAML read and written again); Revert puts the
//! person's own file back, comments and all, and the card says so.

use std::path::{Path, PathBuf};

use serde_yaml::{Mapping, Value};

use super::{Handoff, Offer, Plan, Write};
use crate::wire::settings::ProviderStoreEntry;

pub(crate) struct Hermes;

/// Hermes's own gateway service, which loads the desktop plugin (harnesses/lib/install/hermes.sh).
const UNIT: &str = "hermes-gateway.service";

fn config(home: &Path) -> PathBuf {
    home.join(".hermes/config.yaml")
}

impl Handoff for Hermes {
    fn harness(&self) -> &'static str {
        "hermes"
    }

    fn name(&self) -> &'static str {
        "Hermes"
    }

    fn files(&self, home: &Path) -> Vec<PathBuf> {
        vec![config(home)]
    }

    fn unit(&self) -> Option<&'static str> {
        Some(UNIT)
    }

    fn plan(&self, _home: &Path, _provider: &ProviderStoreEntry) -> Result<Plan, String> {
        Err("Hermes chooses its model itself (Choose model); give it Yantrik models instead".into())
    }

    fn plan_gateway(&self, home: &Path, offer: &Offer) -> Result<Plan, String> {
        let path = config(home);
        let mut doc: Mapping = match std::fs::read_to_string(&path) {
            Ok(text) if !text.trim().is_empty() => match serde_yaml::from_str::<Value>(&text) {
                Ok(Value::Mapping(m)) => m,
                _ => return Err(format!("{} is not a YAML mapping, so it is left alone", path.display())),
            },
            _ => Mapping::new(),
        };
        let model = doc.entry(Value::from("model")).or_insert_with(|| Value::Mapping(Mapping::new()));
        // An older config names the model as a bare string; it becomes the mapping Hermes reads now.
        if !model.is_mapping() {
            *model = Value::Mapping(Mapping::new());
        }
        let m = model.as_mapping_mut().expect("made a mapping above");
        m.insert("default".into(), yantrik_gateway::PICKED.into());
        m.insert("provider".into(), "custom".into());
        m.insert("base_url".into(), offer.base_url().into());
        m.insert("api_key".into(), offer.token.0.clone().into());
        let content = serde_yaml::to_string(&Value::Mapping(doc)).map_err(|e| e.to_string())?;
        Ok(super::gateway_plan(
            self,
            offer,
            vec![Write {
                path,
                content,
                what: "its model, the gateway's address and its gateway token (the file's comments are not kept; Revert puts yours back)".into(),
            }],
        ))
    }
}
