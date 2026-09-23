//! The lock: a state the desktop is in, not a screen it shows — and the screen PIN.
//!
//! # Locked is a state
//!
//! The lock screen used to be screen 3 and nothing more, so anything that could change the screen
//! could unlock the machine: `yos act shell open_lens` from any process running as the person took
//! a locked desktop straight to the conversation, with no card and no PIN (#203), and the login
//! screen after an install fell the same way. Every door that navigates — the control surface, the
//! command palette, a notification, the boot screen finishing, Ctrl+K — was a door past the lock.
//!
//! So the lock is held here, as [`Lock`], and the screen follows it rather than the other way
//! round. While it is held:
//!
//! - **the dispatch refuses** every act on the shell's surface that is not on [`WHILE_LOCKED`],
//!   with [`LOCKED`]. The rule is installed once, as the surface's hold ([`guard`]), in the one
//!   function every `app.act` crosses — so an action added tomorrow is refused while locked without
//!   anybody remembering to say so. It is an allow-list for that reason;
//! - **`describe shell` says `locked: true`** and keeps only [`DESCRIBED_WHILE_LOCKED`] — no
//!   conversation, no notifications, no window titles, no agents' tasks ([`redact`]);
//! - **the shell's own paths stay put**: `navigate` puts the lock's screen back ([`refuse_screen`]),
//!   the dock launches nothing, and app.slint mirrors the state (`locked-on`) so its keys, its
//!   boot handoff and its overlays stand down, with a `changed current-screen` handler as the
//!   back-stop for any path that sets the screen and forgets to ask.
//!
//! It is released by exactly two things ([`release`]): the lock screen's PIN check below, and the
//! login screen's password check (`wire::login`, the system's own `unix_chkpwd`). Nothing on the
//! control surface can release it; `lock_is_released_only_by_the_screens_that_check_a_secret` holds
//! the source to that.
//!
//! # The PIN
//!
//! The PIN here guards the shell's own canvas and nothing else. It is a comparison against
//! `~/.yantrik/lock_pin`, which is a file on the same disk as everything it is guarding, so it
//! keeps a passer-by out of an unattended desktop and stops at exactly that. It is not what
//! protects the credential vault: that is a passphrase the vault's key is *wrapped* under, so
//! there is nothing on disk to read and nothing to compare against. See `crate::vault_unlock`,
//! which the same screen drives.
//!
//! Creates a default PIN "0000" on first use, and the lock screen asks for a PIN, because that is
//! what this checks. The right secret for this screen is the account's own password, checked by
//! the system the way the login screen checks it (`wire::login::verify_password`); until the lock
//! screen does that, it must not call what it asks for a password.
//!
//! Idle lock triggers after a configurable timeout (default 5 minutes) — when something reports
//! the person idle, which on this build nothing does yet.

use std::path::PathBuf;
use std::sync::atomic::{AtomicI32, Ordering};

use yantrik_app_runtime::control::{App as ControlSurface, View};

use crate::App;

/// The lock screen's id in app.slint.
pub const LOCK_SCREEN: i32 = 3;
/// The login screen's id: the lock an installed machine boots into (`YANTRIK_START_SCREEN=32`).
pub const LOGIN_SCREEN: i32 = 32;
/// Where unlocking goes.
const DESKTOP_SCREEN: i32 = 1;

/// What the shell's dispatch answers, while the desktop is locked, to anything not on
/// [`WHILE_LOCKED`]. A client branches on `LOCKED:`; the words are for whoever reads them.
pub const LOCKED: &str = "LOCKED: the desktop is locked; nothing opens until the person unlocks it";

/// What the shell's surface still does while the desktop is locked, and why each is safe with
/// nobody at the machine. Everything else is refused with [`LOCKED`] — including an action added
/// after this list was written, which is the point of it being a list of what is allowed.
///
/// None of these navigates, draws anything, opens anything or reads anything the person said:
/// they take something away, or they are the plumbing a mind's harness needs so that work it had
/// already started keeps its footing until the person is back.
pub const WHILE_LOCKED: &[(&str, &str)] = &[
    ("lock", "locking a locked desktop takes nothing away; the screen that holds it stays"),
    (
        "start_service",
        "starts one of this machine's own services, which an app or a mind's tool asks for before it \
         can work; nothing is shown",
    ),
    (
        "approval_status",
        "a caller reading where its own request stands; the card itself is never drawn over the lock",
    ),
    (
        "consume_approval",
        "another surface spending a grant the person gave before the lock, exactly once; nothing is \
         shown and nothing new is allowed",
    ),
    (
        "record_unasked_action",
        "writes the audit line for something a mind did unasked; the person reads it after unlocking",
    ),
    (
        "agent_job",
        "an agent waiting on a command of its own that was already running; it reads that command \
         and nothing else, and draws nothing",
    ),
    ("agent_kill", "stops one of an agent's own commands, which only takes something away"),
];

