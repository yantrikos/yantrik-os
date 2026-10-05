//! The Mind's search grants, from the shell's side (design/mind-egress-2026-09-29.md, section 6).
//!
//! The host asks the person with a card when the Mind wants to search the web in its own words
//! (`yantrik_harness::host::grant`) and tells this module what came of it. This module never
//! writes a grant: on *This session* or *Always* it asks root to, through the updater's own sudo
//! rule (`yantrik-update mind-grant add`), which names the person as the caller and refuses the
//! mind and egress accounts. The query goes to the updater on stdin, never on its command line,
//! which every account reads in /proc and sudo logs. Every answer and every use goes to the
//! journal (`yantrik-mind-grant`) with the query, escaped, and its SHA-256, so an audit can
//! compare what was approved with what was searched: nothing on this machine binds the two but
//! the Mind's own planner (design/mind-egress-2026-09-29.md, section 6). Root journals the adds
//! and revokes it makes.
//!
//! Settings → Harnesses lists the grants in force, read with the same trust checks the Mind
//! makes, each with Revoke.

use yantrik_harness::grants::{self, Grant};
use yantrik_harness::GrantNotice;

/// What the card's answer or a grant's use asks of the machine.
#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    /// One line for the journal.
    Journal(String),
    /// `yantrik-update` with these arguments and this on its stdin (it journals itself).
    Updater(Vec<String>, String),
}

/// What to do about a notice from the host. Pure, for the tests.
pub fn steps(notice: &GrantNotice) -> Vec<Step> {
    match notice {
        GrantNotice::Answered { harness, query, scope_id, answer } => {
            let said = Step::Journal(format!("answered {answer} agent={harness} {}", shown(query)));
            let add = |scope: &str, id: Option<&str>| {
                let mut args = vec!["mind-grant".to_string(), "add".into(), "--scope".into(), scope.into()];
                if let Some(id) = id {
                    args.extend(["--session-id".into(), id.to_string()]);
                }
                args.push("--query-stdin".into());
                Step::Updater(args, query.clone())
            };
            match *answer {
                "session" => vec![said, add("session", Some(scope_id))],
                "always" => vec![said, add("always", None)],
                _ => vec![said],
            }
        }
        GrantNotice::Used { harness, query, grant } => vec![Step::Journal(format!(
            "used {} agent={harness} scope={} {}",
            grant.id,
            grant.scope,
            shown(query)
        ))],
    }
}

/// Act on a notice, off the caller's thread: the updater asks sudo, which may take a moment.
pub fn on_notice(notice: GrantNotice) {
    let steps = steps(&notice);
    std::thread::spawn(move || {
        for step in steps {
            match step {
                Step::Journal(line) => journal(&line),
                Step::Updater(args, input) => {
                    let args: Vec<&str> = args.iter().map(String::as_str).collect();
                    match crate::control_update::run_updater_with_input(&args, Some(input.as_bytes())) {
                        Ok(run) if run.code == 0 => {}
                        Ok(run) => tracing::warn!(code = run.code, why = %run.stderr.trim(), "the search grant was not written"),
                        Err(e) => tracing::warn!(why = %e, "the search grant was not written"),
                    }
                }
            }
        }
    });
}

/// Take a grant back: root rewrites both files without it.
pub fn revoke(id: &str) -> Result<(), String> {
    let run = crate::control_update::run_updater(&["mind-grant", "revoke", id])?;
    if run.code == 0 {
        Ok(())
    } else {
        Err(format!("could not revoke the grant: {}", run.stderr.lines().last().unwrap_or("the updater refused")))
    }
}

/// One row of Settings: the grant's id, what it allows, for how long.
pub fn row(grant: &Grant, now: u64) -> (String, String, String) {
    let title = "Search the web in its own words".to_string();
    let how_long = match grant.expires_at {
        None => "always, until you revoke it".to_string(),
        Some(at) => format!("{} more minutes", at.saturating_sub(now).div_ceil(60)),
    };
    let scope = match grant.scope.as_str() {
        "always" => "The Mind".to_string(),
        "session" => "This session of the Mind".to_string(),
        _ => format!("Run {}", grant.scope_id.as_deref().unwrap_or("?")),
    };
    let by = if grant.granted_by == "run-starter" { "started with the run" } else { "you allowed it" };
    (grant.id.clone(), title, format!("{scope} · {how_long} · {by}"))
}

