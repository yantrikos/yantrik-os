//! Which process holds the person's Browser's DevTools pipe (#1124, gate 5 of the sealed workshop).
//!
//! The Browser runs in the wall, `bwrap … -- chromium`. The pid the shell spawns is the outer
//! `bwrap`; bwrap hands fds 3 and 4 on to the program it runs, and with `--unshare-pid` that is
//! two processes down, past the reaper that is pid 1 of the new pid namespace (a real tree on
//! bubblewrap 0.9: outer bwrap, child of the shell, holding its own signalfd on fd 3 → bwrap
//! reaper → chromium, holding the stream on fds 3 and 4). The service reads `/proc/<pid>/fd/3`
//! of the pid it is told, so the shell tells it the process that really holds the stream: found
//! here by the kernel's own name for the socket, never by position in the tree.

use std::path::Path;

/// How deep under the spawned process the holder may be: the reaper and the program.
const MAX_DEPTH: usize = 3;

fn parent_of(proc: &Path, pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(proc.join(pid.to_string()).join("stat")).ok()?;
    stat.rsplit_once(')')?.1.split_whitespace().nth(1)?.parse().ok()
}

fn holds_stream(proc: &Path, pid: u32, inode: u64) -> bool {
    std::fs::read_link(proc.join(pid.to_string()).join("fd").join("3"))
        .is_ok_and(|link| link.to_string_lossy() == format!("socket:[{inode}]"))
}

fn children_of(proc: &Path, parent: u32) -> Vec<u32> {
    let Ok(dir) = std::fs::read_dir(proc) else { return Vec::new() };
    dir.flatten()
        .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
        .filter(|pid| parent_of(proc, *pid) == Some(parent))
        .collect()
}

/// The process at or below `root` (the one the shell spawned) whose fd 3 is the socket with this
/// inode: `root` itself for a plain launch, the program under the wall's bwrap otherwise. The
/// shallowest holder wins, so the browser and not a helper it started.
pub fn holder_of(proc: &Path, root: u32, inode: u64) -> Option<u32> {
    let mut level = vec![root];
    for _ in 0..=MAX_DEPTH {
        if let Some(pid) = level.iter().find(|pid| holds_stream(proc, **pid, inode)) {
            return Some(*pid);
        }
        level = level.into_iter().flat_map(|pid| children_of(proc, pid)).collect();
        if level.is_empty() {
            return None;
        }
    }
    None
}

/// What the person is told when the Browser they asked for joined one the shell did not start.
pub const NOT_DRIVABLE: &str = "A Browser was already open that Yantrik did not start (a link in another program opened it), \
    so it is outside the protected view and Yantrik cannot drive it. Close it, then open the Browser from the dock again.";

/// Whether the person's account runs a Chromium main process (not one of its helpers) that is
/// not `ours`: a browser the shell did not start with a pipe, which a dock launch then joins and
/// nothing can drive. Read from the command line's first word, which a Chromium keeps.
pub fn another_browser_is_open(proc: &Path, ours: Option<u32>, uid: u32) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Ok(dir) = std::fs::read_dir(proc) else { return false };
    dir.flatten().any(|e| {
        let Some(pid) = e.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { return false };
        let at = proc.join(pid.to_string());
        let Ok(raw) = std::fs::read(at.join("cmdline")) else { return false };
        let line = String::from_utf8_lossy(&raw).replace('\0', " ");
        let mut words = line.split_whitespace();
        let program = words.next().and_then(|w| w.rsplit('/').next()).unwrap_or_default();
        let chromium = matches!(program, "chromium" | "chromium-browser" | "chrome" | "google-chrome" | "google-chrome-stable");
        chromium
            && !line.contains("--type=")
            && Some(pid) != ours
            && std::fs::metadata(&at).is_ok_and(|m| m.uid() == uid)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(proc: &Path, pid: u32, parent: u32, fd3: Option<&str>) {
        let at = proc.join(pid.to_string());
        std::fs::create_dir_all(at.join("fd")).unwrap();
        std::fs::write(at.join("stat"), format!("{pid} (bwrap x) S {parent} 1 1 0\n")).unwrap();
        if let Some(target) = fd3 {
            std::os::unix::fs::symlink(target, at.join("fd").join("3")).unwrap();
        }
    }

    fn tree(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("yb-holder-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The tree seen on a real bwrap: outer (its own signalfd on fd 3), reaper, program.
    #[test]
    fn a_walled_browser_is_found_under_the_outer_bwrap() {
        let proc = tree("wrapped");
        entry(&proc, 100, 1, Some("anon_inode:[signalfd]"));
        entry(&proc, 101, 100, None);
        entry(&proc, 102, 101, Some("socket:[777]"));
        entry(&proc, 103, 102, Some("socket:[999]")); // a helper of the browser
        assert_eq!(holder_of(&proc, 100, 777), Some(102));
        assert_eq!(holder_of(&proc, 100, 555), None, "a stream nobody here holds");
    }

    #[test]
    fn a_plain_launch_is_its_own_holder() {
        let proc = tree("plain");
        entry(&proc, 200, 1, Some("socket:[777]"));
        assert_eq!(holder_of(&proc, 200, 777), Some(200));
    }

    fn browser(proc: &Path, pid: u32, line: &str) {
        entry(proc, pid, 1, None);
        std::fs::write(proc.join(pid.to_string()).join("cmdline"), line.replace(' ', "\0")).unwrap();
    }

    /// The xdg-open case: a Chromium with no pipe, which a dock launch joins.
    #[test]
    fn a_browser_the_shell_did_not_start_is_noticed_and_helpers_and_its_own_are_not() {
        let proc = tree("another");
        let me = unsafe { libc::getuid() };
        browser(&proc, 400, "/usr/lib/chromium/chromium --type=renderer --no-sandbox");
        assert!(!another_browser_is_open(&proc, None, me), "a helper is not a browser");
        browser(&proc, 401, "/usr/lib/chromium/chromium --remote-debugging-pipe");
        assert!(!another_browser_is_open(&proc, Some(401), me), "the shell's own is not another");
        assert!(another_browser_is_open(&proc, None, me));
        assert!(!another_browser_is_open(&proc, None, me + 1), "another account's is not the person's");
    }

    #[test]
    fn a_process_outside_the_spawned_tree_is_never_the_holder() {
        let proc = tree("outside");
        entry(&proc, 300, 1, None);
        entry(&proc, 301, 5, Some("socket:[777]"));
        assert_eq!(holder_of(&proc, 300, 777), None);
    }
}
