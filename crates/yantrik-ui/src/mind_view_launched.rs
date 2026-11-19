//! The apps the shell launched in Mind View for a Mind are that Mind's (#1027).
//!
//! What marks them is a cgroup, not a parent chain. Every app the shell starts in Mind View runs
//! in its own transient systemd user scope (`yantrik-mindview-<key>-<ms>-<n>`), entered before
//! the app's first instruction because `systemd-run --scope` places itself in the scope and then
//! executes the app. `<key>` is the caller's identity key, hex-encoded, or `0` when it has none.
//!
//! The cgroup only says who is a Mind where the Mind cannot move it. The person's user manager is
//! delegated to the person's uid, and so is everything under it: a process of that uid can ask
//! the manager for any unit, or move itself to any cgroup in that subtree. So the scope's name
//! proves nothing about a caller outside it, and nothing under the user manager is ever the
//! person. The person is positively the caller whose systemd cgroup equals the shell's own, the
//! login session's scope (`user-<uid>.slice` is root's, so that uid cannot move itself into it),
//! when that is not itself under a user manager. Every other caller is a Mind: one named by its
//! Mind View scope, else an unnamed one that can ask and never act. No unit is trusted by name.
//!
//! A scope's name is a claim, so it never gives a **named key**: [`named_by_scope`] is `None`, and
//! the per-Mind rules (the files cursor, capture's "only yours") treat a Mind View app as an
//! unnamed Mind. The Mind's display name is shown only for a unit the shell itself created and
//! still holds (`scoped` records it, `forget` drops it when the app's scope ends); any other name
//! reads "a mind". A caller whose uid is 0 is the person: root already holds the machine and no
//! Mind runs as root.
//! [`requester_now`] reads `/proc/<caller>/cgroup` and nothing else here.
//!
//! [`requester_now`]: crate::mind_view::requester_now

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::mind_identity::Identity;

/// What is said of a Mind that cannot be named.
pub const UNNAMED: &str = "a mind";

const PREFIX: &str = "yantrik-mindview-";
const SUFFIX: &str = ".scope";
/// The key part of a scope with no named key. Hex is always an even number of digits, so no key
/// can spell it.
const RESERVED: &str = "0";
/// A longer key goes unnamed rather than push the unit name toward systemd's length limit.
const MAX_KEY: usize = 64;

/// The scope units this shell created and still holds.
static HELD: Mutex<Option<HashSet<String>>> = Mutex::new(None);

fn held(unit: &str) -> bool {
    HELD.lock().unwrap_or_else(|p| p.into_inner()).as_ref().is_some_and(|h| h.contains(unit))
}

/// Drop a unit when its app's scope has ended.
pub fn forget(unit: &str) {
    if let Some(h) = HELD.lock().unwrap_or_else(|p| p.into_inner()).as_mut() {
        h.remove(unit);
    }
}

/// The unit a command built by [`scoped`] runs in (`""` for the person's launch).
pub fn unit_of(command: &std::process::Command) -> String {
    command.get_args().find_map(|a| a.to_str()?.strip_prefix("--unit=").map(str::to_string)).unwrap_or_default()
}

/// The command line that starts `program` inside a new scope; unchanged for `None` (the person's
/// desktop). `Some(key)` is a Mind's: the identity key of the caller (`Identity::named_key`), or
/// `None` when it has none. Nothing runs unscoped for a Mind: with no `systemd-run` or no user
/// manager the scope does not start, so neither does the app ([`available`] says so first).
pub fn scoped(program: PathBuf, argv: Vec<String>, mind: Option<Option<&str>>) -> (PathBuf, Vec<String>) {
    let Some(key) = mind else { return (program, argv) };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    // Time as well as a count: a scope left by an earlier run of the shell keeps its name.
    let millis = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
    let part = key.filter(|k| !k.is_empty() && k.len() <= MAX_KEY).map_or_else(|| RESERVED.to_string(), hex);
    let unit = format!("{PREFIX}{part}-{millis}-{}", NEXT.fetch_add(1, Ordering::Relaxed));
    HELD.lock().unwrap_or_else(|p| p.into_inner()).get_or_insert_with(HashSet::new).insert(unit.clone());
    let mut all = vec!["--user".into(), "--scope".into(), "--quiet".into(), "--collect".into(), format!("--unit={unit}"), "--".into()];
    all.push(program.display().to_string());
    all.extend(argv);
    (PathBuf::from("systemd-run"), all)
}

