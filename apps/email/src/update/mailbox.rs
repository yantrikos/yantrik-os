//! The mailbox: the look at the service, the folders, the list, search and the triage tabs.

use serde_json::json;
use yantrik_app::effects::Effects;
use yantrik_app::jobs::{Concurrency, JobKey, Scope};
use yantrik_app::Refused;
use yantrik_ipc_contracts::email::EmailSummary;

use super::{clear_notice, say};
use crate::backend::{calls, look::{self, Loaded}};
use crate::command::{Command, Listing, Look, Work};
use crate::describe::counts_json;
use crate::model::{part, Model};
use crate::state::{FolderCounts, MailState, Triage};
use crate::{rows, Email};

/// Listing a folder and searching one replace each other: the last one asked for is the one
/// shown, as it was when each held the window until it answered.
fn listing() -> JobKey {
    JobKey { feature: "listing", scope: Scope::App }
}

/// What a job replaced before it started carries. The framework drops a replaced job's
/// result, so this is never shown.
pub const REPLACED: &str = "a newer request replaced this one";

/// One look at the mail service, which starts it if it is not running. On a job: a mailbox sync
/// crosses the internet. "Synced" is said only by what the look found.
pub fn look(m: &mut Model, why: Look, fx: &mut Effects<Email>) {
    if m.syncing {
        return;
    }
    m.syncing = true;
    m.sync_status = "Syncing\u{2026}".into();
    fx.changed(part::STATUS);
    let folder = m.folder.clone();
    let mail = m.mail.clone();
    fx.spawn_keyed_or(
        JobKey { feature: "sync", scope: Scope::App },
        Concurrency::RejectIfRunning,
        Command::Unfinished(Work::Sync),
        move |_| {
            let loaded = look::look_at_mail(mail.as_ref(), &folder);
            Command::Looked { why, folder, loaded }
        },
    );
}

/// Put a look on screen, whichever of the three states it turned out to be.
pub fn looked(m: &mut Model, why: Look, folder: &str, loaded: Loaded, fx: &mut Effects<Email>) {
    m.syncing = false;
    m.looked = true;
    m.loading = false;
    let count = loaded.messages.len();
    let ok = matches!(loaded.state, MailState::Ready { .. }) && loaded.mailbox_error.is_none();
    m.state = loaded.state.clone();
    m.account_name = loaded.state.account_name();
    fx.changed(part::STATE);
    fx.changed(part::SETUP);

    // The folder that is open, highlighted by name. `is_selected: idx == 0` was hardcoded, so
    // after switching mailboxes the sidebar still pointed at whichever folder came back first.
    let selected = loaded.folders.iter().position(|f| f.name.eq_ignore_ascii_case(&m.folder)).unwrap_or(0);
    m.folders = loaded.folders.iter().enumerate().map(|(i, f)| rows::folder(f, i == selected)).collect();
    m.selected_folder = selected;
    fx.changed(part::FOLDERS);
    set_rows(m, &loaded.messages, fx);

    match (&loaded.state, &loaded.mailbox_error) {
        (MailState::Ready { .. }, Some(e)) => {
            m.sync_status = "Mailbox did not open".into();
            say(m, fx, format!("The account is configured and the mailbox did not open: {e}"));
        }
        (MailState::Ready { .. }, None) => {
            m.sync_status = "Synced".into();
            clear_notice(m, fx);
        }
        (MailState::NoAccount { .. }, _) => {
            m.sync_status = "No account".into();
            clear_notice(m, fx);
        }
        (MailState::Unreachable { .. }, _) => {
            m.sync_status = "Service unreachable".into();
            say(m, fx, loaded.state.notice());
        }
    }
    if ok && why == Look::Sync {
        // The count it returned, because that is what was observed.
        m.sync_status = format!("Synced \u{00b7} {count} in {folder}");
    }
    if why == Look::Start {
        super::drafts::offer_unsent(m, fx);
    }
}

/// Put a folder's messages into the model, through the triage filter.
pub fn set_rows(m: &mut Model, messages: &[EmailSummary], fx: &mut Effects<Email>) {
    m.rows = rows::rows(messages);
    fx.changed(part::LIST);
}

