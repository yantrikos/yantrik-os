//! The end of a window's event loop, whichever end it is.
//!
//! A Slint event loop ends one of two ways. The person closes the window, or the program quits
//! it, and `run()` returns `Ok`. Or the compositor goes away underneath it — a logout, a session
//! restart, labwc exiting — and winit returns an error, because the connection it was reading
//! has gone. Every main used to unwrap that error, so every logout panicked every program that
//! had a window open. The panic hook filed a crash record for each of them (VM 520's Problems
//! screen listed one `yantrik-ui` "crash" per session restart, issue #196), and whatever a main
//! does after its loop never ran: the shell stopping its services and killing agents' command
//! process groups, an editor writing its last recovery draft.
//!
//! A display that goes away is an ending, not a crash. [`run_until_closed`] logs it as one and
//! returns, and the main carries on to its own shutdown.
//!
//! # Telling a lost display from a real failure
//!
//! The error cannot say which one it is. Slint flattens winit's error into a string,
//! `PlatformError::Other("Error running winit event loop: Exit Failure: 1")`, and winit 0.30's
//! `ExitFailure` means only that the connection to the display failed: `1` when a flush of it
//! failed or the failure carried no errno, the errno otherwise. A compositor that has gone away
//! fails it (in a live run, a dispatch error with no errno: `1`). So does a compositor that is
//! still running and closed *our* connection because we sent it something illegal — a protocol
//! error, which is a bug in us. Read during a dispatch that one arrives as `EPROTO`
//! (calloop-wayland-source's mapping), but caught by the flush first it is `1`, exactly what a
//! logout looks like. And a display can be lost first by the renderer, whose error says nothing
//! about Wayland at all.
//!
//! So the display is asked instead of the error. When the loop ends in an error, the socket named
//! by `WAYLAND_DISPLAY` is connected to: a compositor that has gone refuses. One that is still
//! answering after [`GRACE`] — long enough for one that is part-way through exiting — did not go
//! away, so the loop ended for a reason of its own, and that is written down as a problem record
//! like any other failure. A panic anywhere still goes through the panic hook, unchanged; only the
//! loop's own `Err` is read here. With no Wayland socket to ask (X11, a KMS backend) the error's
//! text is the only evidence, and only winit's `Exit Failure` other than `EPROTO` counts as the
//! display going away.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::problems;

/// How long a compositor that is still taking connections is given to stop. A compositor on its
/// way out drops its clients before it closes its socket, so the first answer after a logout can
/// come from one that is still leaving.
pub const GRACE: Duration = Duration::from_secs(3);

/// Between two attempts to connect to the display.
const STEP: Duration = Duration::from_millis(100);

/// `EPROTO` on Linux: calloop-wayland-source's code for a protocol error received from the
/// compositor, which winit hands on as `Exit Failure: 71`. The compositor was there to send it.
const EPROTO: i32 = 71;

/// Run `ui`'s event loop until it ends, and say how it ended.
///
/// Returns `true` when the window was closed. Returns `false` when the loop ended in an error:
/// logged as the display going away when it did (a logout, a session restart), or logged and
/// written down as a problem record when the display is still there and the loop failed on its
/// own. It never panics, so what the caller does after its loop — stopping services, a last
/// save — runs on every ending. `program` is the name the caller gave `init_tracing`, e.g.
/// `yantrik-weather`; a record is filed under it.
///
/// See the module documentation for why the display is asked rather than the error read.
pub fn run_until_closed(ui: &impl slint::ComponentHandle, program: &str) -> bool {
    run_until_ended(ui, program) == LoopEnd::Closed
}

/// How a window's event loop ended, for a caller that does something different for each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopEnd {
    /// The window was closed, or the program quit its own loop.
    Closed,
    /// The display went away and took the loop with it: a logout, a session restart.
    DisplayGone,
    /// The loop failed while the display was still there, and a problem record was written.
    ///
    /// The shell exits with a status of its own for this one, because it is how a GPU that Mesa
    /// accelerates but the compositor cannot use shows itself: on VirtualBox labwc refused the
    /// shell's first frame and dropped its connection, and the session reads that status to fall
    /// back to software (deploy/yantrik-os/yantrik-session).
    Failed,
}

/// [`run_until_closed`], saying which of the three endings it was.
pub fn run_until_ended(ui: &impl slint::ComponentHandle, program: &str) -> LoopEnd {
    match ui.run() {
        Ok(()) => LoopEnd::Closed,
        Err(error) => {
            let socket = wayland_socket();
            match ended_in_error(program, &error.to_string(), socket.as_deref(), GRACE, &problems::dir()) {
                Ending::DisplayGone => LoopEnd::DisplayGone,
                Ending::Failure => LoopEnd::Failed,
            }
        }
    }
}

