//! The card for a chat message leaving the machine (#1311): `whatsapp_send` and `telegram_send`
//! ask `send_message` ("cannot be undone") right before the send, with who it goes to and the
//! whole text, quoted. A text the card cannot show whole is not sent.

use yantrik_companion_tools::asks::{act, Card, Question};

/// Ask, then run `send`. `tool` names the channel's tool; `to` says who receives it.
pub fn send(tool: &str, to: &str, text: &str, send: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    let card = Card::new(tool).text("to", to).text("text", text);
    act(Question::SendMessage, &card, send)
}
