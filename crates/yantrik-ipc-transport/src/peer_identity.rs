//! What `/proc` says about the process behind a [`PeerCred`](crate::PeerCred) — and refusing to
//! guess when it says nothing.
//!
//! # Why this is in the transport
//!
//! The kernel stamps the peer's pid on every accepted unix socket (`SO_PEERCRED`, read at accept
//! in `server.rs`) and for a while the shell was the only thing that did anything with it: its
//! approval card walks up from that pid, past our own `yos`/`yos-mcp` plumbing, to the first
//! program a person would recognise, and prints that beside the name the caller gave itself
//! (issue #43). The notifications service had the same pid on every `notify` and threw it away,
//! so a mind could post as `app: "Yantrik"` and the store kept no record of who had really
//! called (issue #114).
//!
//! The walk is the same question in both places — *which program is on this socket?* — so it
//! lives here, next to the credential it starts from, where every service and the shell can
//! reach it. What the shell adds on top (matching the ancestry against an attached mind, and
//! calling out a borrowed name) stays in `crates/yantrik-ui/src/caller_identity.rs`, because
//! only the shell knows which minds are attached.
//!
//! # What it can and cannot establish
//!
//! It identifies a **program**, never an intent and never a person. Everything below holds only
//! against a caller that is not already running as this user with the ability to fork whatever
//! it likes — see the "still not verified" section of `design/approvals-2026-09-21.md`.
//!
//! # The shape of a real chain
//!
//! The direct peer is almost never the interesting process. For a request from Hermes it is:
//!
//! ```text
//! python3 /opt/yantrik/bin/yos                    ← the peer. Short-lived; usually already gone.
//! python3 /opt/yantrik/bin/yos-mcp                ← our bridge
//! …/hermes-agent/venv/bin/python -m hermes_cli…   ← the first thing a person would recognise
//! systemd --user                                  ← stop
//! ```
//!
//! So the walk goes up, skipping our own plumbing (`yos`, `yos-mcp`) and bare shells, and the
//! first thing left is what gets named. Bounded at [`MAX_ANCESTORS`], cycle-safe, and every
//! `/proc` read is allowed to fail — the peer in particular has usually exited by the time
//! anyone looks, which is why the chain is captured at handler time and kept.

/// How far up the process tree to walk.
///
/// Eight covers every real chain on this desktop with room to spare (Hermes' is four, a bare
/// `yos act` from the Terminal app is four). A bound rather than a loop-until-init because this
/// runs inside a request handler, and `/proc` on a busy machine is not free.
pub const MAX_ANCESTORS: usize = 8;

/// How much of a command line is kept. One line at `fs-micro` in a 404px card.
pub const CMDLINE_CHARS: usize = 72;

/// Our own plumbing between a mind and a socket. Never the answer to "who is asking".
const BRIDGE_PROGRAMS: &[&str] = &["yos", "yos-mcp"];

/// Shells that are a way of starting something rather than a thing somebody would name.
const SHELLS: &[&str] = &["sh", "bash", "dash", "zsh", "ksh", "fish", "busybox", "ash"];

/// Interpreters, which name nothing on their own: `python3` is not what a person would call
/// the program, the script or module it was given to run is.
const INTERPRETERS: &[&str] = &["node", "nodejs", "deno", "bun", "perl", "ruby", "php", "lua", "java"];

/// Where a walk stops. pid 1 by any name, and the user session manager above every app.
const ROOTS: &[&str] = &["systemd", "init"];

/// How much of the verified line fits on one elided card row at `fs-micro` in a 404px card.
///
/// The card's height is arithmetic, so this row is one line and a long command line is cut
/// rather than wrapped. The pid and the suffix are never the part that gets cut: they are what
/// makes the line checkable against `ps`.
pub const LINE_CHARS: usize = 66;

/// What the line says when there was nothing to say it about. One spelling, so the shell's card
/// and a notification's sender line cannot drift into two near-misses.
pub const UNIDENTIFIED: &str = "could not be identified";

/// One process, as `/proc` described it at the moment somebody looked.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProcessFacts {
    pub pid: i32,
    /// `/proc/<pid>/exe` resolved, or empty when it could not be read (the process is gone, or
    /// it belongs to another user and this one may not look).
    pub exe: String,
    /// The command line, absolute paths cut to their basenames and the whole thing bounded to
    /// one card line. Empty for a kernel thread, or when `/proc` gave nothing.
    pub short_cmdline: String,
    /// Field 22 of `/proc/<pid>/stat` — the process's start time in clock ticks since boot.
    ///
    /// This is the only thing that distinguishes a pid from the pid it will be reused as. It is
    /// read before and after the other two files and the facts are thrown away if it moved, so
    /// a chain entry can never be half one process and half another.
    pub started: u64,
}

