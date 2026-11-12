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

use yantrik_browser_ports::Profiles;

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
/// nothing can drive. Read from the command line's first argument, which a Chromium keeps.
///
/// The Minds' browsers are not that: a browser that owns a Mind's profile (`place_in`: its
/// `SingletonLock` or an open `LOCK`, as the listener decides; a `--user-data-dir` in its text
/// proves nothing) and the headless ones (a whole `--headless` / `--headless=…` argument) run as
/// the person too, and the person cannot close them. A process whose files cannot be read is not
/// counted: nothing says it is the person's.
pub fn another_browser_is_open(proc: &Path, ours: Option<u32>, uid: u32, profiles: &Profiles) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Ok(dir) = std::fs::read_dir(proc) else { return false };
    dir.flatten().any(|e| {
        let Some(pid) = e.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { return false };
        let at = proc.join(pid.to_string());
        let Ok(raw) = std::fs::read(at.join("cmdline")) else { return false };
        let argv: Vec<&[u8]> = raw.split(|b| *b == 0).filter(|a| !a.is_empty()).collect();
        let program = argv.first().and_then(|w| w.rsplit(|b| *b == b'/').next()).unwrap_or_default();
        let chromium = matches!(
            program,
            b"chromium" | b"chromium-browser" | b"chrome" | b"google-chrome" | b"google-chrome-stable"
        );
        chromium
            && !argv.iter().any(|a| a.starts_with(b"--type="))
            && !argv.iter().any(|a| *a == b"--headless" || a.starts_with(b"--headless="))
            && Some(pid) != ours
            && std::fs::metadata(&at).is_ok_and(|m| m.uid() == uid)
            && matches!(yantrik_browser_ports::place_in(proc, profiles, pid), Ok(yantrik_browser_ports::Place::Person))
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

    fn profiles() -> Profiles {
        Profiles { mind_view: "/home/p/.local/share/minds/mind-view".into(), identities: "/home/p/.local/share/minds/identities".into() }
    }

    /// The Minds' browsers run as the person too: they are not "another browser".
    #[test]
    fn a_minds_browser_is_not_another_browser() {
        let me = unsafe { libc::getuid() };
        let proc = tree("minds");
        browser(&proc, 410, "/usr/bin/chromium --user-data-dir=/home/p/.local/share/minds/mind-view --remote-debugging-port=9223");
        assert!(!another_browser_is_open(&proc, None, me, &profiles()), "Mind View's browser");
        browser(&proc, 411, "/usr/bin/chromium --user-data-dir=/home/p/.local/share/minds/identities/work --remote-debugging-port=9300");
        assert!(!another_browser_is_open(&proc, None, me, &profiles()), "a Mind's other identity");
        let proc = tree("headless");
        browser(&proc, 420, "/usr/bin/chromium --headless=new --remote-debugging-port=9300 --user-data-dir=/tmp/x");
        assert!(!another_browser_is_open(&proc, None, me, &profiles()), "a headless browser");
        browser(&proc, 421, "/usr/bin/chromium --headless --disable-gpu");
        assert!(!another_browser_is_open(&proc, None, me, &profiles()));
        // The person's own browser, on a profile of theirs, still counts beside them.
        browser(&proc, 422, "/usr/bin/chromium --user-data-dir=/home/p/.config/chromium");
        assert!(another_browser_is_open(&proc, None, me, &profiles()));
    }

    /// A Mind's browser is known by the profile it owns, not by what its command line says.
    #[test]
    fn a_browser_is_a_minds_by_the_profile_it_owns() {
        let me = unsafe { libc::getuid() };
        let proc = tree("owned");
        let root = tree("owned-profiles");
        let owned = Profiles { mind_view: root.join("mind-view"), identities: root.join("identities") };
        std::fs::create_dir_all(&owned.mind_view).unwrap();
        std::os::unix::fs::symlink("host-430", owned.mind_view.join("SingletonLock")).unwrap();
        browser(&proc, 430, "/usr/bin/chromium --remote-debugging-port=9223");
        assert!(!another_browser_is_open(&proc, None, me, &owned), "owns Mind View's profile, names none");
    }

    /// The three crafted cases: text in a command line that is not an argument of that kind.
    #[test]
    fn crafted_command_lines_do_not_pass_for_a_minds_browser() {
        let me = unsafe { libc::getuid() };
        // 1. A URL that carries `--user-data-dir=<a Mind's profile>`.
        let proc = tree("crafted-url");
        browser(&proc, 440, "/usr/bin/chromium https://x/#--user-data-dir=/home/p/.local/share/minds/identities/a");
        assert!(another_browser_is_open(&proc, None, me, &profiles()), "a URL claimed a Mind's profile");
        // 2. A path that climbs out of the identities directory.
        let proc = tree("crafted-dots");
        browser(&proc, 441, "/usr/bin/chromium --user-data-dir=/home/p/.local/share/minds/identities/../../../.config/chromium");
        assert!(another_browser_is_open(&proc, None, me, &profiles()), "a `..` path claimed a Mind's profile");
        // 3. A file argument that merely begins `--headless` after a space (one argument).
        let proc = tree("crafted-headless");
        let at = proc.join("442");
        entry(&proc, 442, 1, None);
        std::fs::write(at.join("cmdline"), b"/usr/bin/chromium\0/home/p/a --headless.html\0").unwrap();
        assert!(another_browser_is_open(&proc, None, me, &profiles()), "a file name claimed to be headless");
        std::fs::write(at.join("cmdline"), b"/usr/bin/chromium\0--headless.html\0").unwrap();
        assert!(another_browser_is_open(&proc, None, me, &profiles()), "`--headless.html` is not `--headless`");
    }

    /// The xdg-open case: a Chromium with no pipe, which a dock launch joins.
    #[test]
    fn a_browser_the_shell_did_not_start_is_noticed_and_helpers_and_its_own_are_not() {
        let proc = tree("another");
        let me = unsafe { libc::getuid() };
        browser(&proc, 400, "/usr/lib/chromium/chromium --type=renderer --no-sandbox");
        assert!(!another_browser_is_open(&proc, None, me, &profiles()), "a helper is not a browser");
        browser(&proc, 401, "/usr/lib/chromium/chromium --remote-debugging-pipe");
        assert!(!another_browser_is_open(&proc, Some(401), me, &profiles()), "the shell's own is not another");
        assert!(another_browser_is_open(&proc, None, me, &profiles()));
        assert!(!another_browser_is_open(&proc, None, me + 1, &profiles()), "another account's is not the person's");
    }

    #[test]
    fn a_process_outside_the_spawned_tree_is_never_the_holder() {
        let proc = tree("outside");
        entry(&proc, 300, 1, None);
        entry(&proc, 301, 5, Some("socket:[777]"));
        assert_eq!(holder_of(&proc, 300, 777), None);
    }
}
