//! The keyboard cheat sheet (Super+/): every key `config/labwc/rc.xml` binds, read from that file.
//!
//! A hand-written list of shortcuts is wrong the week after it is written: a key is rebound, a
//! snap layout is added, and the sheet goes on naming the old one. So there is no list. The
//! sheet is parsed from the rc.xml the session ships (embedded here, so what a person reads is
//! exactly what the compositor was given), and the words for each row live in that file, in a
//! comment just before the binding it describes:
//!
//! ```xml
//! <!-- @help group="Windows" text="Close the window" -->
//! <keybind key="A-F4">
//! ```
//!
//! A binding with no such comment is a test failure, not a blank row. This is also the one
//! parser of rc.xml's keybinds: the `rc_keys` tests read through [`keybinds`] too, so the keys
//! the sheet shows and the keys the screens are checked against cannot be two readings of one
//! file.

use std::collections::BTreeSet;

/// The rc.xml the session installs. Embedded, so there is no path to find at run time.
const SHIPPED_RC: &str = include_str!("../../../config/labwc/rc.xml");

/// The sheet's sections, in the order they are drawn. A binding's group must be one of these.
pub(crate) const GROUPS: [&str; 6] = ["Windows", "Snap", "Workspaces", "Shell", "Minds", "Capture"];

/// What a `@help` comment says about the binding after it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Help {
    pub group: String,
    pub text: String,
}

/// One `<keybind key="...">` in rc.xml.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Keybind {
    /// As labwc spells it: `W-S-s`.
    pub spec: String,
    /// Modifiers in a fixed order, then the key, lowercase: `super+shift+s`.
    pub canonical: String,
    pub help: Option<Help>,
}

/// One row of the sheet.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Row {
    pub group: &'static str,
    /// The key as drawn, one cap per entry: `["Super", "Shift", "S"]`.
    pub caps: Vec<String>,
    pub text: String,
    pub canonical: String,
}

/// A shortcut in one canonical spelling: modifiers in a fixed order, then the key, lowercase.
/// "Super K", "Super+K" and labwc's "W-k" all become "super+k".
pub(crate) fn canonical(modifiers: &BTreeSet<&'static str>, key: &str) -> String {
    let mut out: Vec<String> = ["super", "ctrl", "alt", "shift"]
        .iter()
        .filter(|m| modifiers.contains(*m))
        .map(|m| m.to_string())
        .collect();
    let key = key.to_lowercase();
    out.push(match key.as_str() {
        "esc" => "escape".to_string(),
        // labwc names the key; a screen writes the character.
        "slash" => "/".to_string(),
        _ => key,
    });
    out.join("+")
}

/// `group="Windows" text="Close the window"` out of a comment's body, or None when the comment
/// is not a `@help` one. A `@help` comment that is malformed panics: it is the shipped file, and
/// the test that reads it would otherwise report a missing description for a typo.
fn help_in(comment: &str) -> Option<Help> {
    let at = comment.find("@help")?;
    let rest = &comment[at + "@help".len()..];
    let attr = |name: &str| -> String {
        let marker = format!("{name}=\"");
        let from = rest
            .find(&marker)
            .unwrap_or_else(|| panic!("rc.xml: a @help comment without {name}=\"...\": {comment}"))
            + marker.len();
        let len = rest[from..]
            .find('"')
            .unwrap_or_else(|| panic!("rc.xml: an unterminated {name}=\" in a @help comment: {comment}"));
        rest[from..from + len].trim().to_string()
    };
    Some(Help { group: attr("group"), text: attr("text") })
}

/// Every keybind in `rc`, in file order. A `@help` comment belongs to the keybind that follows
/// it directly; any other comment between them cuts the tie, so a description can never slide
/// onto the wrong key. A key that is only talked about inside a comment (a reserved one) is not
/// a binding.
pub(crate) fn keybinds(rc: &str) -> Vec<Keybind> {
    const KEYBIND: &str = "<keybind key=\"";
    let mut out = Vec::new();
    let mut pending: Option<Help> = None;
    let mut rest = rc;
    loop {
        let comment = rest.find("<!--");
        let keybind = rest.find(KEYBIND);
        match (comment, keybind) {
            (Some(c), k) if k.map_or(true, |k| c < k) => {
                let body_from = c + "<!--".len();
                let Some(len) = rest[body_from..].find("-->") else { break };
                pending = help_in(&rest[body_from..body_from + len]);
                rest = &rest[body_from + len + "-->".len()..];
            }
            (_, Some(k)) => {
                let after = &rest[k + KEYBIND.len()..];
                let Some(end) = after.find('"') else { break };
                let spec = &after[..end];
                let mut pieces: Vec<&str> = spec.split('-').collect();
                let key = pieces.pop().unwrap_or_default();
                let mut modifiers = BTreeSet::new();
                for m in pieces {
                    match m {
                        "W" => modifiers.insert("super"),
                        "C" => modifiers.insert("ctrl"),
                        "A" => modifiers.insert("alt"),
                        "S" => modifiers.insert("shift"),
                        other => panic!("rc.xml binds {spec}: unknown modifier {other:?}"),
                    };
                }
                out.push(Keybind {
                    spec: spec.to_string(),
                    canonical: canonical(&modifiers, key),
                    help: pending.take(),
                });
                rest = &after[end..];
            }
            _ => break,
        }
    }
    out
}

