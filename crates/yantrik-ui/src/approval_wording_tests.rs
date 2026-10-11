//! Tests for approval_wording.rs: each rule of the card's wording, and each way a caller tried to
//! write a line of the card's own (security review of #639).
use super::*;

const DELETE_EVENT: &str = "Take an event off the calendar. It is not recoverable";

fn app(text: &str) -> Published<'_> {
    Published::app(text)
}

/// The sign-off's own example: a mind calling a deletion "Tidy up" must not get a "Tidy up"
/// button. The description is a `Published`, made only from the card's app-published field, so the
/// caller's words cannot be handed to the label; and whatever the caller wrote, the label is the
/// app's action.
#[test]
fn the_label_never_comes_from_the_callers_purpose() {
    for caller in ["Tidy up", "Allow once", "Keep everything safe", "Delete everything"] {
        let said = format!("{DELETE_EVENT} {caller}");
        let label = confirm_label(destructive("sensitive", &said), "delete_event", app(DELETE_EVENT));
        assert_eq!(label, "Delete event", "caller wrote {caller:?}");
    }
    // A caller that adds "cannot be undone" to a harmless action makes the card more careful,
    // never relabels it with its own words.
    let label = confirm_label(destructive("standard", "Open a note. This cannot be undone"), "open_note", app("Open a note."));
    assert_eq!(label, ALLOW_ONCE);

    // The type is the proof: outside tests a `Published` is made by `Published::of(card)` alone,
    // from `Card::purpose` (the app's sentence, never `caller_says`), and every caller of the
    // label and the consequences passes one made that way.
    let src = include_str!("approval_wording.rs");
    let imp = &src[src.find("impl<'a> Published<'a>").unwrap()..];
    let imp = &imp[..imp.find("\n}\n").unwrap()];
    assert_eq!(imp.matches("pub fn ").count(), 2, "`of` and the test-only `app`, nothing else:\n{imp}");
    assert!(imp.contains("Published(&card.purpose)"), "made from the card's app-published field");
    assert!(imp.contains("#[cfg(test)]\n    pub fn app"), "the free constructor is for tests only");
    let rows = include_str!("control_approvals.rs");
    let rows = rows.split("#[cfg(test)]").next().unwrap();
    assert!(!rows.contains("Published::app"), "the shell never makes one from free text");
    assert!(rows.contains("let published = approval_wording::Published::of(&card);"));
}

/// Unrecoverable or dangerous: the action's own label. Everything else: "Allow once".
#[test]
fn destructive_cards_name_the_action_and_the_rest_allow_once() {
    assert!(destructive("sensitive", DELETE_EVENT), "the app says it is not recoverable");
    assert!(destructive("dangerous", "End a running process by pid"), "graded dangerous");
    assert!(!destructive("sensitive", "Start a recipe with its inputs."));

    let label = |action: &str, said: &str| confirm_label(true, action, app(said));
    assert_eq!(label("delete_event", DELETE_EVENT), "Delete event");
    assert_eq!(label("kill_process", "End a running process by pid"), "Kill process");
    assert_eq!(label("files_delete", "Move a file or folder to recoverable Trash"), "Move a file or folder");
    assert_eq!(label("installer_reboot", "Reboot into the installed system"), "Reboot");
    assert_eq!(label("apply_update", "Download, verify, and install the channel's latest build, then restart"), "Apply update");
    assert_eq!(label("remove", "Delete a container and its writable layer. It cannot be undone."), "Delete a container");
    assert_eq!(label("wifi_disconnect", "Leave the Wi-Fi network this machine is on."), "Leave the Wi-Fi network");
    assert_eq!(label("delete", "Throw a snippet away. It cannot be undone."), "Delete", "a phrasal verb is not cut at its particle");
    assert_eq!(label("wifi_radio", "Turn the Wi-Fi radio on or off."), ALLOW_ONCE);
    assert_eq!(label("", ""), ALLOW_ONCE);
    // Not destructive: always "Allow once", never the verb.
    assert_eq!(confirm_label(false, "delete_event", app(DELETE_EVENT)), ALLOW_ONCE);
    // A phrase too long for the button is not cut into a different one.
    assert_eq!(verb_phrase("x", app("Delete the extraordinarily long-winded archive")), None);
}

/// A phrase cut at a particle has lost part of its verb (review of #639, N4).
#[test]
fn a_phrase_cut_at_a_particle_is_no_label() {
    assert_eq!(verb_phrase("radio", app("Leave the radio on.")), None);
    assert_eq!(verb_phrase("firewall", app("Switch off the firewall.")), None);
    assert_eq!(verb_phrase("switch_off", app("")), None, "nor from an id");
    assert_eq!(confirm_label(true, "firewall", app("Switch off the firewall.")), ALLOW_ONCE);
    assert_eq!(verb_phrase("x", app("Switch the theme to dark.")), Some("Switch the theme".into()));
}

