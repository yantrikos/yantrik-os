//! The keyboard cheat sheet's rows: shown when it opens, narrowed as the person types.
//!
//! The rows are `cheat_sheet::shipped()`, which reads them out of rc.xml. Opening is not wired
//! here: Super+/, the control surface and a click all set `cheat-sheet-open`, and the one hook
//! they reach is `shell-overlay-opened` (see `shell_overlays`), which calls [`refresh_on_open`].

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::cheat_sheet::{matching, shipped, Row};
use crate::{App, CheatRowData};

pub fn wire(ui: &App) {
    let problems = crate::cheat_sheet::shipped_problems();
    if !problems.is_empty() {
        tracing::error!(?problems, "rc.xml has bindings the cheat sheet cannot describe");
    }
    let weak = ui.as_weak();
    ui.on_cheat_sheet_search(move |query| {
        if let Some(ui) = weak.upgrade() {
            show(&ui, &query);
        }
    });
}

/// A fresh open: the search is empty and every binding is listed.
pub(super) fn refresh_on_open(ui: &App) {
    ui.set_cheat_sheet_query("".into());
    show(ui, "");
}

fn show(ui: &App, query: &str) {
    let all = shipped();
    ui.set_cheat_sheet_rows(ModelRc::new(VecModel::from(model_rows(&matching(&all, query)))));
}

/// Section headings between the rows, in the order the rows arrive (they are already grouped).
fn model_rows(rows: &[&Row]) -> Vec<CheatRowData> {
    let mut out = Vec::new();
    let mut group = "";
    for row in rows {
        if row.group != group {
            group = row.group;
            out.push(CheatRowData { heading: true, text: group.into(), caps: ModelRc::default() });
        }
        let caps: Vec<SharedString> = row.caps.iter().map(|c| c.as_str().into()).collect();
        out.push(CheatRowData { heading: false, text: row.text.as_str().into(), caps: ModelRc::new(VecModel::from(caps)) });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_section_gets_one_heading_and_no_empty_section() {
        let all = shipped();
        let rows = model_rows(&matching(&all, ""));
        let headings: Vec<String> = rows.iter().filter(|r| r.heading).map(|r| r.text.to_string()).collect();
        assert_eq!(headings, crate::cheat_sheet::GROUPS, "every section once, in the sheet's order");
        assert_eq!(rows.iter().filter(|r| !r.heading).count(), all.len(), "one row per binding");
        // A search that leaves one section draws only that section's heading.
        let snapped = model_rows(&matching(&all, "third"));
        assert_eq!(snapped.iter().filter(|r| r.heading).count(), 1);
        assert!(model_rows(&matching(&all, "zzzz")).is_empty());
    }
}
