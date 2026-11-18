//! Which browser holds a loopback port, from facts a page cannot influence: the kernel's table of
//! listening sockets, the processes holding that socket, their account, and which profile each
//! process OWNS (its `SingletonLock`, or a `LOCK` file it holds open).
//!
//! Nothing here reads a command line. A real Chromium's `/proc/<pid>/cmdline` comes in several
//! shapes (NUL separated; one space-joined string; that with a trailing NUL), a process can
//! retitle itself, and a URL can carry `--user-data-dir=`: text proves nothing either way.
//! The files a process holds are what it cannot fake (the 520 gate, on #1088).

use std::fs;
use std::path::Path;

use crate::{place_in, Place, Profiles};

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
const UNREADABLE: &str = "could not be read";

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

/// Every TCP socket in LISTEN state in `proc`'s tables (IPv4 and IPv6) as (port, socket inode),
/// whatever its address or account.
pub fn listening_in(proc: &Path) -> Vec<(u16, u64)> {
    ["net/tcp", "net/tcp6"]
        .iter()
        .filter_map(|table| fs::read_to_string(proc.join(table)).ok())
        .flat_map(|text| {
            text.lines()
                .skip(1)
                .filter_map(|line| {
                    let f: Vec<&str> = line.split_whitespace().collect();
                    if f.get(3)? != &"0A" {
                        return None;
                    }
                    let port = u16::from_str_radix(f.get(1)?.rsplit(':').next()?, 16).ok()?;
                    Some((port, f.get(9)?.parse().ok()?))
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// The socket on 127.0.0.1:`port` and the processes holding it: (account, inode, pids).
pub fn holders_in(proc: &Path, port: u16) -> Result<(u32, u64, Vec<u32>), Why> {
    let table = fs::read_to_string(proc.join("net/tcp")).map_err(|e| Why::Unclear(format!("the socket table {UNREADABLE} ({e})")))?;
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
    for entry in fs::read_dir(proc).map_err(|e| Why::Unclear(format!("/proc {UNREADABLE} ({e})")))?.flatten() {
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
    Ok((first.uid, first.inode, pids))
}

/// Who holds 127.0.0.1:`port`, reading `proc` (a `/proc`): a mind's browser when a holder owns a
/// mind's profile, the person's (or nobody's we know) when none does.
pub fn identify_in(proc: &Path, port: u16, profiles: &Profiles) -> Result<Listener, Why> {
    let (uid, inode, pids) = holders_in(proc, port)?;
    let mut place = Place::Person;
    for pid in &pids {
        let here = place_in(proc, profiles, *pid).map_err(Why::Unclear)?;
        if here.is_mind_side() {
            place = here;
        }
    }
    Ok(Listener { port, uid, inode, pids, place })
}

/// [`identify_in`] on the real `/proc`, taken again twice, briefly, when it failed only for being
/// momentarily unreadable.
pub fn identify(port: u16) -> Result<Listener, Why> {
    let profiles = Profiles::current();
    let mut seen = identify_in(Path::new("/proc"), port, &profiles);
    for _ in 0..2 {
        if !seen.as_ref().is_err_and(Why::transient) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
        seen = identify_in(Path::new("/proc"), port, &profiles);
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

/// Whether `port` is held by a browser of a mind, run by `own`. Says why not otherwise, in words
/// a person can read.
pub fn mind_side_in(proc: &Path, port: u16, own: u32, profiles: &Profiles) -> Result<Listener, String> {
    match identify_in(proc, port, profiles) {
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
    mind_side_in(Path::new("/proc"), port, own_uid(), &Profiles::current())
}

#[cfg(all(test, unix))]
pub(crate) mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use std::path::PathBuf;

    /// A fake `/proc` and the profile directories the processes in it own or do not.
    pub struct Fake {
        pub root: PathBuf,
        pub profiles: Profiles,
    }

    impl Fake {
        pub fn new(name: &str) -> Fake {
            let root = std::env::temp_dir().join(format!("yb-ports-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(root.join("proc/net")).unwrap();
            let profiles = Profiles { mind_view: root.join("data/mind-view/chromium"), identities: root.join("data/browsers") };
            fs::create_dir_all(&profiles.mind_view).unwrap();
            fs::create_dir_all(&profiles.identities).unwrap();
            Fake { root, profiles }
        }
        pub fn proc(&self) -> PathBuf {
            self.root.join("proc")
        }
        pub fn table(&self, rows: &[(&str, u32, u64)]) {
            let mut t = String::from("  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n");
            for (i, (local, uid, inode)) in rows.iter().enumerate() {
                t += &format!("   {i}: {local} 00000000:0000 0A 00000000:00000000 00:00000000 00000000  {uid}        0 {inode} 1 0\n");
            }
            fs::write(self.proc().join("net/tcp"), t).unwrap();
        }
        /// A process holding socket `inode`, with `cmdline` as its (irrelevant) command line.
        pub fn process(&self, pid: u32, inode: u64, cmdline: &[u8]) {
            let dir = self.proc().join(pid.to_string());
            fs::create_dir_all(dir.join("fd")).unwrap();
            let link = dir.join("fd/7");
            let _ = fs::remove_file(&link);
            symlink(format!("socket:[{inode}]"), link).unwrap();
            fs::write(dir.join("cmdline"), cmdline).unwrap();
        }
        /// `pid` takes `profile`, as Chromium does: a `SingletonLock` link naming it.
        pub fn takes(&self, profile: &Path, pid: u32) {
            let lock = profile.join("SingletonLock");
            let _ = fs::remove_file(&lock);
            symlink(format!("{}-{pid}", crate::this_host().unwrap_or("h".into())), lock).unwrap();
        }
        /// `pid` holds a `LOCK` file under `profile` open.
        pub fn holds_lock(&self, pid: u32, profile: &Path) {
            let file = profile.join("Local Storage/LOCK");
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(&file, b"").unwrap();
            let fds = self.proc().join(pid.to_string()).join("fd");
            fs::create_dir_all(&fds).unwrap();
            let _ = fs::remove_file(fds.join("9"));
            symlink(&file, fds.join("9")).unwrap();
        }
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    /// The three shapes a real Chromium's `/proc/<pid>/cmdline` has been seen in; none of them may
    /// matter. The third is what a packaged Chromium showed on 520: one space-joined string and a
    /// trailing NUL.
    fn shapes(profile: &Path) -> Vec<Vec<u8>> {
        let flag = format!("--user-data-dir={}", profile.display());
        let words = ["/usr/lib/chromium/chromium", "--remote-debugging-port=9444", flag.as_str(), "about:blank"];
        vec![
            format!("{}\0", words.join("\0")).into_bytes(),
            words.join(" ").into_bytes(),
            format!("{}\0", words.join(" ")).into_bytes(),
        ]
    }

    #[test]
    fn mind_views_browser_is_known_by_what_it_owns_whatever_shape_its_command_line_has() {
        for (n, line) in shapes(Path::new("/x")).into_iter().enumerate() {
            let fake = Fake::new(&format!("mv{n}"));
            fake.table(&[("0100007F:24E4", 1000, 555)]);
            fake.process(41, 555, &line);
            fake.takes(&fake.profiles.mind_view, 41);
            let l = identify_in(&fake.proc(), 9444, &fake.profiles).unwrap();
            assert_eq!((l.place, l.uid, l.pids.clone()), (Place::MindView, 1000, vec![41]), "shape {n}");
            assert!(mind_side_in(&fake.proc(), 9444, 1000, &fake.profiles).is_ok());
        }
    }

    /// The 520 failure: the line was one string with a trailing NUL, the reader saw no profile,
    /// and Mind View's own browser was refused in every order. Here the line claims nothing and
    /// the process still is Mind View's, because it owns the profile.
    #[test]
    fn a_browser_with_a_useless_command_line_is_still_told_by_its_files() {
        let fake = Fake::new("blank");
        fake.table(&[("0100007F:24E4", 1000, 5)]);
        fake.process(9, 5, b"chromium\0");
        fake.holds_lock(9, &fake.profiles.mind_view);
        assert_eq!(identify_in(&fake.proc(), 9444, &fake.profiles).unwrap().place, Place::MindView);
    }

    /// A process that rewrites its argv to claim Mind View's profile owns nothing, so it is not a
    /// mind's; and Mind View's real browser is one whatever it rewrites its argv to.
    #[test]
    fn arguments_prove_nothing_either_way() {
        let liar = Fake::new("liar");
        liar.table(&[("0100007F:2406", 1000, 77)]);
        for line in shapes(&liar.profiles.mind_view) {
            liar.process(9, 77, &line);
            let l = identify_in(&liar.proc(), 9222, &liar.profiles).unwrap();
            assert_eq!(l.place, Place::Person, "a claim in the arguments made it a mind's");
            assert!(mind_side_in(&liar.proc(), 9222, 1000, &liar.profiles).unwrap_err().contains("not a mind's own"));
        }
        let owner = Fake::new("owner");
        owner.table(&[("0100007F:24E4", 1000, 5)]);
        owner.process(9, 5, b"chromium --user-data-dir=/home/p/.config/chromium");
        owner.takes(&owner.profiles.mind_view, 9);
        assert_eq!(identify_in(&owner.proc(), 9444, &owner.profiles).unwrap().place, Place::MindView);
    }

    #[test]
    fn a_companion_identity_is_a_minds_too() {
        let fake = Fake::new("id");
        let work = fake.profiles.identities.join("work");
        fs::create_dir_all(&work).unwrap();
        fake.table(&[("0100007F:2410", 1000, 3)]);
        fake.process(3, 3, b"chromium");
        fake.takes(&work, 3);
        assert_eq!(identify_in(&fake.proc(), 0x2410, &fake.profiles).unwrap().place, Place::Identity);
    }

    #[test]
    fn another_account_nothing_and_the_unreadable_are_all_refusals() {
        let fake = Fake::new("x");
        fake.table(&[("0100007F:24E4", 1001, 5)]);
        fake.process(3, 5, b"chromium");
        fake.takes(&fake.profiles.mind_view, 3);
        assert!(mind_side_in(&fake.proc(), 9444, 1000, &fake.profiles).unwrap_err().contains("another account"));
        assert_eq!(identify_in(&fake.proc(), 9999, &fake.profiles), Err(Why::Nobody));
        // Listening, but the holder is not visible to this account.
        fake.table(&[("0100007F:24E4", 1000, 6)]);
        assert!(matches!(identify_in(&fake.proc(), 9444, &fake.profiles), Err(Why::Unclear(_))));
        // Visible, but the files it holds cannot be read.
        fake.table(&[("0100007F:24E4", 1000, 5)]);
        fs::remove_file(fake.profiles.mind_view.join("SingletonLock")).unwrap();
        fs::remove_dir_all(fake.proc().join("3/fd")).unwrap_or(());
        assert!(identify_in(&fake.proc(), 9444, &fake.profiles).is_err());
        // No table at all.
        fs::remove_file(fake.proc().join("net/tcp")).unwrap();
        assert!(matches!(identify_in(&fake.proc(), 9444, &fake.profiles), Err(Why::Unclear(_))));
    }

    #[test]
    fn two_sockets_or_the_person_and_a_mind_on_one_socket_are_not_taken_for_the_persons() {
        let fake = Fake::new("two");
        fake.table(&[("0100007F:24E4", 1000, 1), ("0100007F:24E4", 1000, 2)]);
        fake.process(1, 1, b"chromium");
        fake.process(2, 2, b"chromium");
        assert!(matches!(identify_in(&fake.proc(), 9444, &fake.profiles), Err(Why::Unclear(w)) if w.contains("two")));
        // One socket shared by the person's process and Mind View's: it is Mind View's, so the
        // person's side refuses it and a mind's side accepts nothing of the person's.
        fake.table(&[("0100007F:24E4", 1000, 1)]);
        fake.process(2, 1, b"chromium");
        fake.takes(&fake.profiles.mind_view, 2);
        assert_eq!(identify_in(&fake.proc(), 9444, &fake.profiles).unwrap().place, Place::MindView);
    }

    #[test]
    fn the_exact_address_beats_a_wildcard_one_as_the_kernel_does() {
        let fake = Fake::new("wild");
        fake.table(&[("00000000:24E4", 1000, 1), ("0100007F:24E4", 1000, 2)]);
        fake.process(1, 1, b"chromium");
        fake.process(2, 2, b"chromium");
        fake.takes(&fake.profiles.mind_view, 2);
        assert_eq!(identify_in(&fake.proc(), 9444, &fake.profiles).unwrap().place, Place::MindView);
        // A wildcard alone still answers on loopback, and a process owning nothing is not a mind's.
        fake.table(&[("00000000:24E4", 1000, 1)]);
        assert_eq!(identify_in(&fake.proc(), 9444, &fake.profiles).unwrap().place, Place::Person);
    }
}
