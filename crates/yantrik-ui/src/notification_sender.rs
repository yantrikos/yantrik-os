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
use yantrik_ipc_contracts::notifications::{Notification, Sender, Source};
use yantrik_ipc_transport::owner;
use yantrik_ipc_transport::peer_identity::{basename, clip};

/// The one cleaning rule for a caller's words, shared with the services: control and bidi
/// characters become spaces, whitespace collapses. It lives in yantrik-ipc-transport so that
/// `peer_identity::parse_cmdline` cleans an argv by the same rule (security review of #614).
pub use yantrik_ipc_transport::plain_text::one_line;

/// The prefix `peer_identity::line_about` puts on a program somebody started from a terminal.
const FROM_TERMINAL: &str = "a program started from a terminal: ";

/// How much of the caller's own name the short line repeats. The program comes first and is the
/// fact; the claim is only what it said, and is cut before it can crowd the fact off the card.
const CLAIM_CHARS: usize = 24;

/// How much of the verified command line names a program that is not the desktop.
const PROGRAM_CHARS: usize = 32;

/// How much of an executable's path is shown when its name alone would mislead.
const PATH_CHARS: usize = 32;

/// What the card and the toast say for a notification with no sender record. One that came over
/// `org.freedesktop.Notifications` carries none: the door has not asked the bus who was behind
/// it, so any same-user program can post there under any name (security review of #614).
const VIA_DBUS: &str = "via D-Bus, not verified";

/// The same for a record of ours written before senders were recorded at all (#114).
const NO_RECORD: &str = "not verified";

