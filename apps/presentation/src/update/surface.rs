//! A call from the surface, carried out, and its answer.
//!
//! Every answer is built from that call's own work, when it committed: the file it read or wrote,
//! the slide it changed. Nothing the person does meanwhile, and no other call, becomes this
//! call's answer: a call that starts a job is settled when that job lands, with what the job
//! found. A refusal is said twice, as it always was: to the caller, and in the window's notice,
//! which is `describe.notice`.

use std::path::PathBuf;

use serde_json::{json, Value};
use yantrik_app::effects::Effects;
use yantrik_app::Refused;

use super::saving::Target;
use super::{opening, saving, slides};
use crate::agent_rule::{HIDDEN, HIDDEN_REFUSAL};
use crate::caller::Caller;
use crate::command::{Act, AfterSave, Cause, Index, Next};
use crate::deck::{Edits, Slide};
use crate::files::names::{expanded, export_path};
use crate::model::Model;
use crate::YPresent;

type Done = Result<Option<Value>, String>;

pub fn act(m: &mut Model, fx: &mut Effects<YPresent>, act: Act, caller: Caller) {
    let done = match act {
        Act::Open { path } => open(m, fx, &path, caller),
        Act::Show => show(m, caller),
        Act::Save => save(m, fx, caller),
        Act::SaveAs { path, overwrite } => save_as(m, fx, &path, overwrite, caller),
        Act::NewDeck { title } => new_deck(m, fx, title, caller),
        Act::AddSlide { title, body, notes, after } => add_slide(m, caller, title, body, notes, after),
        Act::SetSlide { index, title, body, notes } => set_slide(m, caller, index, title, body, notes),
        Act::DeleteSlide { index } => delete_slide(m, caller, index),
        Act::MoveSlide { from, to } => move_slide(m, caller, from, to),
        Act::GoTo { index } => go_to(m, caller, index),
        Act::Next => step(m, caller, 1),
        Act::Previous => step(m, caller, -1),
        Act::PresenterView { mode } => presenter_view(m, caller, &mode),
        Act::ExportMarkdown { path } => export_markdown(m, fx, path, caller),
    };
    match done {
        Ok(Some(value)) => answer(m, fx, value),
        // Answered when its job lands.
        Ok(None) => {}
        Err(sentence) => refuse(m, fx, sentence),
    }
}

/// The call answered with `value`, in the same command that committed its work. A success clears
/// the notice, so what is on screen is always about the last thing that happened.
pub fn answer(m: &mut Model, fx: &mut Effects<YPresent>, value: Value) {
    m.notice.clear();
    m.answer = value;
    fx.settle(Ok(()));
}

/// A refusal the person at the window sees too: the notice, and the call's error.
pub fn refuse(m: &mut Model, fx: &mut Effects<YPresent>, sentence: impl Into<String>) {
    let sentence = sentence.into();
    tracing::warn!(error = %sentence, "yPresent refused an action");
    m.notice.set(sentence.clone());
    settle_refused(fx, sentence);
}

/// The call refused with `sentence`, when the notice already says it in the form its caller may
/// read.
pub fn settle_refused(fx: &mut Effects<YPresent>, sentence: impl Into<String>) {
    fx.settle(Err(Refused::new("refused", sentence.into())));
}

/// Refuse an agent anything that reads or changes the deck in front when it is one the person
/// opened from a place an agent may not read (#443). Hiding the content in `describe` was not
/// enough: `go_to` tells which slide is which, `save_as` copies the deck somewhere readable, and
/// `add_slide` then `save` writes it back. The refusal does not name the file: the notice it
/// leaves is in `describe` too.
fn not_hidden(m: &Model, caller: Caller) -> Result<(), String> {
    if caller.agent && m.doc.hidden {
        return Err(HIDDEN_REFUSAL.into());
    }
    Ok(())
}

/// The slide the deck holds, as the canvas would show it: a call changes the deck, and the window
/// has nothing typed into it that the deck has not already taken.
fn own_canvas(m: &Model) -> Edits {
    Edits::from(m.deck().current_slide())
}

/// A slide number out of a call, or why it is not one.
fn index(arg: &Index) -> Result<usize, String> {
    arg.clone()
}

/// Where the deck now is. One shape, so a caller never has to learn a different one per verb, and
/// every field is read back out of the deck after the change rather than composed from the
/// arguments that went in.
fn position(m: &Model) -> Value {
    json!({
        "index": m.deck().current(), "title": m.deck().current_slide().title,
        "slides": m.deck().len(), "dirty": m.modified(),
    })
}

