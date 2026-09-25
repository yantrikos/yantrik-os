//! Window switcher — focus a window by title via wlrctl — and the taskbar entry's own menu.

use slint::{ComponentHandle, ModelRc, VecModel};

use crate::app_context::AppContext;
use crate::wire::{dock, pins};
use crate::{App, YMenuAction};

/// Focus the window with this title, restoring it first if it was minimized.
///
/// # The bug this carries the fix for
///
/// This used to run `wlrctl toplevel focus <title>`, and wlrctl's matchspec says a match with no
/// key "is assumed to be an app_id". Our Slint windows set a title and no wayland app_id at all,
/// so every click on a taskbar entry asked the compositor to focus an app_id that did not exist,
/// matched nothing, and returned success. Clicking the taskbar did nothing, quietly, for every
/// window — the result was discarded with `let _` so nothing was even logged.
///
/// The key is now given explicitly. `wlrctl` is also waited on rather than spawned and forgotten,
/// so a failure can be reported instead of vanishing.
pub fn wire(ui: &App, ctx: &AppContext) {
    ui.on_switch_window(move |title| {
        let title = title.to_string();
        tracing::info!(title = %title, "Switching to window");

        // wlrctl exits non-zero when nothing matched, which is the interesting case: the taskbar
        // is showing a window the compositor does not have under that name.
        if !crate::windows::present(&title) {
            tracing::warn!(
                title = %title,
                "no window matched that title; the taskbar and the compositor disagree"
            );
        }
    });

    // The entry's right-click menu (#232). The rows are built here rather than in the .slint
    // because one of them — the pin — depends on state only Rust can read, and because every id
    // is a `shell` control action's name, which is what keeps the menu from being a second path.
    {
        let weak = ui.as_weak();
        let catalogue = ctx.installed_apps.clone();
        ui.on_window_menu(move |title, app_id, x, y| {
            let Some(ui) = weak.upgrade() else { return };
            let installed = catalogue.get();
            let id = app_id.as_str();
            let pinned = if !id.is_empty() && pins::is_pinnable(id) {
                let is = pins::is_pinned(id);
                // Offered only where a pin could mean something: one already there, so it can
                // be taken off, or an app that can actually launch — a pin for something that
                // cannot open is a START tile that does nothing, which `publish` filters out
                // and `pin_app` refuses for the same reason.
                if is || dock::is_launchable(&pins::pin_id(id), &installed) {
                    Some(is)
                } else {
                    None
                }
            } else {
                None
            };
            ui.set_taskbar_menu_actions(ModelRc::new(VecModel::from(menu_rows(pinned))));
            ui.set_taskbar_menu_title(title.clone());
            ui.set_taskbar_menu_app_id(app_id.clone());
            ui.set_taskbar_menu_x(x);
            ui.set_taskbar_menu_y(y);
            ui.set_taskbar_menu_open(true);
        });
    }

    {
        let weak = ui.as_weak();
        let catalogue = ctx.installed_apps.clone();
        ui.on_window_menu_action(move |id, title, app_id| {
            let Some(ui) = weak.upgrade() else { return };
            // The same resolution the control surface's own window verbs run. The title the
            // taskbar showed came from a snapshot up to nine seconds old, so it is resolved
            // against what is open now exactly like anything a caller types — and `close` keeps
            // `window_to_close`'s refusals: no coin toss on an ambiguous name, never the shell.
            let result = match id.as_str() {
                "close_window" => crate::windows::window_to_close(&title, &crate::windows::addressable_titles())
                    .and_then(|t| crate::windows::close(&t)),
                "minimise_window" => crate::windows::window_named(&title, &crate::windows::addressable_titles())
                    .and_then(|t| crate::windows::minimise(&t)),
                "maximise_window" => crate::windows::window_named(&title, &crate::windows::addressable_titles())
                    .and_then(|t| crate::windows::maximise(&t)),
                "pin_app" => {
                    // The toggle, which is what the row's label promised when the menu was
                    // built, and the same function the `pin_app` action ends in.
                    pins::toggle(&app_id);
                    pins::publish(&ui, &catalogue.get());
                    Ok(())
                }
                other => Err(format!("the taskbar menu has no action `{other}`")),
            };
            if let Err(why) = result {
                // The window the menu was opened over is simply gone, usually: the poll had it,
                // the compositor no longer does.
                tracing::warn!(
                    action = %id, window = %title, reason = %why,
                    "the taskbar menu's action did not run"
                );
            }
        });
    }
}

/// The menu's rows. `pinned` is `None` when no pin row belongs on this menu at all; otherwise it
/// says which way the row reads.
///
/// Every id is the name of a `shell` control action — `close_window`, `minimise_window`,
/// `maximise_window`, `pin_app` — so what a pointer can choose and what a mind can ask for are
/// the same verbs, graded the same way and ending in the same functions.
fn menu_rows(pinned: Option<bool>) -> Vec<YMenuAction> {
    let row = |id: &str, label: &str| YMenuAction {
        id: id.into(),
        label: label.into(),
        ..Default::default()
    };
    let mut rows = vec![
        // The one verb here that can cost work, wearing the colour the kit gives it. Closing is
        // still a request — an app with unsaved work may answer with its own dialog and stay —
        // but it is the row that reads as dangerous, so it looks like it.
        YMenuAction { is_danger: true, ..row("close_window", "Close window") },
        // "Minimise", not "Minimise / Restore": restoring is clicking the entry itself, which
        // is what it has always been, and the list carries no state to toggle a label against.
        row("minimise_window", "Minimise"),
        // "Maximise", not "Maximise / Restore": wlrctl 0.2.2 has no unmaximize and `toplevel
        // list` reports no state, so a label that promised a toggle would be guessing.
        row("maximise_window", "Maximise"),
    ];
    if let Some(pinned) = pinned {
        // The divider exists to separate the pin from the window verbs, so it is only drawn
        // when a pin is — a separator with nothing under it is a line at the bottom of a menu.
        if let Some(last) = rows.last_mut() {
            last.separator_after = true;
        }
        rows.push(row("pin_app", if pinned { "Unpin from START" } else { "Pin to START" }));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The menu is a view of the control surface, not a second path beside it (#232): every
    /// row's id is an action `yos act shell` can run. An id nothing on the surface answers would
    /// be a verb only a pointer could use — the divergence the menu must not add.
    #[test]
    fn every_row_is_a_shell_action_name() {
        let known = ["close_window", "minimise_window", "maximise_window", "pin_app"];
        for pinned in [None, Some(false), Some(true)] {
            for row in menu_rows(pinned) {
                assert!(
                    known.contains(&row.id.as_str()),
                    "the taskbar menu offers `{}`, which is no `shell` action",
                    row.id
                );
            }
        }
    }

    /// What the rows say: close wears the danger colour, the pin reads the way it will act, and
    /// the divider is only there when a pin is below it.
    #[test]
    fn the_rows_say_what_they_do() {
        let rows = menu_rows(Some(false));
        let labels: Vec<&str> = rows.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(labels, ["Close window", "Minimise", "Maximise", "Pin to START"]);
        assert!(rows[0].is_danger, "Close is the row that can cost work; it wears the colour");
        assert!(
            rows[2].separator_after && !rows[3].separator_after,
            "the divider separates the pin from the window verbs, and only them"
        );

        assert_eq!(menu_rows(Some(true)).last().unwrap().label, "Unpin from START");

        // An app that can carry no pin gets the three window verbs and no dangling divider.
        let rows = menu_rows(None);
        assert_eq!(rows.len(), 3);
        assert!(!rows.iter().any(|r| r.separator_after));
    }
}
