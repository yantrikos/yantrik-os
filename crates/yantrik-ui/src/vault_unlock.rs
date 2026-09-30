//! What locks the vault on this machine, and what does not.
//!
//! The engine can wrap the vault's data key under a passphrase with Argon2id
//! (`yantrikdb_core::vault::set_passphrase` / `unlock` / `lock`). With no passphrase set it does
//! not: the key stays in `memory.db` in the clear, next to the ciphertext it decrypts, which is
//! obfuscation with an extra step. That is the state every shipped machine is in, because nothing
//! in this OS ever called `set_passphrase` unless a mind happened to run the `vault_set_pin` tool.
//!
//! # Where a passphrase could come from, and where it actually does
//!
//! Two tiers, and the difference between them is not a setting — it is whether a person's typed
//! secret is ever in this process at all.
//!
//! **A session password.** The graphical login screen (screen 32) takes a username and a password
//! and checks them against `/etc/shadow`. On success the plaintext is in `wire::login`, verified,
//! for the length of one call. That is the ideal case and it needs no new secret: the password
//! *is* the passphrase. [`adopt`] takes it from there.
//!
//! **Nothing.** The live ISO autologins (`agetty --autologin yantrik`) and so does a machine
//! installed by `deploy/yantrik-os/yantrik-install.sh`, which asks for a password, writes it to
//! `/etc/shadow`, and then arranges for nothing ever to ask for it again. On those machines the
//! shell never sees a password, so there is nothing to derive a key from, and saying the vault is
//! protected would be a lie. [`status`] says so instead, in words, and the person can set a
//! separate vault passphrase — typed into a prompt this shell draws, which is the only way in.
//!
//! # What the screen lock is, and is not
//!
//! Locking the screen calls [`on_screen_lock`], which zeroes the key in memory. Unlocking offers
//! the typed secret to the vault. The screen lock's *own* check (`crate::lock`) is a comparison
//! against a file; the vault's is AEAD authentication against a wrapped key, and the two are not
//! the same strength. They are deliberately not joined: a wrong vault passphrase leaves the vault
//! locked and does not keep a person out of their own desktop, because a desktop that cannot be
//! unlocked is a worse failure than a vault that stays shut.
//!
//! # The one rule about the passphrase itself
//!
//! It is never stored, never logged, never put in an error, and never held past the call that
//! uses it. Every value this module returns is a fixed sentence chosen from a list, and
//! [`Outcome::message`] is where that list lives. `secret_never_reaches_a_message` is the test.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use rusqlite::Connection;
use yantrikdb_core::vault;

/// Whether a person's verified login password reached this process during this session.
///
/// Evidence, not configuration. Set once by `wire::login` when `/etc/shadow` has agreed, and read
/// by [`status`] to decide which of the two tiers this machine is actually in. An autologin
/// session never sets it, which is exactly the fact that has to reach the person.
static SAW_SESSION_PASSWORD: AtomicBool = AtomicBool::new(false);

/// The last thing the engine said about whether the key is wrapped.
///
/// Cached because `describe shell` is answered on the UI thread and `is_protected` needs the
/// database connection, which lives on the companion's worker thread. The value changes only when
/// [`adopt`] or [`rewrap`] writes, and both refresh it. `unlocked` needs no cache: the engine
/// keeps it in a process-global of its own and the answer is a load.
static PROTECTED: AtomicBool = AtomicBool::new(false);
/// Whether [`PROTECTED`] has ever been filled in, so "false" and "not asked yet" stay apart.
static PROTECTION_KNOWN: AtomicBool = AtomicBool::new(false);

/// The prompt the shell draws when a passphrase is wanted, or nothing.
static PROMPT: Mutex<Option<Prompt>> = Mutex::new(None);

/// The sentence a mind gets when it asks the vault for a credential and the vault is shut.
///
/// Defined where the tools that return it live, and named here so the shell and its tests read
/// the same constant rather than a copy that can drift out of step with it.
pub use yantrik_companion::tools::vault::LOCKED_ANSWER;

// ── Reaching the database ──

