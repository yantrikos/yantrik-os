//! Finding in the library: filtering the list, reading many notes at once, and choosing its
//! section. None of them opens a note.

use yantrik_app::intents::Intents;

use super::{act, add, arg, given, needed};
use crate::command::Act;
use crate::Notes;

pub(super) fn table(table: &mut Intents<Notes>) {
    add(
        table,
        act(
            "search",
            "Filter the library list to the notes whose text or metadata contains this, without \
             opening anything; `read_notes` returns their text. `set_folder` picks the section.",
        )
        .arg(arg(
            "query",
            "Matched case-insensitively against each note's text and its notebook and tags; an \
             empty string clears the filter.",
        )),
        |a| Ok(Act::Search(given(a, "query"))),
    );

    add(
        table,
        act(
            "read_notes",
            "Read the text of many notes in one call, newest first: every note whose text, \
             notebook or tags contain `query`, or all of them, without opening any. One page at \
             a time; the answer names the `skip` for the next page.",
        )
        .arg(
            arg(
                "query",
                "Matched case-insensitively like `search`; leave it out to read every note \
                 outside the Trash.",
            )
            .optional(),
        )
        .arg(
            arg(
                "skip",
                "How many matching notes to pass over, as the last answer's `how_to_see_the_rest` \
                 says; leave it out for the first page.",
            )
            .optional(),
        )
        // A read that opens nothing and moves no view. Left at the default grade, a mind's work
        // log cut its answer at 900 characters, so the model saw the envelope and none of the
        // notes, and asked eight more times.
        .risk("safe"),
        |a| {
            let skip = a
                .raw()
                .get("skip")
                .and_then(|v| v.as_u64().or_else(|| v.as_str()?.trim().parse().ok()))
                .unwrap_or(0) as usize;
            Ok(Act::ReadNotes { query: given(a, "query"), skip })
        },
    );

    add(
        table,
        act(
            "set_folder",
            "Switch the library list to one section of the library. It changes what `search` and \
             `describe.matches` list, and opens nothing.",
        )
        .arg(arg(
            "folder",
            "One of `all`, `recent` (touched in the last seven days), `favorites` (pinned), \
             `trash`, or `book:<notebook name>`.",
        )),
        |a| {
            Ok(Act::SetFolder(needed(
                a,
                "set_folder",
                "folder",
                "One of `all`, `recent`, `favorites`, `trash`, `book:<name>`.",
            )?))
        },
    );
}
