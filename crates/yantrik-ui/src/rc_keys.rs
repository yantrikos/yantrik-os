//! `config/labwc/rc.xml` is the one source of the desktop's keys. These tests keep the screens
//! and the file honest with each other (tests only).
//!
//! Super+V, Super+A, Super+D and Shift/Ctrl+Print were once declared in a generator in
//! yantrik-os that wrote its own rc.xml only when none existed. The session copies the shipped
//! file over it at every login, so none of them was ever bound, and the Clipboard panel went on
//! saying "Super+V". A screen that names a key nothing binds is a lie about the machine; this
//! is the check that would have caught it.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn rc_xml() -> String {
    let path = repo_root().join("config/labwc/rc.xml");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

use crate::cheat_sheet::canonical;

fn modifier_word(word: &str) -> Option<&'static str> {
    match word.to_lowercase().as_str() {
        "super" | "win" => Some("super"),
        "ctrl" | "control" => Some("ctrl"),
        "alt" => Some("alt"),
        "shift" => Some("shift"),
        _ => None,
    }
}

/// Whether a word is a key as a screen would write it: one letter, digit or `/`, a function
/// key, or a named key. Deliberately not "any word", so that "Press Super to talk" is not a
/// shortcut and "Ctrl+Space to cancel" is.
fn is_key_word(word: &str) -> bool {
    const NAMED: &[&str] = &[
        "space", "tab", "enter", "return", "escape", "esc", "print", "left", "right", "up", "down",
        "home", "end", "delete", "backspace",
    ];
    let lower = word.to_lowercase();
    let mut chars = word.chars();
    let single = matches!((chars.next(), chars.next()), (Some(c), None) if c.is_ascii_alphanumeric() || c == '/');
    let function = lower.len() >= 2
        && lower.starts_with('f')
        && lower[1..].chars().all(|c| c.is_ascii_digit());
    single || function || NAMED.contains(&lower.as_str())
}

/// Every shortcut a piece of text names, canonical.
fn shortcuts_in(text: &str) -> Vec<String> {
    let words: Vec<&str> = text
        .split(|c: char| c.is_whitespace() || c == '+')
        .map(|w| w.trim_matches(|c: char| ",.;:!?()\"'".contains(c)))
        .filter(|w| !w.is_empty())
        .collect();
    let mut found = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let mut modifiers = BTreeSet::new();
        let mut j = i;
        while j < words.len() {
            match modifier_word(words[j]) {
                Some(m) => {
                    modifiers.insert(m);
                    j += 1;
                }
                None => break,
            }
        }
        if !modifiers.is_empty() && j < words.len() && is_key_word(words[j]) {
            found.push(canonical(&modifiers, words[j]));
            i = j + 1;
        } else {
            i = j.max(i + 1);
        }
    }
    found
}

/// Every `key="..."` rc.xml binds, canonical. Read by the cheat sheet's parser, the only one, so
/// a key that is only talked about in a comment (the reserved Super+N) does not count as bound.
pub(crate) fn bound_keys(rc: &str) -> BTreeSet<String> {
    crate::cheat_sheet::keybinds(rc).into_iter().map(|k| k.canonical).collect()
}

/// The text of every string literal on a line of Slint, with `//` comments cut off.
fn string_literals(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_string = false;
    let mut escaped = false;
    let mut prev = '\0';
    for c in line.chars() {
        if in_string {
            if escaped {
                current.push(c);
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                out.push(std::mem::take(&mut current));
                in_string = false;
            } else {
                current.push(c);
            }
        } else if c == '"' {
            in_string = true;
        } else if c == '/' && prev == '/' {
            break;
        }
        prev = c;
    }
    out
}

fn slint_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            slint_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "slint") {
            out.push(path);
        }
    }
}

/// Shortcuts the shell handles itself, inside its own window, which never need the compositor
/// and so are not in rc.xml. Each is `(file, shortcut)` and is allowed in that file only, so a
/// new screen advertising the same key by accident still has to be looked at. An entry here
/// that no longer appears in its file fails the test, so the list cannot go stale.
///
/// Super+K/V/A/Escape are not here: they work from inside any app, which only the compositor
/// can give, so they must be bound.
const HANDLED_IN_THE_WINDOW: &[(&str, &str)] = &[
    // The Files screen's address bar and the Settings search box take these as key events.
    ("ui/file_browser.slint", "ctrl+l"),
    ("ui/settings.slint", "ctrl+f"),
    // The terminal's own tab and find keys, read by the terminal widget.
    ("ui/terminal_workbench.slint", "ctrl+shift+t"),
    ("ui/terminal_workbench.slint", "ctrl+shift+f"),
    // Cancels a recording while the voice overlay has the keyboard.
    ("ui/voice_overlay.slint", "ctrl+space"),
];

