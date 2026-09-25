//! Turning a pid into something a person can read — and refusing to guess when it cannot.
//!
//! # Why this exists
//!
//! An approval card used to say `Hermes Agent 0.14.0` over the words `self-declared name`, and
//! that second line was the whole of the honesty: the string came from the MCP `initialize`
//! handshake, the bridge passed it through as an argument, and anything that could open the
//! shell's unix socket could put `Your bank` there instead. A person deciding whether to allow
//! something was judging partly by a label nobody had checked (issue #43).
//!
//! The kernel already knew the answer and was being thrown away. `SO_PEERCRED` on an accepted
//! unix socket gives the peer's pid, uid and gid, stamped by the kernel at `connect` from the
//! peer's own process — not from anything the peer wrote. `yantrik-app-runtime::control` reads
//! it at accept and carries it to the handler. The walk up `/proc` from that pid — past `yos`
//! and `yos-mcp`, past bare shells, to the first program a person would recognise — is
//! `yantrik_ipc_transport::peer_identity`, shared with the notifications service since #114
//! (it had the same pid on every `notify` and kept nothing). This module is what the shell adds
//! on top: which attached mind, if any, that ancestry belongs to, and the one disagreement
//! worth interrupting somebody for.
//!
//! # What it can and cannot establish
//!
//! It identifies a **program**, never an intent and never a person. Everything below holds only
//! against a caller that is not already running as this user with the ability to fork whatever
//! it likes — see the "still not verified" section of `design/approvals-2026-09-21.md`.

use yantrik_ipc_transport::peer_identity::{self, basename, clip, is_bridge, line_about, ProcessFacts};

/// One mind the shell has attached, as far as matching a process against it is concerned.
///
/// The pid is the kernel's word: the host records it from `SO_PEERCRED` when the harness
/// attaches, and a caller whose walked ancestry contains it is that harness or a process it
/// started — the same descent `HostTokens` requires before it believes an agent token. The
/// name is the fallback for a harness the host has no pid for, and it is weaker than it sounds:
/// a name with no distinctive word in it ("Pi") matches no command line at all (#206).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mind {
    pub id: String,
    pub name: String,
    /// The process that attached, as the kernel reported it. `None` for built-ins and for
    /// transports that could not say (the TCP dev path).
    pub pid: Option<u32>,
}

/// What this machine established about whoever opened the socket.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CallerIdentity {
    /// The peer itself. `None` when there was no pid, or `/proc` said nothing about it.
    pub direct: Option<ProcessFacts>,
    /// The first ancestor that is not our own plumbing — what a person would recognise.
    pub recognisable: Option<ProcessFacts>,
    /// The attached mind this ancestry belongs to, by name, if one matched.
    pub attached_mind: Option<String>,
    /// Everything that was walked, deepest first. Kept because the peer usually exits within
    /// milliseconds and this is the only record that it was ever there.
    pub chain: Vec<ProcessFacts>,
    /// A bare shell stood between the peer and the recognisable program: somebody typed this,
    /// or a script ran it.
    pub via_shell: bool,
}

impl CallerIdentity {
    /// The one line the card prints under the name the caller gave itself.
    ///
    /// Never empty and never a guess. The three shapes are deliberately different sentences so
    /// that a person can tell "this is the mind you are talking to" from "this is some program"
    /// from "this machine could not tell" without reading carefully.
    pub fn line(&self) -> String {
        // "a program started from a terminal" is for a person's own script. An attached mind
        // that happens to have a shell in its ancestry is still the mind, and says so instead.
        let from_terminal = self.attached_mind.is_none() && self.via_shell;
        let note = if self.attached_mind.is_some() { " \u{b7} the attached mind" } else { "" };
        line_about(self.recognisable.as_ref().or(self.direct.as_ref()), from_terminal, note)
    }

    /// The executable the line is about, for `describe shell` and the audit log.
    pub fn exe(&self) -> String {
        self.recognisable
            .as_ref()
            .or(self.direct.as_ref())
            .map(|f| f.exe.clone())
            .unwrap_or_default()
    }

