//! Window switcher — focus a window by title via wlrctl.

use crate::app_context::AppContext;
use crate::App;

pub fn wire(ui: &App, _ctx: &AppContext) {
    ui.on_switch_window(move |title| {
        let title_str = title.to_string();
        tracing::info!(title = %title_str, "Switching to window");
        let _ = std::process::Command::new("wlrctl")
            .args(["toplevel", "focus", &title_str])
            .spawn();
    });
}
