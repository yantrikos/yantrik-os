//! The companion's side of "Prepare for me": the real writer (the person's chosen model, no
//! tools) and the one call the shell's Think arm makes (#1416, slice 5).
//!
//! The Think arm is the one loop. Each tick it asks [`crate::agenda::triggers`] what is due, and
//! for the soonest not yet prepared item calls [`CompanionService::prepare_for_me`]. That runs as the
//! scheduler's background class: a self-started call (the gateway classes it `own` unless a
//! person's turn is open), paused while the person has a turn open (`Controls::paused`), within the
//! companion's daily share (#1403's gateway rule applies to every model call it makes).

use yantrik_ml::{ChatMessage, GenerationConfig};

use super::{Controls, Item, Outcome, Settings, Writer};
use crate::companion::CompanionService;
use crate::tools::{parse_permission, ToolContext};

/// Writes content with the companion's model: one chat call, no tools, low temperature.
pub struct ModelWriter<'a> {
    llm: &'a dyn yantrik_ml::LLMBackend,
}

impl Writer for ModelWriter<'_> {
    fn write(&mut self, instruction: &str, material: &str, max_tokens: usize) -> Result<(String, usize), String> {
        let prompt = format!(
            "{instruction}\n\nWrite plain text only. Use only the material below and what the instruction says. \
             You have no tools and must not ask for any.\n\n{material}"
        );
        let config = GenerationConfig { max_tokens, temperature: 0.3, ..Default::default() };
        let r = self.llm.chat(&[ChatMessage::user(&prompt)], &config, None).map_err(|e| e.to_string())?;
        Ok((r.text.clone(), r.prompt_tokens + r.completion_tokens))
    }
}

impl CompanionService {
    /// Prepare `item` as a background job. Never in incognito. The caller holds no database
    /// connection (the job takes its own, one statement at a time).
    pub fn prepare_for_me(&self, item: &Item, controls: &Controls<'_>, settings: &Settings) -> Outcome {
        let ctx = ToolContext {
            db: &self.db,
            max_permission: parse_permission(&self.config.tools.max_permission),
            registry_metadata: None,
            task_manager: Some(&self.task_manager),
            incognito: false,
            agent_spawner: None,
        };
        // The scheduler's background class: the gateway classes a self-started call `own`.
        let _own = yantrik_ml::asked::self_started();
        let mut writer = ModelWriter { llm: &*self.llm };
        super::run(&self.registry, &ctx, &mut writer, item, controls, settings)
    }
}
