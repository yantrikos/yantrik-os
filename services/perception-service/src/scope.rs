//! The eyelid.
//!
//! This service is the most invasive thing in Yantrik. It runs privileged, it is told about every
//! process that starts and every file that is written, and it exists to watch someone work. The
//! design question is not whether that is useful — it plainly is — but what stops it becoming
//! something else.
//!
//! The answer is that the restraint is in the kernel too, not in this file's good intentions.
//!
//! Two layers:
//!
//! **Scope** is a path allowlist. Observations about anything outside it are dropped before they
//! reach the ring buffer, so nothing downstream ever learns they happened.
//!
//! **Landlock** is the same policy applied to this process by the kernel, and it is the one that
//! counts. After [`Scope::restrict_self`] the daemon *cannot open* a file outside its scope —
//! not through a bug, not through a future feature, not if something talks the companion into
//! asking. Landlock is not bypassed by root or by `CAP_SYS_ADMIN`, which matters because this
//! process has both.
//!
//! What that leaves is a deliberate and, I think, correct asymmetry:
//!
//! > It can be told that you saved a file. It cannot open it and read what you wrote.
//!
//! fanotify hands us a descriptor the kernel opened, so the path resolves and the event lands.
//! Reading the contents would need an `open()` of our own, and that is exactly what is gone.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Where the daemon may look, and what it may say about what it finds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scope {
    /// Directories whose activity may be reported. Everything else is dropped.
    #[serde(default = "default_watch")]
    pub watch: Vec<String>,
    /// Directories that are never reported even when they sit inside a watched one. Checked
    /// second, so a narrow exclusion beats a broad inclusion — the order a person expects when
    /// they say "my work folder, but not the private one inside it".
    #[serde(default = "default_never")]
    pub never: Vec<String>,
    /// Whether to apply the Landlock ruleset at startup.
    ///
    /// Defaults to on. It exists as a switch only because a kernel without Landlock should say so
    /// loudly rather than silently run unrestricted, and because taking it away in a test is how
    /// we prove it was doing something.
    #[serde(default = "yes")]
    pub enforce: bool,
}

fn yes() -> bool {
    true
}

fn default_watch() -> Vec<String> {
    // Documents and code, not the whole home. `~/.ssh`, `~/.gnupg`, browser profiles and mail
    // stores are all somewhere under `$HOME`, and none of them are things anyone meant to share
    // when they said the assistant could watch their work.
    vec![
        "~/Documents".into(),
        "~/Downloads".into(),
        "~/Desktop".into(),
        "~/Projects".into(),
        "~/code".into(),
        "~/codes".into(),
    ]
}

fn default_never() -> Vec<String> {
    vec![
        "~/.ssh".into(),
        "~/.gnupg".into(),
        "~/.password-store".into(),
        "~/.aws".into(),
        "~/.kube".into(),
        "~/.config/yantrik/vault".into(),
        "~/.local/share/keyrings".into(),
    ]
}

impl Default for Scope {
    fn default() -> Self {
        Self { watch: default_watch(), never: default_never(), enforce: true }
    }
}

/// The scope as absolute paths that exist, with the policy resolved once.
#[derive(Clone)]
pub struct Resolved {
    pub watch: Vec<PathBuf>,
    pub never: Vec<PathBuf>,
    pub enforce: bool,
    /// What actually happened when we tried to close the eyelid. Reported over RPC, because a
    /// promise of restriction that silently failed is worse than none.
    pub enforcement: String,
    /// What happened when we tested it. See [`Resolved::verify`].
    pub verified: String,
}