impl ProcessFacts {
    /// What to call this program on a card. The command line if there is one, else the binary.
    pub fn label(&self) -> String {
        if !self.short_cmdline.is_empty() {
            return self.short_cmdline.clone();
        }
        if !self.exe.is_empty() {
            return basename(&self.exe).to_string();
        }
        format!("pid {}", self.pid)
    }

    /// Everything about this process as one lowercase haystack, for name matching.
    pub fn haystack(&self) -> String {
        format!("{} {}", self.exe, self.short_cmdline).to_ascii_lowercase()
    }
}

/// What this machine established about whoever opened the socket.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Program {
    /// The peer itself. `None` when there was no pid, or `/proc` said nothing about it.
    pub direct: Option<ProcessFacts>,
    /// The first ancestor that is not our own plumbing — what a person would recognise.
    pub recognisable: Option<ProcessFacts>,
    /// Everything that was walked, deepest first. Kept because the peer usually exits within
    /// milliseconds and this is the only record that it was ever there.
    pub chain: Vec<ProcessFacts>,
    /// A bare shell stood between the peer and the recognisable program: somebody typed this,
    /// or a script ran it.
    pub via_shell: bool,
}

impl Program {
    /// The process the line is about: the recognisable one, or failing that the peer.
    ///
    /// Everything above the peer may be our own plumbing or unreadable, in which case naming
    /// the peer is still worth more than naming nothing: it is a real pid and a real binary,
    /// just not an interesting one.
    pub fn found(&self) -> Option<&ProcessFacts> {
        self.recognisable.as_ref().or(self.direct.as_ref())
    }

    /// The one line to print under the name the caller gave itself. Never empty, never a guess.
    pub fn line(&self) -> String {
        line_about(self.found(), self.via_shell, "")
    }

    /// The executable the line is about, for a log and for `describe`.
    pub fn exe(&self) -> String {
        self.found().map(|f| f.exe.clone()).unwrap_or_default()
    }

    /// The pid the line is about. `0` when nothing was established.
    pub fn pid(&self) -> i32 {
        self.found().map(|f| f.pid).unwrap_or(0)
    }

    /// The program's own name, for filing something under when the caller gave no name.
    ///
    /// The binary's basename, unless the binary is an interpreter — `python`, `node`, a shell —
    /// in which case it names nothing and the first thing it was given to run does:
    /// `hermes_cli.main` for `python -m hermes_cli.main gateway run`, `forge.py` for
    /// `python3 forge.py`, `deploy.sh` for `bash deploy.sh`. Empty when nothing was established,
    /// so the caller decides what to say about that rather than this inventing a word.
    ///
    /// A title a daemon rewrote over its own argv — `sshd-session: yantrik@notty` — names which
    /// session a call arrived through, never which program made it, so a peer that has a name
    /// of its own wins over it (issue #151).
    pub fn name(&self) -> String {
        let Some(found) = self.found() else {
            return String::new();
        };
        // `ssh yantrik@vm 'yos notify …'` was filed under `sshd-session:` (#151): everything
        // above the peer was plumbing, so the walk ended at the session title sshd rewrites
        // into its own argv[0] — and on the VM even the user's own `sshd-session` refuses
        // `/proc/<pid>/exe`, so all that was left of it was that title, colon included. The
        // peer is `python3 /opt/yantrik/bin/yos`, an interpreter running a script, and the
        // script is a program name; the session the call arrived through is not one.
        if is_rewritten_title(found) {
            if let Some(direct) = &self.direct {
                if direct.pid != found.pid {
                    if let Some(script) = script_of(direct) {
                        return script;
                    }
                }
            }
        }
        name_of(found)
    }

    /// Nothing was knowable. Kept as a named constructor so the "no pid at all" path and the
    /// "`/proc` refused everything" path produce the same answer rather than two near-misses.
    pub fn unknown() -> Program {
        Program::default()
    }
}

