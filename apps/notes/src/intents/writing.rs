//! Making a note, opening one, and changing the open note's text: the actions that write what
//! the person reads.

use serde_json::Value;
use yantrik_app::intents::Intents;

use super::{act, add, arg, given, needed, refuse};
use crate::command::Act;
use crate::Notes;

pub(super) fn table(table: &mut Intents<Notes>) {
    add(
        table,
        act(
            "new_note",
            "Create a note with this title, put `text` in it if given, save it, and open it. The \
             title is written as the note's first line, `# <title>`, which is what the library \
             lists it by; whatever note was open is saved first.",
        )
        .arg(arg(
            "title",
            "The new note's title. It is written as the first line, `# <title>`, and is what \
             `open_note` and `search` find it by.",
        ))
        .arg(
            arg(
                "text",
                "The body, written under the title on its own lines. Leave it out for a note \
                 that is only a heading; `append` can add to it later.",
            )
            .optional(),
        ),
        |a| {
            let title = needed(
                a,
                "new_note",
                "title",
                "It becomes the note's `# heading` line — for example title=\"Groceries\".",
            )?;
            Ok(Act::NewNote { title, body: given(a, "text") })
        },
    );

    add(
        table,
        act(
            "open_note",
            "Open an existing note by its exact title or its filename and show it in the editor. \
             The note that was open is saved first. Only live notes are matched, never the Trash; \
             with several of one title the first listed wins. A trashed note opens read-only, \
             and only by `trash:<filename>`.",
        )
        .arg(arg(
            "title",
            "The note's exact title, or the filename `describe` lists it under (`note-….md`). \
             For a note in the Trash give `trash:<filename>`; a title never matches the Trash.",
        )),
        |a| {
            let wanted = needed(a, "open_note", "title", "Use a title or filename from `describe.notes`.")?;
            Ok(Act::OpenNote { wanted })
        },
    );

    add(
        table,
        act(
            "set_title",
            "Rename the note that is open by rewriting its first `# heading` line, adding one at \
             the top if it has none, and save it. Nothing else in the note changes.",
        )
        .arg(arg("title", "The new title, on one line; any line breaks in it become spaces.")),
        |a| Ok(Act::SetTitle(needed(a, "set_title", "title", "It replaces the `# heading` line.")?)),
    );

    add(
        table,
        act(
            "append",
            "Add text to the end of the note that is open and save it. Nothing already in the \
             note is removed; use `set_content` to replace it instead.",
        )
        .arg(arg(
            "text",
            "Exactly what to add. It starts on a new line: a newline goes before it unless the \
             note is empty, already ends with one, or the text starts with one.",
        )),
        |a| {
            Ok(Act::Append(needed(
                a,
                "append",
                "text",
                "There is nothing to append without it, and the note was left untouched.",
            )?))
        },
    );

    add(
        table,
        // Sensitive: it overwrites the whole note, unsaved paragraphs included. What it replaces
        // is on disk only if it had been saved, and `undo` only reaches it while this window
        // stays on this note.
        act(
            "set_content",
            "Replace everything in the note that is open with this text and save it. What was \
             there is gone from the window; `undo` takes it back while the note stays open.",
        )
        .risk("sensitive")
        .arg(arg(
            "text",
            "The note's entire new text, Markdown included — keep the `# <title>` first line if \
             the note should keep its title. An empty string empties the note.",
        )),
        |a| match a.raw().get("text").and_then(Value::as_str) {
            Some(text) => Ok(Act::SetContent(text.to_string())),
            None => Err(refuse("`set_content` needs `text`: the note's entire new text.".into())),
        },
    );
}
