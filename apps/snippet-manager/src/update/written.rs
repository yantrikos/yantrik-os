//! Writing the library, and what is done when a write lands: the change's own answer, or the
//! file's library put back on screen when the write did not land.

use super::{fail, say, worked, Fx};
use crate::answers;
use crate::command::{After, Command, Work, Write, Written};
use crate::lane::Landed;
use crate::model::{Model, Offer, Restore, Why, PART_DETAIL, PART_LIST, PART_RECOVERY};
use crate::store::{file, Library, ALL, FAVORITES};

/// The library as it now is in memory, written in a job. `after` is what the change does once
/// its write lands; `restore` is where the window was before it, put back if the write fails and
/// the window would otherwise show something the file does not hold.
pub(super) fn write(m: &mut Model, fx: &mut Fx, after: After, restore: Restore) {
    let bytes = match file::encode(&m.library) {
        Ok(bytes) => bytes,
        Err(why) => {
            // Nothing can be written that cannot be encoded; the change is taken back as a write
            // that did not finish would be.
            tracing::error!(error = %why, "the library could not be encoded");
            did_not_finish(m, after, &restore, fx);
            return;
        }
    };
    let ticket = m.lane.ticket();
    m.work += 1;
    // Every draft whose text these bytes hold waits on this write to go (`drafts::release`).
    for draft in m.drafts.values_mut() {
        if m.library.get(draft.kept_as).is_some() && !m.library.differs(draft.kept_as, &draft.text.patch()) {
            draft.pending = Some(ticket.revision);
        }
    }
    let write = Write { ticket, after, restore };
    let job = m.lane.job(ticket, bytes);
    let failed = Command::WorkFailed(Box::new(Work::Write(Box::new(write.clone()))));
    fx.spawn_or(failed, move |_| {
        Command::Written(Box::new(Written { write, landed: job() }))
    });
}

pub(super) fn update(m: &mut Model, command: Command, fx: &mut Fx) {
    match command {
        Command::Written(written) => {
            let Written { write, landed } = *written;
            m.lane.landed();
            match landed {
                Landed::Saved | Landed::Covered => {
                    m.lane.confirm(write.ticket.revision);
                    kept(m, write.after, fx);
                }
                Landed::Failed { why, on_disk } => {
                    super::drafts::not_landed(m, write.ticket.revision);
                    if m.lane.spend(write.ticket) {
                        match on_disk {
                            Ok(library) => take_back(m, library, &write.restore, fx),
                            Err(e) => {
                                tracing::warn!(error = %e, "the library could not be read back");
                            }
                        }
                    }
                    not_kept(m, write.after, &why, fx);
                    keep_typing(m, &write.restore, &why, fx);
                }
                Landed::Undone(why) => {
                    super::drafts::not_landed(m, write.ticket.revision);
                    not_kept(m, write.after, &why, fx);
                    keep_typing(m, &write.restore, &why, fx);
                }
            }
        }
        Command::Reloaded(result) => {
            // Taken only while nothing else is being written: a write in flight says for itself
            // what the file holds once it lands.
            if let (Ok(library), true) = (result, m.lane.idle()) {
                let restore = m.restore_point();
                take_back(m, library, &restore, fx);
            }
        }
        _ => {}
    }
}

/// A write's job ended without saying whether the file took it. The window cannot know what is
/// kept, so the file is read again, and the change is not claimed. The typing it was keeping goes
/// to its draft now, whatever memory says, and no draft is let go until the file has been read
/// (#938 review, M2).
pub(super) fn did_not_finish(m: &mut Model, after: After, restore: &Restore, fx: &mut Fx) {
    let why = "Saving did not finish, so the change may not be kept; the saved snippets are being read again.";
    not_kept(m, after, why, fx);
    // Typing the change carried has a draft (`drafts::keep` hands it over as it is typed); a change
    // that carried none, a star say, has nothing to keep.
    let typing = &restore.typing;
    if m.drafts.contains_key(&typing.id) && super::drafts::hold(m, fx, typing, restore.typed) {
        say(m, fx, &format!("{why} Your text is kept as a draft."));
    }
    let disk = m.lane.disk();
    let path = m.lane.path().to_path_buf();
    fx.spawn_or(Command::WorkFailed(Box::new(Work::Reload)), move |_| {
        Command::Reloaded(disk.read_back(&path))
    });
}

