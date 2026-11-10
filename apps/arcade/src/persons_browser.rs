//! Playing a game in the person's Browser, which the desktop opens (#1137, #1151).
//!
//! That Browser is driven over a DevTools pipe the shell makes when it starts it, so a game is not
//! opened there by CDP and not watched. Only the shell starts it: a Chromium started from here
//! while none is open would become the primary instance, without a pipe, and the shell's own
//! launch would then forward to it and leave a Browser nothing can drive. So the shell is asked,
//! and the game is handed to the Browser only once it runs with its pipe.

use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::play::{ask_shell_for_browser, file_url};

const PIPE_FLAG: &[u8] = b"--remote-debugging-pipe";
/// A cold start on a small VM takes several seconds (the shell waits 12 s for a window).
const BROWSER_UP_WITHIN: Duration = Duration::from_secs(25);
const LOOK_EVERY: Duration = Duration::from_millis(300);

pub fn play(abs: &Path) -> Result<String, String> {
    ask_shell_for_browser()?;
    let program = installed_browser().ok_or("no web browser is installed")?;
    wait_until(BROWSER_UP_WITHIN, || a_browser_has_the_pipe(Path::new("/proc"))).map_err(|()| {
        format!(
            "the desktop's Browser did not come up with its DevTools pipe within {} s, so the game was not handed to it",
            BROWSER_UP_WITHIN.as_secs()
        )
    })?;
    // A second launch of a browser already running hands the address to it and exits.
    std::process::Command::new(program)
        .arg(file_url(abs))
        .stdin(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("the Browser could not be handed the game: {e}"))?;
    let name = abs.file_name().and_then(|n| n.to_str()).unwrap_or("the game");
    Ok(format!("opened {name} in the desktop Browser; whether it draws was not checked (the Browser has no debugging port)"))
}

fn wait_until(budget: Duration, mut ready: impl FnMut() -> bool) -> Result<(), ()> {
    let deadline = Instant::now() + budget;
    while !ready() {
        if Instant::now() > deadline {
            return Err(());
        }
        std::thread::sleep(LOOK_EVERY);
    }
    Ok(())
}

/// Whether a process of this account runs with the DevTools pipe switch: the shell's Browser.
fn a_browser_has_the_pipe(proc: &Path) -> bool {
    let Ok(me) = std::fs::metadata(proc.join("self")).map(|m| m.uid()) else { return false };
    let Ok(entries) = std::fs::read_dir(proc) else { return false };
    entries.flatten().any(|entry| {
        entry.file_name().to_string_lossy().bytes().all(|b| b.is_ascii_digit())
            && entry.metadata().is_ok_and(|m| m.uid() == me)
            && std::fs::read(entry.path().join("cmdline")).is_ok_and(|c| c.split(|b| *b == 0).any(|arg| arg == PIPE_FLAG))
    })
}

fn installed_browser() -> Option<&'static str> {
    ["chromium", "chromium-browser", "google-chrome-stable", "google-chrome", "firefox", "firefox-esr"]
        .into_iter()
        .find(|name| std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(name).is_file())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proc_with(name: &str, cmdlines: &[(&str, &str)]) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("arcade-proc-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (pid, cmdline) in cmdlines {
            std::fs::create_dir_all(root.join(pid)).unwrap();
            std::fs::write(root.join(pid).join("cmdline"), cmdline.replace(' ', "\0")).unwrap();
        }
        std::os::unix::fs::symlink(root.join(cmdlines[0].0), root.join("self")).unwrap();
        root
    }

    #[test]
    fn the_browser_the_shell_started_has_the_pipe_switch() {
        let proc = proc_with("pipe", &[("11", "bash"), ("12", "/usr/lib/chromium/chromium --remote-debugging-pipe --no-first-run")]);
        assert!(a_browser_has_the_pipe(&proc));
    }

    /// A Browser started outside the shell has no pipe, and Mind View's has a port: neither counts.
    #[test]
    fn a_browser_without_the_pipe_does_not_count() {
        let proc = proc_with("nopipe", &[("11", "bash"), ("12", "chromium file:///g/index.html"), ("13", "chromium --remote-debugging-port=9223")]);
        assert!(!a_browser_has_the_pipe(&proc));
    }

    #[test]
    fn waiting_gives_up_loudly_after_its_budget() {
        assert_eq!(wait_until(Duration::from_millis(50), || false), Err(()));
        let mut looks = 0;
        assert_eq!(wait_until(Duration::from_secs(5), || { looks += 1; looks == 3 }), Ok(()));
    }
}
