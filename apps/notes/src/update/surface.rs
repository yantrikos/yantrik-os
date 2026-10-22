//! The surface's calls (`app.act`), each doing what the same action in the window does, and
//! answered once its work has landed: with what the note now is on disk, or with a refusal that
//! is said in the window's notice too (contract point 4: failure is said twice).
//!
//! A call that only starts work records what it waits for in `Model::asked`; [`finish`] runs
//! after every command and answers it as soon as nothing of it is still running or waiting.

use serde_json::{json, Value};
use yantrik_app::effects::Effects;
use yantrik_app::intents::Origin;
use yantrik_app::Refused;

use super::editing::Undone;
use super::{editing, outside, recovery, relist, run, said, saving, window, windows};
use crate::command::{Act, Step, WindowAction};
use crate::describe::{listed, read_notes};
use crate::model::{Asked, Expect, Model, Pending};
use crate::note::Note;
use crate::store::{expand_home, validate};
use crate::view::render::{CHROME, CONTENT, FIELDS, STATUS};
use crate::Notes;

mod answer;

pub fn act(m: &mut Model, fx: &mut Effects<Notes>, act: Act) {
    m.answer = Value::Null;
    match act {
        Act::Refuse(sentence) => refuse(m, fx, sentence),
        Act::NewNote { title, body } => {
            if library_open(m, fx) {
                ask(m, fx, Expect::Written);
                save_then(m, fx, Step::NewNote { title, body });
            }
        }
        Act::OpenNote { wanted } => open_note(m, fx, &wanted),
        Act::SetTitle(title) => {
            let Some(text) = open_text(m, fx) else { return };
            commit(m, fx, retitled(&text, &title.replace(['\r', '\n'], " ")));
        }
        Act::Append(addition) => {
            let Some(text) = open_text(m, fx) else { return };
            commit(m, fx, text + &addition);
        }
        Act::SetContent(text) => {
            if open_text(m, fx).is_some() {
                commit(m, fx, text);
            }
        }
        Act::Search(query) => {
            m.pane.query = query.clone();
            fx.changed(CHROME);
            relist(m, fx);
            let (matches, titles) = listed(m);
            m.answer = json!({ "query": query, "folder": m.pane.folder, "matches": matches, "titles": titles });
        }
        Act::ReadNotes { query, skip } => m.answer = read_notes(&m.library, &query, skip),
        Act::SetFolder(folder) => {
            m.pane.folder = folder.clone();
            fx.changed(CHROME);
            relist(m, fx);
            let (listed, titles) = listed(m);
            m.answer = json!({ "folder": folder, "listed": listed, "titles": titles });
        }
        Act::Notebook(notebook) => {
            if open_text(m, fx).is_some() {
                let tags = m.pane.tags.clone();
                fields(m, fx, notebook, tags);
            }
        }
        Act::Tags(tags) => {
            if open_text(m, fx).is_some() {
                let notebook = m.pane.notebook.clone();
                fields(m, fx, notebook, tags);
            }
        }
        Act::Save => {
            if open_text(m, fx).is_some() {
                ask(m, fx, Expect::Saved);
                saving::save(m, fx, saving::How::Asked);
            }
        }
        Act::Trash => trash(m, fx),
        Act::Restore => restore(m, fx),
        Act::Copy => {
            let Some(from) = m.current().map(|c| c.id().to_string()) else {
                return refuse(m, fx, "No note is open, so there is nothing to copy.".into());
            };
            ask(m, fx, Expect::Copied { from });
            window::act(m, fx, WindowAction::Copy);
        }
        Act::Reload => {
            ask(m, fx, Expect::Reloaded);
            // Saved first as the window's Refresh is, except that a save that fails refuses the
            // call: nothing was reloaded. A read-only library reloads with no save first.
            let reload = Step::Window(WindowAction::Reload);
            if window::saves_first(m, &WindowAction::Reload) {
                save_then(m, fx, reload);
            } else {
                run(m, fx, reload);
            }
        }
        Act::Preview => {
            window::act(m, fx, WindowAction::Preview);
            m.answer = json!({
                "preview": m.pane.preview,
                "blocks": m.pane.blocks.len(),
                "title": m.current().map(|c| c.title()),
            });
        }
        Act::Focus => {
            window::act(m, fx, WindowAction::Focus);
            m.answer = json!({ "focus_mode": m.pane.focus_mode, "open_note": m.current().map(|c| c.title()) });
        }
        Act::Undo => step_history(m, fx, false),
        Act::Redo => step_history(m, fx, true),
        Act::Import { path } => {
            let from = expand_home(path.trim());
            if !from.is_absolute() {
                return refuse(m, fx, format!("`import` needs an absolute path; `{path}` is not one."));
            }
            if library_open(m, fx) {
                ask(m, fx, Expect::Imported { from });
                let agent = by_agent(fx);
                save_then(m, fx, Step::Import { path, agent });
            }
        }
        Act::OpenInNewWindow { wanted } => open_in_new_window(m, fx, &wanted),
        Act::RestoreDraft => recovered(m, fx, WindowAction::RestoreDraft),
        Act::DiscardDraft => recovered(m, fx, WindowAction::DiscardDraft),
        Act::KeepMineAsCopy => resolve(m, fx, WindowAction::KeepMine, false),
        Act::TakeTheirs => resolve(m, fx, WindowAction::TakeTheirs, true),
        Act::Compare => resolve(m, fx, WindowAction::Compare, true),
        Act::Export { path } => {
            let to = expand_home(path.trim());
            if !to.is_absolute() {
                return refuse(m, fx, format!("`export` needs an absolute path; `{path}` is not one."));
            }
            if open_text(m, fx).is_none() {
                return;
            }
            let text = m.shown().map(|c| c.text.clone()).unwrap_or_default();
            if library_open(m, fx) {
                ask(m, fx, Expect::Exported { to, text });
                // A read-only library would refuse the save first; the export writes the text as
                // it is in the window, outside the vault, which is how it gets out.
                let agent = by_agent(fx);
                if m.vault.read_only().is_some() {
                    run(m, fx, Step::Export { path, agent });
                } else {
                    save_then(m, fx, Step::Export { path, agent });
                }
            }
        }
    }
}

