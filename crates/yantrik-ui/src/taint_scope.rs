//! The taint's scope: which of the person's conversations a mind's private reads count against.
//!
//! `yos-mcp` refuses to put data into a page, and asks about every hand-off and command, once the
//! conversation it serves has read private state. It used to count that per bridge PROCESS, on
//! the reasoning that a harness spawns its bridge once per session. A gateway-shaped harness does
//! not: Hermes keeps one bridge across every "+ new conversation" in the Lens, so one calendar
//! read tainted every conversation after it, with no way back short of restarting the harness —
//! and the mind, told to "start a fresh session", told the person to start a new chat that would
//! change nothing (E.REF1, VM 541, 15 Oct 2026).
//!
//! So the desktop keeps the scope, because the desktop is where the person's decision is made.
//! One scope per harness — its `main` conversation, the person's chat with that mind in the Lens
//! — with an id, who began it, and what has been read in it. The rules:
//!
//! - **Only the person's press of + begins a scope** ([`begin_by_person`], called from the Lens's
//!   own `lens-new-chat` callback when [`is_press`] says it was a press). No action on the
//!   control surface begins one: a mind that runs as the person and presents no token cannot be
//!   told from the person on the socket (`control::agent_is_calling`'s own words), and Hermes's
//!   bridge carries no token. `new_chat` on the surface starts the conversation again for a
//!   client that is not the Lens, and leaves the taint scope where it was.
//! - **A restart is not a new scope.** #807 starts every shell in a new HAND-OVER conversation —
//!   nothing said before the restart is handed to the next mind spoken to — and that stays. The
//!   taint scope is kept on disk instead: the harness may resume its own session across the
//!   restart (Hermes keeps it on disk), and a mind that can restart the desktop must not untaint
//!   itself by doing so. A conversation that read private state stays tainted for its whole life.
//!   And a scope read back from disk is reported `begun: start`, never `person`: only a press in
//!   this run of the shell is the person's, so a forged record that says otherwise clears nothing.
//! - **Marks only add.** [`mark`] appends to the caller's current scope; nothing here removes a
//!   read except the press.
//! - **A record that cannot be read is tainted.** No file, or no entry for a harness, is a clean
//!   first scope (`begun: start`). A file that exists and cannot be read gives every scope made
//!   from it the read [`UNREADABLE`], until the person presses +.
//!
//! The bridge reads all of this through `control_taint_scope` (`taint_scope`, `taint_mark`) and
//! never from the mind: neither action takes a harness or an id.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

use serde::{Deserialize, Serialize};

/// The read a scope is born with when the record it should have come from could not be read.
pub const UNREADABLE: &str = "an unreadable record of this conversation";

/// The file, beside `agents/` in the person's data directory, where the mind account cannot write.
const FILE: &str = "taint-scopes.json";

/// A scope's reads are names (`os_describe`, `shell.read_screen`), not data, and a handful at
/// most; the bound is so a mind marking in a loop cannot grow the file without end.
const READS_KEPT: usize = 64;
/// And a read is a name, not a paragraph.
const READ_CHARS: usize = 80;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Begun {
    /// The person pressed + in the Lens.
    Person,
    /// The desktop made it because there was none: the first use of a harness, or no record.
    Start,
}

