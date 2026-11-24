//! Retry, under the Lens's stopped line (`lens_stopped`): the person's last message, sent again to
//! the same mind through the one path the composer uses (the App's `send-message`). The press
//! comes up from the line (`TimelineLine.retry`) through `ChatMessage` and the Lens to the App's
//! `lens-retry`, which is wired here.

use slint::{ComponentHandle, Model, VecModel};

use crate::{App, MessageData};

/// Wire the Lens's Retry.
pub fn wire(ui: &App) {
    let weak = ui.as_weak();
    ui.on_lens_retry(move || {
        if let Some(ui) = weak.upgrade() {
            resend(&ui);
        }
    });
}

/// Send the person's last message again.
fn resend(ui: &App) {
    let messages = ui.get_messages();
    let asked = messages.as_any().downcast_ref::<VecModel<MessageData>>().and_then(|model| {
        let rows: Vec<(String, String)> = model.iter().map(|m| (m.role.to_string(), m.content.to_string())).collect();
        last_asked(&rows)
    });
    if let Some(text) = asked {
        ui.invoke_send_message(text.into());
    }
}

/// The newest message the person sent, from the conversation's (role, text) rows.
pub fn last_asked(rows: &[(String, String)]) -> Option<String> {
    rows.iter().rev().find(|(role, text)| role == "user" && !text.trim().is_empty()).map(|(_, text)| text.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_sends_the_last_thing_the_person_said() {
        let rows = [
            ("user".to_string(), "open a page".to_string()),
            ("assistant".to_string(), String::new()),
            ("user".to_string(), "report its heading".to_string()),
            ("assistant".to_string(), String::new()),
        ];
        assert_eq!(last_asked(&rows).as_deref(), Some("report its heading"));
        assert_eq!(last_asked(&rows[1..2]), None);
    }
}
