//! The outside bar's choices, for a mind as for the person: the same commands its buttons run.

use yantrik_app::intents::Intents;

use super::{act, add};
use crate::command::Act;
use crate::Notes;

pub(super) fn table(table: &mut Intents<Notes>) {
    // The outside bar's choices, for a mind as for the person: the same commands its buttons run.
    // *Not now* is the bar's alone: it only puts the bar away, and a mind's saves are refused
    // until a choice is made either way.
    add(
        table,
        // Sensitive: it writes a new note into the library and moves the window to it. Nothing is
        // lost or overwritten: the file the other program wrote is left as it is.
        act(
            "keep_mine_as_copy",
            "When the open note's file was changed outside Notes while it had unsaved edits, or was \
             moved or deleted (`describe.outside` is `conflict` or `deleted`), save the text in the \
             window as a new note and open it. The file on disk is left exactly as it is.",
        )
        .risk("sensitive"),
        |_| Ok(Act::KeepMineAsCopy),
    );

    add(
        table,
        // Sensitive: the text in the window is replaced. It is not `dangerous` because that text is
        // saved as a note of its own first, which `open_note` reaches.
        act(
            "take_theirs",
            "When the open note's file was changed outside Notes while it had unsaved edits \
             (`describe.outside` is `conflict`), replace the text in the window with the file's. The \
             window's text is first saved as a separate note, and the undo history is cleared.",
        )
        .risk("sensitive"),
        |_| Ok(Act::TakeTheirs),
    );

    add(
        table,
        // Safe: it reads the file and shows the two versions side by side, read-only. Nothing is
        // written, saved or replaced.
        act(
            "compare",
            "When the open note's file was changed outside Notes while it had unsaved edits, show the \
             window's text beside the file's, read-only, and answer with both and the first line \
             where they differ. Nothing is changed or saved.",
        )
        .risk("safe"),
        |_| Ok(Act::Compare),
    );
}
