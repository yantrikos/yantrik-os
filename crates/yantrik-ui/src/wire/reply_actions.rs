//! What a press inside a reply's words does: a link in a paragraph opens, a code block's Copy
//! copies. Both go through the shell's one path for the thing — `open_url::open`, which refuses
//! anything but an http(s) address, and `wl-copy`, the clipboard the Lens already uses — so a
//! reply cannot open or copy by any way a person's own press could not.

use crate::{App, ReplyActions};
use slint::ComponentHandle;

/// Put `text` on the clipboard, exactly as given.
pub fn copy_text(text: &str) -> Result<(), String> {
    use std::io::Write;
    let mut child = std::process::Command::new("wl-copy")
        .stdin(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not run wl-copy: {e}"))?;
    if let Some(stdin) = child.stdin.as_mut() {
        stdin
            .write_all(text.as_bytes())
            .map_err(|e| format!("wl-copy: {e}"))?;
    }
    child.wait().map_err(|e| format!("wl-copy: {e}"))?;
    Ok(())
}

pub fn wire(ui: &App) {
    let actions = ui.global::<ReplyActions>();
    actions.on_open_link(|url| {
        let url = url.to_string();
        let _ = std::thread::Builder::new()
            .name("reply-link".into())
            .spawn(move || {
                if let Err(e) = crate::open_url::open(&url) {
                    tracing::warn!(error = %e, "a link in a reply was not opened");
                }
            });
    });
    actions.on_copy_text(|text| {
        if let Err(e) = copy_text(&text) {
            tracing::warn!(error = %e, "a code block was not copied");
        }
    });
}
