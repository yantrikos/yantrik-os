//! The short line a notification card shows for who sent it.
//!
//! VM 520 sweep, 4 October: every card carried "“Yantrik” says the caller · verified by this
//! machine: yantrik-ui config.yaml (pid 189858)". That line is the approval card's two voices
//! (#114): the name the caller gave itself, and the program the kernel-stamped pid on the socket
//! resolved to. Both still matter — a mind once posted as "Yantrik" with a false body, and the
//! only defence is showing which program really sent it — but a pid and a config file name are
//! a debugger's words. The card leads with one short sentence and keeps the full line
//! (`notifications::sender_line`) behind its Details control.
//!
//! The short sentence must never let a sender pass for the desktop. Two reviews of #611 found
//! the ways it could:
//!
//! - judging the claim against the program by substring, on names the sender chooses (`rik.py`
//!   "agreed" with "Yantrik");
//! - judging the desktop by the record's `exe`, which is the first RECOGNISABLE process in the
//!   chain, not the one on the socket — so a mind calling through a bridge the shell spawned
//!   (`yos` on the socket, `yantrik-ui` above it) looked like the shell;
//! - labelling everyone by their command line, which they write: a script titled `yantrik-ui`
//!   read as `yantrik-ui`.
//!
//! So: the plain `Sent by yantrik-ui · verified` comes from `Sender::desktop` alone, which the
//! notifications service sets at call time from the socket's own process and the install
//! directory. Everyone else says `(verified)`, names the real executable whenever it is not
//! what the command line calls itself, keeps a terminal origin in sight, and puts any claim
//! last, cut short and on one line.
use yantrik_ipc_contracts::notifications::{Notification, Sender};
use yantrik_ipc_transport::owner;
use yantrik_ipc_transport::peer_identity::{basename, clip};

/// The prefix `peer_identity::line_about` puts on a program somebody started from a terminal.
const FROM_TERMINAL: &str = "a program started from a terminal: ";

/// How much of the caller's own name the short line repeats. The program comes first and is the
/// fact; the claim is only what it said, and is cut before it can crowd the fact off the card.
const CLAIM_CHARS: usize = 24;

/// How much of the verified command line names a program that is not the desktop.
const PROGRAM_CHARS: usize = 32;

/// How much of an executable's path is shown when its name alone would mislead.
const PATH_CHARS: usize = 32;

/// The card's short line. Empty when there is no sender record (an old notification, or one from
/// the freedesktop door), as `sender_line` is.
pub fn sender_summary(n: &Notification) -> String {
    let Some(sender) = &n.sender else {
        return String::new();
    };
    let claim = sender.claimed.as_deref().map(clean_claim).filter(|c| !c.is_empty());
    if sender.pid == 0 {
        return match claim {
            Some(c) => format!("Not verified \u{b7} calls itself \u{201c}{c}\u{201d}"),
            None => "Sender not verified".to_string(),
        };
    }
    let terminal = sender.verified.starts_with(FROM_TERMINAL);
    if sender.desktop && !terminal {
        // The desktop itself may file under any name it likes; that is what `Yantrik` means.
        return format!("Sent by {} \u{b7} verified", exe_name(sender));
    }
    let (program, tag) = who_and_tag(sender);
    let origin = if terminal { " from a terminal" } else { "" };
    let fact = format!("Sent by {program}{origin} ({tag}verified)");
    match claim {
        // A claim that is exactly what the line already says adds nothing.
        Some(c) if !c.eq_ignore_ascii_case(&program) => {
            format!("{fact} \u{b7} calls itself \u{201c}{c}\u{201d}")
        }
        _ => fact,
    }
}

/// What a sender that is not the desktop is called, and what goes inside its "(… verified)".
fn who_and_tag(sender: &Sender) -> (String, String) {
    let exe = sender.exe.strip_suffix(" (deleted)").unwrap_or(&sender.exe);
    // The walk found nothing recognisable between the socket and the installed shell, yet the
    // socket was not the shell: a bridge, a mind or a helper the shell started. Naming it
    // `yantrik-ui` would be the bridge case the review found.
    if owner::is_installed_desktop_binary(exe) {
        return (format!("a program {} started", basename(exe)), String::new());
    }
    let verified = sender.verified.strip_prefix(FROM_TERMINAL).unwrap_or(&sender.verified);
    let label = verified.rfind(" (pid ").map_or(verified, |at| &verified[..at]).trim();
    let label = if label.is_empty() { basename(exe).to_string() } else { clip(label, PROGRAM_CHARS) };
    let argv0 = label.split_whitespace().next().unwrap_or("");
    let name = basename(exe);
    let tag = if exe.is_empty() {
        String::new()
    } else if owner::DESKTOP_BINARIES.contains(&name) {
        // Called after the desktop but not it: where it really is, so it cannot pass for it.
        format!("exe {}, ", clip_left(exe, PATH_CHARS))
    } else if name != argv0 {
        // A command line is the caller's to write; the executable is the kernel's word.
        format!("exe {name}, ")
    } else {
        String::new()
    };
    (label, tag)
}

