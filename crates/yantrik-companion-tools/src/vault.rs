//! Vault tools — secure credential storage with AES-256-GCM encryption.
//!
//! Tools: vault_store, vault_get, vault_list, vault_delete, vault_generate_password, vault_set_pin
//!
//! # A mind does not carry the passphrase
//!
//! Every one of these tools used to take a `pin` argument, with a description telling the model to
//! "ask the user for it". That was written when the PIN was a hash in a table that gated the tools
//! while the key sat beside it in the clear — so the PIN was a formality and relaying it cost
//! nothing that was not already lost.
//!
//! It is not a formality now. The PIN *is* the passphrase the vault's key is wrapped under, and a
//! model that asks for it puts it in a conversation: in the transcript, in the context window, on
//! the wire to whatever provider is answering, and in whatever that provider keeps. A companion
//! running over Telegram would have carried it across a third party's servers to unlock a vault on
//! a desk. Asking the person is right; asking them *through the model* is not.
//!
//! So the argument is gone, and with it the whole class of bug where a caller supplies the secret.
//! A locked vault answers [`LOCKED_ANSWER`] and [`request_unlock`] puts a prompt on the desktop,
//! which the person types into and the model never sees. What the model can do about a locked
//! vault is tell the person the desktop is asking — which is the true and only answer.
//!
//! Security model:
//!   - All credentials encrypted with AES-256-GCM at rest (vault-specific DEK)
//!   - The DEK is wrapped under a passphrase with Argon2id when the vault is protected; a vault
//!     that has never been protected keeps it in the file, and `vault_list` says so out loud
//!   - A protected vault that has not been unlocked in this process returns nothing at all

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};

/// What a mind is told when it asks the vault for something and the vault is shut.
///
/// A named constant because three places have to agree on it: the tools that return it, the shell
/// that recognises it, and the tests. Distinguishable from a generic failure on purpose — `Error:
/// the vault is locked; unlock it with its passphrase` was true, arrived in the same shape as
/// every transient error a model has learned to retry, and said nothing about the one thing that
/// resolves it, which is a person at the machine.
pub const LOCKED_ANSWER: &str =
    "VAULT_LOCKED: the vault is locked; the person has to unlock it. Do not ask for the \
     passphrase — you cannot carry it. Tell them the desktop is asking for it.";

/// Set when a mind wanted the vault and could not have it.
///
/// The shell polls this and draws the unlock prompt. A latch rather than a call, because this
/// crate is a leaf: the tools run on the companion's worker thread inside the shell's process, and
/// a direct call would mean this crate depending on the shell that depends on it.
static UNLOCK_WANTED: AtomicBool = AtomicBool::new(false);
/// Why, in the person's words, and whether answering it protects the vault for the first time.
///
/// The `bool` is carried rather than worked out by the shell, because the caller already knows it
/// — `ensure_open` only ever fires on a vault that *is* protected — and the shell would have to
/// infer it from a cached read that may not have landed yet. Getting it backwards would ask
/// someone to invent a new passphrase for a vault that already has one.
static UNLOCK_REASON: Mutex<Option<(String, bool)>> = Mutex::new(None);

/// Ask the desktop to put its unlock prompt on the screen.
///
/// `first_time` is true only when the vault has no passphrase yet, so the card asks the person to
/// choose one instead of asking for one they do not have.
pub fn request_unlock(reason: &str, first_time: bool) {
    if let Ok(mut guard) = UNLOCK_REASON.lock() {
        // First reason wins: a mind retrying three times must not rewrite what the person has
        // already started reading.
        if guard.is_none() {
            *guard = Some((reason.to_string(), first_time));
        }
    }
    UNLOCK_WANTED.store(true, Ordering::Relaxed);
}

/// Take the pending request, if there is one. Clears it.
pub fn take_unlock_request() -> Option<(String, bool)> {
    if !UNLOCK_WANTED.swap(false, Ordering::Relaxed) {
        return None;
    }
    UNLOCK_REASON
        .lock()
        .ok()
        .and_then(|mut g| g.take())
        .or_else(|| Some(("a mind asked for a credential".to_string(), false)))
}

