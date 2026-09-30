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
pub(crate) fn adopt_session_password(bridge: &Arc<crate::bridge::CompanionBridge>, password: &str) {
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
///
/// `unix_chkpwd` is PAM's setgid-shadow helper: run as the account itself it checks that
/// account's password, which it reads from stdin **NUL-terminated**, in the mode named by its
/// second argument. That argument was `chkexpiry`, commented as a dummy the helper ignores. It is
/// not ignored: `chkexpiry` is the expiry check, which never reads a password and exits 0 for any
/// account that has not expired — so every login, and every unlock, accepted any password at all
/// (verified on VM 520: a wrong password, exit 0). `nonull` is the password check (an empty
/// password is refused), and the password went down the pipe newline-terminated, which the helper
/// reads as part of the password. `chkpwd_request` is the one place both are decided.
///
/// A failed check is final. Only when the helper cannot be run at all does root read
/// /etc/shadow itself (the installer's chroot); there is no third way.
pub(crate) fn verify_password(username: &str, password: &str) -> bool {
    // The helper reads up to a NUL: "right\0anything" would be checked as "right".
    if password.is_empty() || password.contains('\0') {
        return false;
    }
    for chkpwd_path in &["/usr/sbin/unix_chkpwd", "/sbin/unix_chkpwd"] {
        if !std::path::Path::new(chkpwd_path).exists() {
            continue;
        }
        let (mode, stdin) = chkpwd_request(password);
        match Command::new(chkpwd_path)
            .args([username, mode])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(mut child) => {
                if let Some(mut pipe) = child.stdin.take() {
                    use std::io::Write;
                    let _ = pipe.write_all(&stdin);
                }
                return match child.wait() {
                    Ok(status) => status.success(),
                    Err(e) => {
                        tracing::warn!(error = %e, "unix_chkpwd wait failed");
                        false
                    }
                };
            }
            Err(e) => {
                tracing::warn!(path = chkpwd_path, error = %e, "unix_chkpwd spawn failed");
            }
        }
    }

    // No helper: only root can read /etc/shadow, and then it checks the hash itself.
    if let Ok(shadow) = std::fs::read_to_string("/etc/shadow") {
        for line in shadow.lines() {
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() >= 2 && parts[0] == username {
                let stored_hash = parts[1];
                if stored_hash.starts_with('!') || stored_hash.starts_with('*') || stored_hash.is_empty() {
                    tracing::warn!(user = username, "Account is locked or has no password");
                    return false;
                }
                return verify_shadow_hash(password, stored_hash);
            }
        }
        tracing::warn!(user = username, "User not found in /etc/shadow");
    } else {
        tracing::warn!("No unix_chkpwd and no /etc/shadow to read: the password cannot be checked");
    }
    false
}

/// What `unix_chkpwd` is asked, and what goes down its stdin: the password check (`nonull`), and
/// the password NUL-terminated, exactly as typed.
fn chkpwd_request(password: &str) -> (&'static str, Vec<u8>) {
    let mut stdin = password.as_bytes().to_vec();
    stdin.push(0);
    ("nonull", stdin)
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
    match Command::new("/usr/bin/openssl")
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

#[cfg(test)]
mod password_check_tests {
    use super::*;

    /// `chkexpiry` never reads the password and passes every account that has not expired: any
    /// password logged in. The check is `nonull`, and the helper reads up to a NUL.
    #[test]
    fn the_helper_is_asked_to_check_the_password_as_typed() {
        let (mode, stdin) = chkpwd_request("pa ss\u{e9}");
        assert_eq!(mode, "nonull", "any other mode does not check the password");
        assert_eq!(stdin, b"pa ss\xc3\xa9\0", "the password, exactly, then NUL; no newline");
    }

    #[test]
    fn an_empty_password_is_wrong_before_anything_is_asked() {
        assert!(!verify_password("nobody-at-all", ""));
        assert!(!verify_password("nobody-at-all", "right\0junk"), "a NUL would end the password early");
    }

    /// Against the real helper, as the account being checked (it only checks its caller's own).
    /// `YOS_CHKPWD_PASSWORD` is that account's password; run where one is set up for it:
    /// `YOS_CHKPWD_PASSWORD=… cargo test -- --ignored the_real_helper`.
    #[test]
    #[ignore]
    fn the_real_helper_takes_the_right_password_and_refuses_a_wrong_one() {
        let user = String::from_utf8(Command::new("/usr/bin/id").arg("-un").output().unwrap().stdout).unwrap();
        let user = user.trim();
        let right = std::env::var("YOS_CHKPWD_PASSWORD").expect("YOS_CHKPWD_PASSWORD");
        assert!(verify_password(user, &right), "the right password was refused");
        assert!(!verify_password(user, "definitely-not-the-password"), "a wrong password was accepted");
        assert!(!verify_password(user, &format!("{right} ")), "a different password was accepted");
    }
}
