//! Lock screen — what unlocks it, and idle lock management.
//!
//! The screen unlocks with the account's login password (#414). The PIN it used to take lived in
//! `~/.yantrik/lock_pin`, a file anything running as this user — any mind with a shell — could
//! read, or overwrite with a PIN of its own choosing. The password lives in /etc/shadow, which
//! nothing running as this user can read or change, and it is checked the way the login screen
//! checks it (`unix_chkpwd`). It is also the secret the vault's key is wrapped under, so coming
//! back to the machine opens both with one entry.
//!
//! Only an account with no usable password keeps the PIN: there, the password is nothing to
//! check, and asking for it would make the lock a lockout.
//!
//! Wrong answers slow down (`delay_after`): the lock is the one place anybody at the keyboard may
//! guess without limit.

use std::path::PathBuf;
use std::time::Duration;

/// The secret the lock asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Secret {
    /// The account's login password — every account that has one.
    Password,
    /// `~/.yantrik/lock_pin` — only an account with no usable password.
    Pin,
}

impl Secret {
    /// What the lock screen says above its field.
    pub fn prompt(self) -> &'static str {
        match self {
            Secret::Password => "Enter your password to unlock",
            Secret::Pin => "Enter PIN to unlock",
        }
    }

    /// What it says after a wrong entry.
    pub fn wrong(self) -> &'static str {
        match self {
            Secret::Password => "Wrong password",
            Secret::Pin => "Wrong PIN",
        }
    }

    /// How the session-lock client is told which to ask for.
    pub fn arg(self) -> &'static str {
        match self {
            Secret::Password => "password",
            Secret::Pin => "pin",
        }
    }
}

/// The account this desktop runs as, from the system rather than the environment.
fn account_name() -> Option<String> {
    let out = std::process::Command::new("/usr/bin/id").arg("-un").output().ok()?;
    let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !name.is_empty()).then_some(name)
}

/// Whether `passwd -S` says, in so many words, that the account has no password: `NP` (none) or
/// `L`/`LK` (locked). Only that answer earns the PIN. Anything else, including no answer at all,
/// is a password (a same-uid process can make `passwd` fail to start, by exhausting the process
/// limit, and a failure must not hand it the PIN it can write).
fn says_no_password(status_line: &str) -> bool {
    matches!(status_line.split_whitespace().nth(1), Some("NP" | "L" | "LK"))
}

/// Whether `passwd -S` says, in so many words, that this account has no password. Only that
/// answer lets the desktop start open. Not knowing starts it locked: a same-uid process can make
/// `passwd` fail to start (the process limit), and that must not open the desktop. Locked is not
/// a lockout either way: every attempt decides the secret again, so an account that really has
/// no password gets the PIN as soon as `passwd` answers.
pub fn account_says_no_password() -> bool {
    std::process::Command::new("/usr/bin/passwd")
        .arg("-S")
        .output()
        .is_ok_and(|out| out.status.success() && says_no_password(&String::from_utf8_lossy(&out.stdout)))
}

/// Whether this is the live image rather than an installed machine: its password is published,
/// so a lock at boot would keep nobody out and everybody guessing.
pub fn live_session() -> bool {
    std::path::Path::new("/run/live").exists()
        || std::fs::read_to_string("/proc/cmdline").is_ok_and(|c| c.split_whitespace().any(|w| w == "boot=live"))
}

/// Which secret unlocks this account's screen. Asked at every lock and every attempt: an
/// installer or a person may set the password while the desktop runs.
pub fn secret_for_this_account() -> Secret {
    match std::process::Command::new("/usr/bin/passwd").arg("-S").output() {
        Ok(out) if out.status.success() && says_no_password(&String::from_utf8_lossy(&out.stdout)) => {
            tracing::warn!("This account has no usable password; the lock asks for the PIN (#414)");
            Secret::Pin
        }
        Ok(_) => Secret::Password,
        Err(e) => {
            tracing::warn!(error = %e, "Cannot ask passwd about this account; the lock asks for the password");
            Secret::Password
        }
    }
}

/// How long the next attempt waits after the `failures`-th wrong one in a row: nothing for the
/// first three typos, then 1, 2, 4 … seconds, at most 30.
pub fn delay_after(failures: u32) -> Duration {
    if failures < 3 {
        return Duration::ZERO;
    }
    Duration::from_secs((1u64 << (failures - 3).min(5)).min(30))
}

/// What an attempt to unlock came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The secret was right, and which secret it was: the one actually checked, not the one the
    /// screen was showing a prompt for, since the account can change between the two.
    Open(Secret),
    /// It was wrong; the next attempt is refused unchecked for this long (zero: straight away).
    Wrong(Duration),
    /// Too soon after a wrong one: not checked at all.
    Wait(Duration),
}

