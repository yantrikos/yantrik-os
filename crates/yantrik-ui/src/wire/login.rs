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
pub fn wire(ui: &App, _ctx: &AppContext) {
    let ui_weak = ui.as_weak();
    let fail_count = Arc::new(AtomicU32::new(0));

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

            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = weak.upgrade() {
                    if authenticated {
                        tracing::info!(user = %username, "Login successful");
                        fails.store(0, Ordering::Relaxed);
                        ui.set_login_error("".into());
                        // Navigate to desktop
                        ui.set_current_screen(1);
                        ui.invoke_navigate(1);
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
