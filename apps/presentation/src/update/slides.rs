//! Changing the deck: what typing, the buttons and the surface all do to its slides. Every one
//! commits the canvas first, so the sentence being typed into one slide is never undone by a
//! change to another, and every one puts the slide it leaves selected back on the canvas.

use crate::deck::{Edits, Slide};
use crate::model::Model;

/// The person typed in the canvas: the slide catches up, the deck starts saying it has unsaved
/// changes, and the draft is owed. The canvas is not reloaded: it is the one path where it must
/// not be, or the cursor would jump to the start of the field on every keystroke.
pub fn typed(m: &mut Model, canvas: &Edits) {
    m.deck_mut().commit(canvas);
}

/// The one way the selection moves: commit what is on the canvas, move, draw the slide moved to.
pub fn go_to(m: &mut Model, canvas: &Edits, index: i64) -> usize {
    m.reload = true;
    m.deck_mut().go_to(canvas, index)
}

/// Add a slide after `after` (or after the current one) and select it.
pub fn add(m: &mut Model, canvas: &Edits, after: Option<usize>, slide: Slide) -> Result<usize, String> {
    m.reload = true;
    m.deck_mut().add_slide(canvas, after, slide)
}

/// The slide the Add button makes.
pub fn next_blank(m: &Model) -> Slide {
    Slide::new(format!("Slide {}", m.deck().len() + 1), "", 1)
}

/// Remove a slide. The canvas is committed first: deleting slide 3 must not also throw away the
/// sentence being typed into slide 2.
pub fn delete(m: &mut Model, canvas: &Edits, index: usize) -> Result<Slide, String> {
    m.reload = true;
    m.deck_mut().commit(canvas);
    m.deck_mut().delete_slide(index)
}

pub fn duplicate(m: &mut Model, canvas: &Edits) -> Result<usize, String> {
    m.reload = true;
    m.deck_mut().duplicate(canvas)
}

pub fn move_slide(m: &mut Model, canvas: &Edits, from: usize, to: usize) -> Result<usize, String> {
    m.reload = true;
    m.deck_mut().commit(canvas);
    m.deck_mut().move_slide(from, to)
}

/// Change the text of one slide. The canvas is committed first, or editing slide 5 through the
/// surface would be undone by the next commit of whatever slide the person is looking at.
pub fn set_slide(
    m: &mut Model,
    canvas: &Edits,
    index: usize,
    title: Option<&str>,
    body: Option<&str>,
    notes: Option<&str>,
) -> Result<(), String> {
    m.reload = true;
    m.deck_mut().commit(canvas);
    m.deck_mut().set_slide(index, title, body, notes)
}

/// Step back or forward over the deck's content, the canvas committed first so the run of typing
/// just done is the thing that comes back.
pub fn undo(m: &mut Model, canvas: &Edits, redo: bool) -> bool {
    m.reload = true;
    m.deck_mut().commit(canvas);
    m.deck_mut().undo(redo)
}
