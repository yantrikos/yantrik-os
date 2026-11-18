//! Putting a page in front of the person: the Google consent page, or a message's original.

use std::time::Duration;

/// Open `url` with `xdg-open`.
///
/// Detached, and bounded. `xdg-open` normally hands the URL to a handler and exits immediately;
/// on a machine with no handler it exits non-zero, which is the case that has to be *said* rather
/// than swallowed — a click that silently opens nothing is the report this whole change came
/// from. So: spawn, wait a moment and a half for an exit code, and treat "still running" as a
/// browser that is starting. The child is reaped on a thread either way, because a zombie for
/// every sign-in is a small leak in a process that stays open all day.
pub fn open(url: &str) -> Result<(), String> {
    use std::process::{Command, Stdio};

    let mut child = Command::new("xdg-open")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("xdg-open could not be run: {e}"))?;

    let deadline = std::time::Instant::now() + Duration::from_millis(1500);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => {
                return Err(format!(
                    "xdg-open exited {} \u{2014} this machine has no handler for a web address",
                    status.code().map(|c| c.to_string()).unwrap_or_else(|| "on a signal".into())
                ))
            }
            Ok(None) if std::time::Instant::now() >= deadline => {
                // Still running after a second and a half: it has a handler and is waiting on the
                // browser it started. That is a success from here.
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                return Ok(());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(format!("xdg-open could not be waited for: {e}")),
        }
    }
}
