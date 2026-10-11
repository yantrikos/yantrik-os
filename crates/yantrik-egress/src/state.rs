//! What the proxy holds, and where it keeps it.
//!
//! `/var/lib/yantrik-egress` (the proxy's own, 0700): `policy.yaml`, the person's rules, written
//! only when the control socket changes them; `seen.json`, the ledger, written every little while
//! when it changed; `private`, which exists while the person's Private mode is on, so a restart
//! in the middle of it stays private until the shell says otherwise.
//!
//! It also reads, never writes, the mind's status file (`EGRESS_MIND_STATUS`,
//! /run/yantrik-mind-egress/mind-egress.json), to say whether the kernel's table has caught up
//! with the policy ([`State::kernel_current`]).

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::ledger::Ledger;
use crate::policy::Policy;

pub struct State {
    pub policy: Policy,
    pub ledger: Ledger,
    pub private: bool,
    /// The mind's status file, which `yantrik-update mind-egress apply` writes once the kernel has
    /// loaded the table made from the policy.
    pub mind_status: PathBuf,
    dir: PathBuf,
}

/// Where the mind's status file is, unless `EGRESS_MIND_STATUS` says.
pub const MIND_STATUS: &str = "/run/yantrik-mind-egress/mind-egress.json";

impl State {
    pub fn load(dir: &Path) -> State {
        State {
            policy: Policy::load(&dir.join("policy.yaml")),
            ledger: Ledger::load(&dir.join("seen.json")),
            private: private_at(dir),
            mind_status: std::env::var_os("EGRESS_MIND_STATUS").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(MIND_STATUS)),
            dir: dir.to_path_buf(),
        }
    }

    pub fn save_policy(&self) -> std::io::Result<()> {
        let text = serde_yaml::to_string(&self.policy).map_err(std::io::Error::other)?;
        write_atomic(&self.dir.join("policy.yaml"), text.as_bytes())
    }

    pub fn save_ledger(&mut self) -> std::io::Result<()> {
        if !self.ledger.changed {
            return Ok(());
        }
        let text = serde_json::to_vec(&self.ledger).map_err(std::io::Error::other)?;
        write_atomic(&self.dir.join("seen.json"), &text)?;
        self.ledger.changed = false;
        Ok(())
    }

    /// Whether the kernel's table follows the policy as it is now: the status file names this
    /// mode and this Private mode, and was made from this policy file or a later one. The proxy
    /// follows a switch at once; the kernel when the policy file's path unit has run `apply`, so
    /// for a moment after a switch this is `false`. `None`: there is no status file to read.
    ///
    /// "Made from" is by the two files' modification times, to the nanosecond: `apply` takes the
    /// policy file's before it reads the policy, and sets the status file's to it once the table
    /// is loaded (`touch -d`), so a status file no older than the policy file was made from it. A
    /// policy written while an apply was loading an older one is newer than that status file —
    /// and has `apply` load again. Not `loaded_at`: it is whole seconds (the status file's
    /// version 3 says so), and the time of the load, not of the policy loaded.
    pub fn kernel_current(&self) -> Option<bool> {
        let status: serde_json::Value = serde_json::from_slice(&std::fs::read(&self.mind_status).ok()?).ok()?;
        status["loaded_at"].as_u64()?;
        let modified = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
        let loaded = modified(&self.mind_status)?;
        let caught_up = modified(&self.dir.join("policy.yaml")).is_none_or(|written| written <= loaded);
        Some(status["mode"].as_str() == Some(self.policy.mode.word()) && status["private"].as_bool() == Some(self.private) && caught_up)
    }

    pub fn set_private(&mut self, on: bool) -> std::io::Result<()> {
        self.private = on;
        let p = self.dir.join("private");
        if on {
            write_atomic(&p, b"on\n")
        } else {
            match std::fs::remove_file(&p) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            }
        }
    }
}

/// Whether the person's Private mode is on, by its marker in `dir`. Can't tell (the directory
/// unreadable): private. Only "it is not there" is off.
pub fn private_at(dir: &Path) -> bool {
    dir.join("private").try_exists().unwrap_or(true)
}

/// A new file beside the old at 0600, renamed over it.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let mut f = options.open(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::Outcome;
    use crate::policy::{Mode, Rule};
    use std::time::Duration;

    #[test]
    fn what_it_holds_survives_a_restart() {
        let d = std::env::temp_dir().join(format!("yantrik-egress-state-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let mut s = State::load(&d);
        assert!(!s.private);
        s.policy.mode = Mode::Enforce;
        s.policy.allow(Rule { host: "api.x.ai".into(), ports: vec![443], http: false, lan: false, why: "the model".into(), seeded: false }).unwrap();
        s.save_policy().unwrap();
        s.ledger.record("api.x.ai", 443, Outcome::Allowed, false, false, "", 5);
        s.save_ledger().unwrap();
        s.set_private(true).unwrap();
        let back = State::load(&d);
        assert_eq!(back.policy, s.policy);
        assert_eq!(back.ledger.list().len(), 1);
        assert!(back.private, "a restart during Private mode stays private");
        s.set_private(false).unwrap();
        assert!(!State::load(&d).private);
    }

    #[test]
    fn the_kernel_has_caught_up_once_the_status_file_says_this_policy() {
        let d = std::env::temp_dir().join(format!("yantrik-egress-kernel-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let mut s = State::load(&d);
        s.mind_status = d.join("mind-egress.json");
        assert_eq!(s.kernel_current(), None, "no status file");
        let status = |mode: &str| {
            std::fs::write(d.join("mind-egress.json"), format!(r#"{{"mode": "{mode}", "private": false, "loaded_at": 1759600000, "version": 3}}"#)).unwrap()
        };
        let at = |file: &str, t: std::time::SystemTime| std::fs::File::options().write(true).open(d.join(file)).unwrap().set_modified(t).unwrap();
        status("audit");
        assert_eq!(s.kernel_current(), Some(true), "audit, and no policy file yet");
        s.policy.mode = Mode::Guarded;
        s.save_policy().unwrap();
        assert_eq!(s.kernel_current(), Some(false), "switched; the kernel still has audit");
        status("guarded");
        let t = std::time::SystemTime::now();
        at("policy.yaml", t);
        at("mind-egress.json", t);
        assert_eq!(s.kernel_current(), Some(true), "made from this policy: apply set its time to the policy's");
        at("policy.yaml", t + Duration::from_nanos(1));
        assert_eq!(s.kernel_current(), Some(false), "a policy written a nanosecond later is not loaded yet");
        at("policy.yaml", t);
        at("mind-egress.json", t + Duration::from_millis(400));
        assert_eq!(s.kernel_current(), Some(true), "an apply that set no time: written 0.4 s after the switch");
        // A rule allowed 0.3 s after that apply, in the same second: not loaded yet.
        at("policy.yaml", t + Duration::from_millis(700));
        assert_eq!(s.kernel_current(), Some(false), "a policy written after the status file, within its second");
        at("mind-egress.json", t + Duration::from_millis(900));
        assert_eq!(s.kernel_current(), Some(true));
        s.private = true;
        assert_eq!(s.kernel_current(), Some(false), "Private mode is not in the kernel yet");
    }
}
