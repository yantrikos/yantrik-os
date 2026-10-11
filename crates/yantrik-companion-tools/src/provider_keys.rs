//! The keys the OS uses to call AI providers (the free pool's), kept in the vault and never the
//! companion's.
//!
//! They are stored, read and removed here, by the shell, on a path no model turn is on: the
//! setup card takes a key from the clipboard, checks it with its provider, and stores it. The
//! companion's own vault tools pass over every entry here (`is_os_entry`), so no model can read
//! one, list one, delete one, or write one. Writing matters as much as reading: a page that got a
//! model to overwrite the Groq key with an account of its own would have had the person's private
//! turns run under that account, where its owner can read them.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};

use rusqlite::Connection;

// ── The clipboard, while a key is on its way ──
//
// The person copies a key on a provider's page and presses Paste on the card. Between the two
// the key is on the clipboard, and every tool a model can call that reads or writes the
// clipboard is refused: a turn a web page steered could otherwise read the key, or put a key of
// its own there for the person to paste (security review of #544, 1 Oct 2026). Same process as
// the shell, so one flag serves both.
static CLIPBOARD_HELD: AtomicBool = AtomicBool::new(false);

/// Hold, or release, the clipboard for a key on its way (the shell's free AI card).
pub fn hold_clipboard(on: bool) {
    CLIPBOARD_HELD.store(on, Ordering::SeqCst);
}

/// Whether a key is on its way.
pub fn clipboard_held() -> bool {
    CLIPBOARD_HELD.load(Ordering::SeqCst)
}

/// The refusal a clipboard tool answers with while a key is on its way, or `None`.
pub fn clipboard_refusal() -> Option<String> {
    clipboard_held().then(|| {
        "Refused: the person is copying an API key into Settings right now, and the clipboard is not read or written until they are done.".to_string()
    })
}

/// The clipboard's text, bounded and with a deadline (`clipboard::read_text_bounded`).
pub use crate::clipboard::read_text_bounded as read_clipboard;

/// Whether text looks like a provider's API key (the shapes the free AI card accepts).
pub fn looks_like_a_key(text: &str) -> bool {
    yantrik_ml::provider::pool::signup::looks_like_a_provider_key(text)
}

/// The vault category of the OS's own provider keys.
pub const CATEGORY: &str = "os:provider-key";

/// The vault service name a provider's value is kept under (`groq`, `cloudflare_account`, …).
pub fn service(id: &str) -> String {
    format!("{CATEGORY}:{id}")
}

/// Whether a vault entry is the OS's own, and none of the companion's tools' business.
pub fn is_os_entry(service: &str, category: &str) -> bool {
    category.starts_with("os:") || service.starts_with("os:")
}

/// Why a key could not be kept or read.
#[derive(Debug, PartialEq, Eq)]
pub enum Failure {
    /// The vault has a passphrase and is shut: the person unlocks it first.
    Locked,
    /// The engine refused; its text is not shown (it can name the store).
    Store,
}

fn open(conn: &Connection) -> Result<yantrikdb_core::encryption::EncryptionProvider, Failure> {
    // Idempotent; the companion makes them at start, and this does not depend on that order.
    yantrikdb_core::vault::init_tables(conn);
    if yantrikdb_core::vault::is_protected(conn) && !yantrikdb_core::vault::is_unlocked() {
        return Err(Failure::Locked);
    }
    yantrikdb_core::vault::vault_encryption(conn).map_err(|_| Failure::Store)
}

/// Keep `value` for provider value `id`, replacing what was there.
pub fn store(conn: &Connection, id: &str, value: &str) -> Result<(), Failure> {
    let enc = open(conn)?;
    yantrikdb_core::vault::store(conn, &enc, &service(id), "yantrik-os", value, None, None, Some(CATEGORY))
        .map(|_| ())
        .map_err(|_| Failure::Store)
}

/// Every provider value kept, by id: what the pool calls with. Selected by the category alone,
/// with no limit: a search on the name (LIMIT 20) let twenty look-alike entries a model made
/// push the real ones out of the answer (security review of #544).
pub fn load_all(conn: &Connection) -> Result<BTreeMap<String, String>, Failure> {
    let enc = open(conn)?;
    let prefix = format!("{CATEGORY}:");
    let rows: Vec<(String, String)> = {
        let mut stmt = conn
            .prepare("SELECT service, password_enc FROM vault_entries WHERE category = ?1")
            .map_err(|_| Failure::Store)?;
        let mapped = stmt.query_map([CATEGORY], |r| Ok((r.get(0)?, r.get(1)?))).map_err(|_| Failure::Store)?;
        mapped.collect::<Result<_, _>>().map_err(|_| Failure::Store)?
    };
    let mut out = BTreeMap::new();
    for (service, sealed) in rows {
        let Some(id) = service.strip_prefix(&prefix) else { continue };
        let value = enc.decrypt_string(&sealed).map_err(|_| Failure::Store)?;
        out.insert(id.to_string(), value);
    }
    Ok(out)
}

/// Which provider values are kept, without opening any: for a card that says "ready".
pub fn kept(conn: &Connection) -> Vec<String> {
    let prefix = format!("{CATEGORY}:");
    yantrikdb_core::vault::list(conn)
        .unwrap_or_default()
        .into_iter()
        .filter(|e| e.category == CATEGORY)
        .filter_map(|e| e.service.strip_prefix(&prefix).map(String::from))
        .collect()
}

/// Forget provider value `id`. Removing needs no key, but a shut vault is not changed by
/// something that cannot open it.
pub fn remove(conn: &Connection, id: &str) -> Result<bool, Failure> {
    open(conn)?;
    yantrikdb_core::vault::delete_by_service(conn, &service(id)).map(|n| n > 0).map_err(|_| Failure::Store)
}

/// The last four characters, the most a card shows of a key.
pub fn tail(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    chars[chars.len().saturating_sub(4)..].iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_oss_entries_are_told_apart_from_the_persons() {
        assert!(is_os_entry(&service("groq"), CATEGORY));
        assert!(is_os_entry("os:provider-key:groq", "general"), "by its name, whatever category a caller gave it");
        assert!(is_os_entry("github.com", "os:anything"));
        assert!(!is_os_entry("github.com", "general"));
        assert_eq!(tail("gsk_abcdef7f3k"), "7f3k");
        assert_eq!(tail("ab"), "ab");
    }

    /// While a key is on its way the model's clipboard tools are refused, and a key-shaped
    /// clipboard is never handed to one (security review of #544).
    #[test]
    fn the_clipboard_is_out_of_reach_while_a_key_is_on_its_way() {
        hold_clipboard(true);
        let refused = clipboard_refusal().expect("a refusal while held");
        assert!(refused.starts_with("Refused"), "{refused}");
        hold_clipboard(false);
        assert!(clipboard_refusal().is_none());
        assert!(looks_like_a_key(&format!("gsk_{}", "Zz9y".repeat(13))));
        assert!(!looks_like_a_key("an ordinary line of text"));
    }
}
