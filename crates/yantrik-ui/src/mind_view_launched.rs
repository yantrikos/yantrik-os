//! The apps the shell launched in Mind View for a Mind are that Mind's (#1027).
//!
//! What marks them is a cgroup, not a parent chain. Every app the shell starts in Mind View runs
//! in its own transient systemd user scope named for the Mind (`yantrik-mindview-<mind>-<n>`),
//! entered before the app's first instruction because `systemd-run --scope` places itself in the
//! scope and then executes the app. Every descendant starts in its parent's cgroup, a process
//! cannot leave its cgroup without privilege, and reparenting does not change it, so a process
//! that outlives its parent or detaches from it is still inside. [`requester_now`] reads
//! `/proc/<caller>/cgroup` and nothing else here: no pid record, no ancestry walk, no depth cap.
//!
//! [`requester_now`]: crate::mind_view::requester_now

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

/// What is said of a Mind that cannot be named.
pub const UNNAMED: &str = "a mind";

const PREFIX: &str = "yantrik-mindview-";
const SUFFIX: &str = ".scope";

/// The command line that starts `program` inside a new scope named for `mind`; unchanged for
/// `None` (the person's desktop). Nothing runs unscoped for a Mind: with no `systemd-run` or no
/// user manager the scope does not start, so neither does the app.
pub fn scoped(program: PathBuf, argv: Vec<String>, mind: Option<&str>) -> (PathBuf, Vec<String>) {
    let Some(mind) = mind else { return (program, argv) };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    // Time as well as a count: a scope left by an earlier run of the shell keeps its name.
    let millis = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
    let unit = format!("{PREFIX}{}-{millis}-{}", label(mind), NEXT.fetch_add(1, Ordering::Relaxed));
    let mut all = vec!["--user".into(), "--scope".into(), "--quiet".into(), "--collect".into(), format!("--unit={unit}"), "--".into()];
    all.push(program.display().to_string());
    all.extend(argv);
    (PathBuf::from("systemd-run"), all)
}

/// A Mind's name as a unit-name part: letters and digits, everything else `_`, never a `-`, so
/// the name ends at the first `-` when read back.
fn label(mind: &str) -> String {
    let l: String = mind.trim().to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    if l.is_empty() { "mind".to_string() } else { l }
}

/// Pure: the Mind named by a `/proc/<pid>/cgroup` text, if the process is inside one of the
/// Mind View scopes of the user manager of `uid` (any user when `None`). The scope must sit under
/// `/user.slice/…/user@<uid>.service/`: the same name elsewhere (`/system.slice`, a login
/// session's scope) is a lookalike and names no one.
pub fn mind_of_cgroup(text: &str, uid: Option<u32>) -> Option<String> {
    text.lines().filter_map(|line| line.splitn(3, ':').nth(2)).find_map(|path| {
        let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
        if parts.first() != Some(&"user.slice") {
            return None;
        }
        let manager = parts.iter().position(|p| {
            p.strip_prefix("user@").and_then(|r| r.strip_suffix(".service")).is_some_and(|n| match uid {
                Some(u) => n == u.to_string(),
                None => !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()),
            })
        })?;
        parts[manager + 1..].iter().find_map(|p| {
            let rest = p.strip_prefix(PREFIX)?.strip_suffix(SUFFIX)?;
            let (name, tail) = rest.split_once('-')?;
            (!name.is_empty() && !tail.is_empty()).then(|| name.to_string())
        })
    })
}

/// The Mind a process belongs to by its cgroup, `None` when it is in none, or unreadable.
pub fn named_by_scope(pid: u32) -> Option<String> {
    mind_of_cgroup(&std::fs::read_to_string(format!("/proc/{pid}/cgroup")).ok()?, own_uid())
}

fn own_uid() -> Option<u32> {
    use std::os::unix::fs::MetadataExt as _;
    std::fs::metadata("/proc/self").ok().map(|m| m.uid())
}

/// The Mind a caller belongs to, if any: by an attached harness (`resolve`), else by its cgroup
/// (`cgroup`, the text of its `/proc/<pid>/cgroup`, `None` when it cannot be read).
///
/// Fails closed: a cgroup that cannot be read is not the person's. The caller is classified as an
/// unnamed Mind, which can only ask and never act as the person; a real person's call that raced
/// its own exit gets the stricter path, which is the safe direction.
pub fn attached_mind_with(
    pid: u32,
    resolve: impl Fn(u32) -> Option<String>,
    cgroup: impl Fn(u32) -> Option<String>,
    uid: Option<u32>,
) -> Option<String> {
    resolve(pid).or_else(|| match cgroup(pid) {
        Some(text) => mind_of_cgroup(&text, uid),
        None => Some(UNNAMED.to_string()),
    })
}

