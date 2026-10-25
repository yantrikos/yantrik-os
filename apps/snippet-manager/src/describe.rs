//! What the Mind reads: the same keys and values the app published before the framework, read off
//! the committed model.
//!
//! In Private mode the framework replaces all of it (#924); this app keeps none of its keys there
//! (`tests/private_mode.rs`), because every one names, counts or quotes the person's snippets.

use serde_json::{json, Value};
use yantrik_app::control::View;

use crate::answers::LISTING_CAP;
use crate::model::Model;

pub fn view(m: &Model) -> View {
    let total = m.library.snippets().len();
    let rows = m.rows();
    let open = m.open().cloned();
    let query = m.query.trim();

    let summary = match (&open, query.is_empty()) {
        (Some(s), true) => format!("Snippets \u{2014} \u{201c}{}\u{201d}, {total} kept", s.title),
        (Some(s), false) => format!(
            "Snippets \u{2014} \u{201c}{}\u{201d}, {} of {total} match \u{201c}{query}\u{201d}",
            s.title,
            rows.len()
        ),
        (None, true) if total == 0 => "Snippets \u{2014} nothing kept yet".to_string(),
        (None, true) => format!("Snippets \u{2014} {total} kept, none open"),
        (None, false) => format!(
            "Snippets \u{2014} {} of {total} match \u{201c}{query}\u{201d}, none open",
            rows.len()
        ),
    };

    let listed: Vec<Value> = rows
        .iter()
        .take(LISTING_CAP)
        .map(|s| {
            json!({
                "id": s.id,
                "title": s.title,
                "language": s.language,
                "tags": s.tag_list(),
                "favorite": s.favorite,
                "collection": m.library.collection_name(s.collection),
                "lines": s.code.lines().count(),
                "preview": s.preview(),
            })
        })
        .collect();

    View::new(summary)
        .with("total", total as i64)
        .with("matching", rows.len() as i64)
        .with("shown", listed.len() as i64)
        .with("snippets", listed)
        .with("favorites", m.library.favorites() as i64)
        .with("query", m.query.clone())
        .with("tag", m.tag.clone())
        .with("collection", m.library.collection_name(m.collection))
        .with(
            "collections",
            m.library.collections().iter().map(|c| c.name.clone()).collect::<Vec<_>>(),
        )
        .with("open", open.as_ref().map(|s| s.title.clone()))
        .with("open_id", open.as_ref().map(|s| s.id as i64))
        .with("open_language", open.as_ref().map(|s| s.language.clone()))
        .with("open_favorite", open.as_ref().map(|s| s.favorite))
        .with("open_lines", open.as_ref().map(|s| s.code.lines().count() as i64))
        .with("store", m.lane.path().display().to_string())
        .with("copied_recently", m.copied)
        .with("notice", m.notice.clone())
        .with("recovery", recovery(m))
}

/// The texts offered back on the recovery bar, the bar's first: which snippet, by title, and
/// whether it is still kept. Never the text.
fn recovery(m: &Model) -> Vec<Value> {
    m.offers
        .iter()
        .filter_map(|o| m.drafted(o.id).map(|text| (o, text)))
        .map(|(o, text)| json!({ "id": o.id, "title": text.title, "gone": m.library.get(o.id).is_none() }))
        .collect()
}
