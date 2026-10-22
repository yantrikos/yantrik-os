//! What each intent answers, built from its own result, and settled in the command where that
//! result lands.
//!
//! A change's answer is made when the change is made, from the library right after it, and is
//! carried by its write to the command where the write lands (`After::Answer`); nothing another
//! call or the person does meanwhile can become it. An intent that writes nothing is answered in
//! its own command. Either way the answer is written to `Model::answer` and read by the reply in
//! the same command, before anything else can change it.

use serde_json::{json, Value};
use yantrik_app::effects::Effects;

use crate::model::Model;
use crate::store::Snippet;
use crate::SnippetManager;

/// How many snippets `describe` and `search` carry. The state is a glance, not a transcript —
/// the true count travels beside it, so a caller is never misled about how many there are.
pub const LISTING_CAP: usize = 20;

/// Settle the invocation this command runs under with `answer`, if it came through the surface.
pub fn settle(m: &mut Model, fx: &mut Effects<SnippetManager>, answer: Value) {
    if fx.invocation().is_some() {
        m.answer = answer;
        fx.settle(Ok(()));
    }
}

/// What `new`, `save` and `open` answer: the snippet as the library holds it right after the
/// change. Never its code, and no length of it: a length is the same hint at a secret whichever
/// action reveals it.
pub fn stored(m: &Model, id: i32) -> Value {
    let stored = m.library.get(id);
    json!({
        "id": id,
        "title": stored.map(|s| s.title.clone()),
        "language": stored.map(|s| s.language.clone()),
        "tags": stored.map(|s| s.tag_list()),
        "favorite": stored.map(|s| s.favorite),
        "collection": stored.map(|s| m.library.collection_name(s.collection)),
        "stored_in": m.lane.path().display().to_string(),
        "total": m.library.snippets().len(),
    })
}

/// What `search` answers: how much matched, and the first titles.
pub fn searched(m: &Model, query: &str) -> Value {
    let rows = m.rows();
    json!({
        "query": query,
        "matching": rows.len(),
        "shown": rows.len().min(LISTING_CAP),
        "of": m.library.snippets().len(),
        "titles": rows.iter().take(LISTING_CAP).map(|s| s.title.clone()).collect::<Vec<_>>(),
    })
}

/// What `delete` answers: what went, read back from the library rather than assumed.
pub fn deleted(m: &Model, removed: &Snippet) -> Value {
    json!({
        "deleted": removed.title,
        "id": removed.id,
        "still_present": m.library.get(removed.id).is_some(),
        "remaining": m.library.snippets().len(),
    })
}

/// What `toggle_favorite` answers.
pub fn starred(m: &Model, id: i32) -> Value {
    let snippet = m.library.get(id);
    json!({
        "id": id,
        "title": snippet.map(|s| s.title.clone()),
        "favorite": snippet.map(|s| s.favorite),
        "favorites": m.library.favorites(),
    })
}

/// What `copy` answers: that it copied, and which snippet. Never the code, and no length or size
/// of it either: a length is a hint at a secret (a four-digit PIN is four bytes).
pub fn copied(id: i32, title: &str, use_count: i32, saved: bool) -> Value {
    json!({
        "copied": title,
        "id": id,
        "with": "wl-copy",
        "use_count": use_count,
        "use_count_saved": saved,
    })
}

/// What `import` answers.
pub fn imported(m: &Model, ids: &[i32], kind: &str) -> Value {
    json!({
        "imported": ids.len(),
        "as": kind,
        "ids": ids,
        "titles": ids.iter().filter_map(|id| m.library.get(*id).map(|s| s.title.clone())).collect::<Vec<_>>(),
        "total": m.library.snippets().len(),
    })
}
