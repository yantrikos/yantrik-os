//! The mind-side browsers among the running processes, so what is stopped, counted or trimmed is
//! a mind's own browser and never the person's: the watchdogs used to `pgrep chromium` and kill
//! renderers, or all of it, on a machine where the person's Browser is the biggest Chromium.
//!
//! A browser is a mind's by the profile it owns (the pid its `SingletonLock` names), not by its
//! arguments.

use std::fs;
use std::path::Path;

use crate::{listener, lock_pid, Place, Profiles, PERSON_PORT};

/// One process, as `/proc` shows it.
#[derive(Clone, Debug)]
pub struct Proc {
    pub pid: u32,
    pub ppid: u32,
    pub uid: u32,
}

/// The parent in a `/proc/<pid>/stat` line. The name is in parentheses and may hold anything.
fn parent_in(stat: &str) -> Option<u32> {
    let after = &stat[stat.rfind(')')? + 1..];
    after.split_whitespace().nth(1)?.parse().ok()
}

/// Every process `proc` shows. The ones that cannot be read are left out.
pub fn snapshot_in(proc: &Path) -> Vec<Proc> {
    let Ok(dir) = fs::read_dir(proc) else { return Vec::new() };
    dir.flatten()
        .filter_map(|e| {
            let pid = e.file_name().to_str()?.parse::<u32>().ok()?;
            let ppid = parent_in(&fs::read_to_string(e.path().join("stat")).ok()?)?;
            #[cfg(unix)]
            let uid = {
                use std::os::unix::fs::MetadataExt;
                fs::metadata(e.path()).ok()?.uid()
            };
            #[cfg(not(unix))]
            let uid = 0;
            Some(Proc { pid, ppid, uid })
        })
        .collect()
}

pub fn snapshot() -> Vec<Proc> {
    snapshot_in(Path::new("/proc"))
}

/// The main processes of the browsers whose profile is a mind's (Mind View's, or one of the
/// companion's identities), run by `own`, with which: the pid each profile's `SingletonLock`
/// names, if that process is running. Never the person's.
pub fn mind_side_mains<'a>(procs: &'a [Proc], own: u32, profiles: &Profiles) -> Vec<(&'a Proc, Place)> {
    let mut dirs = vec![(profiles.mind_view.clone(), Place::MindView)];
    if let Ok(entries) = fs::read_dir(&profiles.identities) {
        dirs.extend(entries.flatten().filter(|e| e.path().is_dir()).map(|e| (e.path(), Place::Identity)));
    }
    dirs.into_iter()
        .filter_map(|(dir, place)| {
            let pid = lock_pid(&dir)?;
            procs.iter().find(|p| p.pid == pid && p.uid == own).map(|p| (p, place))
        })
        .collect()
}

/// Mind View's browsers that hold the person's port: one an earlier build left there, which would
/// keep the person's own Browser from having it. `on_persons_port` is the pids holding that
/// port's socket. Stopping one costs a mind its open tabs and nothing of the person's.
pub fn mind_view_off_port<'a>(procs: &'a [Proc], own: u32, profiles: &Profiles, on_persons_port: &[u32]) -> Vec<&'a Proc> {
    mind_side_mains(procs, own, profiles)
        .into_iter()
        .filter(|(p, place)| *place == Place::MindView && on_persons_port.contains(&p.pid))
        .map(|(p, _)| p)
        .collect()
}

/// `roots` and everything started under them.
pub fn with_descendants(procs: &[Proc], roots: &[&Proc]) -> Vec<u32> {
    let mut family: Vec<u32> = roots.iter().map(|p| p.pid).collect();
    let mut grew = true;
    while grew {
        grew = false;
        for p in procs {
            if !family.contains(&p.pid) && family.contains(&p.ppid) {
                family.push(p.pid);
                grew = true;
            }
        }
    }
    family
}

