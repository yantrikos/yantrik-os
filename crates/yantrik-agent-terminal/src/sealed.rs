//! A sealed command: the same terminal, inside a bubblewrap sandbox that can reach the workshop
//! and nothing else (issue #1120).
//!
//! # What the sandbox denies
//!
//! `--unshare-all` (network, IPC, pids, user, hostname, cgroup), `--die-with-parent`,
//! `--new-session` (no TIOCSTI into the person's terminals) and `--clearenv`. The system is
//! mounted read-only, and only the named files of `/etc`; there is no `$HOME`, no `/run`, no
//! session bus, no Wayland or X socket and no portal. The one writable path is the workshop, and
//! `/tmp` is a fresh tmpfs. `HOME` inside is the workshop.
//!
//! # Caps
//!
//! CPU, memory and wall time are a systemd scope (`systemd-run --user --scope`), the way the
//! capture recorder is run. A machine with no `systemd-run` or no `bwrap` runs nothing sealed:
//! the failure is a refusal, never a command run unsealed.
//!
//! # Why a shell stands between the PTY and bwrap
//!
//! The PTY makes its child a session leader, and `bwrap --new-session` calls `setsid`, which a
//! session leader cannot. So the PTY's child is a small `bash` that runs the scope as its child
//! (`"$@"; exit $?`, never the last command of the string, so bash does not exec it). A kill of
//! that bash's group ends the bash, and `--die-with-parent` ends the sandbox with it.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// What one sealed command may spend.
#[derive(Clone, Debug)]
pub struct Caps {
    /// `MemoryMax`, e.g. `8G`. Swap is held at zero beside it.
    pub memory_max: String,
    /// `CPUQuota` in percent of one core: 400 is four cores.
    pub cpu_percent: u32,
    /// `RuntimeMaxSec`: the scope is stopped after this.
    pub wall: Duration,
}

impl Default for Caps {
    fn default() -> Self {
        Caps { memory_max: "8G".into(), cpu_percent: 400, wall: Duration::from_secs(6 * 3600) }
    }
}

/// A command to run sealed in `workshop`.
#[derive(Clone, Debug)]
pub struct Sealed {
    pub workshop: PathBuf,
    pub caps: Caps,
}

/// The program, arguments and environment of one sealed spawn.
pub(crate) struct Spec {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

/// Directories of the system mounted read-only. A merged-usr `/bin` is a symlink, and is recreated
/// as one.
const SYSTEM_DIRS: [&str; 6] = ["usr", "bin", "sbin", "lib", "lib32", "lib64"];

/// The parts of `/etc` a build needs (certificates, the loader, users, locale), and no more.
const ETC_FILES: [&str; 12] = [
    "ld.so.cache",
    "ld.so.conf",
    "ld.so.conf.d",
    "alternatives",
    "ssl",
    "ca-certificates",
    "passwd",
    "group",
    "nsswitch.conf",
    "localtime",
    "mime.types",
    "os-release",
];

const SYSTEM_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

impl Sealed {
    pub fn new(workshop: PathBuf) -> Sealed {
        Sealed { workshop, caps: Caps::default() }
    }