/// `{"added_at": 2, …position}`: the position with one more key.
fn with(mut answer: Value, key: &str, value: Value) -> Value {
    answer[key] = value;
    answer
}

fn path_arg(raw: &str) -> Result<PathBuf, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("`path` is empty".into());
    }
    Ok(expanded(raw))
}

/// `open`: the file read in a job, and the answer built from what it found. A deck with unsaved
/// changes is written to a file of its own before it is replaced.
fn open(m: &mut Model, fx: &mut Effects<YPresent>, path: &str, caller: Caller) -> Done {
    let target = path_arg(path)?;
    opening::start(m, fx, target, Cause::Surface(caller), caller);
    Ok(None)
}

/// `show`: the window is the framework's to bring forward (`present`); this answers that it is
/// there, or says it is not.
fn show(m: &Model, caller: Caller) -> Done {
    if m.lingering {
        return Err("yPresent's window is closed while it finishes a save; launch it again to bring the window back.".into());
    }
    let showing = if caller.agent && m.doc.hidden { HIDDEN } else { m.title() };
    Ok(Some(json!({ "showing": showing })))
}

/// `save`: the deck written to its file, or, for a deck with no file, to a fresh one in the
/// default folder.
fn save(m: &mut Model, fx: &mut Effects<YPresent>, caller: Caller) -> Done {
    // A deck whose file's text was never read (it came back from the old recovery file) is kept in
    // a file of its own rather than written onto one that may have changed.
    let target = if m.path().is_some() && m.doc.on_disk.is_some() { Target::Own } else { Target::Unnamed };
    if caller.agent && m.doc.hidden && !matches!(target, Target::Own) {
        return Err(HIDDEN_REFUSAL.into());
    }
    saving::start(m, fx, target, false, AfterSave::Save(caller), caller.agent);
    Ok(None)
}

/// `save_as`: the deck written to this path, and kept there from then on.
fn save_as(m: &mut Model, fx: &mut Effects<YPresent>, path: &str, overwrite: bool, caller: Caller) -> Done {
    // The content written is the deck's: saving one the person opened from outside the home into
    // the home would put it where `describe` reads it back.
    not_hidden(m, caller)?;
    let target = path_arg(path)?;
    saving::start(m, fx, Target::Path(target), overwrite, AfterSave::SaveAs(caller), caller.agent);
    Ok(None)
}

/// `new_deck`: an empty deck. Whatever was open is written to a file of its own first, so it
/// never silently ends a deck someone was in the middle of.
fn new_deck(m: &mut Model, fx: &mut Effects<YPresent>, title: Option<String>, caller: Caller) -> Done {
    let asked = title.as_deref().unwrap_or("Untitled").trim().to_string();
    let title = if asked.is_empty() { "Untitled".to_string() } else { asked };
    opening::replace(m, fx, Next::Blank(title), Cause::Surface(caller));
    Ok(None)
}

fn add_slide(
    m: &mut Model,
    caller: Caller,
    title: Option<String>,
    body: Option<String>,
    notes: Option<String>,
    after: Option<Index>,
) -> Done {
    not_hidden(m, caller)?;
    let after = after.as_ref().map(index).transpose()?;
    let title = title.unwrap_or_else(|| format!("Slide {}", m.deck().len() + 1));
    let mut slide = Slide::new(title, body.unwrap_or_default(), 1);
    slide.notes = notes.unwrap_or_default();
    let canvas = own_canvas(m);
    let at = slides::add(m, &canvas, after, slide)?;
    Ok(Some(with(position(m), "added_at", json!(at))))
}

fn set_slide(
    m: &mut Model,
    caller: Caller,
    at: Index,
    title: Option<String>,
    body: Option<String>,
    notes: Option<String>,
) -> Done {
    not_hidden(m, caller)?;
    let at = index(&at)?;
    if title.is_none() && body.is_none() && notes.is_none() {
        return Err("give at least one of `title`, `body` or `notes`".into());
    }
    let canvas = own_canvas(m);
    slides::set_slide(m, &canvas, at, title.as_deref(), body.as_deref(), notes.as_deref())?;
    let slide = m.deck().slide(at).ok_or("the slide went away")?;
    Ok(Some(json!({
        "index": at, "title": slide.title, "body": slide.body, "notes": slide.notes, "dirty": m.modified(),
    })))
}

