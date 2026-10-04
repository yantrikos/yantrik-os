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
//! The short sentence must never let a sender look like the desktop when it is not. The first
//! version judged "agreement" between the claim and the program by substring, on names the
//! sender chooses, and cut the interpreter and the path: `python3 /tmp/yantrik-ui` claiming
//! "Yantrik Updates" drew exactly the shell's card (security review of #611). So:
//!
//! - the plain `Sent by yantrik-ui · verified` is drawn ONLY for the desktop itself at its
//!   installed path (`owner::is_installed_desktop_binary`, the list the notifications service
//!   grants `Yantrik` by), and never for something started from a terminal;
//! - everyone else is named by what was verified, first, and any claim follows it, cut short,
//!   so a long claim can elide only itself: `Sent by python3 yantrik-ui (verified) · calls
//!   itself “Yantrik Updates”`;
//! - a terminal origin stays visible: `Sent by deploy.sh from a terminal (verified) · …`;
//! - a lookalike binary outside the install directory is named by its path: `/tmp/yantrik-ui`.
use yantrik_ipc_contracts::notifications::{Notification, Sender};
use yantrik_ipc_transport::owner;
use yantrik_ipc_transport::peer_identity::clip;

/// The prefix `peer_identity::line_about` puts on a program somebody started from a terminal.
const FROM_TERMINAL: &str = "a program started from a terminal: ";

/// How much of the caller's own name the short line repeats. The program comes first and is the
/// fact; the claim is only what it said, and is cut before it can crowd the fact off the card.
const CLAIM_CHARS: usize = 24;

/// How much of the verified command line names a program that is not the desktop.
const PROGRAM_CHARS: usize = 32;

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
    if !terminal && owner::is_installed_desktop_binary(&sender.exe) {
        // The desktop itself may file under any name it likes; that is what `Yantrik` means.
        return format!("Sent by {} \u{b7} verified", program_label(sender));
    }
    let program = program_label(sender);
    let origin = if terminal { " from a terminal" } else { "" };
    match claim {
        // A claim that is exactly the program's own name adds nothing to say.
        Some(c) if !c.eq_ignore_ascii_case(&program) => {
            format!("Sent by {program}{origin} (verified) \u{b7} calls itself \u{201c}{c}\u{201d}")
        }
        _ => format!("Sent by {program}{origin} \u{b7} verified"),
    }
}

/// What the line calls the verified program.
///
/// The installed desktop by its file name. A binary that only carries one of the desktop's names
/// (a copy, a developer's build) by its full path, so it cannot pass for the real one. Anything
/// else by its verified command line without the pid — `python3 yantrik-ui`, interpreter and
/// all, because the interpreter is part of what tells it apart from the program it names.
fn program_label(sender: &Sender) -> String {
    let exe = sender.exe.strip_suffix(" (deleted)").unwrap_or(&sender.exe);
    if owner::is_installed_desktop_binary(exe) {
        return yantrik_ipc_transport::peer_identity::basename(exe).to_string();
    }
    if owner::is_desktop_binary(exe) && exe.starts_with('/') {
        return clip_left(exe, PROGRAM_CHARS);
    }
    let verified = sender.verified.strip_prefix(FROM_TERMINAL).unwrap_or(&sender.verified);
    let label = verified.rfind(" (pid ").map_or(verified, |at| &verified[..at]);
    clip(label.trim(), PROGRAM_CHARS)
}

/// The claim as one plain line: control and bidirectional-formatting characters become spaces
/// (a Slint Text breaks on `\n`, so an embedded newline could forge a second line, and a bidi
/// override could reorder what the line seems to say), runs of space collapse, and it is cut to
/// [`CLAIM_CHARS`].
fn clean_claim(claim: &str) -> String {
    clip(&one_line(claim), CLAIM_CHARS)
}