    /// The `bwrap` arguments, up to and including the `--` before the command. `root` is where
    /// the system is read from (`/`; a test gives its own).
    pub fn bwrap_args(&self, root: &Path, cwd: &Path, env: &[(String, String)]) -> Result<Vec<String>, String> {
        let ws = path_str(&self.workshop)?;
        let cwd = path_str(cwd)?;
        if !self.workshop.is_absolute() || !Path::new(cwd).starts_with(&self.workshop) {
            return Err("a sealed command runs inside its workshop, and `cwd` is not in it.".into());
        }
        let var = |name: &str, fallback: &str| {
            env.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone()).unwrap_or_else(|| fallback.to_string())
        };
        let mut a: Vec<String> = ["--unshare-all", "--die-with-parent", "--new-session", "--clearenv", "--cap-drop", "ALL"]
            .map(String::from)
            .to_vec();
        for dir in SYSTEM_DIRS {
            let host = root.join(dir);
            match std::fs::symlink_metadata(&host) {
                Ok(m) if m.file_type().is_symlink() => {
                    let target = std::fs::read_link(&host).map_err(|e| format!("cannot read /{dir}: {e}"))?;
                    a.extend(["--symlink".into(), path_str(&target)?.to_string(), format!("/{dir}")]);
                }
                Ok(_) => a.extend(["--ro-bind".into(), path_str(&host)?.to_string(), format!("/{dir}")]),
                Err(_) => {}
            }
        }
        for name in ETC_FILES {
            let host = root.join("etc").join(name);
            if std::fs::symlink_metadata(&host).is_ok() {
                a.extend(["--ro-bind".into(), path_str(&host)?.to_string(), format!("/etc/{name}")]);
            }
        }
        a.extend(["--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp"].map(String::from));
        a.extend(["--bind".into(), ws.to_string(), ws.to_string()]);
        let path = format!("{ws}/.local/bin:{SYSTEM_PATH}");
        for (k, v) in [
            ("HOME", ws.to_string()),
            ("USER", var("USER", "mind")),
            ("PATH", path),
            ("LANG", var("LANG", "C.UTF-8")),
            ("TERM", var("TERM", "xterm-256color")),
            ("TMPDIR", "/tmp".to_string()),
            ("PWD", cwd.to_string()),
        ] {
            a.extend(["--setenv".into(), k.into(), v]);
        }
        a.extend(["--chdir".into(), cwd.to_string(), "--".into()]);
        Ok(a)
    }

    /// The `systemd-run` words that hold the caps, up to and including the `--`.
    pub fn scope_args(&self) -> Vec<String> {
        let c = &self.caps;
        let mut a: Vec<String> = ["systemd-run", "--user", "--scope", "--quiet", "--collect"].map(String::from).to_vec();
        for p in [
            format!("MemoryMax={}", c.memory_max),
            "MemorySwapMax=0".to_string(),
            format!("CPUQuota={}%", c.cpu_percent),
            format!("RuntimeMaxSec={}", c.wall.as_secs()),
        ] {
            a.extend(["-p".into(), p]);
        }
        a.push("--".into());
        a
    }

    /// The whole spawn: the outer bash, the scope, the sandbox and the command's own wrapper.
    /// `env` is the scrubbed environment a job would start with.
    pub(crate) fn spec(&self, bash: &str, wrapper: &str, command: &str, cwd: &Path, env: &[(String, String)]) -> Result<Spec, String> {
        let bwrap = find_tool("bwrap")
            .ok_or("sealed mode needs bubblewrap (bwrap), which this machine does not have; nothing was run.")?;
        let systemd_run = find_tool("systemd-run")
            .ok_or("sealed mode caps CPU, memory and time with systemd-run, which this machine does not have; nothing was run.")?;
        let mut args: Vec<String> = ["--noprofile", "--norc", "-c", "\"$@\"; exit $?", "bash"].map(String::from).to_vec();
        let mut scope = self.scope_args();
        scope[0] = systemd_run;
        args.extend(scope);
        args.push(bwrap);
        args.extend(self.bwrap_args(Path::new("/"), cwd, env)?);
        args.extend([bash, "--noprofile", "--norc", "-c", wrapper, "bash", command].map(String::from));
        // What the outer processes need and the sandbox never gets (`--clearenv`): where the user's
        // systemd is.
        let mut env: Vec<(String, String)> = env.to_vec();
        for name in ["XDG_RUNTIME_DIR", "DBUS_SESSION_BUS_ADDRESS"] {
            if let Ok(v) = std::env::var(name) {
                env.push((name.to_string(), v));
            }
        }
        Ok(Spec { program: bash.to_string(), args, env })
    }
}

fn path_str(p: &Path) -> Result<&str, String> {
    p.to_str().filter(|s| !s.chars().any(char::is_control)).ok_or_else(|| format!("{} is not a plain path.", p.display()))
}

