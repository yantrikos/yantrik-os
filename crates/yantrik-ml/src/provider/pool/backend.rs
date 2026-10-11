//! The free pool as a model: an `LLMBackend` that, call by call, asks the pool which free model to
//! use, calls it, tells the pool how it went (its status and rate-limit headers), and on a refusal
//! moves to the next one. To the companion or a harness it is one model that does not run out
//! while any switched-on provider has room.

use std::sync::{Arc, Mutex};

use anyhow::{bail, Result};

use super::headers::observe;
use super::keys::Keys;
use super::quota::Outcome;
use super::select::Need;
use super::{Pick, Pool};
use crate::provider::generic_openai::{GenericOpenAIBackend, MetaSink};
use crate::traits::LLMBackend;
use crate::types::{ChatMessage, GenerationConfig, LLMResponse};

/// How many providers one call may try before it gives up.
const ATTEMPTS: usize = 5;

pub struct PoolBackend {
    pool: Arc<Mutex<Pool>>,
    keys: Keys,
    /// What every call through this backend needs (its task, whether it is private, code…).
    need: Need,
    /// A provider reached at another address than its own: a test's stand-in, never in a build.
    addresses: std::collections::HashMap<String, String>,
}

impl PoolBackend {
    pub fn new(pool: Arc<Mutex<Pool>>, keys: Keys, need: Need) -> PoolBackend {
        PoolBackend { pool, keys, need, addresses: Default::default() }
    }

    /// Reach `provider` at `base_url` instead of its own address: a test's stand-in only. A key
    /// goes to its provider's own address and nowhere else.
    #[cfg(test)]
    pub fn with_address(mut self, provider: &str, base_url: &str) -> PoolBackend {
        self.addresses.insert(provider.to_string(), base_url.to_string());
        self
    }

    fn now() -> i64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
    }

    /// This call's needs: the backend's own, plus tools and room for the conversation.
    fn need_for(&self, messages: &[ChatMessage], config: &GenerationConfig, tools: Option<&[serde_json::Value]>) -> Need {
        let chars: usize = messages.iter().map(|m| m.content.len()).sum();
        let mut need = self.need.clone();
        need.tools = need.tools || tools.is_some_and(|t| !t.is_empty());
        need.min_context = need.min_context.max((chars / 3 + config.max_tokens) as u32);
        need
    }

    fn backend_for(&self, pick: &Pick, sink: &MetaSink) -> GenericOpenAIBackend {
        let key = if pick.tier.needs_key { self.keys.get(pick.tier.id).map(String::from) } else { None };
        let stand_in = self.addresses.get(pick.tier.id).cloned();
        let backend = GenericOpenAIBackend::for_provider(
            pick.tier.id,
            stand_in.clone().unwrap_or_else(|| self.keys.base_url(pick.tier)),
            key,
            pick.model.id,
        )
        .with_meta_sink(sink.clone());
        // The provider's own address: HTTPS, and no redirect followed. A test's local stand-in
        // is plain http.
        if stand_in.is_some() { backend } else { backend.strict_transport() }
    }

    /// Try providers in the pool's order until one answers.
    fn run(
        &self,
        messages: &[ChatMessage],
        config: &GenerationConfig,
        tools: Option<&[serde_json::Value]>,
        mut call: impl FnMut(&GenericOpenAIBackend) -> (Result<LLMResponse>, bool),
    ) -> Result<LLMResponse> {
        let need = self.need_for(messages, config, tools);
        let mut last = String::new();
        for _ in 0..ATTEMPTS {
            let pick = {
                let mut pool = self.pool.lock().unwrap_or_else(|e| e.into_inner());
                match pool.pick(&need, Self::now()) {
                    Ok(p) => p,
                    Err(e) if last.is_empty() => bail!("free pool: {}", e.reason),
                    Err(e) => bail!("free pool: {} (last: {last})", e.reason),
                }
            };
            let sink: MetaSink = Arc::new(Mutex::new(None));
            let (result, started) = call(&self.backend_for(&pick, &sink));
            let meta = sink.lock().unwrap_or_else(|e| e.into_inner()).take();
            let status = meta.as_ref().map_or(if result.is_ok() { 200 } else { 0 }, |m| m.status);
            // A provider reports these: saturating, so a hostile `usage` cannot overflow the count.
            let tokens = result.as_ref().map_or(0, |r| (r.prompt_tokens as u64).saturating_add(r.completion_tokens as u64));
            let observed = meta.as_ref().map(|m| observe(|n| m.header(n))).unwrap_or_default();
            let outcome = Outcome { status, observed, tokens };
            self.pool.lock().unwrap_or_else(|e| e.into_inner()).record(pick, &need, Self::now(), &outcome);
            match result {
                Ok(r) => return Ok(r),
                // A stream that already reached the caller is not replayed on another model.
                Err(e) if started => return Err(e.context(format!("free pool: {} stopped mid-answer", pick.tier.name))),
                Err(e) => last = format!("{} {}: {e}", pick.tier.name, pick.model.id),
            }
        }
        bail!("free pool: {ATTEMPTS} providers refused in a row (last: {last})")
    }
}

impl LLMBackend for PoolBackend {
    fn chat(&self, messages: &[ChatMessage], config: &GenerationConfig, tools: Option<&[serde_json::Value]>) -> Result<LLMResponse> {
        self.run(messages, config, tools, |b| (b.chat(messages, config, tools), false))
    }

    fn chat_streaming(
        &self,
        messages: &[ChatMessage],
        config: &GenerationConfig,
        tools: Option<&[serde_json::Value]>,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<LLMResponse> {
        self.run(messages, config, tools, |b| {
            let mut started = false;
            let r = b.chat_streaming(messages, config, tools, &mut |t| {
                started = true;
                on_token(t);
            });
            (r, started)
        })
    }

    fn count_tokens(&self, text: &str) -> Result<usize> {
        Ok(text.len() / 4)
    }

    fn backend_name(&self) -> &str {
        "free-pool"
    }

    fn model_id(&self) -> &str {
        "free-pool"
    }
}
