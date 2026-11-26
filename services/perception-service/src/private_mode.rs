//! Reading Private mode from inside the sandbox (#1415).
//!
//! Every call asks `privacy::is_private()`, which fails closed: a read that is refused counts as
//! the person being private. The Landlock ruleset once left the file out, so from 21 Oct the
//! service believed the person private on every machine where Landlock applies, recorded
//! nothing, and said nothing. This module is what makes that loud: after the sandbox closes the
//! service reads the switch once and logs an error that names the path, and its scope report says
//! live whether it can.

use yantrik_ipc_transport::privacy::{self, Unreadable};

/// What the scope report says for a read of the switch.
pub fn describe(read: &Result<bool, Unreadable>) -> serde_json::Value {
    match read {
        Ok(private) => serde_json::json!({ "readable": true, "private": private }),
        Err(e) => serde_json::json!({
            "readable": false,
            "path": e.path.as_ref().map(|p| p.display().to_string()),
            "error": e.error.to_string(),
            "effect": "perception cannot read Private mode, so it takes the person to be private: \
                       it records nothing and refuses every caller",
        }),
    }
}

/// The report's live entry.
pub fn report() -> serde_json::Value {
    describe(&privacy::read_state())
}

/// Read the switch once, after the sandbox closed; an error names the path and the reason.
/// Returns whether it could be read.
pub fn check_at_start() -> bool {
    let read = privacy::read_state();
    match &read {
        Ok(private) => tracing::info!(private, "Private mode is readable from inside the sandbox"),
        Err(e) => tracing::error!(
            path = ?e.path,
            error = %e.error,
            "perception cannot read Private mode from inside its sandbox: it will take the person to be \
             private, record nothing and refuse every caller"
        ),
    }
    read.is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scope::Scope;
    use std::io::{BufRead, Write};
    use std::process::{Command, Stdio};

    /// The tests below own `HOME`, one at a time.
    static HOME: std::sync::Mutex<()> = std::sync::Mutex::new(());

    const CHILD: &str = "YANTRIK_TEST_PRIVATE_CHILD";

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("perception-private-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The sandboxed half: close the eyelid with the default scope, then report what
    /// `is_private()` says each time the parent writes a line. Does nothing unless spawned.
    #[test]
    fn child_in_the_sandbox() {
        if std::env::var_os(CHILD).is_none() {
            return;
        }
        let home = std::path::PathBuf::from(std::env::var("HOME").unwrap());
        let mut scope = Scope::default().resolve();
        scope.restrict_self(&home.join("run"));
        if !scope.enforcement.starts_with("landlock ABI") {
            println!("SKIP {}", scope.enforcement);
            return;
        }
        assert!(std::fs::read(home.join(".config/yantrik/settings.yaml")).is_err(), "sandbox must be closed");
        assert!(check_at_start());
        let stdin = std::io::stdin();
        loop {
            println!("PRIVATE {}", privacy::is_private());
            std::io::stdout().flush().unwrap();
            let mut line = String::new();
            if stdin.lock().read_line(&mut line).unwrap() == 0 {
                return;
            }
        }
    }

    #[test]
    fn the_sandbox_reads_private_mode_through_the_shells_own_writes() {
        let _home = HOME.lock().unwrap_or_else(|e| e.into_inner());
        let home = scratch("sandbox");
        std::fs::create_dir_all(home.join("run")).unwrap();
        std::env::set_var("HOME", &home);
        // The vault is in the config directory, and must stay out of reach.
        std::fs::create_dir_all(home.join(".config/yantrik/vault")).unwrap();
        std::fs::write(home.join(".config/yantrik/settings.yaml"), "x: 1\n").unwrap();
        privacy::migrate_legacy(1).unwrap();
        privacy::publish(false, 1).unwrap();

        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "private_mode::tests::child_in_the_sandbox", "--nocapture", "--test-threads=1"])
            .env(CHILD, "1")
            .env("HOME", &home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut out = std::io::BufReader::new(child.stdout.take().unwrap());
        let mut stdin = child.stdin.take().unwrap();
        let mut next = || -> Option<String> {
            loop {
                let mut line = String::new();
                if out.read_line(&mut line).unwrap() == 0 {
                    return None;
                }
                // libtest prints "test <name> ... " before the test's own first line.
                if let Some(at) = line.find("PRIVATE ").or_else(|| line.find("SKIP ")) {
                    return Some(line[at..].trim().to_string());
                }
            }
        };
        let first = next().expect("the child answered");
        if first.starts_with("SKIP") {
            eprintln!("{first}: no Landlock here, nothing to prove");
            return;
        }
        assert_eq!(first, "PRIVATE false");
        for (flip, expect) in [(true, "PRIVATE true"), (false, "PRIVATE false")] {
            privacy::publish(flip, 2).unwrap(); // the shell's own path: a new file, renamed over
            stdin.write_all(b"\n").unwrap();
            assert_eq!(next().as_deref(), Some(expect));
        }
        drop(stdin);
        assert!(child.wait().unwrap().success());
    }

    #[test]
    fn a_privacy_path_that_cannot_be_read_is_reported_not_swallowed() {
        let _home = HOME.lock().unwrap_or_else(|e| e.into_inner());
        let home = scratch("unreadable");
        std::env::set_var("HOME", &home);
        // A directory where the file should be: a read error that is not "not found".
        std::fs::create_dir_all(privacy::privacy_path().unwrap()).unwrap();
        let read = privacy::read_state();
        assert!(read.is_err());
        assert!(privacy::is_private(), "and the person is taken to be private");
        let said = describe(&read);
        assert_eq!(said["readable"], false);
        assert!(said["path"].as_str().unwrap().ends_with("private-mode/privacy.json"), "{said}");
        assert!(said["effect"].as_str().unwrap().contains("cannot read Private mode"));
        assert!(!check_at_start());
    }

    #[test]
    fn a_readable_switch_is_reported_as_such() {
        let _home = HOME.lock().unwrap_or_else(|e| e.into_inner());
        let home = scratch("readable");
        std::env::set_var("HOME", &home);
        assert_eq!(describe(&privacy::read_state()), serde_json::json!({"readable": true, "private": false}));
    }
}
