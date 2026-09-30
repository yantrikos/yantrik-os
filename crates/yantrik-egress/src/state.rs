//! What the proxy holds, and where it keeps it.
//!
//! `/var/lib/yantrik-egress` (the proxy's own, 0700): `policy.yaml`, the person's rules, written
//! only when the control socket changes them; `seen.json`, the ledger, written every little while
//! when it changed; `private`, which exists while the person's Private mode is on, so a restart
//! in the middle of it stays private until the shell says otherwise.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::ledger::Ledger;
use crate::policy::Policy;

pub struct State {
    pub policy: Policy,
    pub ledger: Ledger,
    pub private: bool,
    dir: PathBuf,
}

impl State {
    pub fn load(dir: &Path) -> State {
        State {
            policy: Policy::load(&dir.join("policy.yaml")),
            ledger: Ledger::load(&dir.join("seen.json")),
            // Can't tell (the directory unreadable): private. Only "it is not there" is off.
            private: dir.join("private").try_exists().unwrap_or(true),
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

    #[test]
    fn what_it_holds_survives_a_restart() {
        let d = std::env::temp_dir().join(format!("yantrik-egress-state-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let mut s = State::load(&d);
        assert!(!s.private);
        s.policy.mode = Mode::Enforce;
        s.policy.allow(Rule { host: "api.x.ai".into(), ports: vec![443], http: false, lan: false, why: "the model".into() }).unwrap();
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
}
