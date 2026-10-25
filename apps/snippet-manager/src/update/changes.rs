//! The changes to what is kept: new, save, delete, star, language, and the collections. Each is
//! made in memory, validated first, and written by the lane (`written::write`); a change the store
//! refuses is said on the notice line and as the call's refusal, and nothing is written for it.

use serde_json::Value;
use yantrik_app::Refused;

use super::browsing::named;
use super::{fail, say, written, Fx};
use crate::answers;
use crate::command::{After, Command};
use crate::model::{Editor, Model, Restore, PART_DETAIL, PART_LIST};
use crate::store::{Patch, ALL, FAVORITES};

/// The editor's typing, kept: the store takes it before anything moves the editor or repaints it
/// from the store. Says whether there was any, so the caller writes it.
pub(super) fn commit(m: &mut Model) -> Result<bool, String> {
    // Text offered back is kept only by the person's Restore (or Save): nothing is put back by a
    // move of the editor. It stays in its draft.
    if !m.editor_dirty() || m.offered(m.editor.id).is_some() {
        return Ok(false);
    }
    let saved = m.library.save(m.editor.id, m.editor.patch())?;
    m.editor = Editor::of(&saved);
    Ok(true)
}

pub(super) fn update(m: &mut Model, command: Command, fx: &mut Fx) -> Result<(), Refused> {
    match command {
        Command::New => create(m, fx, "Untitled Snippet", "Rust", "", "", false),
        Command::Add { title, language, code, tags } => create(m, fx, &title, &language, &code, &tags, true),
        Command::Save(editor) => {
            let restore = m.restore_point();
            let id = editor.id;
            match m.library.save(id, editor.patch()) {
                Ok(_) => {
                    m.offers.retain(|o| o.id != id);
                    fx.changed(crate::model::PART_RECOVERY);
                    follow(m, id);
                    written::write(m, fx, After::Answer(Value::Null), restore);
                }
                Err(why) => fail(m, fx, &why),
            }
            redraw(fx);
        }
        Command::Change { id, patch, collection } => change(m, fx, &id, patch, collection)?,
        Command::Delete(id) => delete(m, fx, id, false),
        Command::DeleteNamed(needle) => {
            let id = named(m, &needle)?;
            delete(m, fx, id, true);
        }
        Command::ToggleFavorite(id) => star(m, fx, id, false),
        Command::FavoriteNamed(needle) => {
            let id = named(m, &needle)?;
            star(m, fx, id, true);
        }
        Command::SetLanguage(language) => {
            let id = m.editor.id;
            if id < 0 {
                say(m, fx, "Open a snippet before choosing its language.");
                return Ok(());
            }
            // As the favourite button: the typing is kept, then one field saved, then a repaint
            // out of the store.
            let restore = m.restore_point();
            let Some(committed) = commit_or_say(m, fx) else { return Ok(()) };
            let outcome = m
                .library
                .save(id, Patch { language: Some(language), ..Patch::default() })
                .map(|_| Value::Null);
            settle_change(m, fx, outcome, committed, restore, id);
        }
        Command::CollectionCreate(name) => {
            let restore = m.restore_point();
            let name = free_collection_name(m, name.trim());
            match m.library.collection_create(&name) {
                Ok(id) => {
                    m.collection = id;
                    written::write(m, fx, After::Answer(Value::Null), restore);
                }
                Err(why) => fail(m, fx, &why),
            }
            fx.changed(PART_LIST);
        }
        Command::CollectionRename(id, name) => {
            let restore = m.restore_point();
            match m.library.collection_rename(id, &name) {
                Ok(_) => written::write(m, fx, After::Answer(Value::Null), restore),
                Err(why) => fail(m, fx, &why),
            }
            fx.changed(PART_LIST);
        }
        Command::CollectionDelete(id) => {
            let restore = m.restore_point();
            match m.library.collection_delete(id) {
                Ok(moved) => {
                    // The sidebar cannot stay pointed at a collection that has gone.
                    if m.collection == id {
                        m.collection = ALL;
                    }
                    // Said out loud: a person who deletes a folder expecting its contents to go
                    // with it should not have to work out where they went.
                    let after = if moved > 0 {
                        After::Say(format!("{moved} snippet(s) moved back to All Snippets."))
                    } else {
                        After::Answer(Value::Null)
                    };
                    written::write(m, fx, after, restore);
                }
                Err(why) => fail(m, fx, &why),
            }
            // The list and the sidebar, not the editor: nothing the editor shows belongs to a
            // collection, and repainting it here would throw away uncommitted typing.
            fx.changed(PART_LIST);
        }
        _ => {}
    }
    Ok(())
}

/// Keep a new snippet and make it the one on screen. `asked`: the Mind's `new`, answered with it.
fn create(m: &mut Model, fx: &mut Fx, title: &str, language: &str, code: &str, tags: &str, asked: bool) {
    let restore = m.restore_point();
    let Some(committed) = commit_or_say(m, fx) else { return };
    // A snippet made while a collection is selected belongs to it. This is the only way a custom
    // collection gains a member from the window: there is no move-between-collections control,
    // and the `save` action's `collection` argument is the other way in. Favourites is a filter,
    // so creating there files under no collection and starring is what puts a snippet in it.
    let outcome = m.library.create(title, language, code, tags, m.collection).map(|id| {
        // The new snippet must be visible, or "New" looks like it did nothing. Whatever was
        // narrowing the list is cleared, and the sidebar follows the snippet.
        m.select(id);
        m.query.clear();
        m.tag.clear();
        if m.collection == FAVORITES {
            m.collection = ALL;
        }
        if asked {
            answers::stored(m, id)
        } else {
            Value::Null
        }
    });
    let id = m.selected;
    settle_change(m, fx, outcome, committed, restore, id);
}

