//! The card for a file the companion is about to overwrite or edit (#1311): `write_file` onto a
//! path that exists, and `edit_file`. Both ask `delete_for_good` ("cannot be undone": what was
//! there is gone) immediately before the `fs::write`, after the path is validated and the
//! replacement computed, so no way into either tool reaches the write around the question.
//!
//! The card shows the path, how much changes, and the first and last lines of what replaces
//! it, each line cut to [`LINE_CHARS`] with the cut marked: the lines are a preview, and the
//! size says how much changes. A path the card cannot show whole is not asked about, and the
//! file is left alone ([`crate::asks::Question::fits`]).

use crate::asks::{act, Card, Question};

/// The longest first or last line the card shows before it marks a cut. Three of these and a
/// path fit a card that shows each value up to [`crate::asks::WHOLE_VALUE_CHARS`].
const LINE_CHARS: usize = 60;

/// One line of text for the card, cut with the cut marked: what was cut is said, not hidden.
fn line(text: &str) -> String {
    let count = text.chars().count();
    if count <= LINE_CHARS {
        return text.to_string();
    }
    format!("{}… ({count} characters in all)", text.chars().take(LINE_CHARS).collect::<String>())
}

fn size(bytes: usize, text: &str) -> String {
    let lines = text.lines().count();
    format!("{bytes} bytes, {lines} line{}", if lines == 1 { "" } else { "s" })
}

/// The first and last line of `text` as the card shows them.
fn ends(card: Card, key_first: &str, key_last: &str, text: &str) -> Card {
    let first = text.lines().next().unwrap_or("");
    let last = text.lines().last().unwrap_or("");
    card.text(key_first, &line(first)).text(key_last, &line(last))
}

/// `write_file` onto a path that exists, without `append`: ask, then replace its content.
pub fn overwrite(path: &str, expanded: &str, content: &str) -> Result<(), String> {
    let before = std::fs::read(expanded).map(|b| String::from_utf8_lossy(&b).into_owned()).ok();
    let was = match &before {
        Some(old) => size(old.len(), old),
        None => "a file whose current size could not be read".to_string(),
    };
    let card = Card::new("write_file").text("path", path).plain("change", format!("replaces {was} with {}", size(content.len(), content)));
    let card = ends(card, "first_line", "last_line", content);
    act(Question::DeleteForGood, &card, || {
        std::fs::write(expanded, content).map_err(|e| format!("Error writing file: {e}"))
    })
}

/// `edit_file`: ask, then write the edited content back.
pub fn edit(path: &str, expanded: &str, replaced: usize, old: &str, new: &str, before: &str, after: &str) -> Result<(), String> {
    let card = Card::new("edit_file")
        .text("path", path)
        .plain("change", format!("replaces {replaced} place(s) in a file of {}; it becomes {}", size(before.len(), before), size(after.len(), after)));
    let card = ends(ends(card, "removes_first", "removes_last", old), "first_line", "last_line", new);
    act(Question::DeleteForGood, &card, || std::fs::write(expanded, after).map_err(|e| format!("Error writing file: {e}")))
}
