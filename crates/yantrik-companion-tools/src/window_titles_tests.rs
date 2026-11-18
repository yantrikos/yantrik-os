//! #1247, gate 12: the companion's window tools name no window whose title names a secret. They run the
//! real tool's formatter on what `wlrctl` prints and on what the shell answers over the bus (the fake
//! shell of the tests above), since a CI machine has no compositor.

use super::*;

const SECRET_TITLES: [&str; 4] = [
    "tokens.json (~/backup) - gedit",
    "tokens (copy 1).json (~/backup) - gedit",
    "#!home!ann!.kube!config# - GNU Emacs at host",
    "Local State - gedit",
];

fn nothing_secret_in(out: &str) {
    for part in ["tokens", ".kube", "Local State"] {
        assert!(!out.contains(part), "{part}: {out}");
    }
}

#[test]
fn list_windows_hides_a_secret_title_and_keeps_the_rest() {
    let printed = format!(
        "gedit: {}\nfirefox: Hacker News - Firefox\nemacs: {}\nfoot: notes.md - nvim\nfoot: {}\n",
        SECRET_TITLES[0], SECRET_TITLES[2], SECRET_TITLES[3]
    );
    let out = listing(&printed);
    nothing_secret_in(&out);
    assert!(out.starts_with("Open windows (5):") && out.contains("Hacker News") && out.contains("notes.md"), "{out}");
    assert_eq!(out.matches(HIDDEN_WINDOW).count(), 3, "{out}");
}

#[cfg(unix)]
#[test]
fn focus_and_close_name_no_secret_window_the_shell_resolved() {
    for title in SECRET_TITLES {
        let (desk, _) = super::tests::desk_with_shell("g12", move |params| match params["action"].as_str() {
            Some("focus_window") => serde_json::json!({"result": {"accepted": true, "action_id": "app-shell#1", "settled": false, "result": {"focused": title}}}),
            _ => serde_json::json!({"result": {"accepted": true, "action_id": "app-shell#2", "settled": false, "result": {
                "closing": title, "note": format!("{title} was asked to close"),
            }}}),
        });
        let focused = focus_report(focus_via(&desk, "gedit"));
        let closed = close_report(close_via(&desk, "gedit"));
        nothing_secret_in(&focused);
        nothing_secret_in(&closed);
        assert!(focused.contains(HIDDEN_WINDOW) && closed.contains(HIDDEN_WINDOW), "{focused} / {closed}");
    }
    let (desk, _) = super::tests::desk_with_shell("g12-plain", |_| serde_json::json!({"result": {"accepted": true, "action_id": "a#1", "settled": false, "result": {"focused": "Notes", "closing": "Notes"}}}));
    assert_eq!(focus_report(focus_via(&desk, "notes")), "Focused window: Notes");
    assert_eq!(close_report(close_via(&desk, "notes")), "Asked window to close: Notes");
}

/// The shell's refusal names what IS open; each title in it that names a secret is hidden, line by line.
#[test]
fn a_refusal_that_lists_the_open_windows_hides_the_secret_ones() {
    let why = format!("no window answers to 'x'. Open:\n  {}\n  Firefox\n  {}", SECRET_TITLES[0], SECRET_TITLES[1]);
    for report in [focus_report(Err(why.clone())), close_report(Err(why))] {
        nothing_secret_in(&report);
        assert!(report.contains("Firefox") && report.contains("no window answers"), "{report}");
    }
}

#[test]
fn focus_context_reports_no_secret_title() {
    let lines = [format!("gedit: {}", SECRET_TITLES[0]), "firefox: Hacker News - Firefox".to_string(), format!("foot: {}", SECRET_TITLES[3])];
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let out = context_report("coding", &refs[..1], &refs[1..], &format!("\nFocused: {}", shown(&lines[0])));
    nothing_secret_in(&out);
    assert!(out.contains("[KEEP]") && out.contains("[DISTRACTOR] firefox: Hacker News - Firefox"), "{out}");
}