/// What `describe shell` keeps while the desktop is locked: where the shell is, which build it is,
/// and the policy a mind's bridge reads before it acts (the ceiling, the mode, which mind is
/// answering) — nothing the person said, was sent, or had open. Every other key is left out,
/// including any added to `describe` after this list was written.
pub const DESCRIBED_WHILE_LOCKED: &[&str] = &[
    "screen",
    "screen_id",
    "version",
    "tool_permission",
    "mind_mode",
    "minds",
    "services",
    "companion_online",
    "clock",
    "date",
];

/// Whether `action` is one [`WHILE_LOCKED`] lets through.
pub fn allowed_while_locked(action: &str) -> bool {
    WHILE_LOCKED.iter().any(|(name, _)| *name == action)
}

/// Whether the desktop is locked, and on which screen.
///
/// One number: 0 when unlocked, otherwise the screen that holds the lock — [`LOCK_SCREEN`] or
/// [`LOGIN_SCREEN`]. Which screen matters: a desktop locked at the login screen is released by
/// the account's password, and the lock screen's PIN must not be a way round that.
pub struct Lock {
    on: AtomicI32,
}

impl Lock {
    pub const fn new() -> Lock {
        Lock { on: AtomicI32::new(0) }
    }

    pub fn is_locked(&self) -> bool {
        self.held_on().is_some()
    }

    /// The screen that holds the lock, or `None` when the desktop is unlocked.
    pub fn held_on(&self) -> Option<i32> {
        match self.on.load(Ordering::SeqCst) {
            0 => None,
            screen => Some(screen),
        }
    }

    /// Lock on `screen`, unless the desktop is locked already — then the screen that holds it
    /// keeps it. Answers the screen holding the lock now.
    fn engage(&self, screen: i32) -> i32 {
        match self.on.compare_exchange(0, screen, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => screen,
            Err(held) => held,
        }
    }

    /// Unlock, if and only if `screen` is the one holding the lock. `true` when it was released.
    fn release(&self, screen: i32) -> bool {
        self.on.compare_exchange(screen, 0, Ordering::SeqCst, Ordering::SeqCst).is_ok()
    }

    /// The rule the shell's dispatch asks before every act.
    pub fn check(&self, action: &str) -> Result<(), String> {
        if self.is_locked() && !allowed_while_locked(action) {
            return Err(LOCKED.to_string());
        }
        Ok(())
    }
}

impl Default for Lock {
    fn default() -> Self {
        Lock::new()
    }
}

/// The desktop's lock. One per shell.
static DESKTOP: Lock = Lock::new();

/// Whether the desktop is locked.
pub fn is_locked() -> bool {
    DESKTOP.is_locked()
}

/// Hold `surface` to `lock`: every act on it meets [`Lock::check`] in the dispatch, before its
/// arguments, the ceiling, the mode, any grant and the handler.
pub fn guard_with(surface: ControlSurface, lock: &'static Lock) -> ControlSurface {
    surface.hold(move |action| lock.check(action))
}

/// Hold the shell's surface to the desktop's lock. `control::publish` calls this, once.
pub fn guard(surface: ControlSurface) -> ControlSurface {
    guard_with(surface, &DESKTOP)
}

/// `describe shell` while locked: [`DESCRIBED_WHILE_LOCKED`] of `view` and nothing else, with
/// `locked: true` and the screen that holds the lock — read from the lock, not from the screen
/// property, so a path that set the screen a moment ago is not what gets reported.
pub fn redact(view: View, held_on: i32) -> View {
    let mut kept = serde_json::Map::new();
    for key in DESCRIBED_WHILE_LOCKED {
        if let Some(value) = view.state.get(*key) {
            kept.insert((*key).to_string(), value.clone());
        }
    }
    let screen = crate::control::screen_name(held_on);
    kept.insert("screen".into(), screen.into());
    kept.insert("screen_id".into(), held_on.into());
    kept.insert("locked".into(), true.into());
    View::new(format!(
        "Yantrik — locked, on the {screen} screen. Nothing opens, and nothing the person has open is \
         described, until they unlock it"
    ))
    .state(serde_json::Value::Object(kept))
}

/// The desktop's `describe`, as its lock allows it: whole when unlocked, [`redact`]ed when not.
pub fn describe(view: View) -> View {
    describe_with(view, &DESKTOP)
}

/// `view` as `lock` allows it.
pub fn describe_with(view: View, lock: &Lock) -> View {
    match lock.held_on() {
        Some(held) => redact(view, held),
        None => view.with("locked", false),
    }
}

