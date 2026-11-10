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

/// What the shell makes of a mind's detail before it is listed: `(mind id, what it announced)`,
/// the line to show instead (`None`: show what it announced).
type ModelLine = Box<dyn Fn(&str, Option<&str>) -> Option<String> + Send + Sync>;

static MODEL_LINE: std::sync::RwLock<Option<ModelLine>> = std::sync::RwLock::new(None);

/// The shell says which model a mind's detail line names. The shell routes the mind's calls, so it
/// knows the model before the mind does: after a restart, before the mind's next turn, and after
/// a pick changes. The mind's own detail still says the rest ("Hermes 0.14.0"). One per process,
/// like the one host.
pub fn set_model_line(line: impl Fn(&str, Option<&str>) -> Option<String> + Send + Sync + 'static) {
    *MODEL_LINE.write().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(line));
}

/// `rows` with the shell's model line on each attached mind's detail. Takes the guard so that the
/// state's lock is let go before the shell is asked: its answer reads files and logs and must not
/// be able to wait on the host.
pub(super) fn displayed(state: std::sync::MutexGuard<'_, super::State>, mut rows: Vec<crate::Entry>) -> Vec<crate::Entry> {
    drop(state);
    let hook = MODEL_LINE.read().unwrap_or_else(|e| e.into_inner());
    if let Some(line) = hook.as_ref() {
        for row in rows.iter_mut().filter(|r| !r.builtin) {
            if let Some(shown) = line(&row.id, row.detail.as_deref()).and_then(|l| shown_detail(&l)) {
                row.detail = Some(shown);
            }
        }
    }
    rows
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

    /// #1162: the shell's model line replaces what a mind announced, at once, with no turn and no
    /// re-attach; a mind it says nothing of keeps its own words. The hook is global, so this mind has an id no other test uses.
    #[test]
    fn the_shells_model_line_is_what_a_listed_detail_says() {
        let host = Host::new(Vec::new());
        host.handle(protocol::ATTACH, &serde_json::json!({ "id": "shelled-1162", "name": "Hermes", "detail": "Hermes 0.14.0" })).unwrap();
        host.handle(protocol::ATTACH, &serde_json::json!({ "id": "pi", "name": "Pi", "detail": "Pi 1" })).unwrap();
        super::set_model_line(|id, announced| (id == "shelled-1162").then(|| format!("{} \u{b7} qwen3.8:27b", announced.unwrap_or("Hermes").split(" \u{b7} ").next().unwrap())));
        let said = |id: &str| host.list().into_iter().find(|e| e.id == id).unwrap().detail;
        assert_eq!(said("shelled-1162").as_deref(), Some("Hermes 0.14.0 \u{b7} qwen3.8:27b"));
        assert_eq!(said("pi").as_deref(), Some("Pi 1"));
        *super::MODEL_LINE.write().unwrap() = None;
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