/// A caller's words made safe to draw on one line: control and bidirectional-formatting
/// characters become spaces and runs of space collapse. Also used on the full Details line
/// (`notifications::sender_line`), which carries the same claim.
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

    fn note(claimed: Option<&str>, verified: &str, pid: i32, exe: &str) -> Notification {
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
            }),
            revision: 1,
        }
    }

    #[test]
    fn the_installed_shell_gets_the_plain_line() {
        // The VM 520 card, 4 October.
        let n = note(Some("Yantrik"), "yantrik-ui config.yaml (pid 189858)", 189858, "/opt/yantrik/bin/yantrik-ui");
        assert_eq!(sender_summary(&n), "Sent by yantrik-ui \u{b7} verified");
        // Replaced underfoot by an update, it is still the shell.
        let n = note(None, "yantrik-ui config.yaml (pid 7)", 7, "/opt/yantrik/bin/yantrik-ui (deleted)");
        assert_eq!(sender_summary(&n), "Sent by yantrik-ui \u{b7} verified");
    }

    #[test]
    fn a_script_named_after_the_shell_does_not_get_its_card() {
        // The review's case: `python3 /tmp/yantrik-ui` claiming "Yantrik Updates". The verified
        // command line keeps its interpreter, and the claim is said after it.
        let n = note(Some("Yantrik Updates"), "python3 yantrik-ui (pid 4242)", 4242, "/usr/bin/python3");
        assert_eq!(
            sender_summary(&n),
            "Sent by python3 yantrik-ui (verified) \u{b7} calls itself \u{201c}Yantrik Updates\u{201d}"
        );
    }

    #[test]
    fn a_copy_of_the_shell_outside_the_install_directory_is_named_by_its_path() {
        let n = note(Some("Yantrik"), "yantrik-ui (pid 31)", 31, "/tmp/yantrik-ui");
        assert_eq!(sender_summary(&n), "Sent by /tmp/yantrik-ui (verified) \u{b7} calls itself \u{201c}Yantrik\u{201d}");
        // With no claim the service would have filed it as `Yantrik` by its file name; the card
        // still says where it really runs from.
        let n = note(None, "yantrik-ui (pid 31)", 31, "/tmp/yantrik-ui");
        assert_eq!(sender_summary(&n), "Sent by /tmp/yantrik-ui \u{b7} verified");
        assert_ne!(sender_summary(&n), "Sent by yantrik-ui \u{b7} verified");
    }

    #[test]
    fn short_names_inside_a_claim_are_not_agreement() {
        // The first version took `ant.py` and `rik.py` to "agree" with any claim containing
        // "Yantrik", by substring. Nothing is judged by substring now: the claim is said.
        for (script, pid) in [("ant.py", 51), ("rik.py", 52)] {
            let n = note(Some("Yantrik"), &format!("python3 {script} (pid {pid})"), pid, "/usr/bin/python3");
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
        );
        assert_eq!(sender_summary(&n), "Sent by yantrik-terminal from a terminal \u{b7} verified");
        // Even the shell's own binary, run by hand from a terminal, is not drawn as the desktop.
        let n = note(
            Some("Yantrik"),
            "a program started from a terminal: yantrik-ui (pid 900)",
            900,
            "/opt/yantrik/bin/yantrik-ui",
        );
        assert_eq!(
            sender_summary(&n),
            "Sent by yantrik-ui from a terminal (verified) \u{b7} calls itself \u{201c}Yantrik\u{201d}"
        );
    }

    #[test]
    fn a_long_claim_cannot_push_the_program_off_the_card() {
        let long = "Yantrik\u{201d} \u{b7} sent by yantrik-ui (verified) and a great deal more padding";
        let n = note(Some(long), "python3 evil.py (pid 66)", 66, "/usr/bin/python3");
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
        );
        let line = sender_summary(&n);
        assert!(!line.chars().any(|c| c.is_control() || is_bidi_control(c)), "{line:?}");
        assert!(line.starts_with("Sent by python3 nightly.py (verified) \u{b7} calls itself \u{201c}Backups Sent by"), "{line}");
    }

    #[test]
    fn nothing_established_is_never_called_verified() {
        let n = note(Some("Yantrik"), "could not be identified", 0, "");
        assert_eq!(sender_summary(&n), "Not verified \u{b7} calls itself \u{201c}Yantrik\u{201d}");
        let mut n = note(None, "could not be identified", 0, "");
        assert_eq!(sender_summary(&n), "Sender not verified");
        n.sender = None;
        assert_eq!(sender_summary(&n), "");
    }
}