/// Change one folder's entry in the list, for the cases where the mail server has already agreed
/// — a message read, deleted or moved. The next sync replaces the whole list with the server's
/// count; until then this is what is known. A folder that is not listed is left alone, and the
/// header counts what is in hand. A folder that is listed without counts stays without them:
/// there is no number to take one from.
pub fn change_counts_of(m: &mut Model, folder: &str, change: impl FnOnce(FolderCounts) -> FolderCounts, fx: &mut Effects<Email>) {
    if let Some(row) = m.folders.iter_mut().find(|f| f.name.eq_ignore_ascii_case(folder)) {
        if let Some(counts) = row.counts {
            row.counts = Some(change(counts));
        }
    }
    fx.changed(part::FOLDERS);
    fx.changed(part::LIST);
}

/// Why a folder cannot be listed before anything is asked: there is no account to list it in.
fn no_account(m: &Model) -> String {
    match m.unreachable_reason() {
        Some(reason) => format!("the mail service could not be reached: {reason}"),
        None => "no email account is configured".to_string(),
    }
}

/// Open a folder: its messages are asked for in a job, and [`listed`] says what came.
pub fn load_folder(m: &mut Model, folder: &str, why: Listing, fx: &mut Effects<Email>) -> Result<(), String> {
    let account = m.account();
    if account.is_empty() {
        return Err(no_account(m));
    }
    m.loading = true;
    fx.changed(part::STATUS);
    let mail = m.mail.clone();
    let folder = folder.to_string();
    fx.spawn_keyed_or(listing(), Concurrency::ReplacePrevious, Command::Unfinished(Work::Listing), move |cancel| {
        // Replaced before it started: the newer listing is the one asked for, and this one's
        // result would be dropped anyway.
        let outcome = if cancel.is_set() {
            Err(REPLACED.to_string())
        } else {
            calls::list_messages(mail.as_ref(), &account, &folder, 1)
        };
        Command::Listed { why, folder, outcome }
    });
    Ok(())
}

/// A folder of the sidebar, pressed.
pub fn folder_clicked(m: &mut Model, idx: i32, fx: &mut Effects<Email>) {
    let Some(name) = usize::try_from(idx).ok().and_then(|i| m.folders.get(i)).map(|f| f.name.clone()) else {
        return;
    };
    m.selected_folder = idx as usize;
    fx.changed(part::FOLDERS);
    // Nothing to say when there is no account: the screen already says why.
    let _ = load_folder(m, &name, Listing::Folder, fx);
}

/// `select_folder`: a folder by its name or its kind. It looks local and is not: switching
/// mailbox asks the mail server for that folder's messages.
pub fn select_folder(m: &mut Model, want: &str, fx: &mut Effects<Email>) -> Result<(), Refused> {
    let want = want.trim().to_lowercase();
    if want.is_empty() {
        return Err(Refused::new("no_folder", "no folder was named"));
    }
    let row = m
        .folders
        .iter()
        .position(|f| f.name.to_lowercase() == want || f.kind.to_lowercase() == want)
        .ok_or_else(|| {
            let names: Vec<&str> = m.folders.iter().map(|f| f.name.as_str()).collect();
            let sentence = if names.is_empty() {
                format!("no folder called \u{201c}{want}\u{201d}; no folders are listed")
            } else {
                format!("no folder called \u{201c}{want}\u{201d}; there is: {}", names.join(", "))
            };
            Refused::new("no_such_folder", sentence)
        })?;
    // Checked before anything changes, so a refusal leaves the sidebar as it was.
    if m.account().is_empty() {
        return Err(Refused::new("no_account", no_account(m)));
    }
    let name = m.folders[row].name.clone();
    m.selected_folder = row;
    fx.changed(part::FOLDERS);
    load_folder(m, &name, Listing::Folder, fx).map_err(|e| Refused::new("no_account", e))
}