/// Whether the call being run is an agent's. Read here, as the call arrives: the step it asks
/// for runs once a save lands, which may be a save the person's typing started.
fn by_agent(fx: &Effects<Notes>) -> bool {
    matches!(fx.origin(), Origin::Surface { agent: true })
}

/// Answers the call waiting in `Model::asked`, once nothing of it runs or waits any more. Only
/// under that call's own invocation: it is the one the answer is owed to.
pub fn finish(m: &mut Model, fx: &mut Effects<Notes>) {
    let idle = m.busy.is_none() && m.pending.is_none();
    if !idle || m.asked.as_ref().is_none_or(|a| a.invocation != fx.invocation()) {
        return;
    }
    let Some(asked) = m.asked.take() else { return };
    // About the note of the window that was focused when the call came, wherever the focus is now
    // (the focused window's, should that one have closed meanwhile).
    let window = Some(asked.window).filter(|w| m.pane_of(*w).is_some()).unwrap_or(m.pane.window);
    let outcome = match asked.refused {
        Some(sentence) => Err(sentence),
        None => m
            .within(window, |m| answer::answer(m, asked.expect, asked.landed, asked.kept))
            .unwrap_or_else(|| Err(said::WINDOW_GONE.into())),
    };
    match outcome {
        Ok(answer) => {
            m.answer = answer;
            fx.settle(Ok(()));
        }
        Err(sentence) => refuse(m, fx, sentence),
    }
}

/// A refusal the person at the window sees too: the notice, and the call's error.
pub(super) fn refuse(m: &mut Model, fx: &mut Effects<Notes>, sentence: String) {
    m.notice = sentence.clone();
    fx.changed(STATUS);
    fx.settle(Err(Refused::new("refused", sentence)));
}

