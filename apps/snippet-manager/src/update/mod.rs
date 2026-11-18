//! The only code that changes the model: every button, intent, launch and job result lands here
//! as a `Command`, and asks for its effects.
//!
//! The person's button and the Mind's action are one path wherever they do the same thing, so
//! neither can report an outcome the other would not. Every write of the library, every copy to
//! the clipboard and every file read or written runs in a job; what it came to comes back here as
//! a command.

use yantrik_app::effects::Effects;
use yantrik_app::Refused;

use crate::command::{Command, Work};
use crate::model::{Model, NOTHING, PART_NOTICE};
use crate::SnippetManager;

mod browsing;
mod changes;
mod copying;
pub(crate) mod drafts;
mod ending;
mod files;
mod written;

#[cfg(test)]
pub(crate) use copying::COPY_FEEDBACK;
#[cfg(test)]
pub(crate) use ending::CLOSE_BACKSTOP;

type Fx = Effects<SnippetManager>;

/// What a change asked for while the app is closing is told.
pub const CLOSING: &str = "Snippets is closing, so nothing more is written.";

pub fn update(m: &mut Model, command: Command, fx: &mut Fx) -> Result<(), Refused> {
    // Every command names what it changed; this one names nothing until it says otherwise.
    fx.changed(NOTHING);
    if matches!(&command, Command::Launch(files) if !files.is_empty()) && m.closing && !m.ending {
        // A second launch while a close waits: the framework opens the window again right after
        // (`Event::Presented`), so the close is over and the file it hands over is taken.
        m.closing = false;
    }
    yantrik_app::for_mind::refuse_if(m.for_mind, crate::mind_view::refuses(&command))?;
    if (m.ending || m.closing) && starts_work(m, &command) {
        return Err(Refused::new("closing", CLOSING));
    }
    if lands_work(&command) {
        m.work = m.work.saturating_sub(1);
    }
    dispatch(m, command, fx)?;
    drafts::keep(m, fx, false);
    // The end waits for the work already started, so none of it is lost to a close or a logout:
    // it is each job's own result that says what happened, and the app ends after the last.
    if (m.ending || m.closing) && m.work == 0 {
        fx.exit();
    }
    Ok(())
}

fn dispatch(m: &mut Model, command: Command, fx: &mut Fx) -> Result<(), Refused> {
    match command {
        Command::Search(_)
        | Command::TagFilter(_)
        | Command::Select(_)
        | Command::Edited(_)
        | Command::CollectionSelect(_)
        | Command::ContextActivated(_)
        | Command::Open(_)
        | Command::Find(_)
        | Command::Show => browsing::update(m, command, fx),
        Command::New
        | Command::Save(_)
        | Command::Delete(_)
        | Command::ToggleFavorite(_)
        | Command::SetLanguage(_)
        | Command::CollectionCreate(_)
        | Command::CollectionRename(..)
        | Command::CollectionDelete(_)
        | Command::Add { .. }
        | Command::Change { .. }
        | Command::DeleteNamed(_)
        | Command::FavoriteNamed(_) => changes::update(m, command, fx),
        Command::Copy(_) | Command::CopyNamed(_) | Command::Copied(_) | Command::CopyFeedbackOver => {
            copying::update(m, command, fx)
        }
        Command::Import { .. }
        | Command::Launch(_)
        | Command::Read(_)
        | Command::ExportAll
        | Command::Exported(_) => files::update(m, command, fx),
        Command::Written(_) | Command::Reloaded(_) => {
            written::update(m, command, fx);
            Ok(())
        }
        Command::WorkFailed(work) => {
            work_failed(m, *work, fx);
            Ok(())
        }
        Command::AiExplain => {
            if m.open().is_none() {
                say(m, fx, "Open a snippet before asking about it.");
                return Ok(());
            }
            // Said on the notice line, where the button is, and not pretended into an answer.
            if let Some(hint) = m.rail.reach().hint() {
                say(m, fx, hint);
                return Ok(());
            }
            // The same path the rail's suggestion takes, so the two cannot come to differ.
            yantrik_app::rail::update::<SnippetManager>(
                m,
                yantrik_app::rail::RailCommand::Activate(crate::rail::EXPLAIN.into()),
                fx,
            )
        }
        Command::Rail(rail) => yantrik_app::rail::update::<SnippetManager>(m, rail, fx),
        Command::Closing(window) => {
            ending::closing(m, window, fx);
            Ok(())
        }
        Command::Ending => {
            ending::ending(m, fx);
            Ok(())
        }
        Command::Reopened => {
            ending::reopened(m, fx);
            Ok(())
        }
        Command::DraftNotKept { name, why } => {
            drafts::not_drafted(m, fx, &name, &why);
            Ok(())
        }
        Command::ShowOffer | Command::RestoreOffer | Command::DiscardOffer => {
            drafts::choose(m, command, fx);
            Ok(())
        }
    }
}