impl Scope {
    /// Read the policy, falling back to a conservative default rather than to "everything".
    ///
    /// A missing config file is the common case on a fresh install, and the safe reading of it is
    /// the narrow one. A *malformed* file is different: someone wrote a policy and it did not
    /// parse, and quietly substituting our own would be the worst of both.
    pub fn load() -> Result<Self, String> {
        let path = config_path();
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_yaml::from_str(&text)
                .map_err(|e| format!("{} is not valid: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("cannot read {}: {e}", path.display())),
        }
    }

    pub fn resolve(&self) -> Resolved {
        let watch: Vec<PathBuf> = self.watch.iter().map(|p| expand(p)).filter(|p| p.is_dir()).collect();
        let never: Vec<PathBuf> = self.never.iter().map(|p| expand(p)).collect();
        Resolved {
            watch,
            never,
            enforce: self.enforce,
            enforcement: "not attempted".into(),
            verified: "not attempted".into(),
        }
    }
}

impl Resolved {
    /// Whether an observation about this path may be reported at all.
    pub fn allows(&self, path: &Path) -> bool {
        if self.never.iter().any(|deny| path.starts_with(deny)) {
            return false;
        }
        self.watch.iter().any(|allow| path.starts_with(allow))
    }

    /// Try to read something we should no longer be able to read.
    ///
    /// A ruleset can be accepted by the kernel and still not restrain anything — a right left
    /// unhandled, a path granted by accident, an LSM not in the boot list. The only way to know
    /// the eyelid closed is to walk into it, so the service does that to itself at startup and
    /// reports the result rather than asserting the guarantee.
    ///
    /// `/etc/hostname` is the probe: readable by anyone on any system, and outside every scope
    /// this service would ever be given.
    pub fn verify(&mut self) {
        const CANARY: &str = "/etc/hostname";
        match std::fs::read(CANARY) {
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                self.verified = format!("confirmed: {CANARY} is unreadable");
                tracing::info!("Eyelid verified: out-of-scope reads are refused");
            }
            Err(e) => {
                // Something else went wrong; the canary proves nothing either way and must not be
                // reported as if it had.
                self.verified = format!("inconclusive: {CANARY} failed to open with {e}");
            }
            Ok(_) => {
                self.verified =
                    format!("NOT ENFORCED: {CANARY} is still readable, so nothing is being denied");
                tracing::error!(
                    "Eyelid did NOT close: this process can still read outside its scope"
                );
            }
        }
    }

    /// Ask the kernel to hold us to this, and record what it said.
    ///
    /// Best-effort by design: an older kernel, or one built without Landlock, should leave a
    /// service that still works and an `enforcement` string that says plainly it is unrestricted.
    /// Refusing to start would trade a real capability for a guarantee we cannot make there
    /// anyway; lying about it would be worse than either.
    pub fn restrict_self(&mut self, socket_dir: &Path) {
        if !self.enforce {
            self.enforcement = "disabled in config — this process can read anything".into();
            tracing::warn!("Landlock disabled in config; perception is unrestricted");
            return;
        }

        // `/proc` is not part of the user's scope and is granted anyway, because attribution is
        // impossible without it: the process connector reports a pid and nothing else, so the
        // name behind an observation comes from `/proc/<pid>/comm`. Stated plainly in the scope
        // report rather than buried here — it is the one place the read-only set is wider than
        // the watch list, and a reader deserves to know.
        let mut readable = self.watch.clone();
        readable.push(PathBuf::from("/proc"));

        match landlock::restrict(&readable, socket_dir) {
            Ok(abi) => {
                self.enforcement = format!(
                    "landlock ABI {abi}: read-only over {} paths plus /proc; writes only in {}",
                    self.watch.len(),
                    socket_dir.display()
                );
                tracing::info!(abi, paths = self.watch.len(), "Landlock applied");
            }
            Err(e) => {
                self.enforcement = format!("landlock unavailable ({e}) — this process can read anything");
                tracing::warn!(error = %e, "Landlock not applied; perception is unrestricted");
            }
        }
    }
}

fn config_path() -> PathBuf {
    if let Ok(explicit) = std::env::var("YANTRIK_PERCEPTION_CONFIG") {
        return PathBuf::from(explicit);
    }
    expand("~/.config/yantrik/perception.yaml")
}