/// The line, from its parts.
///
/// The three shapes are deliberately different sentences so that a person can tell "this is
/// some program" from "somebody typed this" from "this machine could not tell" without reading
/// carefully. `note` goes after the pid — the shell puts ` · the attached mind` there — and
/// `from_terminal` says whether to prefix the terminal sentence, which the shell suppresses for
/// an attached mind.
pub fn line_about(found: Option<&ProcessFacts>, from_terminal: bool, note: &str) -> String {
    let Some(found) = found else {
        return UNIDENTIFIED.to_string();
    };
    let prefix = if from_terminal { "a program started from a terminal: " } else { "" };
    let tail = format!(" (pid {}){note}", found.pid);
    // The label is the only part that may be cut: the pid and the suffix are what makes this
    // line checkable against `ps`, and a truncated pid would be worse than no pid at all.
    let room = LINE_CHARS.saturating_sub(prefix.chars().count() + tail.chars().count());
    format!("{prefix}{}{tail}", clip(&found.label(), room.max(8)))
}

// ── Choosing, from a chain somebody already walked ───────────────────
//
// Everything here is pure. `resolve` reads `/proc` and then calls this, so the judgement that
// decides what a person is shown can be tested against fixture chains rather than against
// whatever happens to be running on the machine running the tests.

/// Pick the recognisable program out of a chain that is already read.
pub fn choose(chain: Vec<ProcessFacts>) -> Program {
    let direct = chain.first().cloned();

    // Skip our own plumbing and bare shells, and never name the session manager: "systemd --user
    // is asking to use this machine" is true of literally everything and tells nobody anything.
    let mut via_shell = false;
    let mut recognisable = None;
    for facts in &chain {
        if is_root(facts) {
            break;
        }
        if is_bridge(facts) {
            continue;
        }
        if is_bare_shell(facts) {
            via_shell = true;
            continue;
        }
        recognisable = Some(facts.clone());
        break;
    }

    Program { direct, recognisable, chain, via_shell }
}

/// Everything knowable about the process that opened the socket, right now.
///
/// Call it at handler time and keep the answer. The direct peer is usually `yos`, which runs one
/// JSON-RPC call and exits, so by the time anyone looks later it is gone — the chain in
/// [`Program`] is the only record that it existed. `None` — no credentials at all — is the same
/// answer as a pid `/proc` knows nothing about.
pub fn resolve(pid: Option<i32>) -> Program {
    match pid {
        Some(pid) => choose(walk(pid)),
        None => Program::unknown(),
    }
}

/// `yos` or `yos-mcp`, however they were started.
///
/// Both are Python scripts, so the executable is `python3` and the name that matters is in the
/// command line. Only the first two tokens are looked at: `yos act notes append text='yos'`
/// must not make Notes' own arguments decide what this is.
pub fn is_bridge(facts: &ProcessFacts) -> bool {
    if BRIDGE_PROGRAMS.contains(&basename(&facts.exe)) {
        return true;
    }
    facts
        .short_cmdline
        .split_whitespace()
        .take(2)
        .any(|token| BRIDGE_PROGRAMS.contains(&basename(token)))
}

/// A shell with nothing of its own to say.
///
/// `bash /home/pranab/deploy.sh` is a script somebody wrote and is exactly what the card should
/// name. `bash`, `-bash` and `sh -c '…'` are the way something else was started, and naming them
/// would tell a person only that a shell exists.
fn is_bare_shell(facts: &ProcessFacts) -> bool {
    let name = basename(&facts.exe);
    let argv0 = facts.short_cmdline.split_whitespace().next().unwrap_or("");
    // A login shell is `-bash` in argv[0] and `bash` as the binary.
    let shell = SHELLS.contains(&name) || SHELLS.contains(&basename(argv0.trim_start_matches('-')));
    if !shell {
        return false;
    }
    let rest: Vec<&str> = facts.short_cmdline.split_whitespace().skip(1).collect();
    match rest.first() {
        None => true,
        // `sh -c '…'` is how something else was started, and the something else is its child.
        Some(first) if *first == "-c" => true,
        // Flags only (`bash -l`, `sh -i`) is still a bare shell; a path is a script.
        _ => rest.iter().all(|token| token.starts_with('-')),
    }
}

/// A binary that runs what it is handed rather than being the program itself.
fn is_interpreter(name: &str) -> bool {
    name.starts_with("python") || INTERPRETERS.contains(&name) || SHELLS.contains(&name)
}

