//! Putting text on the clipboard without ever holding the UI thread: `wl-copy` runs on a thread
//! of its own, under a deadline, and the result comes back to whoever asked. A `wl-copy` that is
//! missing, exits with an error, or never finishes is a failure to report, not a frozen shell.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long `wl-copy` has to take the text before it is killed.
pub const DEADLINE: Duration = Duration::from_secs(3);

/// Run `program args`, hand it `text` on stdin, and wait for it up to `deadline`, killing it after.
/// Blocks the calling thread: call it from one that may.
pub fn run_with(
    program: &str,
    args: &[&str],
    text: &str,
    deadline: Duration,
) -> Result<(), String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("could not run {program}: {e}"))?;
    // Written from its own thread: a program that never reads must not hold this one before the
    // deadline can be kept. Killing the child closes the pipe and ends the writer.
    let writer = child.stdin.take().map(|mut stdin| {
        let text = text.to_owned();
        std::thread::spawn(move || {
            let _ = stdin.write_all(text.as_bytes());
        })
    });
    let started = Instant::now();
    let outcome = loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break Ok(()),
            Ok(Some(status)) => break Err(format!("{program} failed: {status}")),
            Ok(None) if started.elapsed() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(format!(
                    "{program} did not finish in {} s",
                    deadline.as_secs_f32()
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => break Err(format!("{program}: {e}")),
        }
    };
    // Joined only when the program finished: it has then closed its end and the writer is done. After
    // a kill the writer may still hold a pipe a grandchild keeps open, and is left to end on its own.
    if let (Ok(()), Some(writer)) = (&outcome, writer) {
        let _ = writer.join();
    }
    outcome
}

/// Copy `text` with `wl-copy` on a thread of its own, and call `done` there with how it went. The
/// caller returns at once.
pub fn copy_async(text: String, done: impl FnOnce(Result<(), String>) + Send + 'static) {
    let spawned = std::thread::Builder::new()
        .name("copy-text".into())
        .spawn(move || {
            done(run_with("wl-copy", &[], &text, DEADLINE));
        });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "could not start the copy thread");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(script: &str, text: &str, deadline: Duration) -> Result<(), String> {
        run_with("sh", &["-c", script], text, deadline)
    }

    #[test]
    fn a_copy_that_succeeds_hands_the_program_the_exact_text() {
        let out = std::env::temp_dir().join(format!("copy-text-{}.txt", std::process::id()));
        let text = "fn main() {\n    let x = 1;\n\n}\n";
        let script = format!("cat > {}", out.display());
        assert_eq!(sh(&script, text, Duration::from_secs(5)), Ok(()));
        assert_eq!(std::fs::read_to_string(&out).unwrap(), text);
        let _ = std::fs::remove_file(out);
    }

    #[test]
    fn a_program_that_exits_with_an_error_is_a_failure() {
        let failed = sh("cat >/dev/null; exit 3", "x", Duration::from_secs(5));
        assert!(failed.unwrap_err().contains("failed"));
    }

    #[test]
    fn a_program_that_is_missing_is_a_failure() {
        assert!(run_with("no-such-clipboard-program", &[], "x", DEADLINE)
            .unwrap_err()
            .contains("could not run"));
    }

    /// A program that never finishes is killed at the deadline, and the caller gets its answer
    /// then, not when the program decides.
    #[test]
    fn a_program_that_never_finishes_is_killed_at_the_deadline() {
        let started = Instant::now();
        let failed = sh("sleep 30", "x", Duration::from_millis(300));
        assert!(failed.unwrap_err().contains("did not finish"));
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "took {:?}",
            started.elapsed()
        );
    }

    /// Text larger than a pipe holds, to a program that never reads it: still the deadline's answer.
    #[test]
    fn a_program_that_never_reads_a_large_text_does_not_hold_the_caller() {
        let started = Instant::now();
        let failed = sh("sleep 30", &"x".repeat(1 << 20), Duration::from_millis(300));
        assert!(failed.is_err());
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn copy_async_returns_at_once_and_reports_on_its_own_thread() {
        let (tx, rx) = std::sync::mpsc::channel();
        let started = Instant::now();
        copy_async("x".into(), move |result| {
            let _ = tx.send((
                result.is_ok() || result.is_err(),
                std::thread::current().name().map(str::to_owned),
            ));
        });
        assert!(started.elapsed() < Duration::from_millis(500));
        let (_, thread) = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("done is called");
        assert_eq!(thread.as_deref(), Some("copy-text"));
    }
}
