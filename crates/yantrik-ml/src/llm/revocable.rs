//! A backend the person can take back at once: pressing Remove on an AI provider cuts it on the
//! pressing thread, not through the companion's queue behind the turn that is running.
//!
//! Every clone of a [`Revocable`] handed out before the cut (the turn, the judge, sub-agents)
//! shares one flag, so a cut ends them all: a call not yet sent is refused before any request is
//! built, a call in flight is thrown away, and a reply being streamed is closed by the client
//! (`ApiLLM::with_cut`). [`LlmSwitch`] holds the current one and drops its reference to the
//! inner backend, and so to its key, when it cuts.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Result;

use super::cut_flag::CutFlag;
use crate::traits::LLMBackend;
use crate::types::{ChatMessage, GenerationConfig, LLMResponse};

/// What the companion says when no AI provider is set up. One sentence, here, so the stopped
/// backend, the companion's marker and the screen cannot drift apart.
pub const NO_PROVIDER: &str = "No AI provider is set up: the provider this companion used was removed. Add or choose one in Settings → AI & Intelligence.";

/// [`LLMBackend::backend_name`] of a backend that cannot think: stopped, or never given one.
pub const STOPPED_BACKEND: &str = "none";

/// The error of a call refused or cut because the backend was stopped. A typed marker, the way
/// `ProviderRefused` and `StreamError` are, so the companion answers with [`NO_PROVIDER`] and
/// never mistakes it for an offline provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stopped;

impl std::fmt::Display for Stopped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(NO_PROVIDER)
    }
}

impl std::error::Error for Stopped {}

/// A fresh, unset cut flag: pass one clone to `ApiLLM::with_cut` and the other to
/// [`LlmSwitch::arm_flagged`].
pub fn new_cut_flag() -> Arc<CutFlag> {
    Arc::new(CutFlag::default())
}

/// A backend that can be cut for good.
pub struct Revocable {
    inner: Mutex<Option<Arc<dyn LLMBackend>>>,
    cut: Arc<CutFlag>,
    /// The inner backend's name and model, read once: they are answered after the cut as well.
    name: String,
    model: String,
}

impl Revocable {
    fn new(inner: Arc<dyn LLMBackend>, cut: Arc<CutFlag>) -> Self {
        let (name, model) = (
            inner.backend_name().to_string(),
            inner.model_id().to_string(),
        );
        Self {
            inner: Mutex::new(Some(inner)),
            cut,
            name,
            model,
        }
    }

    fn is_cut(&self) -> bool {
        self.cut.is_cut()
    }

    /// The backend to call, or [`Stopped`] before anything is built or sent.
    fn live(&self) -> Result<Arc<dyn LLMBackend>> {
        if self.is_cut() {
            return Err(Stopped.into());
        }
        let held = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        held.clone().ok_or_else(|| Stopped.into())
    }

    /// An answer that arrived after the cut is thrown away.
    fn unless_cut(&self, answer: Result<LLMResponse>) -> Result<LLMResponse> {
        if self.is_cut() {
            return Err(Stopped.into());
        }
        answer
    }

    fn cut(&self) {
        // Sets the flag and shuts every socket a call of this backend has open.
        self.cut.cut();
        // Dropping the reference drops the key with it once the calls in flight return.
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).take();
    }
}

impl LLMBackend for Revocable {
    fn chat(
        &self,
        messages: &[ChatMessage],
        config: &GenerationConfig,
        tools: Option<&[serde_json::Value]>,
    ) -> Result<LLMResponse> {
        let inner = self.live()?;
        let answer = inner.chat(messages, config, tools);
        self.unless_cut(answer)
    }