/// Whether a Mind View app can be started at all: it runs in a scope of the user's systemd
/// manager, so it needs `systemd-run` and that manager.
pub fn available() -> Result<(), String> {
    let manager = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).or_else(|| own_uid().map(|u| PathBuf::from(format!("/run/user/{u}"))));
    available_with(crate::wire::dock::find_program("systemd-run").is_some(), manager.is_some_and(|d| d.join("systemd/private").exists()))
}

fn available_with(has_systemd_run: bool, has_manager: bool) -> Result<(), String> {
    if has_systemd_run && has_manager {
        return Ok(());
    }
    Err("Mind View needs the user's systemd manager (systemd-run --user); not started".to_string())
}

fn hex(key: &str) -> String {
    key.bytes().map(|b| format!("{b:02x}")).collect()
}

fn unhex(part: &str) -> Option<String> {
    if part.len() % 2 != 0 || !part.is_ascii() {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..part.len()).step_by(2).map(|i| u8::from_str_radix(&part[i..i + 2], 16).ok()).collect();
    String::from_utf8(bytes?).ok()
}

/// What a Mind View scope says of its caller.
#[derive(Debug, PartialEq, Eq)]
pub enum Scope {
    /// A Mind with this identity key, in this scope unit.
    Key(String, String),
    /// A Mind that had no named key (or whose name does not read back).
    Unnamed,
}

/// The systemd path of a `/proc/<pid>/cgroup` text: the `name=systemd` line on v1, else `0::`.
fn systemd_path(text: &str) -> Option<String> {
    let mut unified = None;
    for line in text.lines() {
        let mut f = line.splitn(3, ':');
        let (Some(id), Some(controllers), Some(path)) = (f.next(), f.next(), f.next()) else { continue };
        if controllers == "name=systemd" {
            return Some(path.to_string());
        }
        if id == "0" && controllers.is_empty() {
            unified = Some(path.to_string());
        }
    }
    unified
}

fn is_manager(part: &str, uid: Option<u32>) -> bool {
    part.strip_prefix("user@").and_then(|r| r.strip_suffix(".service")).is_some_and(|n| match uid {
        Some(u) => n == u.to_string(),
        None => !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()),
    })
}

fn under_a_user_manager(path: &str) -> bool {
    path.split('/').any(|p| p.starts_with("user@") && p.ends_with(".service"))
}

/// Pure: the Mind View scope a `/proc/<pid>/cgroup` text is inside, if any: one of the scopes of
/// the user manager of `uid` (any user when `None`). It must sit under
/// `/user.slice/…/user@<uid>.service/`: the same name elsewhere (`/system.slice`, a login
/// session's scope) is a lookalike and names no one.
pub fn scope_of_cgroup(text: &str, uid: Option<u32>) -> Option<Scope> {
    let path = systemd_path(text)?;
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    if parts.first() != Some(&"user.slice") {
        return None;
    }
    let manager = parts.iter().position(|p| is_manager(p, uid))?;
    parts[manager + 1..].iter().find_map(|p| {
        let rest = p.strip_prefix(PREFIX)?.strip_suffix(SUFFIX)?;
        let (part, tail) = rest.split_once('-')?;
        if part.is_empty() || tail.is_empty() {
            return None;
        }
        Some(if part == RESERVED { Scope::Unnamed } else { unhex(part).map_or(Scope::Unnamed, |key| Scope::Key(key, format!("{PREFIX}{rest}"))) })
    })
}

/// Never a named key: a scope's name is a claim, not a proof (see the module doc). A Mind View
/// app is an unnamed Mind to every rule that tells Minds apart.
pub fn named_by_scope(_pid: u32) -> Option<Identity> {
    None
}