fn find_tool(name: &str) -> Option<String> {
    ["/usr/bin", "/bin", "/usr/local/bin"].iter().map(|d| format!("{d}/{name}")).find(|p| Path::new(p).exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> Vec<(String, String)> {
        [("HOME", "/home/p"), ("USER", "p"), ("PATH", "/x"), ("LANG", "C.UTF-8"), ("TERM", "xterm")]
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .to_vec()
    }

    fn argv() -> Vec<String> {
        let s = Sealed::new(PathBuf::from("/home/p/Documents/shop"));
        s.bwrap_args(Path::new("/"), Path::new("/home/p/Documents/shop/app"), &env()).unwrap()
    }

    #[test]
    fn every_denial_is_in_the_argv() {
        let a = argv();
        for flag in ["--unshare-all", "--die-with-parent", "--new-session", "--clearenv"] {
            assert!(a.iter().any(|x| x == flag), "{flag} missing from {a:?}");
        }
        assert!(a.windows(2).any(|w| w == ["--cap-drop", "ALL"]));
        assert!(a.windows(2).any(|w| w == ["--tmpfs", "/tmp"]));
        assert_eq!(a.last().unwrap(), "--");
        for banned in ["--share-net", "--unshare-all-try", "--dev-bind", "--bind-try", "--setenv-all"] {
            assert!(!a.iter().any(|x| x == banned), "{banned} must never appear");
        }
    }

    #[test]
    fn only_the_workshop_is_writable_and_home_is_not_mounted() {
        let a = argv();
        let ws = "/home/p/Documents/shop".to_string();
        let writable: Vec<&[String]> = a.windows(3).filter(|w| w[0] == "--bind").map(|w| &w[1..]).collect();
        assert_eq!(writable, [&[ws.clone(), ws][..]]);
        // The system is read-only, and nothing names home or the run directory as a mount.
        for w in a.windows(3).filter(|w| w[0].contains("bind")) {
            assert!(w[0] == "--bind" || w[0] == "--ro-bind", "{w:?}");
            for p in &w[1..] {
                assert!(p != "/" && p != "/home" && p != "/home/p" && !p.starts_with("/run"), "{w:?}");
            }
        }
        assert!(a.windows(3).all(|w| w[0] != "--setenv" || (w[1] != "DBUS_SESSION_BUS_ADDRESS" && w[1] != "WAYLAND_DISPLAY")));
        assert!(a.windows(3).any(|w| w[0] == "--setenv" && w[1] == "HOME" && w[2] == "/home/p/Documents/shop"));
    }

    #[test]
    fn the_environment_is_built_not_inherited() {
        let setenvs: Vec<String> = argv().windows(2).filter(|w| w[0] == "--setenv").map(|w| w[1].clone()).collect();
        assert_eq!(setenvs, ["HOME", "USER", "PATH", "LANG", "TERM", "TMPDIR", "PWD"]);
    }

    #[test]
    fn a_cwd_outside_the_workshop_is_refused_here_too() {
        let s = Sealed::new(PathBuf::from("/home/p/Documents/shop"));
        assert!(s.bwrap_args(Path::new("/"), Path::new("/home/p"), &env()).is_err());
        assert!(s.bwrap_args(Path::new("/"), Path::new("/home/p/Documents/shop2"), &env()).is_err());
    }

    #[test]
    fn the_caps_are_a_scope() {
        let a = Sealed::new(PathBuf::from("/w")).scope_args().join(" ");
        for want in ["--scope", "MemoryMax=8G", "MemorySwapMax=0", "CPUQuota=400%", "RuntimeMaxSec=21600"] {
            assert!(a.contains(want), "{want} in {a}");
        }
    }

    #[test]
    fn merged_usr_symlinks_stay_symlinks() {
        let root = std::env::temp_dir().join(format!("sealed-root-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("usr/bin")).unwrap();
        std::os::unix::fs::symlink("usr/bin", root.join("bin")).unwrap();
        let s = Sealed::new(PathBuf::from("/w"));
        let a = s.bwrap_args(&root, Path::new("/w"), &env()).unwrap();
        assert!(a.windows(3).any(|w| w == ["--symlink", "usr/bin", "/bin"]), "{a:?}");
        assert!(a.windows(3).any(|w| w[0] == "--ro-bind" && w[2] == "/usr"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