impl Verdict {
    /// What the lock screen says, for anything but `Open`.
    pub fn message(self, secret: Secret) -> String {
        match self {
            Verdict::Open(_) => String::new(),
            Verdict::Wrong(wait) if wait.is_zero() => secret.wrong().to_string(),
            Verdict::Wrong(wait) => format!("{}. Try again in {} s", secret.wrong(), secs(wait)),
            Verdict::Wait(wait) => format!("Too many tries. Try again in {} s", secs(wait)),
        }
    }
}

fn secs(d: Duration) -> u64 {
    d.as_secs() + u64::from(d.subsec_nanos() > 0)
}

/// Wrong answers in a row across both lock screens, and when the next may be checked.
#[derive(Debug, Default)]
pub struct Tries {
    failures: u32,
    not_before: Option<std::time::Instant>,
}

impl Tries {
    /// One attempt at `now`. `check` runs only when an attempt is due, so a burst of guesses
    /// costs the guesser the wait, not the machine a check each.
    pub fn attempt(&mut self, now: std::time::Instant, check: impl FnOnce() -> Option<Secret>) -> Verdict {
        if let Some(due) = self.not_before {
            if now < due {
                return Verdict::Wait(due - now);
            }
        }
        if let Some(checked) = check() {
            *self = Tries::default();
            return Verdict::Open(checked);
        }
        self.failures += 1;
        let wait = delay_after(self.failures);
        self.not_before = (!wait.is_zero()).then(|| now + wait);
        Verdict::Wrong(wait)
    }
}

/// One ledger for both lock screens, and one check at a time: the shell's own screen takes an
/// attempt on a thread per Enter, and parallel checks would make the slow-down a queue.
static TRIES: std::sync::Mutex<Tries> = std::sync::Mutex::new(Tries { failures: 0, not_before: None });

/// Whether `input` unlocks the screen. Runs `unix_chkpwd`, so it is called off the UI thread, and
/// it never sleeps: an attempt that comes too soon is refused unchecked, with how long to wait.
pub fn check_unlock(input: &str) -> Verdict {
    let mut tries = TRIES.lock().unwrap_or_else(|e| e.into_inner());
    let verdict = tries.attempt(std::time::Instant::now(), || match secret_for_this_account() {
        Secret::Password => account_name()
            .is_some_and(|user| crate::wire::login::verify_password(&user, input))
            .then_some(Secret::Password),
        Secret::Pin => check_pin(input).then_some(Secret::Pin),
    });
    if !matches!(verdict, Verdict::Open(_)) {
        tracing::info!(?verdict, failures = tries.failures, "Unlock refused");
    }
    verdict
}

/// Default idle lock timeout in seconds (5 minutes).
pub const DEFAULT_IDLE_LOCK_SECS: u64 = 300;

/// Path to the PIN file.
pub fn pin_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
    PathBuf::from(home).join(".yantrik/lock_pin")
}

