//! The deck, and everything that can be decided about it without a window or a disk.
//!
//! Two faults put this module here. The first is that Next and Previous moved
//! `current-slide-index` and nothing else: the canvas still showed the slide you had just been
//! typing into, and the next thing that committed the canvas wrote slide N's text over slide
//! N+1. The fix is structural rather than two more `commit_current()` calls: there is one
//! [`Deck::go_to`], it commits before it moves, and every navigation path in the app goes
//! through it. A future navigation path cannot forget, because forgetting is not a thing it can
//! do.
//!
//! The second is that Save and Load were log lines, so a deck existed only while the window was
//! open. A deck is now a file; the framework's document layer reads and writes it, and what is
//! here is the content, the selection, Undo, and what "unsaved" is measured against.

mod content;
mod export;
#[cfg(test)]
mod tests;

pub use content::{
    encode, parse, theme_spec, Content, Edits, LoadError, Slide, EXTENSION, LAYOUT_COUNT, MAX_FILE_BYTES,
    MAX_SLIDES, THEMES,
};
pub(crate) use content::{bounded, check_version};
pub use export::slug;

/// How many steps back Undo can go before the oldest is dropped.
const UNDO_DEPTH: usize = 64;

/// A deck, the selection in it, and what its file holds.
///
/// It serialises as its content and nothing else: the framework's document wants a content it can
/// write, and the selection, the history and the file's state are not part of a deck.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Deck {
    content: Content,
    #[serde(skip)]
    current: usize,
    /// The content as it is on disk, or as it was when the deck began (a new deck is clean).
    /// `dirty()` is a comparison against this rather than a flag, so a change that was typed and
    /// then undone stops counting as one: a flag would keep saying "unsaved" for a deck that is
    /// byte-for-byte the file.
    #[serde(skip)]
    saved: Option<Content>,
    #[serde(skip)]
    undo: Vec<Content>,
    #[serde(skip)]
    redo: Vec<Content>,
    /// Bumped by every change of content, so the framework's document can be told it changed.
    #[serde(skip)]
    revision: u64,
}

impl Deck {
    /// A brand new deck: one title slide.
    ///
    /// It starts clean although it is on no disk. The only thing in it is the placeholder text
    /// this function just wrote, and a window that says "unsaved changes" from the moment it
    /// opens, and leaves a draft for a deck nobody has typed in, teaches a person to ignore the
    /// word.
    pub fn blank(title: impl Into<String>) -> Self {
        let content = Content {
            title: title.into(),
            theme: 0,
            slides: vec![Slide::new("Title Slide", "Click to add subtitle", 0)],
        };
        Self::opened(content)
    }

    /// A deck that is exactly its file: clean.
    pub fn opened(content: Content) -> Self {
        Self { saved: Some(content.clone()), content, current: 0, undo: Vec::new(), redo: Vec::new(), revision: 0 }
    }

    /// A deck whose file holds `saved` (`None`: no file holds anything of it), shown at `content`.
    pub fn restored(content: Content, saved: Option<Content>) -> Self {
        Self { content, saved, current: 0, undo: Vec::new(), redo: Vec::new(), revision: 0 }
    }

    // ── Reading ──

