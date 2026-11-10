//! `harness.detail`: a harness correcting the line the picker shows under its name.
//!
//! A mind's detail ("Hermes 0.9 · qwen3.8:27b") is announced at attach, but what it names can change
//! while the mind stays attached: the person picks another model. A re-attach would replace the
//! entry and fail the turns it owes, so this changes the one line and nothing else. It is held to
//! the same rules as the attach (the caller must hold the session; an address loses its sign-in).

use super::{Host, Peer};

/// The most characters of a mind's detail that are ever kept.
const DETAIL_CHARS: usize = 120;

/// A mind's detail as it may be drawn, for the attach and for `harness.detail` alike (one rule, so
/// the two cannot differ): on one line (control, newline and bidi characters become spaces; the
/// zero-width ones go), an address without its sign-in, at most [`DETAIL_CHARS`] characters.
/// `None` when nothing is left to show.
pub(super) fn shown_detail(raw: &str) -> Option<String> {
    // Bound the work first: a harness can send a megabyte.
    let bounded: String = raw.chars().take(DETAIL_CHARS * 8).filter(|c| !is_invisible(*c)).collect();
    let spaced: String = bounded.chars().map(|c| if c.is_control() || is_bidi(c) { ' ' } else { c }).collect();
    let line = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    let shown = yantrik_shown_address::without_sign_in(&line);
    let mut kept: String = shown.chars().take(DETAIL_CHARS).collect();
    if shown.chars().count() > DETAIL_CHARS {
        kept.pop();
        kept.push('\u{2026}');
    }
    Some(kept).filter(|d| !d.trim().is_empty())
}

/// Unicode's bidirectional formatting characters: marks, embeddings, overrides and isolates (the
/// host is transport-free, so it cannot borrow `yantrik-ipc-transport`'s list of them).
fn is_bidi(c: char) -> bool {
    matches!(c, '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// Format characters that draw nothing (zero-width spaces and joiners, the word joiner, a BOM).
fn is_invisible(c: char) -> bool {
    matches!(c, '\u{200B}'..='\u{200D}' | '\u{2060}' | '\u{FEFF}')
}

impl Host {
    /// `harness.detail`.
    pub(super) fn set_detail(&self, params: &serde_json::Value, peer: Peer) -> Result<serde_json::Value, String> {
        let detail = params["detail"].as_str().ok_or("`detail` is missing; it is the text to show under the name")?;
        let mut state = self.lock();
        let harness = Self::touch(&mut state.attached, params, peer, &*self.descends)?;
        harness.announced.detail = shown_detail(detail);
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

    fn list_detail(host: &Host) -> Option<String> {
        host.list().into_iter().find(|e| e.id == "hermes").unwrap().detail
    }

    /// L1 of #1162: the attach and `harness.detail` share one rule, and each is tested against it.
    fn hostile() -> Vec<(String, &'static str)> {
        vec![
            ("Hermes 1 \u{b7} \u{202E}gpj.4o-tpg\nUndo: possible\r\t\u{200B}".to_string(), "Hermes 1 \u{b7} gpj.4o-tpg Undo: possible"),
            ("x".repeat(1_000_000), "capped"),
            ("qwen \u{b7} https://user:sk-secret@aig.example/v1".to_string(), "no sign-in"),
        ]
    }

    fn check(shown: &str, kind: &str) {
        assert!(!shown.chars().any(|c| c.is_control() || super::is_bidi(c) || super::is_invisible(c)), "{shown:?}");
        assert!(shown.chars().count() <= super::DETAIL_CHARS, "{} chars", shown.chars().count());
        assert!(!shown.contains("sk-secret"), "{shown}");
        if kind != "capped" && kind != "no sign-in" {
            assert_eq!(shown, kind);
        }
    }

    #[test]
    fn a_detail_sent_by_harness_detail_is_one_short_plain_line() {
        for (raw, kind) in hostile() {
            let host = Host::new(Vec::new());
            let attached = host.handle(protocol::ATTACH, &serde_json::json!({ "id": "hermes", "name": "Hermes", "detail": "a" })).unwrap();
            let session = attached["session"].as_str().unwrap().to_string();
            host.handle(protocol::DETAIL, &serde_json::json!({ "session": session, "detail": raw })).unwrap();
            check(&list_detail(&host).unwrap(), kind);
        }
    }

    #[test]
    fn a_detail_sent_at_attach_is_held_to_the_same_rule() {
        for (raw, kind) in hostile() {
            let host = Host::new(Vec::new());
            host.handle(protocol::ATTACH, &serde_json::json!({ "id": "hermes", "name": "Hermes", "detail": raw })).unwrap();
            check(&list_detail(&host).unwrap(), kind);
        }
    }

    #[test]
    fn a_detail_with_nothing_to_show_is_none() {
        assert_eq!(super::shown_detail(" \u{200B}\n"), None);
    }

    #[test]
    fn a_stranger_cannot_change_a_detail() {
        let host = Host::new(Vec::new());
        host.handle(protocol::ATTACH, &serde_json::json!({ "id": "hermes", "name": "Hermes", "detail": "a" })).unwrap();
        assert!(host.handle(protocol::DETAIL, &serde_json::json!({ "session": "nope", "detail": "b" })).is_err());
        assert_eq!(host.list().into_iter().find(|e| e.id == "hermes").unwrap().detail.as_deref(), Some("a"));
    }
}
