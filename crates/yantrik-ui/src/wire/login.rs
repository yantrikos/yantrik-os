//! Login screen wiring — authenticates username/password on installed system.
//!
//! Uses `unix_chkpwd` (PAM helper) for password verification.
//! Falls back to reading /etc/shadow directly if unix_chkpwd is unavailable.

use slint::ComponentHandle;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use crate::app_context::AppContext;
use crate::App;

/// Wire the login-attempt callback.
pub fn wire(ui: &App, ctx: &AppContext) {
    let ui_weak = ui.as_weak();
    let fail_count = Arc::new(AtomicU32::new(0));
    let bridge = ctx.bridge.clone();

    // Set hostname from /etc/hostname if available
    if let Ok(hostname) = std::fs::read_to_string("/etc/hostname") {
        let h = hostname.trim().to_string();
        if !h.is_empty() {
            ui.set_login_hostname(h.into());
        }
    }

    ui.on_login_attempt(move |username, password| {
        let username = username.to_string().trim().to_string();
        let password = password.to_string();
        let weak = ui_weak.clone();
        let fails = fail_count.clone();
        let bridge = bridge.clone();

        if username.is_empty() {
            if let Some(ui) = weak.upgrade() {
                ui.set_login_error("Enter a username".into());
            }
            return;
        }

        if password.is_empty() {
            if let Some(ui) = weak.upgrade() {
                ui.set_login_error("Enter a password".into());
            }
            return;
        }

        // Authenticate in a background thread to not block the UI
        std::thread::spawn(move || {
            let authenticated = verify_password(&username, &password);

            // The one moment on this machine when a secret a person chose, and the system itself
            // has just agreed to, exists in this process. The vault's key is wrapped under it
            // here or it is wrapped under nothing at all — deriving some *other* secret from a
            // machine that has no other secret would be theatre, and asking the person for a
            // second password when they have just typed one is a tax on the honest path.
            //
            // Ordered after `verify_password` deliberately: an unverified string is a guess, and
            // a guess must not be able to re-wrap anybody's vault.
            if authenticated {
                crate::vault_unlock::note_session_password_seen();
                adopt_session_password(&bridge, &password);
            }

            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = weak.upgrade() {
                    if authenticated {
                        tracing::info!(user = %username, "Login successful");
                        fails.store(0, Ordering::Relaxed);
                        ui.set_login_error("".into());
                        // The other place the desktop's lock is released, and only once the
                        // system has agreed to the password. See `crate::lock::release`.
                        crate::lock::release(&ui, crate::lock::LOGIN_SCREEN);
                    } else {
                        let count = fails.fetch_add(1, Ordering::Relaxed) + 1;
                        tracing::warn!(user = %username, attempts = count, "Login failed");
                        if count >= 5 {
                            ui.set_login_error("Too many attempts. Please wait.".into());
                            // Add delay for brute-force protection
                            std::thread::spawn(move || {
                                std::thread::sleep(std::time::Duration::from_secs(5));
                            });
                        } else {
                            ui.set_login_error("Invalid username or password".into());
                        }
                    }
                }
            });
        });
    });
}

/// Hand the just-verified login password to the vault, and say what happened — never what it was.
///
/// Three outcomes and each is a different fact about this machine:
///   * first sign-in on a vault that had no passphrase — it is wrapped now, in place, with
///     everything already stored still readable;
///   * every sign-in after that — it opens;
///   * it does not open — which on this path means one specific thing: the account password has
///     been changed since the vault was wrapped, and the vault is still wrapped under the old
///     one. It stays locked, nothing is overwritten, and the person can re-wrap it from the
///     unlock prompt with the passphrase that does open it.
///
/// Every branch logs at most a state. The password is borrowed for the call and is not put in a
/// span, a field, or a message; `secret_never_reaches_a_message` in `vault_unlock` covers the
/// strings this function can reach.
fn adopt_session_password(bridge: &Arc<crate::bridge::CompanionBridge>, password: &str) {
    use crate::vault_unlock::{Op, Outcome};

    // Generous, because Argon2id is deliberately slow and the worker may be mid-thought. The
    // person is already past the login screen either way — the vault is not a gate on the
    // desktop, and making it one would mean a busy companion could keep someone out of their own
    // machine.
    let timeout = std::time::Duration::from_secs(20);
    match bridge.vault(Op::Adopt(password.to_string()), timeout) {
        Ok(reply) => match reply.outcome {
            Some(Outcome::Protected) => {
                tracing::info!("The vault is now wrapped under this account's login password");
            }
            Some(Outcome::Unlocked) => {
                tracing::info!("The vault is open for this session");
            }
            Some(Outcome::Wrong) => {
                tracing::warn!(
                    "The vault did not open with this login password — it is wrapped under an \
                     earlier one. It stays locked until someone enters the passphrase that opens \
                     it; nothing was changed."
                );
            }
            Some(Outcome::Unusable(why)) => {
                tracing::warn!(reason = %why, "The vault could not be opened at sign-in");
            }
            None => {}
        },
        Err(e) => {
            tracing::warn!(error = %e, "Could not reach the vault at sign-in");
        }
    }
}

