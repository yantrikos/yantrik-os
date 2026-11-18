//! A tab's undo and redo: whole texts, bounded, owned by the tab.

use super::limits::MAX_BYTES;

/// How many steps a tab keeps, and how much text they may hold between them.
const STEPS: usize = 64;
const HELD: usize = 2 * MAX_BYTES;

#[derive(Debug, Clone, Default)]
pub struct History {
    pub undo: Vec<String>,
    pub redo: Vec<String>,
}

impl History {
    /// `before` is what the tab held until an edit replaced it: one more step back, and nothing
    /// to redo any more.
    pub fn record(&mut self, before: String) {
        self.undo.push(before);
        self.redo.clear();
        while self.undo.len() > STEPS || self.undo.iter().map(String::len).sum::<usize>() > HELD {
            self.undo.remove(0);
        }
    }

    /// The text `undo` (or with `redo`, redo) would put back, without taking it.
    pub fn next(&self, redo: bool) -> Option<&String> {
        if redo {
            self.redo.last()
        } else {
            self.undo.last()
        }
    }

    /// Steps back (or forward): `now` is what the tab holds, and is kept on the other side.
    /// `None` when there is nothing to step to.
    pub fn step(&mut self, redo: bool, now: String) -> Option<String> {
        let (from, to) = if redo { (&mut self.redo, &mut self.undo) } else { (&mut self.undo, &mut self.redo) };
        let text = from.pop()?;
        to.push(now);
        Some(text)
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_and_redo_walk_whole_texts_and_an_edit_drops_redo() {
        let mut h = History::default();
        h.record("".into());
        h.record("a".into());
        assert_eq!(h.step(false, "ab".into()).as_deref(), Some("a"));
        assert_eq!(h.next(true).map(String::as_str), Some("ab"));
        assert_eq!(h.step(true, "a".into()).as_deref(), Some("ab"));
        h.record("ab".into());
        assert!(h.redo.is_empty());
        for n in 0..100 {
            h.record(n.to_string());
        }
        assert_eq!(h.undo.len(), STEPS);
        assert!(History::default().step(false, "x".into()).is_none());
    }
}