/// [`attached_mind_with`] on this machine: what `requester_now` asks.
pub fn attached_mind(pid: u32) -> Option<String> {
    attached_mind_with(
        pid,
        |p| crate::caller_identity::resolve(p as i32).attached_mind,
        |p| std::fs::read_to_string(format!("/proc/{p}/cgroup")).ok(),
        own_uid(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mind_view::{classify, CallerFacts, Requester};

    const UID: Option<u32> = Some(1000);
    const SCOPE: &str = "0::/user.slice/user-1000.slice/user@1000.service/app.slice/yantrik-mindview-pi-1700000000000-3.scope\n";
    const SESSION: &str = "0::/user.slice/user-1000.slice/session-2.scope\n";
    const FAKE_SYSTEM: &str = "0::/system.slice/yantrik-mindview-pi-1-0.scope\n";
    const FAKE_SESSION: &str = "0::/user.slice/user-1000.slice/session-2.scope/yantrik-mindview-pi-1-0.scope\n";
    const OTHER_USER: &str = "0::/user.slice/user-1001.slice/user@1001.service/app.slice/yantrik-mindview-pi-1-0.scope\n";

    fn requester(pid: u32, cgroup: Option<&'static str>) -> Requester {
        let facts = CallerFacts {
            pid: Some(pid),
            agent: None,
            attached_mind: attached_mind_with(pid, |_| None, |_| cgroup.map(str::to_string), UID),
            mind_account: false,
        };
        classify(&facts, 1)
    }

    #[test]
    fn a_caller_in_a_mind_view_scope_is_that_minds_named() {
        assert_eq!(requester(200, Some(SCOPE)), Requester::Mind("pi".into()));
        let nested = "0::/user.slice/user-1000.slice/user@1000.service/app.slice/yantrik-mindview-pi-1-3.scope/sub\n";
        assert_eq!(requester(201, Some(nested)), Requester::Mind("pi".into()), "a cgroup inside the scope");
        let v1 = "1:name=systemd:/user.slice/user-1000.slice/user@1000.service/app.slice/yantrik-mindview-pi-1-3.scope\n0::/x\n";
        assert_eq!(requester(202, Some(v1)), Requester::Mind("pi".into()), "a v1 line");
    }

    #[test]
    fn the_persons_session_is_the_person() {
        assert_eq!(requester(300, Some(SESSION)), Requester::Person);
        assert_eq!(requester(301, Some("0::/\n")), Requester::Person);
    }

    #[test]
    fn a_cgroup_that_cannot_be_read_is_not_the_person() {
        assert_eq!(requester(400, None), Requester::Mind(UNNAMED.into()));
    }

    #[test]
    fn a_lookalike_outside_the_user_manager_names_no_one() {
        for text in [FAKE_SYSTEM, FAKE_SESSION, OTHER_USER] {
            assert_eq!(requester(500, Some(text)), Requester::Person, "{text}");
        }
        assert_eq!(mind_of_cgroup(OTHER_USER, None), Some("pi".into()), "no uid known: any user's manager");
    }

    /// The wiring: the harness answer comes first, and a scope is consulted when it says nothing.
    #[test]
    fn an_attached_harness_wins_and_the_scope_is_the_fallback() {
        assert_eq!(attached_mind_with(1, |_| Some("hermes".into()), |_| Some(SCOPE.into()), UID), Some("hermes".into()));
        assert_eq!(attached_mind_with(1, |_| None, |_| Some(SCOPE.into()), UID), Some("pi".into()));
    }

    /// `requester_now` must ask this module; deleting the cgroup branch (the fix for #1027) must
    /// fail a test, not only a review.
    #[test]
    fn requester_now_asks_the_cgroup() {
        let src = include_str!("mind_view.rs");
        let f = &src[src.find("pub fn requester_now(").unwrap()..];
        let f = &f[..f.find("\n}\n").unwrap()];
        assert!(f.contains("mind_view_launched::attached_mind("), "{f}");
        let i = include_str!("mind_identity.rs");
        assert!(i.contains("mind_view_launched::named_by_scope("), "the identity names a Mind View app's Mind too");
    }

    #[test]
    fn a_launch_is_a_named_scope_and_the_name_reads_back() {
        let (program, argv) = scoped("/usr/bin/foot".into(), vec!["-e".into(), "x".into()], Some("Hermes Agent"));
        assert_eq!(program, PathBuf::from("systemd-run"));
        assert_eq!(&argv[..4], ["--user", "--scope", "--quiet", "--collect"]);
        let unit = argv[4].strip_prefix("--unit=").unwrap();
        assert!(unit.starts_with("yantrik-mindview-hermes_agent-"), "{unit}");
        assert_eq!(argv[5..], ["--", "/usr/bin/foot", "-e", "x"]);
        let cg = format!("0::/user.slice/user-1000.slice/user@1000.service/app.slice/{unit}.scope\n");
        assert_eq!(mind_of_cgroup(&cg, UID), Some("hermes_agent".into()));
        let (p, a) = scoped("/usr/bin/foot".into(), vec!["x".into()], None);
        assert_eq!((p, a), (PathBuf::from("/usr/bin/foot"), vec!["x".to_string()]), "the person's launch is untouched");
    }
}