impl Begun {
    pub fn as_str(self) -> &'static str {
        match self {
            Begun::Person => "person",
            Begun::Start => "start",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scope {
    pub id: String,
    pub begun: Begun,
    #[serde(default)]
    pub read: Vec<String>,
}

#[derive(Default, Serialize, Deserialize)]
struct OnDisk {
    #[serde(default)]
    scopes: BTreeMap<String, Scope>,
}

/// Every harness's scope, and where they are kept.
pub struct Scopes {
    scopes: BTreeMap<String, Scope>,
    /// The record existed and could not be read: every scope made from here on starts tainted.
    record_lost: bool,
    /// `None` keeps them in memory only (tests).
    path: Option<PathBuf>,
    fresh_id: Box<dyn Fn() -> String + Send>,
}

impl Scopes {
    /// The scopes kept at `path`: none if there is no file, tainted if there is one that cannot
    /// be read.
    pub fn load(path: PathBuf) -> Scopes {
        let (scopes, record_lost) = match std::fs::read_to_string(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (BTreeMap::new(), false),
            Err(e) => {
                tracing::warn!(error = %e, path = %path.display(), "Taint scopes unreadable: every conversation starts tainted");
                (BTreeMap::new(), true)
            }
            Ok(text) => match serde_json::from_str::<OnDisk>(&text) {
                // Every scope read back is `begun: start`, whatever the file says. The press is
                // an event in this run of the shell, not a field in a file: a record forged to say
                // `person` (a mind that could write the data directory, then restart the desktop)
                // then gains nothing, since the bridge carries its reads over into a scope the
                // person did not begin (security review of #817, design point 3). The cost: a +
                // the bridge had not yet heard of before a restart carries the old reads over,
                // and the person presses + again.
                Ok(disk) => (
                    disk.scopes
                        .into_iter()
                        .filter(|(_, s)| valid_id(&s.id))
                        .map(|(harness, s)| (harness, Scope { begun: Begun::Start, ..s }))
                        .collect(),
                    false,
                ),
                Err(e) => {
                    tracing::warn!(error = %e, path = %path.display(), "Taint scopes unreadable: every conversation starts tainted");
                    (BTreeMap::new(), true)
                }
            },
        };
        Scopes { scopes, record_lost, path: Some(path), fresh_id: Box::new(fresh_id) }
    }

    /// Scopes in memory only, with ids from `fresh_id`. For tests.
    #[cfg(test)]
    pub fn in_memory(fresh_id: impl Fn() -> String + Send + 'static) -> Scopes {
        Scopes { scopes: BTreeMap::new(), record_lost: false, path: None, fresh_id: Box::new(fresh_id) }
    }

    /// `harness`'s scope as it stands, made (and kept) if it has none.
    pub fn current(&mut self, harness: &str) -> Scope {
        if let Some(scope) = self.scopes.get(harness) {
            return scope.clone();
        }
        let read = if self.record_lost { vec![UNREADABLE.to_string()] } else { Vec::new() };
        let scope = Scope { id: (self.fresh_id)(), begun: Begun::Start, read };
        self.scopes.insert(harness.to_string(), scope.clone());
        self.save();
        scope
    }

    /// The person pressed + in the Lens while `harness` was answering: a scope that has read
    /// nothing. The only way any read is ever taken away.
    pub fn begin_by_person(&mut self, harness: &str) -> Scope {
        let scope = Scope { id: (self.fresh_id)(), begun: Begun::Person, read: Vec::new() };
        self.scopes.insert(harness.to_string(), scope.clone());
        self.save();
        scope
    }

    /// `harness`'s conversation has read `what`. Only ever adds.
    pub fn mark(&mut self, harness: &str, what: &str) -> Scope {
        let what: String = what.trim().chars().take(READ_CHARS).collect();
        let mut scope = self.current(harness);
        if !what.is_empty() && !scope.read.contains(&what) && scope.read.len() < READS_KEPT {
            scope.read.push(what);
            self.scopes.insert(harness.to_string(), scope.clone());
            self.save();
        }
        scope
    }

    fn save(&self) {
        let Some(path) = &self.path else { return };
        if let Err(e) = write_atomically(path, &OnDisk { scopes: self.scopes.clone() }) {
            // Kept in memory still, so this run of the shell holds it; only a restart before the
            // next good write could lose it, and a lost record starts tainted only if the file
            // is there and unreadable. Said loudly, because that window is real.
            tracing::warn!(error = %e, path = %path.display(), "Taint scopes not saved");
        }
    }
}

fn write_atomically(path: &Path, disk: &OnDisk) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let partial = path.with_extension(format!("json.{}", std::process::id()));
    std::fs::write(&partial, serde_json::to_vec_pretty(disk).map_err(std::io::Error::other)?)?;
    std::fs::rename(&partial, path)
}

/// An id is 32 lowercase hex characters: what [`fresh_id`] makes, and what the bridge accepts.
pub fn valid_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// 128 bits from the kernel, as hex. Not a secret — any caller may read its scope — but unique,
/// so that no scope is ever mistaken for another.
fn fresh_id() -> String {
    let mut bytes = [0u8; 16];
    let read = std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut bytes));
    if read.is_err() {
        // Not expected on Linux. Unique is what matters here, and the clock and the pid give it.
        use sha2::{Digest, Sha256};
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        let digest = Sha256::digest(format!("{}:{}:{:?}", std::process::id(), now.as_nanos(), std::thread::current().id()));
        bytes.copy_from_slice(&digest[..16]);
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Where the scopes are kept: `$XDG_DATA_HOME/yantrik/taint-scopes.json`, beside `agents/`.
fn path() -> PathBuf {
    let agents = crate::agents::dir();
    agents.parent().map(Path::to_path_buf).unwrap_or(agents).join(FILE)
}

fn scopes() -> MutexGuard<'static, Scopes> {
    static SCOPES: OnceLock<Mutex<Scopes>> = OnceLock::new();
    SCOPES.get_or_init(|| Mutex::new(Scopes::load(path()))).lock().unwrap_or_else(|e| e.into_inner())
}

