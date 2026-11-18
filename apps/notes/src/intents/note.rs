//! The window's own actions on the note that is open: its notebook and tags, saving it, the
//! Trash, a copy, a refresh, the preview and focus toggles, and its history.

use yantrik_app::intents::Intents;

use super::{act, add, arg, given};
use crate::command::Act;
use crate::Notes;

pub(super) fn table(table: &mut Intents<Notes>) {
    add(
        table,
        act(
            "notebook",
            "File the note that is open under a notebook, which is the section `set_folder \
             book:<name>` lists, and save it. It replaces whatever notebook it was in.",
        )
        .arg(arg(
            "text",
            "The notebook's name, up to 80 characters; an empty string files the note under none.",
        )),
        |a| Ok(Act::Notebook(given(a, "text"))),
    );

    add(
        table,
        act(
            "tags",
            "Replace the tags on the note that is open and save it. Tags are searched by \
             `search` along with the note's text.",
        )
        .arg(arg(
            "text",
            "Comma-separated tags, e.g. `work, ideas`, up to 512 characters; an empty string \
             clears them.",
        )),
        |a| Ok(Act::Tags(given(a, "text"))),
    );

    add(
        table,
        act(
            "save",
            "Write the note that is open to its file now instead of waiting for the 750 ms \
             autosave, and check that the bytes on disk are the text in the window.",
        ),
        |_| Ok(Act::Save),
    );

    add(
        table,
        // Sensitive: the note leaves the library and its file is removed from the notes
        // directory. It is not `dangerous` because `restore` puts back exactly this note from
        // the Trash record written before the file is unlinked.
        act(
            "trash",
            "Move the note that is open to the Trash: it leaves the library, its file is removed \
             from the notes directory, and `restore` puts it back.",
        )
        .risk("sensitive"),
        |_| Ok(Act::Trash),
    );

    add(
        table,
        act(
            "restore",
            "Put the trashed note that is open back into the library, writing its file into the \
             notes directory again. It refuses rather than overwrite a note of the same filename.",
        ),
        |_| Ok(Act::Restore),
    );

    add(
        table,
        act(
            "copy",
            "Duplicate the note that is open as a new note with its own filename, save that, and \
             open it. Unsaved changes go with the copy — it is the way out of \"this note changed \
             on disk\" — and the original file is left as it is.",
        ),
        |_| Ok(Act::Copy),
    );

    add(
        table,
        act(
            "reload",
            "Re-read every note from the notes directory, picking up files changed outside this \
             window. The open note is saved first and stays open if its file is still there.",
        ),
        |_| Ok(Act::Reload),
    );

    add(
        table,
        act(
            "preview",
            "Turn the rendered Markdown preview of the open note on or off — it is a toggle, and \
             the answer says which it now is. The note itself is not changed.",
        ),
        |_| Ok(Act::Preview),
    );

    add(
        table,
        act(
            "focus",
            "Turn focus mode, which hides the library list and the side panels, on or off — it \
             is a toggle, and the answer says which it now is.",
        ),
        |_| Ok(Act::Focus),
    );

    add(
        table,
        act(
            "undo",
            "Take back the last change made to the note that is open in this window, and save \
             the result. The history is cleared whenever another note is opened.",
        ),
        |_| Ok(Act::Undo),
    );

    add(
        table,
        act(
            "redo",
            "Put back the change that `undo` took off the note that is open, and save the \
             result. Editing the note discards what redo was holding.",
        ),
        |_| Ok(Act::Redo),
    );
}