/// A vault operation for the companion's worker thread, which owns the database connection.
///
/// A command of its own rather than a `RunTool`, and that is the point rather than a detail.
/// `RunTool` is the door every mind and the whole control surface come through, and it carries
/// `serde_json::Value`: anything that can be expressed as a tool argument can be expressed by a
/// caller on the socket. A passphrase must not be expressible that way, so it travels in a typed
/// enum that nothing serialises, constructed in exactly two places — the login wiring and the
/// callback behind the shell's own prompt. `no_published_action_can_carry_a_passphrase` is the
/// test that keeps it that way.
#[derive(Debug, Clone)]
pub enum Op {
    /// Protect the vault with this passphrase, or open it with it.
    Adopt(String),
    /// Open a protected vault with this passphrase, and nothing else: an unprotected vault stays
    /// as it is. What the screen lock offers, because what unlocks a screen is not necessarily a
    /// secret anything should be wrapped under.
    Open(String),
    /// Re-wrap: the old passphrase has to open it before the new one replaces the wrapping.
    Rewrap { old: String, new: String },
    /// Read the state and refresh the cache. Carries no secret.
    Read,
}

/// What came back from the worker.
#[derive(Debug, Clone)]
pub struct Reply {
    /// What happened, for an [`Op`] that offered a passphrase. `None` for [`Op::Read`].
    pub outcome: Option<Outcome>,
    /// The vault's state afterwards.
    pub status: Status,
}

/// Run one operation against an open connection. Called on the thread that owns it.
pub fn run(conn: &Connection, op: Op) -> Reply {
    let outcome = match op {
        Op::Adopt(passphrase) => Some(adopt(conn, &passphrase)),
        Op::Open(passphrase) => Some(open_only(conn, &passphrase)),
        Op::Rewrap { old, new } => Some(rewrap(conn, &old, &new)),
        Op::Read => None,
    };
    Reply {
        outcome,
        status: status(conn),
    }
}

// ── The two tiers ──

/// Where this session's vault passphrase can come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// A password the system itself verified reached this process. Reuse it; derive nothing.
    SessionPassword,
    /// Nobody typed anything to get in here. There is no secret to reuse.
    NoSessionSecret,
}

impl Tier {
    /// Why the vault is unprotected on a machine in this tier, in the person's words.
    ///
    /// Only ever asked about an unprotected vault, which is why both arms read as an explanation
    /// rather than a state.
    pub fn why_unprotected(self) -> &'static str {
        match self {
            Tier::NoSessionSecret => {
                "this session signs in without a password, so there is nothing to lock the vault \
                 with"
            }
            Tier::SessionPassword => {
                "you signed in with a password, but the vault has not been locked with it yet — \
                 it will be on your next sign-in"
            }
        }
    }
}

/// Which tier this session is in, from what has actually happened in it.
pub fn tier() -> Tier {
    if SAW_SESSION_PASSWORD.load(Ordering::Relaxed) {
        Tier::SessionPassword
    } else {
        Tier::NoSessionSecret
    }
}

/// Record that a password this process can trust arrived. Called only from the login wiring.
pub fn note_session_password_seen() {
    SAW_SESSION_PASSWORD.store(true, Ordering::Relaxed);
}

// ── The state machine ──

/// What happened when a passphrase was offered to the vault.
///
/// No variant carries the passphrase, and no variant carries anything derived from it. See
/// [`Outcome::message`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The vault had no passphrase and now has this one. Everything already in it survived:
    /// `set_passphrase` wraps the existing key rather than minting a new one.
    Protected,
    /// The vault was already protected and this passphrase opened it.
    Unlocked,
    /// It did not open the vault. Nothing was written and nothing was damaged — a wrong
    /// passphrase fails at AEAD authentication, before anything is decrypted.
    Wrong,
    /// The vault could not be used at all, for a reason that is not a wrong passphrase.
    Unusable(String),
}

impl Outcome {
    /// Whether the vault is open for this process after this.
    pub fn opened(&self) -> bool {
        matches!(self, Outcome::Protected | Outcome::Unlocked)
    }