    /// The pid the line is about. `0` when nothing was established.
    pub fn pid(&self) -> i32 {
        self.recognisable.as_ref().or(self.direct.as_ref()).map(|f| f.pid).unwrap_or(0)
    }

    /// Nothing was knowable. Kept as a named constructor so the "no pid at all" path and the
    /// "`/proc` refused everything" path produce the same card rather than two near-misses.
    pub fn unknown() -> CallerIdentity {
        CallerIdentity::default()
    }
}

// ── The disagreement worth interrupting somebody for ─────────────────

/// How much of the mismatch sentence fits on one elided card line.
const WARNING_CHARS: usize = 58;

/// Does the name the caller gave itself claim to be a mind the ancestry does not support?
///
/// Narrow on purpose. It fires only when the claimed name names a mind that is actually attached
/// to this desktop *and* the verified ancestry belongs to something else — which is the case a
/// person cannot possibly catch by reading, because the name will be exactly right. It stays
/// quiet when `/proc` gave nothing (absence of evidence is not disagreement) and when the claim
/// is some name no mind here uses (there is nothing to contradict: the card already says the
/// name is self-declared and prints the verified program beside it).
pub fn mismatch(claimed: &str, identity: &CallerIdentity, minds: &[Mind]) -> String {
    if identity.chain.is_empty() {
        return String::new();
    }
    let claimed_lower = claimed.trim().to_ascii_lowercase();
    if claimed_lower.is_empty() {
        return String::new();
    }

    let Some(named) = minds.iter().find(|m| {
        let name = m.name.trim().to_ascii_lowercase();
        // "Hermes Agent 0.14.0" claims to be the mind called "Hermes Agent": a version suffix is
        // still the same claim. An id match covers a client that sends its id as its name.
        !name.is_empty()
            && (claimed_lower == name
                || claimed_lower.starts_with(&format!("{name} "))
                || claimed_lower == m.id.trim().to_ascii_lowercase())
    }) else {
        return String::new();
    };

    if identity.attached_mind.as_deref() == Some(named.name.as_str()) {
        return String::new();
    }
    clip(&format!("\u{201c}{}\u{201d} is attached here — this is not it.", named.name), WARNING_CHARS)
}

// ── Choosing, from a chain somebody already walked ───────────────────
//
// Everything below is pure. `resolve` reads `/proc` and then calls this, so the judgement that
// decides what a person is shown can be tested against fixture chains rather than against
// whatever happens to be running on the machine running the tests.

/// Pick the recognisable program and the mind, out of a chain that is already read.
///
/// Which program is `peer_identity::choose`, the same rule every service uses; which mind is
/// the shell's own question, because only the shell knows what is attached.
pub fn identify(chain: Vec<ProcessFacts>, minds: &[Mind]) -> CallerIdentity {
    let program = peer_identity::choose(chain);
    let attached_mind = mind_for(&program.chain, minds);
    CallerIdentity {
        direct: program.direct,
        recognisable: program.recognisable,
        attached_mind,
        chain: program.chain,
        via_shell: program.via_shell,
    }
}

/// Which attached mind, if any, this ancestry belongs to.
///
/// By pid first: the chain was walked up from the caller, so a harness's recorded pid appearing
/// in it means the caller IS that harness or runs under it — the descent `HostTokens` requires
/// before it believes an agent token. That is the only match there is for a mind whose name
/// cannot appear in a command line: the card for a genuine call from pi said “Pi” is attached
/// here — this is not it, because "pi" is two characters and the name tokens below require four
/// (#206). The chain is the outer loop so that the harness nearest the caller wins if two of
/// them are somehow in one ancestry.
///
/// By name when the host has no pid for the harness. Only tokens of four characters or more
/// count, so a mind called "AI" or "OS" cannot match half the process table, and our own bridge
/// processes are excluded from the search: a mind's name appearing in the path of the program we
/// wrote to talk to it would prove nothing.
fn mind_for(chain: &[ProcessFacts], minds: &[Mind]) -> Option<String> {
    for facts in chain {
        if let Some(mind) =
            minds.iter().find(|m| m.pid.is_some_and(|pid| pid > 0 && pid as i32 == facts.pid))
        {
            return Some(mind.name.clone());
        }
    }
    for facts in chain {
        if is_bridge(facts) || is_this_desktop(facts) {
            continue;
        }
        let haystack = facts.haystack();
        if haystack.is_empty() {
            continue;
        }
        for mind in minds {
            if name_tokens(&mind.name)
                .into_iter()
                .chain(name_tokens(&mind.id))
                .any(|token| haystack.contains(&token))
            {
                return Some(mind.name.clone());
            }
        }
    }
    None
}

