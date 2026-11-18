//! Private mode, and the callers the kernel vouches for while it is on (#924; #926 review, M1/M2).
//!
//! Two questions, one answer to who is calling. [`reads_while_private`]: may this caller read an
//! `app.act`'s own `result` and refusals while the person is private — the shell, the called app's
//! own installed program (a second launch handing over), and [`PRIVATE_READERS`] toward the
//! surface each is listed for. And [`content_for_owners`], below:
//!
//! Notes, mail and calendar events live in services the person's own apps read them through. A
//! blanket refusal while the person is private would leave their own Notes, Email and Calendar
//! blank; no check at all hands the content to anything that can open the socket. So while the
//! person is private these reads answer only the app the content belongs to, and the shell, and
//! everyone else gets the transport's `PRIVATE:` sentence ([`crate::privacy::REFUSAL`]).
//!
//! Who is calling is the kernel's account, never the caller's: the pid `SO_PEERCRED` stamped on
//! the connection, and of that process ([`PeerFacts`]):
//!
//! - `/proc/<pid>/exe` resolved, compared with the installed program — a directory and a name,
//!   never a name alone, as [`crate::owner::is_desktop_binary_in`] compares them. A copy called
//!   `yantrik-email` in `/tmp` is not the Email app;
//! - its mount namespace, which must be this process's. Without that, `unshare -Urm` and a tmpfs
//!   over the install directory — no privilege needed — make any program read as an installed one;
//! - for a program still on disk, the file itself: `/proc/<pid>/exe` and the install path must be
//!   the same device and inode, so a path that only names the program is not enough.
//!
//! # Threat note
//!
//! This is against careless agents, not hostile ones. A process running as the person can still
//! read the content's files directly (the notes folder, the calendar store, the mail account file).
//! And it can still be vouched for: connect, then exec an installed program (`SO_PEERCRED` is the
//! pid at connect time, `/proc/<pid>/exe` is read later), or `LD_PRELOAD` or attach a debugger to
//! an installed program, or wait for an installed program to reuse its pid. Vouched that way it
//! reads any app's act `result` and refusals, not only the chat view. Each of these is a deliberate
//! evasion of a refusal that says why, outside the careless-agent threat model, and closing them
//! takes a caller identity worth more than a program's name: #43. What this ends is an agent that
//! uses the documented door — a harness the person started by hand, which sends no token — reading
//! the person's notes, mail, events or any app's answers while they are private.

use std::path::Path;

use crate::owner::{exe_of, INSTALL_DIR, SHELL_BINARY};
use crate::server::PeerCred;

/// What Linux appends to `/proc/<pid>/exe` when the program has been replaced under a running
/// process, as an update does. That process is still the program it was started as.
const DELETED: &str = " (deleted)";

/// The desktop's programs, besides the shell and the called app's own, that read an `app.act`'s
/// `result` and refusals while the person is private, each toward the one surface it needs: the
/// program's name in the install directory, and the surface's id. Every entry says why, and an
/// addition is a decision for a reader.
pub const PRIVATE_READERS: &[(&str, &str)] = &[
    // The person's chat with the Mind in a terminal: it reads `chat_view` and `send_message`'s
    // results from the shell, so without it the person's own terminal goes blank while they are
    // private. Toward the shell only. Residual, accepted until #43: an agent can exec yantrik-term
    // itself and read the chat view — the person's Mind conversation.
    ("yantrik-term", "shell"),
];

/// What the kernel says about the process on the other end of a call, for deciding whether it is
/// vouched for. Gathered by [`facts_of`]; built by hand in tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerFacts {
    /// `/proc/<pid>/exe`, resolved.
    pub exe: String,
    /// Its mount namespace is this process's.
    pub same_mount_ns: bool,
    /// The program it runs is the file its path names here (same device and inode), or it has been
    /// replaced on disk since it started (`exe` ends ` (deleted)`).
    pub same_file: bool,
}

