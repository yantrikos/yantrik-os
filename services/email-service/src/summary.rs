//! The two facts about a message that a list row does not show and a caller that acts on the
//! message needs: who it came from as an address, and its `Message-ID`.
//!
//! The row shows the sender's name, or the address when there is no name. A caller that writes
//! back to the sender, or tells one sender from another, cannot work from a name. No sockets and
//! no state, so the pieces can be tested as they stand.

/// The sender's `mailbox@host`, or empty when the envelope gave neither part.
pub fn address(mailbox: Option<&[u8]>, host: Option<&[u8]>) -> String {
    let part = |raw: Option<&[u8]>| raw.map(|b| String::from_utf8_lossy(b).trim().to_string()).unwrap_or_default();
    let (mailbox, host) = (part(mailbox), part(host));
    if mailbox.is_empty() || host.is_empty() {
        return String::new();
    }
    format!("{mailbox}@{host}")
}

/// A `Message-ID` header value as the envelope carries it (angle brackets and all), trimmed.
pub fn message_id(raw: Option<&[u8]>) -> String {
    raw.map(|b| String::from_utf8_lossy(b).trim().to_string()).unwrap_or_default()
}

/// The messages a `mark_read` call names: `message_ids` (many, so marking a folder read is one
/// mailbox session and not hundreds), else the one `message_id`. Every id must be a UID.
pub fn uids_to_mark(params: &serde_json::Value) -> Result<Vec<u32>, yantrik_ipc_contracts::email::ServiceError> {
    use yantrik_ipc_contracts::email::ServiceError;
    let bad = || ServiceError { code: -32602, message: "Invalid message ID".to_string() };
    let uids: Vec<u32> = match params["message_ids"].as_array() {
        Some(many) => many
            .iter()
            .map(|v| v.as_str().and_then(|s| s.parse().ok()).ok_or_else(bad))
            .collect::<Result<_, _>>()?,
        None => vec![params["message_id"].as_str().and_then(|s| s.parse().ok()).ok_or_else(bad)?],
    };
    if uids.is_empty() {
        return Err(bad());
    }
    Ok(uids)
}

/// An IMAP UID set: `4,7,9`.
pub fn uid_set(uids: &[u32]) -> String {
    uids.iter().map(u32::to_string).collect::<Vec<_>>().join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_needs_both_halves() {
        assert_eq!(address(Some(b"ada"), Some(b"example.com")), "ada@example.com");
        assert_eq!(address(Some(b"ada"), None), "");
        assert_eq!(address(None, Some(b"example.com")), "");
        assert_eq!(address(Some(b" "), Some(b"example.com")), "");
    }

    #[test]
    fn one_id_or_many_name_the_messages_to_mark() {
        assert_eq!(uids_to_mark(&serde_json::json!({"message_id": "12"})).unwrap(), vec![12]);
        let many = serde_json::json!({"message_ids": ["4", "7"], "message_id": "99"});
        assert_eq!(uids_to_mark(&many).unwrap(), vec![4, 7]);
        assert_eq!(uid_set(&[4, 7, 9]), "4,7,9");
    }

    #[test]
    fn an_id_that_is_not_a_uid_marks_nothing() {
        assert!(uids_to_mark(&serde_json::json!({"message_id": "x"})).is_err());
        assert!(uids_to_mark(&serde_json::json!({"message_ids": ["1", "y"]})).is_err());
        assert!(uids_to_mark(&serde_json::json!({"message_ids": []})).is_err());
        assert!(uids_to_mark(&serde_json::json!({})).is_err());
    }

    #[test]
    fn a_message_id_is_kept_as_sent() {
        assert_eq!(message_id(Some(b" <a.1@example.com> ")), "<a.1@example.com>");
        assert_eq!(message_id(None), "");
    }
}
