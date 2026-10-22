//! Problem records: what a program writes down when it goes wrong, so a person can decide to
//! send it.
//!
//! Nothing in this module sends anything. That is the design, not an omission: the README
//! promises that the OS itself sends nothing anywhere except what the person asked for, and a
//! crash reporter that posted on its own would make that false. So a panic, a crash or a failure
//! becomes a small JSON file under `~/.local/share/yantrik/problems/`, already scrubbed of the
//! user's home, name and anything shaped like a credential — and the shell's "Report a problem"
//! screen shows the person that exact file before they press Send. See
//! `design/problem-reports-2026-09-23.md`.
//!
//! The record is scrubbed *before* it is written, so nothing in the OS ever holds an unscrubbed
//! copy, and a person who opens the file sees precisely what a report would carry.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// How many records are kept. The oldest go first; fifty crashes is a pattern, not an archive.
pub const KEEP: usize = 50;

/// How many of the process's own most recent log lines a record carries.
pub const LOG_TAIL: usize = 40;

/// How many bytes of backtrace text a record carries. A frame runs to about a hundred and fifty
/// bytes, so this holds close to a hundred frames — deeper than any stack this OS grows — and
/// the record stays a small file, well inside the intake's body limit.
pub const BACKTRACE_MAX: usize = 16 * 1024;

/// The folder this process files into, when it runs somewhere other than the session's own
/// data folder: an app under `$YANTRIK_HOME` (a second profile, a scratch run) files into that
/// home, never into the person's `problems/`.
static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Files this process's records into `dir` from now on, panics included. The first call wins:
/// a process has one home. Says whether this call was the one taken.
pub fn set_dir(dir: PathBuf) -> bool {
    DIR.set(dir).is_ok()
}

/// Where records live: the folder [`set_dir`] gave, else `$XDG_DATA_HOME/yantrik/problems`, or
/// `~/.local/share/yantrik/problems`.
pub fn dir() -> PathBuf {
    if let Some(dir) = DIR.get() {
        return dir.clone();
    }
    default_dir()
}

/// The session's own folder, whatever [`set_dir`] said.
pub fn default_dir() -> PathBuf {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(".local").join("share"));
    data.join("yantrik").join("problems")
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

/// One thing that went wrong. Every string field is stored scrubbed.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Problem {
    /// `panic` (a Rust panic in this process), `crash` (a child exited badly, written by the
    /// launcher), or `failure` (something that did not crash and still went wrong).
    pub kind: String,
    /// The binary, e.g. `yantrik-studio`.
    pub program: String,
    pub version: String,
    pub git: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    /// The stack at a panic, scrubbed and cut to [`BACKTRACE_MAX`]. Absent when the platform
    /// cannot capture one, and on `crash` and `failure` records, which have no panic to walk.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backtrace: Option<String>,
    pub log_tail: Vec<String>,
    pub machine: serde_json::Value,
    /// Unix seconds.
    pub when: u64,
}

/// A fixed-size ring of this process's most recent log lines, filled by [`note_log_line`].
static RECENT_LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Remember one line this process logged, for the `log_tail` of a record written later.
///
/// Called by the tracing layer the runtime installs; an app never calls it directly.
pub fn note_log_line(line: &str) {
    if let Ok(mut ring) = RECENT_LOG.lock() {
        if ring.len() >= LOG_TAIL {
            ring.remove(0);
        }
        ring.push(line.to_string());
    }
}

fn recent_log() -> Vec<String> {
    RECENT_LOG.lock().map(|r| r.clone()).unwrap_or_default()
}

