//! #1247 review R6-L4: the window-title filter is under the same guard as the names. Every name the
//! rounds asserted is a title too: `<name> - gedit` is hidden from a mind for a credential and for
//! an editor's copy of a protected file, and whole for a decoy.

use super::*;

/// The file `yantrik-ipc-contracts` guards its names with, read here as it is.
const ROUNDS: &str = include_str!("../../yantrik-ipc-contracts/src/home_paths/regression_names.txt");

/// Decoy names that a title hides on purpose. Four read as `NAME: …`, a sentence that opens with a
/// credential's name, and `tokens.json (copy).txt` has the name as a word: a title cannot tell them
/// from `Opening tokens.json: done`. The file's own `readable` rule holds them readable; only the
/// title is cautious, and only for a mind.
const HIDDEN_ON_PURPOSE: [&str; 5] = [
    "credentials: how to rotate.md", "cookies: chocolate chip.md", "tokens.json: what it holds.md", "login data: export notes.txt",
    "tokens.json (copy).txt",
];

#[test]
fn every_name_any_round_asserted_is_judged_as_a_title_too() {
    let (mut hidden, mut whole) = (0, 0);
    for line in ROUNDS.lines().filter(|l| !l.starts_with('#') && !l.is_empty()) {
        let Some((kind, name)) = line.split_once('\t') else { panic!("a kind, a tab and a name: {line}") };
        let title = format!("{name} - gedit");
        match kind {
            "secret" | "encoded" => {
                hidden += 1;
                assert!(hides(&title, true), "a credential that a title shows: {title}");
                assert!(!hides(&title, false), "the person sees the title whole: {title}");
            }
            "readable" if HIDDEN_ON_PURPOSE.contains(&name) => {
                hidden += 1;
                assert!(hides(&title, true), "the exemption is stale: {title}");
            }
            "readable" => {
                whole += 1;
                assert!(!hides(&title, true), "a decoy that a title hides: {title}");
            }
            // Review R7-L3: a path is a title too, with the home the file spells as `/home/ann`.
            "protected" => {
                hidden += 1;
                let title = format!("{name} - nvim");
                assert!(hides(&title, true), "a protected path that a title shows: {title}");
                assert!(!hides(&title, false), "the person sees the title whole: {title}");
            }
            "open" => {
                whole += 1;
                let title = format!("{name} - nvim");
                assert!(!hides(&title, true), "a decoy path that a title hides: {title}");
            }
            _ => {}
        }
    }
    assert!(hidden > 200 && whole > 70, "both lists are read: {hidden} hidden, {whole} whole");
}
