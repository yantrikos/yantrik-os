//! The lock, held by the compositor (#313).
//!
//! The lock screen was screen 3 inside the shell's own window, an ordinary toplevel: an app
//! window in front when the desktop locked stayed in front and usable, and the compositor would
//! raise any window over the "lock" on request (VM 520: Weather and Calendar over it, working,
//! with `describe` saying `locked: true`). Now locking also starts `yantrik-lock`, which takes the
//! session lock (`ext-session-lock-v1`): the compositor shows only its surfaces and sends input
//! only to them.
//!
//! It does not know the secret: it asks this process, one line at a time, and this answers with
//! `lock::check_unlock` — the same check the shell's own screen makes, in one place: the login
//! password, or the PIN on an account without one (#414). On `ok` it unlocks the session and the
//! shell runs its own unlock (back to the desktop, the vault offered the same secret). A compositor without the protocol (exit 3) leaves the shell's own screen as
//! the lock, as before, and says so. A lock client that dies while the session is locked is
//! started again, so the person can always get back in.

use std::io::{BufRead, BufReader, Write};
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::App;

/// One lock client at a time, however many ways the desktop was asked to lock.
static RUNNING: AtomicBool = AtomicBool::new(false);

/// Whether the desktop is still meant to be locked. A client that keeps failing to start is
/// started again only while it is: once the person is back in (through the shell's own screen,
/// when no client could take the lock), a client that finally connects must not lock the desktop
/// they are using.
static WANTED: AtomicBool = AtomicBool::new(false);

/// Whether a lock client has held the session since this shell started. After that, a client the
/// compositor refuses ("already locked": the one before it had not gone yet) is started again,
/// not taken as a compositor without the protocol.
static LOCKED_ONCE: AtomicBool = AtomicBool::new(false);

/// The person is back in; stop starting lock clients.
pub fn released() {
    WANTED.store(false, Ordering::SeqCst);
}

/// The compositor has no session lock.
const EXIT_UNSUPPORTED: i32 = 3;
/// The longest pause between starts of a lock client that keeps ending without unlocking.
const MOST_PAUSE_SECS: u64 = 5;

/// Lock clients left by a shell that is gone: this user's `yantrik-lock` processes. A client
/// answers only the shell that started it, so one whose shell ended (an update restarting the
/// shell, a crash) can never unlock again while it holds the session locked.
fn orphaned_clients() -> Vec<u32> {
    use std::os::unix::fs::MetadataExt;
    let me = unsafe { libc::getuid() };
    let Ok(procs) = std::fs::read_dir("/proc") else { return Vec::new() };
    procs
        .flatten()
        .filter_map(|e| {
            let pid: u32 = e.file_name().to_str()?.parse().ok()?;
            let owner = std::fs::metadata(e.path()).ok()?.uid();
            let comm = std::fs::read_to_string(e.path().join("comm")).ok()?;
            (owner == me && comm.trim() == "yantrik-lock").then_some(pid)
        })
        .collect()
}