    /// One fixed sentence, safe to show a person and safe to put in a log.
    ///
    /// Fixed is the point. Every other string in this module is a literal chosen here, so there is
    /// exactly one place to check that no passphrase can reach a message — and `Unusable` is the
    /// only arm carrying anything from outside, which is why [`scrub`] exists.
    pub fn message(&self) -> String {
        match self {
            Outcome::Protected => {
                "The vault is now locked with this passphrase. Everything already stored in it is \
                 still there."
                    .into()
            }
            Outcome::Unlocked => "The vault is open.".into(),
            Outcome::Wrong => "That did not open the vault.".into(),
            Outcome::Unusable(why) => format!("The vault could not be opened: {why}"),
        }
    }
}

/// Offer a passphrase to the vault: protect it the first time, open it every time after.
///
/// This is the whole state machine and both tiers use it — a verified login password on one, a
/// passphrase typed into the shell's own prompt on the other. Which secret it is does not change
/// what happens to it, and it is not kept: the `&str` is borrowed for this call and the caller's
/// copy is the caller's problem.
pub fn adopt(conn: &Connection, passphrase: &str) -> Outcome {
    if passphrase.is_empty() {
        // Checked here as well as in the engine, because the engine's own message names the
        // function rather than the situation, and this one is read by a person.
        return Outcome::Unusable("a vault passphrase cannot be empty".into());
    }

    let protected = vault::is_protected(conn);
    remember_protection(protected);

    let outcome = if protected {
        match vault::unlock(conn, passphrase) {
            Ok(_) => Outcome::Unlocked,
            Err(e) => classify(e, passphrase),
        }
    } else {
        // Migration in place. The key already in the file is kept and wrapped, so entries stored
        // before this moment stay readable; what goes away is the copy of the key in the clear.
        match vault::set_passphrase(conn, passphrase) {
            Ok(()) => Outcome::Protected,
            Err(e) => Outcome::Unusable(scrub(e.to_string(), passphrase)),
        }
    };

    if outcome.opened() {
        remember_protection(true);
    }
    outcome
}

/// Open a protected vault with `passphrase`; never protect an unprotected one.
pub fn open_only(conn: &Connection, passphrase: &str) -> Outcome {
    let protected = vault::is_protected(conn);
    remember_protection(protected);
    if !protected {
        return Outcome::Unusable("the vault is not protected; there is nothing to open".into());
    }
    adopt(conn, passphrase)
}

/// Re-wrap the vault under a new passphrase, when the old one still opens it.
///
/// The password-change path. Order matters and it is the cautious one: the old passphrase has to
/// produce the key before the new one is allowed to wrap anything, so a mistyped current password
/// cannot replace the wrapping of a vault whose contents would then be unreachable.
pub fn rewrap(conn: &Connection, old: &str, new: &str) -> Outcome {
    if new.is_empty() {
        return Outcome::Unusable("a vault passphrase cannot be empty".into());
    }
    if vault::is_protected(conn) {
        match vault::unlock(conn, old) {
            Ok(_) => {}
            Err(e) => return classify(e, old),
        }
    }
    // Unlocked (or never protected), so `set_passphrase` keeps the key in force and re-wraps it.
    match vault::set_passphrase(conn, new) {
        Ok(()) => {
            remember_protection(true);
            Outcome::Protected
        }
        Err(e) => Outcome::Unusable(scrub(e.to_string(), new)),
    }
}

/// Forget the key. Called when the screen locks, and on nothing else.
///
/// The engine overwrites the bytes rather than dropping them. After this the vault cannot be read
/// again in this process without the passphrase, which is the entire reason the screen lock is
/// worth wiring to it: a locked screen with the key still in memory protects a screen.
pub fn on_screen_lock() {
    vault::lock();
}

/// The engine's failure, sorted into "that was the wrong passphrase" and everything else.
///
/// The engine maps an AEAD authentication failure — the only thing a wrong passphrase can
/// produce — to the literal `wrong passphrase`, and uses distinct wording for every structural
/// problem. Matching on it is not elegant; the alternative is reporting a corrupted `kdf_salt` row
/// as a typo and sending the person to retype a passphrase that was right the first time.
fn classify(e: yantrikdb_core::YantrikDbError, secret: &str) -> Outcome {
    let text = e.to_string();
    if text.contains("wrong passphrase") {
        Outcome::Wrong
    } else {
        Outcome::Unusable(scrub(text, secret))
    }
}