/// How one canonical key is written on a cap.
fn cap(word: &str) -> String {
    let named = match word {
        "super" => "Super",
        "ctrl" => "Ctrl",
        "alt" => "Alt",
        "shift" => "Shift",
        "left" => "←",
        "right" => "→",
        "up" => "↑",
        "down" => "↓",
        "space" => "Space",
        "escape" => "Esc",
        "return" => "Enter",
        "tab" => "Tab",
        "print" => "PrtSc",
        "xf86audioraisevolume" => "Volume up",
        "xf86audiolowervolume" => "Volume down",
        "xf86audiomute" => "Mute",
        "xf86audiomicmute" => "Mic mute",
        "xf86monbrightnessup" => "Brightness up",
        "xf86monbrightnessdown" => "Brightness down",
        _ => "",
    };
    if !named.is_empty() {
        return named.to_string();
    }
    // F11 and a letter or digit.
    word.to_uppercase()
}

/// The caps for a canonical shortcut: `super+shift+s` is Super, Shift, S.
fn caps(canonical: &str) -> Vec<String> {
    canonical.split('+').map(cap).collect()
}

/// The sheet for `rc`, or every binding that has no `@help` comment or names a group the sheet
/// does not draw. The shell never shows the error: the same check is a test on the shipped file.
pub(crate) fn sheet(rc: &str) -> Result<Vec<Row>, Vec<String>> {
    let mut rows = Vec::new();
    let mut problems = Vec::new();
    for bind in keybinds(rc) {
        let Some(help) = bind.help else {
            problems.push(format!("{} has no @help comment just before it", bind.spec));
            continue;
        };
        let Some(group) = GROUPS.iter().find(|g| **g == help.group) else {
            problems.push(format!("{} is in the group {:?}, which the sheet does not draw ({GROUPS:?})", bind.spec, help.group));
            continue;
        };
        if help.text.is_empty() {
            problems.push(format!("{} has an empty description", bind.spec));
            continue;
        }
        rows.push(Row { group, caps: caps(&bind.canonical), text: help.text, canonical: bind.canonical });
    }
    if problems.is_empty() {
        // Sections in the sheet's order; inside one, the file's.
        rows.sort_by_key(|r| GROUPS.iter().position(|g| *g == r.group));
        Ok(rows)
    } else {
        Err(problems)
    }
}

/// The shipped sheet. Never empty in a build that passed its tests; if the file were broken the
/// sheet would show nothing rather than a wrong list, and [`shipped_problems`] says why.
pub(crate) fn shipped() -> Vec<Row> {
    sheet(SHIPPED_RC).unwrap_or_default()
}

/// What is wrong with the shipped rc.xml, for the log; empty when every binding is described.
pub(crate) fn shipped_problems() -> Vec<String> {
    sheet(SHIPPED_RC).err().unwrap_or_default()
}

