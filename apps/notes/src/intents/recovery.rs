//! The recovery bar's choices (step 8d-3), the same commands its buttons run.

use yantrik_app::intents::Intents;

use super::{act, add};
use crate::command::Act;
use crate::Notes;

pub(super) fn table(table: &mut Intents<Notes>) {
    // The recovery bar's choices (step 8d-3), the same commands its buttons run, about the text a
    // run of Notes kept and did not save. `describe.recovery` lists what is offered, the bar's
    // first: the one these act on.
    add(
        table,
        // Sensitive: the text in the note is replaced by the text kept from before Notes closed,
        // and saved. When the note's file changed since, nothing is replaced: the outside choices
        // are offered instead.
        act(
            "restore_draft",
            "Put back the unsaved text Notes kept from before it closed (the one the recovery bar \
             shows, first in `describe.recovery`) into its note, open it, and save it. If the note's \
             file changed since, nothing is replaced: `describe.outside` becomes `conflict` and the \
             outside choices apply.",
        )
        .risk("sensitive"),
        |_| Ok(Act::RestoreDraft),
    );

    add(
        table,
        // Dangerous: the kept text is the only copy of what was typed before Notes closed, and it
        // is removed for good. The note itself stays as its file holds it.
        act(
            "discard_draft",
            "Throw away the unsaved text Notes kept from before it closed (the one the recovery bar \
             shows, first in `describe.recovery`). The note stays as its file holds it; the text cannot \
             be got back.",
        )
        .risk("dangerous"),
        |_| Ok(Act::DiscardDraft),
    );
}
