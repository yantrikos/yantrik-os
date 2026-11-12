//! The headless browser's DevTools stream: a connected socket pair the Arcade holds one end of.
//!
//! Chromium started with `--remote-debugging-pipe` reads CDP on its fd 3 and writes it on its
//! fd 4, each message ended by a NUL byte, and listens on no port (#1200). Here the Arcade starts
//! the browser and drives it itself, so there is nothing to hand over: the browser's two fds are
//! one end of a socket pair made here, and the other end is this module's `Pipe`. No other process
//! can reach the stream (a socket, unlike a pipe, cannot be opened again through `/proc/<pid>/fd`),
//! and no other process can find a port to speak to.

use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::process::Command;
use std::time::Duration;

use serde_json::Value;

/// How long a read waits for a message before it says there is none yet: the longest a job takes
/// to notice it has been cancelled.
const LOOK: Duration = Duration::from_millis(150);

/// Makes `command`'s fds 3 and 4 the browser's end of a new stream. Returns this end, and the
/// browser's, which must stay open until the command has been spawned and then be dropped, so the
/// stream ends when the browser does.
pub fn attach(command: &mut Command) -> std::io::Result<(UnixStream, OwnedFd)> {
    use std::os::unix::process::CommandExt as _;
    // Close-on-exec on both ends (std's). Fds the Arcade itself inherited without that flag (a
    // parent's jobserver pipe, a shell's redirections) are closed in the child by `close_from_5`.
    let (mine, theirs) = UnixStream::pair()?;
    // Above 4, so dup2 onto 3 and 4 cannot land on itself; the copy is close-on-exec too.
    // SAFETY: plain fcntl on an fd we own.
    let high = unsafe { libc::fcntl(theirs.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 10) };
    if high < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `high` is a new fd and ours.
    let theirs = unsafe { OwnedFd::from_raw_fd(high) };
    let raw = theirs.as_raw_fd();
    // SAFETY: only async-signal-safe calls (dup2) between fork and exec.
    unsafe {
        command.pre_exec(move || {
            if libc::dup2(raw, 3) < 0 || libc::dup2(raw, 4) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            close_from_5();
            Ok(())
        });
    }
    Ok((mine, theirs))
}

/// Closes every fd from 5 up in the child, so the browser holds fds 0 to 4 and nothing else, even
/// an fd this process inherited that was never close-on-exec. Runs between fork and exec, so it
/// makes raw syscalls only: no allocation, no directory listing.
fn close_from_5() {
    // SAFETY: close_range and close on fds the child owns; both are async-signal-safe.
    unsafe {
        if libc::syscall(libc::SYS_close_range, 5u32, u32::MAX, 0u32) == 0 {
            return;
        }
        // An older kernel: close one by one up to the soft limit, bounded in case it is huge.
        let mut limit = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
        let top = if libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) == 0 { limit.rlim_cur.min(65536) as i32 } else { 1024 };
        for fd in 5..top {
            libc::close(fd);
        }
    }
}

/// This end of the stream: JSON messages out and in, each ended by a NUL.
pub struct Pipe {
    stream: UnixStream,
    buf: Vec<u8>,
}

impl Pipe {
    pub fn new(stream: UnixStream) -> Result<Pipe, String> {
        // Reads must not block for ever: the gate loop polls between them, and a job is cancelled
        // between polls.
        stream.set_read_timeout(Some(LOOK)).map_err(|e| format!("cannot set up the browser's stream: {e}"))?;
        stream.set_write_timeout(Some(Duration::from_secs(30))).map_err(|e| format!("cannot set up the browser's stream: {e}"))?;
        Ok(Pipe { stream, buf: Vec::new() })
    }

    pub fn send(&mut self, message: &Value) -> Result<(), String> {
        let mut bytes = message.to_string().into_bytes();
        bytes.push(0);
        self.stream.write_all(&bytes).map_err(|e| format!("the browser's stream refused a command: {e}"))
    }

    /// The next message, or `None` when none arrived within a short wait. An error is the stream
    /// ending: the browser has gone.
    pub fn read(&mut self) -> Result<Option<Value>, String> {
        loop {
            if let Some(end) = self.buf.iter().position(|b| *b == 0) {
                let frame: Vec<u8> = self.buf.drain(..=end).collect();
                // Something that is not JSON is not a message; the next frame may be.
                match serde_json::from_slice(&frame[..end]) {
                    Ok(v) => return Ok(Some(v)),
                    Err(_) => continue,
                }
            }
            let mut chunk = [0u8; 65536];
            match self.stream.read(&mut chunk) {
                Ok(0) => return Err("the browser closed its stream".into()),
                Ok(n) => self.buf.extend_from_slice(&chunk[..n]),
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => return Ok(None),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(format!("the browser's stream failed: {e}")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn pair() -> (Pipe, UnixStream) {
        let (a, b) = UnixStream::pair().unwrap();
        (Pipe::new(a).unwrap(), b)
    }

    #[test]
    fn a_message_goes_out_ended_by_a_nul_and_comes_in_whole_even_split() {
        let (mut pipe, mut far) = pair();
        pipe.send(&json!({"id": 1, "method": "Runtime.enable"})).unwrap();
        let mut got = [0u8; 64];
        let n = far.read(&mut got).unwrap();
        assert_eq!(got[n - 1], 0, "a NUL ends it");
        assert_eq!(serde_json::from_slice::<Value>(&got[..n - 1]).unwrap()["id"], 1);

        far.write_all(b"{\"id\":2,\"res").unwrap();
        assert_eq!(pipe.read().unwrap(), None, "half a message is not one");
        far.write_all(b"ult\":{}}\0not json\0{\"id\":3}\0").unwrap();
        assert_eq!(pipe.read().unwrap().unwrap()["id"], 2);
        assert_eq!(pipe.read().unwrap().unwrap()["id"], 3, "what is not JSON is skipped");
        assert_eq!(pipe.read().unwrap(), None);
    }

    #[test]
    fn the_stream_ending_is_an_error_and_not_silence() {
        let (mut pipe, far) = pair();
        drop(far);
        assert!(pipe.read().unwrap_err().contains("closed"));
    }

    /// The real thing: a child whose fd 3 is read and fd 4 written is the other end of one stream,
    /// and nothing else of ours came with it.
    #[test]
    fn a_child_reads_its_fd_3_and_writes_its_fd_4_on_the_one_stream() {
        let mut command = Command::new("sh");
        command.arg("-c").arg("read -r line <&3; printf '%s\\0' '{\"id\":7}' >&4; ls /proc/self/fd | tr '\\n' ' ' >&4");
        let (mine, theirs) = attach(&mut command).unwrap();
        let mut child = command.spawn().unwrap();
        drop(theirs);
        let mut pipe = Pipe::new(mine).unwrap();
        pipe.stream.write_all(b"go\n").unwrap();
        let mut message = None;
        for _ in 0..40 {
            if let Some(v) = pipe.read().unwrap() {
                message = Some(v);
                break;
            }
        }
        assert_eq!(message.unwrap()["id"], 7);
        child.wait().unwrap();
        // With the child gone and its end closed here, the stream ends.
        let mut rest = Vec::new();
        let _ = pipe.stream.read_to_end(&mut rest);
        let fds = String::from_utf8_lossy(&rest).to_string();
        for fd in fds.split_whitespace().map(|f| f.parse::<u32>().unwrap()) {
            assert!(fd <= 5, "an fd of ours leaked to the browser: {fds}");
        }
    }
}
