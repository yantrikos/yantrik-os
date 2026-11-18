//! A view's undo and redo: whole texts, newest last, bounded in count and in bytes. A view's own
//! history, cleared whenever the view shows another note, so undo never crosses notes.
//!
//! A note two windows show is one text with two histories. Each window's history holds only that
//! window's own edits, and is good only while the note's text is what that window left it as: once
//! the other window has changed it, stepping back would put an old text over the other window's
//! work, so the history is dropped instead, never misapplied.

use yantrik_app::documents::Fingerprint;

/// The most steps kept, and the most text they may hold together.
const STEPS: usize = 64;
const BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Default)]
pub struct History {
    undo: Vec<String>,
    redo: Vec<String>,
    /// The text as this view's last edit or step left it.
    left: Option<Fingerprint>,
}

/// What one step back (or forward) came to.
#[derive(Debug, PartialEq, Eq)]
pub enum Stepped {
    /// The text to show now.
    To(String),
    /// There was nothing to take.
    Nothing,
    /// The note was changed elsewhere since this view's last edit: its history is dropped.
    Dropped,
}

impl History {
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.left = None;
    }

    /// An edit replaced `previous` with `now`: it can be undone, and what redo held is gone.
    pub fn edited(&mut self, previous: String, now: &str) {
        remember(&mut self.undo, previous);
        self.redo.clear();
        self.left = Some(Fingerprint::of_bytes(now.as_bytes()));
    }

    /// One step back (or forward, for `redo`) from `now`, the note's text: the text to show, with
    /// `now` kept on the other side. When `now` is not what this view left, the history is
    /// dropped and nothing is stepped.
    pub fn step(&mut self, redo: bool, now: String) -> Stepped {
        if self.undo.is_empty() && self.redo.is_empty() {
            return Stepped::Nothing;
        }
        if self.left != Some(Fingerprint::of_bytes(now.as_bytes())) {
            self.clear();
            return Stepped::Dropped;
        }
        let (from, to) = if redo { (&mut self.redo, &mut self.undo) } else { (&mut self.undo, &mut self.redo) };
        let Some(text) = from.pop() else { return Stepped::Nothing };
        remember(to, now);
        self.left = Some(Fingerprint::of_bytes(text.as_bytes()));
        Stepped::To(text)
    }
}

fn remember(history: &mut Vec<String>, text: String) {
    history.push(text);
    while history.len() > STEPS || history.iter().map(String::len).sum::<usize>() > BYTES {
        history.remove(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_step_goes_back_over_this_views_own_edits_and_forward_again() {
        let mut h = History::default();
        h.edited("a".into(), "ab");
        h.edited("ab".into(), "abc");
        assert_eq!(h.step(false, "abc".into()), Stepped::To("ab".into()));
        assert_eq!(h.step(true, "ab".into()), Stepped::To("abc".into()));
        assert_eq!(h.step(true, "abc".into()), Stepped::Nothing);
    }

    #[test]
    fn a_text_changed_elsewhere_drops_the_history_and_is_never_stepped_over() {
        let mut h = History::default();
        h.edited("a".into(), "ab");
        assert_eq!(h.step(false, "ab, and the other window's words".into()), Stepped::Dropped);
        assert_eq!(h.step(false, "ab".into()), Stepped::Nothing, "dropped, not kept for later");
    }
}
