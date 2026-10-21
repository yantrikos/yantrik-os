//! The only code that changes the model: every button, intent, timer and job result lands here
//! as a [`Command`], and asks for its effects. Nothing here performs I/O; the mail service and
//! the companion are asked from jobs, and their answers come back as commands.

use yantrik_app::effects::Effects;
use yantrik_app::Refused;

use crate::command::{Command, Work};
use crate::model::{part, Model};
use crate::Email;

mod ai;
mod compose;
pub(crate) mod drafts;
pub(crate) mod google;
mod mailbox;
mod message;
mod pick;
mod setup;

pub fn update(m: &mut Model, command: Command, fx: &mut Effects<Email>) -> Result<(), Refused> {
    // Every command says what it drew. The composer's text and the setup fields are the window's
    // while the person types, so nothing redraws them unless a command put something there.
    fx.changed(part::MODEL);
    match command {
        Command::Look(why) => mailbox::look(m, why, fx),
        Command::Looked { why, folder, loaded } => mailbox::looked(m, why, &folder, loaded, fx),
        Command::FolderClicked(idx) => mailbox::folder_clicked(m, idx, fx),
        Command::SelectFolder(want) => return mailbox::select_folder(m, &want, fx),
        Command::Listed { why, folder, outcome } => mailbox::listed(m, why, folder, outcome, fx),
        Command::Search(query) => mailbox::search(m, &query, fx),
        Command::SearchFor(query) => mailbox::search_for(m, query, fx),
        Command::Searched { query, outcome } => mailbox::searched(m, &query, outcome, fx),
        Command::Triage(idx) => mailbox::triage(m, idx, fx),
        Command::Open(pick) => return message::open(m, &pick, fx),
        Command::Fetched { target, outcome } => message::fetched(m, target, outcome, fx),
        Command::MarkedOnOpen { id, folder, outcome } => message::marked_on_open(m, &id, &folder, outcome, fx),
        Command::Mark { pick, flag, toggle } => return message::mark(m, &pick, flag, toggle, fx),
        Command::Marked { target, flag, wanted, call, read, listed } => {
            message::marked(m, target, flag, wanted, call, &read, listed, fx)
        }
        Command::Delete(key) => message::remove(m, key, false, fx),
        Command::Archive(key) => message::remove(m, key, true, fx),
        Command::Removed { target, how, call, read, listed } => message::removed(m, target, how, call, &read, listed, fx),
        Command::ToggleThread(idx) => message::toggle_thread(m, idx, fx),
        Command::OpenOriginal => message::open_original(m, fx),
        Command::OriginalOpened(sentence) => say(m, fx, sentence),
        Command::ComposeNew => compose::compose_new(m, fx),
        Command::Compose { to, subject, body } => compose::compose(m, to, subject, body, fx),
        Command::Reply { all } => compose::reply(m, all, fx),
        Command::Forward => compose::forward(m, fx),
        Command::Edited { text, query } => {
            m.composer.text = text;
            m.search.query = query;
        }
        Command::Send(mail) => compose::send(m, mail, fx),
        Command::Sent(outcome) => compose::sent(m, outcome, fx),
        Command::CancelCompose(draft) => compose::cancel(m, draft, fx),
        Command::SaveDraft(draft) => compose::save_draft(m, draft, fx),
        Command::OfferUnsent => drafts::write_unsent(m, fx),
        Command::Ending(how) => drafts::ending(m, how, fx),
        Command::DraftDone { op, outcome } => drafts::done(m, op, outcome, fx),
        Command::Ask(ask) => ai::ask(m, ask, fx),
        Command::Answered { ask, about, outcome } => ai::answered(m, ask, about, outcome, fx),
        Command::AddAccount => setup::add_account(m, fx),
        Command::CancelSetup => setup::cancel_setup(m, fx),
        Command::TestConnection(form) => setup::test_connection(m, &form, fx),
        Command::SaveAccount(form) => setup::save_account(m, &form, fx),
        Command::Tested(outcome) => setup::tested(m, outcome, fx),
        Command::Saved(outcome) => setup::saved(m, outcome, fx),
        Command::SignInWithGoogle => return google::sign_in(m, fx),
        Command::CancelGoogle => google::cancel(m, fx),
        Command::GoogleBegun { begun, opened } => google::begun(m, begun, opened, fx),
        Command::GooglePoll => google::poll(m, fx),
        Command::GoogleStatus { flow, outcome } => google::status(m, &flow, outcome, fx),
        Command::GoogleCancelled(outcome) => google::cancelled(outcome),
        Command::Unfinished(work) => unfinished(m, work, fx),
    }
    Ok(())
}

/// Put a failure where the person can read it. `describe` publishes the same string.
pub(crate) fn say(m: &mut Model, fx: &mut Effects<Email>, text: impl Into<String>) {
    let text = text.into();
    tracing::warn!(notice = %text, "email");
    m.notice = text;
    fx.changed(part::STATUS);
}

pub(crate) fn clear_notice(m: &mut Model, fx: &mut Effects<Email>) {
    m.notice.clear();
    fx.changed(part::STATUS);
}

/// A job ended without its result: its busy flag comes down, and the notice says so. A send is
/// said with care, because the mail may or may not have gone.
fn unfinished(m: &mut Model, work: Work, fx: &mut Effects<Email>) {
    let sentence = match work {
        Work::Sync => {
            m.syncing = false;
            m.loading = false;
            m.looked = true;
            m.sync_status = "Not synced".into();
            "The mailbox could not be read; try Sync again."
        }
        Work::Listing => {
            m.loading = false;
            "The folder could not be read; try again."
        }
        Work::Open => "The message could not be opened; try again.",
        Work::Change => "The change to that message did not finish; Sync shows the folder as it is now.",
        Work::Send => {
            // The job panicked or never started; the framework does not say which, so the
            // sentence is the one that is true of both. The text stays in the composer and the
            // draft.
            let went = m.composer.sending.clone();
            compose::keep_after_send(m, true, went, fx);
            m.sync_status = "Not sent".into();
            "The send did not finish, so this window cannot say whether the message went. It is kept as a draft; look in Sent before sending it again."
        }
        Work::Draft => {
            drafts::next_file_op(m, fx);
            "The draft could not be kept; try again."
        }
        Work::Ai => {
            m.ai_working = false;
            "The companion did not answer; try again."
        }
        Work::Setup => {
            m.setup.busy = false;
            m.setup.testing = false;
            m.setup.ok = false;
            m.setup.status = "The mail service could not try it: the check did not finish.".into();
            fx.changed(part::SETUP);
            "The account check did not finish; try again."
        }
        Work::Google => {
            m.setup.busy = false;
            m.google.waiting = false;
            fx.changed(part::SETUP);
            "The Google sign-in could not be started; try again."
        }
        Work::Original => "The original could not be opened; try again.",
    };
    fx.changed(part::LIST);
    say(m, fx, sentence);
    // The first look did not land: a message left unsent at the last close is offered anyway.
    if work == Work::Sync {
        drafts::offer_unsent(m, fx);
    }
}
