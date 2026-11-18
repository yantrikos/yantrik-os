//! Files outside the library: a note read in from one, or written out to one.

use yantrik_app::intents::Intents;

use super::{act, add, arg, needed};
use crate::command::Act;
use crate::Notes;

pub(super) fn table(table: &mut Intents<Notes>) {
    add(
        table,
        // Sensitive: it reads a file from anywhere this user can read, outside the notes
        // directory, and copies its contents into the library.
        act(
            "import",
            "Read a UTF-8 text or Markdown file from anywhere on disk and add its contents to \
             the library as a new note, then open it. The file itself is left where it is.",
        )
        .risk("sensitive")
        .arg(arg(
            "path",
            "Absolute path to the file to read, or one starting `~/`. Up to 256 KiB of text with \
             no control characters.",
        )),
        |a| Ok(Act::Import { path: needed(a, "import", "path", "An absolute path to a text file.")? }),
    );

    add(
        table,
        // Sensitive: it writes a file outside the notes directory, at a path the caller chooses.
        // It cannot overwrite — the store publishes the new file with a link, which fails if
        // something is already there — so it is not `dangerous`.
        act(
            "export",
            "Write the open note's text to a file outside the library, at this path. It refuses \
             rather than overwrite anything already there, and the note stays open.",
        )
        .risk("sensitive")
        .arg(arg(
            "path",
            "Absolute path of the file to create, or one starting `~/`. Its folder must exist \
             and nothing may already be at that path.",
        )),
        |a| Ok(Act::Export { path: needed(a, "export", "path", "An absolute path to write to.")? }),
    );
}