/// Whether this tool call can read the vault, and what to say if it cannot.
///
/// One place, so the six tools cannot drift into six different answers. `Ok(())` covers both the
/// unlocked vault and the legacy unprotected one — the second is not safe, but refusing it would
/// lock people out of credentials they stored before any of this existed, and `vault_list` is
/// where that vault is told the truth about itself.
fn ensure_open(conn: &rusqlite::Connection, reason: &str) -> Result<(), String> {
    if yantrikdb_core::vault::is_protected(conn) && !yantrikdb_core::vault::is_unlocked() {
        // Never a first time: this branch is only reachable on a vault that already has a
        // passphrase, so the card asks for the one that exists.
        request_unlock(reason, false);
        return Err(LOCKED_ANSWER.to_string());
    }
    Ok(())
}

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(VaultStoreTool));
    reg.register(Box::new(VaultGetTool));
    reg.register(Box::new(VaultListTool));
    reg.register(Box::new(VaultDeleteTool));
    reg.register(Box::new(VaultGeneratePasswordTool));
    reg.register(Box::new(VaultSetPinTool));
}

// ── Vault Store ──

struct VaultStoreTool;

impl Tool for VaultStoreTool {
    fn name(&self) -> &'static str { "vault_store" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "vault" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "vault_store",
                "description": "Store credential securely in vault",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "service": {"type": "string", "description": "Service name (e.g., 'github.com', 'netflix', 'aws-prod')"},
                        "username": {"type": "string", "description": "Username, email, or account identifier"},
                        "password": {"type": "string", "description": "Password or API key to store"},
                        "url": {"type": "string", "description": "Optional URL for the service"},
                        "notes": {"type": "string", "description": "Optional notes (also encrypted)"},
                        "category": {"type": "string", "description": "Category: general, email, social, dev, finance, work"}
                    },
                    "required": ["service", "username", "password"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let service = args.get("service").and_then(|v| v.as_str()).unwrap_or_default();
        let username = args.get("username").and_then(|v| v.as_str()).unwrap_or_default();
        let password = args.get("password").and_then(|v| v.as_str()).unwrap_or_default();
        let url = args.get("url").and_then(|v| v.as_str());
        let notes = args.get("notes").and_then(|v| v.as_str());
        let category = args.get("category").and_then(|v| v.as_str());

        if service.is_empty() || username.is_empty() || password.is_empty() {
            return "Error: service, username, and password are required".to_string();
        }
        if crate::provider_keys::is_os_entry(service, category.unwrap_or("")) {
            return format!("Refused: `{service}` is a name the OS keeps for itself; store this under another.");
        }

        // Writing needs the key as much as reading does — a credential encrypted under a key this
        // process does not hold is a credential nobody can read back.
        if let Err(locked) = ensure_open(&ctx.db.conn(), "a mind wants to save a credential") {
            return locked;
        }

        let enc = match yantrikdb_core::vault::vault_encryption(&ctx.db.conn()) {
            Ok(e) => e,
            Err(e) => return format!("Error: {e}"),
        };

        match yantrikdb_core::vault::store(&ctx.db.conn(), &enc, service, username, password, url, notes, category) {
            Ok(_) => format!("Credential stored securely for '{service}'"),
            Err(e) => format!("Error storing credential: {e}"),
        }
    }
}

// ── Vault Get (PIN-protected) ──

struct VaultGetTool;

impl Tool for VaultGetTool {
    fn name(&self) -> &'static str { "vault_get" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Sensitive }
    fn category(&self) -> &'static str { "vault" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "vault_get",
                "description": "Retrieve credential from vault",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "service": {"type": "string", "description": "Exact service name to look up"},
                        "search": {"type": "string", "description": "Search by partial service name (alternative to exact match)"}
                    }
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let service = args.get("service").and_then(|v| v.as_str());
        let search = args.get("search").and_then(|v| v.as_str());

        // The case this whole design is for. A protected vault this process has not opened has no
        // key, so there is no answer to give — and the answer it gives instead names the one thing
        // that resolves it, and raises the prompt that lets the person do it.
        if let Err(locked) = ensure_open(&ctx.db.conn(), "a mind asked for a saved credential") {
            return locked;
        }

        let enc = match yantrikdb_core::vault::vault_encryption(&ctx.db.conn()) {
            Ok(e) => e,
            Err(e) => return format!("Error: {e}"),
        };

        let entries = if let Some(svc) = service {
            match yantrikdb_core::vault::get(&ctx.db.conn(), &enc, svc) {
                Ok(e) => e,
                Err(e) => return format!("Error: {e}"),
            }
        } else if let Some(q) = search {
            match yantrikdb_core::vault::search(&ctx.db.conn(), &enc, q) {
                Ok(e) => e,
                Err(e) => return format!("Error: {e}"),
            }
        } else {
            return "Error: provide 'service' (exact) or 'search' (partial match)".to_string();
        };

        // The OS's own provider keys are not the companion's to read (provider_keys).
        let entries: Vec<_> = entries.into_iter().filter(|e| !crate::provider_keys::is_os_entry(&e.service, &e.category)).collect();
        if entries.is_empty() {
            return "No credentials found".to_string();
        }

        let mut out = String::new();
        for e in &entries {
            out.push_str(&format!("Service: {}\n", e.service));
            out.push_str(&format!("Username: {}\n", e.username));
            out.push_str(&format!("Password: {}\n", e.password));
            if let Some(url) = &e.url {
                out.push_str(&format!("URL: {url}\n"));
            }
            if let Some(notes) = &e.notes {
                out.push_str(&format!("Notes: {notes}\n"));
            }
            out.push_str(&format!("Category: {}\n", e.category));
            out.push('\n');
        }

        out
    }
}