/// Every process of a mind's browsers on this machine, run by this account.
pub fn mind_side_family() -> Vec<Proc> {
    let all = snapshot();
    let roots: Vec<&Proc> = mind_side_mains(&all, listener::own_uid(), &Profiles::current()).into_iter().map(|(p, _)| p).collect();
    let pids = with_descendants(&all, &roots);
    all.into_iter().filter(|p| pids.contains(&p.pid)).collect()
}

/// Send `signal` (`-15`, `-9`) to processes by pid. Says how many accepted it.
pub fn signal(pids: &[u32], signal: &str) -> usize {
    pids.iter()
        .filter(|pid| {
            std::process::Command::new("kill")
                .args([signal, &pid.to_string()])
                .output()
                .is_ok_and(|o| o.status.success())
        })
        .count()
}

/// Stop Mind View's browsers that hold the person's port, the main process and what it started.
/// Returns how many processes were told to stop.
pub fn stop_mind_view_off_port() -> usize {
    let held = listener::holders_in(Path::new("/proc"), PERSON_PORT).map(|(_, _, pids)| pids).unwrap_or_default();
    if held.is_empty() {
        return 0;
    }
    let all = snapshot();
    let profiles = Profiles::current();
    let roots = mind_view_off_port(&all, listener::own_uid(), &profiles, &held);
    if roots.is_empty() {
        return 0;
    }
    signal(&with_descendants(&all, &roots), "-15")
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::listener::tests::Fake;

    fn p(pid: u32, ppid: u32, uid: u32) -> Proc {
        Proc { pid, ppid, uid }
    }

    #[test]
    fn only_a_minds_browsers_are_the_minds_never_the_persons() {
        let fake = Fake::new("mains");
        let work = fake.profiles.identities.join("work");
        std::fs::create_dir_all(&work).unwrap();
        // 10 and 12 are the person's (they own nothing of a mind's); 20 owns Mind View's profile,
        // 30 a companion identity; 40 would be Mind View's but is another account's.
        fake.takes(&fake.profiles.mind_view, 20);
        fake.takes(&work, 30);
        let all = vec![p(10, 1, 1000), p(11, 10, 1000), p(12, 1, 1000), p(20, 1, 1000), p(21, 20, 1000), p(30, 1, 1000), p(40, 1, 1001)];
        let mains: Vec<u32> = mind_side_mains(&all, 1000, &fake.profiles).iter().map(|(p, _)| p.pid).collect();
        assert_eq!(mains, vec![20, 30]);
        let other = Fake::new("mains2");
        other.takes(&other.profiles.mind_view, 40);
        assert!(mind_side_mains(&all, 1000, &other.profiles).is_empty(), "another account's browser is not ours to stop");
    }

    #[test]
    fn a_mind_view_browser_on_the_persons_port_is_the_only_one_to_move() {
        let fake = Fake::new("off");
        fake.takes(&fake.profiles.mind_view, 30);
        let all = vec![p(10, 1, 1000), p(20, 1, 1000), p(30, 1, 1000), p(31, 30, 1000)];
        // 10 holds the person's port and owns nothing of Mind View's: not touched. 30 does and does.
        assert!(mind_view_off_port(&all, 1000, &fake.profiles, &[10]).is_empty());
        let off = mind_view_off_port(&all, 1000, &fake.profiles, &[10, 30]);
        assert_eq!(off.iter().map(|p| p.pid).collect::<Vec<_>>(), vec![30]);
        assert_eq!(with_descendants(&all, &off), vec![30, 31], "its helpers go with it, nothing else does");
        // Mind View's browser on its own port, not on the person's, is left running.
        assert!(mind_view_off_port(&all, 1000, &fake.profiles, &[]).is_empty());
    }

    #[test]
    fn a_parent_is_read_past_a_name_with_parentheses() {
        assert_eq!(parent_in("123 (chrom) ium) S 77 1 1"), Some(77));
        assert_eq!(parent_in("garbage"), None);
    }
}
