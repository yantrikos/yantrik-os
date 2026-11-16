//! The worker's side of `ReloadLLM`: build the backend the companion is moved onto, armed so a
//! Remove can cut it, unless a Remove came after the reload was asked for.

use std::sync::Arc;

use yantrik_ml::{LLMBackend, LlmSwitch};

/// What a reload asked for, as the bridge queued it.
pub(crate) struct Reload<'a> {
    pub provider_type: &'a str,
    pub base_url: &'a str,
    pub key: Option<&'a str>,
    pub model: &'a str,
    /// [`LlmSwitch::epoch`] when the reload was asked for.
    pub epoch: u64,
}

/// The armed backend for `reload`, or `None` when a stop came after it was asked for: that
/// reload is dropped before anything is built, so the removed provider is not put back, and
/// neither is its key. The backend is built `with_cut`, so a reply being read, or a call still
/// waiting on the server, ends with the cut.
pub(crate) fn armed_reload(switch: &LlmSwitch, reload: &Reload) -> Option<Arc<dyn LLMBackend>> {
    if switch.epoch() != reload.epoch {
        tracing::info!("a reload asked for before a stop is dropped");
        return None;
    }
    let cut = yantrik_ml::new_cut_flag();
    let api = yantrik_ml::ApiLLM::new(reload.base_url, reload.key.map(str::to_string), reload.model)
        .for_provider(reload.provider_type)
        .with_cut(cut.clone());
    // One step under the switch's lock: a stop between the check above and here drops it too.
    switch.arm_flagged_at(reload.epoch, cut, Arc::new(api))
}
