//! `wlrctl` and other helper processes, asked with a deadline (#1347).
//!
//! `Command::output()` and `.status()` wait for ever on a process that hangs. A `wlrctl` that does
//! not answer (a compositor that is stuck, a socket nobody reads) must cost the caller its
//! deadline and no more: Mind View's start holds a lock while it reads who has the keyboard, and
//! a hung read there would block every later start.

use std::io::Read;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// The longest any single `wlrctl` call is waited for. The compositor answers in milliseconds
/// (windows::COMPOSITOR_REPLY_LIMIT); this is for the ones that do not answer at all.
pub(crate) const WLRCTL_LIMIT: Duration = Duration::from_millis(1500);

/// Run `command` and collect its output, waiting at most `limit`. `Ok(None)` is a process that
/// had not finished by then: it is killed and reaped, and the caller carries on without an
/// answer. `Err` is a process that could not start.
pub(crate) fn output_within(command: &mut Command, limit: Duration) -> std::io::Result<Option<Output>> {
    let mut child = command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
    // Read while waiting: a process that writes more than the pipe holds (64 KiB) blocks until
    // someone reads, and would be taken for one that does not answer.
    let reader = child.stdout.take().map(|mut pipe| {
        std::thread::spawn(move || {
            let mut stdout = Vec::new();
            let _ = pipe.read_to_end(&mut stdout);
            stdout
        })
    });
    let deadline = Instant::now() + limit;
    loop {
        match child.try_wait()? {
            Some(status) => {
                let stdout = reader.map(|r| r.join().unwrap_or_default()).unwrap_or_default();
                return Ok(Some(Output { status, stdout, stderr: Vec::new() }));
            }
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                // The reader ends when the pipe closes; it is not joined, in case something the
                // child started still holds the pipe.
                return Ok(None);
            }
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    }
}

/// `wlrctl <args>`, waited for at most [`WLRCTL_LIMIT`].
pub(crate) fn wlrctl(args: &[&str]) -> std::io::Result<Option<Output>> {
    output_within(Command::new("wlrctl").args(args), WLRCTL_LIMIT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_process_that_answers_is_read() {
        let out = output_within(Command::new("sh").args(["-c", "echo hi; exit 3"]), Duration::from_secs(5)).unwrap().unwrap();
        assert_eq!(out.status.code(), Some(3));
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hi");
    }

    #[test]
    fn a_process_that_hangs_costs_its_deadline_and_is_killed() {
        let started = Instant::now();
        let out = output_within(Command::new("sleep").arg("30"), Duration::from_millis(200)).unwrap();
        assert!(out.is_none(), "no answer");
        assert!(started.elapsed() < Duration::from_secs(5), "it did not wait for the 30 seconds");
    }

    /// More than a pipe holds: it must be read while the process is waited for.
    #[test]
    fn output_larger_than_the_pipe_is_read_whole() {
        let out = output_within(Command::new("sh").args(["-c", "head -c 300000 /dev/zero"]), Duration::from_secs(10)).unwrap().expect("answered in time");
        assert!(out.status.success());
        assert_eq!(out.stdout.len(), 300_000);
    }

    #[test]
    fn a_program_that_is_missing_is_an_error_not_a_wait() {
        assert!(output_within(&mut Command::new("yantrik-no-such-program"), Duration::from_secs(1)).is_err());
    }
}
