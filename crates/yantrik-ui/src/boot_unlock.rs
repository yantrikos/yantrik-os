//! Signed in by the disk's password (the macOS FileVault model).
//!
//! On an encrypted install the person types their password once, on the Yantrik pre-boot screen,
//! and the root opens with it. The initramfs leaves a root-only, one-shot marker in /run when, and
//! only when, a person typed it in this boot (deploy/yantrik-os/boot-unlock/local-bottom). The
//! shell asks a root helper about it once, as it starts, over `/run/yantrik-boot-unlock.sock`; the
//! helper uses the marker up and answers `typed <user>` or `no` (boot-unlock/consume).
//!
//! The answer opens the desktop only for the account it names, the one whose password was
//! enrolled at install and is still unchanged, while the disk's LUKS header still has only the
//! one keyslot enrolled with it (so no other secret opens the disk). Only that account may ask
//! (group yantrik-boot-unlock, and the helper checks the asker's uid). Every other answer, and
//! every failure to get one (no socket, a timeout, nonsense), is `no`, and the lock screen asks as
//! it always has. The password itself is never here: the helper knows only that it was typed.

use std::io::Read;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

/// Where the root helper answers.
pub const SOCKET: &str = "/run/yantrik-boot-unlock.sock";
/// The helper answers at once; anything slower is no answer.
const WAIT: Duration = Duration::from_secs(3);

/// What the helper said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// A person typed the disk's password in this boot, and it is still this account's.
    Typed(String),
    /// Anything else.
    No,
}

/// One line from the helper, read strictly: `typed <account>` and nothing else counts.
pub fn parse(reply: &str) -> Answer {
    let Some(line) = reply.strip_suffix('\n') else { return Answer::No };
    if line.contains('\n') {
        return Answer::No;
    }
    match line.strip_prefix("typed ") {
        Some(user) if plausible_account(user) => Answer::Typed(user.to_string()),
        _ => Answer::No,
    }
}

fn plausible_account(user: &str) -> bool {
    let mut chars = user.chars();
    matches!(chars.next(), Some('a'..='z' | '_'))
        && chars.all(|c| matches!(c, 'a'..='z' | '0'..='9' | '_' | '-'))
        && user.len() <= 32
}

/// Ask the helper at `socket`, once. It uses the marker up whatever this does with the answer.
pub fn ask(socket: &Path) -> Answer {
    let Ok(mut stream) = UnixStream::connect(socket) else { return Answer::No };
    if stream.set_read_timeout(Some(WAIT)).is_err() {
        return Answer::No;
    }
    let mut reply = String::new();
    // A short line; anything longer than an account name and a word is not an answer.
    match (&mut stream).take(64).read_to_string(&mut reply) {
        Ok(_) => parse(&reply),
        Err(_) => Answer::No,
    }
}

/// Whether the session starts signed in: the helper's `typed` names `me`.
pub fn signs_in(answer: &Answer, me: Option<&str>) -> bool {
    matches!((answer, me), (Answer::Typed(user), Some(me)) if user == me)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::net::UnixListener;

    #[test]
    fn only_typed_and_an_account_name_is_an_answer() {
        assert_eq!(parse("typed pranab\n"), Answer::Typed("pranab".into()));
        assert_eq!(parse("no\n"), Answer::No);
        assert_eq!(parse(""), Answer::No, "absent means the lock screen");
        assert_eq!(parse("typed pranab"), Answer::No, "a line cut short");
        assert_eq!(parse("typed \n"), Answer::No);
        assert_eq!(parse("typed Pranab\n"), Answer::No);
        assert_eq!(parse("typed pranab\ntyped root\n"), Answer::No);
        assert_eq!(parse("typed ../../etc\n"), Answer::No);
        assert_eq!(parse("TYPED pranab\n"), Answer::No);
    }

    #[test]
    fn it_signs_in_only_the_account_it_names() {
        let typed = Answer::Typed("pranab".into());
        assert!(signs_in(&typed, Some("pranab")));
        assert!(!signs_in(&typed, Some("guest")), "another account's password does not open this one");
        assert!(!signs_in(&typed, None), "not knowing who we are is not signed in");
        assert!(!signs_in(&Answer::No, Some("pranab")));
    }

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("yantrik-boot-unlock-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The helper answers once and is used up (consume), so the second shell to ask locks.
    #[test]
    fn the_first_question_is_answered_and_the_second_is_no() {
        let dir = scratch("once");
        let sock = dir.join("s");
        let listener = UnixListener::bind(&sock).unwrap();
        let server = std::thread::spawn(move || {
            let mut used = false;
            for stream in listener.incoming().take(2) {
                let mut stream = stream.unwrap();
                let reply = if used { "no\n" } else { "typed pranab\n" };
                used = true;
                stream.write_all(reply.as_bytes()).unwrap();
            }
        });
        assert_eq!(ask(&sock), Answer::Typed("pranab".into()));
        assert_eq!(ask(&sock), Answer::No);
        server.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_helper_or_a_silent_one_is_the_lock_screen() {
        let dir = scratch("absent");
        assert_eq!(ask(&dir.join("missing")), Answer::No);
        let sock = dir.join("silent");
        let listener = UnixListener::bind(&sock).unwrap();
        let server = std::thread::spawn(move || {
            let (_stream, _) = listener.accept().unwrap();
            std::thread::sleep(WAIT + Duration::from_millis(500));
        });
        assert_eq!(ask(&sock), Answer::No, "a helper that never answers");
        server.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
