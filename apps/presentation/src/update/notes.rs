//! Speaker notes, from the companion.
//!
//! The one AI control that survives, and the reason it does: the answer has somewhere to land
//! that is not the deck itself, and it lands there unsaved so it is read before it is kept, the
//! same rule Notes and Document Editor follow. The other thirteen offered to rewrite or replace
//! slides through a proposal panel nothing produced; they are off the screen, with the reason at
//! the top of presentation.slint.
//!
//! The Mind may take a minute, and the person goes on working. The answer is for the slide it was
//! asked about: if that slide is no longer there, or no longer says what it said, the answer is
//! not written over some other slide's notes.

use yantrik_app::effects::Effects;

use super::{say, slides};
use crate::command::Command;
use crate::deck::Edits;
use crate::model::{Model, NotesAsk};
use crate::YPresent;

/// Said while the Mind is asked.
const ASKING: &str = "Asking the companion for speaker notes…";

/// Said when a second request is made before the first has been answered.
const STILL_ASKING: &str = "Still waiting for the companion's speaker notes.";

/// Said when the slide changed while the companion wrote.
const SLIDE_CHANGED: &str = "The slide changed while the companion was writing, so its notes were not added.";

/// The prompt for one slide.
fn prompt(title: &str, body: &str) -> String {
    format!(
        "Write speaker notes for one slide of a talk. Use only what the slide says; invent nothing. \
         Three or four sentences of prose, no heading and no bullet markers.\n\nTitle: {title}\n\nBody:\n{body}"
    )
}

/// Ask the Mind for the current slide's notes, in a job.
pub fn ask(m: &mut Model, fx: &mut Effects<YPresent>, canvas: &Edits) {
    if m.asking.is_some() {
        return say(m, STILL_ASKING);
    }
    if canvas.title.trim().is_empty() && canvas.body.trim().is_empty() {
        return say(m, "This slide has no text to write notes from.");
    }
    // What the person sees is what is asked about, and is in the deck before it goes.
    slides::typed(m, canvas);
    let asked = NotesAsk { doc: m.doc.id, index: m.deck().current(), title: canvas.title.clone(), body: canvas.body.clone() };
    say(m, ASKING);
    m.asking = Some(asked.clone());
    let (mind, ask_text) = (m.mind.clone(), prompt(&asked.title, &asked.body));
    let lost = Command::NotesWritten { ask: asked.clone(), result: Err("The companion did not answer.".into()) };
    fx.spawn_or(lost, move |_| Command::NotesWritten {
        ask: asked,
        result: mind.ask(&ask_text).map(|text| text.trim().to_string()).map_err(|e| e.to_string()),
    });
}

/// The Mind answered, or could not.
pub fn written(m: &mut Model, ask: NotesAsk, result: Result<String, String>) {
    if m.asking.as_ref() != Some(&ask) {
        return;
    }
    m.asking = None;
    if ask.doc != m.doc.id {
        return;
    }
    let text = match result {
        Ok(text) => text,
        Err(why) => return say(m, why),
    };
    let same_slide = m.deck().slide(ask.index).is_some_and(|s| s.title == ask.title && s.body == ask.body);
    if !same_slide {
        return say(m, SLIDE_CHANGED);
    }
    let _ = m.deck_mut().set_slide(ask.index, None, None, Some(&text));
    m.reload = m.deck().current() == ask.index;
    m.notice.clear();
}