/// One process's own name: what an interpreter was given to run, else the binary's basename,
/// else — when even the executable could not be read — the first word of the command line.
fn name_of(found: &ProcessFacts) -> String {
    let binary = basename(&found.exe);
    let argv0 = basename(
        found.short_cmdline.split_whitespace().next().unwrap_or("").trim_start_matches('-'),
    );
    let own = if binary.is_empty() {
        // The command line is what is left, and a title the daemon rewrote is cut at its
        // punctuation: the name is `sshd-session`, never `sshd-session:` (#151). A space
        // needs no cutting; the first word already stopped at it.
        match argv0.split_once(':') {
            Some((head, _)) => head,
            None => argv0,
        }
    } else {
        binary
    };
    script_of(found).unwrap_or_else(|| own.to_string())
}

/// What an interpreter was given to run: `hermes_cli.main` for
/// `python -m hermes_cli.main gateway run`, `forge.py` for `python3 forge.py`. `None` when
/// this process is not an interpreter or was given nothing to run — an interpreter with
/// nothing to run is still only itself, and so is anything that is not one.
fn script_of(found: &ProcessFacts) -> Option<String> {
    let binary = basename(&found.exe);
    let mut tokens = found.short_cmdline.split_whitespace();
    let argv0 = basename(tokens.next().unwrap_or("").trim_start_matches('-'));
    if !is_interpreter(binary) && !is_interpreter(argv0) {
        return None;
    }
    let mut module_next = false;
    for token in tokens {
        if module_next {
            return Some(token.to_string());
        }
        if token == "-m" {
            module_next = true;
            continue;
        }
        if token.starts_with('-') {
            continue;
        }
        return Some(basename(token).to_string());
    }
    None
}

/// A process title a daemon rewrote over its own argv: `sshd-session: yantrik@notty`,
/// `postgres: writer process`, `nginx: worker process`. The `name: detail` shape is the
/// daemon describing itself; it says which session or which role, not which program called.
fn is_rewritten_title(facts: &ProcessFacts) -> bool {
    facts.short_cmdline.split_whitespace().next().is_some_and(|argv0| argv0.contains(':'))
}

/// pid 1, or the user's session manager. The walk stops here and never names it.
fn is_root(facts: &ProcessFacts) -> bool {
    if facts.pid <= 1 {
        return true;
    }
    let argv0 = facts.short_cmdline.split_whitespace().next().unwrap_or("");
    ROOTS.contains(&basename(&facts.exe)) || ROOTS.contains(&basename(argv0))
}

// ── Reading /proc ────────────────────────────────────────────────────

/// Walk up from `pid`, deepest first, tolerating everything.
///
/// Cycle-safe by remembering what it has seen rather than by trusting that a process tree is a
/// tree: `/proc` is read one file at a time and a pid that was reused between two reads could
/// otherwise send this round forever.
#[cfg(target_os = "linux")]
pub fn walk(pid: i32) -> Vec<ProcessFacts> {
    let mut chain = Vec::new();
    let mut seen: std::collections::HashSet<i32> = std::collections::HashSet::new();
    let mut at = pid;

    while chain.len() < MAX_ANCESTORS && at > 0 && seen.insert(at) {
        let Some((facts, ppid)) = facts(at) else { break };
        let stop = is_root(&facts);
        chain.push(facts);
        if stop {
            break;
        }
        at = ppid;
    }
    chain
}

/// No `/proc` to read. Everything on this path is honest about knowing nothing, which is what
/// the Windows dev build should say.
#[cfg(not(target_os = "linux"))]
pub fn walk(pid: i32) -> Vec<ProcessFacts> {
    let _ = pid;
    Vec::new()
}

/// One process's facts, or `None` if it moved underneath the read.
///
/// `stat` is read twice around the other two files. If the start time changed, the pid was
/// reused between the reads and the exe and command line belong to a different process than the
/// one the parent pointer came from — which is exactly the sort of fact that must never reach a
/// card. Nothing is better than something half true.
#[cfg(target_os = "linux")]
fn facts(pid: i32) -> Option<(ProcessFacts, i32)> {
    let before = parse_stat(&std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)?;
    let exe = std::fs::read_link(format!("/proc/{pid}/exe"))
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
    let after = parse_stat(&std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)?;
    if before.started != after.started {
        return None;
    }
    Some((
        ProcessFacts {
            pid,
            exe,
            short_cmdline: parse_cmdline(&cmdline),
            started: before.started,
        },
        before.ppid,
    ))
}

// ── The text parsers ─────────────────────────────────────────────────