/// Take the secret out of a message that came from somewhere this module does not control.
///
/// Belt and braces: no error the engine produces today embeds the passphrase, and this module
/// cannot promise the engine never will. The engine is a pinned git dependency that moves, and
/// the day it starts saying `argon2: invalid input "hunter2"` this is what stands between that
/// and the log. `secret_never_reaches_a_message` is the test that keeps it honest.
fn scrub(message: String, secret: &str) -> String {
    if secret.is_empty() || !message.contains(secret) {
        return message;
    }
    message.replace(secret, "<redacted>")
}

// ── What the shell reports about itself ──

/// The vault, as `describe shell` and Settings report it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    /// Whether the data key is wrapped under a passphrase. False means it is in the file.
    pub protected: bool,
    /// Whether this process is currently holding the unwrapped key.
    pub unlocked: bool,
    /// Why it is unprotected, when it is. `None` on a protected vault, because there is nothing
    /// to explain.
    pub why: Option<String>,
}

impl Status {
    /// The same three facts as JSON, for `describe shell`.
    pub fn to_json(&self) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        map.insert("protected".into(), serde_json::Value::Bool(self.protected));
        map.insert("unlocked".into(), serde_json::Value::Bool(self.unlocked));
        if let Some(why) = &self.why {
            map.insert("why".into(), serde_json::Value::String(why.clone()));
        }
        serde_json::Value::Object(map)
    }
}

/// The vault's state, from the database.
pub fn status(conn: &Connection) -> Status {
    let protected = vault::is_protected(conn);
    remember_protection(protected);
    status_from(protected, vault::is_unlocked(), tier())
}

/// The vault's state without a database connection, from what was last read.
///
/// `describe shell` is answered on the UI thread and the connection lives on the companion's
/// worker. Blocking one on the other to answer a three-field question would make `describe` as
/// slow as whatever the companion is in the middle of, which is sometimes a language model.
pub fn cached_status() -> Status {
    status_from(
        PROTECTED.load(Ordering::Relaxed),
        vault::is_unlocked(),
        tier(),
    )
}

/// Whether the cache has ever been filled in. False means no vault read has happened yet.
pub fn protection_known() -> bool {
    PROTECTION_KNOWN.load(Ordering::Relaxed)
}

fn status_from(protected: bool, unlocked: bool, tier: Tier) -> Status {
    Status {
        protected,
        unlocked,
        why: if protected {
            None
        } else {
            Some(tier.why_unprotected().to_string())
        },
    }
}

fn remember_protection(protected: bool) {
    PROTECTED.store(protected, Ordering::Relaxed);
    PROTECTION_KNOWN.store(true, Ordering::Relaxed);
}

// ── The prompt only a person can answer ──

/// A request for the passphrase, waiting on the screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    /// Why it went up, in the person's words. Never contains anything a caller supplied verbatim.
    pub reason: String,
    /// What went wrong with the last attempt, or empty. One of [`Outcome::message`]'s sentences.
    pub error: String,
    /// Whether answering it will protect the vault for the first time rather than open it.
    pub first_time: bool,
}

/// Put the unlock prompt on the screen, or leave the one that is already there.
///
/// Idempotent on purpose: a mind retrying `vault_get` three times must not stack three prompts,
/// and the reason of the first one is the one the person started reading.
pub fn raise(reason: impl Into<String>, first_time: bool) {
    let Ok(mut guard) = PROMPT.lock() else { return };
    if guard.is_none() {
        *guard = Some(Prompt {
            reason: reason.into(),
            error: String::new(),
            first_time,
        });
    }
}

/// The prompt currently on the screen, if any.
pub fn pending() -> Option<Prompt> {
    PROMPT.lock().ok().and_then(|g| g.clone())
}

/// Take the prompt down.
pub fn dismiss() {
    if let Ok(mut guard) = PROMPT.lock() {
        *guard = None;
    }
}

/// Leave the prompt up and say what was wrong with the last answer.
pub fn set_prompt_error(message: impl Into<String>) {
    if let Ok(mut guard) = PROMPT.lock() {
        if let Some(prompt) = guard.as_mut() {
            prompt.error = message.into();
        }
    }
}