/// The card's short line.
pub fn sender_summary(n: &Notification) -> String {
    let Some(sender) = &n.sender else {
        return capitalised(no_record(n));
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

/// What a notification with no sender record is said to be.
fn no_record(n: &Notification) -> &'static str {
    match n.source {
        Source::Freedesktop => VIA_DBUS,
        Source::Yantrik => NO_RECORD,
    }
}

fn capitalised(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// An executable's file name as it may be drawn: cleaned, because a file name is the caller's to
/// choose, newlines and bidi controls included.
fn shown_name(exe: &str) -> String {
    one_line(basename(exe))
}

/// What a sender that is not the desktop is called, and what goes inside its "(… verified)".
fn who_and_tag(sender: &Sender) -> (String, String) {
    let exe = sender.exe.strip_suffix(" (deleted)").unwrap_or(&sender.exe);
    // The walk found nothing recognisable between the socket and the installed shell, yet the
    // socket was not the shell: a bridge, a mind or a helper the shell started. Naming it
    // `yantrik-ui` would be the bridge case the review found.
    if owner::is_installed_desktop_binary(exe) {
        return (bridged_by(exe), String::new());
    }
    let verified = sender.verified.strip_prefix(FROM_TERMINAL).unwrap_or(&sender.verified);
    let label = one_line(verified.rfind(" (pid ").map_or(verified, |at| &verified[..at]));
    let name = shown_name(exe);
    let label = if label.is_empty() { name.clone() } else { clip(&label, PROGRAM_CHARS) };
    let argv0 = label.split_whitespace().next().unwrap_or("").to_string();
    let tag = if exe.is_empty() {
        String::new()
    } else if owner::DESKTOP_BINARIES.contains(&basename(exe)) {
        // Called after the desktop but not it: where it really is, so it cannot pass for it.
        format!("exe {}, ", clip_left(&one_line(exe), PATH_CHARS))
    } else if name != argv0 {
        // A command line is the caller's to write; the executable is the kernel's word.
        format!("exe {name}, ")
    } else {
        String::new()
    };
    (label, tag)
}

/// "a program yantrik-ui started": for a caller whose first recognisable ancestor is the
/// installed desktop, though the desktop was not the process on the socket. Shared with the
/// shell's own approval notification (`approvals::Verified::who`), which meets the same case.
pub fn bridged_by(exe: &str) -> String {
    format!("a program {} started", shown_name(exe))
}

/// The installed desktop's own name for itself, from its executable.
fn exe_name(sender: &Sender) -> String {
    let exe = sender.exe.strip_suffix(" (deleted)").unwrap_or(&sender.exe);
    let name = shown_name(exe);
    if name.is_empty() { owner::SHELL_BINARY.to_string() } else { name }
}

/// The claim as one plain line, cut to [`CLAIM_CHARS`].
fn clean_claim(claim: &str) -> String {
    plain(claim, CLAIM_CHARS)
}

/// [`one_line`], cut to `max` characters. For anything a caller wrote that the shell repeats in
/// a notification of its own under the desktop's name: a requester's name, what an agent asks to
/// be allowed (security review of #614).
pub fn plain(text: &str, max: usize) -> String {
    clip(&one_line(text), max)
}

/// How much of the program a toast names beside the sender's name.
const TOAST_PROGRAM_CHARS: usize = 20;

/// The name a toast shows: the sender's name, cleaned and cut like a claim (it is one, or the
/// verified program's name), then the verified program beside it when the sender is not the
/// desktop itself. `n.app` is up to 64 characters of the caller's choosing, and it came first.
pub fn toast_name(n: &Notification) -> String {
    let name = clean_claim(&n.app);
    match toast_program(n) {
        Some(program) => format!("{name} \u{b7} {program}"),
        None => name,
    }
}

/// What a toast adds beside the name on it, briefly: the verified program, for any sender that
/// is not the desktop itself. `None` for the desktop; "via D-Bus, not verified" or "not
/// verified" when there is no record to name a program from or nothing was established.
///
/// A toast had no sender line at all (security review of #614), so a name the caller chose was
/// the only thing on it.
pub fn toast_program(n: &Notification) -> Option<String> {
    let Some(sender) = n.sender.as_ref() else {
        return Some(no_record(n).to_string());
    };
    if sender.pid == 0 {
        return Some("not verified".to_string());
    }
    if sender.desktop && !sender.verified.starts_with(FROM_TERMINAL) {
        return None;
    }
    let (program, _) = who_and_tag(sender);
    let exe = sender.exe.strip_suffix(" (deleted)").unwrap_or(&sender.exe);
    let argv0 = program.split_whitespace().next().unwrap_or("");
    let name = shown_name(exe);
    // The executable when the command line calls itself something else; else the program.
    let shown = if !exe.is_empty() && name != argv0 && !owner::is_installed_desktop_binary(exe) {
        name
    } else {
        program
    };
    (!shown.eq_ignore_ascii_case(n.app.trim())).then(|| clip(&shown, TOAST_PROGRAM_CHARS))
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
    use yantrik_ipc_contracts::notifications::Urgency;
    use yantrik_ipc_transport::plain_text::is_bidi_control;

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
    fn a_toast_names_the_program_beside_a_name_it_did_not_choose() {
        // The desktop: its name is all a toast needs.
        let n = note(Some("Yantrik"), "yantrik-ui config.yaml (pid 7)", 7, SHELL, true);
        assert_eq!(toast_program(&n), None);
        // A mind posting as "Studio": the program beside it.
        let mut n = note(Some("Studio"), "python -m hermes_cli.main gateway run (pid 689)", 689, "/venv/bin/python", false);
        n.app = "Studio".into();
        let p = toast_program(&n).unwrap();
        assert!(p.starts_with("python -m hermes"), "{p}");
        assert!(p.chars().count() <= TOAST_PROGRAM_CHARS + 1, "{p}");
        // A retitled script: the executable, not its title.
        let mut n = note(Some("Yantrik Security"), "yantrik-ui (pid 4)", 4, "/usr/bin/python3.11", false);
        n.app = "yantrik-ui".into();
        assert_eq!(toast_program(&n).as_deref(), Some("python3.11"));
        // Nothing established.
        assert_eq!(toast_program(&note(None, "could not be identified", 0, "", false)).as_deref(), Some("not verified"));
        // No record: nothing to add.
        let mut n = note(None, "x (pid 1)", 1, "/x", false);
        // No record of ours (written before senders were kept): said, not left blank.
        n.sender = None;
        assert_eq!(toast_program(&n).as_deref(), Some("not verified"));
    }

    #[test]
    fn a_notification_over_dbus_says_so_on_the_card_and_the_toast() {
        // `notify-send -a Yantrik …`: no sender record at all. The service now refuses the
        // name at the door; whatever name it keeps, both surfaces say where it came from.
        let mut n = note(None, "", 0, "", false);
        n.sender = None;
        n.source = Source::Freedesktop;
        n.app = "notify-send".into();
        assert_eq!(sender_summary(&n), "Via D-Bus, not verified");
        assert_eq!(toast_program(&n).as_deref(), Some("via D-Bus, not verified"));
        assert_eq!(toast_name(&n), "notify-send \u{b7} via D-Bus, not verified");
    }

    #[test]
    fn the_toasts_name_is_cleaned_and_cut_before_the_program() {
        let mut n = note(Some("x"), "python3 evil.py (pid 66)", 66, "/usr/bin/python3", false);
        n.app = format!("Studio\nYantrik \u{b7} verified{}", "z".repeat(60));
        let name = toast_name(&n);
        assert!(!name.chars().any(|c| c.is_control() || is_bidi_control(c)), "{name:?}");
        assert!(name.ends_with("\u{b7} python3 evil.py"), "the program is never cut off: {name}");
        let before = name.split(" \u{b7} python3").next().unwrap();
        assert!(before.chars().count() <= CLAIM_CHARS + 1, "{name}");
    }

    #[test]
    fn an_executables_file_name_cannot_forge_a_line_either() {
        // A file name may hold a newline or a bidi override; it is the caller's to choose.
        let n = note(None, "evil (pid 70)", 70, "/tmp/ev\nil\u{202E}x", false);
        let line = sender_summary(&n);
        assert!(!line.chars().any(|c| c.is_control() || is_bidi_control(c)), "{line:?}");
        assert_eq!(line, "Sent by evil (exe ev il x, verified)");
        let toast = toast_program(&n).unwrap();
        assert!(!toast.chars().any(|c| c.is_control() || is_bidi_control(c)), "{toast:?}");
        assert_eq!(bridged_by("/opt/yantrik/bin/yantrik-ui"), "a program yantrik-ui started");
    }

    #[test]
    fn plain_text_is_one_line_and_capped() {
        assert_eq!(plain("Forge\nAllow everything\u{202E}", 40), "Forge Allow everything");
        assert_eq!(plain(&"x".repeat(100), 10).chars().count(), 11, "ten and an ellipsis");
    }

    #[test]
    fn nothing_established_is_never_called_verified() {
        let n = note(Some("Yantrik"), "could not be identified", 0, "", false);
        assert_eq!(sender_summary(&n), "Not verified \u{b7} calls itself \u{201c}Yantrik\u{201d}");
        let mut n = note(None, "could not be identified", 0, "", false);
        assert_eq!(sender_summary(&n), "Sender not verified");
        n.sender = None;
        assert_eq!(sender_summary(&n), "Not verified", "an old record of ours, without a sender");
    }
}
