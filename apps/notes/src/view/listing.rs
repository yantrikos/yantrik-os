//! The library list: which notes a section and a search show, in what order, and the counts and
//! notebooks beside them.

use crate::note::{Kept, Note};
use crate::update::said;

/// One row as the list draws it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// `trash:<file>` for a note in the Trash, so a click opens the right one.
    pub id: String,
    pub title: String,
    pub preview: String,
    pub date: String,
    /// Said beside the date for a note Notes made to keep text aside (a copy, a recovered note),
    /// whose title is the note's it was kept from; empty for any other.
    pub marker: &'static str,
    pub pinned: bool,
    pub selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Book {
    pub name: String,
    pub count: i32,
    pub selected: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Listing {
    pub rows: Vec<Row>,
    pub books: Vec<Book>,
    pub all_count: i32,
    pub pin_count: i32,
    pub trash_count: i32,
}

/// What "recent" reaches back to.
const RECENT_SECS: u64 = 7 * 86400;

/// The list for `folder` and `query`, with `open` (the note the window shows, as it is now) in
/// place of its library entry. `now` is seconds since the epoch.
///
/// `search_text` is whether the query is matched against the notes' text as well as their titles
/// and fields; a window drawn for a Mind matches only what it shows.
pub fn list(library: &[Note], open: Option<&Note>, folder: &str, query: &str, search_text: bool, now: u64) -> Listing {
    let q = query.trim().to_lowercase();
    let is_open = |n: &Note| open.is_some_and(|c| c.id == n.id && c.trash == n.trash);
    let all: Vec<&Note> = library
        .iter()
        .map(|n| if is_open(n) { open.unwrap_or(n) } else { n })
        .collect();
    let mut books = std::collections::BTreeMap::<String, i32>::new();
    for n in &all {
        let name = n.field("notebook");
        if !n.trash && !name.is_empty() {
            *books.entry(name).or_default() += 1;
        }
    }
    let mut visible: Vec<&Note> = all
        .iter()
        .copied()
        .filter(|n| {
            let scope = match folder {
                "trash" => n.trash,
                "favorites" => !n.trash && n.content.pinned(),
                "recent" => !n.trash && now.saturating_sub(n.modified) < RECENT_SECS,
                f if f.starts_with("book:") => !n.trash && n.field("notebook") == f[5..],
                _ => !n.trash,
            };
            scope
                && (q.is_empty()
                    || (search_text && n.content.text.to_lowercase().contains(&q))
                    || n.title().to_lowercase().contains(&q)
                    || n.content.meta.to_lowercase().contains(&q))
        })
        .collect();
    visible.sort_by(|a, b| {
        b.field("pinned")
            .cmp(&a.field("pinned"))
            .then(b.modified.cmp(&a.modified))
            .then(a.id.cmp(&b.id))
    });
    Listing {
        all_count: all.iter().filter(|n| !n.trash).count() as i32,
        pin_count: all.iter().filter(|n| !n.trash && n.content.pinned()).count() as i32,
        trash_count: all.iter().filter(|n| n.trash).count() as i32,
        books: books
            .into_iter()
            .map(|(name, count)| Book { selected: folder == format!("book:{name}"), name, count })
            .collect(),
        rows: visible
            .into_iter()
            .map(|n| Row {
                id: format!("{}{}", if n.trash { "trash:" } else { "" }, n.id),
                title: n.title(),
                preview: n
                    .content
                    .text
                    .lines()
                    .find(|l| !l.trim().is_empty() && !l.starts_with('#'))
                    .unwrap_or("A fresh page")
                    .chars()
                    .take(90)
                    .collect(),
                date: chrono::DateTime::from_timestamp(n.modified as i64, 0)
                    .map(|d| d.with_timezone(&chrono::Local).format("%b %-d · %H:%M").to_string())
                    .unwrap_or_default(),
                marker: Kept::of(&n.id).map(said::kept_marker).unwrap_or_default(),
                pinned: n.content.pinned(),
                selected: is_open(n),
            })
            .collect(),
    }
}
