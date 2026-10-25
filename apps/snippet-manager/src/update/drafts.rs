//! The person's unsaved text, in the framework's draft lane, one draft per snippet; and the
//! recovery bar, which offers back what a draft holds until the person chooses.
//!
//! A snippet's text is drafted while the editor holds typing the store has not taken, debounced,
//! and at once when the app ends or a save of it is refused. One that the editor has moved away
//! from while its text is still not kept (its save was refused, or a mind deleted the snippet
//! under it) is offered back on the bar, as a draft found at the next start is (`lib.rs`). Nothing
//! is put back until the person chooses: *Restore* keeps the text in its snippet (*Restore as new*
//! for one no longer kept), *Discard* throws it away. Notes offers its drafts the same way.
//!
//! # When a draft goes: one rule, in [`release`]
//!
//! A draft is removed only when a write of the library that holds its text has positively landed
//! (`Saved` or `Covered`), or when the person presses *Discard*. Nothing else removes one: not a
//! refused, undone or unfinished write, not a failed re-read, not a move of the text to another
//! snippet's name, not the snippet being kept in memory (#938 delta review, M2b and N1; #925 N-B,
//! #920 E-L7). Every write records which drafts' text it carries (`Draft::pending`,
//! `written::write`); its landing releases them; any failure takes that back.

use yantrik_app::Refused;

use super::changes::commit;
use super::{fail, say, written, Fx};
use crate::command::{After, Command};
use crate::model::{draft_name, Draft, Editor, Model, Offer, Why, PART_DETAIL, PART_LIST, PART_RECOVERY};
use crate::store::ALL;

/// Why a draft may go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Because {
    /// A write holding its text has landed (`Model::lane`'s confirmed revision is at or past the
    /// draft's `pending`).
    Kept,
    /// The person pressed *Discard*.
    Discarded,
}

/// The one way a draft is removed, and the rule it is held to. Says whether it went.
pub(super) fn release(m: &mut Model, fx: &mut Fx, id: i32, because: Because) -> bool {
    let Some(draft) = m.drafts.get(&id) else { return false };
    let allowed = match because {
        Because::Discarded => true,
        Because::Kept => draft.pending.is_some_and(|p| p <= m.lane.confirmed()),
    };
    if !allowed {
        return false;
    }
    m.drafts.remove(&id);
    fx.discard_draft(&draft_name(id));
    if m.offers.iter().any(|o| o.id == id) {
        m.offers.retain(|o| o.id != id);
        fx.changed(PART_RECOVERY);
    }
    true
}

/// The editor's text drafted while it is unsaved; drafts whose text a landed write holds let go;
/// the others offered once the editor has left them. `now`: written at once rather than after the
/// debounce.
pub(super) fn keep(m: &mut Model, fx: &mut Fx, now: bool) {
    let editor = m.editor.clone();
    let typing = editor.id >= 0 && (m.editor_dirty() || m.offered(editor.id).is_some());
    // The person typed in the editor since this snippet's draft was made, and the text differs.
    let typed_since = m.drafts.get(&editor.id).is_some_and(|d| d.text != editor && m.typed > d.typed);
    if typing && (now || m.drafted(editor.id) != Some(&editor)) {
        put(fx, &editor, now);
        // The editor's text is the newest there is for its snippet, and no write holds it yet.
        m.drafts.insert(editor.id, Draft::of(editor.clone(), m.typed));
    } else if !typing && typed_since {
        // The person typed the snippet back to what is kept: the draft follows the editor, and the
        // newest write issued holds that text, so it goes when that write is known to have landed.
        put(fx, &editor, now);
        let mut draft = Draft::of(editor.clone(), m.typed);
        draft.pending = Some(m.lane.revision());
        m.drafts.insert(editor.id, draft);
    }
    let ids: Vec<i32> = m.drafts.keys().copied().collect();
    for id in ids {
        if release(m, fx, id, Because::Kept) {
            continue;
        }
        let draft = &m.drafts[&id];
        let in_editor = id == editor.id && typing;
        let offered = m.offers.iter().any(|o| o.id == id);
        // Held in memory: kept there, and on its way to disk or waiting for the next write (after
        // a write that did not finish, memory may be ahead of the file; the draft stays).
        let held = m.library.get(draft.kept_as).is_some() && !m.library.differs(draft.kept_as, &draft.text.patch());
        if !in_editor && !offered && draft.pending.is_none() && !held {
            m.offers.push(Offer { id, why: Why::NotSaved });
            fx.changed(PART_RECOVERY);
        }
    }
}

/// A text the store did not take, kept now: its draft written at once. A text older than the one
/// the draft lane already holds for its snippet is not: the newest typing always wins (#938
/// review, M1). Says whether it was held.
pub(super) fn hold(m: &mut Model, fx: &mut Fx, text: &Editor, typed: u64) -> bool {
    if m.drafts.get(&text.id).is_some_and(|d| d.typed > typed) {
        return false;
    }
    put(fx, text, true);
    m.drafts.insert(text.id, Draft::of(text.clone(), typed));
    true
}

/// A write whose outcome is not that it landed (refused, undone, unfinished): no draft is let go
/// on its account, nor on any later write's of its epoch.
pub(super) fn not_landed(m: &mut Model, revision: u64) {
    for draft in m.drafts.values_mut() {
        if draft.pending.is_some_and(|p| p >= revision) {
            draft.pending = None;
        }
    }
}