#[test]
fn the_consequence_lines_say_what_changes_and_whether_it_comes_back() {
    let c = consequences("delete_event", app(DELETE_EVENT), DELETE_EVENT, "", &["id: sweep-demo-not-real".into()]);
    assert_eq!(c.what, "Deletes: id: sweep-demo-not-real");
    assert_eq!(c.exactly, "id: sweep-demo-not-real");
    assert_eq!(c.undo, UNDO_APP);
    // A target the app named; the arguments the grant binds beside it.
    let c = consequences("delete_event", app(DELETE_EVENT), DELETE_EVENT, "id 01a0c718\u{2026} is \u{201c}Dentist\u{201d}", &["id: 01a0c718-aaaa".into()]);
    assert_eq!(c.what, "Deletes: id 01a0c718\u{2026} is \u{201c}Dentist\u{201d}");
    assert_eq!(c.exactly, "id: 01a0c718-aaaa");
    let c = consequences("agent_run", app("Run it."), "Run it.", "", &["command: ls".into()]);
    assert_eq!((c.what.as_str(), c.undo.as_str()), ("Runs: command: ls", ""));
    let c = consequences("thing", app("Does a thing."), "Does a thing.", "", &["(no arguments)".into()]);
    assert_eq!(c, Consequences::default());
    // Only the caller said it cannot be undone: the row does not credit the app.
    let c = consequences("set_mode", app("Set the mode."), "Set the mode. This is permanent.", "", &["mode: x".into()]);
    assert_eq!((c.what.as_str(), c.undo.as_str()), ("Sets: mode: x", UNDO_CALLER));
    // The warning is not said twice; a different one still is.
    assert_eq!(warning_beside("The app says this cannot be undone.", UNDO_APP), "");
    assert_ne!(warning_beside("This is graded dangerous and the app says it cannot be undone.", UNDO_APP), "");
    assert_eq!(verb_said(Some("Apply update")), "Applies");
    assert_eq!(verb_said(Some("Switch")), "Switches");
    assert_eq!(verb_said(None), "Acts on");
    // The card draws the undo rows in the warning's red by their exact words.
    let card = include_str!("../../yantrik-ui-slint/ui/components/intent_lens.slint");
    assert!(card.contains(&format!("\"{UNDO_APP}\"")) && card.contains(&format!("\"{UNDO_CALLER}\"")));
}

/// Review of #639, B1: `{"pid": "2210\nUndo: possible, the process restarts itself"}` drew a row
/// in the card's own voice, and a right-to-left override in one argument reversed every one after
/// it. Each value is escaped before the values are joined, visibly, so the bytes the grant binds
/// are still what the person reads.
#[test]
fn no_argument_or_target_can_draw_a_line_or_reorder_the_rest() {
    let forged = "pid: 2210\nUndo: possible, the process restarts itself".to_string();
    let flipped = "name: abc\u{202E}gpj.exe".to_string();
    let hidden = "note: a\u{200B}b\u{2028}c\u{E0041}".to_string();
    let c = consequences("kill_process", app("End a running process by pid"), "", "", &[forged, flipped, hidden]);
    for line in [&c.what, &c.exactly] {
        assert!(!line.chars().any(|ch| ch.is_control() || is_bidi_control(ch) || approvals::is_format_char(ch) || ch == '\u{2028}'), "{line:?}");
        assert!(line.contains("2210\\nUndo: possible"), "the newline is shown, not hidden: {line}");
        assert!(line.contains("abc<U+202E>gpj.exe"), "the override is shown: {line}");
        assert!(line.contains("a<U+200B>b<U+2028>c<U+E0041>"), "{line}");
    }
    // The target — an event title can come from an invitation — the same.
    let c = consequences("delete_event", app(DELETE_EVENT), DELETE_EVENT, "id 01a0 is \u{201c}Lunch\nUndo: possible\u{201d}", &["id: 01a0".into()]);
    assert!(!c.what.contains('\n') && c.what.contains("Lunch\\nUndo: possible"), "{}", c.what);
    assert_eq!(visible("C:\\Users\ttab"), "C:\\\\Users\\ttab", "a backslash and a tab are escapes, other text as it is");
    // A `\n` the caller typed is not the newline it might have sent.
    assert_ne!(visible("a\\nb"), visible("a\nb"));
    assert_eq!(visible("a\\nb"), "a\\\\nb");
    // The shared Cf list reaches the rest of the category (review of #639).
    for c in ['\u{0600}', '\u{06DD}', '\u{070F}', '\u{0890}', '\u{08E2}', '\u{110BD}', '\u{13430}', '\u{1BCA0}', '\u{1D173}'] {
        assert!(approvals::is_format_char(c), "{c:?}");
        assert_eq!(visible(&format!("a{c}b")), format!("a<U+{:04X}>b", c as u32));
    }
}

