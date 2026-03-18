//! Vault tools — secure credential storage with AES-256-GCM encryption.
//!
//! Tools: vault_store, vault_get, vault_list, vault_delete, vault_generate_password, vault_set_pin
//!
//! Security model:
//!   - All credentials encrypted with AES-256-GCM at rest (vault-specific DEK)
//!   - DEK auto-generated on first use, stored in vault_security table
//!   - Optional security PIN protects vault_get (credential retrieval)
//!   - PIN is blake3-hashed and stored in vault_security table
//!   - When PIN is set, vault_get requires PIN parameter before returning passwords
//!   - Prevents unauthorized access over Telegram or shared sessions

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};

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

        let enc = match yantrikdb_core::vault::vault_encryption(ctx.db.conn()) {
            Ok(e) => e,
            Err(e) => return format!("Error: {e}"),
        };

        match yantrikdb_core::vault::store(ctx.db.conn(), &enc, service, username, password, url, notes, category) {
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
                        "search": {"type": "string", "description": "Search by partial service name (alternative to exact match)"},
                        "pin": {"type": "string", "description": "Vault security PIN (required if PIN is configured). Ask the user for this."}
                    }
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let service = args.get("service").and_then(|v| v.as_str());
        let search = args.get("search").and_then(|v| v.as_str());
        let pin = args.get("pin").and_then(|v| v.as_str());

        let enc = match yantrikdb_core::vault::vault_encryption(ctx.db.conn()) {
            Ok(e) => e,
            Err(e) => return format!("Error: {e}"),
        };

        // PIN verification
        if yantrikdb_core::vault::has_pin(ctx.db.conn()) {
            match pin {
                None => return "VAULT_PIN_REQUIRED: A security PIN is required to access credentials. \
                    Please ask the user to provide their vault PIN.".to_string(),
                Some(p) => {
                    if !yantrikdb_core::vault::verify_pin(ctx.db.conn(), p) {
                        return "VAULT_PIN_INVALID: Incorrect PIN. Access denied.".to_string();
                    }
                }
            }
        }

        let entries = if let Some(svc) = service {
            match yantrikdb_core::vault::get(ctx.db.conn(), &enc, svc) {
                Ok(e) => e,
                Err(e) => return format!("Error: {e}"),
            }
        } else if let Some(q) = search {
            match yantrikdb_core::vault::search(ctx.db.conn(), &enc, q) {
                Ok(e) => e,
                Err(e) => return format!("Error: {e}"),
            }
        } else {
            return "Error: provide 'service' (exact) or 'search' (partial match)".to_string();
        };

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
        match yantrikdb_core::vault::list(ctx.db.conn()) {
            Ok(entries) if entries.is_empty() => "Vault is empty. No credentials stored yet.".to_string(),
            Ok(entries) => {
                let pin_status = if yantrikdb_core::vault::has_pin(ctx.db.conn()) {
                    "PIN protection: ENABLED"
                } else {
                    "PIN protection: DISABLED (set one with vault_set_pin for security)"
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
                        "service": {"type": "string", "description": "Service name to delete"},
                        "pin": {"type": "string", "description": "Vault PIN (required if PIN is configured)"}
                    },
                    "required": ["service"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let service = args.get("service").and_then(|v| v.as_str()).unwrap_or_default();
        let pin = args.get("pin").and_then(|v| v.as_str());

        if service.is_empty() {
            return "Error: service is required".to_string();
        }

        // PIN verification for destructive vault operations
        if yantrikdb_core::vault::has_pin(ctx.db.conn()) {
            match pin {
                None => return "VAULT_PIN_REQUIRED: PIN required to delete vault entries. Ask the user for their vault PIN.".to_string(),
                Some(p) if !yantrikdb_core::vault::verify_pin(ctx.db.conn(), p) => {
                    return "VAULT_PIN_INVALID: Incorrect PIN. Access denied.".to_string();
                }
                _ => {}
            }
        }

        match yantrikdb_core::vault::delete_by_service(ctx.db.conn(), service) {
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
                "description": "Set, change, or remove vault PIN",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "action": {"type": "string", "enum": ["set", "change", "remove"], "description": "Action to perform"},
                        "current_pin": {"type": "string", "description": "Current PIN (required for 'change' and 'remove')"},
                        "new_pin": {"type": "string", "description": "New PIN to set (required for 'set' and 'change'). Minimum 4 characters."}
                    },
                    "required": ["action"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("set");
        let current_pin = args.get("current_pin").and_then(|v| v.as_str());
        let new_pin = args.get("new_pin").and_then(|v| v.as_str());
        let has_pin = yantrikdb_core::vault::has_pin(ctx.db.conn());

        match action {
            "set" => {
                if has_pin {
                    return "A PIN is already set. Use action='change' with current_pin to update it.".to_string();
                }
                let pin = match new_pin {
                    Some(p) if p.len() >= 4 => p,
                    Some(_) => return "Error: PIN must be at least 4 characters".to_string(),
                    None => return "Error: new_pin is required".to_string(),
                };
                match yantrikdb_core::vault::set_pin(ctx.db.conn(), pin) {
                    Ok(()) => "Vault PIN set successfully. vault_get and vault_delete now require this PIN.".to_string(),
                    Err(e) => format!("Error: {e}"),
                }
            }
            "change" => {
                if !has_pin {
                    return "No PIN is set. Use action='set' to create one.".to_string();
                }
                match current_pin {
                    None => return "Error: current_pin is required to change PIN".to_string(),
                    Some(p) if !yantrikdb_core::vault::verify_pin(ctx.db.conn(), p) => {
                        return "VAULT_PIN_INVALID: Current PIN is incorrect.".to_string();
                    }
                    _ => {}
                }
                let pin = match new_pin {
                    Some(p) if p.len() >= 4 => p,
                    Some(_) => return "Error: new PIN must be at least 4 characters".to_string(),
                    None => return "Error: new_pin is required".to_string(),
                };
                match yantrikdb_core::vault::set_pin(ctx.db.conn(), pin) {
                    Ok(()) => "Vault PIN changed successfully.".to_string(),
                    Err(e) => format!("Error: {e}"),
                }
            }
            "remove" => {
                if !has_pin {
                    return "No PIN is set.".to_string();
                }
                match current_pin {
                    None => return "Error: current_pin is required to remove PIN".to_string(),
                    Some(p) if !yantrikdb_core::vault::verify_pin(ctx.db.conn(), p) => {
                        return "VAULT_PIN_INVALID: Current PIN is incorrect.".to_string();
                    }
                    _ => {}
                }
                match yantrikdb_core::vault::remove_pin(ctx.db.conn()) {
                    Ok(()) => "Vault PIN removed. Credentials are now accessible without PIN verification.".to_string(),
                    Err(e) => format!("Error: {e}"),
                }
            }
            _ => "Error: action must be 'set', 'change', or 'remove'".to_string(),
        }
    }
}
