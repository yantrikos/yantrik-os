//! Private mode, and the callers the kernel vouches for while it is on (#924; #926 review, M1/M2).
//!
//! Two questions, one answer to who is calling. [`reads_while_private`]: may this caller read an
//! `app.act`'s own `result` and refusals while the person is private — the shell, the called app's
//! own installed program (a second launch handing over), and [`PRIVATE_READERS`]. And
//! [`content_for_owners`], below:
//!
//! Notes, mail and calendar events live in services the person's own apps read them through. A
//! blanket refusal while the person is private would leave their own Notes, Email and Calendar
//! blank; no check at all hands the content to anything that can open the socket. So while the
//! person is private these reads answer only the app the content belongs to, and the shell, and
//! everyone else gets the transport's `PRIVATE:` sentence ([`crate::privacy::REFUSAL`]).
//!
//! Who is calling is the kernel's account, never the caller's: the pid `SO_PEERCRED` stamped on
//! the connection, `/proc/<pid>/exe` resolved, compared with the installed program — a directory
//! and a name, never a name alone, as [`crate::owner::is_desktop_binary_in`] compares them. A copy
//! called `yantrik-email` in `/tmp` is not the Email app.
//!
//! # Threat note
//!
//! This is against careless agents, not hostile ones. A process running as the person can read
//! the content's files directly (the notes folder, the calendar store, the mail account file), or
//! copy a program into the install directory it can write to, or exec a real one after connecting.
//! What this ends is an agent that uses the documented door — a harness the person started by hand,
//! which sends no token — reading the person's notes, mail and events while they are private. The
//! uid is the boundary, as `owner.rs` records for the shell (#154); a caller identity worth more
//! than the binary's name is #43's.

use std::path::Path;

use crate::owner::{exe_of, INSTALL_DIR, SHELL_BINARY};
use crate::server::PeerCred;

/// What Linux appends to `/proc/<pid>/exe` when the program has been replaced under a running
/// process, as an update does. That process is still the program it was started as.
const DELETED: &str = " (deleted)";

/// The desktop's programs, besides the shell and the called app's own, that read an `app.act`'s
/// `result` and refusals while the person is private. Installed in the install directory; a name
/// elsewhere is not one of them. Every entry says why, and an addition is a decision for a reader.
pub const PRIVATE_READERS: &[&str] = &[
    // The person's chat with the Mind in a terminal: it reads `chat_view` and `send_message`'s
    // results from the shell, so without it the person's own terminal goes blank while they are
    // private. Residual, accepted until #43: an agent can exec yantrik-term itself and read the
    // chat view — the person's Mind conversation, not any app's contents.
    "yantrik-term",
];

/// Whether `exe` — the caller's `/proc/<pid>/exe` resolved — may read an act's own answer while the
/// person is private: the shell, one of [`PRIVATE_READERS`], or the same program as `own` (this
/// process's, the app the call is to), each sitting directly in `dir`. Pure, so the rule is a test.
pub fn is_private_reader_in(exe: &str, own: Option<&str>, dir: &Path) -> bool {
    let strip = |e: &str| e.strip_suffix(DELETED).unwrap_or(e).to_string();
    let exe = strip(exe);
    let path = Path::new(&exe);
    if !path.is_absolute() || path.parent() != Some(dir) {
        return false;
    }
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    name == SHELL_BINARY || PRIVATE_READERS.contains(&name) || own.map(strip).as_deref() == Some(exe.as_str())
}

/// Whether the process `pid` — the kernel's account of an `app.act` caller — may read the act's
/// own `result` and refusals while the person is private ([`is_private_reader_in`], against this
/// process's own program and the install directory). Unidentifiable is no.
pub fn reads_while_private(pid: i32) -> bool {
    let Some(exe) = exe_of(pid) else { return false };
    let own = exe_of(std::process::id() as i32);
    is_private_reader_in(&exe, own.as_deref(), Path::new(INSTALL_DIR))
}

/// Whether `exe` — `/proc/<pid>/exe` resolved — is one of `owners`, or the shell, sitting directly
/// in `dir`. Pure, so the rule is a test.
pub fn is_owner_in(exe: &str, dir: &Path, owners: &[&str]) -> bool {
    let exe = Path::new(exe.strip_suffix(DELETED).unwrap_or(exe));
    exe.is_absolute()
        && exe.parent() == Some(dir)
        && exe
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|name| name == SHELL_BINARY || owners.contains(&name))
}