/// At startup: a lock client from an earlier shell means the desktop was locked when that shell
/// ended, and nothing can answer it now. It is replaced, not left: the old client is ended and
/// this shell locks again at once, so the desktop stays locked and can be unlocked. Without this,
/// an update applied while the screen was locked left the person locked out.
pub fn take_over_orphans(ui: &App) {
    let orphans = orphaned_clients();
    if orphans.is_empty() {
        return;
    }
    // Ended, and gone, before this shell asks for the lock: the compositor refuses a new lock
    // while the old client still holds it, and a refused client looks like no protocol at all.
    for (signal, wait) in [(libc::SIGTERM, 2000u64), (libc::SIGKILL, 1000)] {
        for pid in orphans.iter().filter(|p| std::path::Path::new(&format!("/proc/{p}")).exists()) {
            // SAFETY: a plain signal to a process of our own user.
            unsafe {
                libc::kill(*pid as libc::pid_t, signal);
            }
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(wait);
        while orphans.iter().any(|p| std::path::Path::new(&format!("/proc/{p}")).exists())
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
    tracing::warn!(?orphans, "A lock client from an earlier shell was holding the desktop; locking again from this one");
    ui.invoke_lock_screen();
}

/// At startup: an installed machine whose account has a password starts locked (#415).
///
/// The session logs in by itself at boot (agetty --autologin), so a desktop that started open made
/// every lock a screen saver: restart the machine, or kill the shell, and it was open. It starts
/// behind the same session lock as every other lock, asking for the account's password, and the
/// vault opens with that password when it is given. Not on the live image (its password is
/// published) and not on an account with no password (there is nothing to ask for); and not when
/// a start screen is named (the GUI installer's login screen, a developer's override).
pub fn lock_at_start(ui: &App) {
    // A developer's start screen, in a debug build only. In a shipped one the file that names it
    // (~/.config/labwc/environment) is the user's, so a mind could write "start on the desktop".
    if cfg!(debug_assertions) && std::env::var_os("YANTRIK_START_SCREEN").is_some() {
        return;
    }
    if crate::lock::live_session() {
        return;
    }
    if crate::lock::account_says_no_password() {
        tracing::warn!("This account has no password; the desktop starts open. Set one to lock it at boot");
        return;
    }
    // Already locked by the takeover of an earlier shell's lock.
    if ui.get_current_screen() == 3 {
        return;
    }
    // An encrypted disk still starts locked (#400 step b): whatever opened the disk describes the
    // boot, not this start, and the shell starts again whenever the session does (a crash, or
    // Alt+SysRq+K at a locked screen); the vault is handed its key by this unlock; and the disk's
    // passphrase does not follow a later change of the password.
    tracing::info!("Starting locked: the account's password opens the desktop");
    ui.invoke_lock_screen();
}

/// What to answer a line from the lock client, and the secret when the answer unlocks: `ok`, or
/// `no <what the lock screen should say>`.
pub fn answer(
    line: &str,
    asking: crate::lock::Secret,
    check: impl Fn(&str) -> crate::lock::Verdict,
) -> Option<(String, Option<(String, crate::lock::Secret)>)> {
    let secret = line.strip_prefix("secret ")?;
    match check(secret) {
        crate::lock::Verdict::Open(checked) => Some(("ok".into(), Some((secret.to_string(), checked)))),
        refused => Some((format!("no {}", refused.message(asking)), None)),
    }
}

fn lock_bin() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("yantrik-lock")))
        .unwrap_or_else(|| std::path::PathBuf::from("/opt/yantrik/bin/yantrik-lock"))
}

/// Take the session lock, if it is not held already. `on_unlock` runs on the UI thread with the
/// secret that unlocked it.
pub fn engage(
    ui: slint::Weak<App>,
    greeting: String,
    secret: crate::lock::Secret,
    on_unlock: impl Fn(&App, &str, crate::lock::Secret) + Send + Clone + 'static,
) {
    WANTED.store(true, Ordering::SeqCst);
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    let bin = lock_bin();
    if !bin.exists() {
        tracing::warn!(path = %bin.display(), "No session-lock client; the shell's own screen is the lock");
        RUNNING.store(false, Ordering::SeqCst);
        return;
    }
    std::thread::spawn(move || loop {
        hold(&bin, &greeting, secret, &ui, &on_unlock);
        // A lock asked for while this thread was on its way out found RUNNING still set and left
        // the work to it: take it, rather than leave that lock without a compositor client.
        RUNNING.store(false, Ordering::SeqCst);
        if !WANTED.load(Ordering::SeqCst) || RUNNING.swap(true, Ordering::SeqCst) {
            break;
        }
    });
}

/// Keep a lock client running until the person is back in, or the compositor has no session lock.
fn hold(
    bin: &std::path::Path,
    greeting: &str,
    secret: crate::lock::Secret,
    ui: &slint::Weak<App>,
    on_unlock: &(impl Fn(&App, &str, crate::lock::Secret) + Send + Clone + 'static),
) {
    let mut starts: u32 = 0;
    loop {
        starts += 1;
        match run_once(bin, greeting, secret) {
            Outcome::Unlocked(pin, checked) => {
                // Released here as well as by the unlock itself: that runs later, on the UI thread,
                // and the caller must not read "still wanted" in between.
                released();
                let (ui, on_unlock) = (ui.clone(), on_unlock.clone());
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui.upgrade() {
                        on_unlock(&ui, &pin, checked);
                    }
                });
                return;
            }
            Outcome::Unsupported if LOCKED_ONCE.load(Ordering::SeqCst) => {
                // This compositor does have the protocol; it refused because a lock was still
                // held. Another client, as for any other ending.
                std::thread::sleep(std::time::Duration::from_secs(1));
                if !WANTED.load(Ordering::SeqCst) {
                    return;
                }
            }
            Outcome::Unsupported => {
                // The shell's own screen is the lock; its unlock releases it.
                tracing::warn!("This compositor has no session lock; the shell's own screen is the lock (#313)");
                released();
                return;
            }
            Outcome::Died(why) => {
                // The compositor may be holding the session locked with nobody to unlock it.
                // Giving up would leave the person locked out of their own machine, so another is
                // started, however many times it takes.
                if starts <= 3 || starts % 60 == 0 {
                    tracing::warn!(%why, starts, "The session-lock client ended without unlocking; starting it again");
                }
                std::thread::sleep(std::time::Duration::from_secs(u64::from(starts).min(MOST_PAUSE_SECS)));
                if !WANTED.load(Ordering::SeqCst) {
                    tracing::info!("The desktop was unlocked meanwhile; no lock client is needed");
                    return;
                }
            }
        }
    }
}