/// See [`Scopes::current`].
pub fn current(harness: &str) -> Scope {
    scopes().current(harness)
}

/// See [`Scopes::mark`].
pub fn mark(harness: &str, what: &str) -> Scope {
    scopes().mark(harness, what)
}

/// See [`Scopes::begin_by_person`]. Called from the Lens's + and from nowhere else; a test reads
/// the source to keep it that way.
pub fn begin_by_person(harness: &str) -> Scope {
    let scope = scopes().begin_by_person(harness);
    tracing::info!(harness, "New conversation: a taint scope that has read nothing");
    scope
}

// ── Whether the Lens's `lens-new-chat` callback was a press ─────────────────
//
// The Slint callback is one, whether the + was pressed or `new_chat` on the control surface
// invoked it. Only a press begins a scope, so the surface's call says it is not one, and a call
// made while a control dispatch is installed on the thread (a caller, a token) is not one either.

thread_local! {
    static NOT_A_PRESS: Cell<u32> = const { Cell::new(0) };
}

/// Held around a programmatic `invoke_lens_new_chat`: the conversation starts again, and the
/// taint scope does not.
#[must_use = "the call is a press again when this guard is dropped"]
pub struct NotAPress(());

impl NotAPress {
    pub fn enter() -> NotAPress {
        NOT_A_PRESS.with(|n| n.set(n.get() + 1));
        NotAPress(())
    }
}

impl Drop for NotAPress {
    fn drop(&mut self) {
        NOT_A_PRESS.with(|n| n.set(n.get().saturating_sub(1)));
    }
}