/// Whether `method`'s content may go to `peer` now: always while the person is not `private`;
/// while they are, only to one of `owners` (program names in the install directory) or the shell.
/// `Err` is the reason, for the service's log; the caller is told only the transport's sentence.
pub fn content_for_owners(peer: Option<PeerCred>, method: &str, owners: &[&str], private: bool) -> Result<(), String> {
    if !private {
        return Ok(());
    }
    let Some(peer) = peer else {
        return Err(format!("{method} in Private mode: the kernel would not say which process is calling"));
    };
    match exe_of(peer.pid) {
        Some(exe) if is_owner_in(&exe, Path::new(INSTALL_DIR), owners) => Ok(()),
        Some(exe) => Err(format!(
            "{method} in Private mode answers {} and the shell; the caller is {exe} (pid {})",
            owners.join(", "),
            peer.pid
        )),
        None => Err(format!(
            "{method} in Private mode: the calling process (pid {}) could not be identified from /proc",
            peer.pid
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIR: &str = "/opt/yantrik/bin";

    #[test]
    fn the_readers_while_private_are_the_shell_the_apps_own_program_and_the_list() {
        let dir = Path::new(DIR);
        let own = Some("/opt/yantrik/bin/yantrik-notes");
        for yes in [
            "/opt/yantrik/bin/yantrik-ui",
            "/opt/yantrik/bin/yantrik-term",
            "/opt/yantrik/bin/yantrik-notes",
            "/opt/yantrik/bin/yantrik-notes (deleted)",
        ] {
            assert!(is_private_reader_in(yes, own, dir), "{yes}");
        }
        for no in [
            "/opt/yantrik/bin/yantrik-email",
            "/tmp/yantrik-term",
            "/home/p/target/release/yantrik-notes",
            "/usr/bin/python3.11",
            "yantrik-ui",
            "",
        ] {
            assert!(!is_private_reader_in(no, own, dir), "{no}");
        }
        // An app's own program counts only installed: a test binary calling its own socket is no one.
        assert!(!is_private_reader_in("/build/t/deps/x-1a2b", Some("/build/t/deps/x-1a2b"), dir));
        assert!(!reads_while_private(std::process::id() as i32), "this test binary is not installed");
        assert_eq!(PRIVATE_READERS, ["yantrik-term"], "an addition is a decision for a reader");
    }

    #[test]
    fn the_owner_is_the_installed_app_or_the_shell_and_nothing_with_its_name_elsewhere() {
        let dir = Path::new(DIR);
        let owners = ["yantrik-email"];
        for yes in [
            "/opt/yantrik/bin/yantrik-email",
            "/opt/yantrik/bin/yantrik-ui",
            "/opt/yantrik/bin/yantrik-email (deleted)",
        ] {
            assert!(is_owner_in(yes, dir, &owners), "{yes}");
        }
        for no in [
            "/tmp/yantrik-email",
            "/home/p/target/release/yantrik-email",
            "/opt/yantrik/bin/yantrik-notes",
            "/opt/yantrik/bin/sub/yantrik-email",
            "yantrik-email",
            "/usr/bin/python3.11",
            "",
        ] {
            assert!(!is_owner_in(no, dir, &owners), "{no}");
        }
    }

    #[test]
    fn out_of_private_mode_anyone_reads_and_in_it_an_unknown_caller_does_not() {
        assert!(content_for_owners(None, "notes.get", &["yantrik-notes"], false).is_ok());
        assert!(content_for_owners(None, "notes.get", &["yantrik-notes"], true).is_err());
        // This test binary is not in the install directory, so it is nobody's owner.
        let me = PeerCred { pid: std::process::id() as i32, uid: 0, gid: 0 };
        let why = content_for_owners(Some(me), "notes.get", &["yantrik-notes"], true).unwrap_err();
        assert!(why.contains("yantrik-notes") && why.contains("notes.get"), "{why}");
        assert!(content_for_owners(Some(me), "notes.get", &["yantrik-notes"], false).is_ok());
    }
}
