//! Which browser holds a loopback port, from facts a page cannot influence: the kernel's table of
//! listening sockets, the processes holding that socket, their account, and the profile each
//! was started with.
//!
//! Nothing here asks the port who it is. A DevTools endpoint can say anything it likes; the
//! socket's owner and the holder's command line are the kernel's.

use std::fs;
use std::path::Path;

use crate::Place;

/// What holds a port.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listener {
    pub port: u16,
    /// The account the socket was made by.
    pub uid: u32,
    /// The socket's inode: a different one later is a different listener.
    pub inode: u64,
    /// The processes holding the socket.
    pub pids: Vec<u32>,
    pub place: Place,
}

/// Why a port's holder could not be established. Every one of these is a refusal to use the
/// port, never a guess.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Why {
    /// Nothing listens on 127.0.0.1 at that port.
    Nobody,
    /// The table or the processes could not be read, or disagree.
    Unclear(String),
}

/// What a reading says when /proc was only momentarily unreadable (a process exec'ing): worth one
/// more look before refusing.
const UNSEEN: &str = "the process holding it cannot be seen";
const UNREADABLE: &str = "command line is unreadable";

impl Why {
    pub fn transient(&self) -> bool {
        matches!(self, Why::Unclear(w) if w.starts_with(UNSEEN) || w.contains(UNREADABLE))
    }
}

impl std::fmt::Display for Why {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Why::Nobody => write!(f, "nothing is listening there"),
            Why::Unclear(why) => write!(f, "{why}"),
        }
    }
}

struct Row {
    local: String,
    uid: u32,
    inode: u64,
}

/// The listening rows of `/proc/net/tcp` for `port`. IPv4 only: the desktop's browsers are
/// started with `--remote-debugging-address=127.0.0.1`, and the client connects there.
fn rows(table: &str, port: u16) -> Vec<Row> {
    let suffix = format!(":{port:04X}");
    table
        .lines()
        .skip(1)
        .filter_map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            let local = f.get(1)?;
            if f.get(3)? != &"0A" || !local.ends_with(&suffix) {
                return None;
            }
            Some(Row { local: local.to_string(), uid: f.get(7)?.parse().ok()?, inode: f.get(9)?.parse().ok()? })
        })
        .collect()
}

/// Who holds 127.0.0.1:`port`, reading `proc` (a `/proc`).
pub fn identify_in(proc: &Path, port: u16) -> Result<Listener, Why> {
    let table = fs::read_to_string(proc.join("net/tcp")).map_err(|e| Why::Unclear(format!("the socket table is unreadable ({e})")))?;
    let all = rows(&table, port);
    // The kernel hands a connection to the exact address before the wildcard one.
    let exact: Vec<&Row> = all.iter().filter(|r| r.local.starts_with("0100007F:")).collect();
    let wild: Vec<&Row> = all.iter().filter(|r| r.local.starts_with("00000000:")).collect();
    let chosen = if exact.is_empty() { wild } else { exact };
    let Some(first) = chosen.first() else { return Err(Why::Nobody) };
    if chosen.iter().any(|r| r.inode != first.inode) {
        return Err(Why::Unclear("two different sockets listen there".into()));
    }
    let want = format!("socket:[{}]", first.inode);
    let mut pids = Vec::new();
    for entry in fs::read_dir(proc).map_err(|e| Why::Unclear(format!("/proc is unreadable ({e})")))?.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
        let Ok(fds) = fs::read_dir(entry.path().join("fd")) else { continue };
        if fds.flatten().any(|fd| fs::read_link(fd.path()).is_ok_and(|l| l.to_str() == Some(want.as_str()))) {
            pids.push(pid);
        }
    }
    if pids.is_empty() {
        return Err(Why::Unclear(UNSEEN.into()));
    }
    pids.sort_unstable();
    let mut place = None;
    for pid in &pids {
        let raw = fs::read(proc.join(pid.to_string()).join("cmdline"))
            .map_err(|e| Why::Unclear(format!("process {pid}'s {UNREADABLE} ({e})")))?;
        let here = crate::place_of(&raw).ok_or_else(|| Why::Unclear(format!("process {pid} names its profile more than once")))?;
        if place.is_some_and(|p| p != here) {
            return Err(Why::Unclear("the processes holding it disagree about whose it is".into()));
        }
        place = Some(here);
    }
    Ok(Listener { port, uid: first.uid, inode: first.inode, pids, place: place.expect("pids is not empty") })
}

/// [`identify_in`] on the real `/proc`, taken again twice, briefly, when it failed only for being
/// momentarily unreadable.
pub fn identify(port: u16) -> Result<Listener, Why> {
    let mut seen = identify_in(Path::new("/proc"), port);
    for _ in 0..2 {
        if !seen.as_ref().is_err_and(Why::transient) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
        seen = identify_in(Path::new("/proc"), port);
    }
    seen
}

/// The account this process runs as.
pub fn own_uid() -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        fs::metadata("/proc/self").map(|m| m.uid()).unwrap_or(u32::MAX)
    }
    #[cfg(not(unix))]
    {
        u32::MAX
    }
}

/// Whether `port` is held by a browser of a mind, run by this account: a Mind View or companion
/// profile. Says why not otherwise, in words a person can read.
pub fn mind_side_in(proc: &Path, port: u16, own: u32) -> Result<Listener, String> {
    match identify_in(proc, port) {
        Ok(l) if l.uid != own => Err(format!("what holds port {port} runs as another account")),
        Ok(l) if !l.place.is_mind_side() => Err(format!(
            "port {port} is held by a browser that is not a mind's own (it may be the person's, with their sign-ins)"
        )),
        Ok(l) => Ok(l),
        Err(Why::Nobody) => Err(format!("no browser of the mind's is listening on port {port}")),
        Err(Why::Unclear(why)) => Err(format!("what holds port {port} could not be established ({why})")),
    }
}