/// A screen must not advertise a key the compositor does not bind.
#[test]
fn every_shortcut_a_screen_names_is_bound_in_rc_xml_or_handled_in_its_window() {
    let root = repo_root();
    let bound = bound_keys(&rc_xml());
    let mut files = Vec::new();
    slint_files(&root.join("crates/yantrik-ui-slint/ui"), &mut files);
    slint_files(&root.join("crates/yantrik-ui-kit/slint"), &mut files);
    assert!(files.len() > 20, "found only {} .slint files; the scan is looking in the wrong place", files.len());

    let mut unbound = Vec::new();
    let mut allowed_seen: BTreeSet<(&str, &str)> = BTreeSet::new();
    for file in &files {
        let rel = file.to_string_lossy().replace('\\', "/");
        let src = std::fs::read_to_string(file).unwrap_or_default();
        for (n, line) in src.lines().enumerate() {
            for text in string_literals(line) {
                for key in shortcuts_in(&text) {
                    if bound.contains(&key) {
                        continue;
                    }
                    match HANDLED_IN_THE_WINDOW.iter().find(|(f, k)| rel.ends_with(f) && *k == key) {
                        Some(entry) => {
                            allowed_seen.insert(*entry);
                        }
                        None => unbound.push(format!("{rel}:{}: \"{text}\" names {key}", n + 1)),
                    }
                }
            }
        }
    }
    assert!(
        unbound.is_empty(),
        "these screens name a shortcut config/labwc/rc.xml does not bind. Bind it there, reword \
         the text, or (only if the shell handles the key itself in its own window) list it in \
         HANDLED_IN_THE_WINDOW with the reason:\n{}",
        unbound.join("\n")
    );
    for entry in HANDLED_IN_THE_WINDOW {
        assert!(
            allowed_seen.contains(entry),
            "HANDLED_IN_THE_WINDOW lists {entry:?} but no such text is in that file any more; \
             remove the entry"
        );
    }
}

/// A key bound to an action that was renamed or never existed does nothing, and says nothing:
/// `yos` prints to a terminal nobody is looking at.
#[test]
fn every_yos_action_rc_xml_runs_is_published_by_the_shell() {
    let rc = rc_xml();
    let published = crate::control::locked_state_tests::published_actions();
    let mut seen = 0;
    for command in rc.split("<command>").skip(1) {
        let command = command.split("</command>").next().unwrap_or_default();
        let Some(after) = command.split("yos act shell ").nth(1) else { continue };
        let action = after.split_whitespace().next().unwrap_or_default();
        seen += 1;
        assert!(
            published.iter().any(|p| p == action),
            "rc.xml runs `yos act shell {action}`, which the shell does not publish: {command}"
        );
    }
    assert!(seen >= 8, "found only {seen} `yos act shell` bindings; the scan is reading the wrong thing");
}

/// `focus_window title=Mind View` that matched no window would do nothing and say nothing. Every
/// title an rc.xml command asks for must be one the shell actually gives a window.
#[test]
fn every_window_title_rc_xml_asks_for_is_one_the_shell_gives_a_window() {
    let rc = rc_xml();
    let known = [crate::windows::SHELL_WINDOW_TITLE, crate::mind_view::TITLE];
    let mut seen = 0;
    for command in rc.split("<command>").skip(1) {
        let command = command.split("</command>").next().unwrap_or_default();
        let Some(after) = command.split(" title=").nth(1) else { continue };
        let title = match after.strip_prefix('"') {
            Some(quoted) => quoted.split('"').next().unwrap_or_default(),
            None => after.split_whitespace().next().unwrap_or_default(),
        };
        seen += 1;
        assert!(
            known.contains(&title),
            "rc.xml asks for the window titled {title:?}, which the shell never gives a window              (known: {known:?}): {command}"
        );
    }
    assert!(seen >= 1, "found no `title=` binding; the scan is reading the wrong thing");
}

#[test]
fn the_scan_reads_shortcuts_the_way_screens_write_them() {
    assert_eq!(shortcuts_in("Super K"), ["super+k"]);
    assert_eq!(shortcuts_in("Super+V"), ["super+v"]);
    assert_eq!(
        shortcuts_in("Ctrl Shift T  New tab   ·   Ctrl Shift F  Find"),
        ["ctrl+shift+t", "ctrl+shift+f"]
    );
    assert_eq!(shortcuts_in("Say something or Ctrl+Space to cancel"), ["ctrl+space"]);
    // A bare Super and prose are not shortcuts.
    assert!(shortcuts_in("Press Super to talk to me").is_empty());
    assert!(shortcuts_in("Open a pinned app to get started").is_empty());
    let bound = bound_keys("<!-- <keybind key=\"W-n\"> --><keybind key=\"W-S-v\"></keybind>");
    assert!(bound.contains("super+shift+v") && !bound.contains("super+n"));
}
