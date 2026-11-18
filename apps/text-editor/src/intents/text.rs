//! `set_content`, `append`, `undo`, `redo` and `select_tab`: the tab in front's text, and which
//! tab is in front.

use yantrik_app::control::Param;
use yantrik_app::intents::Intents;

use super::{act, add, arg, given, number};
use crate::command::Act;
use crate::TextEditor;

pub(super) fn table(table: &mut Intents<TextEditor>) {
    add(
        table,
        // Sensitive: it overwrites the whole tab, unsaved paragraphs included, in one call. What
        // it replaces is on disk only if it had been saved.
        act(
            "set_content",
            "Replace everything in the active tab with this text. The file on disk is untouched \
             until `save`; whatever was in the tab and unsaved is gone. To start a document with \
             text, `new` with `text` does it without replacing anything.",
        )
        .risk("sensitive")
        .arg(arg(
            "text",
            "The tab's entire new text, up to 1 MiB and 20,000 lines of UTF-8 with no control \
             characters. An empty string empties the tab.",
        )),
        |a| Ok(Act::SetContent { text: given(a, "set_content", "text", "the tab's entire new text.")? }),
    );

    add(
        table,
        // Standard, unlike `set_content`: it adds to the tab and takes nothing away, and `undo`
        // takes it back off. With `new` it is how a mind under a `standard` ceiling writes a
        // draft here (#253).
        act(
            "append",
            "Add this text to the end of the active tab; on a new, empty tab it is the whole \
             text. Nothing already in the tab is changed, and the file on disk is untouched \
             until `save`.",
        )
        .arg(arg(
            "text",
            "The text to add, exactly as given: start it with a newline to begin a new line. The \
             whole tab has to stay within 1 MiB and 20,000 lines.",
        )),
        |a| Ok(Act::Append { text: a.raw().get("text").and_then(|v| v.as_str()).unwrap_or_default().to_string() }),
    );

    add(
        table,
        // Published because `set_content` and `replace-all` rewrite a whole tab in one call and the
        // person at the keyboard had Ctrl+Z while a mind had nothing.
        act(
            "undo",
            "Take back the last change to the active tab, including a whole-tab `set_content` or \
             `replace-all`. Up to 64 steps are kept per tab and none of it touches the file.",
        ),
        |_| Ok(Act::Undo),
    );

    add(
        table,
        act(
            "redo",
            "Put back the change `undo` took off the active tab. Editing the tab in any other \
             way discards what redo was holding.",
        ),
        |_| Ok(Act::Redo),
    );

    add(
        table,
        act(
            "select_tab",
            "Make one of the open tabs the active one, by its position in `describe.tabs`. \
             Nothing is saved, closed or changed.",
        )
        .arg(Param::number("index").describe(
            "Which tab, counting from 0 in the order `describe.tabs` lists them; \
             `describe.active_tab` is the one in front now.",
        )),
        |a| match number(a, "index") {
            Some(index) => Ok(Act::SelectTab { index }),
            None => Err(Act::Refuse("`select_tab` needs `index`: which tab, counting from 0.".into())),
        },
    );
}