enum Outcome {
    Unlocked(String, crate::lock::Secret),
    Unsupported,
    Died(String),
}

fn run_once(bin: &std::path::Path, greeting: &str, secret: crate::lock::Secret) -> Outcome {
    // A socket pair, not pipes: a pipe can be reopened through /proc/<pid>/fd by any process of
    // the same user, which could then write `ok` to the client or answer for it; a socket cannot
    // be opened that way.
    let channel = UnixStream::pair().and_then(|(ours, theirs)| {
        let theirs_out = theirs.try_clone()?;
        Ok((ours, theirs, theirs_out))
    });
    let (ours, theirs, theirs_out) = match channel {
        Ok(c) => c,
        Err(e) => return Outcome::Died(format!("no channel to the lock client: {e}")),
    };
    let mut child = match Command::new(bin)
        .args(["--greeting", greeting, "--ask", secret.arg()])
        .stdin(Stdio::from(OwnedFd::from(theirs)))
        .stdout(Stdio::from(OwnedFd::from(theirs_out)))
        .stderr(Stdio::inherit())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return Outcome::Died(format!("could not start {}: {e}", bin.display())),
    };
    let mut to = match ours.try_clone() {
        Ok(to) => to,
        Err(e) => {
            let _ = child.kill();
            let _ = child.wait();
            return Outcome::Died(format!("no channel to the lock client: {e}"));
        }
    };
    let mut unlocked_with = None;
    // The channel failed while the client may still be running (and waiting for an answer that
    // will not come): it is ended, and another started, rather than waited on for ever.
    let mut lost = false;
    for line in BufReader::new(ours).lines() {
        let Ok(line) = line else {
            lost = true;
            break;
        };
        if line == "locked" {
            LOCKED_ONCE.store(true, Ordering::SeqCst);
            tracing::info!("The compositor locked the session");
            continue;
        }
        if let Some((reply, pin)) = answer(&line, secret, crate::lock::check_unlock) {
            if writeln!(to, "{reply}").and_then(|_| to.flush()).is_err() {
                lost = true;
                break;
            }
            if pin.is_some() {
                unlocked_with = pin;
            }
        }
    }
    // Ours closed before waiting: a client still reading sees the end, not a shell that hangs.
    drop(to);
    if lost {
        let _ = child.kill();
    }
    match child.wait().map(|s| s.code()) {
        Ok(Some(0)) if unlocked_with.is_some() => match unlocked_with {
            Some((pin, checked)) => Outcome::Unlocked(pin, checked),
            None => Outcome::Died("unlocked without a secret".into()),
        },
        Ok(Some(EXIT_UNSUPPORTED)) => Outcome::Unsupported,
        Ok(code) => Outcome::Died(format!("exited {code:?}")),
        Err(e) => Outcome::Died(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_line_is_answered_by_the_shells_own_check_and_nothing_else_is() {
        use crate::lock::{Secret, Verdict};
        use std::time::Duration;
        let check = |p: &str| if p == " correct horse " { Verdict::Open(Secret::Password) } else { Verdict::Wrong(Duration::ZERO) };
        let ask = Secret::Password;
        assert_eq!(
            answer("secret  correct horse ", ask, check),
            Some(("ok".to_string(), Some((" correct horse ".to_string(), Secret::Password)))),
            "the secret exactly as typed, spaces and all"
        );
        assert_eq!(answer("secret 0000", ask, check), Some(("no Wrong password".to_string(), None)));
        assert_eq!(answer("secret ", ask, check), Some(("no Wrong password".to_string(), None)), "an empty entry is a wrong one");
        assert_eq!(
            answer("secret x", ask, |_| Verdict::Wait(Duration::from_secs(8))),
            Some(("no Too many tries. Try again in 8 s".to_string(), None))
        );
        assert_eq!(answer("pin correct horse", ask, check), None, "the old protocol is not a question");
        assert_eq!(answer("locked", ask, check), None, "not a question");
        assert_eq!(answer("ok", ask, check), None, "the client cannot answer itself");
    }

    #[test]
    fn the_lock_client_sits_beside_the_shell() {
        assert!(lock_bin().ends_with("yantrik-lock"));
    }
}