/// The write landed: the change's own answer, and the notice cleared, or set.
fn kept(m: &mut Model, after: After, fx: &mut Fx) {
    match after {
        After::Answer(answer) => {
            worked(m, fx);
            answers::settle(m, fx, answer);
        }
        After::Say(sentence) => say(m, fx, &sentence),
        After::Counted { saved, .. } => {
            worked(m, fx);
            answers::settle(m, fx, saved);
        }
    }
}

/// The write did not land. The change is not kept, and says so; a copy's use count is the one
/// change whose call is still answered, because the code is on the clipboard either way.
fn not_kept(m: &mut Model, after: After, why: &str, fx: &mut Fx) {
    match after {
        After::Counted { unsaved, from_window, .. } => {
            if from_window {
                say(m, fx, "The code is on the clipboard, but the use count could not be saved.");
            } else {
                worked(m, fx);
            }
            answers::settle(m, fx, unsaved);
        }
        After::Answer(_) | After::Say(_) => fail(m, fx, why),
    }
}

/// The typing a refused change was keeping, if the store does not hold it (or no longer holds its
/// snippet): in its draft at once, unless newer typing for it is there already, and said plainly,
/// so the person knows the save failed and where the text is. Text the editor no longer shows is
/// offered on the bar at once, so the sentence that says so is true.
fn keep_typing(m: &mut Model, restore: &Restore, why: &str, fx: &mut Fx) {
    let typing = &restore.typing;
    let kept = m.library.get(typing.id).is_some() && !m.library.differs(typing.id, &typing.patch());
    if typing.id < 0 || kept {
        return;
    }
    super::drafts::hold(m, fx, typing, restore.typed);
    let kept_where = if m.editor.id == typing.id {
        "Your text is still in the editor, and kept as a draft until it is saved."
    } else {
        if !m.offers.iter().any(|o| o.id == typing.id) {
            m.offers.push(Offer { id: typing.id, why: Why::NotSaved });
            fx.changed(PART_RECOVERY);
        }
        "Your text is kept as a draft; the bar offers it back."
    };
    say(m, fx, &format!("Not saved: {why}. {kept_where}"));
}

/// The file's library on screen again: memory is the disk.
///
/// The window is left where it is when what it shows is still kept. When the change that failed
/// had moved it onto something that is now gone (a new snippet, a new collection), it goes back
/// to where it was, with the typing it had, unless the person has typed something new since.
fn take_back(m: &mut Model, mut library: Library, restore: &Restore, fx: &mut Fx) {
    // No id a refused create used is given again (#938 review, M3).
    library.keep_counters_from(&m.library);
    m.library = library;
    let custom = m.collection != ALL && m.collection != FAVORITES;
    if custom && !m.library.collections().iter().any(|c| c.id == m.collection) {
        let back = m.library.collections().iter().any(|c| c.id == restore.collection);
        m.collection = if back { restore.collection } else { ALL };
    }
    // The typing the failed change was keeping, or newer typing for the same snippet the draft
    // lane holds since: the newest wins (#938 review, M1).
    let typing = match m.drafts.get(&restore.editor.id) {
        Some(d) if d.typed > restore.typed => d.text.clone(),
        _ => restore.editor.clone(),
    };
    let typing_lost = m.library.get(restore.selected).is_some() && m.library.differs(typing.id, &typing.patch());
    if m.editor_dirty() {
        // New typing since: it stays where it is.
    } else if typing_lost {
        // The typing the failed change was keeping is not kept: it goes back in the editor, on
        // its own snippet, where it can be saved again or copied out.
        m.selected = restore.selected;
        m.editor = typing;
    } else if m.library.get(m.selected).is_some() {
        m.select(m.selected);
    } else {
        m.select(restore.selected);
    }
    fx.changed(PART_LIST);
    fx.changed(PART_DETAIL);
}
