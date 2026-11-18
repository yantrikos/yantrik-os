//! Playing a game in the person's Browser, which the desktop opens (#1137, #1151).
//!
//! That Browser is driven over a DevTools pipe the shell makes when it starts it, so a game is not
//! opened there by CDP and not watched. Only the shell starts it: a Chromium started from here
//! while none is open would become the primary instance, without a pipe, and the shell's own
//! launch would then forward to it and leave a Browser nothing can drive. So the shell is asked,
//! and the game is handed to the Browser only once it runs with its pipe and holds the profile's
//! singleton lock: the pipe switch shows from the moment of exec, seconds before a cold Chromium
//! takes the lock, and a second launch in that window would win it (review of aff60d15, L7).

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::play::{ask_shell_for_browser, file_url};

const PIPE_FLAG: &[u8] = b"--remote-debugging-pipe";
/// A cold start on a small VM takes several seconds (the shell waits 12 s for a window).
const BROWSER_UP_WITHIN: Duration = Duration::from_secs(25);
const LOOK_EVERY: Duration = Duration::from_millis(300);

pub fn play(abs: &Path) -> Result<String, String> {
    ask_shell_for_browser()?;
    let program = installed_browser().ok_or("no web browser is installed")?;
    let profile = profile_of(program, &config_home());
    let ready = || browser_with_the_pipe(Path::new("/proc")).is_some_and(|pid| profile.as_deref().is_none_or(|p| singleton_is(p, pid)));
    wait_until(BROWSER_UP_WITHIN, ready).map_err(|()| {
        format!(
            "the desktop's Browser did not come up with its DevTools pipe and its profile's lock within {} s, so the game was not handed to it",
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

/// The pid of a process of this account that runs with the DevTools pipe switch: the shell's Browser.
fn browser_with_the_pipe(proc: &Path) -> Option<u32> {
    let me = std::fs::metadata(proc.join("self")).map(|m| m.uid()).ok()?;
    std::fs::read_dir(proc).ok()?.flatten().find_map(|entry| {
        let pid: u32 = entry.file_name().to_str()?.parse().ok()?;
        let mine = entry.metadata().is_ok_and(|m| m.uid() == me);
        let piped = std::fs::read(entry.path().join("cmdline")).is_ok_and(|c| has_the_pipe_switch(&c));
        (mine && piped).then_some(pid)
    })
}

/// Whether a `/proc/<pid>/cmdline` carries the pipe switch as a word. About 0.85 s after it starts
/// Chromium folds its whole argument vector into argv[0] as one space-joined string, so the first
/// element is read as words and the rest as given (the 520 gate on #1151).
fn has_the_pipe_switch(cmdline: &[u8]) -> bool {
    yantrik_browser_ports::cmdline_words(cmdline).iter().any(|arg| *arg == PIPE_FLAG)
}

/// Whether Chromium's `SingletonLock` in `profile` names `pid`: a symlink to `<host>-<pid>`, made
/// once that browser is the profile's primary instance. Only then does a second launch hand its
/// address to it instead of becoming a primary of its own.
fn singleton_is(profile: &Path, pid: u32) -> bool {
    std::fs::read_link(profile.join("SingletonLock"))
        .ok()
        .and_then(|target| target.to_str()?.rsplit('-').next()?.parse::<u32>().ok())
        == Some(pid)
}

/// Where `program` keeps its default profile under the XDG config home, for the browsers whose
/// lock is such a symlink. None for the others (Firefox): the pipe switch alone is then the sign.
fn profile_of(program: &str, config: &Path) -> Option<PathBuf> {
    match program {
        "chromium" | "chromium-browser" => Some(config.join("chromium")),
        "google-chrome-stable" | "google-chrome" => Some(config.join("google-chrome")),
        _ => None,
    }
}

/// `$XDG_CONFIG_HOME`, or `~/.config`: where Chromium puts its default profile.
fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(".config"))
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
        assert_eq!(browser_with_the_pipe(&proc), Some(12));
    }

    /// The 520 failure: a running Chromium has folded its arguments into argv[0], one string.
    #[test]
    fn a_browser_that_retitled_itself_still_has_the_pipe_switch() {
        assert!(has_the_pipe_switch(b"/usr/lib/chromium/chromium --no-first-run --remote-debugging-pipe\0"));
        assert!(!has_the_pipe_switch(b"/usr/lib/chromium/chromium --remote-debugging-pipeline\0"));
        assert!(!has_the_pipe_switch(b"/usr/lib/chromium/chromium --no-first-run\0"));
    }

    /// A Browser started outside the shell has no pipe, and Mind View's has a port: neither counts.
    #[test]
    fn a_browser_without_the_pipe_does_not_count() {
        let proc = proc_with("nopipe", &[("11", "bash"), ("12", "chromium file:///g/index.html"), ("13", "chromium --remote-debugging-port=9223")]);
        assert_eq!(browser_with_the_pipe(&proc), None);
    }

    /// L7: the lock names the primary instance; until it names the piped browser, a second launch
    /// could win the profile.
    #[test]
    fn the_singleton_lock_must_name_the_piped_browser() {
        let profile = std::env::temp_dir().join(format!("arcade-profile-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&profile);
        std::fs::create_dir_all(&profile).unwrap();
        assert!(!singleton_is(&profile, 12), "no lock yet: Chromium is still starting");
        std::os::unix::fs::symlink("yantrik-box-12", profile.join("SingletonLock")).unwrap();
        assert!(singleton_is(&profile, 12));
        assert!(!singleton_is(&profile, 13), "another instance holds the profile");
        assert_eq!(profile_of("chromium", Path::new("/home/p/.config")), Some(PathBuf::from("/home/p/.config/chromium")));
        assert_eq!(profile_of("google-chrome", Path::new("/home/p/.config")), Some(PathBuf::from("/home/p/.config/google-chrome")));
        assert_eq!(profile_of("firefox", Path::new("/home/p/.config")), None);
    }

    #[test]
    fn waiting_gives_up_loudly_after_its_budget() {
        assert_eq!(wait_until(Duration::from_millis(50), || false), Err(()));
        let mut looks = 0;
        assert_eq!(wait_until(Duration::from_secs(5), || { looks += 1; looks == 3 }), Ok(()));
    }
}