/// Lock the desktop on `screen` — [`LOCK_SCREEN`], or [`LOGIN_SCREEN`] at boot. Every path that
/// locks comes here: the control surface's `lock`, Super+L, the power menu, the command palette,
/// the Lens, the idle timer and the login screen an installed machine starts on.
///
/// Locked already, it stays on the screen that holds it: `lock` while the login screen is up must
/// not swap the account's password for the lock screen's PIN.
pub fn engage(ui: &App, screen: i32) {
    let held = DESKTOP.engage(screen);
    // Before the screen goes dark, not after: the key is zeroed while this is still the person's
    // own action. A locked screen with the vault's key still in memory protects a screen.
    crate::vault_unlock::on_screen_lock();
    ui.set_locked_on(held);
    // What is drawn over every screen goes, so none of it sits on top of the lock.
    ui.set_lens_open(false);
    ui.set_app_grid_open(false);
    ui.set_command_palette_open(false);
    ui.set_ai_quick_switcher_open(false);
    ui.set_mind_menu_open(false);
    ui.set_quick_settings_open(false);
    ui.set_power_menu_open(false);
    crate::wire::toast::clear(ui);
    ui.set_current_screen(held);
    ui.set_lock_error("".into());
    ui.set_lock_date_text(crate::app_context::current_date_text().into());
    ui.set_lock_greeting(ui.get_greeting_text());
    tracing::info!(screen = crate::control::screen_name(held), "Desktop locked — the vault's key was zeroed with it");
    // In front of whatever app window was: Super+L pressed inside Notes locked a shell that sat
    // behind Notes. Off the UI thread, because wlrctl is a process.
    let _ = std::thread::Builder::new().name("lock-raise".into()).spawn(|| {
        if let Err(why) = crate::windows::raise_shell() {
            tracing::warn!(%why, "The desktop is locked, but the shell could not be brought in front");
        }
    });
}

/// Unlock the desktop held by `screen`, and go to the desktop.
///
/// For the lock screen's PIN check (`wire::callbacks`) and the login screen's password check
/// (`wire::login`), after the secret was checked, and for nothing else — a test reads the source
/// to keep it that way. `false`, and nothing changes, when `screen` is not the one holding the lock.
pub fn release(ui: &App, screen: i32) -> bool {
    if !DESKTOP.release(screen) {
        tracing::warn!(
            by = crate::control::screen_name(screen),
            held_on = ?DESKTOP.held_on(),
            "An unlock from a screen that does not hold the lock was ignored"
        );
        return false;
    }
    ui.set_locked_on(0);
    ui.set_current_screen(DESKTOP_SCREEN);
    ui.invoke_navigate(DESKTOP_SCREEN);
    true
}

/// For a path about to show `screen`: `true` when the desktop is locked and `screen` is not the
/// one holding the lock — the lock's screen has been put back, and the caller must do nothing.
pub fn refuse_screen(ui: &App, screen: i32) -> bool {
    match DESKTOP.held_on() {
        Some(held) if screen != held => {
            ui.set_current_screen(held);
            tracing::info!(
                asked = screen,
                held_on = crate::control::screen_name(held),
                "Not shown: the desktop is locked"
            );
            true
        }
        _ => false,
    }
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
}

/// Locked is a state (#203): what the dispatch lets through, what `describe` keeps, who may unlock.
#[cfg(test)]
mod locked_tests {
    use super::*;
    use std::path::Path;

