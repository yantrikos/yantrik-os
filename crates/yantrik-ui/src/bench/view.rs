//! The model as the screen draws it.

use slint::{ComponentHandle, SharedString};
use yantrik_bench::items::Kind;

use super::Bench;
use crate::apps::DesktopEntry;
use crate::{App, BenchItem, BenchState, YMenuAction};

/// Every icon, as `BenchItem`, with its picture (if one is made) and, for a shortcut, the tile of
/// the app it opens. Asks the picture worker for any picture that is not made yet.
pub fn items(b: &mut Bench, installed: &[DesktopEntry]) -> Vec<BenchItem> {
    let dir = b.model.desktop.clone();
    let mut out = Vec::with_capacity(b.model.placed.len());
    // Disjoint borrows: the listing is read while the pictures and shortcuts caches are written.
    let Bench { model, pictures, shortcuts, .. } = b;
    let by_name: std::collections::HashMap<&str, &yantrik_bench::items::Item> =
        model.listing.iter().map(|i| (i.name.as_str(), i)).collect();
    for p in &model.placed {
        let Some(item) = by_name.get(p.name.as_str()).copied() else { continue };
        let mut row = BenchItem {
            id: item.name.clone().into(),
            label: item.label().into(),
            kind: item.kind.word().into(),
            x: p.x as f32,
            y: p.y as f32,
            selected: model.selection.contains(&item.name),
            renaming: model.renaming.as_deref() == Some(item.name.as_str()),
            shortcut: false,
            app_id: SharedString::default(),
            has_icon: false,
            icon: Default::default(),
            has_thumb: false,
            thumb: Default::default(),
        };
        if item.kind == Kind::App {
            if let Some(sc) = shortcuts.get(&dir, item) {
                row.shortcut = true;
                row.label = yantrik_bench::items::display_name(&sc.name).into();
                match super::shortcuts::resolve(&sc, installed) {
                    Some(target) => {
                        row.app_id = target.icon_id.into();
                        if let Some(icon) = crate::icons::resolve(&target.theme_icon) {
                            row.has_icon = true;
                            row.icon = icon;
                        }
                    }
                    // An app this machine does not have: its own initial on the neutral tile.
                    None => row.app_id = sc.stem.into(),
                }
            }
        } else if let Some(img) = pictures.get(&dir, item) {
            row.has_thumb = true;
            row.thumb = img;
        }
        out.push(row);
    }
    out
}

/// Put the model on the screen: the icons (updated in place, so a drag in progress is not
/// dropped when a picture arrives), and the few flags beside them.
pub fn publish(ui: &App, b: &mut Bench) {
    let installed = b.catalogue.get();
    let rows = items(b, &installed);
    let state = ui.global::<BenchState>();
    if let Some(model) = crate::models::update(state.get_items(), rows, |r| r.id.clone()) {
        state.set_items(model);
    }
    state.set_show_icons(b.model.show_icons());
    state.set_snap(b.model.store.snap);
    let problem = b.model.problem.clone().unwrap_or_default();
    if state.get_problem().as_str() != problem {
        state.set_problem(problem.into());
    }
    publish_continue(&state, b);
}

/// The Continue row: the last thing opened, while it can still be opened.
pub fn publish_continue(state: &BenchState<'_>, b: &Bench) {
    let shown = b.last_opened.as_ref().filter(|r| r.is_open_able());
    let (title, sub) = shown.map(|r| (r.title.clone(), r.subtitle.clone())).unwrap_or_default();
    if state.get_has_continue() != shown.is_some() {
        state.set_has_continue(shown.is_some());
    }
    if state.get_continue_title().as_str() != title {
        state.set_continue_title(title.into());
    }
    if state.get_continue_subtitle().as_str() != sub {
        state.set_continue_subtitle(sub.into());
    }
}

/// The rows of a menu as the kit's menu draws them.
pub fn menu_rows(rows: &[super::menu::Row]) -> Vec<YMenuAction> {
    rows.iter()
        .map(|r| YMenuAction {
            id: r.id.clone().into(),
            icon: SharedString::default(),
            label: r.label.clone().into(),
            shortcut: r.hint.clone().into(),
            disabled: r.id == "none",
            separator_after: r.separator_after,
            is_danger: false,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use yantrik_bench::layout::METRICS;

    /// The Slint side draws a cell as `cell-w` by `cell-h` and the drop hint rounds to `pad-x`/`pad-y`;
    /// the layout code places icons by `METRICS`. The two must be one set of numbers, and the
    /// preview (tests/ui-preview) draws with the Slint defaults, so those are what is checked.
    #[test]
    fn the_slint_grid_defaults_are_the_layouts_metrics() {
        let slint = include_str!("../../../yantrik-ui-slint/ui/components/bench_state.slint");
        let default = |name: &str| -> i32 {
            let line = slint.lines().find(|l| l.contains(&format!("property <length> {name}:"))).unwrap_or_else(|| panic!("{name}"));
            line.split(':').nth(1).unwrap().trim().trim_end_matches(';').trim_end_matches("px").trim().parse().unwrap()
        };
        assert_eq!(default("cell-w"), METRICS.cell_w);
        assert_eq!(default("cell-h"), METRICS.cell_h);
        assert_eq!(default("pad-x"), METRICS.pad_x);
        assert_eq!(default("pad-y"), METRICS.pad_y);
        // The wiring hands the tile size over by hand; the default is what the preview draws.
        assert_eq!(default("tile"), 56);
        assert!(include_str!("wiring.rs").contains("state.set_tile(56.0)"));
    }

    /// A menu row for something that cannot be done is left out, not drawn grey; the one row that
    /// is disabled is the note that nothing opens a file.
    #[test]
    fn only_the_note_that_nothing_opens_is_disabled() {
        let rows = super::menu_rows(&crate::bench::menu::open_with(&[]));
        let disabled: Vec<_> = rows.iter().filter(|r| r.disabled).map(|r| r.id.to_string()).collect();
        assert_eq!(disabled, ["none"]);
    }
}