/// The rows a search leaves: a row stays when every word typed is in its description, its group
/// or its keys. Case does not matter. An empty search leaves all of them.
pub(crate) fn matching<'a>(rows: &'a [Row], query: &str) -> Vec<&'a Row> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    rows.iter()
        .filter(|row| {
            let haystack = format!("{} {} {}", row.text, row.group, row.caps.join(" ")).to_lowercase();
            words.iter().all(|w| haystack.contains(w.as_str()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_help_comment_belongs_to_the_binding_right_after_it() {
        let rc = r#"
            <!-- @help group="Windows" text="Close the window" -->
            <keybind key="A-F4"></keybind>
            <!-- @help group="Shell" text="Lock" -->
            <!-- a second comment cuts the tie -->
            <keybind key="W-l"></keybind>
            <keybind key="W-e"></keybind>
            <!-- <keybind key="W-n"> a reserved key, only talked about -->
        "#;
        let binds = keybinds(rc);
        assert_eq!(binds.len(), 3, "a keybind inside a comment is not a binding");
        assert_eq!(binds[0].help, Some(Help { group: "Windows".into(), text: "Close the window".into() }));
        assert_eq!(binds[0].canonical, "alt+f4");
        assert_eq!(binds[1].help, None, "the comment between them cut the tie");
        assert_eq!(binds[2].help, None, "a help comment describes one binding, not the rest of the file");
    }

    #[test]
    fn a_binding_without_help_is_reported_by_name() {
        let rc = r#"<!-- @help group="Windows" text="Close" --><keybind key="A-F4"></keybind><keybind key="W-slash"></keybind>"#;
        let problems = sheet(rc).unwrap_err();
        assert_eq!(problems.len(), 1);
        assert!(problems[0].starts_with("W-slash has no @help"), "{problems:?}");
    }

    #[test]
    fn a_group_the_sheet_does_not_draw_is_reported() {
        let rc = r#"<!-- @help group="Misc" text="Something" --><keybind key="W-x"></keybind>"#;
        assert!(sheet(rc).unwrap_err()[0].contains("does not draw"));
    }

    #[test]
    fn keys_are_drawn_as_caps() {
        assert_eq!(caps("super+shift+s"), ["Super", "Shift", "S"]);
        assert_eq!(caps("super+/"), ["Super", "/"]);
        assert_eq!(caps("super+alt+left"), ["Super", "Alt", "←"]);
        assert_eq!(caps("alt+f4"), ["Alt", "F4"]);
        assert_eq!(caps("xf86audioraisevolume"), ["Volume up"]);
        assert_eq!(caps("super+escape"), ["Super", "Esc"]);
    }

    #[test]
    fn search_matches_words_in_any_order_across_text_group_and_keys() {
        let rows = sheet(
            r#"<!-- @help group="Capture" text="Screenshot of a region" --><keybind key="W-S-s"></keybind>
               <!-- @help group="Windows" text="Close the window" --><keybind key="A-F4"></keybind>"#,
        )
        .unwrap();
        assert_eq!(matching(&rows, "").len(), 2);
        assert_eq!(matching(&rows, "REGION shot").len(), 1);
        assert_eq!(matching(&rows, "shift").len(), 1, "keys are searchable");
        assert_eq!(matching(&rows, "capture").len(), 1, "so is the group");
        assert!(matching(&rows, "nonsense").is_empty());
    }

    /// The sheet is the shipped rc.xml and nothing else: every key the compositor is given is on
    /// it, and nothing is on it that the compositor is not given. A binding without a description
    /// fails here, naming the binding, so adding a key to rc.xml means saying what it does.
    #[test]
    fn the_sheet_lists_exactly_the_keys_rc_xml_binds() {
        let rows = sheet(SHIPPED_RC).unwrap_or_else(|problems| {
            panic!("every keybind in config/labwc/rc.xml needs a `<!-- @help group=\"...\" text=\"...\" -->` comment just before it:\n{}", problems.join("\n"))
        });
        let listed: BTreeSet<String> = rows.iter().map(|r| r.canonical.clone()).collect();
        let bound = crate::rc_keys::bound_keys(SHIPPED_RC);
        assert_eq!(listed.len(), rows.len(), "a key is on the sheet twice");
        assert_eq!(
            listed.difference(&bound).collect::<Vec<_>>(),
            Vec::<&String>::new(),
            "the sheet lists keys rc.xml does not bind"
        );
        assert_eq!(
            bound.difference(&listed).collect::<Vec<_>>(),
            Vec::<&String>::new(),
            "rc.xml binds keys the sheet does not list"
        );
        assert!(bound.contains("super+/"), "Super+/ opens the sheet and must be bound");
        assert!(rows.len() >= 40, "only {} rows; the scan is reading the wrong thing", rows.len());
    }

    #[test]
    fn every_binding_has_a_group_and_a_plain_description_and_every_group_is_used() {
        for bind in keybinds(SHIPPED_RC) {
            let help = bind.help.unwrap_or_else(|| panic!("{} has no @help comment", bind.spec));
            assert!(GROUPS.contains(&help.group.as_str()), "{}: unknown group {:?}", bind.spec, help.group);
            assert!(help.text.len() >= 4, "{}: description {:?} says nothing", bind.spec, help.text);
            assert!(!help.text.ends_with('.'), "{}: descriptions are phrases, no full stop: {:?}", bind.spec, help.text);
        }
        let rows = shipped();
        for group in GROUPS {
            assert!(rows.iter().any(|r| r.group == group), "nothing in {group}");
        }
    }

    /// Super+/ must run the action the shell publishes for it.
    #[test]
    fn super_slash_opens_the_sheet_through_the_control_surface() {
        let at = SHIPPED_RC.find("<keybind key=\"W-slash\">").expect("Super+/ is bound");
        let binding = &SHIPPED_RC[at..SHIPPED_RC[at..].find("</keybind>").unwrap() + at];
        assert!(binding.contains("yos act shell open_cheat_sheet"), "{binding}");
    }
}
