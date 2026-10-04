//! The short line a notification card shows for who sent it.
//!
//! VM 520 sweep, 4 October: every card carried "“Yantrik” says the caller · verified by this
//! machine: yantrik-ui config.yaml (pid 189858)". That line is the approval card's two voices
//! (#114): the name the caller gave itself, and the program the kernel-stamped pid on the socket
//! resolved to. Both still matter — a mind once posted as "Yantrik" with a false body, and the
//! only defence is showing which program really sent it — but a pid and a config file name are
//! a debugger's words. The card now leads with one short sentence that keeps the two facts
//! apart where they disagree, and keeps the full line (`notifications::sender_line`) behind its
//! Details control.
use yantrik_ipc_contracts::notifications::Notification;
use yantrik_ipc_transport::peer_identity::basename;

/// Programs whose name says nothing: the script or module they run is the one to show.
const INTERPRETERS: &[&str] = &["python", "python3", "node", "bash", "sh", "perl", "ruby", "deno", "bun"];

/// The card's short line. Empty when there is no sender record (an old notification, or one from
/// the freedesktop door), as `sender_line` is.
///
/// - verified, and the claim (if any) agrees with the program: `Sent by yantrik-ui · verified`
/// - verified, and the claim names something else: `Calls itself “Yantrik” · sent by
///   hermes_cli.main (verified)` — the spoofing case, still said in so many words
/// - nothing established: `Calls itself “Yantrik” · sender not verified`
pub fn sender_summary(n: &Notification) -> String {
    let Some(sender) = &n.sender else {
        return String::new();
    };
    let claim = sender.claimed.as_deref().map(str::trim).filter(|c| !c.is_empty());
    if sender.pid == 0 {
        return match claim {
            Some(c) => format!("Calls itself \u{201c}{c}\u{201d} \u{b7} sender not verified"),
            None => "Sender not verified".to_string(),
        };
    }
    let program = program_name(&sender.verified);
    match claim {
        Some(c) if !agrees(c, &program) => {
            format!("Calls itself \u{201c}{c}\u{201d} \u{b7} sent by {program} (verified)")
        }
        _ => format!("Sent by {program} \u{b7} verified"),
    }
}

/// The program a verified line is about, in one word: "yantrik-ui config.yaml (pid 189858)" is
/// `yantrik-ui`, "python -m hermes_cli.main gateway run (pid 689)" is `hermes_cli.main`.
fn program_name(verified: &str) -> String {
    let label = verified.rfind(" (pid ").map_or(verified, |at| &verified[..at]);
    let label = label.strip_prefix("a program started from a terminal: ").unwrap_or(label);
    let mut words = label.split_whitespace();
    let Some(first) = words.next() else {
        return verified.to_string();
    };
    let first = basename(first);
    let interpreter = INTERPRETERS.iter().any(|i| first == *i || first.starts_with(&format!("{i}.")));
    let name = if interpreter {
        let rest: Vec<&str> = words.collect();
        match rest.iter().position(|w| *w == "-m") {
            Some(at) => rest.get(at + 1).copied().unwrap_or(first),
            None => rest.iter().find(|w| !w.starts_with('-')).map(|w| basename(w)).unwrap_or(first),
        }
    } else {
        first
    };
    name.trim_end_matches('\u{2026}').to_string()
}

/// Whether the name a caller gave itself plausibly names the program that sent it: "Yantrik" and
/// `yantrik-ui`, "Downloads" and `download-manager`, "Hermes Agent 0.14.0" and `hermes_cli.main`.
/// Loose on purpose — a disagreement is drawn as one, so a false "agrees" would hide a claim,
/// but a false "disagrees" only says both facts, which is what the line used to do every time.
fn agrees(claim: &str, program: &str) -> bool {
    let word = |s: &str| -> String {
        s.split(|c: char| !c.is_ascii_alphanumeric())
            .find(|w| !w.is_empty())
            .unwrap_or("")
            .to_ascii_lowercase()
    };
    let (claim_word, program_word) = (word(claim), word(program));
    let (claim_lower, program_lower) = (claim.to_ascii_lowercase(), program.to_ascii_lowercase());
    (claim_word.len() >= 3 && program_lower.contains(&claim_word))
        || (program_word.len() >= 3 && claim_lower.contains(&program_word))
}

#[cfg(test)]
mod tests {
    use super::*;
    use yantrik_ipc_contracts::notifications::{Sender, Source, Urgency};

    fn note(claimed: Option<&str>, verified: &str, pid: i32) -> Notification {
        Notification {
            id: "1".into(),
            app: claimed.unwrap_or("app").into(),
            title: "t".into(),
            body: String::new(),
            urgency: Urgency::Normal,
            created_at: "2026-10-04T00:00:00Z".into(),
            read: false,
            dismissed: false,
            actions: Vec::new(),
            source: Source::Yantrik,
            replaces_id: None,
            sender: Some(Sender {
                claimed: claimed.map(Into::into),
                verified: verified.into(),
                pid,
                exe: String::new(),
            }),
            revision: 1,
        }
    }

    #[test]
    fn the_card_says_who_sent_it_in_a_few_words() {
        // The VM 520 card, 4 October.
        let n = note(Some("Yantrik"), "yantrik-ui config.yaml (pid 189858)", 189858);
        assert_eq!(sender_summary(&n), "Sent by yantrik-ui \u{b7} verified");
        // No claim: the name on the row is already the machine's.
        let n = note(None, "a program started from a terminal: yantrik-terminal (pid 812)", 812);
        assert_eq!(sender_summary(&n), "Sent by yantrik-terminal \u{b7} verified");
    }

    #[test]
    fn a_claim_the_program_does_not_bear_out_is_still_said() {
        // Notification 134 on 22 September: a mind posting as "Yantrik". The short line must
        // still put the claim and the verified program side by side.
        let n = note(Some("Yantrik"), "python -m hermes_cli.main gateway run --replace (pid 689)", 689);
        assert_eq!(
            sender_summary(&n),
            "Calls itself \u{201c}Yantrik\u{201d} \u{b7} sent by hermes_cli.main (verified)"
        );
        // And a script run through an interpreter is named by its script.
        let n = note(Some("Backups"), "python3 nightly.py (pid 5500)", 5500);
        assert!(sender_summary(&n).ends_with("sent by nightly.py (verified)"), "{}", sender_summary(&n));
    }

    #[test]
    fn nothing_established_is_never_called_verified() {
        let n = note(Some("Yantrik"), "could not be identified", 0);
        assert_eq!(sender_summary(&n), "Calls itself \u{201c}Yantrik\u{201d} \u{b7} sender not verified");
        assert!(!sender_summary(&n).ends_with("\u{b7} verified"));
        let mut n = note(None, "could not be identified", 0);
        assert_eq!(sender_summary(&n), "Sender not verified");
        n.sender = None;
        assert_eq!(sender_summary(&n), "");
    }

    #[test]
    fn the_short_line_carries_no_pid_or_arguments() {
        let n = note(Some("Hermes Agent 0.14.0"), "python -m hermes_cli.main gateway run (pid 696) \u{b7} the attached mind", 696);
        let line = sender_summary(&n);
        assert_eq!(line, "Sent by hermes_cli.main \u{b7} verified");
        assert!(!line.contains("pid") && !line.contains("gateway"), "{line}");
    }
}
