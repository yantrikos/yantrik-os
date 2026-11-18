//! `close`, `cancel` and `discard`: closing a tab, and answering the dialog that asks.

use yantrik_app::intents::Intents;

use super::{act, add};
use crate::command::Act;
use crate::TextEditor;

pub(super) fn table(table: &mut Intents<TextEditor>) {
    add(
        table,
        act(
            "close",
            "Close the active tab. A tab with unsaved changes is not closed: the window asks, \
             and `save`, `discard` or `cancel` answers it; an agent is refused such a tab when \
             the person opened it. The last tab closed leaves an empty one.",
        ),
        |_| Ok(Act::Close),
    );

    add(
        table,
        act(
            "cancel",
            "Dismiss whichever dialog is open and leave every tab exactly as it was. Nothing is \
             closed, saved or discarded.",
        ),
        |_| Ok(Act::Cancel),
    );

    add(
        table,
        // Sensitive: this is the action that throws away work. The tab's unsaved text is gone
        // when it returns, its draft removed with it, and nothing on this machine keeps a copy.
        act(
            "discard",
            "Throw away the unsaved changes in the tab the window is asking about and close it. \
             The text that was never saved is gone and is not recoverable.",
        )
        .risk("sensitive"),
        |_| Ok(Act::Discard),
    );
}
