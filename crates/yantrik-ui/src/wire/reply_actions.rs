//! What a press inside a reply's words does: a link in a paragraph opens, a code block's Copy
//! copies. Both go through the shell's one path for the thing — `open_url::open`, which refuses
//! anything but an http(s) address, and `wl-copy`, the clipboard the Lens already uses, run off
//! the UI thread (`copy_text`) — so a reply cannot open or copy by any way a person's own press
//! could not.

use super::copy_text::copy_async;
use crate::{App, ReplyActions};
use slint::ComponentHandle;

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
    // The result goes back to the UI thread, which the code block's button reads: `copied` names
    // the text that was copied, `failed` the text that could not be.
    let weak = ui.as_weak();
    actions.on_copy_text(move |text| {
        let (weak, shown) = (weak.clone(), text.clone());
        copy_async(text.to_string(), move |result| {
            if let Err(e) = &result {
                tracing::warn!(error = %e, "a code block was not copied");
            }
            let ok = result.is_ok();
            let _ = weak.upgrade_in_event_loop(move |ui| {
                let g = ui.global::<ReplyActions>();
                if ok {
                    g.set_copied(shown)
                } else {
                    g.set_failed(shown)
                }
            });
        });
    });
}