    /// The shell's source files that publish actions — `control.rs` and every `control_*.rs`.
    pub(super) fn control_sources() -> Vec<std::path::PathBuf> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut found: Vec<_> = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("control") && n.ends_with(".rs"))
            })
            .collect();
        found.sort();
        found
    }

    /// Every action the shell publishes: the name in each `Action::new("…"` of those files.
    pub(super) fn published_actions() -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for path in control_sources() {
            let src = std::fs::read_to_string(&path).unwrap();
            for (at, _) in src.match_indices("Action::new(") {
                let rest = &src[at + "Action::new(".len()..];
                let Some(open) = rest.find('"') else { continue };
                if rest[..open].chars().any(|c| !c.is_whitespace()) {
                    continue;
                }
                let Some(close) = rest[open + 1..].find('"') else { continue };
                let name = rest[open + 1..open + 1 + close].to_string();
                if !out.contains(&name) {
                    out.push(name);
                }
            }
        }
        assert!(out.len() > 40, "the scan found only {out:?}; has the declaration style changed?");
        out
    }

    /// What navigates, shows something, opens something or reads what the person said. Each must
    /// be refused while locked — the doors #203 was reproduced with, and the ones like them.
    const OPENS_OR_SHOWS: &[&str] = &[
        "open_lens", "show_screen", "open_app", "show_app", "show_agent", "focus_window",
        "send_message", "read_message", "read_agent", "request_approval", "show_mind_audit",
        "files_open", "files_go", "files_enter", "editor_new", "installer_go_to", "new_agent",
        "hand_off", "minimise_window", "close_window", "set_mind_panel", "report_problem",
    ];

    fn locked_on(screen: i32) -> Lock {
        let lock = Lock::new();
        assert_eq!(lock.engage(screen), screen);
        lock
    }

    /// Every action the shell publishes is refused while the desktop is locked, in the one
    /// sentence, unless the allow-list names it — and once unlocked, the rule says nothing.
    #[test]
    fn a_locked_desktop_refuses_every_action_but_its_allow_list() {
        for screen in [LOCK_SCREEN, LOGIN_SCREEN] {
            let lock = locked_on(screen);
            for action in published_actions() {
                let verdict = lock.check(&action);
                if allowed_while_locked(&action) {
                    assert_eq!(verdict, Ok(()), "{action} is on the allow-list and was refused");
                } else {
                    assert_eq!(verdict, Err(LOCKED.to_string()), "{action} ran on a locked desktop");
                }
            }
            for action in OPENS_OR_SHOWS {
                assert_eq!(lock.check(action), Err(LOCKED.to_string()), "{action}, locked on {screen}");
            }
        }
        let open = Lock::new();
        for action in published_actions() {
            assert_eq!(open.check(&action), Ok(()), "{action}: an unlocked desktop holds nothing back");
        }
    }

    /// An allow-list, not a deny-list: an action added without a thought for the lock is refused
    /// while locked, because nobody put it on the list.
    #[test]
    fn an_action_nobody_thought_about_is_refused_while_locked() {
        let lock = locked_on(LOCK_SCREEN);
        for action in ["open_something_new", "show_the_person_everything", "", "LOCK", "lock "] {
            assert_eq!(lock.check(action), Err(LOCKED.to_string()), "{action:?}");
        }
        assert_eq!(Lock::new().check("open_something_new"), Ok(()));
    }

    /// The allow-list names actions the shell really publishes, says why each is safe, and holds
    /// nothing that navigates or shows.
    #[test]
    fn the_allow_list_is_short_real_and_shows_nothing() {
        let published = published_actions();
        for (name, why) in WHILE_LOCKED {
            assert!(published.iter().any(|p| p == name), "`{name}` is allowed while locked but the shell publishes no such action");
            assert!(why.len() > 20, "`{name}` is allowed while locked without saying why");
            assert!(!OPENS_OR_SHOWS.contains(name), "`{name}` opens or shows something and is allowed while locked");
        }
        assert!(allowed_while_locked("lock"), "locking has to work on a locked desktop");
        assert!(WHILE_LOCKED.len() <= 8, "the allow-list is growing: {WHILE_LOCKED:?}");
    }

    /// A lock is released only by the screen that holds it, and locking again keeps the holder:
    /// `lock` on the login screen must not swap the account's password for the screen PIN.
    #[test]
    fn only_the_screen_that_holds_the_lock_releases_it() {
        let lock = locked_on(LOGIN_SCREEN);
        assert_eq!(lock.engage(LOCK_SCREEN), LOGIN_SCREEN, "locking again moved the lock");
        assert!(!lock.release(LOCK_SCREEN), "the lock screen's PIN released the login screen's lock");
        assert_eq!(lock.held_on(), Some(LOGIN_SCREEN));
        assert!(lock.release(LOGIN_SCREEN));
        assert!(!lock.is_locked());
        assert_eq!(lock.check("open_lens"), Ok(()), "unlocked, it allows");
        assert!(!lock.release(LOGIN_SCREEN), "releasing what is not held is nothing");
    }

    /// A `describe shell` with everything in it, as the shell builds one.
    fn full_view() -> View {
        View::new("Yantrik — files at ~/secret-plans, 12 items, 3 windows open")
            .with("screen", "files")
            .with("screen_id", 8)
            .with("version", "v0.1.0-500")
            .with("conversation", serde_json::json!([{"index": 0, "role": "user", "text": "my bank PIN is 4821"}]))
            .with("notifications", serde_json::json!({"unread": 1, "recent": [{"title": "Dr. Rao: results are in"}]}))
            .with("windows", serde_json::json!([{"title": "Divorce papers — Editor", "app": "editor"}]))
            .with("agents", serde_json::json!([{"id": "pi", "task": "draft the resignation letter"}]))
            .with("agent_jobs", serde_json::json!([{"command": "cat ~/diary.txt"}]))
            .with("pending_approvals", serde_json::json!([{"action": "send_mail"}]))
            .with("mind_audit_recent", serde_json::json!([{"action": "delete_event"}]))
            .with("lens", serde_json::json!({"open": true, "text": "search my messages for"}))
            .with("files", serde_json::json!({"path": "~/secret-plans"}))
            .with("tool_permission", "sensitive")
            .with("mind_mode", serde_json::json!({"mode": "ask", "session_rules": []}))
            .with("minds", serde_json::json!([{"id": "builtin", "answering": true}]))
            .with("services", serde_json::json!([{"id": "weather", "status": "running"}]))
            .with("companion_online", true)
            .with("clock", "09:41")
            .with("a_field_added_next_month", "something personal")
    }

    /// Locked, `describe` says so and keeps only where the shell is and the policy a mind reads
    /// before acting: no conversation, notifications, window titles or agents' tasks — nor a key
    /// added to `describe` later — and a summary that names nothing the person has open.
    #[test]
    fn describe_while_locked_says_locked_and_leaves_out_everything_personal() {
        let lock = locked_on(LOCK_SCREEN);
        let view = describe_with(full_view(), &lock);
        let state = view.state.as_object().expect("an object");
        assert_eq!(state["locked"], true);
        assert_eq!(state["screen"], "lock", "the lock's screen, not the one a path set a moment ago");
        assert_eq!(state["screen_id"], LOCK_SCREEN);
        for gone in [
            "conversation", "notifications", "windows", "agents", "agent_jobs", "pending_approvals",
            "mind_audit_recent", "lens", "files", "a_field_added_next_month",
        ] {
            assert!(!state.contains_key(gone), "`{gone}` is described while locked: {state:?}");
        }
        for key in state.keys() {
            assert!(key == "locked" || DESCRIBED_WHILE_LOCKED.contains(&key.as_str()), "`{key}` is not on the list");
        }
        for kept in ["tool_permission", "mind_mode", "minds", "version"] {
            assert!(state.contains_key(kept), "`{kept}` is what a mind's bridge reads before it acts");
        }
        let text = serde_json::to_string(&view.state).unwrap() + &view.summary;
        for secret in ["4821", "Dr. Rao", "Divorce", "resignation", "diary", "secret-plans", "search my messages"] {
            assert!(!text.contains(secret), "`{secret}` reached a locked describe: {text}");
        }
        assert!(view.summary.contains("locked"), "{}", view.summary);

        let on_login = describe_with(full_view(), &locked_on(LOGIN_SCREEN));
        assert_eq!(on_login.state["screen"], "login");

        // Unlocked, everything, and `locked: false`.
        let open = describe_with(full_view(), &Lock::new());
        assert_eq!(open.state["locked"], false);
        assert_eq!(open.state["conversation"][0]["text"], "my bank PIN is 4821");
        assert_eq!(open.state["screen"], "files");
    }

    fn source(rel: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
    }

    /// Code, without comments or tests: a rule a comment merely mentions is not a rule.
    fn code(rel: &str) -> String {
        source(rel)
            .split("#[cfg(test)]")
            .next()
            .unwrap_or_default()
            .lines()
            .map(|l| l.split("//").next().unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn rust_sources(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                rust_sources(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

    /// Every file of this crate, as (path under src/, its code without comments or tests).
    fn crate_code() -> Vec<(String, String)> {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        rust_sources(&src, &mut files);
        files
            .into_iter()
            .map(|p| {
                let rel = p.strip_prefix(&src).unwrap().to_string_lossy().replace('\\', "/");
                let text = code(&format!("src/{rel}"));
                (rel, text)
            })
            .collect()
    }

    /// Only the two screens that check a secret release the lock: the lock screen after its PIN
    /// check, the login screen after the system's password check. Nothing on the control surface,
    /// and nothing else anywhere in the shell.
    #[test]
    fn lock_is_released_only_by_the_screens_that_check_a_secret() {
        let mut callers: Vec<String> = Vec::new();
        for (file, text) in crate_code() {
            if file == "lock.rs" {
                continue;
            }
            for _ in text.matches("lock::release(") {
                callers.push(file.clone());
            }
            assert!(!text.contains("DESKTOP.release") && !text.contains("set_locked_on("), "{file} reaches past `lock::release`");
        }
        callers.sort();
        assert_eq!(callers, ["wire/callbacks.rs", "wire/login.rs"], "who releases the desktop's lock");

        let pin = code("src/wire/callbacks.rs");
        let checked = pin.find("lock::check_pin(&pin)").expect("the lock screen checks the PIN");
        assert!(pin.find("lock::release(").unwrap() > checked, "released before the PIN was checked");
        let login = code("src/wire/login.rs");
        let verified = login.find("verify_password(&username, &password)").expect("the login screen checks the password");
        let agreed = login.find("if authenticated {").expect("and acts on the answer");
        let released = login.find("lock::release(").unwrap();
        assert!(released > verified && released > agreed, "released before the password was checked");
        // The lock's own `release` is the only place its state goes back to unlocked.
        assert_eq!(code("src/lock.rs").matches("DESKTOP.release(").count(), 1, "one way out");
    }

    /// Every way in goes through `engage`: nothing else in the shell puts the lock or login screen
    /// up by setting the screen — which is how the power menu and Super+L used to lock the screen
    /// and leave the vault's key in memory, and how a path could "lock" without the dispatch
    /// knowing.
    #[test]
    fn every_way_to_lock_goes_through_engage() {
        for (file, text) in crate_code() {
            if file == "lock.rs" {
                continue;
            }
            for setter in ["set_current_screen(3)", "set_current_screen(32)", "set_current_screen(lock::", "set_locked_on("] {
                assert!(!text.contains(setter), "{file} calls `{setter}` itself instead of `lock::engage`");
            }
        }
        let callbacks = code("src/wire/callbacks.rs");
        let handler = &callbacks[callbacks.find("on_lock_screen(").unwrap()..];
        let handler = &handler[..handler.len().min(400)];
        assert!(handler.contains("lock::engage(&ui, lock::LOCK_SCREEN)"), "{handler}");
        assert!(code("src/main.rs").contains("lock::engage(&ui, screen)"), "an installed machine starts locked");
    }

    /// The rule is at the shell's dispatch, installed once, on the one surface the shell serves —
    /// and `lock` is `safe` with a description, so Super+L locks without a card (#215).
    #[test]
    fn the_shell_surface_is_held_to_the_lock_and_lock_is_safe() {
        let all: String = control_sources()
            .iter()
            .map(|p| std::fs::read_to_string(p).unwrap().split("#[cfg(test)]").next().unwrap_or_default().to_string())
            .collect();
        let control = code("src/control.rs");
        assert_eq!(
            control.matches("crate::lock::guard(ControlSurface::new(\"shell\"))").count(),
            1,
            "the shell's surface is not held to the lock"
        );
        assert_eq!(all.matches("ControlSurface::new(").count(), 1, "a second surface in the shell would not be held");
        assert!(control.contains("crate::lock::describe(View::new(summary)"), "describe shell is not passed through the lock");

        let at = control.find("\"lock\",").expect("the shell publishes `lock`");
        let decl = &control[at..(at + 300).min(control.len())];
        assert!(decl.contains(".risk(\"safe\")"), "`lock` must be safe: locking only takes access away. {decl}");
        assert!(decl.contains("PIN"), "`lock` says how it is undone: {decl}");
    }

    /// The shell's own navigation stands down while locked: `navigate` puts the lock back first
    /// thing, the dock launches nothing, and app.slint mirrors the lock — its keys, its boot
    /// handoff, the overlays drawn over every screen, and a back-stop for any path that sets the
    /// screen without asking.
    #[test]
    fn the_shells_own_paths_stand_down_while_locked() {
        let navigate = code("src/wire/navigate.rs");
        let body = &navigate[navigate.find("ui.on_navigate(").unwrap()..];
        let guard = body.find("crate::lock::refuse_screen(&ui, screen)").expect("navigate asks the lock");
        assert!(guard < body.find("match screen").unwrap(), "navigate loads a screen before asking the lock");

        let dock = code("src/wire/dock.rs");
        let launch = &dock[dock.find("ui.on_launch_app(").unwrap()..];
        assert!(
            launch.find("crate::lock::is_locked()").unwrap() < launch.find("resolve(&app").unwrap(),
            "the dock launches before asking the lock"
        );

        let palette = code("src/wire/command_palette.rs");
        let selected = &palette[palette.find("on_command_palette_selected(").unwrap()..];
        assert!(selected.find("crate::lock::is_locked()").unwrap() < selected.find("screen_for(&action)").unwrap());

        let app = source("../yantrik-ui-slint/ui/app.slint");
        let flat: String = app.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(flat.contains("in property <int> locked-on: 0;"), "app.slint does not mirror the lock");
        assert!(
            flat.contains("changed current-screen => { if (root.locked-on != 0 && root.current-screen != root.locked-on) { root.current-screen = root.locked-on; } }"),
            "app.slint has no back-stop putting the lock's screen back"
        );
        let boot = &flat[flat.find("boot-complete => {").unwrap()..];
        assert!(
            boot.find("if root.locked-on == 0 {").unwrap() < boot.find("root.current-screen = 1;").unwrap(),
            "the boot handoff goes past the lock"
        );
        let keys = &flat[flat.find("capture-key-pressed(event) =>").unwrap()..];
        assert!(
            keys.find("if root.locked-on != 0 { return reject; }").unwrap() < keys.find("root.current-screen = 1;").unwrap(),
            "Ctrl+K goes past the lock"
        );
        for overlay in [
            "if toast-queue.length > 0 && root.locked-on == 0 : ToastBanner",
            "if voice-active && root.locked-on == 0 : VoiceOverlay",
            "if root.ai-quick-switcher-open && root.locked-on == 0 : Rectangle",
        ] {
            assert!(flat.contains(overlay), "drawn over the lock: {overlay}");
        }
    }

    /// The lock screen asks for what it checks: a PIN (lock.rs compares ~/.yantrik/lock_pin), not
    /// a password (#215).
    #[test]
    fn the_lock_screen_asks_for_what_it_checks() {
        let screen = source("../yantrik-ui-slint/ui/lock.slint");
        assert!(screen.contains("text: \"Enter your PIN to unlock\";"), "the lock screen's prompt");
        assert!(!screen.to_lowercase().contains("password to unlock"), "it checks a PIN and asks for a password");
    }
}

/// The rule, reached the way `yos` reaches it: a real socket, the shell's own dispatch
/// (`yantrik_app_runtime::control` — the call read, the grant, the hop to the UI thread,
/// `Registry::act`) on a stand-in for the UI thread, and a surface held by `guard_with` exactly as
/// `control::publish` holds the shell's. Every action the shell publishes is on it, as a stand-in
/// handler that records that it ran.
#[cfg(all(test, unix))]
mod socket_tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    use std::path::Path;
    use std::sync::{Mutex, OnceLock};
    use yantrik_app_runtime::control::{Action, Param};

    /// The socket surface's own lock, so no other test's state is this one's.
    static SOCKET_LOCK: Lock = Lock::new();
    /// Which stand-in handlers ran.
    static RAN: Mutex<Vec<String>> = Mutex::new(Vec::new());
    const ADDED_WITHOUT_A_THOUGHT: &str = "open_the_new_thing";

    fn runtime_dir() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("yantrik-lock-socket-{}", std::process::id()))
    }

    fn served() -> &'static str {
        static ADDRESS: OnceLock<String> = OnceLock::new();
        ADDRESS.get_or_init(|| {
            let mut names = super::locked_tests::published_actions();
            names.push(ADDED_WITHOUT_A_THOUGHT.to_string());
            yantrik_app_runtime::control::serve_on_a_standin(
                move || {
                    let mut surface = guard_with(ControlSurface::new("shell"), &SOCKET_LOCK).describe(|| {
                        describe_with(
                            View::new("Yantrik — desktop screen, 1 window open")
                                .with("screen", "desktop")
                                .with("conversation", serde_json::json!([{"role": "user", "text": "the thing I told nobody"}]))
                                .with("windows", serde_json::json!([{"title": "Private — Editor"}]))
                                .with("tool_permission", "sensitive"),
                            &SOCKET_LOCK,
                        )
                    });
                    for name in names {
                        // `safe`, so the machine's own ceiling and mode never decide these calls:
                        // what is under test is the lock, which comes before both.
                        let spec = Action::new(&name, "a stand-in for the shell's own").risk("safe");
                        let spec = if name == "show_screen" { spec.arg(Param::text("screen").optional()) } else { spec };
                        let ran = name.clone();
                        surface = surface.action(spec, move |_| {
                            RAN.lock().unwrap().push(ran.clone());
                            if ran == "lock" {
                                SOCKET_LOCK.engage(LOCK_SCREEN);
                            }
                            Ok(serde_json::json!({ "ran": ran }))
                        });
                    }
                    surface
                },
                &runtime_dir(),
            )
        })
    }

    fn call(method: &str, params: serde_json::Value) -> serde_json::Value {
        let mut socket = UnixStream::connect(served()).expect("connect");
        let request = serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
        socket.write_all(format!("{request}\n").as_bytes()).unwrap();
        let mut line = String::new();
        BufReader::new(socket).read_line(&mut line).unwrap();
        serde_json::from_str(&line).expect(&line)
    }

    fn act(action: &str, args: serde_json::Value) -> serde_json::Value {
        call("app.act", serde_json::json!({ "action": action, "args": args }))
    }

    fn ran(action: &str) -> bool {
        RAN.lock().unwrap().iter().any(|a| a == action)
    }

    /// `yos`, as a person's script or a mind's shell tool runs it, pointed at this socket. `None`
    /// when there is no python3 to run it with.
    ///
    /// `yos` writes to `app-shell.sock` only when a `yantrik-ui` binary is what listens there, and
    /// what listens here is this test binary — so it is added to what `yos` accepts, the way
    /// yos-selftest.py adds its own interpreter. Everything else is `yos` as shipped.
    fn yos(args: &[&str]) -> Option<(bool, String)> {
        const RUN: &str = "import importlib.machinery, importlib.util, sys\n\
            loader = importlib.machinery.SourceFileLoader('yos', sys.argv[1])\n\
            yos = importlib.util.module_from_spec(importlib.util.spec_from_loader('yos', loader))\n\
            loader.exec_module(yos)\n\
            yos.SHELL_BINARIES = ('yantrik-ui', sys.argv[2])\n\
            sys.argv = ['yos'] + sys.argv[3:]\n\
            yos.main()\n";
        let yos = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../deploy/yantrik-os/yos");
        let me = std::env::current_exe().ok()?;
        let me = me.file_name()?.to_string_lossy().to_string();
        let out = std::process::Command::new("python3")
            .arg("-c")
            .arg(RUN)
            .arg(&yos)
            .arg(&me)
            .args(args)
            .env("XDG_RUNTIME_DIR", runtime_dir())
            .output()
            .ok()?;
        Some((
            out.status.success(),
            format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)),
        ))
    }

    /// #203, end to end: lock through `yos`, then every door the issue named — and every other
    /// action the shell publishes, and one added without a thought — is refused over the socket
    /// with LOCKED before its handler, before its arguments and before any grant; `describe` says
    /// locked and carries nothing personal; unlocked, the same calls run.
    #[test]
    fn over_the_socket_a_locked_shell_refuses_what_opens_and_unlocking_lets_it_through() {
        served();
        let python = yos(&["ls"]).is_some();

        // Locked the way Super+L and release-check lock it.
        if python {
            let (ok, out) = yos(&["act", "shell", "lock", "--no-ask"]).unwrap();
            assert!(ok && out.contains("accepted: True"), "yos act shell lock --no-ask: {out}");
        } else {
            eprintln!("no python3: locking over the raw socket instead of through yos");
            let reply = act("lock", serde_json::json!({}));
            assert_eq!(reply["result"]["accepted"], true, "{reply}");
        }
        assert_eq!(SOCKET_LOCK.held_on(), Some(LOCK_SCREEN));

        let every = super::locked_tests::published_actions();
        for action in every.iter().map(String::as_str).chain([ADDED_WITHOUT_A_THOUGHT]) {
            let reply = act(action, serde_json::json!({}));
            if allowed_while_locked(action) {
                assert!(reply["error"].is_null(), "{action} is allowed while locked: {reply}");
            } else {
                assert_eq!(reply["error"]["message"], LOCKED, "{action}: {reply}");
                assert_eq!(reply["error"]["code"], -32602, "a policy answer, not a transport fault");
                assert!(!ran(action), "{action} was refused and its handler ran anyway");
            }
        }
        // Before its arguments: a call its arguments would refuse is refused for the lock.
        let reply = act("show_screen", serde_json::json!({"screen": "desktop", "not_an_argument": 1}));
        assert_eq!(reply["error"]["message"], LOCKED, "{reply}");
        // Before any grant: a call carrying one is refused for the lock, not for the grant.
        let reply = call("app.act", serde_json::json!({"action": "open_lens", "args": {}, "grant": "made-up"}));
        assert_eq!(reply["error"]["message"], LOCKED, "{reply}");

        let described = call("app.describe", serde_json::json!({}));
        let state = &described["result"]["state"];
        assert_eq!(state["locked"], true, "{described}");
        assert_eq!(state["screen"], "lock", "{described}");
        let text = described.to_string();
        assert!(!text.contains("the thing I told nobody") && !text.contains("Private — Editor"), "{text}");

        if python {
            let (ok, out) = yos(&["act", "shell", "open_lens"]).unwrap();
            assert!(!ok && out.contains(LOCKED), "yos act shell open_lens on a locked desktop: {out}");
            let (ok, out) = yos(&["act", "shell", "show_screen", "screen=desktop"]).unwrap();
            assert!(!ok && out.contains(LOCKED), "yos act shell show_screen screen=desktop: {out}");
            let (_, out) = yos(&["describe", "shell"]).unwrap();
            assert!(out.contains("locked") && !out.contains("the thing I told nobody"), "{out}");
        }
        assert!(!ran("open_lens") && !ran("show_screen"), "a door past the lock was opened");

        // The lock screen's own check releases it — here, the state it releases.
        assert!(SOCKET_LOCK.release(LOCK_SCREEN));
        let reply = act("open_lens", serde_json::json!({}));
        assert_eq!(reply["result"]["accepted"], true, "unlocked, it runs: {reply}");
        assert!(ran("open_lens"));
        let reply = act(ADDED_WITHOUT_A_THOUGHT, serde_json::json!({}));
        assert_eq!(reply["result"]["accepted"], true, "{reply}");
        let described = call("app.describe", serde_json::json!({}));
        assert_eq!(described["result"]["state"]["locked"], false, "{described}");
        assert!(described.to_string().contains("the thing I told nobody"), "{described}");
    }
}