    fn chat_streaming(
        &self,
        messages: &[ChatMessage],
        config: &GenerationConfig,
        tools: Option<&[serde_json::Value]>,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<LLMResponse> {
        let inner = self.live()?;
        // Words that arrive after the cut are not passed on.
        let mut forward = |token: &str| {
            if !self.is_cut() {
                on_token(token);
            }
        };
        let answer = inner.chat_streaming(messages, config, tools, &mut forward);
        self.unless_cut(answer)
    }

    fn count_tokens(&self, text: &str) -> Result<usize> {
        self.live()?.count_tokens(text)
    }

    fn backend_name(&self) -> &str {
        if self.is_cut() {
            STOPPED_BACKEND
        } else {
            &self.name
        }
    }

    fn is_degraded(&self) -> bool {
        self.live().map(|b| b.is_degraded()).unwrap_or(false)
    }

    fn model_id(&self) -> &str {
        if self.is_cut() {
            STOPPED_BACKEND
        } else {
            &self.model
        }
    }
}

/// The current armed backend: what a Remove cuts.
#[derive(Default)]
pub struct LlmSwitch {
    current: Mutex<Option<Arc<Revocable>>>,
    /// Counts the cuts. A reload asked for under one epoch is stale once the epoch has moved: it
    /// was asked for before a stop and must not arm anything after it.
    epoch: AtomicU64,
}

impl LlmSwitch {
    pub fn new() -> Self {
        Self::default()
    }

    /// The number of cuts so far: read it when a reload is asked for, compare it when the reload
    /// is about to arm.
    pub fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::SeqCst)
    }

    /// Wrap `inner` so it can be cut, and remember it as the one to cut. The one armed before is
    /// cut first: a backend replaced is a backend nothing may keep calling.
    pub fn arm(&self, inner: Arc<dyn LLMBackend>) -> Arc<dyn LLMBackend> {
        self.arm_flagged(new_cut_flag(), inner)
    }

    /// As [`arm`](Self::arm), for an `inner` that already holds `cut` (`ApiLLM::with_cut`), so a
    /// reply being streamed stops with the flag.
    pub fn arm_flagged(
        &self,
        cut: Arc<CutFlag>,
        inner: Arc<dyn LLMBackend>,
    ) -> Arc<dyn LLMBackend> {
        let armed = Arc::new(Revocable::new(inner, cut));
        self.install(None, armed.clone());
        armed
    }

    /// As [`arm_flagged`](Self::arm_flagged), only if no cut has come since `epoch` was read: a
    /// reload asked for before a stop must not arm anything after it. The check and the arming
    /// are one step under the switch's lock, so a cut cannot fall between them. `None` when the
    /// epoch has moved; nothing was armed.
    pub fn arm_flagged_at(
        &self,
        epoch: u64,
        cut: Arc<CutFlag>,
        inner: Arc<dyn LLMBackend>,
    ) -> Option<Arc<dyn LLMBackend>> {
        let armed = Arc::new(Revocable::new(inner, cut));
        self.install(Some(epoch), armed.clone())
            .then_some(armed as Arc<dyn LLMBackend>)
    }

    /// Make `armed` the current one, cutting the one it replaces; false, and nothing changed,
    /// when `only_at` is given and the epoch is no longer it.
    fn install(&self, only_at: Option<u64>, armed: Arc<Revocable>) -> bool {
        let mut current = self.current.lock().unwrap_or_else(|e| e.into_inner());
        if only_at.is_some_and(|epoch| epoch != self.epoch()) {
            return false;
        }
        let before = current.replace(armed);
        drop(current);
        if let Some(before) = before {
            before.cut();
        }
        true
    }

    /// Cut the armed backend, on the calling thread: every clone of it is refused or ended, and
    /// the switch no longer holds the inner backend or its key. Moves the epoch: a reload asked
    /// for before this is stale.
    pub fn cut(&self) {
        self.take_and_cut(true);
    }

    /// As [`cut`](Self::cut), but the epoch stays: for the worker applying a stop that was already
    /// cut on the pressing thread. A reload asked for after that press carries the new epoch and
    /// must still be armed, so applying the stop must not make it stale.
    pub fn release(&self) {
        self.take_and_cut(false);
    }

    fn take_and_cut(&self, new_epoch: bool) {
        let armed = {
            let mut current = self.current.lock().unwrap_or_else(|e| e.into_inner());
            // Under the lock, so `arm_flagged_at` sees the epoch and the backend change together.
            if new_epoch {
                self.epoch.fetch_add(1, Ordering::SeqCst);
            }
            current.take()
        };
        if let Some(armed) = armed {
            armed.cut();
        }
    }
}