/// The shell and the programs it ships, which are the ancestry of everything a person starts.
///
/// Every window on this desktop descends from `/opt/yantrik/bin/yantrik-ui`, and every built-in
/// mind is called Yantrik something — so a script run from the Terminal app matched "yantrik"
/// four processes up and was labelled *the attached mind*. That is the one line on the approval
/// card that exists to unmask a program pretending to be a mind, and it was awarding the badge
/// to the pretender: `forge.py`, claiming to be Hermes, was shown as "python3 forge.py · the
/// attached mind". The desktop's own binaries prove that a process was started from the desktop,
/// which is true of nearly everything, and nothing about which mind it is.
fn is_this_desktop(facts: &ProcessFacts) -> bool {
    let exe = facts.exe.as_str();
    exe.starts_with("/opt/yantrik/bin/") || basename(exe).starts_with("yantrik-")
}

/// The words in a mind's name that are distinctive enough to match a path on.
///
/// The OS's own name is not one of them. The built-in minds are all "Yantrik something", and
/// "yantrik" is also the user this desktop runs as, its home directory and every path under
/// `/opt/yantrik` — so `sshd-session: yantrik@notty`, a probe run over ssh, was shown on the
/// card as "the attached mind". The word that tells the minds apart is the other one.
fn name_tokens(name: &str) -> Vec<String> {
    name.to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| t.len() >= 4 && *t != OS_NAME)
        .map(|t| t.to_string())
        .collect()
}

/// The one word that is in every path on this machine and therefore names nothing.
const OS_NAME: &str = "yantrik";

// ── Reading /proc ────────────────────────────────────────────────────

/// Everything knowable about the process that opened the socket, right now.
///
/// Call it at handler time and keep the answer. The direct peer is usually `yos`, which runs one
/// JSON-RPC call and exits, so by the time a person looks at the card it is gone — the chain in
/// [`CallerIdentity`] is the only record that it existed.
pub fn resolve(pid: i32) -> CallerIdentity {
    resolve_with(pid, &attached_minds())
}

/// The same, over a list of minds the caller already has.
///
/// [`mismatch`] needs that list too, and `Host::list` takes a lock and reaps departed harnesses
/// on every call — which is fine once per request and pointless twice, on the UI thread.
///
/// Off Linux there is no `/proc` and the walk is empty, so the Windows dev build says "could not
/// be identified" rather than anything it does not know.
pub fn resolve_with(pid: i32, minds: &[Mind]) -> CallerIdentity {
    identify(peer_identity::walk(pid), minds)
}

