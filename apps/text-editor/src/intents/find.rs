//! `find`, `find-next`, `find-prev`, `replace_text`, `replace` and `replace-all`: the find bar.

use yantrik_app::intents::Intents;

use super::{act, add, arg, given, needed};
use crate::command::Act;
use crate::TextEditor;

pub(super) fn table(table: &mut Intents<TextEditor>) {
    add(
        table,
        act(
            "find",
            "Search the active tab for this text, open the find bar, and select the first match. \
             It reads the document and changes nothing in it.",
        )
        .arg(arg(
            "text",
            "The text to look for, matched literally. Case is ignored unless the find bar's \
             match-case box is ticked; an empty query clears the matches.",
        )),
        |a| Ok(Act::Find { text: needed(a, "find", "text", "The text to look for.")? }),
    );

    add(
        table,
        act(
            "find-next",
            "Move the selection to the next match of the current `find` query, wrapping round at \
             the end of the document. Nothing is changed.",
        ),
        |_| Ok(Act::FindNext),
    );

    add(
        table,
        act(
            "find-prev",
            "Move the selection to the previous match of the current `find` query, wrapping \
             round at the start of the document. Nothing is changed.",
        ),
        |_| Ok(Act::FindPrev),
    );

    add(
        table,
        act(
            "replace_text",
            "Set the text that `replace` and `replace-all` will put in place of a match. By \
             itself it changes nothing in the document.",
        )
        .arg(arg("text", "What each match becomes. An empty string is allowed and deletes the match instead.")),
        |a| Ok(Act::ReplaceText { text: given(a, "replace_text", "text", "what each match becomes.")? }),
    );

    add(
        table,
        act(
            "replace",
            "Replace the one match the selection is on with the `replace_text` text. The tab is \
             changed in the window; nothing is written until `save`.",
        ),
        |_| Ok(Act::Replace),
    );

    add(
        table,
        act(
            "replace-all",
            "Replace every match of the current `find` query in the active tab at once. The tab \
             is changed in the window; nothing is written until `save`.",
        ),
        |_| Ok(Act::ReplaceAll),
    );
}