/// [`mind_side_in`] on the real `/proc`.
pub fn mind_side(port: u16) -> Result<Listener, String> {
    mind_side_in(Path::new("/proc"), port, own_uid())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    /// A fake `/proc`: the socket table, and processes with a socket, a command line and nothing else.
    struct Fake(std::path::PathBuf);

    impl Fake {
        fn new(name: &str) -> Fake {
            let root = std::env::temp_dir().join(format!("yb-ports-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(root.join("net")).unwrap();
            Fake(root)
        }
        fn table(&self, rows: &[(&str, u32, u64)]) {
            let mut t = String::from("  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n");
            for (i, (local, uid, inode)) in rows.iter().enumerate() {
                t += &format!("   {i}: {local} 00000000:0000 0A 00000000:00000000 00:00000000 00000000  {uid}        0 {inode} 1 0\n");
            }
            fs::write(self.0.join("net/tcp"), t).unwrap();
        }
        fn process(&self, pid: u32, inode: u64, cmdline: &[u8]) {
            let dir = self.0.join(pid.to_string());
            fs::create_dir_all(dir.join("fd")).unwrap();
            symlink(format!("socket:[{inode}]"), dir.join("fd/7")).unwrap();
            fs::write(dir.join("cmdline"), cmdline).unwrap();
        }
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn line(profile: Option<&Path>) -> Vec<u8> {
        let mut s = String::from("/usr/lib/chromium/chromium --remote-debugging-port=9223");
        if let Some(p) = profile {
            s += &format!(" --user-data-dir={}", p.display());
        }
        s.into_bytes()
    }

    #[test]
    fn mind_views_browser_is_known_by_its_profile_even_when_chromium_joins_its_command_line() {
        let fake = Fake::new("mv");
        fake.table(&[("0100007F:2407", 1000, 555)]);
        fake.process(41, 555, &line(Some(&crate::mind_view_profile())));
        let l = identify_in(&fake.0, 9223).unwrap();
        assert_eq!((l.place, l.uid, l.pids.clone()), (Place::MindView, 1000, vec![41]));
        assert!(mind_side_in(&fake.0, 9223, 1000).is_ok());
    }

    #[test]
    fn a_browser_on_the_default_profile_or_the_persons_own_is_not_a_minds() {
        for (name, profile) in [("d", None), ("o", Some(Path::new("/home/p/.config/chromium")))] {
            let fake = Fake::new(name);
            fake.table(&[("0100007F:2406", 1000, 77)]);
            fake.process(9, 77, &line(profile));
            let why = mind_side_in(&fake.0, 9222, 1000).unwrap_err();
            assert!(why.contains("not a mind's own"), "{why}");
        }
    }

    #[test]
    fn another_account_nothing_and_the_unreadable_are_all_refusals() {
        let fake = Fake::new("x");
        fake.table(&[("0100007F:2407", 1001, 5)]);
        fake.process(3, 5, &line(Some(&crate::mind_view_profile())));
        assert!(mind_side_in(&fake.0, 9223, 1000).unwrap_err().contains("another account"));
        assert_eq!(identify_in(&fake.0, 9999), Err(Why::Nobody));
        // Listening, but the holder is not visible to this account.
        fake.table(&[("0100007F:2407", 1000, 6)]);
        assert!(matches!(identify_in(&fake.0, 9223), Err(Why::Unclear(_))));
        // No table at all.
        fs::remove_file(fake.0.join("net/tcp")).unwrap();
        assert!(matches!(identify_in(&fake.0, 9223), Err(Why::Unclear(_))));
    }

    #[test]
    fn two_sockets_or_holders_that_disagree_are_not_guessed_at() {
        let fake = Fake::new("two");
        fake.table(&[("0100007F:2407", 1000, 1), ("0100007F:2407", 1000, 2)]);
        fake.process(1, 1, &line(Some(&crate::mind_view_profile())));
        assert!(matches!(identify_in(&fake.0, 9223), Err(Why::Unclear(_))));
        fake.table(&[("0100007F:2407", 1000, 1)]);
        fake.process(2, 1, &line(None));
        assert!(matches!(identify_in(&fake.0, 9223), Err(Why::Unclear(why)) if why.contains("disagree")));
    }

    #[test]
    fn the_exact_address_beats_a_wildcard_one_as_the_kernel_does() {
        let fake = Fake::new("wild");
        fake.table(&[("00000000:2407", 1000, 1), ("0100007F:2407", 1000, 2)]);
        fake.process(1, 1, &line(None));
        fake.process(2, 2, &line(Some(&crate::mind_view_profile())));
        assert_eq!(identify_in(&fake.0, 9223).unwrap().place, Place::MindView);
        // A wildcard alone still answers on loopback.
        fake.table(&[("00000000:2407", 1000, 1)]);
        assert_eq!(identify_in(&fake.0, 9223).unwrap().place, Place::PersonDefault);
    }

    #[test]
    fn a_profile_named_twice_is_unclear() {
        let fake = Fake::new("twice");
        fake.table(&[("0100007F:2407", 1000, 1)]);
        fake.process(1, 1, b"chromium --user-data-dir=/a --user-data-dir=/b");
        assert!(matches!(identify_in(&fake.0, 9223), Err(Why::Unclear(_))));
    }
}