/// The verified fact first, built from the walk's structured findings; the self-declared name
/// under it.
#[test]
fn the_verified_fact_leads_and_the_claim_follows() {
    let terminal = Verified { line: "a program started from a terminal: sshd-session (pid 2290461)".into(), pid: 2290461, exe: "/usr/lib/openssh/sshd-session".into(), from_terminal: true, ..Verified::default() };
    assert_eq!(identity(&terminal), Identity { fact: "Caller process confirmed: sshd-session \u{b7} PID 2290461 \u{b7} from a terminal".into(), tag: "" });
    let mind = Verified { line: "pi --mode rpc (pid 4242) \u{b7} the attached mind".into(), pid: 4242, exe: "/usr/bin/node".into(), attached_mind: "pi".into(), mind_by_pid: true, ..Verified::default() };
    assert_eq!(identity(&mind).fact, "Caller process confirmed: node \u{b7} PID 4242 \u{b7} the attached mind pi");
    // Security review of #648, M2: a mind matched only by a word of the program's own name (its
    // argv0 or script) is a name that matches, not the mind.
    let named = Verified { mind_by_pid: false, ..mind.clone() };
    assert_eq!(identity(&named), Identity { fact: "Caller process confirmed: node \u{b7} PID 4242 \u{b7} name matches pi".into(), tag: "not verified" });
    assert!(!identity(&named).fact.contains("the attached mind"));
    let typed = Verified { from_terminal: true, ..named.clone() };
    assert_eq!(identity(&typed).fact, "Caller process confirmed: node \u{b7} PID 4242 \u{b7} from a terminal \u{b7} name matches pi");
    let program = Verified { line: "curl -s (pid 9)".into(), pid: 9, exe: "/usr/bin/curl".into(), ..Verified::default() };
    assert_eq!(identity(&program).fact, "Caller process confirmed: curl \u{b7} PID 9");
    for nothing in [Verified::default(), Verified { line: "could not be identified".into(), ..Verified::default() }] {
        assert_eq!(identity(&nothing), Identity { fact: "Caller process could not be identified".into(), tag: "not verified" });
    }
    // A card the shell raised itself (the recipe executor's hand_off) has no pid and is no
    // stranger: the desktop is asking.
    let desktop = Verified { line: "the shell's recipe executor, for the Council recipe (r-1)".into(), raised_by_desktop: true, ..Verified::default() };
    assert_eq!(identity(&desktop), Identity { fact: "Raised by this desktop (a recipe step)".into(), tag: "" });
    // And only the flag makes it so: the same words in the line do not.
    let pretender = Verified { line: "Raised by this desktop (a recipe step)".into(), ..Verified::default() };
    assert_eq!(identity(&pretender).tag, "not verified");
    let src = include_str!("control_agents.rs");
    assert!(src.contains("raised_by_desktop: true,"), "the recipe executor's card says the desktop raised it");
    // Review of #639, N2: an argv set with `exec -a` cannot make a program read as a terminal's
    // or as the attached mind. The line is not read at all.
    let argv = Verified { line: "a program started from a terminal: x (pid 77) \u{b7} the attached mind".into(), pid: 77, exe: "/tmp/x".into(), ..Verified::default() };
    assert_eq!(identity(&argv).fact, "Caller process confirmed: x \u{b7} PID 77");
    let odd = Verified { pid: 5, exe: "/tmp/ev\nil\u{202E}".into(), ..Verified::default() };
    assert_eq!(identity(&odd).fact, "Caller process confirmed: ev\\nil<U+202E> \u{b7} PID 5");

    assert_eq!(claim("design-sweep"), "Claimed name: \u{201c}design-sweep\u{201d}");
    assert_eq!(claim(""), "Claimed name: none given");
    // The claim cannot close the card's quote, break a line, or carry anything invisible.
    let forged = claim("x\u{201d} \u{b7} verified\nby the kernel");
    assert!(forged.matches('\u{201d}').count() == 1 && !forged.contains('\n'), "{forged}");
    for quote in ['\u{201f}', '\u{2033}', '\u{ff02}', '\u{301d}', '\u{301e}', '\u{275d}', '\u{275e}', '\u{2039}', '\u{203a}'] {
        let c = claim(&format!("a{quote}b"));
        assert_eq!(c, "Claimed name: \u{201c}a'b\u{201d}", "{quote:?}");
    }
    let c = claim("ve\u{200B}ri\u{E0041}fied\u{2060}");
    assert_eq!(c, "Claimed name: \u{201c}verified\u{201d}", "zero-width and tag characters are dropped: {c:?}");
    assert!(claim(&"n".repeat(300)).chars().count() < 60, "cut short");
}

#[test]
fn the_countdown_is_in_minutes_and_says_what_happens_then() {
    assert_eq!(expires_text(120), "Expires in 2 min, then declined");
    assert_eq!(expires_text(61), "Expires in 2 min, then declined");
    assert_eq!(expires_text(60), "Expires in 1 min, then declined");
    assert_eq!(expires_text(59), "Expires in under a minute, then declined");
    assert_eq!(expires_text(0), "Expires in under a minute, then declined");
}