/// Ensure the PIN file exists. Creates with default "0000" if missing.
pub fn ensure_pin_file() {
    let path = pin_path();
    if path.exists() {
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if std::fs::write(&path, "0000").is_err() {
        return;
    }
    // Under the default umask this file came out 0644 — readable by every other account on the
    // machine. It is a weak secret already; there was no reason to publish it as well.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    tracing::info!("Created default lock PIN at {}", path.display());
}

/// Check if the given input matches the stored PIN.
/// Returns true if authentication succeeds.
///
/// This used to end with `Err(_) => true` — "fail-open for dev". On a shipped machine that made
/// `rm ~/.yantrik/lock_pin` the whole bypass: delete one file the locked-out person can reach from
/// any other tty and every PIN is accepted. A lock with a documented way past it is a screensaver.
///
/// A missing file is not treated as an error, because it has an obvious right answer: put the
/// default back and check against that. Anything else — unreadable, a directory, an I/O failure —
/// refuses, because at that point the machine does not know what the PIN is and "I do not know"
/// must not resolve to "come in".
pub fn check_pin(input: &str) -> bool {
    let path = pin_path();
    match std::fs::read_to_string(&path) {
        Ok(stored) => stored.trim() == input.trim(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::warn!("Lock PIN file is missing — restoring the default and checking against it");
            ensure_pin_file();
            std::fs::read_to_string(&path)
                .map(|stored| stored.trim() == input.trim())
                .unwrap_or(false)
        }
        Err(e) => {
            tracing::warn!(error = %e, "Cannot read the lock PIN file — refusing to unlock");
            false
        }
    }
}

#[cfg(test)]
mod lock_tests {
    use super::*;

    /// `HOME` is process-wide, so these take turns.
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct TempHome {
        dir: PathBuf,
        previous: Option<String>,
    }

    impl TempHome {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("yantrik-lock-{}-{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let previous = std::env::var("HOME").ok();
            std::env::set_var("HOME", &dir);
            TempHome { dir, previous }
        }
    }

    impl Drop for TempHome {
        fn drop(&mut self) {
            match &self.previous {
                Some(home) => std::env::set_var("HOME", home),
                None => std::env::remove_var("HOME"),
            }
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn a_missing_pin_file_does_not_unlock_the_screen() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let _home = TempHome::new("missing");

        // Nothing created yet: this is the state `rm ~/.yantrik/lock_pin` produces.
        assert!(!pin_path().exists());
        assert!(
            !check_pin("whatever"),
            "deleting the PIN file must not be a way past the lock screen"
        );
        // And it put the default back, so the person is not locked out either.
        assert!(pin_path().exists());
        assert!(check_pin("0000"));
    }

    #[test]
    fn the_pin_file_is_not_readable_by_anyone_else() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let _home = TempHome::new("mode");
        ensure_pin_file();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(pin_path()).unwrap().permissions().mode() & 0o777;
            assert_eq!(
                mode, 0o600,
                "the lock PIN was written {mode:o} — every account on the machine could read it"
            );
        }
    }

    #[test]
    fn the_right_pin_unlocks_and_the_wrong_one_does_not() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let _home = TempHome::new("compare");
        ensure_pin_file();
        std::fs::write(pin_path(), "4821\n").unwrap();

        assert!(check_pin("4821"));
        assert!(check_pin(" 4821 "), "a trailing space is a typo, not a different PIN");
        assert!(!check_pin("4822"));
        assert!(!check_pin(""));
    }

    /// Both lock screens ask for the secret that is checked (#215): the label is not fixed in
    /// markup, it is the `Secret` the shell decided on, and the wrong-entry message names the same
    /// secret. A screen that said "PIN" while checking the password would send the person to the
    /// one secret that is not wanted.
    #[test]
    fn the_lock_screens_ask_for_the_secret_that_is_checked() {
        for secret in [Secret::Password, Secret::Pin] {
            let word = if secret == Secret::Password { "password" } else { "PIN" };
            assert!(secret.prompt().contains(word) && secret.wrong().contains(word), "{secret:?}");
        }
        for ui in ["../yantrik-ui-slint/ui/lock.slint", "../yantrik-lock/ui/lock.slint"] {
            let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(ui);
            let slint = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
            assert!(
                !slint.contains("\"Enter PIN to unlock\"") && !slint.contains("\"Enter your password"),
                "{ui} fixes the prompt in markup; it must show the secret the shell checks"
            );
        }
    }

    #[test]
    fn only_an_account_that_says_it_has_no_password_gets_the_pin() {
        assert!(!says_no_password("yantrik P 2026-09-17 0 99999 7 -1"));
        assert!(says_no_password("yantrik NP 2026-09-17 0 99999 7 -1"), "no password: the PIN");
        assert!(says_no_password("yantrik L 2026-09-17 0 99999 7 -1"), "locked: the PIN");
        assert!(!says_no_password(""), "no answer is not \"no password\": a failure must not earn the PIN");
        assert!(!says_no_password("passwd: something went wrong"));
    }

    #[test]
    fn a_guess_too_soon_is_not_checked_and_a_right_one_clears_the_count() {
        use std::time::Instant;
        let t0 = Instant::now();
        let mut tries = Tries::default();
        for _ in 0..2 {
            assert_eq!(tries.attempt(t0, || None), Verdict::Wrong(Duration::ZERO));
        }
        assert_eq!(tries.attempt(t0, || None), Verdict::Wrong(Duration::from_secs(1)));
        let mut checked = false;
        let soon = tries.attempt(t0 + Duration::from_millis(500), || {
            checked = true;
            Some(Secret::Password)
        });
        assert!(matches!(soon, Verdict::Wait(_)) && !checked, "inside the wait nothing is checked, not even a right one");
        assert_eq!(tries.attempt(t0 + Duration::from_secs(1), || None), Verdict::Wrong(Duration::from_secs(2)));
        assert_eq!(tries.attempt(t0 + Duration::from_secs(3), || Some(Secret::Password)), Verdict::Open(Secret::Password));
        assert_eq!(tries.attempt(t0 + Duration::from_secs(3), || None), Verdict::Wrong(Duration::ZERO), "counting starts again");
    }

    #[test]
    fn the_screen_says_how_long_to_wait() {
        assert_eq!(Verdict::Wrong(Duration::ZERO).message(Secret::Password), "Wrong password");
        assert_eq!(Verdict::Wrong(Duration::from_secs(4)).message(Secret::Pin), "Wrong PIN. Try again in 4 s");
        assert_eq!(Verdict::Wait(Duration::from_millis(1500)).message(Secret::Password), "Too many tries. Try again in 2 s");
    }

    #[test]
    fn wrong_entries_slow_down_after_three_and_never_past_half_a_minute() {
        let secs: Vec<u64> = (1..=12).map(|n| delay_after(n).as_secs()).collect();
        assert_eq!(secs, [0, 0, 1, 2, 4, 8, 16, 30, 30, 30, 30, 30]);
    }
}