/// The minds attached to this desktop, as the picker shows them.
pub fn attached_minds() -> Vec<Mind> {
    crate::wire::harness::host()
        .map(|host| {
            host.list().into_iter().map(|e| Mind { id: e.id, name: e.name, pid: e.pid }).collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod caller_identity_tests {
    use super::*;

    fn facts(pid: i32, exe: &str, cmdline: &str) -> ProcessFacts {
        ProcessFacts {
            pid,
            exe: exe.to_string(),
            short_cmdline: cmdline.to_string(),
            started: pid as u64 * 100,
        }
    }

    fn hermes() -> Vec<Mind> {
        vec![
            Mind { id: "companion".into(), name: "Companion".into(), pid: None },
            Mind { id: "hermes".into(), name: "Hermes Agent".into(), pid: None },
        ]
    }

    /// The real chain, as `debug-proc.sh` printed it on the VM.
    fn hermes_chain() -> Vec<ProcessFacts> {
        vec![
            facts(7311, "/usr/bin/python3.11", "python3 yos act calendar delete_event id=evt-3"),
            facts(7300, "/usr/bin/python3.11", "python3 yos-mcp"),
            facts(696, "/home/pranab/hermes-agent/venv/bin/python", "python -m hermes_cli.main gateway run"),
            facts(1, "/usr/lib/systemd/systemd", "systemd --user"),
        ]
    }

    // ── Choosing what to name ──
    //
    // The walk, the parsers and the program-choosing rule are tested where they live now, in
    // `yantrik_ipc_transport::peer_identity`. These are about what the shell adds: the mind.

    #[test]
    fn caller_identity_the_bridge_is_skipped_and_the_mind_is_named() {
        let who = identify(hermes_chain(), &hermes());

        assert_eq!(who.direct.as_ref().map(|f| f.pid), Some(7311), "the peer is still recorded");
        assert_eq!(
            who.recognisable.as_ref().map(|f| f.pid),
            Some(696),
            "yos and yos-mcp are ours; the first thing a person would recognise is above them"
        );
        assert_eq!(who.attached_mind.as_deref(), Some("Hermes Agent"));
        assert!(who.line().contains("hermes_cli.main"), "{}", who.line());
        assert!(who.line().contains("pid 696"), "{}", who.line());
        assert!(who.line().contains("the attached mind"), "{}", who.line());
        assert_eq!(who.pid(), 696);
        assert!(who.exe().ends_with("venv/bin/python"), "{}", who.exe());
    }

    #[test]
    fn caller_identity_a_bare_yos_in_the_terminal_names_the_terminal() {
        // What somebody typing `yos act shell open_app name=notes` into the Terminal app looks
        // like from the socket: the peer is ours, the shell is plumbing, and the app they are
        // actually sitting in is two steps up.
        let chain = vec![
            facts(9001, "/usr/bin/python3.11", "python3 yos act shell open_app name=notes"),
            facts(8800, "/usr/bin/bash", "bash"),
            facts(812, "/usr/bin/yantrik-terminal", "yantrik-terminal"),
            facts(1, "/usr/lib/systemd/systemd", "systemd --user"),
        ];
        let who = identify(chain, &hermes());

        assert_eq!(who.recognisable.as_ref().map(|f| f.pid), Some(812));
        assert_eq!(who.attached_mind, None, "nobody's mind typed this");
        assert!(who.via_shell, "a bash stood between the peer and the terminal");
        assert!(who.line().contains("yantrik-terminal"), "{}", who.line());
        assert!(who.line().contains("terminal"), "{}", who.line());
    }

    #[test]
    fn a_mind_with_a_shell_in_its_ancestry_is_still_the_mind() {
        // The terminal sentence is for a person's own script; a mind started through `sh -c`
        // is named as the mind, not as "a program started from a terminal".
        let chain = vec![
            facts(7311, "/usr/bin/python3.11", "python3 yos act shell x"),
            facts(7300, "/usr/bin/python3.11", "python3 yos-mcp"),
            facts(7200, "/usr/bin/bash", "sh -c python -m hermes_cli.main gateway run"),
            facts(696, "/home/pranab/hermes-agent/venv/bin/python", "python -m hermes_cli.main gateway run"),
        ];
        let who = identify(chain, &hermes());
        assert!(who.via_shell);
        assert_eq!(who.attached_mind.as_deref(), Some("Hermes Agent"));
        assert!(!who.line().starts_with("a program started from a terminal"), "{}", who.line());
        assert!(who.line().ends_with("the attached mind"), "{}", who.line());
    }

    #[test]
    fn a_script_run_from_the_terminal_app_is_not_the_attached_mind() {
        // forge.py claimed to be Hermes and was launched from the Terminal app, whose ancestry
        // is /opt/yantrik/bin/yantrik-terminal ← /opt/yantrik/bin/yantrik-ui. "yantrik" is a
        // token of "Yantrik Companion", and the card said "python3 forge.py · the attached mind".
        let chain = vec![
            facts(9001, "/usr/bin/python3.13", "python3 forge.py"),
            facts(9000, "/opt/yantrik/bin/yantrik-terminal", "/opt/yantrik/bin/yantrik-terminal"),
            facts(8000, "/opt/yantrik/bin/yantrik-ui", "/opt/yantrik/bin/yantrik-ui /opt/yantrik/config.yaml"),
        ];
        let minds = vec![
            Mind { id: "companion".into(), name: "Yantrik Companion".into(), pid: None },
            Mind { id: "hermes".into(), name: "Hermes Agent".into(), pid: None },
            Mind { id: "mind".into(), name: "Yantrik Mind".into(), pid: None },
        ];
        assert_eq!(mind_for(&chain, &minds), None, "the desktop's own binaries name no mind");
        // ...while a real Hermes gateway process still matches by its own command line.
        let hermes = vec![facts(7000, "/home/u/.local/bin/python3.11", "python -m hermes_cli.main gateway")];
        assert_eq!(mind_for(&hermes, &minds).as_deref(), Some("Hermes Agent"));
    }

    #[test]
    fn the_desktops_own_name_in_a_path_or_a_username_names_no_mind() {
        // Seen live: a probe run over ssh as the desktop's user came up as "sshd-session:
        // yantrik@notty (pid 638879) · the attached mind". "yantrik" is a token of every
        // built-in mind's name, and it is also the username, the home directory and /opt/yantrik.
        let minds = vec![
            Mind { id: "companion".into(), name: "Yantrik Companion".into(), pid: None },
            Mind { id: "hermes".into(), name: "Hermes Agent".into(), pid: None },
        ];
        let ssh = vec![
            facts(5400, "/usr/bin/bash", "bash -c python3 /home/yantrik/forge.py"),
            facts(5300, "/usr/sbin/sshd-session", "sshd-session: yantrik@notty"),
        ];
        assert_eq!(mind_for(&ssh, &minds), None, "{:?}", mind_for(&ssh, &minds));
        let home = vec![facts(5500, "/usr/bin/python3.13", "python3 /home/yantrik/forge.py")];
        assert_eq!(mind_for(&home, &minds), None);
        // The other word still does: the companion's own process is named by it.
        let companion = vec![facts(5600, "/usr/bin/python3.13", "python3 companion_bridge.py")];
        assert_eq!(mind_for(&companion, &minds).as_deref(), Some("Yantrik Companion"));
    }

    #[test]
    fn caller_identity_a_script_over_ssh_names_the_script() {
        let chain = vec![
            facts(5501, "/usr/bin/python3.11", "python3 yos act files delete name=x"),
            facts(5500, "/usr/bin/python3.11", "python3 nightly.py"),
            facts(5400, "/usr/bin/bash", "bash -c python3 /srv/nightly.py"),
            facts(5300, "/usr/sbin/sshd", "sshd: pranab@notty"),
            facts(1, "/usr/lib/systemd/systemd", "systemd"),
        ];
        let who = identify(chain, &hermes());

        assert_eq!(
            who.recognisable.as_ref().map(|f| f.pid),
            Some(5500),
            "the script is above the bridge and below the shell; it is the thing to name"
        );
        assert!(who.line().contains("nightly.py"), "{}", who.line());
        assert_eq!(who.attached_mind, None);
    }

    #[test]
    fn caller_identity_knowing_nothing_says_so_rather_than_leaving_a_blank() {
        let nothing = CallerIdentity::unknown();
        assert_eq!(nothing.line(), "could not be identified");
        assert_eq!(nothing.exe(), "");
        assert_eq!(nothing.pid(), 0);

        // An empty chain is the same answer, and the card must never render an empty line.
        let empty = identify(Vec::new(), &hermes());
        assert_eq!(empty.line(), "could not be identified");
        assert!(!empty.line().is_empty());
    }

    #[test]
    fn caller_identity_a_short_mind_name_cannot_match_half_the_process_table() {
        // A mind called "AI" would otherwise match `/usr/bin/chain` and every path with an `ai`
        // in it. Four characters is the bar, and an id gets the same treatment as a name.
        let minds = vec![Mind { id: "ai".into(), name: "AI".into(), pid: None }];
        let who = identify(hermes_chain(), &minds);
        assert_eq!(who.attached_mind, None);

        // While a real name matches on any one of its distinctive words.
        let who =
            identify(hermes_chain(), &[Mind { id: "h".into(), name: "Hermes".into(), pid: None }]);
        assert_eq!(who.attached_mind.as_deref(), Some("Hermes"));
    }

    #[test]
    fn caller_identity_our_own_bridge_cannot_stand_in_for_a_mind() {
        // If the bridge's own path carried a mind's name, matching it would let any caller at
        // all borrow that mind's identity simply by going through the bridge everybody uses.
        let chain = vec![
            facts(2001, "/usr/bin/python3.11", "python3 /opt/hermes/bin/yos act shell x"),
            facts(2000, "/usr/bin/python3.11", "python3 /opt/hermes/bin/yos-mcp"),
            facts(1900, "/usr/bin/curl", "curl --unix-socket app-shell.sock"),
        ];
        let who = identify(chain, &hermes());
        assert_eq!(who.attached_mind, None, "the path of OUR bridge is not evidence about a mind");
        assert!(who.line().contains("curl"), "{}", who.line());
    }

    // ── The claim against the ancestry ──

    #[test]
    fn caller_identity_a_borrowed_name_is_called_out() {
        // The attack the whole feature is for: something that is not Hermes says it is Hermes.
        let chain = vec![
            facts(2001, "/usr/bin/python3.11", "python3 yos act calendar delete_event id=evt-3"),
            facts(1900, "/home/pranab/tmp/helper", "helper --quiet"),
            facts(1800, "/usr/bin/bash", "bash"),
        ];
        let who = identify(chain, &hermes());
        let said = mismatch("Hermes Agent 0.14.0", &who, &hermes());
        assert!(said.contains("Hermes Agent"), "{said}");
        assert!(!said.is_empty());

        // And the honest case says nothing, so the warning still means something when it fires.
        assert_eq!(mismatch("Hermes Agent 0.14.0", &identify(hermes_chain(), &hermes()), &hermes()), "");
    }

    #[test]
    fn caller_identity_a_name_no_mind_uses_is_not_a_mismatch() {
        // "Your bank" is not a claim this can contradict — no mind here is called that, so there
        // is nothing to disagree with. The card already prints the verified program beside it,
        // which is the answer. A warning here would fire on every ordinary unnamed caller and
        // train people to ignore the one that matters.
        let chain = vec![facts(2001, "/usr/bin/curl", "curl --unix-socket app-shell.sock")];
        let who = identify(chain, &hermes());
        assert_eq!(mismatch("Your bank", &who, &hermes()), "");
        assert_eq!(mismatch("", &who, &hermes()), "");
    }

    #[test]
    fn caller_identity_nothing_known_is_not_a_disagreement() {
        // /proc gave nothing. That is not evidence that the claim is false, and saying so would
        // be the machine asserting something it does not know.
        assert_eq!(mismatch("Hermes Agent", &CallerIdentity::unknown(), &hermes()), "");
    }

    #[test]
    fn caller_identity_the_warning_is_one_bounded_line() {
        // The card's height is arithmetic; a warning that wrapped to three lines would push the
        // buttons off the bottom of the screen, which is the defect this card already had once.
        let long = vec![Mind {
            id: "x".into(),
            name: "A Mind With A Preposterously Long Self Chosen Name Indeed".into(),
            pid: None,
        }];
        let chain = vec![facts(2001, "/usr/bin/curl", "curl")];
        let said = mismatch(&long[0].name, &identify(chain, &long), &long);
        assert!(!said.is_empty());
        assert!(said.chars().count() <= WARNING_CHARS + 1, "{} chars: {said}", said.chars().count());
        assert!(!said.contains('\n'), "one line: {said}");
    }

    // ── The walk, from here ──

    #[cfg(target_os = "linux")]
    #[test]
    fn caller_identity_a_pid_that_is_not_there_is_not_an_error() {
        // Every /proc read is allowed to fail, and the commonest one does: the direct peer is
        // `yos`, which has usually exited before anybody looks at the card.
        let gone = resolve(0);
        assert_eq!(gone.line(), "could not be identified");
    }

    // ── The mind the kernel can name when the command line cannot (#206) ──

    #[test]
    fn a_call_from_pi_itself_is_the_attached_mind_however_short_its_name() {
        // The live card of #206: pi called for itself, verified by pid and by its agent token
        // `pi:main`, and the card still said in red that "Pi" is attached here and this is not
        // it. The name "Pi" has no token of four characters, so matching it against a command
        // line can never succeed; the pid the kernel stamped at attach is the match that can.
        let minds = vec![
            Mind { id: "companion".into(), name: "Yantrik Companion".into(), pid: None },
            Mind { id: "pi".into(), name: "Pi".into(), pid: Some(4242) },
        ];
        let adapter = || facts(4242, "/usr/bin/python3.11", "python3 yantrik_pi.py");
        let pi = || facts(102549, "/usr/local/bin/pi", "pi --mode agent");

        // The adapter itself: the process that attached is the peer.
        let who = identify(vec![adapter()], &minds);
        assert_eq!(who.attached_mind.as_deref(), Some("Pi"));
        assert_eq!(mismatch("Pi 1.0", &who, &minds), "", "the real Pi is not an impostor");

        // Its child: pi running a turn.
        let who = identify(vec![pi(), adapter()], &minds);
        assert_eq!(who.attached_mind.as_deref(), Some("Pi"));
        assert_eq!(mismatch("Pi 1.0", &who, &minds), "");

        // The grandchild chain a real act arrives over: yos ← yos-mcp ← pi ← the adapter.
        let who = identify(
            vec![
                facts(102600, "/usr/bin/python3.11", "python3 yos act files delete name=x"),
                facts(102590, "/usr/bin/python3.11", "python3 yos-mcp"),
                pi(),
                adapter(),
            ],
            &minds,
        );
        assert_eq!(who.attached_mind.as_deref(), Some("Pi"));
        assert_eq!(mismatch("Pi 1.0", &who, &minds), "");
        assert!(who.line().contains("the attached mind"), "{}", who.line());

        // An unrelated process claiming the name is still called out: no descent, and a name
        // this short cannot match anything by accident — which is the point of keeping the
        // pid the kernel gave rather than softening the claim check.
        let who = identify(
            vec![
                facts(9001, "/usr/bin/python3.13", "python3 forge.py"),
                facts(9000, "/usr/bin/bash", "bash"),
            ],
            &minds,
        );
        assert_eq!(who.attached_mind, None);
        assert!(mismatch("Pi 1.0", &who, &minds).contains("is attached here"), "{who:?}");
    }

    #[test]
    fn a_harness_the_host_has_no_pid_for_still_matches_by_name() {
        // Attached over the TCP dev path, or a built-in: nothing to descend from, so the
        // ancestry is matched the weaker way it always was — and a claim it does not support
        // is still called out.
        let who = identify(hermes_chain(), &hermes());
        assert_eq!(who.attached_mind.as_deref(), Some("Hermes Agent"));
        assert_eq!(mismatch("Hermes Agent 0.14.0", &who, &hermes()), "");

        // A pid of zero is no pid: the transport writes 0 when the kernel gave none, and 0
        // must not match whatever the walk makes of a process it could not read.
        let zero = vec![Mind { id: "pi".into(), name: "Pi".into(), pid: Some(0) }];
        let who = identify(vec![facts(9001, "/usr/bin/curl", "curl")], &zero);
        assert_eq!(who.attached_mind, None);
    }
}
