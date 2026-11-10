//! `harness.detail`: a harness correcting the line the picker shows under its name.
//!
//! A mind's detail ("Hermes 0.9 · qwen3.8:27b") is announced at attach, but what it names can change
//! while the mind stays attached: the person picks another model. A re-attach would replace the
//! entry and fail the turns it owes, so this changes the one line and nothing else. It is held to
//! the same rules as the attach (the caller must hold the session; an address loses its sign-in).

use super::{Host, Peer};

impl Host {
    /// `harness.detail`.
    pub(super) fn set_detail(&self, params: &serde_json::Value, peer: Peer) -> Result<serde_json::Value, String> {
        let detail = params["detail"].as_str().ok_or("`detail` is missing; it is the text to show under the name")?;
        let mut state = self.lock();
        let harness = Self::touch(&mut state.attached, params, peer, &*self.descends)?;
        harness.announced.detail = Some(yantrik_shown_address::without_sign_in(detail)).filter(|d| !d.trim().is_empty());
        Ok(serde_json::json!({}))
    }
}

#[cfg(test)]
mod tests {
    use crate::{host::Host, protocol};

    #[test]
    fn a_harness_can_change_its_detail_without_attaching_again() {
        let host = Host::new(Vec::new());
        let attached = host.handle(protocol::ATTACH, &serde_json::json!({ "id": "hermes", "name": "Hermes", "detail": "Hermes 1 \u{b7} picked" })).unwrap();
        let session = attached["session"].as_str().unwrap().to_string();
        host.handle(protocol::DETAIL, &serde_json::json!({ "session": session, "detail": "Hermes 1 \u{b7} qwen3.8:27b" })).unwrap();
        let row = host.list().into_iter().find(|e| e.id == "hermes").unwrap();
        assert_eq!(row.detail.as_deref(), Some("Hermes 1 \u{b7} qwen3.8:27b"));
    }

    #[test]
    fn a_stranger_cannot_change_a_detail() {
        let host = Host::new(Vec::new());
        host.handle(protocol::ATTACH, &serde_json::json!({ "id": "hermes", "name": "Hermes", "detail": "a" })).unwrap();
        assert!(host.handle(protocol::DETAIL, &serde_json::json!({ "session": "nope", "detail": "b" })).is_err());
        assert_eq!(host.list().into_iter().find(|e| e.id == "hermes").unwrap().detail.as_deref(), Some("a"));
    }
}
