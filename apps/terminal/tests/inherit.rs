//! What a shell starts with, whatever the app was holding or ignoring when it started it: nothing
//! of the app's, every signal at its default and none blocked.
//!
//! A test binary of its own, because it changes the whole process's signal dispositions and holds
//! descriptors open on purpose.
//!
//! The descriptors are checked by identity, not by an exact list. A program may open files of its
//! own while it starts (a shell opens `/dev/tty` and reads its startup files), and for a moment
//! after `spawn` returns the exec is still closing descriptors: so the test waits for the program
//! to settle, and then asks only that nothing of the app's is open, under any number.

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use yantrik_terminal::Session;

fn start(program: &str, args: &[&str]) -> Session {
    Session::spawn_shell(program, args, Path::new("/tmp"), 24, 80, Arc::new(|| {})).unwrap()
}

fn mask(pid: u32, field: &str) -> u64 {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
    let line = status.lines().find_map(|l| l.strip_prefix(field)).unwrap();
    u64::from_str_radix(line.trim(), 16).unwrap()
}

/// One open descriptor: its number, where it points, and the file's identity (device, inode).
type Open = (u32, PathBuf, Option<(u64, u64)>);

/// The open descriptors of a process.
fn open_files(pid: u32) -> Vec<Open> {
    let mut files: Vec<_> = std::fs::read_dir(format!("/proc/{pid}/fd"))
        .unwrap()
        .flatten()
        .filter_map(|e| {
            let number = e.file_name().to_string_lossy().parse().ok()?;
            let target = std::fs::read_link(e.path()).ok()?;
            let identity = std::fs::metadata(e.path()).ok().map(|m| (m.dev(), m.ino()));
            Some((number, target, identity))
        })
        .collect();
    files.sort();
    files
}

/// Wait until the program has been exec'd (`/proc/<pid>/exe` is no longer this test) and has
/// stopped changing its descriptors: the same list twice, 50 ms apart. Bounded.
fn settled(pid: u32) {
    let until = Instant::now() + Duration::from_secs(5);
    let this = std::env::current_exe().unwrap();
    while std::fs::read_link(format!("/proc/{pid}/exe")).is_ok_and(|exe| exe == this) && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(5));
    }
    let mut before = open_files(pid);
    while Instant::now() < until {
        std::thread::sleep(Duration::from_millis(50));
        let now = open_files(pid);
        if now == before {
            return;
        }
        before = now;
    }
}

#[test]
fn a_shell_starts_clean_whatever_the_app_was_holding_and_ignoring() {
    // A file opened without close-on-exec, as a graphics library does, on a number no shell would
    // pick for itself.
    let leaked_path = std::env::temp_dir().join(format!("yantrik-terminal-leak-{}", std::process::id()));
    std::fs::write(&leaked_path, "x").unwrap();
    let identity = unsafe {
        let fd = libc::open(std::ffi::CString::new(leaked_path.to_str().unwrap()).unwrap().as_ptr(), libc::O_RDONLY);
        assert!(fd >= 0);
        assert_eq!(libc::dup2(fd, 40), 40);
        libc::close(fd);
        let meta = std::fs::metadata(&leaked_path).unwrap();
        (meta.dev(), meta.ino())
    };
    // Signals ignored and blocked, as a launch from a non-interactive `&` leaves them.
    unsafe {
        libc::signal(libc::SIGINT, libc::SIG_IGN);
        libc::signal(libc::SIGQUIT, libc::SIG_IGN);
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGINT);
        libc::sigaddset(&mut set, libc::SIGQUIT);
        libc::sigaddset(&mut set, libc::SIGUSR1);
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
    }

    // A program that changes nothing, and a shell that opens files of its own.
    for (program, args) in [("/bin/cat", &[][..]), ("/bin/sh", &["-i"][..])] {
        let session = start(program, args);
        let pid = session.pid();
        settled(pid);
        let files = open_files(pid);

        assert!(
            files.iter().all(|(_, _, id)| *id != Some(identity)),
            "{program} holds the app's file, under some number: {files:?}"
        );
        for (number, target, _) in &files {
            let t = target.to_string_lossy();
            assert!(
                !(t.contains("ptmx") || t.contains("udmabuf") || t.contains("memfd") || t.starts_with("socket:")),
                "{program} holds fd {number} -> {t}: {files:?}"
            );
        }
        for number in 0..3 {
            let target = files.iter().find(|(n, _, _)| *n == number).map(|(_, t, _)| t.to_string_lossy().into_owned());
            assert!(
                target.as_deref().is_some_and(|t| t.starts_with("/dev/pts/")),
                "{program}: fd {number} is {target:?}, not its terminal: {files:?}"
            );
        }

        let ignored = mask(pid, "SigIgn:");
        let blocked = mask(pid, "SigBlk:");
        let bit = |signal: i32| 1u64 << (signal - 1);
        if program == "/bin/cat" {
            for signal in [libc::SIGINT, libc::SIGQUIT] {
                assert_eq!(ignored & bit(signal), 0, "signal {signal} is ignored in {program}: {ignored:x}");
            }
        }
        assert_eq!(blocked & (bit(libc::SIGINT) | bit(libc::SIGQUIT) | bit(libc::SIGUSR1)), 0, "{program} blocks: {blocked:x}");
        session.shutdown();
    }
    let _ = std::fs::remove_file(&leaked_path);

    // A shell that cannot be started is still reported: the pipe std uses for that survives.
    let missing = Session::spawn_shell("/no/such/shell", &[], Path::new("/tmp"), 24, 80, Arc::new(|| {}));
    assert!(missing.is_err());
}