/// A command that starts work the end would have to wait for.
fn starts_work(m: &Model, command: &Command) -> bool {
    match command {
        Command::New
        | Command::Save(_)
        | Command::Delete(_)
        | Command::Copy(_)
        | Command::ToggleFavorite(_)
        | Command::CollectionCreate(_)
        | Command::CollectionRename(..)
        | Command::CollectionDelete(_)
        | Command::ExportAll
        | Command::Add { .. }
        | Command::Change { .. }
        | Command::CopyNamed(_)
        | Command::DeleteNamed(_)
        | Command::FavoriteNamed(_)
        | Command::Import { .. }
        | Command::RestoreOffer => true,
        Command::SetLanguage(_) => m.editor.id >= 0,
        // Moving to another snippet keeps the typing first, which is a write.
        Command::Select(_) | Command::Open(_) | Command::ContextActivated(_) => m.editor_dirty(),
        Command::Launch(files) => !files.is_empty(),
        _ => false,
    }
}

/// A command that carries the outcome of counted work back.
fn lands_work(command: &Command) -> bool {
    match command {
        Command::Written(_) | Command::Copied(_) | Command::Read(_) | Command::Exported(_) => true,
        Command::WorkFailed(work) => !matches!(**work, Work::Reload),
        _ => false,
    }
}

/// A job ended without its result. What it was doing is said; nothing it would have reported is
/// claimed.
fn work_failed(m: &mut Model, work: Work, fx: &mut Fx) {
    match work {
        Work::Write(write) => {
            m.lane.landed();
            drafts::not_landed(m, write.ticket.revision);
            written::did_not_finish(m, write.after, &write.restore, fx);
        }
        Work::Copy { from_window } => {
            m.copied = false;
            fx.changed(crate::model::PART_COPY);
            let why = "The copy did not finish; nothing is known to be on the clipboard.";
            if from_window {
                say(m, fx, why);
            }
            fx.settle(Err(Refused::new("copy_failed", why)));
        }
        Work::Import => fail(m, fx, "Reading the file did not finish; nothing was imported."),
        Work::Export => fail(m, fx, "The export did not finish; try it again."),
        // The file could not be read again; memory stays as it is, and every draft with it until
        // a write holding its text has landed (`drafts::release`).
        Work::Reload => {}
    }
}

/// A sentence on the notice line.
fn say(m: &mut Model, fx: &mut Fx, sentence: &str) {
    m.notice = sentence.to_string();
    fx.changed(PART_NOTICE);
}

/// Something that went wrong, said twice: on the notice line, where the person sees it, and as the
/// refusal of the call this command runs under, where a mind reads it.
fn fail(m: &mut Model, fx: &mut Fx, why: &str) {
    tracing::warn!(error = %why, "snippet action failed");
    say(m, fx, why);
    fx.settle(Err(Refused::new("not_done", why)));
}

/// What worked clears the notice: by then it describes a world that has moved on.
fn worked(m: &mut Model, fx: &mut Fx) {
    if !m.notice.is_empty() {
        m.notice.clear();
        fx.changed(PART_NOTICE);
    }
}