/// The installed desktop's own name for itself, from its executable.
fn exe_name(sender: &Sender) -> String {
    let exe = sender.exe.strip_suffix(" (deleted)").unwrap_or(&sender.exe);
    let name = basename(exe);
    if name.is_empty() { owner::SHELL_BINARY.to_string() } else { name.to_string() }
}

/// The claim as one plain line, cut to [`CLAIM_CHARS`].
fn clean_claim(claim: &str) -> String {
    clip(&one_line(claim), CLAIM_CHARS)
}

/// A caller's words made safe to draw on one line: control and bidirectional-formatting
/// characters become spaces and runs of space collapse. A Slint Text breaks on `\n`, so an
/// embedded newline could forge a second line, and a bidi override could reorder what the line
/// seems to say. Also used on the full Details line (`notifications::sender_line`), which
/// carries the same claim.
pub fn one_line(text: &str) -> String {
    let spaced: String = text
        .chars()
        .map(|c| if c.is_control() || is_bidi_control(c) { ' ' } else { c })
        .collect();
    spaced.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Unicode's bidirectional formatting characters: marks, embeddings, overrides and isolates.
fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// Keep the END of a path, where its name is: `…/release/yantrik-ui`.
fn clip_left(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let tail: String = text.chars().skip(count - max).collect();
    format!("\u{2026}{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use yantrik_ipc_contracts::notifications::{Source, Urgency};

    fn note(claimed: Option<&str>, verified: &str, pid: i32, exe: &str, desktop: bool) -> Notification {
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
                exe: exe.into(),
                desktop,
            }),
            revision: 1,
        }
    }

    const SHELL: &str = "/opt/yantrik/bin/yantrik-ui";

    #[test]
    fn the_desktop_the_service_vouched_for_gets_the_plain_line() {
        // The VM 520 card, 4 October.
        let n = note(Some("Yantrik"), "yantrik-ui config.yaml (pid 189858)", 189858, SHELL, true);
        assert_eq!(sender_summary(&n), "Sent by yantrik-ui \u{b7} verified");
        let n = note(None, "yantrik-ui config.yaml (pid 7)", 7, "/opt/yantrik/bin/yantrik-ui (deleted)", true);
        assert_eq!(sender_summary(&n), "Sent by yantrik-ui \u{b7} verified");
    }

    #[test]
    fn a_bridge_under_the_installed_shell_is_not_the_shell() {
        // The re-review's blocker: `yos` on the socket, `yantrik-ui` above it. The record's `exe`
        // is the shell's, because that is the first recognisable process; the service said the
        // socket was not the desktop. The card must not draw the shell's line.
        let n = note(Some("Yantrik"), "yantrik-ui config.yaml (pid 7456)", 7456, SHELL, false);
        let line = sender_summary(&n);
        assert_ne!(line, "Sent by yantrik-ui \u{b7} verified");
        assert_eq!(line, "Sent by a program yantrik-ui started (verified) \u{b7} calls itself \u{201c}Yantrik\u{201d}");
        let n = note(None, "yantrik-ui config.yaml (pid 7456)", 7456, SHELL, false);
        assert_eq!(sender_summary(&n), "Sent by a program yantrik-ui started (verified)");
        // An old record, written before `desktop` existed, reads the same way.
        assert!(!sender_summary(&n).ends_with("\u{b7} verified"));
    }

    #[test]
    fn a_script_named_after_the_shell_does_not_get_its_card() {
        // Round one: `python3 /tmp/yantrik-ui` claiming "Yantrik Updates".
        let n = note(Some("Yantrik Updates"), "python3 yantrik-ui (pid 4242)", 4242, "/usr/bin/python3", false);
        assert_eq!(
            sender_summary(&n),
            "Sent by python3 yantrik-ui (verified) \u{b7} calls itself \u{201c}Yantrik Updates\u{201d}"
        );
        // A script that rewrote its title to `yantrik-ui`, with no claim: the executable is named.
        let n = note(None, "yantrik-ui (pid 4243)", 4243, "/usr/bin/python3.11", false);
        assert_eq!(sender_summary(&n), "Sent by yantrik-ui (exe python3.11, verified)");
    }

    #[test]
    fn a_copy_of_the_shell_outside_the_install_directory_is_named_by_its_path() {
        let n = note(Some("Yantrik"), "yantrik-ui (pid 31)", 31, "/tmp/yantrik-ui", false);
        assert_eq!(
            sender_summary(&n),
            "Sent by yantrik-ui (exe /tmp/yantrik-ui, verified) \u{b7} calls itself \u{201c}Yantrik\u{201d}"
        );
        let n = note(None, "yantrik-ui (pid 31)", 31, "/tmp/yantrik-ui", false);
        assert_eq!(sender_summary(&n), "Sent by yantrik-ui (exe /tmp/yantrik-ui, verified)");
    }

    #[test]
    fn short_names_inside_a_claim_are_not_agreement() {
        for (script, pid) in [("ant.py", 51), ("rik.py", 52)] {
            let n = note(Some("Yantrik"), &format!("python3 {script} (pid {pid})"), pid, "/usr/bin/python3", false);
            assert_eq!(
                sender_summary(&n),
                format!("Sent by python3 {script} (verified) \u{b7} calls itself \u{201c}Yantrik\u{201d}")
            );
        }
        // Notification 134 on 22 September: a mind posting as "Yantrik".
        let n = note(
            Some("Yantrik"),
            "python -m hermes_cli.main gateway run --replace (pid 689)",
            689,
            "/home/yantrik/.hermes/hermes-agent/venv/bin/python",
            false,
        );
        let line = sender_summary(&n);
        assert!(line.starts_with("Sent by python -m hermes_cli.main"), "{line}");
        assert!(line.ends_with("(verified) \u{b7} calls itself \u{201c}Yantrik\u{201d}"), "{line}");
        assert!(!line.contains("pid"), "{line}");
    }

    #[test]
    fn a_terminal_origin_stays_on_the_line() {
        let n = note(
            None,
            "a program started from a terminal: yantrik-terminal (pid 812)",
            812,
            "/opt/yantrik/bin/yantrik-terminal",
            false,
        );
        assert_eq!(sender_summary(&n), "Sent by yantrik-terminal from a terminal (verified)");
        let n = note(Some("Backups"), "a program started from a terminal: deploy.sh (pid 90)", 90, "/usr/bin/bash", false);
        assert_eq!(
            sender_summary(&n),
            "Sent by deploy.sh from a terminal (exe bash, verified) \u{b7} calls itself \u{201c}Backups\u{201d}"
        );
    }

    #[test]
    fn a_long_claim_cannot_push_the_program_off_the_card() {
        let long = "Yantrik\u{201d} \u{b7} sent by yantrik-ui (verified) and a great deal more padding";
        let n = note(Some(long), "python3 evil.py (pid 66)", 66, "/usr/bin/python3", false);
        let line = sender_summary(&n);
        assert!(line.starts_with("Sent by python3 evil.py (verified) \u{b7} calls itself \u{201c}"), "{line}");
        let quoted = line.rsplit('\u{201c}').next().unwrap();
        assert!(quoted.chars().count() <= CLAIM_CHARS + 2, "the claim is cut: {line}");
        assert!(!line.contains("great deal"), "{line}");
    }

    #[test]
    fn control_and_bidi_characters_cannot_forge_a_second_line() {
        let n = note(
            Some("Backups\nSent by yantrik-ui \u{b7} verified\u{202E}gnp\u{2066}"),
            "python3 nightly.py (pid 5500)",
            5500,
            "/usr/bin/python3",
            false,
        );
        let line = sender_summary(&n);
        assert!(!line.chars().any(|c| c.is_control() || is_bidi_control(c)), "{line:?}");
        assert!(line.starts_with("Sent by python3 nightly.py (verified) \u{b7} calls itself \u{201c}Backups Sent by"), "{line}");
        assert_eq!(one_line("a\r\n\tb\u{200F}c"), "a b c");
    }

    #[test]
    fn nothing_established_is_never_called_verified() {
        let n = note(Some("Yantrik"), "could not be identified", 0, "", false);
        assert_eq!(sender_summary(&n), "Not verified \u{b7} calls itself \u{201c}Yantrik\u{201d}");
        let mut n = note(None, "could not be identified", 0, "", false);
        assert_eq!(sender_summary(&n), "Sender not verified");
        n.sender = None;
        assert_eq!(sender_summary(&n), "");
    }
}
