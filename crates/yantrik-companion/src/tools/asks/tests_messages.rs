//! `whatsapp_send` and `telegram_send` ask before the message leaves the machine (#1311).

use yantrik_companion_tools::asks_scenarios::{check_all, Scenario};

use super::messages;
use super::tests_support::q;

#[test]
fn a_whatsapp_message_asks_with_who_gets_it_and_every_word() {
    check_all(&Scenario {
        action: "send_chat_message",
        grade: "sensitive",
        card: serde_json::json!({ "tool": "whatsapp_send", "to": q("WhatsApp +15550100"), "text": q("Running late; call: me") }),
        call: &|variant, effect| {
            let text = match variant {
                0 => "Running late; call: me".to_string(),
                1 => "Never mind".to_string(),
                _ => "w".repeat(300),
            };
            messages::send("whatsapp_send", "WhatsApp +15550100", &text, effect)
        },
    });
}

#[test]
fn a_telegram_message_asks_with_the_chat_and_every_word() {
    check_all(&Scenario {
        action: "send_chat_message",
        grade: "sensitive",
        card: serde_json::json!({ "tool": "telegram_send", "to": q("Telegram chat 4242"), "text": q("On my way") }),
        call: &|variant, effect| {
            let text = match variant {
                0 => "On my way".to_string(),
                1 => "Stuck in traffic".to_string(),
                _ => "t".repeat(300),
            };
            messages::send("telegram_send", "Telegram chat 4242", &text, effect)
        },
    });
}