/// What `/proc/<pid>/stat` says about lineage and age.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StatFacts {
    pub ppid: i32,
    pub started: u64,
}

/// Parse `/proc/<pid>/stat`, from the LAST `)`.
///
/// The second field is the executable's name in parentheses and the kernel does not escape it.
/// A program called `my (weird) name` produces `123 (my (weird) name) S 1 …`, so splitting on
/// whitespace from the left, or finding the first `)`, gets the wrong fields — and the fields
/// this wants are the parent pid and the start time, which is to say the two that decide which
/// process the card is about.
///
/// Numbering is the kernel's, one-based: state is 3, ppid is 4, starttime is 22. After the last
/// `)` the first token is field 3, so field N is at index N-3.
pub fn parse_stat(text: &str) -> Option<StatFacts> {
    let tail = &text[text.rfind(')')? + 1..];
    let fields: Vec<&str> = tail.split_whitespace().collect();
    Some(StatFacts {
        ppid: fields.get(4 - 3)?.parse().ok()?,
        started: fields.get(22 - 3)?.parse().ok()?,
    })
}

/// Parse `PPid` out of `/proc/<pid>/status`.
///
/// A second way to the same number, kept because `status` is the readable one and a future
/// reader will reach for it. Not used by the walk, which needs the start time anyway and takes
/// both from one read of `stat`.
pub fn parse_ppid(status: &str) -> Option<i32> {
    for line in status.lines() {
        let (key, value) = line.split_once(':')?;
        if key.trim() == "PPid" {
            return value.trim().parse().ok();
        }
    }
    None
}

/// Turn a NUL-separated `/proc/<pid>/cmdline` into one readable line.
///
/// Three things happen here and each of them is about fitting on a card. The separators are NUL
/// bytes, and there is usually a trailing one, so a naive split ends in an empty token. Absolute
/// paths are cut to their basenames, because `/home/pranab/src/hermes-agent/venv/bin/python` is
/// the same information as `python` plus 45 characters somebody has to read past. And the whole
/// thing is bounded, naming its true length when it is cut.
///
/// A kernel thread has an empty `cmdline`; so does a process that exited between the `stat` read
/// and this one. Both come back as an empty string and the caller shows the executable instead.
pub fn parse_cmdline(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    let joined: Vec<String> = text
        .split('\0')
        .filter(|t| !t.is_empty())
        .map(|token| {
            if token.starts_with('/') && token.len() > 1 {
                basename(token).to_string()
            } else {
                token.to_string()
            }
        })
        .collect();
    clip(&joined.join(" "), CMDLINE_CHARS)
}