/// The draft lane could not write a snippet's draft: said, so the person knows the text is only
/// in the window. No other copy of it is removed (`release` waits for a landed write).
pub(super) fn not_drafted(m: &mut Model, fx: &mut Fx, name: &str, why: &str) {
    if name.starts_with(crate::model::DRAFT_PREFIX) {
        say(m, fx, &format!("Your unsaved text could not be kept as a draft ({why}); it is still in Snippets. Save it, or copy it out."));
    }
}

fn put(fx: &mut Fx, text: &Editor, now: bool) {
    let Ok(bytes) = serde_json::to_vec(text) else { return };
    if now {
        fx.checkpoint_draft(&draft_name(text.id), bytes);
    } else {
        fx.save_draft(&draft_name(text.id), bytes);
    }
}

/// The bar's choices, about the first text it offers.
pub(super) fn choose(m: &mut Model, command: Command, fx: &mut Fx) {
    let Some(offer) = m.offers.first().copied() else { return };
    let Some(draft) = m.drafts.get(&offer.id).cloned() else {
        m.offers.retain(|o| o.id != offer.id);
        fx.changed(PART_RECOVERY);
        return;
    };
    match command {
        Command::ShowOffer => {
            if m.library.get(offer.id).is_some() {
                super::browsing::move_to(m, fx, offer.id, false);
            }
        }
        Command::RestoreOffer => restore(m, fx, offer, draft),
        Command::DiscardOffer => {
            release(m, fx, offer.id, Because::Discarded);
            if m.editor.id == offer.id {
                // The editor shows the snippet as it is kept again.
                m.select(offer.id);
            }
        }
        _ => {}
    }
    fx.changed(PART_RECOVERY);
    fx.changed(PART_DETAIL);
    fx.changed(PART_LIST);
}

/// *Restore*: the text back into its snippet, opened and kept; *Restore as new* for a snippet no
/// longer kept. The typing in the editor goes in first, as any move of the editor keeps it. The
/// draft stays, under its own name, until the write that holds its text has landed (`release`),
/// so a refused write offers it again.
fn restore(m: &mut Model, fx: &mut Fx, offer: Offer, draft: Draft) {
    let Draft { text, typed, .. } = draft;
    let mut restore = m.restore_point();
    let committed = match commit(m) {
        Ok(committed) => committed,
        Err(why) => return fail(m, fx, &why),
    };
    m.offers.retain(|o| o.id != offer.id);
    let outcome = if m.library.get(offer.id).is_some() {
        m.select(offer.id);
        m.editor = Editor { id: offer.id, ..text };
        commit(m).map(|_| ())
    } else {
        m.library.create(&text.title, &text.language, &text.code, &text.tags, ALL).map(|id| {
            m.select(id);
            // The draft keeps its own name and file, and now names the new snippet as where its
            // text is kept: it goes when the new snippet's write has landed, and never before
            // (#938 delta review, N1). If that write is refused, it is this draft the refusal
            // keeps and says is kept (#938 review, L1).
            if let Some(d) = m.drafts.get_mut(&offer.id) {
                d.kept_as = id;
            }
            restore.typing = Editor { id: offer.id, ..text.clone() };
            restore.typed = typed;
        })
    };
    match outcome {
        Ok(()) => written::write(m, fx, After::Answer(serde_json::Value::Null), restore),
        Err(why) => {
            m.offers.insert(0, offer);
            fail(m, fx, &why);
            if committed {
                written::write(m, fx, After::Say(why), restore);
            }
        }
    }
}

/// The bar's button, as the window sends it.
pub fn choice(action: &str) -> Result<Command, Refused> {
    match action {
        "show" => Ok(Command::ShowOffer),
        "restore" => Ok(Command::RestoreOffer),
        "discard" => Ok(Command::DiscardOffer),
        other => Err(Refused::new("no_such_choice", format!("the recovery bar has no `{other}`"))),
    }
}

/// What the bar says and offers: the sentence, and 1 when its snippet is open in the editor, 2
/// when it is kept and not open (Show), 3 when it is no longer kept (Restore as new). `None`: no
/// bar.
pub fn bar(m: &Model) -> Option<(String, i32)> {
    let offer = m.offers.first()?;
    let text = m.drafted(offer.id)?;
    let title = format!("\u{201c}{}\u{201d}", text.title);
    let (state, said) = match (m.library.get(offer.id).is_some(), m.editor.id == offer.id, offer.why) {
        (false, _, _) => (3, format!("Your unsaved changes to {title} are kept, but that snippet is no longer kept.")),
        (true, true, Why::Closed) => (1, format!("Snippets closed before your changes to {title} were kept. They are in the editor, not saved yet.")),
        (true, false, Why::Closed) => (2, format!("Snippets closed before your changes to {title} were kept. They are kept as a draft.")),
        (true, true, Why::NotSaved) => (1, format!("Your changes to {title} could not be saved. They are in the editor and kept as a draft.")),
        (true, false, Why::NotSaved) => (2, format!("Your changes to {title} could not be saved. They are kept as a draft.")),
    };
    let more = m.offers.len();
    let said = if more > 1 { format!("{said} (1 of {more})") } else { said };
    Some((said, state))
}
