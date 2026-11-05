//! Whether `claude_think` is provably an answer and nothing else on this machine, and the private
//! directory it runs from (#1063 review, M4).
//!
//! The arguments say "no tools", but a CLI decides what they mean, and it changes: the earlier
//! deny-list left about thirty tools on 2.1.316, and its `--no-input` stopped the call outright.
//! So the CLI installed here is asked once, with the exact arguments a call uses, what it would
//! run with: its first stream event lists its tools and MCP servers, and is read before any
//! answer, after which the probe is stopped. Only "none and none" is contained; anything else, or
//! no answer, and each call is put to the person like any other send.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// `Ok` when this machine's `claude` runs a think call with no tools and no MCP servers; `Err` is
/// why not, in words for the card.
pub(super) fn contained() -> Result<(), String> {
    static VERDICT: OnceLock<Result<(), String>> = OnceLock::new();
    VERDICT.get_or_init(probe).clone()
}

fn probe() -> Result<(), String> {
    let dir = PrivateDir::new()?;
    let mut args = super::think_args(".");
    let at = args.iter().position(|a| a == "--").unwrap_or(args.len());
    args.splice(at..at, ["--output-format".to_string(), "stream-json".to_string(), "--verbose".to_string()]);
    let mut child = Command::new("claude")
        .args(&args)
        .current_dir(dir.path())
        .env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("the claude CLI could not be started to check it runs with no tools ({e})"))?;
    let out = child.stdout.take().expect("piped");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                return;
            }
        }
    });
    let deadline = Instant::now() + Duration::from_secs(30);
    let verdict = loop {
        let Some(left) = deadline.checked_duration_since(Instant::now()) else { break Err("it did not say".to_string()) };
        let Ok(line) = rx.recv_timeout(left) else { break Err("it did not say".to_string()) };
        if let Some(found) = init_verdict(&line) {
            break found;
        }
    };
    let _ = child.kill();
    let _ = child.wait();
    verdict.map_err(|why| format!("this machine's claude CLI was not shown to run with no tools and no MCP servers: {why}"))
}

/// The verdict in one stream line, if it is the `init` event.
pub(super) fn init_verdict(line: &str) -> Option<Result<(), String>> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    if v["type"] != "system" || v["subtype"] != "init" {
        return None;
    }
    let names = |k: &str| v[k].as_array().map(|a| a.iter().map(|t| t.as_str().map(str::to_string).unwrap_or_else(|| t["name"].as_str().unwrap_or("?").to_string())).collect::<Vec<_>>());
    Some(match (names("tools"), names("mcp_servers")) {
        (Some(t), Some(m)) if t.is_empty() && m.is_empty() => Ok(()),
        (Some(t), Some(m)) => Err(format!("it has tools {t:?} and MCP servers {m:?}")),
        _ => Err("its start did not list its tools".to_string()),
    })
}

/// A new, empty, private (0700, random name, made fresh) directory for one call, out of the file
/// tools' reach and not under the shared `/tmp` (whose `CLAUDE.md`, or a directory made first at a
/// predictable name, any account could place); removed when dropped.
pub(super) struct PrivateDir(yantrik_ml::private_dir::FreshDir);

impl PrivateDir {
    pub(super) fn new() -> Result<PrivateDir, String> {
        yantrik_ml::private_dir::fresh_work_dir("claude-think").map(PrivateDir).map_err(|e| format!("no private directory to run it in ({e})"))
    }

    pub(super) fn path(&self) -> &std::path::Path {
        self.0.path()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_no_tools_and_no_servers_is_contained() {
        let line = |tools: &str, servers: &str| format!(r#"{{"type":"system","subtype":"init","tools":{tools},"mcp_servers":{servers}}}"#);
        assert_eq!(init_verdict(&line("[]", "[]")), Some(Ok(())));
        assert!(matches!(init_verdict(&line(r#"["Skill","Workflow"]"#, "[]")), Some(Err(e)) if e.contains("Skill")));
        assert!(matches!(init_verdict(&line("[]", r#"[{"name":"x","status":"connected"}]"#)), Some(Err(_))));
        assert!(matches!(init_verdict(r#"{"type":"system","subtype":"init"}"#), Some(Err(_))));
        assert_eq!(init_verdict(r#"{"type":"assistant"}"#), None);
    }

    /// The real CLI, where it is installed: with the arguments a call uses it lists no tools and no
    /// MCP servers (the old deny-list left about thirty, and `--no-input` stopped it starting).
    #[test]
    fn the_installed_cli_runs_a_think_call_with_no_tools() {
        if Command::new("claude").arg("--version").output().is_err() {
            eprintln!("skipped: no claude CLI here");
            return;
        }
        assert_eq!(probe(), Ok(()));
    }

    #[test]
    fn each_call_gets_a_fresh_private_directory_that_is_removed_after() {
        let Ok(a) = PrivateDir::new() else { return eprintln!("skipped: no private directory here") };
        let b = PrivateDir::new().unwrap();
        assert_ne!(a.path(), b.path());
        assert!(!a.path().starts_with(std::env::temp_dir()), "{}", a.path().display());
        #[cfg(unix)]
        assert_eq!(std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(a.path()).unwrap().permissions()) & 0o777, 0o700);
        let path = a.path().to_path_buf();
        drop(a);
        assert!(!path.exists());
    }
}
