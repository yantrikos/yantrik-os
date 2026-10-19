//! Guard: no instinct may claim feelings it can't have or manufacture obligation (#755).
//!
//! A source scan over every `.rs` file in `src/`, so the rule holds for copy that has not been
//! written yet. Case-insensitive, with typographic apostrophes folded to ASCII.

use std::fs;
use std::path::Path;

/// Phrases instinct copy must never contain.
///
/// From `design/companion-who-cares-2026-10-11.md` ("Never": claimed feelings, neediness and
/// relationship theatre) and `design/ux-psychology-2026-10-11.md` ("What we will not build": a
/// mind that pretends feelings to create obligation). Evidence, never testimony: the companion
/// shows what it kept and did; it never says what it felt, and the person is never responsible
/// for its wellbeing.
const BANNED_PHRASES: &[&str] = &[
    // "missed you" also catches "I've missed you" and "I really missed you".
    "missed you",
    "i was worried",
    "worried about you",
    "i care about you",
    "genuinely care",
    // Catches "I was / I've been / I keep thinking about you".
    "thinking about you",
    "no take-backs",
    "stuck with me",
    "i'm lonely",
    "i was lonely",
    "don't leave",
    "you haven't talked to me",
    "we're in this together",
];

fn instinct_sources() -> Vec<(String, String)> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files: Vec<(String, String)> = fs::read_dir(&src)
        .expect("src/ is readable")
        .map(|entry| entry.expect("a directory entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .map(|path| {
            let text = fs::read_to_string(&path).expect("an instinct source is readable");
            (path.file_name().unwrap().to_string_lossy().into_owned(), text)
        })
        .collect();
    files.sort();
    files
}

fn normalise(text: &str) -> String {
    text.to_lowercase().replace(['\u{2019}', '\u{2018}'], "'")
}

#[test]
fn no_instinct_claims_feelings_or_manufactures_obligation() {
    let sources = instinct_sources();
    assert!(sources.len() > 40, "the scan found the instinct sources ({})", sources.len());

    let mut found = Vec::new();
    for (file, text) in &sources {
        let text = normalise(text);
        for phrase in BANNED_PHRASES {
            if text.contains(phrase) {
                found.push(format!("{file}: \"{phrase}\""));
            }
        }
    }
    assert!(
        found.is_empty(),
        "instinct copy claims feelings or manufactures obligation (#755):\n{}",
        found.join("\n")
    );
}

#[test]
fn the_guard_catches_what_it_bans() {
    // The retired bond_milestone line, as it shipped.
    let line = normalise("I realize I genuinely care about what happens to you. You\u{2019}re stuck with me now.");
    assert!(BANNED_PHRASES.iter().any(|p| line.contains(p)));
    // Variants of the same claims, with a typographic apostrophe.
    for line in ["I\u{2019}ve missed you.", "I\u{2019}ve been thinking about you all day."] {
        let line = normalise(line);
        assert!(BANNED_PHRASES.iter().any(|p| line.contains(p)), "{line}");
    }
}