/// Reset every process-global this module owns. Tests only.
///
/// The engine's unlocked key and this module's caches are process-wide, and `cargo test` runs on
/// threads of one process, so a test that does not start from a known state is a test that passes
/// depending on what ran before it.
#[cfg(test)]
fn reset_for_test() {
    vault::lock();
    SAW_SESSION_PASSWORD.store(false, Ordering::Relaxed);
    PROTECTED.store(false, Ordering::Relaxed);
    PROTECTION_KNOWN.store(false, Ordering::Relaxed);
    dismiss();
}

#[cfg(test)]
mod vault_unlock_tests {
    use super::*;

    /// The process-globals below are shared by every test in this binary, so these tests take
    /// turns. Without it, one test's `vault::lock()` lands in the middle of another's unlock.
    static SERIAL: Mutex<()> = Mutex::new(());

    /// A real vault in a real SQLite file, with the real engine functions.
    ///
    /// Not a mock: the whole question this module answers is what `yantrikdb_core::vault` does to
    /// a database on disk when a passphrase is set on a vault that did not have one, and a mock
    /// would answer whatever it was written to answer.
    struct TempVault {
        conn: Connection,
        path: std::path::PathBuf,
    }

    impl TempVault {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "yantrik-vault-{}-{}-{tag}.db",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0)
            ));
            let _ = std::fs::remove_file(&path);
            let conn = Connection::open(&path).expect("open temp vault");
            vault::init_tables(&conn);
            TempVault { conn, path }
        }

        /// Put a credential in, so the migration has something to lose.
        fn store(&self, service: &str, password: &str) {
            let enc = vault::vault_encryption(&self.conn).expect("encryption");
            vault::store(&self.conn, &enc, service, "someone", password, None, None, None)
                .expect("store");
        }

        /// Read a credential back, or say why not.
        fn read(&self, service: &str) -> Result<String, String> {
            let enc = vault::vault_encryption(&self.conn).map_err(|e| e.to_string())?;
            let entries =
                vault::get(&self.conn, &enc, service).map_err(|e| e.to_string())?;
            entries
                .first()
                .map(|e| e.password.clone())
                .ok_or_else(|| "no such entry".to_string())
        }

        /// Whether the data key is still sitting in the file in the clear.
        fn key_is_in_the_file(&self) -> bool {
            self.conn
                .query_row(
                    "SELECT value FROM vault_security WHERE key = 'vault_dek'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .is_ok()
        }
    }

    impl Drop for TempVault {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    /// A shipped machine's vault: unprotected, with the key in the file beside the data.
    #[test]
    fn a_new_vault_is_unprotected_and_says_why() {
        let _lock = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        reset_for_test();
        let v = TempVault::new("fresh");
        v.store("github.com", "s3cret");

        assert!(
            v.key_is_in_the_file(),
            "the fixture is not reproducing the shipped state: the key should be in the file"
        );

        let status = status(&v.conn);
        assert!(!status.protected);
        assert!(!status.unlocked);
        assert_eq!(
            status.why.as_deref(),
            Some("this session signs in without a password, so there is nothing to lock the \
                  vault with"),
            "an autologin machine has to say what is missing, not just report a false"
        );
    }

    /// First login on an unprotected vault protects it, and the credentials survive.
    ///
    /// The migration is the part that could quietly destroy someone's passwords, so it is checked
    /// by reading one back rather than by trusting the return value.
    #[test]
    fn first_login_protects_the_vault_in_place() {
        let _lock = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        reset_for_test();
        let v = TempVault::new("first");
        v.store("github.com", "s3cret");

        assert_eq!(adopt(&v.conn, "correct horse battery"), Outcome::Protected);

        assert!(vault::is_protected(&v.conn));
        assert!(
            !v.key_is_in_the_file(),
            "set_passphrase left the plaintext key in the file — the wrapping protects nothing"
        );
        assert_eq!(
            v.read("github.com").as_deref(),
            Ok("s3cret"),
            "protecting the vault must not cost the person what was already in it"
        );
    }

    /// The second sign-in opens it rather than re-protecting it.
    #[test]
    fn a_later_login_unlocks_rather_than_rewrapping() {
        let _lock = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        reset_for_test();
        let v = TempVault::new("later");
        v.store("github.com", "s3cret");
        assert_eq!(adopt(&v.conn, "correct horse battery"), Outcome::Protected);

        on_screen_lock();
        assert!(!vault::is_unlocked());

        assert_eq!(adopt(&v.conn, "correct horse battery"), Outcome::Unlocked);
        assert!(vault::is_unlocked());
        assert_eq!(v.read("github.com").as_deref(), Ok("s3cret"));
    }

    /// Locking the screen zeroes the key, and the vault will not answer until it is opened again.
    #[test]
    fn screen_lock_closes_the_vault() {
        let _lock = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        reset_for_test();
        let v = TempVault::new("screenlock");
        v.store("github.com", "s3cret");
        adopt(&v.conn, "correct horse battery");

        on_screen_lock();

        assert!(!vault::is_unlocked());
        let refused = v.read("github.com").expect_err(
            "a locked vault must refuse, not quietly fall back to a key from somewhere else",
        );
        assert!(
            refused.contains("locked"),
            "the refusal has to say it is locked; it said {refused:?}"
        );
    }

    /// A wrong passphrase opens nothing and breaks nothing.
    ///
    /// The second half is the one worth having: a failed unwrap that had damaged the wrapped key
    /// would turn one typo into a permanently unreadable vault, and the person would find out
    /// later.
    #[test]
    fn a_wrong_passphrase_neither_opens_nor_corrupts() {
        let _lock = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        reset_for_test();
        let v = TempVault::new("wrong");
        v.store("github.com", "s3cret");
        adopt(&v.conn, "correct horse battery");
        on_screen_lock();

        assert_eq!(adopt(&v.conn, "wrong horse battery"), Outcome::Wrong);
        assert!(!vault::is_unlocked());
        assert!(
            vault::is_protected(&v.conn),
            "a failed attempt must leave the wrapping exactly where it was"
        );

        // And the right one still works afterwards.
        assert_eq!(adopt(&v.conn, "correct horse battery"), Outcome::Unlocked);
        assert_eq!(v.read("github.com").as_deref(), Ok("s3cret"));
    }

    /// Changing the account password re-wraps the vault under the new one.
    #[test]
    fn a_password_change_rewraps_and_the_old_one_stops_working() {
        let _lock = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        reset_for_test();
        let v = TempVault::new("rewrap");
        v.store("github.com", "s3cret");
        adopt(&v.conn, "old password");

        assert_eq!(rewrap(&v.conn, "old password", "new password"), Outcome::Protected);

        on_screen_lock();
        assert_eq!(adopt(&v.conn, "old password"), Outcome::Wrong);
        assert_eq!(adopt(&v.conn, "new password"), Outcome::Unlocked);
        assert_eq!(v.read("github.com").as_deref(), Ok("s3cret"));
    }

    /// A mistyped current password does not re-wrap anything.
    #[test]
    fn a_rewrap_with_the_wrong_current_password_changes_nothing() {
        let _lock = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        reset_for_test();
        let v = TempVault::new("rewrap-wrong");
        v.store("github.com", "s3cret");
        adopt(&v.conn, "old password");
        on_screen_lock();

        assert_eq!(rewrap(&v.conn, "not it", "new password"), Outcome::Wrong);

        on_screen_lock();
        assert_eq!(
            adopt(&v.conn, "old password"),
            Outcome::Unlocked,
            "the vault must still open with the passphrase it was wrapped under"
        );
    }

    /// An empty passphrase is refused rather than protecting the vault with nothing.
    #[test]
    fn an_empty_passphrase_is_not_a_passphrase() {
        let _lock = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        reset_for_test();
        let v = TempVault::new("empty");

        let outcome = adopt(&v.conn, "");
        assert!(matches!(outcome, Outcome::Unusable(_)));
        assert!(
            !vault::is_protected(&v.conn),
            "an empty passphrase must not count as protection"
        );
    }

    /// A session that signed in with a password says something different about an open vault.
    #[test]
    fn the_tier_comes_from_what_happened_not_from_configuration() {
        let _lock = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        reset_for_test();
        assert_eq!(tier(), Tier::NoSessionSecret);

        note_session_password_seen();
        assert_eq!(tier(), Tier::SessionPassword);
        assert!(
            Tier::SessionPassword.why_unprotected() != Tier::NoSessionSecret.why_unprotected(),
            "the two tiers must not give the person the same explanation"
        );
        reset_for_test();
    }

    /// A protected vault explains nothing, because there is nothing to explain.
    #[test]
    fn a_protected_vault_carries_no_excuse() {
        let _lock = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        reset_for_test();
        let v = TempVault::new("json");
        adopt(&v.conn, "correct horse battery");

        let status = status(&v.conn);
        assert!(status.protected);
        assert_eq!(status.why, None);

        let json = status.to_json();
        assert_eq!(json["protected"], serde_json::json!(true));
        assert!(
            json.get("why").is_none(),
            "a protected vault must not ship a `why` key for something to read as a problem"
        );
    }

    /// The sentinel: a passphrase must not survive into anything a person or a log can see.
    ///
    /// Every outcome of every path, with a passphrase distinctive enough that a substring search
    /// cannot miss it. This is the test that would have caught the engine one day deciding to
    /// quote its input back in an error.
    #[test]
    fn secret_never_reaches_a_message() {
        let _lock = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        reset_for_test();
        const SECRET: &str = "zQ7-tripwire-passphrase-9xV";

        let v = TempVault::new("sentinel");
        v.store("github.com", "s3cret");

        let mut seen: Vec<String> = Vec::new();

        // Protect, unlock, get it wrong, re-wrap, get the re-wrap wrong, and offer an empty one.
        let first = adopt(&v.conn, SECRET);
        seen.push(format!("{first:?}"));
        seen.push(first.message());

        on_screen_lock();
        let again = adopt(&v.conn, SECRET);
        seen.push(format!("{again:?}"));
        seen.push(again.message());

        on_screen_lock();
        let wrong = adopt(&v.conn, "definitely not it");
        seen.push(format!("{wrong:?}"));
        seen.push(wrong.message());

        let changed = rewrap(&v.conn, SECRET, SECRET);
        seen.push(format!("{changed:?}"));
        seen.push(changed.message());

        on_screen_lock();
        let bad_change = rewrap(&v.conn, "definitely not it", SECRET);
        seen.push(format!("{bad_change:?}"));
        seen.push(bad_change.message());

        let empty = adopt(&v.conn, "");
        seen.push(format!("{empty:?}"));
        seen.push(empty.message());

        // And everything the shell publishes about the vault.
        on_screen_lock();
        let status = status(&v.conn);
        seen.push(format!("{status:?}"));
        seen.push(status.to_json().to_string());
        seen.push(format!("{:?}", cached_status()));

        raise("a mind asked for a credential", false);
        set_prompt_error(Outcome::Wrong.message());
        seen.push(format!("{:?}", pending()));
        dismiss();

        seen.push(LOCKED_ANSWER.to_string());
        seen.push(Tier::SessionPassword.why_unprotected().to_string());
        seen.push(Tier::NoSessionSecret.why_unprotected().to_string());

        for text in &seen {
            assert!(
                !text.contains(SECRET),
                "a passphrase reached a string the shell can show or log: {text:?}"
            );
        }
        assert!(seen.len() > 15, "the sentinel stopped covering the paths it was written for");
    }

    /// `scrub` takes the secret out even when the message is otherwise useful.
    #[test]
    fn scrub_redacts_without_throwing_the_message_away() {
        let cleaned = scrub("argon2: rejected \"hunter2\" as too short".into(), "hunter2");
        assert!(!cleaned.contains("hunter2"));
        assert!(
            cleaned.contains("argon2"),
            "redaction must not cost the reason: {cleaned:?}"
        );
        // Nothing to do, nothing done.
        assert_eq!(scrub("plain".into(), ""), "plain");
        assert_eq!(scrub("plain".into(), "absent"), "plain");
    }

    /// The prompt does not stack, and it does not vanish on its own.
    #[test]
    fn the_prompt_is_raised_once_and_taken_down_deliberately() {
        let _lock = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        reset_for_test();

        assert!(pending().is_none());
        raise("a mind asked for a credential", false);
        raise("something else entirely", true);

        let prompt = pending().expect("a prompt should be up");
        assert_eq!(
            prompt.reason, "a mind asked for a credential",
            "a retrying caller must not be able to rewrite what the person is reading"
        );
        assert!(!prompt.first_time);

        dismiss();
        assert!(pending().is_none());
        reset_for_test();
    }
}