fn own_uid() -> Option<u32> {
    use std::os::unix::fs::MetadataExt as _;
    std::fs::metadata("/proc/self").ok().map(|m| m.uid())
}

/// How the person reads a scope's key: a Mind by its name, anything else as "an agent" or unnamed;
/// only for a unit the shell holds.
fn words(scope: Scope, held: &dyn Fn(&str) -> bool) -> String {
    match scope {
        Scope::Key(key, unit) if held(&unit) => match Identity::from_key(&key) {
            Some(Identity::Mind(name)) => name,
            Some(Identity::Agent(_)) => "an agent".to_string(),
            _ => UNNAMED.to_string(),
        },
        _ => UNNAMED.to_string(),
    }
}

/// Pure: `None` (the person) only for a caller in the shell's own systemd cgroup, when that is
/// not under a user manager; the Mind its scope names; every other caller an unnamed Mind.
pub fn mind_of_cgroup(text: &str, own: Option<&str>, uid: Option<u32>, held: &dyn Fn(&str) -> bool) -> Option<String> {
    if let Some(scope) = scope_of_cgroup(text, uid) {
        return Some(words(scope, held));
    }
    let path = systemd_path(text);
    let shell = own.and_then(systemd_path);
    match (path, shell) {
        (Some(p), Some(s)) if p == s && !under_a_user_manager(&p) => None,
        _ => Some(UNNAMED.to_string()),
    }
}

/// The Mind a caller belongs to, if any: by an attached harness (`resolve`), else by its cgroup
/// (`cgroup`, the text of its `/proc/<pid>/cgroup`, `None` when it cannot be read) against the
/// shell's own (`own`). `caller_uid` is the caller's uid: 0 is the person.
///
/// Fails closed: a cgroup that cannot be read is not the person's. The caller is classified as an
/// unnamed Mind, which can only ask and never act as the person; a real person's call that raced
/// its own exit gets the stricter path, which is the safe direction.
pub fn attached_mind_with(
    pid: u32,
    resolve: impl Fn(u32) -> Option<String>,
    cgroup: impl Fn(u32) -> Option<String>,
    own: Option<&str>,
    uid: Option<u32>,
    caller_uid: Option<u32>,
) -> Option<String> {
    resolve(pid).or_else(|| {
        if caller_uid == Some(0) {
            return None; // root already holds the machine; no Mind runs as root
        }
        match cgroup(pid) {
            Some(text) => mind_of_cgroup(&text, own, uid, &held),
            None => Some(UNNAMED.to_string()),
        }
    })
}

/// What this machine says of the shell itself: its `/proc/self/cgroup` text and its uid.
fn this_machine() -> (Option<String>, Option<u32>) {
    (std::fs::read_to_string("/proc/self/cgroup").ok(), own_uid())
}

/// The uid of a process, from `/proc`; `None` when it is gone.
pub fn uid_of(pid: u32) -> Option<u32> {
    use std::os::unix::fs::MetadataExt as _;
    std::fs::metadata(format!("/proc/{pid}")).ok().map(|m| m.uid())
}

/// [`attached_mind_with`] on this machine, for a caller whose harness answer (`resolved`, from the
/// ancestry walk) is already in hand, so `/proc` is walked once: what `requester_now` and
/// `caller_class::class_of` ask, so the two cannot disagree.
pub fn attached_mind_given(pid: u32, resolved: Option<String>, caller_uid: Option<u32>) -> Option<String> {
    static LOGGED: std::sync::Once = std::sync::Once::new();
    let (own, uid) = this_machine();
    if own.as_deref().and_then(systemd_path).is_some_and(|p| under_a_user_manager(&p)) {
        LOGGED.call_once(|| tracing::warn!("the shell runs under a user manager, so no caller can be told from a Mind by its cgroup: every caller is read as an unnamed Mind"));
    }
    attached_mind_with(pid, |_| resolved.clone(), |p| std::fs::read_to_string(format!("/proc/{p}/cgroup")).ok(), own.as_deref(), uid, caller_uid)
}

