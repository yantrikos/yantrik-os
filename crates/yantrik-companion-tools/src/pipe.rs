//! Running a helper program with its input on stdin and its result on stdout.
//!
//! The tools used to hand curl, dot and grim their input and output as files at fixed scratch
//! names (`-d @payload.json`, `-o diagram.png`). A pipe does the same job with no file at all:
//! nothing at a predictable name for anyone to read, swap or plant a link at between our write
//! and the program's open, and nothing to clean up after a failure.

use std::io::Write;
use std::process::{Command, Output, Stdio};

/// Run `cmd` with `input` on its stdin, collecting stdout and stderr.
pub fn run_with_stdin(mut cmd: Command, input: Vec<u8>) -> std::io::Result<Output> {
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdin = child.stdin.take().expect("stdin was piped");
    // From its own thread: a large input (a base64 screenshot) can fill the pipe while the program
    // waits for us to read its output, and each side would wait on the other for ever. Dropping
    // `stdin` when the write ends is what tells the program its input is complete.
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let output = child.wait_with_output()?;
    let _ = writer.join();
    Ok(output)
}

/// POST `body` as JSON to `url` with curl (body on stdin, `--data-binary @-`), failing on an HTTP
/// error (`-f`) or after `max_time_secs`. The caller checks the status and parses stdout.
pub fn post_json(url: &str, max_time_secs: u32, body: Vec<u8>) -> std::io::Result<Output> {
    let mut cmd = Command::new("curl");
    cmd.args(["-fsSL", "--max-time", &max_time_secs.to_string()])
        .args(["-H", "Content-Type: application/json", "--data-binary", "@-", url]);
    run_with_stdin(cmd, body)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn input_goes_in_on_stdin_and_comes_back_on_stdout() {
        // Larger than a pipe buffer, to show the write cannot deadlock against the read.
        let input = vec![b'x'; 1 << 20];
        let out = run_with_stdin(Command::new("cat"), input.clone()).unwrap();
        assert!(out.status.success());
        assert_eq!(out.stdout, input);
    }
}