// ── Vault List ──

struct VaultListTool;

impl Tool for VaultListTool {
    fn name(&self) -> &'static str { "vault_list" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "vault" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "vault_list",
                "description": "List stored services only, not secrets",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, _args: &serde_json::Value) -> String {
        // Bound first: a `match` scrutinee holds its temporaries — here the connection guard —
        // for the whole match, and an arm below asks for the same lock.
        let listed = yantrikdb_core::vault::list(&ctx.db.conn()).map(|entries| {
            entries.into_iter().filter(|e| !crate::provider_keys::is_os_entry(&e.service, &e.category)).collect::<Vec<_>>()
        });
        match listed {
            Ok(entries) if entries.is_empty() => "Vault is empty. No credentials stored yet.".to_string(),
            Ok(entries) => {
                // What "off" actually means, rather than a label. An unprotected vault is not a
                // vault with a feature switched off: its key is in the same file as the
                // ciphertext, so anyone holding a copy of that file holds every credential in it.
                // A caller relaying this to a person should be able to say so.
                let pin_status = if yantrikdb_core::vault::has_pin(&ctx.db.conn()) {
                    if yantrikdb_core::vault::is_unlocked() {
                        "Locked with a passphrase, and open in this session"
                    } else {
                        "Locked with a passphrase, and shut — the person has to unlock it"
                    }
                } else {
                    "NOT protected: the key is stored in the memory database beside the \
                     credentials, so anyone with a copy of that file can read all of them. The \
                     person can set a passphrase from Settings > Privacy & Security."
                };
                let mut out = format!("{} credentials stored | {pin_status}\n\n", entries.len());
                for e in &entries {
                    out.push_str(&format!("- {} [{}]", e.service, e.category));
                    if let Some(url) = &e.url {
                        out.push_str(&format!(" ({url})"));
                    }
                    out.push('\n');
                }
                out
            }
            Err(e) => format!("Error: {e}"),
        }
    }
}

// ── Vault Delete ──

struct VaultDeleteTool;

impl Tool for VaultDeleteTool {
    fn name(&self) -> &'static str { "vault_delete" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "vault" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "vault_delete",
                "description": "Delete vault credential by service",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "service": {"type": "string", "description": "Service name to delete"}
                    },
                    "required": ["service"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let service = args.get("service").and_then(|v| v.as_str()).unwrap_or_default();

        if service.is_empty() {
            return "Error: service is required".to_string();
        }
        if crate::provider_keys::is_os_entry(service, "") {
            return format!("Refused: `{service}` is the OS's own, and is removed from Settings, not here.");
        }

        // Deleting does not need the key — the rows come out whatever is in them — but a locked
        // vault must not be destroyable by something that cannot read it. A caller that cannot
        // tell you what it is about to delete should not be deleting it.
        if let Err(locked) = ensure_open(&ctx.db.conn(), "a mind wants to delete a credential") {
            return locked;
        }

        match yantrikdb_core::vault::delete_by_service(&ctx.db.conn(), service) {
            Ok(0) => format!("No credentials found for '{service}'"),
            Ok(n) => format!("Deleted {n} credential(s) for '{service}'"),
            Err(e) => format!("Error: {e}"),
        }
    }
}

// ── Vault Generate Password ──

struct VaultGeneratePasswordTool;

