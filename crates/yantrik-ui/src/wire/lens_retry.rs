//! Retry, under the Lens's stopped line (`lens_stopped`): the person's last message, sent again to
//! the same mind through the one path the composer uses (the App's `send-message`). There is no
//! second send path: the reply's Retry presses `open-run` with [`RUN`], which `show_agent`'s
//! callback hands here before it is taken for a run to open.

use slint::{ComponentHandle, Model, VecModel};

use crate::{App, MessageData};

/// What the reply's Retry presses.
pub const RUN: &str = "retry:last";

/// If `run` is the Retry press, send the person's last message again and say so.
pub fn take(ui: &App, run: &str) -> bool {
    if run != RUN {
        return false;
    }
    let messages = ui.get_messages();
    let asked = messages.as_any().downcast_ref::<VecModel<MessageData>>().and_then(|model| {
        let rows: Vec<(String, String)> = model.iter().map(|m| (m.role.to_string(), m.content.to_string())).collect();
        last_asked(&rows)
    });
    if let Some(text) = asked {
        ui.invoke_send_message(text.into());
    }
    true
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
