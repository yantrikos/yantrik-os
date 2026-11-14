//! The verifier's headless Chromium, and the view it starts in (#1124, round 8, finding 2).
//! `Headless::start` builds its command here, so it holds wherever Arcade runs: on the desk, in
//! Mind View, from the command line.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The flags of a headless verifier browser on `url`. `--no-sandbox` is Chromium's own sandbox,
/// which cannot nest here; the view around it is [`walled`].
pub(crate) fn headless_args(profile: &Path, url: &str) -> Vec<String> {
    let mut args: Vec<String> = [
        "--headless=new",
        // Software rendering: WSL here has no GPU path for headless Chromium.
        "--use-gl=angle",
        "--use-angle=swiftshader",
        "--enable-unsafe-swiftshader",
        "--no-sandbox",
        "--disable-dev-shm-usage",
        "--disable-crash-reporter",
        "--mute-audio",
        "--hide-scrollbars",
        "--window-size=1280,720",
        // The DevTools pipe: a stream only this app holds, and no port (#1200).
        yantrik_browser_ports::PIPE_FLAG,
    ]
    .map(String::from)
    .to_vec();
    args.push(format!("--user-data-dir={}", profile.display()));
    args.push(url.to_string());
    args
}

/// `binary args` as a command in the browser view, with the workshops hidden from it (#1124,
/// round 8, finding 2): without the view the page it opens could read every workshop. Refused, with the reason, where that cannot be done.
pub(crate) fn walled(binary: &Path, args: &[String]) -> Result<Command, String> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    walled_at(&home, binary, args)
}

fn walled_at(home: &Path, binary: &Path, args: &[String]) -> Result<Command, String> {
    let (program, argv) = yantrik_ipc_contracts::home_paths::workshops::browser_command(home, &binary.to_string_lossy(), args)?;
    let mut command = Command::new(program);
    command.args(argv);
    Ok(command)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use yantrik_ipc_contracts::home_paths::workshops;

    /// The verifier's browser is started in the view: with the base there, behind a tmpfs over it;
    /// with no bwrap and workshops held, refused; never started bare where it can be walled.
    #[test]
    fn the_verifier_browser_is_started_in_the_view_or_refused() {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
        let got = walled_at(&home, Path::new("/usr/bin/chromium"), &headless_args(Path::new("/tmp/p"), "file:///x.html"));
        match got {
            Ok(c) => {
                let a: Vec<String> = c.get_args().map(|x| x.to_string_lossy().into_owned()).collect();
                assert!(a.iter().any(|x| x == yantrik_browser_ports::PIPE_FLAG) && !a.iter().any(|x| x.starts_with("--remote-debugging-port")) && (c.get_program() == "/usr/bin/chromium" || a.iter().any(|x| x == "/usr/bin/chromium")));
                if workshops::find_bwrap().is_some() {
                    assert!(c.get_program().to_string_lossy().ends_with("bwrap"));
                    assert_eq!(a[..3], ["--dev-bind", "/", "/"]);
                    if Path::new(workshops::BASE).is_dir() {
                        assert_eq!(a[3..5], ["--tmpfs", workshops::BASE]);
                    }
                }
            }
            Err(e) => assert!(e.contains("nothing was run"), "{e}"),
        }
    }

    /// `Headless::start` has no spawn of its own: no `Command::new(binary)` in browser.rs's code.
    #[test]
    fn start_goes_through_the_view() {
        let src = include_str!("browser.rs");
        let code = &src[..src.find("#[cfg(test)]").unwrap_or(src.len())];
        assert!(!code.contains("Command::new(binary)") && code.contains("browser_view::walled("));
    }
}
