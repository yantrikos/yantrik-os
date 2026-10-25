//! A call from the surface, carried out, and its answer.
//!
//! Every answer is built from that call's own work, when it committed: the tab it acted on, the
//! file it wrote. What the disk holds is read back in a job (files/answer.rs) and the call is
//! settled when that lands, so nothing the person does meanwhile, and no other call, becomes this
//! call's answer. A refusal is said twice, as it always was: to the caller, and in the window's
//! notice, which is `describe.notice`.

use serde_json::Value;
use yantrik_app::effects::Effects;
use yantrik_app::Refused;

use crate::caller::Caller;
use crate::command::{Act, AfterSave, Command};
use crate::files::answer::{read_back, ReadBack};
use crate::model::Model;
use crate::prompt;
use crate::TextEditor;

pub mod answers;
mod documents;
mod text;

/// What an agent is told when it reaches for a tab it is not shown.
pub const HIDDEN_REFUSAL: &str = "The tab in front holds a file an agent is not shown, and its text is left \
                                  alone; `select_tab` another or `new` one.";

pub fn act(m: &mut Model, fx: &mut Effects<TextEditor>, act: Act, caller: Caller) {
    let agent = caller.agent;
    let done = match act {
        Act::New { text } => documents::new(m, fx, text, &caller),
        Act::Open { path } => documents::open(m, fx, path, caller),
        Act::Read { from_line, tab } => documents::read(m, from_line, tab, &caller),
        Act::Save => documents::save(m, fx, caller),
        Act::SaveAs { path, overwrite } => documents::save_as(m, fx, path, overwrite, caller),
        Act::Show => documents::show(m, fx, &caller),
        Act::Close => documents::close(m, fx, &caller),
        Act::Cancel => documents::cancel(m, fx, &caller),
        Act::Discard => documents::discard(m, fx, &caller),
        Act::Find { text } => text::find(m, text, &caller),
        Act::FindNext => text::step(m, true, &caller),
        Act::FindPrev => text::step(m, false, &caller),
        Act::ReplaceText { text } => text::replace_text(m, text, &caller),
        Act::Replace => text::replace(m, fx, false, &caller),
        Act::ReplaceAll => text::replace(m, fx, true, &caller),
        Act::SetContent { text } => text::set_content(m, fx, text, &caller),
        Act::Append { text } => text::append(m, fx, text, &caller),
        Act::Undo => text::history(m, fx, false, &caller),
        Act::Redo => text::history(m, fx, true, &caller),
        Act::SelectTab { index } => documents::select_tab(m, fx, index, &caller),
        Act::Refuse(sentence) => Err(sentence),
    };
    match done {
        Ok(Some(answer)) => answer_with(m, fx, answer, agent),
        // Answered when its job lands.
        Ok(None) => {}
        Err(sentence) => refuse(m, fx, sentence),
    }
}

/// A refusal the person at the window sees too: the notice, and the call's error.
pub fn refuse(m: &mut Model, fx: &mut Effects<TextEditor>, sentence: impl Into<String>) {
    let sentence = sentence.into();
    m.notice.set(sentence.clone());
    fx.settle(Err(Refused::new("refused", sentence)));
}

/// Refuses the call a save was for, if it was a call's, with the notice the landing already said,
/// in the form this caller may read.
pub fn refuse_if_owed(m: &Model, fx: &mut Effects<TextEditor>, then: &AfterSave) {
    let agent = match then {
        AfterSave::Window => return,
        AfterSave::Save { caller, .. } | AfterSave::SaveAs { caller, .. } => caller.agent,
    };
    fx.settle(Err(Refused::new("refused", m.notice.read(agent).to_string())));
}

/// The answer, once what the disk holds is read into it: at once when nothing is to be read.
///
/// While a read-back is out, a close or an exit already decided waits for it (`Model::finish`):
/// a call whose own work closed the window is still answered from that work.
pub fn answer_with(m: &mut Model, fx: &mut Effects<TextEditor>, answer: ReadBack, agent: bool) {
    if answer.fills.is_empty() {
        m.answer = answer.value;
        return fx.settle(Ok(()));
    }
    m.answers_pending += 1;
    let lost = Command::Answered { value: Value::Null, failed: Some("The answer could not be read back.".into()), agent };
    fx.spawn_or(lost, move |_| {
        let (value, failed) = read_back(answer);
        Command::Answered { value, failed, agent }
    });
}

/// The disk was read into an answer: the call is settled with it, or refused when the disk did
/// not hold what it was owed, in the words of what went wrong when the notice says so.
pub fn answered(m: &mut Model, fx: &mut Effects<TextEditor>, value: Value, failed: Option<String>, agent: bool) {
    m.answers_pending = m.answers_pending.saturating_sub(1);
    match failed {
        Some(otherwise) => {
            let said = m.notice.read(agent);
            let why = if said.is_empty() || said == "Saved" { otherwise } else { said.to_string() };
            fx.settle(Err(Refused::new("refused", why)));
        }
        None => {
            m.answer = value;
            fx.settle(Ok(()));
        }
    }
}

/// Refuse while a dialog is waiting for an answer, naming the answer it wants. The window ignores
/// almost every action while a dialog is up, and a caller told `accepted` for an action the window
/// had dropped is the fault this closes.
pub fn no_dialog(m: &Model, name: &str) -> Result<(), String> {
    match prompt::dialog(m) {
        0 => Ok(()),
        prompt::PROMPT => Err(format!(
            "`{name}` cannot run while the editor is asking about unsaved changes ({}). Answer it first: \
             `save` writes the tab and closes it, `discard` throws the changes away, `cancel` leaves it \
             open.",
            // In the notice, which anyone's `describe` reads next: a hidden tab is not named (#443).
            prompt::label(m, true)
        )),
        other => Err(format!(
            "`{name}` cannot run while the {} dialog is open; `cancel` closes it.",
            match other {
                prompt::OPEN => "Open file",
                prompt::SAVE_AS => "Save As",
                _ => "Go to line",
            }
        )),
    }
}

/// Refuse an agent anything that reads or changes the tab in front when that tab is one the
/// person opened from a place an agent may not read (#443). Hiding `content` in `describe` was not
/// enough: `find` counts matches, `save_as` copies the text somewhere readable, and `append` then
/// `save` writes it back. The refusal does not name the file: the notice it leaves is in
/// `describe` too.
pub fn not_hidden(m: &Model, caller: &Caller) -> Result<(), String> {
    if caller.agent && m.active_tab().hidden {
        return Err(HIDDEN_REFUSAL.into());
    }
    Ok(())
}
