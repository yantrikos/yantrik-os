//! `new`, `open`, `read`, `save`, `save_as` and `show`: tabs and the files they hold.

use yantrik_app::control::Param;
use yantrik_app::intents::Intents;

use super::{act, add, arg, needed, number};
use crate::command::Act;
use crate::TextEditor;

pub(super) fn table(table: &mut Intents<TextEditor>) {
    add(
        table,
        // Standard, with or without `text`: a new tab replaces nothing. With `save_as` it is the
        // way to write a file's text without asking anyone, the route a mind running unattended
        // needs, since `set_content` is sensitive and waits for a person (#253).
        act(
            "new",
            "Open a new tab and make it the active one, empty or holding the text given. Nothing \
             is written to disk until `save_as` gives it a path. Tabs open until they would hold \
             64 MiB between them; the refusal then names a tab of yours to close.",
        )
        .arg(
            arg(
                "text",
                "What the new tab holds, up to 1 MiB and 20,000 lines of UTF-8 with no control \
                 characters. Leave it out for an empty tab.",
            )
            .optional(),
        ),
        |a| Ok(Act::New { text: a.raw().get("text").and_then(|v| v.as_str()).unwrap_or_default().to_string() }),
    );

    add(
        table,
        act(
            "open",
            "Open the UTF-8 text file at this path in a tab and make it active; a file already \
             open is brought forward instead of opened twice. The file is not changed.",
        )
        .arg(arg(
            "path",
            "Absolute path to the file, or one starting `~/`. Up to 1 MiB of UTF-8 text; a \
             binary or larger file is refused and nothing is opened.",
        )),
        |a| Ok(Act::Open { path: needed(a, "open", "path", "An absolute path to a text file.")? }),
    );

    add(
        table,
        // Safe: it reads and changes nothing, not even which tab is in front. A mind keeps about
        // 4,000 characters of a safe read and 900 of anything graded higher, so the grade is what
        // lets a page reach it whole. Open tabs only, never a path: a safe read is run unasked by
        // plan mode, phone turns and read-only roles, and a path would reach every text file in
        // the home. A file not yet open goes through `open`, which is `standard`.
        act(
            "read",
            "Read an open tab's text one page at a time, changing nothing: the tab in front or \
             another by number. A file not open yet is `open`ed first. The answer names the \
             `from_line` of the next page.",
        )
        .arg(Param::number("from_line").optional().describe(
            "The first line to return, counting from 1, as the last answer's \
             `how_to_see_the_rest` names it. Leave it out to start at line 1.",
        ))
        .arg(Param::number("tab").optional().describe(
            "Which open tab to read, counting from 0 as `describe.tabs` lists them; it is not \
             brought forward. Leave it out to read the tab in front.",
        ))
        .risk("safe"),
        |a| {
            Ok(Act::Read { from_line: number(a, "from_line").unwrap_or(1).max(1) as usize, tab: number(a, "tab") })
        },
    );

    add(
        table,
        act(
            "save",
            "Write the active tab back to the file it was opened from and check the bytes on \
             disk. A tab with no file is refused — give `save_as` a path. When the editor is \
             asking about unsaved changes, this is the answer that writes the tab and closes it.",
        ),
        |_| Ok(Act::Save),
    );

    add(
        table,
        // Standard, like `save`: writing a file the caller named is this app's one job, and an
        // existing path is refused rather than silently replaced, unless the caller says
        // `overwrite=true`, having read the refusal that names it (#86).
        act(
            "save_as",
            "Write the active tab to this path and keep the tab on it from now on. A file \
             already at that path is refused unless `overwrite` says to replace it.",
        )
        .arg(arg(
            "path",
            "Absolute path of the file to write, or one starting `~/`. Its folder must exist; a \
             file already at the path needs `overwrite=true`.",
        ))
        .arg(Param::flag("overwrite").optional().describe(
            "Replace a file that is already at that path. Leave it out and an existing \
             file is refused rather than overwritten.",
        )),
        |a| {
            Ok(Act::SaveAs {
                path: needed(a, "save_as", "path", "An absolute path to write to.")?,
                overwrite: a.raw().get("overwrite").and_then(|v| v.as_bool()).unwrap_or(false),
            })
        },
    );

    add(
        table,
        act("show", "Bring the Editor window to the front. Nothing in any tab is read or changed."),
        |_| Ok(Act::Show),
    );
}
