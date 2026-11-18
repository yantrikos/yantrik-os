//! What a mind reads first: one line that says which sheet is in front, what is selected and
//! whether it is saved; and the state behind it, with the same keys and values ySheets has always
//! published.
//!
//! A sheet the person opened from a place an agent may not read is not read back to one: not its
//! cells, its size, its selection, its counts, its status line or its notice, any of which can
//! quote a cell (security reviews of #727).

use serde_json::json;
use yantrik_app::control::View;

use crate::address::{self, Cell};
use crate::caller::Caller;
use crate::formula::functions::NAMES;
use crate::model::Model;

pub fn view(m: &Model) -> View {
    let caller = Caller::now();
    if caller.agent && m.doc.hidden {
        return hidden(m);
    }
    let sheet = m.sheet();
    let (used_rows, used_cols) = sheet.used();
    let (filled, errors) = sheet.counts();
    let notice = m.notice.read(caller.agent).to_string();
    let mut summary = format!(
        "ySheets — {} · {} rows × {} columns · {} selected · {}",
        m.title(),
        sheet.rows(),
        sheet.cols(),
        m.active.name(),
        if m.modified() { "unsaved changes" } else { "saved" }
    );
    if !notice.is_empty() {
        summary.push_str(&format!(" · {notice}"));
    }
    View::new(summary)
        .with("sheet", m.title())
        .with("path", m.path().map(|p| p.display().to_string()))
        .with("rows", sheet.rows() as i64)
        .with("cols", sheet.cols() as i64)
        .with(
            "used",
            if used_rows == 0 {
                "".to_string()
            } else {
                address::Range::new(Cell::new(0, 0), Cell::new(used_rows - 1, used_cols - 1)).name()
            },
        )
        .with(
            "selection",
            json!({
                "cell": m.active.name(),
                "typed": sheet.raw(m.active),
                "value": sheet.value(m.active).to_json(),
            }),
        )
        .with("dirty", m.modified())
        .with("can_undo", m.book().history.can_undo())
        .with("can_redo", m.book().history.can_redo())
        .with("filled_cells", filled as i64)
        .with("error_cells", errors as i64)
        .with("first_error", sheet.first_error().map(Cell::name))
        .with("status", m.status().text)
        .with("notice", notice)
        .with("hidden", None::<String>)
        .with("functions", NAMES.to_vec())
}

/// What an agent sees of a sheet it may not read: its name, whether it has unsaved changes (an
/// agent's `open` is refused until it is saved), and the sentence every refused action gives.
/// Nothing of its shape (size, used range, counts, the first error, the selected cell, its
/// history), and not the status line or the notice, which can quote a cell. The path is left out
/// too; the sentence names the file's place as every refusal does.
fn hidden(m: &Model) -> View {
    let summary = format!(
        "ySheets — {} · not read for an agent · {}",
        m.title(),
        if m.modified() { "unsaved changes" } else { "saved" }
    );
    View::new(summary)
        .with("sheet", m.title())
        .with("path", None::<String>)
        .with("rows", json!(null))
        .with("cols", json!(null))
        .with("used", json!(null))
        .with("selection", json!(null))
        .with("dirty", m.modified())
        .with("can_undo", json!(null))
        .with("can_redo", json!(null))
        .with("filled_cells", json!(null))
        .with("error_cells", json!(null))
        .with("first_error", None::<String>)
        .with("status", "")
        .with("notice", "")
        .with("hidden", Some(crate::agent_rule::HIDDEN_REFUSAL.to_string()))
        .with("functions", NAMES.to_vec())
}