/// Whether the `lens-new-chat` being handled on this thread is the person's press.
pub fn is_press() -> bool {
    NOT_A_PRESS.with(|n| n.get() == 0)
        && yantrik_app_runtime::control::caller().is_none()
        && yantrik_app_runtime::control::agent_token().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    fn counting() -> impl Fn() -> String + Send + 'static {
        let n = Arc::new(AtomicU64::new(0));
        move || format!("{:032x}", n.fetch_add(1, Ordering::SeqCst) + 1)
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yantrik-taint-scope-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(FILE)
    }

    /// Reads made in a conversation stay with it, however often it is asked.
    #[test]
    fn a_conversation_keeps_what_it_read() {
        let mut s = Scopes::in_memory(counting());
        let first = s.current("hermes");
        assert_eq!((first.begun, first.read.len()), (Begun::Start, 0));
        s.mark("hermes", "os_describe");
        s.mark("hermes", "os_describe");
        s.mark("hermes", "shell.read_screen");
        let now = s.current("hermes");
        assert_eq!(now.id, first.id, "marking is not a new conversation");
        assert_eq!(now.read, vec!["os_describe", "shell.read_screen"]);
    }

    /// The person's + is a conversation that has read nothing, for the mind it was pressed for
    /// only: another mind's conversation, which + did not start again, keeps its reads.
    #[test]
    fn the_persons_new_conversation_is_clean_for_that_mind_only() {
        let mut s = Scopes::in_memory(counting());
        let old = s.mark("hermes", "os_describe");
        s.mark("pi", "os_describe");
        let new = s.begin_by_person("hermes");
        assert_ne!(new.id, old.id);
        assert_eq!((new.begun, new.read.clone()), (Begun::Person, Vec::<String>::new()));
        assert_eq!(s.current("hermes"), new);
        assert_eq!(s.current("pi").read, vec!["os_describe"], "+ for Hermes does not start Pi's again");
    }

    /// A restart reads the scopes back: the same id and the same reads, not a new conversation.
    #[test]
    fn a_restart_keeps_the_conversation_and_its_reads() {
        let path = scratch("restart");
        let before = {
            let mut s = Scopes::load(path.clone());
            s.mark("hermes", "os_describe")
        };
        let mut again = Scopes::load(path.clone());
        assert_eq!(again.current("hermes"), before, "a restart is not the person's +");
        assert!(valid_id(&before.id), "{}", before.id);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// A scope read back from disk was not begun by a press in this run of the shell, so it is
    /// `begun: start` whatever the file says: a record forged to say `person` clears nothing.
    #[test]
    fn a_scope_read_from_disk_was_not_begun_by_the_person() {
        let path = scratch("forged");
        let pressed = {
            let mut s = Scopes::load(path.clone());
            s.mark("hermes", "os_describe");
            s.begin_by_person("hermes")
        };
        assert_eq!(pressed.begun, Begun::Person, "the press itself is the person's");
        let mut again = Scopes::load(path.clone());
        let back = again.current("hermes");
        assert_eq!((back.id.as_str(), back.begun), (pressed.id.as_str(), Begun::Start), "a restart is not a press");

        let forged = format!(
            r#"{{"scopes":{{"hermes":{{"id":"{}","begun":"person","read":[]}}}}}}"#,
            "f".repeat(32)
        );
        std::fs::write(&path, forged).unwrap();
        let mut loaded = Scopes::load(path.clone());
        let scope = loaded.current("hermes");
        assert_eq!((scope.id, scope.begun), ("f".repeat(32), Begun::Start), "a file cannot say the person pressed +");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// No record is a clean first conversation; a record that is there and cannot be read is a
    /// tainted one, until the person presses +.
    #[test]
    fn an_unreadable_record_fails_closed() {
        let path = scratch("unreadable");
        let mut fresh = Scopes::load(path.clone());
        assert!(fresh.current("hermes").read.is_empty(), "no file: nothing was read yet");

        std::fs::write(&path, "{ not json").unwrap();
        let mut lost = Scopes::load(path.clone());
        assert_eq!(lost.current("hermes").read, vec![UNREADABLE]);
        assert_eq!(lost.current("pi").read, vec![UNREADABLE], "every scope made from it");
        assert!(lost.begin_by_person("hermes").read.is_empty(), "the person's + is clean");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// A mark cannot be used to take anything away, or to grow the record without end.
    #[test]
    fn a_mark_only_adds_and_is_bounded() {
        let mut s = Scopes::in_memory(counting());
        s.mark("hermes", "os_describe");
        let blank = s.mark("hermes", "   ");
        assert_eq!(blank.read, vec!["os_describe"]);
        for n in 0..200 {
            s.mark("hermes", &format!("read-{n}"));
        }
        let scope = s.current("hermes");
        assert_eq!(scope.read.len(), READS_KEPT);
        assert_eq!(scope.read[0], "os_describe");
        let long = s.begin_by_person("pi");
        assert!(long.read.is_empty());
        assert!(s.mark("pi", &"x".repeat(500)).read[0].chars().count() <= READ_CHARS);
    }

    /// The control surface's `new_chat` is not a press, and neither is anything inside a guard.
    #[test]
    fn a_call_inside_the_guard_is_not_a_press() {
        assert!(is_press(), "outside any dispatch, with no guard, the callback is the press");
        {
            let _guard = NotAPress::enter();
            assert!(!is_press());
            {
                let _nested = NotAPress::enter();
                assert!(!is_press());
            }
            assert!(!is_press(), "still inside the outer guard");
        }
        assert!(is_press());
        let _who = yantrik_app_runtime::control::CallerScope::enter(Some(yantrik_app_runtime::control::Caller {
            pid: 4242,
            uid: 1000,
            gid: 1000,
        }));
        assert!(!is_press(), "a call that came over the socket is never a press");
    }

    /// The source, read the way `control_approvals` reads it: the scope is begun in one place —
    /// the Lens's own new-chat callback, past its `is_press` check — and the callback is invoked
    /// programmatically in one place, under the guard. Anything new that starts a conversation
    /// has to come past here, and be decided on, rather than quietly untaint a mind.
    #[test]
    fn only_the_lens_press_begins_a_scope() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut begins = Vec::new();
        let mut invokes = Vec::new();
        let mut stack = vec![src];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().and_then(|e| e.to_str()) != Some("rs") || path.ends_with("taint_scope.rs") {
                    continue;
                }
                let whole = std::fs::read_to_string(&path).unwrap();
                let code = whole.split("#[cfg(test)]").next().unwrap_or_default();
                let name = path.strip_prefix(env!("CARGO_MANIFEST_DIR")).unwrap().display().to_string();
                if code.contains("begin_by_person(") {
                    begins.push(name.clone());
                }
                if code.contains("invoke_lens_new_chat(") {
                    invokes.push((name, code.to_string()));
                }
            }
        }
        assert_eq!(begins, vec!["src/wire/chat.rs"], "a taint scope is begun by the Lens's + only");
        let chat = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/wire/chat.rs")).unwrap();
        let at = chat.find("begin_by_person(").unwrap();
        assert!(
            chat[..at].rfind("is_press()").is_some_and(|p| at - p < 400),
            "begin_by_person is called past an is_press() check"
        );
        assert_eq!(invokes.len(), 1, "invoke_lens_new_chat is called in one place: {:?}", invokes.iter().map(|i| &i.0).collect::<Vec<_>>());
        let (file, code) = &invokes[0];
        let at = code.find("invoke_lens_new_chat(").unwrap();
        assert!(
            code[..at].rfind("NotAPress::enter()").is_some_and(|p| at - p < 300),
            "{file} invokes the Lens's new chat without NotAPress: a socket call would begin a taint scope"
        );
    }
}