    pub fn content(&self) -> &Content {
        &self.content
    }
    pub fn title(&self) -> &str {
        &self.content.title
    }
    pub fn theme(&self) -> usize {
        self.content.theme
    }
    pub fn slides(&self) -> &[Slide] {
        &self.content.slides
    }
    pub fn len(&self) -> usize {
        self.content.slides.len()
    }
    pub fn is_empty(&self) -> bool {
        self.content.slides.is_empty()
    }
    pub fn current(&self) -> usize {
        self.current.min(self.len().saturating_sub(1))
    }
    pub fn slide(&self, index: usize) -> Option<&Slide> {
        self.content.slides.get(index)
    }
    pub fn current_slide(&self) -> &Slide {
        // A deck always holds at least one slide: `blank` makes one, `delete_slide` refuses the
        // last, and every load path rejects an empty file. This is that invariant, said once.
        &self.content.slides[self.current()]
    }
    /// Whether the content differs from what its file holds.
    pub fn dirty(&self) -> bool {
        self.saved.as_ref() != Some(&self.content)
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Slide titles, in order. The outline a `describe` reports and the outline export writes.
    pub fn outline(&self) -> Vec<String> {
        self.content.slides.iter().map(|s| s.title.clone()).collect()
    }

    /// The slides whose title, body or notes hold `query`, in order.
    pub fn search(&self, query: &str) -> Vec<usize> {
        if query.trim().is_empty() {
            return Vec::new();
        }
        self.content
            .slides
            .iter()
            .enumerate()
            .filter(|(_, s)| s.matches(query.trim()))
            .map(|(i, _)| i)
            .collect()
    }

    // ── Changing ──

    /// Apply a change, and record it for Undo only if it changed something.
    ///
    /// The guard is the whole reason this is one function: `commit` runs on every keystroke and
    /// again before every navigation, and without it half the undo stack would be steps that
    /// restore the state they were taken from.
    fn mutate<T>(&mut self, f: impl FnOnce(&mut Content) -> T) -> T {
        let before = self.content.clone();
        let out = f(&mut self.content);
        if before != self.content {
            self.undo.push(before);
            if self.undo.len() > UNDO_DEPTH {
                self.undo.remove(0);
            }
            self.redo.clear();
            self.revision += 1;
        }
        out
    }

    /// Write what is on the canvas back into the slide it came from.
    ///
    /// Everything that moves the selection calls this first, through [`Deck::go_to`]. Calling it
    /// directly is for the edit path, which does not move anything.
    pub fn commit(&mut self, canvas: &Edits) {
        let at = self.current();
        let layout = canvas.layout.clamp(0, LAYOUT_COUNT - 1);
        self.mutate(|c| {
            if let Some(slide) = c.slides.get_mut(at) {
                slide.title = canvas.title.clone();
                slide.body = canvas.body.clone();
                slide.notes = canvas.notes.clone();
                slide.layout = layout;
            }
        });
    }

    /// The one way the selection moves.
    ///
    /// Commit, move, and report which slide the caller must now draw. `on_next_slide` and
    /// `on_prev_slide` used to do the middle step alone: the index said slide 2 while the canvas
    /// still held slide 1's text, and the next thing that committed wrote slide 1 over slide 2.
    ///
    /// Out-of-range is clamped rather than refused: this is where a keystroke at the end of a
    /// deck arrives, and stopping at the last slide is what that should do.
    pub fn go_to(&mut self, canvas: &Edits, index: i64) -> usize {
        self.commit(canvas);
        let last = self.len().saturating_sub(1) as i64;
        self.current = index.clamp(0, last) as usize;
        self.current
    }

    /// Forward or back by `delta`, through [`Deck::go_to`].
    pub fn step(&mut self, canvas: &Edits, delta: i64) -> usize {
        self.go_to(canvas, self.current() as i64 + delta)
    }

    /// Add a slide after `after` (default: after the current one) and select it.
    pub fn add_slide(&mut self, canvas: &Edits, after: Option<usize>, slide: Slide) -> Result<usize, String> {
        if self.len() >= MAX_SLIDES {
            return Err(format!("this deck already holds {MAX_SLIDES} slides"));
        }
        self.commit(canvas);
        let at = (after.unwrap_or(self.current()) + 1).min(self.len());
        self.mutate(|c| c.slides.insert(at, slide));
        self.current = at;
        Ok(at)
    }

    /// Remove a slide. The last one stays: a deck with no slides has nothing to show, and
    /// nothing in this app could make the first one again.
    pub fn delete_slide(&mut self, index: usize) -> Result<Slide, String> {
        if self.len() <= 1 {
            return Err("a deck keeps at least one slide; this is the only one".into());
        }
        if index >= self.len() {
            return Err(format!("there is no slide {} in a deck of {}", index + 1, self.len()));
        }
        let gone = self.mutate(|c| c.slides.remove(index));
        self.current = index.min(self.len() - 1);
        Ok(gone)
    }

    /// Copy the current slide in after itself and select the copy.
    ///
    /// The commit comes first and the copy is taken after it, so Duplicate copies what the
    /// person can see rather than what the model last heard about: the same class of fault as
    /// the navigation bug, one button along.
    pub fn duplicate(&mut self, canvas: &Edits) -> Result<usize, String> {
        self.commit(canvas);
        let copy = self.current_slide().clone();
        self.add_slide(canvas, None, copy)
    }

    /// Move a slide to another position, and follow it with the selection.
    pub fn move_slide(&mut self, from: usize, to: usize) -> Result<usize, String> {
        if from >= self.len() {
            return Err(format!("there is no slide {} in a deck of {}", from + 1, self.len()));
        }
        let to = to.min(self.len() - 1);
        self.mutate(|c| {
            let slide = c.slides.remove(from);
            c.slides.insert(to, slide);
        });
        self.current = to;
        Ok(to)
    }

    /// Set any of one slide's three fields, leaving the others alone.
    pub fn set_slide(
        &mut self,
        index: usize,
        title: Option<&str>,
        body: Option<&str>,
        notes: Option<&str>,
    ) -> Result<(), String> {
        if index >= self.len() {
            return Err(format!("there is no slide {} in a deck of {}", index + 1, self.len()));
        }
        self.mutate(|c| {
            let slide = &mut c.slides[index];
            if let Some(t) = title {
                slide.title = t.to_string();
            }
            if let Some(b) = body {
                slide.body = b.to_string();
            }
            if let Some(n) = notes {
                slide.notes = n.to_string();
            }
        });
        Ok(())
    }

    pub fn set_layout(&mut self, index: usize, layout: i32) {
        let layout = layout.clamp(0, LAYOUT_COUNT - 1);
        self.mutate(|c| {
            if let Some(s) = c.slides.get_mut(index) {
                s.layout = layout;
            }
        });
    }

    /// Choose a theme, wrapping, so the Design tab's dropdown can simply count upwards.
    pub fn set_theme(&mut self, index: usize) -> usize {
        let index = index % THEMES.len();
        self.mutate(|c| c.theme = index);
        index
    }

    /// Step back, or forward again. Content only: the selection is clamped but not restored,
    /// because where a person was looking is not part of what they undid.
    pub fn undo(&mut self, redo: bool) -> bool {
        let (source, target) =
            if redo { (&mut self.redo, &mut self.undo) } else { (&mut self.undo, &mut self.redo) };
        let Some(content) = source.pop() else { return false };
        target.push(std::mem::replace(&mut self.content, content));
        self.current = self.current.min(self.len().saturating_sub(1));
        self.revision += 1;
        true
    }

    // ── Keeping it ──

    /// The deck's file now holds `written`: what "unsaved" is measured against from here on.
    /// Edits made while the write was in flight stay unsaved.
    pub fn saved_as(&mut self, written: Content) {
        self.saved = Some(written);
    }

    /// The file changed under a deck with nothing unsaved, and now holds `content`: the deck
    /// shows it, with no history of its own to undo into.
    pub fn followed(&mut self, content: Content) {
        let (current, revision) = (self.current, self.revision + 1);
        *self = Deck::opened(content);
        self.current = current.min(self.len().saturating_sub(1));
        self.revision = revision;
    }
}