/// The Mind's `save`: whatever the person has typed is kept first, and the caller's change lands
/// on top of it, so a mind saving a snippet never silently takes somebody's unfinished edit.
fn change(m: &mut Model, fx: &mut Fx, needle: &str, mut patch: Patch, collection: Option<String>) -> Result<(), Refused> {
    let id = named(m, needle)?;
    if let Some(wanted) = collection {
        patch.collection = Some(collection_named(m, &wanted).map_err(|why| Refused::new("not_found", why))?);
    }
    if patch.is_empty() {
        return Err(Refused::new(
            "nothing_to_change",
            "nothing to change: give at least one of title, language, code, tags, favorite or collection",
        ));
    }
    let restore = m.restore_point();
    let Some(committed) = commit_or_say(m, fx) else { return Ok(()) };
    let answer = m.library.save(id, patch).map(|_| answers::stored(m, id));
    settle_change(m, fx, answer, committed, restore, id);
    Ok(())
}

fn delete(m: &mut Model, fx: &mut Fx, id: i32, asked: bool) {
    let restore = m.restore_point();
    match m.library.delete(id) {
        Ok(removed) => {
            if m.selected == id {
                m.select(-1);
            }
            let draft_goes = !asked && m.drafts.contains_key(&id);
            let answer = if asked { answers::deleted(m, &removed) } else { Value::Null };
            written::write(m, fx, After::Answer(answer), restore);
            // The person deleting the snippet throws its unsaved text away with it, once the write
            // that deletes it has landed; a refused delete brings both back.
            if draft_goes {
                let issued = m.lane.revision();
                if let Some(d) = m.drafts.get_mut(&id) {
                    d.pending = Some(issued);
                }
            }
        }
        Err(why) => fail(m, fx, &why),
    }
    redraw(fx);
}

/// Starring saves one field and the repaint after it comes out of the store, so whatever is in
/// the editor goes in first or the star costs the person their typing.
fn star(m: &mut Model, fx: &mut Fx, id: i32, asked: bool) {
    let restore = m.restore_point();
    let Some(committed) = commit_or_say(m, fx) else { return };
    let answer = m
        .library
        .toggle_favorite(id)
        .map(|_| if asked { answers::starred(m, id) } else { Value::Null });
    settle_change(m, fx, answer, committed, restore, id);
}

/// A change to one snippet, made or refused: written with its answer, or said and refused. The
/// editor follows the store when it shows that snippet.
fn settle_change(
    m: &mut Model,
    fx: &mut Fx,
    outcome: Result<Value, String>,
    committed: bool,
    restore: Restore,
    id: i32,
) {
    match outcome {
        Ok(answer) => {
            follow(m, id);
            written::write(m, fx, After::Answer(answer), restore);
        }
        Err(why) => {
            fail(m, fx, &why);
            if committed {
                // The typing was taken before the change was refused; it is still written.
                written::write(m, fx, After::Say(why), restore);
            }
        }
    }
    redraw(fx);
}

/// [`commit`], or the reason it could not be kept said and refused: `None`, and nothing moves.
fn commit_or_say(m: &mut Model, fx: &mut Fx) -> Option<bool> {
    match commit(m) {
        Ok(committed) => Some(committed),
        Err(why) => {
            fail(m, fx, &why);
            redraw(fx);
            None
        }
    }
}

/// The editor shows what is kept for `id` once a change to it is made.
fn follow(m: &mut Model, id: i32) {
    if m.selected == id {
        m.select(id);
    }
}

fn redraw(fx: &mut Fx) {
    fx.changed(PART_LIST);
    fx.changed(PART_DETAIL);
}

/// A name no collection is using yet.
///
/// The "+" button has no text field behind it, so it always asks for "New Collection"; the store
/// refuses a duplicate name, and a button that fails the second time it is pressed is not a
/// working button. A caller that names a collection itself still gets the refusal.
fn free_collection_name(m: &Model, base: &str) -> String {
    let taken = |name: &str| {
        m.library
            .collections()
            .iter()
            .any(|c| c.name.to_lowercase() == name.to_lowercase())
    };
    if !taken(base) {
        return base.to_string();
    }
    for n in 2..1000 {
        let candidate = format!("{base} {n}");
        if !taken(&candidate) {
            return candidate;
        }
    }
    format!("{base} {}", crate::store::text::now())
}

/// The id of the collection a caller named, by id or by name.
fn collection_named(m: &Model, wanted: &str) -> Result<i32, String> {
    let wanted = wanted.trim();
    if wanted.is_empty() {
        return Ok(ALL);
    }
    let collections = m.library.collections();
    if let Ok(id) = wanted.parse::<i32>() {
        if id == ALL || collections.iter().any(|c| c.id == id) {
            return Ok(id);
        }
    }
    collections
        .iter()
        .find(|c| c.name.to_lowercase() == wanted.to_lowercase())
        .map(|c| c.id)
        .ok_or_else(|| {
            let known: Vec<String> = collections.iter().map(|c| c.name.clone()).collect();
            if known.is_empty() {
                format!("no collection called `{wanted}`; this store has none yet")
            } else {
                format!("no collection called `{wanted}`; there is {}", known.join(", "))
            }
        })
}