/// The rows for the grants in force now.
pub fn rows() -> Vec<(String, String, String)> {
    let now = grants::now();
    grants::in_force().iter().map(|g| row(g, now)).collect()
}

/// A query as journal fields: `sha256=` over its bytes as the Mind sent them, then `query=` the
/// text quoted, with every character outside printable ASCII (and the quote and backslash)
/// written as an escape, so nothing in it can split, hide or recolour the line.
fn shown(query: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest: String = Sha256::digest(query.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256={digest} query=\"{}\"", query.escape_default())
}

fn journal(line: &str) {
    tracing::info!(target: "yantrik_mind_grant", "{line}");
    let _ = std::process::Command::new("logger").args(["-t", "yantrik-mind-grant", "--", line]).status();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answered(answer: &'static str) -> GrantNotice {
        GrantNotice::Answered { harness: "mind".into(), query: "rust 2027 edition".into(), scope_id: "5f1c2a9e0b7d4c3e".into(), answer }
    }

    /// SHA-256 of "rust 2027 edition".
    fn digest() -> String {
        use sha2::{Digest, Sha256};
        Sha256::digest(b"rust 2027 edition").iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn once_and_no_are_journalled_with_the_query_and_its_digest_and_store_nothing() {
        assert_eq!(
            steps(&answered("once")),
            [Step::Journal(format!("answered once agent=mind sha256={} query=\"rust 2027 edition\"", digest()))]
        );
        assert_eq!(steps(&answered("no")).len(), 1);
    }

    #[test]
    fn a_query_is_journalled_escaped_and_its_digest_is_of_the_bytes_sent() {
        let q = "caf\u{e9} \u{202e}x\n\"y\"\\";
        let line = shown(q);
        assert!(line.ends_with(r#"query="caf\u{e9} \u{202e}x\n\"y\"\\""#), "{line}");
        assert!(line.is_ascii() && !line.contains('\n'), "{line}");
        use sha2::{Digest, Sha256};
        let want: String = Sha256::digest(q.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
        assert!(line.starts_with(&format!("sha256={want} ")), "{line}");
    }

    #[test]
    fn session_and_always_ask_root_to_write_the_grant() {
        let s = steps(&answered("session"));
        assert_eq!(
            s[1],
            Step::Updater(
                ["mind-grant", "add", "--scope", "session", "--session-id", "5f1c2a9e0b7d4c3e", "--query-stdin"].map(String::from).to_vec(),
                "rust 2027 edition".into()
            )
        );
        let a = steps(&answered("always"));
        assert_eq!(
            a[1],
            Step::Updater(["mind-grant", "add", "--scope", "always", "--query-stdin"].map(String::from).to_vec(), "rust 2027 edition".into())
        );
        // The query is on stdin, never among the arguments (/proc/*/cmdline, sudo's log).
        for step in [&s[1], &a[1]] {
            let Step::Updater(args, _) = step else { unreachable!() };
            assert!(args.iter().all(|a| !a.contains("rust")), "{args:?}");
        }
    }

    #[test]
    fn a_use_is_journalled_with_the_grant_and_the_query() {
        let grant = Grant {
            id: "g-0123456789ab".into(),
            agent: "mind".into(),
            capability: grants::CAPABILITY.into(),
            scope: "always".into(),
            scope_id: None,
            granted_at: 1000,
            expires_at: None,
            granted_by: "person".into(),
        };
        let used = GrantNotice::Used { harness: "mind".into(), query: "q\nx".into(), grant: grant.clone() };
        let line = format!("used g-0123456789ab agent=mind scope=always {}", shown("q\nx"));
        assert_eq!(steps(&used), [Step::Journal(line.clone())]);
        assert!(line.ends_with(r#"query="q\nx""#), "{line}");
        assert_eq!(row(&grant, 2000).2, "The Mind · always, until you revoke it · you allowed it");
        let run = Grant { scope: "run".into(), scope_id: Some("research-42".into()), expires_at: Some(1000 + 4 * 3600), granted_by: "run-starter".into(), ..grant };
        assert_eq!(row(&run, 1000).2, "Run research-42 · 240 more minutes · started with the run");
    }
}
