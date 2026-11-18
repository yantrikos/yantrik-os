//! What a call from the surface is answered with once its work has landed: read from the model
//! and the disk then, never predicted (update/surface.rs records what each call waits for).

use serde_json::{json, Value};

use crate::describe::written;
use crate::model::{Expect, Model};
use crate::store::{read, LIMIT};

const NOTHING_OPEN: &str = "No note is open, so there is nothing to report.";

/// The answer for what the call waited on, read from the model and the disk now. `kept` is the
/// note an outside change's resolution kept the window's text as.
pub(super) fn answer(m: &Model, expect: Expect, landed: bool, kept: Option<String>) -> Result<Value, String> {
    // What went wrong, as the notice says it, or else what is known not to have happened.
    let why = |otherwise: String| if m.notice.is_empty() { otherwise } else { m.notice.clone() };
    let note = || written(m).ok_or_else(|| NOTHING_OPEN.to_string());
    match expect {
        Expect::Written => note(),
        Expect::Saved => {
            let answer = note()?;
            if answer["matches_disk"] != true {
                let path = answer["path"].as_str().unwrap_or_default();
                return Err(why(format!("The note was not written to {path}; the draft is still open.")));
            }
            Ok(answer)
        }
        Expect::Trashed { id, title, path } => {
            if !m.library.iter().any(|n| n.id == id && n.trash) {
                return Err(why(format!("\"{title}\" is still in the library; nothing was trashed.")));
            }
            Ok(json!({
                "trashed": title,
                "filename": id,
                "file_removed": !path.exists(),
                "in_trash": true,
                "trash_count": m.library.iter().filter(|n| n.trash).count(),
                "notes": m.library.iter().filter(|n| !n.trash).count(),
                // Nothing, unless text typed while the job ran became a note of its own.
                "open_note": m.current().map(|c| c.id().to_string()),
            }))
        }
        Expect::Restored { id } => {
            if m.library.iter().any(|n| n.id == id && n.trash) {
                return Err(why("The note is still in the Trash; nothing was restored.".into()));
            }
            note()
        }
        Expect::Recovered { id } => {
            if m.recovery.offer_for(&id).is_some() || m.current().is_none_or(|c| c.id() != id) {
                return Err(why("The unsaved text is still waiting; nothing was restored.".into()));
            }
            note()
        }
        Expect::Discarded { id } => {
            if m.recovery.offer_for(&id).is_some() {
                return Err(why("The unsaved text is still waiting; nothing was discarded.".into()));
            }
            Ok(json!({ "discarded": id, "still_offered": m.recovery.offers.len(), "notice": m.notice }))
        }
        Expect::Copied { from } => {
            let mut answer = note()?;
            if answer["filename"] == json!(from) {
                return Err(why("Nothing was copied; the same note is still open.".into()));
            }
            answer["copied_from"] = json!(from);
            Ok(answer)
        }
        Expect::TookTheirs { id } => {
            let took = m.current().is_some_and(|c| c.id() == id && !c.doc.blocked());
            if !took {
                return Err(why("Nothing was replaced; the window still holds its own text.".into()));
            }
            let mut answer = note()?;
            answer["kept_as"] = json!(kept);
            Ok(answer)
        }
        Expect::Compared => {
            if !landed {
                return Err(why("The version on disk could not be read, so nothing was compared.".into()));
            }
            compared(m).ok_or_else(|| NOTHING_OPEN.to_string())
        }
        Expect::Reloaded => Ok(json!({
            "notes": m.library.iter().filter(|n| !n.trash).count(),
            "in_trash": m.library.iter().filter(|n| n.trash).count(),
            "open_note": m.current().map(|c| c.id().to_string()),
            "library_ready": m.ready,
            "notice": m.notice,
        })),
        Expect::Imported { from } => {
            if !landed {
                return Err(why(format!("Nothing was imported from {}.", from.display())));
            }
            let mut answer = note()?;
            answer["imported_from"] = json!(from.display().to_string());
            Ok(answer)
        }
        Expect::Exported { to, text } => {
            let bytes = read(&to, LIMIT).ok().flatten();
            if !landed || bytes.is_none() {
                return Err(why(format!("Nothing was written to {}.", to.display())));
            }
            Ok(json!({
                "path": to.display().to_string(),
                "bytes": bytes.as_deref().map(str::len),
                "matches_note": bytes.as_deref() == Some(text.as_str()),
                "title": m.current().map(|c| c.title()),
            }))
        }
    }
}

/// How much of each side a comparison answers with: two of them fit a safe read's answer.
const COMPARED_CHARS: usize = 1500;

/// The comparison as a mind reads it: both texts (each cut at [`COMPARED_CHARS`], and said when it
/// is) with their sidecars, whether they are the same, and the first line where the texts part.
fn compared(m: &Model) -> Option<Value> {
    let c = m.current()?;
    let mine = c.content();
    let theirs = m.pane.theirs.as_ref();
    let cut = |text: &str| text.chars().take(COMPARED_CHARS).collect::<String>();
    let first_difference = theirs.and_then(|theirs| {
        let (mut a, mut b) = (mine.text.lines(), theirs.text.lines());
        // The first line number where one side has a line the other does not; none when equal.
        (1..)
            .find_map(|line| match (a.next(), b.next()) {
                (None, None) => Some(None),
                (x, y) if x != y => Some(Some(line)),
                _ => None,
            })
            .flatten()
    });
    Some(json!({
        "title": c.title(),
        "mine": cut(&mine.text),
        "mine_cut_short": mine.text.chars().count() > COMPARED_CHARS,
        "mine_metadata": mine.meta,
        "theirs": theirs.map(|t| cut(&t.text)),
        "theirs_cut_short": theirs.is_some_and(|t| t.text.chars().count() > COMPARED_CHARS),
        "theirs_metadata": theirs.map(|t| t.meta.as_str()),
        "file_exists": theirs.is_some(),
        "identical": theirs == Some(mine),
        "first_different_line": first_difference,
    }))
}