fn ask(m: &mut Model, fx: &Effects<Notes>, expect: Expect) {
    m.asked = Some(Asked {
        invocation: fx.invocation(),
        window: m.pane.window,
        expect,
        refused: None,
        landed: false,
        kept: None,
    });
}

/// A choice about an outside change, through the same command as the bar's button. Refused, in
/// the same words the window would use, when the open note has nothing to resolve.
fn resolve(m: &mut Model, fx: &mut Effects<Notes>, action: WindowAction, theirs: bool) {
    if let Some(sentence) = outside::unresolvable(m.current(), theirs) {
        return refuse(m, fx, sentence.into());
    }
    let Some(id) = m.current().map(|c| c.id().to_string()) else { return };
    let expect = match action {
        WindowAction::TakeTheirs => Expect::TookTheirs { id },
        WindowAction::Compare => Expect::Compared,
        _ => Expect::Copied { from: id },
    };
    ask(m, fx, expect);
    window::act(m, fx, action);
}

/// The recovery bar's choices, through the same commands as its buttons: refused, in the window's
/// words, when nothing is offered. A restore is answered once the text is back and saved (or its
/// conflict is offered), a discard with what is left.
fn recovered(m: &mut Model, fx: &mut Effects<Notes>, action: WindowAction) {
    let Some(offer) = recovery::shown(m).cloned() else {
        return refuse(m, fx, said::NOTHING_OFFERED.into());
    };
    let expect = match action {
        WindowAction::RestoreDraft => Expect::Recovered { id: offer.id },
        _ => Expect::Discarded { id: offer.id },
    };
    ask(m, fx, expect);
    window::act(m, fx, action);
}

/// The calls that leave the open note save it first, and need the library open to do so.
fn library_open(m: &mut Model, fx: &mut Effects<Notes>) -> bool {
    if !m.ready {
        refuse(m, fx, "The library is unavailable; resolve the storage error and run `reload`.".into());
    }
    m.ready
}

/// `step` once the open note is saved; if that save fails, the call is refused and nothing else
/// is done. An open note an outside edit blocks is refused at once, as a failed save is.
fn save_then(m: &mut Model, fx: &mut Effects<Notes>, step: Step) {
    if m.unsaved() {
        m.pending = Some(Pending { step, refuse_if_unsaved: true, window: m.pane.window });
        saving::save(m, fx, saving::How::Asked);
    } else {
        run(m, fx, step);
    }
}

/// The open note's text, or a refusal that says what to do about it: one sentence each for
/// "no note is open" and "that note is in the Trash", for every editing call.
fn open_text(m: &mut Model, fx: &mut Effects<Notes>) -> Option<String> {
    let refusal = match m.current() {
        None => "No note is open. Open one with `open_note`, or make one with `new_note`.",
        Some(c) if c.trash() => "The open note is in the Trash and cannot be edited; `restore` puts it back first.",
        Some(c) => return Some(c.text().to_string()),
    };
    refuse(m, fx, refusal.into());
    None
}

/// New text in the open note, saved, and answered with what the note then holds.
fn commit(m: &mut Model, fx: &mut Effects<Notes>, text: String) {
    if let Err(e) = validate(&text) {
        return refuse(m, fx, e);
    }
    editing::edit(m, fx, text);
    fx.changed(CONTENT);
    ask(m, fx, Expect::Written);
    saving::save(m, fx, saving::How::Asked);
}

/// The notebook or tags fields set as the window's own would be, then saved.
fn fields(m: &mut Model, fx: &mut Effects<Notes>, notebook: String, tags: String) {
    editing::metadata(m, fx, notebook, tags);
    fx.changed(FIELDS);
    ask(m, fx, Expect::Written);
    saving::save(m, fx, saving::How::Asked);
}

/// The note's first `# heading` line rewritten, or one added at the top. Line breaks are joined
/// by `\n` again, as they always were.
fn retitled(text: &str, title: &str) -> String {
    let mut replaced = false;
    let lines: Vec<String> = text
        .lines()
        .map(|l| {
            if !replaced && l.starts_with("# ") {
                replaced = true;
                format!("# {title}")
            } else {
                l.into()
            }
        })
        .collect();
    let joined = lines.join("\n");
    if replaced {
        joined
    } else {
        format!("# {title}\n\n{joined}")
    }
}

