//! What a press inside a reply's words does: a link in a paragraph opens, a code block's Copy
//! copies. Both go through the shell's one path for the thing — `open_url::open`, which refuses
//! anything but an http(s) address, and `wl-copy`, the clipboard the Lens already uses — so a
//! reply cannot open or copy by any way a person's own press could not.

use crate::{App, ReplyActions};
use slint::ComponentHandle;
use std::time::{Duration, Instant};

/// How long a clipboard program may take before it is killed.
const COPY_DEADLINE: Duration = Duration::from_secs(3);

/// Run `program args` with `text` on its stdin, exactly as given, and wait for it for at most
/// `deadline`: a program that stalls is killed rather than waited on. Blocking: call it from a
/// thread of its own, never the UI's.
fn run_copy(program: &str, args: &[&str], text: &str, deadline: Duration) -> Result<(), String> {
    use std::io::Write;
    let mut child = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not run {program}: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        // Its own thread: a program that does not read would block the write past the deadline.
        let text = text.to_owned();
        std::thread::spawn(move || {
            let _ = stdin.write_all(text.as_bytes());
        });
    }
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => return Err(format!("{program} failed: {status}")),
            Ok(None) if started.elapsed() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{program} did not finish in {}s", deadline.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{program}: {e}"));
            }
        }
    }
}

/// Put `text` on the clipboard (`wl-copy`) on a thread of its own, with a deadline, and hand the
/// result to `done` there. The one copy for the shell: a code block's Copy and the calculator's.
/// `done` runs off the UI thread; it reaches the UI with `slint::invoke_from_event_loop`.
pub fn copy_text(text: String, done: impl FnOnce(Result<(), String>) + Send + 'static) {
    let spawned = std::thread::Builder::new().name("reply-copy".into()).spawn(move || {
        done(run_copy("wl-copy", &[], &text, COPY_DEADLINE));
    });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "could not start a copy");
    }
}

pub fn wire(ui: &App) {
    let ui_weak = ui.as_weak();
    let actions = ui.global::<ReplyActions>();
    actions.on_open_link(|url| {
        let url = url.to_string();
        let _ = std::thread::Builder::new()
            .name("reply-link".into())
            .spawn(move || {
                if let Err(e) = crate::open_url::open(&url) {
                    tracing::warn!(error = %e, "a link in a reply was not opened");
                }
            });
    });
    // A code block's Copy: the result goes back to the block that asked, by its id.
    actions.on_copy_text(move |text, id| {
        let ui_weak = ui_weak.clone();
        copy_text(text.to_string(), move |result| {
            if let Err(e) = &result {
                tracing::warn!(error = %e, "a code block was not copied");
            }
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui_weak.upgrade() {
                    let actions = ui.global::<ReplyActions>();
                    actions.set_done_ok(result.is_ok());
                    actions.set_done_id(id);
                }
            });
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECS: Duration = Duration::from_secs(5);

    #[test]
    fn a_program_that_takes_the_text_and_exits_cleanly_is_a_copy() {
        assert_eq!(run_copy("sh", &["-c", "cat >/dev/null"], "x\n  y", SECS), Ok(()));
    }

    #[test]
    fn a_program_that_exits_non_zero_is_a_failed_copy() {
        let err = run_copy("sh", &["-c", "cat >/dev/null; exit 3"], "x", SECS).unwrap_err();
        assert!(err.contains("failed"), "{err}");
    }

    #[test]
    fn a_program_that_is_not_there_is_a_failed_copy() {
        assert!(run_copy("no-such-clipboard-program", &[], "x", SECS).unwrap_err().contains("could not run"));
    }

    #[test]
    fn a_program_that_stalls_is_killed_at_the_deadline() {
        let started = Instant::now();
        let err = run_copy("sh", &["-c", "sleep 30"], "x", Duration::from_millis(200)).unwrap_err();
        assert!(err.contains("did not finish"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(5), "it was waited on for {:?}", started.elapsed());
    }

    #[test]
    fn a_program_that_never_reads_a_large_text_is_still_killed() {
        let big = "x".repeat(1 << 20);
        let err = run_copy("sh", &["-c", "sleep 30"], &big, Duration::from_millis(200)).unwrap_err();
        assert!(err.contains("did not finish"), "{err}");
    }

    #[test]
    fn the_result_reaches_the_caller_off_the_calling_thread() {
        let (tx, rx) = std::sync::mpsc::channel();
        let caller = std::thread::current().id();
        copy_text("x".into(), move |r| tx.send((std::thread::current().id(), r.is_ok() || r.is_err())).unwrap());
        let (worker, _) = rx.recv_timeout(Duration::from_secs(10)).expect("the copy reports back");
        assert_ne!(worker, caller);
    }
}