/// [`attached_mind_given`] with the walk done here: what `requester_now` asks.
pub fn attached_mind(pid: u32, caller_uid: Option<u32>) -> Option<String> {
    attached_mind_given(pid, crate::caller_identity::resolve(pid as i32).attached_mind, caller_uid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mind_view::{classify, CallerFacts, Requester};

    const UID: Option<u32> = Some(1000);
    const UM: &str = "/user.slice/user-1000.slice/user@1000.service";
    /// The shell's own cgroup: the login session's scope.
    const OWN: &str = "0::/user.slice/user-1000.slice/session-2.scope\n";
    const SESSION: &str = OWN;
    const OTHER_SESSION: &str = "0::/user.slice/user-1000.slice/session-3.scope\n";
    const FAKE_SYSTEM: &str = "0::/system.slice/yantrik-mindview-70-1-0.scope\n";
    const FAKE_SESSION: &str = "0::/user.slice/user-1000.slice/session-2.scope/yantrik-mindview-70-1-0.scope\n";
    const FAKE_SLICE: &str = "0::/system.slice/x.service/user@1000.service/app.slice/yantrik-mindview-70-1-0.scope\n";
    const OTHER_USER: &str = "0::/user.slice/user-1001.slice/user@1001.service/app.slice/yantrik-mindview-70-1-0.scope\n";

    fn cg(unit: &str) -> String {
        format!("0::{UM}/app.slice/{unit}.scope\n")
    }

    /// The cgroup text of an app launched for `key` (`None`: a Mind with no named key).
    fn launched(key: Option<&str>) -> String {
        let (_, argv) = scoped("/usr/bin/foot".into(), vec![], Some(key));
        let unit = argv[4].strip_prefix("--unit=").unwrap().to_string();
        cg(&unit)
    }

    /// The key a scope's name spells (what the name claims, not who is in it).
    fn key_of(text: &str) -> Option<Identity> {
        match scope_of_cgroup(text, UID)? {
            Scope::Key(key, _) => Identity::from_key(&key),
            Scope::Unnamed => None,
        }
    }

    fn requester(pid: u32, cgroup: Option<&'static str>, own: Option<&str>) -> Requester {
        requester_as(pid, cgroup, own, Some(1000))
    }

    fn requester_as(pid: u32, cgroup: Option<&'static str>, own: Option<&str>, caller_uid: Option<u32>) -> Requester {
        let facts = CallerFacts {
            pid: Some(pid),
            agent: None,
            attached_mind: attached_mind_with(pid, |_| None, |_| cgroup.map(str::to_string), own, UID, caller_uid),
            mind_account: false,
        };
        classify(&facts, 1)
    }

    #[test]
    fn a_caller_in_a_mind_view_scope_is_that_minds_named() {
        let pi = launched(Some("mind:pi"));
        assert_eq!(attached_mind_with(200, |_| None, |_| Some(pi.clone()), Some(OWN), UID, Some(1000)), Some("pi".into()));
        assert_eq!(attached_mind_with(201, |_| None, |_| Some(format!("{pi}").replace(".scope\n", ".scope/sub\n")), Some(OWN), UID, Some(1000)), Some("pi".into()), "a cgroup inside the scope");
        let v1 = format!("1:name=systemd:{}\n0::/x\n", pi.trim().trim_start_matches("0::"));
        assert_eq!(attached_mind_with(202, |_| None, |_| Some(v1.clone()), Some(OWN), UID, Some(1000)), Some("pi".into()), "a v1 line");
    }

    #[test]
    fn the_persons_session_is_the_person() {
        assert_eq!(requester(300, Some(SESSION), Some(OWN)), Requester::Person);
        assert_eq!(requester(301, Some("0::/\n"), Some("0::/\n")), Requester::Person, "a shell at / and a caller at /");
        assert_eq!(requester(302, Some("0::/\n"), Some(OWN)), Requester::Mind(UNNAMED.into()), "not the shell's own cgroup");
        assert_eq!(requester(303, Some(SESSION), None), Requester::Mind(UNNAMED.into()), "the shell's own is unknown");
    }

    #[test]
    fn a_cgroup_that_cannot_be_read_is_not_the_person() {
        assert_eq!(requester(400, None, Some(OWN)), Requester::Mind(UNNAMED.into()));
    }

    #[test]
    fn a_lookalike_outside_the_user_manager_names_no_one_and_is_not_the_person() {
        for text in [FAKE_SYSTEM, FAKE_SESSION, FAKE_SLICE, OTHER_USER] {
            assert_eq!(requester(500, Some(text), Some(OWN)), Requester::Mind(UNNAMED.into()), "{text}");
            assert_eq!(scope_of_cgroup(text, UID), None, "{text}");
        }
        assert_eq!(scope_of_cgroup(OTHER_USER, None), Some(Scope::Key("p".into(), "yantrik-mindview-70-1-0".into())), "no uid known: any user's manager");
    }

    #[test]
    fn any_unit_under_the_user_manager_is_a_mind_whatever_its_name() {
        for unit in ["app.slice/yantrik-capture-rec-1.service", "app.slice/anything.scope", "session.slice/yantrik-shell.service", "app.slice/app-flatpak-x.scope"] {
            let text = format!("0::{UM}/{unit}\n");
            assert_eq!(requester(600, Some(Box::leak(text.into_boxed_str())), Some(OWN)), Requester::Mind("a mind".into()), "{unit}");
        }
    }

    #[test]
    fn another_session_is_not_the_person() {
        assert_eq!(requester(700, Some(OTHER_SESSION), Some(OWN)), Requester::Mind(UNNAMED.into()));
    }

    #[test]
    fn when_the_shell_is_under_a_user_manager_nothing_is_the_person() {
        let own = format!("0::{UM}/session.slice/yantrik-shell.service\n");
        assert_eq!(requester(800, Some(Box::leak(own.clone().into_boxed_str())), Some(&own)), Requester::Mind(UNNAMED.into()), "even the shell's own cgroup");
    }

    /// The wiring: the harness answer comes first, and a scope is consulted when it says nothing.
    #[test]
    fn an_attached_harness_wins_and_the_scope_is_the_fallback() {
        let pi = launched(Some("mind:pi"));
        assert_eq!(attached_mind_with(1, |_| Some("hermes".into()), |_| Some(pi.clone()), Some(OWN), UID, Some(1000)), Some("hermes".into()));
        assert_eq!(attached_mind_with(1, |_| None, |_| Some(pi.clone()), Some(OWN), UID, Some(1000)), Some("pi".into()));
    }

    /// The production wrapper reads this process's own cgroup and uid; swapping either for a stub
    /// must fail here. The test process is the shell's stand-in: the person, unless it runs under
    /// a user manager, where nothing is.
    #[test]
    fn attached_mind_reads_the_real_cgroup() {
        let own = std::fs::read_to_string("/proc/self/cgroup").expect("this process has a cgroup");
        let expected = mind_of_cgroup(&own, Some(&own), own_uid(), &held);
        let under = systemd_path(&own).is_some_and(|p| under_a_user_manager(&p));
        assert_eq!(expected, if under { Some(UNNAMED.to_string()) } else { None }, "this process's own cgroup is the person's unless under a user manager");
        assert_eq!(attached_mind_given(std::process::id(), None, own_uid()), expected);
        assert_eq!(this_machine(), (Some(own), own_uid()), "the wrapper reads this process's own cgroup and uid");
        assert!(own_uid().is_some());
    }

    /// `requester_now` must ask this module; deleting the cgroup branch (the fix for #1027) must
    /// fail a test, not only a review.
    #[test]
    fn requester_now_asks_the_cgroup() {
        let src = include_str!("mind_view.rs");
        let f = &src[src.find("pub fn requester_now(").unwrap()..];
        let f = &f[..f.find("\n}\n").unwrap()];
        assert!(f.contains("mind_view_launched::attached_mind("), "{f}");
        let c = include_str!("caller_class.rs");
        assert!(c.contains("mind_view_launched::attached_mind_given("), "class_of asks the same rule, with the walk it already did");
        let i = include_str!("mind_identity.rs");
        assert!(i.contains("mind_view_launched::named_by_scope("), "the identity asks the scope, which names no key");
    }

    #[test]
    fn a_launch_is_a_named_scope_and_the_key_reads_back() {
        let (program, argv) = scoped("/usr/bin/foot".into(), vec!["-e".into(), "x".into()], Some(Some("mind:Hermes Agent")));
        assert_eq!(program, PathBuf::from("systemd-run"));
        assert_eq!(&argv[..4], ["--user", "--scope", "--quiet", "--collect"]);
        let unit = argv[4].strip_prefix("--unit=").unwrap();
        assert!(unit.starts_with(&format!("yantrik-mindview-{}-", hex("mind:Hermes Agent"))), "{unit}");
        assert_eq!(argv[5..], ["--", "/usr/bin/foot", "-e", "x"]);
        assert_eq!(key_of(&cg(unit)), Some(Identity::Mind("Hermes Agent".into())));
        let (p, a) = scoped("/usr/bin/foot".into(), vec!["x".into()], None);
        assert_eq!((p, a), (PathBuf::from("/usr/bin/foot"), vec!["x".to_string()]), "the person's launch is untouched");
    }

    #[test]
    fn two_minds_whose_names_mangled_alike_now_differ() {
        let keys = ["mind:Hermes Agent", "mind:hermes-agent", "mind:HERMES_AGENT", "mind:hermes_agent"];
        let read: Vec<_> = keys.iter().map(|k| key_of(&launched(Some(k))).unwrap().key()).collect();
        assert_eq!(read, keys, "each reads back as itself");
        for k in ["mind:ünï-cödé ☃", "agent:abc-123"] {
            assert_eq!(key_of(&launched(Some(k))).unwrap().key(), k, "lossless");
        }
    }

    #[test]
    fn agents_and_the_companion_share_no_named_key() {
        let a = key_of(&launched(Identity::Agent("a1".into()).named_key().as_deref()));
        let b = key_of(&launched(Identity::Agent("a2".into()).named_key().as_deref()));
        assert_eq!((a.clone(), b.clone()), (Some(Identity::Agent("a1".into())), Some(Identity::Agent("a2".into()))));
        assert_ne!(a, b, "two agents' apps differ");
        let companion = launched(Identity::Companion.named_key().as_deref());
        assert_eq!(key_of(&companion), None, "the companion's apps have no named key");
        assert_eq!(scope_of_cgroup(&companion, UID), Some(Scope::Unnamed));
        assert_eq!(mind_of_cgroup(&companion, Some(OWN), UID, &held), Some(UNNAMED.into()));
    }

    /// Round 2's "a Mind's own key equals its apps' key" is withdrawn: a scope gives no named key,
    /// so per-Mind rules read a Mind View app as an unnamed Mind.
    #[test]
    fn a_mind_view_app_has_no_named_key() {
        let apps = launched(Some("mind:pi"));
        assert_eq!(named_by_scope(std::process::id()), None);
        assert_eq!(key_of(&apps), Some(Identity::Mind("pi".into())), "the name is only a claim");
        assert!(include_str!("mind_identity.rs").contains("named_by_scope("), "the identity asks, and is told no key");
    }

    /// The display name is shown only for a unit the shell created and still holds.
    #[test]
    fn a_name_is_shown_only_for_a_unit_the_shell_holds() {
        let ours = launched(Some("mind:pi"));
        assert_eq!(mind_of_cgroup(&ours, Some(OWN), UID, &held), Some("pi".into()));
        let forged = cg(&format!("yantrik-mindview-{}-1-0", hex("mind:pi"))); // the shell never made it
        assert_eq!(mind_of_cgroup(&forged, Some(OWN), UID, &held), Some(UNNAMED.into()));
        let forged_agent = cg(&format!("yantrik-mindview-{}-1-0", hex("agent:a1")));
        assert_eq!(mind_of_cgroup(&forged_agent, Some(OWN), UID, &held), Some(UNNAMED.into()));
        let unit = ours.trim_end().rsplit('/').next().unwrap().trim_end_matches(".scope").to_string();
        forget(&unit);
        assert_eq!(mind_of_cgroup(&ours, Some(OWN), UID, &held), Some(UNNAMED.into()), "dropped when the scope ends");
    }

    #[test]
    fn a_unit_is_read_off_the_command() {
        let (program, argv) = scoped("/usr/bin/foot".into(), vec![], Some(Some("mind:pi")));
        let mut c = std::process::Command::new(program);
        c.args(argv);
        assert!(unit_of(&c).starts_with("yantrik-mindview-"));
        assert_eq!(unit_of(&std::process::Command::new("foot")), "");
    }

    /// Root is the person, wherever its cgroup; a person-uid caller outside the session is not.
    #[test]
    fn root_is_the_person_and_the_persons_uid_outside_the_session_is_not() {
        assert_eq!(requester_as(900, Some(OTHER_SESSION), Some(OWN), Some(0)), Requester::Person, "uid 0 over ssh");
        assert_eq!(requester_as(901, None, Some(OWN), Some(0)), Requester::Person, "uid 0, cgroup unreadable");
        assert_eq!(requester_as(902, Some(OTHER_SESSION), Some(OWN), Some(1000)), Requester::Mind(UNNAMED.into()), "the person's uid in another session");
        assert_eq!(requester_as(903, Some(OTHER_SESSION), Some(OWN), None), Requester::Mind(UNNAMED.into()));
        assert_eq!(attached_mind_with(904, |_| Some("hermes".into()), |_| None, Some(OWN), UID, Some(0)), Some("hermes".into()), "an attached harness is still a Mind");
    }

    /// The nested labwc starts in a reserved scope and has nothing that runs a program.
    #[test]
    fn mind_views_own_labwc_runs_in_an_unnamed_scope_and_executes_nothing() {
        let (program, argv) = scoped("labwc".into(), vec![], Some(None));
        assert_eq!(program, PathBuf::from("systemd-run"));
        assert!(argv[4].starts_with("--unit=yantrik-mindview-0-"), "{argv:?}");
        assert!(include_str!("mind_view.rs").contains("mind_view_launched::scoped(labwc"), "start() puts labwc in a scope");
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/labwc-mind");
        let mut seen = 0;
        for entry in std::fs::read_dir(&dir).expect("config/labwc-mind") {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "xml") {
                seen += 1;
                let text = std::fs::read_to_string(&path).unwrap();
                assert!(!text.contains("Execute") && !text.contains("<menu"), "{} would let labwc start a program outside a Mind's scope", path.display());
            }
        }
        assert!(seen > 0, "rc.xml is read");
    }

    #[test]
    fn an_over_long_or_empty_key_is_the_reserved_part() {
        for key in [Some(""), Some(&*"x".repeat(MAX_KEY + 1)), None] {
            let text = launched(key);
            assert!(text.contains("yantrik-mindview-0-"), "{text}");
            assert_eq!(scope_of_cgroup(&text, UID), Some(Scope::Unnamed));
        }
        assert_eq!(scope_of_cgroup(&cg("yantrik-mindview-zz-1-0"), UID), Some(Scope::Unnamed), "a part that is not hex");
    }

    #[test]
    fn a_launch_needs_the_user_manager_and_says_so_when_it_is_missing() {
        let said = "Mind View needs the user's systemd manager (systemd-run --user); not started".to_string();
        assert_eq!(available_with(true, true), Ok(()));
        assert_eq!(available_with(false, true), Err(said.clone()));
        assert_eq!(available_with(true, false), Err(said));
        let src = include_str!("mind_view.rs");
        let f = &src[src.find("pub fn ensure(").unwrap()..];
        assert!(f[..f.find("\n}\n").unwrap()].contains("mind_view_launched::available()?"), "ensure() checks first");
    }
}