impl Tool for VaultGeneratePasswordTool {
    fn name(&self) -> &'static str { "vault_generate_password" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "vault" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "vault_generate_password",
                "description": "Generate strong password; does not store it",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "length": {"type": "integer", "description": "Password length (8-128, default 20)"},
                        "special_chars": {"type": "boolean", "description": "Include special characters (default true)"}
                    }
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let length = args.get("length").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
        let special = args.get("special_chars").and_then(|v| v.as_bool()).unwrap_or(true);

        let password = yantrikdb_core::vault::generate_password(length, special);
        format!("Generated password: {password}\n\nUse vault_store to save it securely.")
    }
}

// ── Vault Set PIN ──

/// Ask the desktop to ask the person. The mind cannot answer this one itself.
///
/// This tool used to take `new_pin` and `current_pin` and write them straight into the vault's
/// wrapping, which meant the model was the thing choosing — and carrying — the secret that
/// protects every credential on the machine. On a companion answering over Telegram that secret
/// crossed a third party to reach a desk it was standing in front of.
///
/// It still exists, and a mind can still start the flow, because "the vault is not protected and
/// it should be" is a genuinely useful thing for a companion to notice and raise. What it can no
/// longer do is finish it: the passphrase is typed into a prompt this shell draws, on the machine,
/// by a person. The tool's answer says that, so a model reading it knows to stop asking.
struct VaultSetPinTool;

impl Tool for VaultSetPinTool {
    fn name(&self) -> &'static str { "vault_set_pin" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Sensitive }
    fn category(&self) -> &'static str { "vault" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "vault_set_pin",
                "description": "Ask the desktop to prompt the person for a vault passphrase. \
                                Takes no passphrase: you cannot set or carry one, and you must \
                                not ask the person to tell you theirs. The person types it into \
                                the desktop's own prompt.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["set", "change"],
                            "description": "'set' for a vault that has no passphrase, 'change' to \
                                            replace the one it has. Both just raise the prompt."
                        }
                    },
                    "required": ["action"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("set");
        let protected = yantrikdb_core::vault::has_pin(&ctx.db.conn());

        // Loud rather than ignored. A model that passed a passphrase here has been told to ask the
        // person for it by something — an older prompt, a habit, its own reasoning — and the one
        // useful thing to do is say that the secret it is holding should not have been said out
        // loud, so whoever reads the transcript knows to change it.
        for leaked in ["new_pin", "current_pin", "pin", "passphrase", "password"] {
            if args.get(leaked).is_some() {
                return format!(
                    "REFUSED: `{leaked}` is not an argument of this tool and was ignored. A vault \
                     passphrase cannot travel through a tool call — it is typed on the machine, \
                     into the desktop's own prompt. If a person just told you one, tell them to \
                     change it: it is now in this conversation."
                );
            }
        }

        match action {
            "set" | "change" => {
                if action == "set" && protected {
                    return "This vault already has a passphrase. Use action='change' to have the \
                            desktop ask for a new one."
                        .to_string();
                }
                if action == "change" && !protected {
                    return "This vault has no passphrase yet. Use action='set' to have the \
                            desktop ask for one."
                        .to_string();
                }
                request_unlock(
                    if protected {
                        "you asked to change the vault's passphrase"
                    } else {
                        "a mind suggested locking the vault with a passphrase"
                    },
                    !protected,
                );
                "The desktop is now asking the person for a vault passphrase. You will not see it \
                 and you do not need it. Tell them it is on screen, and stop here."
                    .to_string()
            }
            "remove" => {
                // Refused outright rather than routed to a prompt. Removing the passphrase writes
                // the key back into the file in the clear — it is the one vault operation that
                // makes the machine less safe, and it is not something a mind should be able to
                // put in front of a person as a one-click suggestion.
                "REFUSED: removing the vault's passphrase puts its key back in the memory \
                 database in the clear, where anyone with a copy of that file can read every \
                 credential. If the person wants that, they can do it in Settings > Privacy & \
                 Security, where it says so."
                    .to_string()
            }
            _ => "Error: action must be 'set' or 'change'".to_string(),
        }
    }
}

#[cfg(test)]
mod vault_tool_tests {
    use super::*;

    /// The latch is process-wide, so these take turns.
    static SERIAL: Mutex<()> = Mutex::new(());

    fn drain() {
        let _ = take_unlock_request();
    }

