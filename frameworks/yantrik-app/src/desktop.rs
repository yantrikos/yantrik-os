//! The desktop, through one typed door (design/app-framework-2026-10-21.md, §8): opening a URL
//! or a file with whatever the person uses for it, and putting text on the clipboard. An app asks
//! with `fx.desktop(request, |outcome| Command)`; the work runs as a job, and the outcome comes
//! back as the command, so an `update` never spawns a process or blocks on one. File pickers
//! and drag-and-drop join here when the first app on the framework needs them (§10).

use std::path::PathBuf;
use std::process::{Command, Stdio};

/// What is asked of the desktop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DesktopRequest {
    /// Open a web address in the person's browser. `http`, `https` and `mailto` only.
    OpenUrl(String),
    /// Open a file or folder with what the person uses for its kind.
    OpenPath(PathBuf),
    /// Put text on the clipboard.
    CopyText(String),
}

/// What came of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DesktopOutcome {
    Done,
    /// It did not happen, with the sentence to show.
    Failed(String),
}

/// The schemes an app may hand to the browser. Anything else (`file:`, `javascript:`, a custom
/// scheme that launches a program) is refused here, before a process is started.
const OPENABLE_SCHEMES: [&str; 3] = ["http://", "https://", "mailto:"];

/// Does the request, blocking: only ever called on a job thread.
pub fn perform(request: DesktopRequest) -> DesktopOutcome {
    match request {
        DesktopRequest::OpenUrl(url) => open_url(&url),
        DesktopRequest::OpenPath(path) => open_path(&path),
        DesktopRequest::CopyText(text) => copy_text(&text),
    }
}

fn open_url(url: &str) -> DesktopOutcome {
    let url = url.trim();
    if url.is_empty() || url.len() > 8 * 1024 {
        return DesktopOutcome::Failed("That is not a web address.".into());
    }
    let lower = url.to_ascii_lowercase();
    if !OPENABLE_SCHEMES.iter().any(|s| lower.starts_with(s)) {
        return DesktopOutcome::Failed(
            "Only web (http, https) and mail addresses are opened from here.".into(),
        );
    }
    if url.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return DesktopOutcome::Failed("That is not a web address.".into());
    }
    // A mail link may name a file to attach (`?attach=`, `?attachment=`), which mail clients
    // have honoured with local paths. A draft that carries a file the app named is not opened
    // from here.
    if lower.starts_with("mailto:") && mailto_attaches(&lower) {
        return DesktopOutcome::Failed("A mail link that attaches a file is not opened from here.".into());
    }
    launch("xdg-open", url)
}

/// Whether a `mailto:` link's query names an attachment, under any of the keys clients read.
fn mailto_attaches(lower: &str) -> bool {
    let Some((_, query)) = lower.split_once('?') else { return false };
    query
        .split('&')
        .filter_map(|pair| pair.split_once('=').map(|(k, _)| k).or(Some(pair)))
        .any(|key| matches!(key.trim(), "attach" | "attachment" | "attachments"))
}

fn open_path(path: &std::path::Path) -> DesktopOutcome {
    if !path.is_absolute() {
        return DesktopOutcome::Failed("A file is opened by its full path.".into());
    }
    if !path.exists() {
        return DesktopOutcome::Failed(format!(
            "{} is not there any more.",
            yantrik_foundation::home::shown(path)
        ));
    }
    // The desktop's handler for a `.desktop` file or an executable is to run it (§8: a picked
    // file is a capability handle, not a program). Until pickers land, such a file is refused.
    if runs_as_a_program(path) {
        return DesktopOutcome::Failed(format!(
            "{} is a program, not a document; it is not opened from here.",
            yantrik_foundation::home::shown(path)
        ));
    }
    launch("xdg-open", &path.to_string_lossy())
}

/// A `.desktop` launcher, or a regular file with an execute bit.
fn runs_as_a_program(path: &std::path::Path) -> bool {
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("desktop")) {
        return true;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if let Ok(meta) = std::fs::metadata(path) {
            return meta.is_file() && meta.permissions().mode() & 0o111 != 0;
        }
    }
    false
}

/// Starts the opener and lets it go: a browser may run for an hour, and the app does not wait
/// for it. Failing to start it is the one failure there is to report. The opener is reaped on a
/// thread of its own when it exits (`xdg-open` returns once it has handed off), so no defunct
/// entry sits under the app for every open; it runs in its own process group, so a signal to
/// the app's does not reach what it opened.
fn launch(program: &str, argument: &str) -> DesktopOutcome {
    let mut command = Command::new(program);
    command
        .arg(argument)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        command.process_group(0);
    }
    match command.spawn() {
        Ok(mut child) => {
            std::thread::Builder::new()
                .name("desktop-reap".into())
                .spawn(move || {
                    let _ = child.wait();
                })
                .ok();
            DesktopOutcome::Done
        }
        Err(e) => DesktopOutcome::Failed(format!("Nothing on this machine opens that ({e}).")),
    }
}

fn copy_text(text: &str) -> DesktopOutcome {
    use std::io::Write as _;
    let mut child = match Command::new("wl-copy")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return DesktopOutcome::Failed(format!("The clipboard is not available ({e})."));
        }
    };
    if let Some(mut stdin) = child.stdin.take() {
        if let Err(e) = stdin.write_all(text.as_bytes()) {
            return DesktopOutcome::Failed(format!("The text could not be copied ({e})."));
        }
    }
    match child.wait() {
        Ok(status) if status.success() => DesktopOutcome::Done,
        Ok(status) => DesktopOutcome::Failed(format!("The clipboard refused it ({status}).")),
        Err(e) => DesktopOutcome::Failed(format!("The text could not be copied ({e})."),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mail_link_that_attaches_a_file_is_refused() {
        assert!(mailto_attaches("mailto:x@y?subject=hi&attach=/home/u/.ssh/id_ed25519"));
        assert!(mailto_attaches("mailto:x@y?attachment=/etc/passwd"));
        assert!(!mailto_attaches("mailto:x@y?subject=attach%20this"));
        assert!(matches!(open_url("mailto:x@y?attach=/x"), DesktopOutcome::Failed(_)));
    }

    #[test]
    fn a_program_is_not_opened_as_a_document() {
        let dir = std::env::temp_dir().join(format!("yantrik-desktop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let launcher = dir.join("evil.desktop");
        std::fs::write(&launcher, "[Desktop Entry]\nExec=rm -rf ~\n").unwrap();
        assert!(runs_as_a_program(&launcher));
        let note = dir.join("note.txt");
        std::fs::write(&note, "hello").unwrap();
        assert!(!runs_as_a_program(&note));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let script = dir.join("run.sh");
            std::fs::write(&script, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
            assert!(runs_as_a_program(&script));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Refused before any process is started: nothing here opens a browser on the test machine.
    #[test]
    fn what_is_refused_before_anything_is_started() {
        for bad in ["", "file:///etc/passwd", "javascript:alert(1)", "ftp://x", "http://a b", " "] {
            assert!(
                matches!(open_url(bad), DesktopOutcome::Failed(_)),
                "{bad:?} must be refused"
            );
        }
        assert!(matches!(
            open_path(std::path::Path::new("relative/thing")),
            DesktopOutcome::Failed(_)
        ));
        let gone = std::env::temp_dir().join("yantrik-desktop-no-such-file");
        let DesktopOutcome::Failed(said) = open_path(&gone) else { panic!("failed") };
        assert!(said.contains("not there"), "{said}");
    }
}