/// The last path component, or the whole string when there is no separator.
pub fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Cut without splitting a character, and say that it was cut. A card line is a known number of
/// characters.
pub fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let head: String = text.chars().take(max).collect();
    format!("{head}\u{2026}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(pid: i32, exe: &str, cmdline: &str) -> ProcessFacts {
        ProcessFacts {
            pid,
            exe: exe.to_string(),
            short_cmdline: cmdline.to_string(),
            started: pid as u64 * 100,
        }
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

    // ── The /proc text parsers ──

    #[test]
    fn stat_is_parsed_from_the_last_paren() {
        // The ordinary case first: `1234 (bash) S 1200 …`, starttime at field 22.
        let ordinary = "1234 (bash) S 1200 1234 1234 34816 1300 4194304 900 0 0 0 5 2 0 0 20 0 \
                        1 0 987654 12345678 900 18446744073709551615";
        let parsed = parse_stat(ordinary).expect("an ordinary stat line");
        assert_eq!(parsed.ppid, 1200);
        assert_eq!(parsed.started, 987654);

        // And the one that breaks every naive parser: the comm field is whatever the program
        // called itself, parentheses and spaces included, and the kernel does not escape it.
        let awkward = "1234 (my (weird) name) S 1200 1234 1234 34816 1300 4194304 900 0 0 0 5 2 \
                       0 0 20 0 1 0 987654 12345678 900 18446744073709551615";
        let parsed = parse_stat(awkward).expect("a comm with spaces and parens");
        assert_eq!(parsed.ppid, 1200, "splitting from the left would have read `weird` here");
        assert_eq!(parsed.started, 987654);
    }

    #[test]
    fn an_unreadable_stat_is_none_not_a_guess() {
        assert_eq!(parse_stat(""), None, "a process that vanished mid-read");
        assert_eq!(parse_stat("1234 (bash"), None, "a truncated line");
        assert_eq!(parse_stat("1234 (bash) S 1200"), None, "no start time in it");
    }

    #[test]
    fn ppid_comes_out_of_status_too() {
        let status = "Name:\tyos\nUmask:\t0022\nState:\tS (sleeping)\nTgid:\t7311\n\
                      Ngid:\t0\nPid:\t7311\nPPid:\t7300\nTracerPid:\t0\nUid:\t1000\t1000\n";
        assert_eq!(parse_ppid(status), Some(7300));
        assert_eq!(parse_ppid("Name:\tinit\n"), None, "no PPid line at all");
        assert_eq!(parse_ppid(""), None, "an empty read is not a parent of zero");
    }

    #[test]
    fn cmdline_is_nul_separated_and_shortened() {
        // What /proc actually hands over, trailing NUL and all.
        let raw = b"/home/pranab/hermes-agent/venv/bin/python\0-m\0hermes_cli.main\0gateway\0run\0";
        assert_eq!(parse_cmdline(raw), "python -m hermes_cli.main gateway run");

        // A relative path is left alone: `./deploy.py` is what the person typed.
        assert_eq!(parse_cmdline(b"python3\0./deploy.py\0"), "python3 ./deploy.py");

        // A kernel thread, and a process that exited between two reads.
        assert_eq!(parse_cmdline(b""), "");
        assert_eq!(parse_cmdline(b"\0\0"), "");

        // Bounded, and it says so rather than pretending that was the whole command.
        let long = parse_cmdline(&[b"yos\0act\0notes\0append\0text=".to_vec(), vec![b'x'; 200]].concat());
        assert!(long.chars().count() <= CMDLINE_CHARS + 1, "{long}");
        assert!(long.ends_with('\u{2026}'), "a cut line has to look cut: {long}");
    }

    // ── Choosing what to name ──

    #[test]
    fn the_bridge_is_skipped_and_the_program_above_it_is_named() {
        let who = choose(hermes_chain());

        assert_eq!(who.direct.as_ref().map(|f| f.pid), Some(7311), "the peer is still recorded");
        assert_eq!(
            who.recognisable.as_ref().map(|f| f.pid),
            Some(696),
            "yos and yos-mcp are ours; the first thing a person would recognise is above them"
        );
        assert!(who.line().contains("hermes_cli.main"), "{}", who.line());
        assert!(who.line().contains("pid 696"), "{}", who.line());
        assert_eq!(who.pid(), 696);
        assert!(who.exe().ends_with("venv/bin/python"), "{}", who.exe());
    }

    #[test]
    fn a_bare_yos_in_the_terminal_names_the_terminal() {
        // What somebody typing `yos act shell open_app name=notes` into the Terminal app looks
        // like from the socket: the peer is ours, the shell is plumbing, and the app they are
        // actually sitting in is two steps up.
        let chain = vec![
            facts(9001, "/usr/bin/python3.11", "python3 yos act shell open_app name=notes"),
            facts(8800, "/usr/bin/bash", "bash"),
            facts(812, "/usr/bin/yantrik-terminal", "yantrik-terminal"),
            facts(1, "/usr/lib/systemd/systemd", "systemd --user"),
        ];
        let who = choose(chain);

        assert_eq!(who.recognisable.as_ref().map(|f| f.pid), Some(812));
        assert!(who.via_shell, "a bash stood between the peer and the terminal");
        assert!(who.line().starts_with("a program started from a terminal: "), "{}", who.line());
        assert!(who.line().contains("yantrik-terminal"), "{}", who.line());
    }

    #[test]
    fn a_script_over_ssh_names_the_script() {
        let chain = vec![
            facts(5501, "/usr/bin/python3.11", "python3 yos act files delete name=x"),
            facts(5500, "/usr/bin/python3.11", "python3 nightly.py"),
            facts(5400, "/usr/bin/bash", "bash -c python3 /srv/nightly.py"),
            facts(5300, "/usr/sbin/sshd", "sshd: pranab@notty"),
            facts(1, "/usr/lib/systemd/systemd", "systemd"),
        ];
        let who = choose(chain);
        assert_eq!(
            who.recognisable.as_ref().map(|f| f.pid),
            Some(5500),
            "the script is above the bridge and below the shell; it is the thing to name"
        );
        assert!(who.line().contains("nightly.py"), "{}", who.line());
    }

    #[test]
    fn a_bash_running_a_script_is_not_a_bare_shell() {
        // The distinction the skip list rests on. `bash deploy.sh` is a program somebody wrote.
        let chain = vec![
            facts(4400, "/usr/bin/python3.11", "python3 yos describe shell"),
            facts(4300, "/usr/bin/bash", "bash deploy.sh"),
            facts(1, "/usr/lib/systemd/systemd", "systemd --user"),
        ];
        let who = choose(chain);
        assert_eq!(who.recognisable.as_ref().map(|f| f.pid), Some(4300));
        assert!(who.line().contains("deploy.sh"), "{}", who.line());
    }

    #[test]
    fn nothing_recognisable_still_names_the_peer() {
        // Everything above the peer was ours or unreadable. Naming the peer is worth more than
        // naming nothing: it is a real pid and a real binary, just not an interesting one.
        let chain = vec![
            facts(4242, "/usr/bin/python3.11", "python3 script.py"),
            facts(4100, "/usr/bin/bash", "bash"),
        ];
        let who = choose(chain);
        assert_eq!(who.recognisable.as_ref().map(|f| f.pid), Some(4242));
        assert!(who.line().contains("script.py"), "{}", who.line());

        // And when the ONLY thing in the chain is our own plumbing, there is nothing above it.
        let only_ours = choose(vec![facts(4242, "/usr/bin/python3.11", "python3 yos act shell x")]);
        assert_eq!(only_ours.recognisable, None);
        assert!(only_ours.line().contains("pid 4242"), "{}", only_ours.line());
    }

    #[test]
    fn knowing_nothing_says_so_rather_than_leaving_a_blank() {
        let nothing = Program::unknown();
        assert_eq!(nothing.line(), UNIDENTIFIED);
        assert_eq!(nothing.exe(), "");
        assert_eq!(nothing.pid(), 0);
        assert_eq!(nothing.name(), "");

        // An empty chain and no credentials at all are the same answer, and a card must never
        // render an empty line.
        assert_eq!(choose(Vec::new()).line(), UNIDENTIFIED);
        assert_eq!(resolve(None).line(), UNIDENTIFIED);
    }

    #[test]
    fn the_session_manager_is_never_the_answer() {
        // "systemd --user is asking to use this machine" is true of everything on this desktop.
        let chain = vec![
            facts(3001, "/usr/bin/python3.11", "python3 yos act shell x"),
            facts(1, "/usr/lib/systemd/systemd", "systemd --user"),
        ];
        let who = choose(chain);
        assert_eq!(who.recognisable, None, "the walk stops at the session manager");
        assert!(!who.line().contains("systemd"), "{}", who.line());
    }

    #[test]
    fn the_line_takes_a_note_after_the_pid_and_never_cuts_it() {
        // The shell appends ` · the attached mind`; whatever is appended, the pid and the note
        // survive and only the label is cut.
        let long = facts(77, "/usr/bin/python3", &"x".repeat(200));
        let line = line_about(Some(&long), false, " \u{b7} the attached mind");
        assert!(line.ends_with("(pid 77) \u{b7} the attached mind"), "{line}");
        assert!(line.contains('\u{2026}'), "a cut label has to look cut: {line}");
        assert!(line.chars().count() <= LINE_CHARS + 1 + " \u{b7} the attached mind".len(), "{line}");
        // And the terminal prefix is the caller's decision, not this function's.
        assert!(line_about(Some(&long), true, "").starts_with("a program started from a terminal: "));
    }

    // ── The program's own name ──

    #[test]
    fn a_program_is_named_by_what_it_runs_not_by_its_interpreter() {
        // Found on 22 September (#114): a mind's `notify` with no `app` was filed as `Yantrik`.
        // The honest default is the program that called, and for a Python program that is not
        // `python`.
        assert_eq!(choose(hermes_chain()).name(), "hermes_cli.main");
        assert_eq!(
            choose(vec![facts(9001, "/usr/bin/python3.13", "python3 forge.py")]).name(),
            "forge.py"
        );
        assert_eq!(
            choose(vec![facts(4300, "/usr/bin/bash", "bash deploy.sh")]).name(),
            "deploy.sh"
        );
        assert_eq!(
            choose(vec![facts(4301, "/usr/bin/python3", "python3 ./deploy.py")]).name(),
            "deploy.py"
        );
        assert_eq!(
            choose(vec![facts(4302, "/usr/bin/node", "node --inspect server.js")]).name(),
            "server.js",
            "flags before the script are skipped"
        );
        // A native binary is its own name.
        assert_eq!(
            choose(vec![facts(812, "/opt/yantrik/bin/yantrik-terminal", "yantrik-terminal")]).name(),
            "yantrik-terminal"
        );
        assert_eq!(
            choose(vec![facts(7456, "/opt/yantrik/bin/yantrik-ui", "yantrik-ui config.yaml")]).name(),
            "yantrik-ui"
        );
        // When even the executable could not be read, the command line is what is left.
        assert_eq!(choose(vec![facts(5, "", "curl --unix-socket x")]).name(), "curl");
        // An interpreter given nothing to run is still only itself.
        assert_eq!(choose(vec![facts(6, "/usr/bin/python3", "python3")]).name(), "python3");
    }

    #[test]
    fn a_call_that_arrived_over_ssh_is_named_by_the_script_that_made_it() {
        // Found on the VM right after #139 was deployed (#151): `ssh yantrik@vm 'yos notify …'`
        // was filed under `sshd-session:`. Everything above the peer was plumbing — a bare
        // shell, then the session title sshd rewrites into its own argv[0] — and the walk
        // named the title. The session is how the call arrived, not the program that made it:
        // the peer is `python3 /opt/yantrik/bin/yos`, and the script it runs is the name.
        // On the VM even the user's own `sshd-session` refuses `/proc/<pid>/exe`
        // (Permission denied), so all that is left of it is the command line, colon and all.
        let chain = vec![
            facts(52590, "/usr/bin/python3.11", "python3 yos notify Deploy check today=build"),
            facts(52589, "/usr/bin/bash", "bash"),
            facts(52584, "", "sshd-session: yantrik@notty"),
        ];
        assert_eq!(choose(chain).name(), "yos");

        // The same when the daemon's binary is readable: a rewritten title still says which
        // session, never which program.
        let readable = vec![
            facts(52590, "/usr/bin/python3.11", "python3 yos notify Deploy check"),
            facts(52589, "/usr/bin/bash", "bash"),
            facts(52584, "/usr/lib/openssh/sshd-session", "sshd-session: yantrik@notty"),
        ];
        assert_eq!(choose(readable).name(), "yos");
    }

    #[test]
    fn a_daemons_rewritten_title_is_a_name_without_its_punctuation() {
        // When there is nothing better than the title itself — the daemon's process is the
        // peer, or the peer was given nothing to run — the name is the title cut at its
        // first punctuation: `sshd-session`, never `sshd-session:`.
        assert_eq!(
            choose(vec![facts(52584, "", "sshd-session: yantrik@notty")]).name(),
            "sshd-session"
        );
        assert_eq!(choose(vec![facts(300, "", "postgres: writer process")]).name(), "postgres");
        // A peer that is a bare shell is not a program name either, so the title still wins
        // there — cut, as always.
        let bare_peer = vec![
            facts(52589, "/usr/bin/bash", "bash"),
            facts(52584, "", "sshd-session: yantrik@notty"),
        ];
        assert_eq!(choose(bare_peer).name(), "sshd-session");
    }

    // ── The walk itself ──

    #[cfg(target_os = "linux")]
    #[test]
    fn the_walk_reads_this_very_process() {
        // The one test that touches the real /proc. It is about the plumbing being connected —
        // the judgement above is tested against fixtures — so it asserts only what cannot be
        // wrong: this process is in its own chain, the walk is bounded, and it terminates.
        let chain = walk(std::process::id() as i32);
        assert!(!chain.is_empty(), "this process is readable in /proc");
        assert_eq!(chain[0].pid, std::process::id() as i32);
        assert!(chain[0].started > 0, "a start time is what makes a pid unambiguous");
        assert!(chain.len() <= MAX_ANCESTORS, "the walk is bounded: {}", chain.len());

        let pids: std::collections::HashSet<i32> = chain.iter().map(|f| f.pid).collect();
        assert_eq!(pids.len(), chain.len(), "a pid must not appear twice: {chain:?}");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_pid_that_is_not_there_is_not_an_error() {
        // Every /proc read is allowed to fail, and the commonest one does: the direct peer is
        // `yos`, which has usually exited before anybody looks at the card.
        assert_eq!(resolve(Some(0)).line(), UNIDENTIFIED);
        // A pid above the maximum cannot exist, so this is the "process vanished" path exactly.
        let never = walk(i32::MAX);
        assert!(never.is_empty(), "{never:?}");
    }
}