    /// The OS's own provider keys are in the vault and none of the companion's business: no
    /// vault tool reads, lists, overwrites or deletes one (1 Oct 2026). The person's own entries
    /// are untouched by this.
    #[test]
    fn the_oss_provider_keys_are_out_of_every_vault_tools_reach() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let db = yantrikdb_core::YantrikDB::new(":memory:", 384).expect("in-memory database");
        let ctx = ToolContext { db: &db, max_permission: PermissionLevel::Dangerous, registry_metadata: None, task_manager: None, incognito: true, agent_spawner: None };
        let key = "gsk_SECRETSECRETSECRETSECRETSECRETSECRET1234";
        crate::provider_keys::store(&db.conn(), "groq", key).expect("stored");
        let run = |tool: &dyn Tool, args: serde_json::Value| tool.execute(&ctx, &args);
        run(&VaultStoreTool, serde_json::json!({"service": "github.com", "username": "me", "password": "mine"}));

        let got = run(&VaultGetTool, serde_json::json!({"search": "os:"}));
        assert!(!got.contains("SECRET"), "vault_get search read the key: {got}");
        let got = run(&VaultGetTool, serde_json::json!({"service": crate::provider_keys::service("groq")}));
        assert!(!got.contains("SECRET"), "vault_get read the key: {got}");
        let listed = run(&VaultListTool, serde_json::json!({}));
        assert!(!listed.contains("provider-key") && listed.starts_with("1 credentials"), "{listed}");
        let overwrite = run(&VaultStoreTool, serde_json::json!({"service": crate::provider_keys::service("groq"), "username": "x", "password": "gsk_attacker"}));
        assert!(overwrite.starts_with("Refused"), "{overwrite}");
        let sneaky = run(&VaultStoreTool, serde_json::json!({"service": "groq", "username": "x", "password": "y", "category": "os:provider-key"}));
        assert!(sneaky.starts_with("Refused"), "{sneaky}");
        let deleted = run(&VaultDeleteTool, serde_json::json!({"service": crate::provider_keys::service("groq")}));
        assert!(deleted.starts_with("Refused"), "{deleted}");

        // Still there, unchanged, for the pool; and the person's own entry still reads.
        let kept = crate::provider_keys::load_all(&db.conn()).unwrap();
        assert_eq!(kept.get("groq").map(String::as_str), Some(key));
        assert_eq!(kept.len(), 1, "the person's entry is not a provider key");
        assert_eq!(crate::provider_keys::kept(&db.conn()), vec!["groq".to_string()]);
        assert!(run(&VaultGetTool, serde_json::json!({"service": "github.com"})).contains("Password: mine"));
        assert!(crate::provider_keys::remove(&db.conn(), "groq").unwrap());
        assert!(crate::provider_keys::load_all(&db.conn()).unwrap().is_empty());
    }

    /// A retrying caller cannot rewrite what the person is already reading.
    ///
    /// The realistic shape of this: a model gets `VAULT_LOCKED`, does not understand that it has
    /// to stop, and calls `vault_get` four more times. Each call raises the latch again. If the
    /// last reason won, the card's explanation would change under the person's eyes while they
    /// typed.
    #[test]
    fn the_first_reason_is_the_one_the_person_reads() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        drain();

        request_unlock("a mind asked for a saved credential", false);
        request_unlock("something else entirely", true);

        let (reason, first_time) = take_unlock_request().expect("a request should be pending");
        assert_eq!(reason, "a mind asked for a saved credential");
        assert!(!first_time);
    }

    /// Taking it clears it. A prompt that came back every second would be unusable.
    #[test]
    fn a_request_is_taken_once() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        drain();

        assert!(take_unlock_request().is_none(), "nothing asked, nothing pending");
        request_unlock("a mind asked for a saved credential", false);
        assert!(take_unlock_request().is_some());
        assert!(take_unlock_request().is_none());
    }

    /// The locked answer is recognisable, and says the two things a model has to know.
    ///
    /// It is the difference between this and a generic error: a model reading `Error: …` retries,
    /// and a model reading this tells the person to look at their screen. The second half — that
    /// it must not ask for the passphrase — is there because the obvious next move for a helpful
    /// model is to ask, and asking is the failure.
    #[test]
    fn the_locked_answer_tells_a_mind_to_stop_rather_than_to_ask() {
        assert!(LOCKED_ANSWER.starts_with("VAULT_LOCKED:"));
        assert!(LOCKED_ANSWER.contains("the person has to unlock it"));
        assert!(LOCKED_ANSWER.contains("you cannot carry it"));
    }
}
