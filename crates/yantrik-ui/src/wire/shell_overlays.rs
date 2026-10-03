//! The bar's panels (Today, Quick Settings, the power menu, the clipboard, the cheat sheet) opening.
//!
//! They open from the bar, from keybinds and from the control surface, and the one place all
//! three arrive is the `changed` hook in app.slint that fires `shell-overlay-opened`. Two things
//! happen when it does: the shell comes in front of whatever app window is over it, and the
//! clipboard loads its entries.
//!
//! Closing does nothing here on purpose: it does not lower the shell. The person
//! was looking at the shell when they dismissed the panel, and sending it behind their app would
//! take the screen away with the panel.

use slint::ComponentHandle;

use crate::app_context::AppContext;
use crate::App;

pub fn wire(ui: &App, ctx: &AppContext) {
    let clip = ctx.clip_history.clone();
    let weak = ui.as_weak();
    ui.on_shell_overlay_opened(move |kind| {
        // The panels are drawn by the shell's own window, which sits behind an app window that
        // is in front of it: from Notes the bar's power button "worked" and showed nothing,
        // like the launcher before it (#219). A Wayland client cannot raise itself, so it asks;
        // when the compositor will not, say so where a panel that never appeared can be traced.
        if let Err(why) = crate::windows::raise_shell() {
            tracing::warn!(%kind, %why, "A panel opened, but the shell could not be brought in front of the window over it");
        }
        if kind == "clipboard" {
            if let Some(ui) = weak.upgrade() {
                super::clipboard::refresh_on_open(&ui, &clip);
            }
        }
        if kind == "today" {
            if let Some(ui) = weak.upgrade() {
                super::today::refresh_on_open(&ui);
            }
        }
        if kind == "cheat-sheet" {
            if let Some(ui) = weak.upgrade() {
                super::cheat_sheet::refresh_on_open(&ui);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    /// The panels open behind an app window unless something brings the shell forward, and the
    /// clipboard shows last time's search and entries unless something refreshes it. Both are
    /// this one handler's job, for all three ways of opening.
    #[test]
    fn opening_a_panel_brings_the_shell_forward_and_refreshes_the_clipboard() {
        let source = include_str!("shell_overlays.rs");
        // Split, so this test's own text is not what the search finds.
        let start = source.find(concat!("ui.on_shell_overlay_", "opened(")).expect("the opened handler");
        let end = start + source[start..].find("\n    });").expect("its end");
        let handler = &source[start..end];
        assert!(
            handler.contains(concat!("crate::windows::raise_", "shell()")),
            "a panel opening must bring the shell's window forward. Handler as written:\n{handler}"
        );
        assert!(
            handler.contains("refresh_on_open"),
            "the clipboard must refresh its entries when it opens. Handler as written:\n{handler}"
        );
    }

    /// The clipboard used to find out it had been opened by polling the flag every 200 ms.
    #[test]
    fn the_clipboard_no_longer_polls_for_being_opened() {
        let source = include_str!("clipboard.rs");
        let code = source.split("#[cfg(test)]").next().unwrap();
        assert!(!code.contains("Timer"), "a timer watching `clip-panel-open` wakes the shell for nothing; the open hook does this");
    }
}
