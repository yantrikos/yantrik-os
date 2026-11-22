//! A secret value is written in the record as its length and a hash, never its text (#1353 round 5,
//! item 5; round 7, item 2).
//!
//! One function, [`masked`], for the two places the shell writes arguments down after the fact: the
//! audit's line and its side file (`mind_mode::record_checked`) and the mode menu's rows, which are
//! the line's own. **Never the approval card**: the person sees what they are asked about, whatever
//! the caller says about it (`approval_whole::rows` does not call this).
//!
//! What is secret in the record: an argument named like one (a password, a token, a key), and the
//! `text` or `value` of a call that says `secret: true`. The browser's driver refuses `secret: true`
//! unless the page itself marks the focused field a password field, before typing anything, so a
//! `secret: true` call that ran is one the browser confirmed. The hash is of the value alone, so it
//! answers "was it that one?" and no more. The grant and the report's match are bound to the
//! arguments as sent, never to this reading.

use serde_json::Value;

/// Argument names that hold a secret on any action, compared in lower case.
const SECRET_KEYS: [&str; 8] = ["password", "passphrase", "secret", "token", "api_key", "apikey", "authorization", "bearer"];
/// The arguments a call's own `secret: true` covers.
const MARKED_KEYS: [&str; 2] = ["text", "value"];

fn secret_in(args: &serde_json::Map<String, Value>, key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    SECRET_KEYS.contains(&lower.as_str()) || (args.get("secret") == Some(&Value::Bool(true)) && MARKED_KEYS.contains(&lower.as_str()))
}

/// What a secret value is written as: how long it is, and its SHA-256 (twelve hex digits are enough
/// to tell two apart; the whole hash is in the record's own `args_full` fields).
pub(crate) fn hidden(value: &Value) -> String {
    let text = match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let sha = crate::audit_store::sha256(&text);
    format!("(secret: {} characters, sha256 {}\u{2026})", text.chars().count(), &sha[..12])
}

/// `args` with each secret value replaced by [`hidden`]. Anything that is not an object passes as it is.
pub(crate) fn masked(args: &Value) -> Value {
    let Some(map) = args.as_object() else { return args.clone() };
    Value::Object(map.iter().map(|(k, v)| (k.clone(), if secret_in(map, k) { Value::String(hidden(v)) } else { v.clone() })).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_named_or_marked_value_is_its_length_and_a_hash_and_nothing_else_moves() {
        let typed = json!({ "ref": "e12", "text": "hunter2-correct", "secret": true, "clear": true });
        let out = masked(&typed);
        assert_eq!(out["ref"], "e12");
        assert_eq!(out["clear"], true);
        let shown = out["text"].as_str().unwrap();
        assert!(shown.contains("15 characters") && shown.contains(&crate::audit_store::sha256("hunter2-correct")[..12]), "{shown}");
        assert!(!out.to_string().contains("hunter2"));
        // Unmarked text is the person's to read on the card: it is not hidden.
        assert_eq!(masked(&json!({ "text": "hello" })), json!({ "text": "hello" }));
        // A name that says what it is.
        assert!(!masked(&json!({ "Password": "p", "command": "ls" })).to_string().contains("\"p\""));
        assert_eq!(masked(&json!("not an object")), json!("not an object"));
    }

    /// The record and the menu read this one function; the card never masks, even for a caller that
    /// says `secret: true` or names an argument like a secret.
    #[test]
    fn the_record_and_the_menu_agree_and_the_card_always_shows_the_text() {
        let args = json!({ "ref": "e1", "text": "s3cret-pass", "secret": true });
        let card = crate::approval_whole::rows("browser", "type", "standard", "Type into a field", &args, false, false).join(" ");
        assert!(card.contains("s3cret-pass"), "the person sees what they are asked about: {card}");
        let named = crate::approval_whole::rows("some-app", "set", "standard", "", &json!({ "password": "named-pass" }), false, false).join(" ");
        assert!(named.contains("named-pass"), "a name does not hide it on the card: {named}");
        let (entry, written) = crate::mind_mode::record_checked("bypass_all", "test", &Default::default(), "secret-args-test", "type", &args, "standard", "ran");
        written.unwrap();
        let menu = entry.args.join(" ");
        let file = std::fs::read_to_string(crate::mind_mode::audit_path()).unwrap();
        let line = file.lines().rev().find(|l| l.contains("secret-args-test")).expect("the record has the line");
        let tag = hidden(&json!("s3cret-pass"));
        for (place, text) in [("menu", menu.as_str()), ("record", line)] {
            assert!(!text.contains("s3cret-pass"), "{place} shows the text: {text}");
        }
        assert!(menu.contains(&tag), "{menu}");
        let full: Value = serde_json::from_str(line).unwrap();
        assert!(full["args_full"]["text"].as_str().unwrap().contains(&tag.replace('\u{2026}', "\\u2026")) || full["args_full"]["text"].as_str().unwrap().contains(&tag), "{line}");
    }
}