/// The facts about `pid`, or `None` when `/proc` will not say.
#[cfg(target_os = "linux")]
pub fn facts_of(pid: i32) -> Option<PeerFacts> {
    use std::os::unix::fs::MetadataExt;
    let exe = exe_of(pid)?;
    let ns = |p: &str| std::fs::read_link(format!("/proc/{p}/ns/mnt")).ok();
    let theirs = ns(&pid.to_string());
    let same_mount_ns = theirs.is_some() && theirs == ns("self");
    let same_file = exe.ends_with(DELETED)
        || match (std::fs::metadata(format!("/proc/{pid}/exe")), std::fs::metadata(&exe)) {
            (Ok(running), Ok(named)) => running.dev() == named.dev() && running.ino() == named.ino(),
            _ => false,
        };
    Some(PeerFacts { exe, same_mount_ns, same_file })
}

#[cfg(not(target_os = "linux"))]
pub fn facts_of(pid: i32) -> Option<PeerFacts> {
    let _ = pid;
    None
}

fn without_deleted(exe: &str) -> &str {
    exe.strip_suffix(DELETED).unwrap_or(exe)
}

/// The program's name, when `exe` sits directly in `dir`.
fn installed_name<'a>(exe: &'a str, dir: &Path) -> Option<&'a str> {
    let path = Path::new(without_deleted(exe));
    if !path.is_absolute() || path.parent() != Some(dir) {
        return None;
    }
    path.file_name().and_then(|n| n.to_str())
}

/// Whether `exe` — the caller's `/proc/<pid>/exe` resolved — may read an act's own answer from the
/// surface `surface` while the person is private: the shell, a [`PRIVATE_READERS`] entry for that
/// surface, or the same program as `own` (this process's, the app the call is to), each directly
/// in `dir`. The path half of the rule; [`is_vouched`] adds the kernel's other facts. Pure.
pub fn is_private_reader_in(exe: &str, own: Option<&str>, dir: &Path, surface: &str) -> bool {
    let Some(name) = installed_name(exe, dir) else { return false };
    name == SHELL_BINARY
        || PRIVATE_READERS.iter().any(|(reader, to)| *reader == name && *to == surface)
        || own.map(without_deleted) == Some(without_deleted(exe))
}

/// Whether a caller with these facts is vouched for toward `surface`: in this mount namespace,
/// running the file its path names, and a reader by [`is_private_reader_in`]. Pure.
pub fn is_vouched(peer: &PeerFacts, own: Option<&str>, dir: &Path, surface: &str) -> bool {
    peer.same_mount_ns && peer.same_file && is_private_reader_in(&peer.exe, own, dir, surface)
}

/// Whether the process `pid` — the kernel's account of an `app.act` caller — may read the act's
/// own `result` and refusals from `surface` while the person is private, against this process's
/// own program and the install directory. Unidentifiable is no.
pub fn reads_while_private(pid: i32, surface: &str) -> bool {
    let Some(peer) = facts_of(pid) else { return false };
    let own = exe_of(std::process::id() as i32);
    is_vouched(&peer, own.as_deref(), Path::new(INSTALL_DIR), surface)
}

/// Whether `exe` — `/proc/<pid>/exe` resolved — is one of `owners`, or the shell, sitting directly
/// in `dir`. The path half of the owner rule. Pure.
pub fn is_owner_in(exe: &str, dir: &Path, owners: &[&str]) -> bool {
    installed_name(exe, dir).is_some_and(|name| name == SHELL_BINARY || owners.contains(&name))
}

/// Whether `method`'s content may go to `peer` now: always while the person is not `private`;
/// while they are, only to one of `owners` (program names in the install directory) or the shell,
/// held to the same mount namespace and file checks as [`is_vouched`]. `Err` is the reason, for the
/// service's log; the caller is told only the transport's sentence.
pub fn content_for_owners(peer: Option<PeerCred>, method: &str, owners: &[&str], private: bool) -> Result<(), String> {
    if !private {
        return Ok(());
    }
    let Some(peer) = peer else {
        return Err(format!("{method} in Private mode: the kernel would not say which process is calling"));
    };
    match facts_of(peer.pid) {
        Some(f) if f.same_mount_ns && f.same_file && is_owner_in(&f.exe, Path::new(INSTALL_DIR), owners) => Ok(()),
        Some(f) => Err(format!(
            "{method} in Private mode answers {} and the shell, installed and in this mount namespace; \
             the caller is {} (pid {})",
            owners.join(", "),
            f.exe,
            peer.pid
        )),
        None => Err(format!(
            "{method} in Private mode: the calling process (pid {}) could not be identified from /proc",
            peer.pid
        )),
    }
}