/// How a loop that returned an error ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ending {
    /// The display went away and took the loop with it.
    DisplayGone,
    /// The loop failed while the display was still there, or with an error no lost display
    /// produces.
    Failure,
}

/// Log how the loop ended, write a record when it failed, and say which it was.
fn ended_in_error(
    program: &str,
    error: &str,
    socket: Option<&Path>,
    grace: Duration,
    records: &Path,
) -> Ending {
    let still_there = socket.map(|s| still_answering(s, grace));
    let ending = classify(error, still_there);
    match ending {
        Ending::DisplayGone => {
            tracing::warn!(
                program,
                error,
                "The display went away, so the event loop ended; closing, not crashing"
            );
        }
        Ending::Failure => {
            tracing::error!(
                program,
                error,
                display_still_there = ?still_there,
                "The event loop ended with an error that was not the display going away; \
                 writing a problem record"
            );
            let message = format!("the event loop ended with an error: {error}");
            problems::write_to(records, &problems::problem("failure", program, &message, None, None));
        }
    }
    ending
}

/// `still_there` is the display's own answer when it could be asked; the error's text decides
/// only when it could not.
fn classify(error: &str, still_there: Option<bool>) -> Ending {
    match still_there {
        Some(false) => Ending::DisplayGone,
        Some(true) => Ending::Failure,
        None => match exit_failure(error) {
            Some(code) if code != EPROTO => Ending::DisplayGone,
            _ => Ending::Failure,
        },
    }
}

/// The code in winit's `Exit Failure: N`, as Slint passes it on.
fn exit_failure(error: &str) -> Option<i32> {
    let (_, code) = error.rsplit_once("Exit Failure: ")?;
    code.trim().parse().ok()
}

/// The socket this process's Wayland display listens on: `WAYLAND_DISPLAY`, under
/// `XDG_RUNTIME_DIR` unless it is absolute. `None` when there is no Wayland socket to name — X11,
/// a KMS backend, or a connection handed over as a descriptor in `WAYLAND_SOCKET`.
fn wayland_socket() -> Option<PathBuf> {
    if std::env::var_os("WAYLAND_SOCKET").is_some() {
        return None;
    }
    let name = PathBuf::from(std::env::var_os("WAYLAND_DISPLAY")?);
    if name.as_os_str().is_empty() {
        return None;
    }
    if name.is_absolute() {
        return Some(name);
    }
    Some(PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?).join(name))
}

/// Whether a compositor is still taking connections on `socket`, asked until it stops or `grace`
/// runs out. Refused, or no socket at all, is gone; anything else is not evidence of going.
///
/// The asking happens on a thread of its own because a connect to a listener that has stopped
/// accepting without closing can block; that is a compositor still there, as far as this can tell,
/// and the caller is not left waiting on it past the grace.
#[cfg(unix)]
fn still_answering(socket: &Path, grace: Duration) -> bool {
    use std::io::ErrorKind;
    use std::os::unix::net::UnixStream;
    use std::time::Instant;

    let socket = socket.to_path_buf();
    let (answer, answered) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let deadline = Instant::now() + grace;
        loop {
            if let Err(e) = UnixStream::connect(&socket) {
                if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::ConnectionRefused) {
                    let _ = answer.send(false);
                    return;
                }
            }
            if Instant::now() >= deadline {
                let _ = answer.send(true);
                return;
            }
            std::thread::sleep(STEP);
        }
    });
    answered.recv_timeout(grace + STEP * 5).unwrap_or(true)
}

