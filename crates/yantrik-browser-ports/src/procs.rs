//! The mind-side browsers among the running processes, so what is stopped, counted or trimmed is
//! a mind's own browser and never the person's: the watchdogs used to `pgrep chromium` and kill
//! renderers, or all of it, on a machine where the person's Browser is the biggest Chromium.

use std::fs;
use std::path::Path;

use crate::{cmdline, listener, Place};

/// One process, as `/proc` shows it.
#[derive(Clone, Debug)]
pub struct Proc {
    pub pid: u32,
    pub ppid: u32,
    pub uid: u32,
    pub cmdline: Vec<u8>,
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
            let cmdline = fs::read(e.path().join("cmdline")).ok()?;
            let ppid = parent_in(&fs::read_to_string(e.path().join("stat")).ok()?)?;
            #[cfg(unix)]
            let uid = {
                use std::os::unix::fs::MetadataExt;
                fs::metadata(e.path()).ok()?.uid()
            };
            #[cfg(not(unix))]
            let uid = 0;
            Some(Proc { pid, ppid, uid, cmdline })
        })
        .collect()
}

pub fn snapshot() -> Vec<Proc> {
    snapshot_in(Path::new("/proc"))
}

/// The main processes of browsers whose profile is a mind's (Mind View's, or one of the
/// companion's identities), run by `own`. Never one on the default profile or the person's.
pub fn mind_side_mains(procs: &[Proc], own: u32) -> Vec<&Proc> {
    procs
        .iter()
        .filter(|p| p.uid == own && cmdline::is_main(&p.cmdline) && crate::place_of(&p.cmdline).is_some_and(Place::is_mind_side))
        .collect()
}

/// Mind View's browsers that are not on Mind View's port: one an earlier build left on the
/// person's port, which would keep the person's own Browser from having it. Stopping one costs
/// a mind its open tabs and nothing of the person's.
pub fn mind_view_off_port(procs: &[Proc], own: u32) -> Vec<&Proc> {
    mind_side_mains(procs, own)
        .into_iter()
        .filter(|p| crate::place_of(&p.cmdline) == Some(Place::MindView) && cmdline::debugging_port(&p.cmdline) != Some(crate::MIND_VIEW_PORT))
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
    let pids = with_descendants(&all, &mind_side_mains(&all, listener::own_uid()));
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

/// Stop Mind View's browsers that are not on its port, the main process and what it started.
/// Returns how many processes were told to stop.
pub fn stop_mind_view_off_port() -> usize {
    let all = snapshot();
    let own = listener::own_uid();
    let roots = mind_view_off_port(&all, own);
    if roots.is_empty() {
        return 0;
    }
    let family = with_descendants(&all, &roots);
    signal(&family, "-15")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pid: u32, ppid: u32, uid: u32, line: &str) -> Proc {
        Proc { pid, ppid, uid, cmdline: line.as_bytes().to_vec() }
    }

    fn fleet() -> Vec<Proc> {
        let mv = crate::mind_view_profile();
        let mv = mv.display();
        vec![
            // The person's Browser: default profile, and the same with its profile spelled out.
            p(10, 1, 1000, "/usr/lib/chromium/chromium --remote-debugging-port=9222"),
            p(11, 10, 1000, "/usr/lib/chromium/chromium --type=renderer --user-data-dir=/home/p/.config/chromium"),
            p(12, 1, 1000, "/usr/lib/chromium/chromium --remote-debugging-port=9222 --user-data-dir=/home/p/.config/chromium"),
            // Mind View's browser on its port, with a renderer; and one an earlier build left on 9222.
            p(20, 1, 1000, &format!("/usr/lib/chromium/chromium --remote-debugging-port=9223 --user-data-dir={mv}")),
            p(21, 20, 1000, &format!("/usr/lib/chromium/chromium --type=renderer --user-data-dir={mv}")),
            p(30, 1, 1000, &format!("/usr/lib/chromium/chromium --remote-debugging-port=9222 --user-data-dir={mv}")),
            p(31, 30, 1000, "chromium --type=gpu-process"),
            // Another account's, and a process that is no browser.
            p(40, 1, 1001, &format!("chromium --remote-debugging-port=9222 --user-data-dir={mv}")),
            p(50, 1, 1000, "vim notes.txt"),
        ]
    }

    #[test]
    fn only_a_minds_browsers_are_the_minds_never_the_persons() {
        let all = fleet();
        let mains: Vec<u32> = mind_side_mains(&all, 1000).iter().map(|p| p.pid).collect();
        assert_eq!(mains, vec![20, 30]);
        assert!(!mains.contains(&10) && !mains.contains(&12), "the person's Browser is never a mind's");
    }

    #[test]
    fn a_mind_view_browser_left_on_the_persons_port_is_the_only_one_to_move() {
        let all = fleet();
        let off: Vec<u32> = mind_view_off_port(&all, 1000).iter().map(|p| p.pid).collect();
        assert_eq!(off, vec![30]);
        let family = with_descendants(&all, &mind_view_off_port(&all, 1000));
        assert_eq!(family, vec![30, 31], "its helpers go with it, nothing else does");
    }

    #[test]
    fn a_parent_is_read_past_a_name_with_parentheses() {
        assert_eq!(parent_in("123 (chrom) ium) S 77 1 1"), Some(77));
        assert_eq!(parent_in("garbage"), None);
    }
}