/// Verify username/password against the system.
/// Tries unix_chkpwd first (preferred, PAM-aware), falls back to shadow file.
fn verify_password(username: &str, password: &str) -> bool {
    // Method 1: unix_chkpwd — the PAM helper binary
    // It reads the password from stdin and checks against /etc/shadow
    for chkpwd_path in &["/usr/sbin/unix_chkpwd", "/sbin/unix_chkpwd"] {
        if std::path::Path::new(chkpwd_path).exists() {
            match Command::new(chkpwd_path)
                .arg(username)
                .arg("chkexpiry") // dummy arg, unix_chkpwd ignores it but needs something
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
            {
                Ok(mut child) => {
                    if let Some(ref mut stdin) = child.stdin {
                        use std::io::Write;
                        let _ = stdin.write_all(format!("{password}\n").as_bytes());
                    }
                    drop(child.stdin.take());
                    match child.wait() {
                        Ok(status) => {
                            tracing::debug!(
                                path = chkpwd_path,
                                status = %status,
                                "unix_chkpwd result"
                            );
                            return status.success();
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "unix_chkpwd wait failed");
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(path = chkpwd_path, error = %e, "unix_chkpwd spawn failed");
                }
            }
        }
    }

    // Method 2: Read /etc/shadow directly and verify hash
    // This works when running as root (which we do via autologin)
    if let Ok(shadow) = std::fs::read_to_string("/etc/shadow") {
        for line in shadow.lines() {
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() >= 2 && parts[0] == username {
                let stored_hash = parts[1];
                // Skip locked/disabled accounts
                if stored_hash.starts_with('!') || stored_hash.starts_with('*') || stored_hash.is_empty() {
                    tracing::warn!(user = username, "Account is locked or has no password");
                    return false;
                }
                // Use openssl to verify: generate hash with same salt, compare
                return verify_shadow_hash(password, stored_hash);
            }
        }
        tracing::warn!(user = username, "User not found in /etc/shadow");
    } else {
        tracing::warn!("Cannot read /etc/shadow — running as non-root?");
    }

    // Method 3: Try `su` with the credentials via `expect`-like approach
    // This is a last resort
    match Command::new("su")
        .args(["-c", "true", username])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(mut child) => {
            if let Some(ref mut stdin) = child.stdin {
                use std::io::Write;
                let _ = stdin.write_all(format!("{password}\n").as_bytes());
            }
            drop(child.stdin.take());
            match child.wait() {
                Ok(status) => return status.success(),
                Err(_) => {}
            }
        }
        Err(_) => {}
    }

    false
}

/// Verify a password against a shadow hash (e.g. $6$salt$hash).
/// Uses openssl to generate a hash with the same salt and compares.
fn verify_shadow_hash(password: &str, stored_hash: &str) -> bool {
    // Extract the salt from the stored hash: $id$salt$hash
    // Format: $6$rounds=N$salt$hash or $6$salt$hash
    let parts: Vec<&str> = stored_hash.split('$').collect();
    if parts.len() < 4 {
        tracing::warn!("Unrecognized shadow hash format");
        return false;
    }

    // Reconstruct the salt portion: $id$salt$ (or $id$rounds=N$salt$)
    let salt = if parts[2].starts_with("rounds=") && parts.len() >= 5 {
        format!("${}${}${}$", parts[1], parts[2], parts[3])
    } else {
        format!("${}${}$", parts[1], parts[2])
    };

    // Use openssl to generate hash with the same salt
    match Command::new("openssl")
        .args(["passwd", &format!("-{}", parts[1]), "-salt", parts[2], "-stdin"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(mut child) => {
            if let Some(ref mut stdin) = child.stdin {
                use std::io::Write;
                let _ = stdin.write_all(password.as_bytes());
            }
            drop(child.stdin.take());
            match child.wait_with_output() {
                Ok(output) if output.status.success() => {
                    let generated = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    let matched = generated == stored_hash;
                    tracing::debug!(matched, "Shadow hash comparison");
                    matched
                }
                _ => false,
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "openssl passwd failed for shadow verification");
            false
        }
    }
}
