//! Windows (step 8d-3b): a note in a window of its own, the same command the note header's
//! *Open in new window* button and Ctrl+Shift+N run. Every other action acts on the focused
//! window's note, which `describe.focused_window` names.

use yantrik_app::intents::Intents;

use super::{act, add, arg, given};
use crate::command::Act;
use crate::Notes;

pub(super) fn table(table: &mut Intents<Notes>) {
    add(
        table,
        // Standard: it opens a window and changes nothing in the library. The note shown in both
        // windows is one note: nothing is copied, saved or replaced.
        act(
            "open_in_new_window",
            "Show a note in a new window of its own, beside the one it is open in: the note that is \
             open, or the one `title` names. Both windows show the same note, so an edit in either \
             is in both; the new window is the focused one afterwards.",
        )
        .arg(
            arg(
                "title",
                "The note's exact title, or the filename `describe` lists it under. Leave it out for \
                 the note open in the focused window.",
            )
            .optional(),
        ),
        |a| Ok(Act::OpenInNewWindow { wanted: given(a, "title") }),
    );
}
