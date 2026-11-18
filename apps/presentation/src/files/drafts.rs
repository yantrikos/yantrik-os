//! What the deck's draft holds: the app's own schema, written through the framework's draft lane
//! (`fx.save_draft`) and read back from `Boot::drafts` at the next start.
//!
//! A draft holds the deck's content, the file it is a draft of, and that file's text as the deck
//! last read or wrote it, so a recovered deck is unsaved against the right baseline and a moved
//! file can still be looked for.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::deck::{bounded, parse, Content};

/// The schema's version.
pub const VERSION: u32 = 1;

/// The deck's draft.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeckDraft {
    pub version: u32,
    /// The deck's file, if it has one.
    pub path: Option<PathBuf>,
    pub content: Content,
    /// The file's text as the deck last read or wrote it. `None` for a deck never written, and
    /// for one the old build's recovery file brought back, whose file's text was never kept.
    pub on_disk: Option<String>,
    /// When the draft was made, in milliseconds: the newest is the one a start opens.
    pub at: u64,
}

impl DeckDraft {
    pub fn bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    /// A draft read back, if it is one this app wrote and holds a deck.
    pub fn parse(bytes: &[u8]) -> Result<DeckDraft, String> {
        let draft: DeckDraft = serde_json::from_slice(bytes).map_err(|e| format!("it could not be read: {e}"))?;
        if draft.version != VERSION {
            return Err(format!("it was written by another version of yPresent ({})", draft.version));
        }
        bounded(draft.content.slides.len()).map_err(|e| e.to_string())?;
        if let Some(on_disk) = &draft.on_disk {
            parse(on_disk).map_err(|e| e.to_string())?;
        }
        if draft.path.as_ref().is_some_and(|p| !p.is_absolute()) {
            return Err("its file is not an absolute path".into());
        }
        Ok(draft)
    }
}

/// Milliseconds since the epoch.
pub use yantrik_app::documents::files::now_ms;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deck::Slide;

    fn deck() -> Content {
        Content { title: "t".into(), theme: 1, slides: vec![Slide::new("a", "b", 0)] }
    }

    #[test]
    fn a_draft_round_trips_and_a_bad_one_is_refused() {
        let d = DeckDraft { version: VERSION, path: Some("/home/p/a.ydeck".into()), content: deck(), on_disk: None, at: 2 };
        assert_eq!(DeckDraft::parse(&d.bytes()).unwrap(), d);
        assert!(DeckDraft::parse(b"{not json").is_err());
        let empty = DeckDraft { content: Content::default(), ..d.clone() };
        assert!(DeckDraft::parse(&empty.bytes()).is_err());
        let relative = DeckDraft { path: Some("a.ydeck".into()), ..d.clone() };
        assert!(DeckDraft::parse(&relative.bytes()).is_err());
        let not_a_deck = DeckDraft { on_disk: Some("not a deck".into()), ..d.clone() };
        assert!(DeckDraft::parse(&not_a_deck.bytes()).is_err());
        let future = DeckDraft { version: 9, ..d };
        assert!(DeckDraft::parse(&future.bytes()).unwrap_err().contains("another version"));
    }
}