#[cfg(not(unix))]
fn still_answering(_socket: &Path, _grace: Duration) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    /// A directory of this test's own; see `problems::tests::tmp` for why a counter.
    fn tmp(name: &str) -> PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let d = std::env::temp_dir().join(format!(
            "yantrik-loop-{}-{}-{}",
            std::process::id(),
            name,
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// What VM 520 recorded at each `loginctl terminate-session`, as Slint words it.
    const LOGOUT: &str = "Error running winit event loop: Exit Failure: 1";

    #[test]
    fn the_code_is_read_out_of_winits_exit_failure() {
        assert_eq!(exit_failure(LOGOUT), Some(1));
        assert_eq!(exit_failure("Error running winit event loop: Exit Failure: 71"), Some(71));
        assert_eq!(exit_failure("Nested event loops are not supported"), None);
        assert_eq!(exit_failure("Exit Failure: soon"), None);
    }

    #[test]
    fn the_display_answers_before_the_error_is_read() {
        // Asked and gone: an ending, whatever the error says.
        assert_eq!(classify(LOGOUT, Some(false)), Ending::DisplayGone);
        assert_eq!(classify("rendering failed", Some(false)), Ending::DisplayGone);
        // Asked and still there: the same `Exit Failure: 1` is a failure, because a compositor
        // that is still running closed our connection, and that is a bug in us.
        assert_eq!(classify(LOGOUT, Some(true)), Ending::Failure);
        // Not asked: only a lost connection that was not a protocol error counts as an ending.
        assert_eq!(classify(LOGOUT, None), Ending::DisplayGone);
        assert_eq!(classify("Error running winit event loop: Exit Failure: 32", None), Ending::DisplayGone);
        assert_eq!(classify("Error running winit event loop: Exit Failure: 71", None), Ending::Failure);
        assert_eq!(classify("Nested event loops are not supported", None), Ending::Failure);
    }

    #[cfg(unix)]
    #[test]
    fn a_display_that_has_gone_is_an_ending_and_leaves_no_record() {
        let d = tmp("gone");
        let records = d.join("problems");
        // Never there at all: the runtime directory a logout removed.
        let ending = ended_in_error("yantrik-test", LOGOUT, Some(&d.join("wayland-0")), GRACE, &records);
        assert_eq!(ending, Ending::DisplayGone);
        // There once, listener closed: the socket file a compositor left behind.
        let socket = d.join("wayland-1");
        drop(std::os::unix::net::UnixListener::bind(&socket).unwrap());
        let ending = ended_in_error("yantrik-test", LOGOUT, Some(&socket), GRACE, &records);
        assert_eq!(ending, Ending::DisplayGone);
        assert!(problems::list_in(&records).is_empty(), "a logout was written down as a problem");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[cfg(unix)]
    #[test]
    fn a_display_still_answering_means_the_loop_failed_and_it_is_written_down() {
        let d = tmp("there");
        let records = d.join("problems");
        let socket = d.join("wayland-0");
        let _compositor = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let ending =
            ended_in_error("yantrik-test", LOGOUT, Some(&socket), Duration::from_millis(300), &records);
        assert_eq!(ending, Ending::Failure);
        let written = problems::list_in(&records);
        assert_eq!(written.len(), 1, "a real failure left no record");
        assert_eq!(written[0].1.kind, "failure");
        assert_eq!(written[0].1.program, "yantrik-test");
        assert!(written[0].1.message.contains("Exit Failure: 1"), "{}", written[0].1.message);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[cfg(unix)]
    #[test]
    fn a_compositor_that_finishes_exiting_within_the_grace_went_away() {
        // A compositor drops its clients before it closes its socket: the first ask can reach one on
        // its way out. It must be waited for, and not for the whole grace.
        let d = tmp("exiting");
        let records = d.join("problems");
        let socket = d.join("wayland-0");
        let compositor = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let closes = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(250));
            drop(compositor);
        });
        let started = Instant::now();
        let ending = ended_in_error("yantrik-test", LOGOUT, Some(&socket), GRACE, &records);
        closes.join().unwrap();
        assert_eq!(ending, Ending::DisplayGone);
        assert!(started.elapsed() < GRACE, "waited out the grace for a compositor that had gone");
        assert!(problems::list_in(&records).is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Every main that opens a window reaches the end of its event loop through
    /// [`run_until_closed`], and none of them unwraps the loop's result again.
    ///
    /// An unwrap there turns every logout into a crash record and skips whatever the main does
    /// after its loop (#196). It was in nineteen mains at once because each was written from the
    /// last, which is how it would come back; so the mains are read, not listed.
    #[test]
    fn no_main_unwraps_its_event_loop() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let Ok(apps) = std::fs::read_dir(root.join("apps")) else {
            eprintln!("skipped: packaged source without the apps tree");
            return;
        };
        let mut mains: Vec<PathBuf> = apps.flatten().map(|e| e.path().join("src/main.rs")).collect();
        mains.push(root.join("crates/yantrik-ui/src/main.rs"));
        let read: Vec<(String, String)> = mains
            .iter()
            .filter_map(|p| {
                let source = std::fs::read_to_string(p).ok()?;
                let name = p.strip_prefix(&root).unwrap_or(p).display().to_string();
                Some((name, source))
            })
            .collect();
        assert!(
            read.len() > 10,
            "only {} mains were read under {}; the scan has stopped finding them",
            read.len(),
            root.display()
        );

        let mut wrong = Vec::new();
        for (name, source) in &read {
            // Without whitespace, so a formatter that breaks the chain across lines cannot hide it.
            let flat: String = source.chars().filter(|c| !c.is_whitespace()).collect();
            if flat.contains(".run().unwrap()") || flat.contains(".run().expect(") {
                wrong.push(format!("{name} unwraps its event loop; use run_until_closed"));
            } else if !flat.contains("run_until_closed(") && !flat.contains("run_until_ended(") {
                wrong.push(format!("{name} does not run its window through run_until_closed"));
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }
}