/// Remove what would identify the person or leak a secret.
///
/// The home directory and `/home/<user>` become `~`; the username as a word becomes `<user>`;
/// anything shaped like a credential becomes `<redacted>`. Conservative on purpose: a scrubbed
/// path that is slightly less useful is a smaller cost than a username in a public issue.
pub fn scrub(text: &str) -> String {
    let mut out = text.to_string();
    let home_dir = home();
    let home_str = home_dir.to_string_lossy().to_string();
    let user = std::env::var("USER")
        .ok()
        .filter(|u| !u.is_empty())
        .or_else(|| home_dir.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_default();

    if home_str.len() > 1 {
        out = out.replace(&home_str, "~");
    }
    if !user.is_empty() && user != "root" {
        out = out.replace(&format!("/home/{user}"), "~");
        out = replace_word(&out, &user, "<user>");
    }
    out = redact_secrets(&out);
    out
}

/// Replace `word` where it stands alone — bounded by non-identifier characters — not inside a
/// longer word, so a user called `al` does not turn `total` into `tot<user>`.
fn replace_word(text: &str, word: &str, with: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'-';
    while i < bytes.len() {
        if text[i..].starts_with(word)
            && (i == 0 || !ident(bytes[i - 1]))
            && (i + word.len() == bytes.len() || !ident(bytes[i + word.len()]))
        {
            out.push_str(with);
            i += word.len();
        } else {
            let ch = text[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

/// The shapes a credential takes in a log line. Each prefix is followed by the token it names,
/// which is replaced up to the next whitespace or quote.
fn redact_secrets(text: &str) -> String {
    const PREFIXES: &[&str] = &[
        "sk-", "ghp_", "gho_", "github_pat_", "xoxb-", "xoxp-", "AKIA", "Bearer ", "bearer ",
        "token=", "TOKEN=", "api_key=", "API_KEY=", "apikey=", "key=", "KEY=", "password=",
        "PASSWORD=", "secret=", "SECRET=",
    ];
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    'scan: while !rest.is_empty() {
        for prefix in PREFIXES {
            if rest.starts_with(prefix) {
                // Keep the prefix that names the thing, drop the thing.
                let after = &rest[prefix.len()..];
                let end = after
                    .find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == ',')
                    .unwrap_or(after.len());
                out.push_str(prefix);
                out.push_str("<redacted>");
                rest = &after[end..];
                continue 'scan;
            }
        }
        let ch = rest.chars().next().unwrap();
        out.push(ch);
        rest = &rest[ch.len_utf8()..];
    }
    out
}

/// What kind of machine this is — enough to tell a GPU box from a headless VM, and nothing that
/// identifies it. Cores, RAM in whole GB, whether any DRM device exists, whether a hypervisor is
/// present, and the kernel release.
pub fn machine() -> serde_json::Value {
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0);
    let ram_gb = fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|m| {
            m.lines()
                .find(|l| l.starts_with("MemTotal:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|kb| kb.parse::<u64>().ok())
        })
        .map(|kb| kb / 1024 / 1024)
        .unwrap_or(0);
    let gpu = fs::read_dir("/sys/class/drm")
        .map(|d| d.flatten().any(|e| e.file_name().to_string_lossy().starts_with("card")))
        .unwrap_or(false);
    let virtualised = fs::read_to_string("/sys/class/dmi/id/product_name")
        .map(|p| {
            let p = p.to_lowercase();
            ["qemu", "kvm", "virtual", "vmware", "virtualbox", "hyper-v", "bochs"]
                .iter()
                .any(|v| p.contains(v))
        })
        .unwrap_or(false)
        || Path::new("/proc/xen").exists();
    let kernel = fs::read_to_string("/proc/sys/kernel/osrelease")
        .map(|k| k.trim().to_string())
        .unwrap_or_default();
    serde_json::json!({
        "cores": cores,
        "ram_gb": ram_gb,
        "gpu": gpu,
        "virtualised": virtualised,
        "kernel": kernel,
    })
}

/// The commit this build was cut from: BUILD's `git=` line on an installed machine, else the
/// `-g<hash>` that `git describe` puts in the version string, else "unknown".
fn git_of_build() -> String {
    if let Ok(text) = fs::read_to_string(yantrik_version::build_marker_path()) {
        if let Some(g) = text.lines().find_map(|l| l.strip_prefix("git=")) {
            let g = g.trim();
            if !g.is_empty() {
                return g.to_string();
            }
        }
    }
    let v = yantrik_version::version();
    if let Some(i) = v.rfind("-g") {
        let h = v[i + 2..].trim_end_matches("-dirty");
        if h.len() >= 7 && h.chars().all(|c| c.is_ascii_hexdigit()) {
            return h.to_string();
        }
    }
    "unknown".to_string()
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Cut backtrace text down to at most [`BACKTRACE_MAX`] bytes, on a character boundary, with
/// the cut itself written into the text so nobody takes the last frame shown for the deepest.
fn cap_backtrace(text: &str) -> String {
    const CUT: &str = "\n[backtrace truncated]";
    if text.len() <= BACKTRACE_MAX {
        return text.to_string();
    }
    let mut end = BACKTRACE_MAX - CUT.len();
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{CUT}", &text[..end])
}

/// Build a record with every text field scrubbed and the machine described.
pub fn problem(
    kind: &str,
    program: &str,
    message: &str,
    location: Option<&str>,
    backtrace: Option<&str>,
) -> Problem {
    Problem {
        kind: kind.to_string(),
        program: program.to_string(),
        version: yantrik_version::version().to_string(),
        git: git_of_build(),
        message: scrub(message),
        location: location.map(scrub),
        // Capped after scrubbing, so what is stored is what fits: scrubbing can make a line
        // longer (a short username becomes `<user>`), and the bound must hold on the record.
        backtrace: backtrace.map(|b| cap_backtrace(&scrub(b))),
        log_tail: recent_log().iter().map(|l| scrub(l)).collect(),
        machine: machine(),
        when: now(),
    }
}

/// Write a record into `dir`, pruning to [`KEEP`]. Returns the path, or `None` if the directory
/// could not be written — a problem record must never itself be a reason to fail.
pub fn write_to(dir: &Path, record: &Problem) -> Option<PathBuf> {
    fs::create_dir_all(dir).ok()?;
    let name = format!("{}-{}.json", record.when, safe_name(&record.program));
    let path = dir.join(name);
    let json = serde_json::to_vec_pretty(record).ok()?;
    let mut f = fs::File::create(&path).ok()?;
    f.write_all(&json).ok()?;
    prune(dir);
    Some(path)
}

/// Write a record into the default directory.
pub fn write(record: &Problem) -> Option<PathBuf> {
    write_to(&dir(), record)
}

/// Record that something went wrong without crashing — a `describe` that timed out, a job that
/// failed, an action the app itself could not complete.
pub fn record_failure(program: &str, what: &str, detail: &str) -> Option<PathBuf> {
    record_failure_in(&dir(), program, what, detail)
}

/// [`record_failure`] into `dir`, so a test can read back what was filed.
pub fn record_failure_in(dir: &Path, program: &str, what: &str, detail: &str) -> Option<PathBuf> {
    let record = problem("failure", program, &format!("{what}: {detail}"), None, None);
    write_to(dir, &record)
}

fn safe_name(program: &str) -> String {
    program
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .take(40)
        .collect()
}

fn prune(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    if files.len() <= KEEP {
        return;
    }
    // Names start with the unix time, so lexical order is time order.
    files.sort();
    for old in files.iter().take(files.len() - KEEP) {
        let _ = fs::remove_file(old);
    }
}

/// Every record on disk, oldest first.
pub fn list() -> Vec<(PathBuf, Problem)> {
    list_in(&dir())
}

pub fn list_in(dir: &Path) -> Vec<(PathBuf, Problem)> {
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<(PathBuf, Problem)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter_map(|p| {
            let text = fs::read_to_string(&p).ok()?;
            let record: Problem = serde_json::from_str(&text).ok()?;
            Some((p, record))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

thread_local! {
    // Set while a panic on this thread is somebody else's to file (a job the runtime catches).
    static WITHOUT_RECORDS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Runs `f` with this thread's panics kept out of the problem records: for work the caller
/// catches and files once itself. The hook still runs, so the trace reaches stderr. The flag is
/// restored however `f` ends, so a panic in it leaves the thread's next work recorded as usual.
pub fn without_panic_records<R>(f: impl FnOnce() -> R) -> R {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            WITHOUT_RECORDS.with(|w| w.set(self.0));
        }
    }
    let _restore = Restore(WITHOUT_RECORDS.with(|w| w.replace(true)));
    f()
}

/// Install a panic hook that writes a `panic` record and then lets the previous hook print the
/// panic as it always did. Idempotent enough: installing twice chains twice, which only means two
/// files for one panic, so the runtime calls it once from `init_tracing`.
pub fn install_panic_hook(program: &str) {
    let program = program.to_string();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if WITHOUT_RECORDS.with(|w| w.get()) {
            previous(info);
            return;
        }
        let message = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "panic with a non-string payload".to_string()
        };
        let location = info.location().map(|l| format!("{}:{}", l.file(), l.line()));
        // Force the capture, whatever the environment says. `Backtrace::capture()` only records
        // when RUST_BACKTRACE is set, and the session does not set it: the renderer panic of
        // #247 left a record with a location, an empty log tail and no stack, so the element
        // being drawn was never known. Capturing walks this process's own frames and resolves
        // symbols in-process; it waits on nothing outside, and `problem()` caps the text.
        let backtrace = std::backtrace::Backtrace::force_capture();
        let backtrace = match backtrace.status() {
            std::backtrace::BacktraceStatus::Captured => Some(backtrace.to_string()),
            _ => None,
        };
        let record = problem("panic", &program, &message, location.as_deref(), backtrace.as_deref());
        if let Some(path) = write(&record) {
            eprintln!("[yantrik] problem record written: {}", path.display());
        }
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory of this test's own. Tests run in parallel, and a name made from the pid and
    /// the second collided the first time two of them started in the same second — they shared
    /// a directory and counted each other's files. A counter cannot collide.
    fn tmp(name: &str) -> PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let d = std::env::temp_dir().join(format!(
            "yantrik-problems-{}-{}-{}",
            std::process::id(),
            name,
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn a_home_path_and_a_username_do_not_survive_scrubbing() {
        // Whatever the machine's real values are, the scrubber must remove them.
        let home_dir = home().to_string_lossy().to_string();
        // The same fallback `scrub` uses, or a bare container (no USER, HOME=/root) sees the
        // scrubber remove `root` while the test looks for `somebody`.
        let user = std::env::var("USER")
            .ok()
            .filter(|u| !u.is_empty())
            .or_else(|| home().file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_default();
        let text = format!("failed to open {home_dir}/Pictures/x.png as {user} in /home/{user}/y");
        let out = scrub(&text);
        if home_dir.len() > 1 {
            assert!(!out.contains(&home_dir), "{out}");
        }
        if !user.is_empty() && user != "root" {
            assert!(!out.contains(&format!("/home/{user}")), "{out}");
            assert!(!out.contains(&format!(" as {user} ")), "{out}");
        }
        assert!(out.contains("~/Pictures/x.png"), "{out}");
    }

    #[test]
    fn a_username_is_replaced_only_as_a_whole_word() {
        assert_eq!(replace_word("total al altogether al.", "al", "<u>"), "total <u> altogether <u>.");
        assert_eq!(replace_word("al", "al", "<u>"), "<u>");
        assert_eq!(replace_word("al-b al_c", "al", "<u>"), "al-b al_c");
    }

    #[test]
    fn things_shaped_like_credentials_are_redacted() {
        let out = redact_secrets(
            "using sk-abc123DEF token=xyz, Authorization: Bearer eyJhbGci ghp_0123456789 done",
        );
        assert_eq!(
            out,
            "using sk-<redacted> token=<redacted>, Authorization: Bearer <redacted> ghp_<redacted> done"
        );
        assert_eq!(redact_secrets("nothing secret here"), "nothing secret here");
    }

    #[test]
    fn a_record_is_written_scrubbed_and_read_back() {
        let d = tmp("written");
        let home_dir = home().to_string_lossy().to_string();
        let record = problem(
            "failure",
            "yantrik-test",
            &format!("could not read {home_dir}/secret.txt with key=abc"),
            Some(&format!("{home_dir}/src/main.rs:12")),
            None,
        );
        let path = write_to(&d, &record).expect("written");
        assert!(path.exists());
        let text = fs::read_to_string(&path).unwrap();
        if home_dir.len() > 1 {
            assert!(!text.contains(&home_dir), "the home path reached the disk: {text}");
        }
        assert!(text.contains("key=<redacted>"), "{text}");
        let back = list_in(&d);
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].1, record);
        assert_eq!(back[0].1.kind, "failure");
        assert_eq!(back[0].1.program, "yantrik-test");
        assert!(back[0].1.machine["cores"].as_u64().unwrap_or(0) >= 1);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn only_the_newest_fifty_are_kept() {
        let d = tmp("fifty");
        for i in 0..(KEEP + 7) {
            let mut r = problem("failure", "yantrik-test", "x", None, None);
            r.when = 1_000_000 + i as u64;
            write_to(&d, &r).unwrap();
        }
        let kept = list_in(&d);
        assert_eq!(kept.len(), KEEP);
        assert_eq!(kept[0].1.when, 1_000_000 + 7, "the oldest seven went");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_panic_on_another_thread_writes_a_record() {
        // The hook installed here writes into a directory of this test's choosing, so the test
        // touches no shared environment. Thread panics run the hook like any other.
        let d = tmp("panic");
        let dir = d.clone();
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let message = info
                .payload()
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .unwrap_or_default();
            let location = info.location().map(|l| format!("{}:{}", l.file(), l.line()));
            let record = problem("panic", "yantrik-test", &message, location.as_deref(), None);
            write_to(&dir, &record);
        }));
        let joined = std::thread::spawn(|| panic!("deliberate")).join();
        std::panic::set_hook(previous);
        assert!(joined.is_err());
        let records = list_in(&d);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].1.kind, "panic");
        assert_eq!(records[0].1.message, "deliberate");
        assert!(records[0].1.location.as_deref().unwrap_or("").contains("problems.rs"));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_record_carries_a_forced_backtrace_and_never_more_than_the_cap() {
        // What the hook does at a real panic, minus the panic: RUST_BACKTRACE is not set here
        // either, so this is exactly the case where `capture()` used to come back disabled and
        // the record went out with no stack at all (#247). Forced, the field must be there.
        let captured = std::backtrace::Backtrace::force_capture().to_string();
        let record = problem("panic", "yantrik-test", "x", None, Some(&captured));
        let back = record.backtrace.expect("the record carries the captured backtrace");
        assert!(back.len() <= BACKTRACE_MAX, "{} bytes", back.len());

        // A stack far deeper than the cap comes back cut to it, on a whole character, and the
        // cut says so, so the last frame shown is not mistaken for the deepest.
        let huge = "  17: some::very::deep::frame\n".repeat(BACKTRACE_MAX);
        let record = problem("panic", "yantrik-test", "x", None, Some(&huge));
        let back = record.backtrace.expect("cut, not dropped");
        assert!(back.len() <= BACKTRACE_MAX, "{} bytes", back.len());
        assert!(back.ends_with("[backtrace truncated]"), "…{}", &back[back.len().min(80)..]);

        // A short stack survives whole, apart from the scrubbing every text field gets.
        let short = "   0: some::frame\n   1: another::frame";
        let record = problem("panic", "yantrik-test", "x", None, Some(short));
        assert_eq!(record.backtrace, Some(scrub(short)));
    }
}