/// Graded sensitive because it destroys writing; see the intent.
fn delete_slide(m: &mut Model, caller: Caller, at: Index) -> Done {
    not_hidden(m, caller)?;
    let at = index(&at)?;
    let canvas = own_canvas(m);
    let gone = slides::delete(m, &canvas, at)?;
    Ok(Some(with(position(m), "deleted", json!(gone.title))))
}

fn move_slide(m: &mut Model, caller: Caller, from: Index, to: Index) -> Done {
    not_hidden(m, caller)?;
    let (from, to) = (index(&from)?, index(&to)?);
    let canvas = own_canvas(m);
    let landed = slides::move_slide(m, &canvas, from, to)?;
    Ok(Some(with(position(m), "moved_to", json!(landed))))
}

fn go_to(m: &mut Model, caller: Caller, at: Index) -> Done {
    not_hidden(m, caller)?;
    let at = index(&at)?;
    let len = m.deck().len();
    if at >= len {
        return Err(format!("there is no slide {} in a deck of {len}", at + 1));
    }
    let canvas = own_canvas(m);
    slides::go_to(m, &canvas, at as i64);
    Ok(Some(position(m)))
}

fn step(m: &mut Model, caller: Caller, delta: i64) -> Done {
    not_hidden(m, caller)?;
    let at = m.deck().current() as i64;
    let canvas = own_canvas(m);
    slides::go_to(m, &canvas, at + delta);
    Ok(Some(position(m)))
}

/// The presenter-view state a `mode` word asks for. `fullscreen` is the natural word for
/// slideshow mode and what a model guesses when asked to present the deck (issue #681), so it
/// joins `on`; `exit` and `windowed` join `off`. The refusal lists every accepted word so a caller
/// that guessed wrong sees the whole set in the error.
pub fn presenting_mode(asked: &str, currently: bool) -> Result<bool, String> {
    match asked {
        "on" | "true" | "start" | "fullscreen" => Ok(true),
        "off" | "false" | "stop" | "exit" | "windowed" => Ok(false),
        "toggle" => Ok(!currently),
        other => Err(format!(
            "unknown mode `{other}`; use on, fullscreen, true, start, off, windowed, exit, false, stop or toggle"
        )),
    }
}

fn presenter_view(m: &mut Model, caller: Caller, mode: &str) -> Done {
    not_hidden(m, caller)?;
    let wanted = presenting_mode(&mode.trim().to_lowercase(), m.presenting)?;
    // Through the same commit as everything else, so entering and leaving both save what is on
    // the canvas: the presenter view reads the model, not the editor's properties.
    let at = m.deck().current() as i64;
    let canvas = own_canvas(m);
    slides::go_to(m, &canvas, at);
    m.presenting = wanted;
    Ok(Some(with(position(m), "presenting", json!(m.presenting))))
}

/// `export_markdown`: a copy written to a path, in a job, the deck left in its own file. With no
/// path it goes beside the deck, with the deck's stem.
fn export_markdown(m: &mut Model, fx: &mut Effects<YPresent>, path: Option<String>, caller: Caller) -> Done {
    not_hidden(m, caller)?;
    let (target, default_name) = match path {
        Some(raw) => (path_arg(&raw)?, false),
        None => (export_path(m.path(), &m.deck_dir, m.title(), "md"), true),
    };
    saving::export(m, fx, target, "md", default_name, Cause::Surface(caller), caller.agent);
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::presenting_mode;

    #[test]
    fn mode_words_set_the_presenter_view() {
        // `fullscreen` is the word a model guesses for "present the deck" (issue #681); `exit`
        // and `windowed` are its mirror for leaving.
        assert_eq!(presenting_mode("fullscreen", false), Ok(true));
        assert_eq!(presenting_mode("on", false), Ok(true));
        assert_eq!(presenting_mode("exit", true), Ok(false));
        assert_eq!(presenting_mode("windowed", true), Ok(false));
        assert_eq!(presenting_mode("off", true), Ok(false));
        assert_eq!(presenting_mode("toggle", true), Ok(false));
        assert_eq!(presenting_mode("toggle", false), Ok(true));
    }

    #[test]
    fn unknown_mode_refusal_lists_every_accepted_word() {
        let err = presenting_mode("zoom", false).unwrap_err();
        for word in ["on", "fullscreen", "true", "start", "off", "windowed", "exit", "false", "stop", "toggle"] {
            assert!(err.contains(word), "refusal `{err}` omits `{word}`");
        }
    }
}