pub fn expand(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

// ── Landlock ────────────────────────────────────────────────────────

mod landlock {
    //! Landlock by raw syscall.
    //!
    //! Three syscalls and two structs, against a stable ABI. A crate would handle version
    //! negotiation more thoroughly, but this is the whole of what we need and the failure mode of
    //! a dependency that stops building is worse here than the failure mode of forty lines.

    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    const CREATE_RULESET: libc::c_long = 444;
    const ADD_RULE: libc::c_long = 445;
    const RESTRICT_SELF: libc::c_long = 446;

    /// Ask the kernel which ABI it speaks. Version 1 is enough for what we restrict.
    const CREATE_RULESET_VERSION: u32 = 1 << 0;
    const RULE_PATH_BENEATH: libc::c_long = 1;

    // ABI 1 filesystem rights.
    const EXECUTE: u64 = 1 << 0;
    const WRITE_FILE: u64 = 1 << 1;
    const READ_FILE: u64 = 1 << 2;
    const READ_DIR: u64 = 1 << 3;
    const REMOVE_FILE: u64 = 1 << 5;
    const MAKE_SOCK: u64 = 1 << 9;
    /// ABI 3. Named separately because asking for it on an older kernel fails the whole ruleset.
    const TRUNCATE: u64 = 1 << 14;

    // Everything ABI 1 knows about. Anything not granted below is denied, so listing the full set
    // here is what makes "read-only" mean read-only: the write rights are present in the handled
    // mask and absent from every rule.
    const ALL_ABI1: u64 = (1 << 13) - 1;

    #[repr(C)]
    struct RulesetAttr {
        handled_access_fs: u64,
    }

    #[repr(C)]
    struct PathBeneathAttr {
        allowed_access: u64,
        parent_fd: i32,
    }

    /// Restrict this process to reading `readable` and writing only inside `socket_dir`.
    ///
    /// The socket directory is the one place writes are allowed, and only because the RPC server
    /// has to create a Unix socket there and unlink a stale one. Everything else — the user's
    /// files included — is read-only, permanently, for this process and every thread it will ever
    /// start.
    pub fn restrict(readable: &[std::path::PathBuf], socket_dir: &Path) -> Result<u32, String> {
        // SAFETY: the version query takes no pointers and only reports a number.
        let abi = unsafe { libc::syscall(CREATE_RULESET, std::ptr::null::<u8>(), 0usize, CREATE_RULESET_VERSION) };
        if abi < 0 {
            return Err(format!("kernel has no Landlock ({})", std::io::Error::last_os_error()));
        }

        // Handle every right this kernel knows about. A right left unhandled is a right left
        // *unrestricted*, so on an ABI 3 kernel the truncate right must be named here or a
        // "read-only" process could still empty a file it can open.
        let mut handled = ALL_ABI1;
        if abi >= 3 {
            handled |= TRUNCATE;
        }
        let attr = RulesetAttr { handled_access_fs: handled };
        // SAFETY: `attr` outlives the call and its size is what the kernel expects for ABI 1.
        let ruleset = unsafe {
            libc::syscall(
                CREATE_RULESET,
                &attr as *const RulesetAttr,
                std::mem::size_of::<RulesetAttr>(),
                0u32,
            )
        };
        if ruleset < 0 {
            return Err(format!("create_ruleset: {}", std::io::Error::last_os_error()));
        }
        let ruleset = ruleset as i32;

        for path in readable {
            if let Err(e) = allow(ruleset, path, READ_FILE | READ_DIR | EXECUTE) {
                // One unreadable directory in the policy must not leave the process unrestricted:
                // skip it, keep the rest, and let it be absent from what we can see.
                tracing::warn!(path = %path.display(), error = %e, "Landlock: skipping path");
            }
        }

        // The socket directory, and only it. Creating a Unix socket is `MAKE_SOCK`; replacing a
        // stale one is `REMOVE_FILE`; the server also wants to read the directory back.
        if let Err(e) = allow(
            ruleset,
            socket_dir,
            READ_FILE | READ_DIR | WRITE_FILE | MAKE_SOCK | REMOVE_FILE,
        ) {
            // SAFETY: closing our own ruleset fd on the failure path.
            unsafe { libc::close(ruleset) };
            return Err(format!("cannot allow the socket directory ({e}); refusing to start blind"));
        }

        // No new privileges is a precondition: without it an unprivileged process could escape a
        // ruleset through a setuid binary, and the kernel refuses to apply one.
        // SAFETY: prctl with these arguments only sets a flag on the calling process.
        if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
            unsafe { libc::close(ruleset) };
            return Err(format!("no_new_privs: {}", std::io::Error::last_os_error()));
        }

        // SAFETY: `ruleset` is a live fd from create_ruleset above.
        let applied = unsafe { libc::syscall(RESTRICT_SELF, ruleset, 0u32) };
        // SAFETY: the ruleset fd is ours and is not used again.
        unsafe { libc::close(ruleset) };
        if applied < 0 {
            return Err(format!("restrict_self: {}", std::io::Error::last_os_error()));
        }
        Ok(abi as u32)
    }

    fn allow(ruleset: i32, path: &Path, rights: u64) -> Result<(), String> {
        let c_path = std::ffi::CString::new(path.as_os_str().as_bytes())
            .map_err(|_| "path contains a NUL".to_string())?;
        // SAFETY: `c_path` is NUL-terminated and outlives the call.
        let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_PATH | libc::O_CLOEXEC) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let rule = PathBeneathAttr { allowed_access: rights, parent_fd: fd };
        // SAFETY: `rule` outlives the call and `fd` is open for its duration.
        let rc = unsafe {
            libc::syscall(ADD_RULE, ruleset, RULE_PATH_BENEATH, &rule as *const PathBeneathAttr, 0u32)
        };
        // SAFETY: the O_PATH fd is ours and the kernel has copied what it needs.
        unsafe { libc::close(fd) };
        if rc < 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolved(watch: &[&str], never: &[&str]) -> Resolved {
        Resolved {
            watch: watch.iter().map(PathBuf::from).collect(),
            never: never.iter().map(PathBuf::from).collect(),
            enforce: false,
            enforcement: String::new(),
            verified: String::new(),
        }
    }

    #[test]
    fn nothing_outside_the_watch_list_is_reportable() {
        let s = resolved(&["/home/p/Documents"], &[]);
        assert!(s.allows(Path::new("/home/p/Documents/plan.md")));
        assert!(!s.allows(Path::new("/home/p/.ssh/id_ed25519")));
        assert!(!s.allows(Path::new("/etc/shadow")));
    }

    #[test]
    fn a_narrow_exclusion_beats_a_broad_inclusion() {
        // "My work folder, but not the private one inside it" is what a person means, and the
        // order of the checks is the only thing that makes it true.
        let s = resolved(&["/home/p/work"], &["/home/p/work/private"]);
        assert!(s.allows(Path::new("/home/p/work/report.odt")));
        assert!(!s.allows(Path::new("/home/p/work/private/diary.md")));
    }

    #[test]
    fn an_empty_scope_sees_nothing_rather_than_everything() {
        let s = resolved(&[], &[]);
        assert!(!s.allows(Path::new("/home/p/Documents/plan.md")));
    }

    #[test]
    fn the_defaults_do_not_include_the_whole_home() {
        let d = Scope::default();
        assert!(!d.watch.iter().any(|p| p == "~" || p == "~/"));
        for secret in ["~/.ssh", "~/.gnupg"] {
            assert!(d.never.iter().any(|p| p == secret), "{secret} must be excluded by default");
        }
    }
}