/// The note a call names, by its exact title or its file name; or the refusal that lists what the
/// library holds.
fn named(m: &Model, wanted: &str) -> Result<Note, String> {
    if let Some(note) = m.library.iter().find(|n| n.id == wanted || n.title() == wanted) {
        return Ok(note.clone());
    }
    let titles: Vec<String> =
        m.library.iter().filter(|n| !n.trash).take(8).map(|n| format!("\"{}\"", n.title())).collect();
    let held = if titles.is_empty() { "nothing yet".to_string() } else { titles.join(", ") };
    Err(format!("No note is called \"{wanted}\". The library holds: {held}"))
}

fn open_note(m: &mut Model, fx: &mut Effects<Notes>, wanted: &str) {
    let (id, trash) = match named(m, wanted) {
        Ok(note) => (note.id, note.trash),
        Err(sentence) => return refuse(m, fx, sentence),
    };
    if library_open(m, fx) {
        ask(m, fx, Expect::Written);
        save_then(m, fx, Step::Window(WindowAction::Open { id, trash }));
    }
}

/// `open_in_new_window`: the named note, or the focused window's, in a window of its own, through
/// the command the window's button runs. Nothing is saved first: no window leaves its note. The
/// answer names the new window, which is the focused one now.
fn open_in_new_window(m: &mut Model, fx: &mut Effects<Notes>, wanted: &str) {
    let note = match wanted {
        "" => None,
        wanted => match named(m, wanted) {
            Ok(note) => Some(note),
            Err(sentence) => return refuse(m, fx, sentence),
        },
    };
    if !library_open(m, fx) {
        return;
    }
    match windows::open(m, fx, note) {
        Ok(window) => {
            m.answer = json!({
                "window": window.0,
                "open_note": m.current().map(|c| c.id().to_string()),
                "title": m.current().map(|c| c.title()),
                "windows": m.windows().len(),
            });
        }
        Err(sentence) => refuse(m, fx, format!("{sentence} {}", said::NAME_A_NOTE)),
    }
}

fn trash(m: &mut Model, fx: &mut Effects<Notes>) {
    let refusal = match m.current() {
        None => Some("No note is open, so there is nothing to trash."),
        Some(c) if c.trash() => Some("That note is already in the Trash."),
        Some(_) => None,
    };
    if let Some(sentence) = refusal {
        return refuse(m, fx, sentence.into());
    }
    let Some(expect) = m.current().map(|c| Expect::Trashed {
        id: c.id().to_string(),
        title: c.title(),
        path: m.vault.dir().join(c.id()),
    }) else {
        return;
    };
    ask(m, fx, expect);
    window::act(m, fx, WindowAction::Trash);
}

fn restore(m: &mut Model, fx: &mut Effects<Notes>) {
    let id = match m.current() {
        None => return refuse(m, fx, "No note is open, so there is nothing to restore.".into()),
        Some(c) if !c.trash() => {
            return refuse(
                m,
                fx,
                "That note is in the library already; only a trashed note can be restored. Open it \
                 from `set_folder trash` first."
                    .into(),
            )
        }
        Some(c) => c.id().to_string(),
    };
    ask(m, fx, Expect::Restored { id });
    window::act(m, fx, WindowAction::Restore);
}

fn step_history(m: &mut Model, fx: &mut Effects<Notes>, redo: bool) {
    let Some(before) = open_text(m, fx) else { return };
    if editing::undo(m, fx, redo) == Undone::Dropped {
        return refuse(m, fx, said::UNDO_DROPPED.into());
    }
    if m.current().is_some_and(|c| c.text() == before) {
        let sentence = if redo {
            "There is nothing to redo on this note."
        } else {
            "There is nothing left to undo on this note in this window."
        };
        return refuse(m, fx, sentence.into());
    }
    ask(m, fx, Expect::Written);
    saving::save(m, fx, saving::How::Asked);
}