/// A folder's messages came back, or did not.
pub fn listed(m: &mut Model, why: Listing, folder: String, outcome: Result<Vec<EmailSummary>, String>, fx: &mut Effects<Email>) {
    m.loading = false;
    fx.changed(part::STATUS);
    match outcome {
        Ok(messages) => {
            let count = messages.len();
            m.folder = folder.clone();
            set_rows(m, &messages, fx);
            m.sync_status = "Synced".into();
            // The listing after a send is the app's own refresh: what the send said stays said.
            if why != Listing::AfterSend {
                clear_notice(m, fx);
            }
            m.answer = match why {
                Listing::SearchCleared => search_answer(m, count),
                Listing::Folder | Listing::AfterSend => {
                    json!({ "folder": folder, "messages": count, "counts": counts_json(&m.header()) })
                }
            };
            fx.settle(Ok(()));
        }
        Err(e) => {
            m.sync_status = "Mailbox did not open".into();
            say(m, fx, format!("Could not open {folder}: {e}"));
            fx.settle(Err(Refused::new("not_listed", e)));
        }
    }
}

/// What `search` answers: how many matched, and the subjects of the rows on screen.
fn search_answer(m: &Model, matched: usize) -> serde_json::Value {
    let subjects: Vec<String> = m.shown().iter().take(25).map(|(_, r)| r.item.subject.clone()).collect();
    json!({ "matched": matched, "subjects": subjects })
}

/// The search box submitted: search the folder on screen, or with an empty box, show it again.
///
/// A search that could not be run is said, never left looking like a mailbox with no matches.
pub fn search(m: &mut Model, query: &str, fx: &mut Effects<Email>) {
    let query = query.trim();
    if query.is_empty() {
        m.search.active = false;
        fx.changed(part::STATUS);
        let folder = m.folder.clone();
        if let Err(e) = load_folder(m, &folder, Listing::SearchCleared, fx) {
            fx.settle(Err(Refused::new("not_listed", e)));
        }
        return;
    }
    let account = m.account();
    if account.is_empty() {
        let text = match m.unreachable_reason() {
            Some(reason) => format!("Cannot search: the mail service could not be reached: {reason}"),
            None => "Cannot search: no email account is configured.".to_string(),
        };
        say(m, fx, text.clone());
        fx.settle(Err(Refused::new("no_account", text)));
        return;
    }
    m.search.active = true;
    fx.changed(part::STATUS);
    // The folder on screen is the folder searched: the results are shown beside it, and each
    // row carries the folder it was found in for whatever is done to it next (#288).
    let folder = m.folder.clone();
    let mail = m.mail.clone();
    let query = query.to_string();
    fx.spawn_keyed_or(listing(), Concurrency::ReplacePrevious, Command::Unfinished(Work::Listing), move |cancel| {
        let outcome = if cancel.is_set() {
            Err(REPLACED.to_string())
        } else {
            calls::search(mail.as_ref(), &account, &folder, &query)
        };
        Command::Searched { query, outcome }
    });
}

/// `search`: the box shows the query, and the folder on screen is searched for it.
pub fn search_for(m: &mut Model, query: String, fx: &mut Effects<Email>) {
    m.search.query = query.clone();
    fx.changed(part::SEARCH_TEXT);
    search(m, &query, fx);
}

pub fn searched(m: &mut Model, query: &str, outcome: Result<Vec<EmailSummary>, String>, fx: &mut Effects<Email>) {
    match outcome {
        Ok(results) => {
            let count = results.len();
            set_rows(m, &results, fx);
            m.search.count = count as i32;
            clear_notice(m, fx);
            m.answer = search_answer(m, count);
            fx.settle(Ok(()));
        }
        Err(e) => {
            let text = format!("Could not search for \u{201c}{query}\u{201d}: {e}");
            say(m, fx, text.clone());
            fx.settle(Err(Refused::new("not_searched", text)));
        }
    }
}

/// A triage tab: a filter over the rows already held, with no question to the mail server, and
/// no tab that filters on something no message carries.
pub fn triage(m: &mut Model, idx: i32, fx: &mut Effects<Email>) {
    match Triage::from_index(idx) {
        Some(t) => {
            m.triage = t;
            fx.changed(part::LIST);
        }
        None => say(m, fx, format!("There is no triage tab {idx}.")),
    }
}

