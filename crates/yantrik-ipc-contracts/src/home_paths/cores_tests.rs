//! `secret_cores` is a necessary condition of `is_secret_name`: a name that holds no core is
//! never a secret's name or a copy of one. A filter that skips the runs of a long title without one
//! is only as right as this (#1247, gate 12, the cost of a page's title).

use super::{is_secret_name, secret_cores, SECRET_NAMES, SECRET_PREFIXES, SECRET_SUFFIXES};

fn holds_a_core(name: &str) -> bool {
    let lowered = name.to_lowercase();
    secret_cores().iter().any(|core| lowered.as_bytes().windows(core.len()).any(|w| w == core.as_slice()))
}

/// Every way a name is marked as a copy, around a stem and an extension.
fn marked(stem: &str, ext: &str) -> Vec<String> {
    let mut forms = Vec::new();
    for mid in ["", " (copy)", " (copy 1)", " (1)", " (2)", " - Copy", " - copy", " copy", " copy 2", " 2", " (another copy)", " (3rd copy)",
        " (Ann's conflicted copy 2026-11-15)", " (case clash 1)", ".sync-conflict-20261115-123456-ABCDEFG", " (conflicted copy 1)"] {
        forms.push(format!("{stem}{mid}{ext}"));
        for end in [".bak", ".old", "~", ".swp", ".save", ".1", ".bak2", "~1~", ":Zone.Identifier", ".tmp", "#", ".un~", ".part"] {
            forms.push(format!("{stem}{mid}{ext}{end}"));
        }
        for front in [".", ".#", "copy of ", ".~lock.", "#", "%home%ann%", "!home!ann!", "%home%ann%.config%", "x:", "backup:"] {
            forms.push(format!("{front}{stem}{mid}{ext}"));
            forms.push(format!("{front}{stem}{mid}{ext}.swp"));
            forms.push(format!("{front}{stem}{mid}{ext}#"));
        }
    }
    forms
}

#[test]
fn a_secret_name_or_a_copy_of_one_holds_a_core() {
    let mut judged = 0;
    let mut stems: Vec<(String, String)> = Vec::new();
    for name in SECRET_NAMES {
        match name.rfind('.').filter(|&at| at > 0) {
            Some(at) => stems.push((name[..at].to_string(), name[at..].to_string())),
            None => stems.push((name.to_string(), String::new())),
        }
    }
    for suffix in SECRET_SUFFIXES {
        stems.push(("server".into(), suffix.to_string()));
        stems.push(("my backup key".into(), suffix.to_string()));
    }
    for prefix in SECRET_PREFIXES {
        for tail in ["", "_123.json", ".bak", ".local", "_2026-11-15.json", ".pub.bak"] {
            stems.push((format!("{prefix}{}", tail.split('.').next().unwrap_or("")), tail.find('.').map(|at| tail[at..].to_string()).unwrap_or_default()));
        }
    }
    for (stem, ext) in &stems {
        for form in marked(stem, ext) {
            judged += 1;
            if is_secret_name(&form) {
                assert!(holds_a_core(&form), "{form:?} is a secret's name and holds no core");
            }
        }
    }
    assert!(judged > 10_000, "{judged}");
}

/// Every quoted name in the rule's own tests, whatever the test says of it.
#[test]
fn no_name_in_the_rules_tests_is_a_secret_without_a_core() {
    let mut seen = 0;
    for source in [include_str!("copy_form_tests.rs"), include_str!("secret_tests.rs"), include_str!("tests.rs")] {
        for literal in source.split('"').skip(1).step_by(2) {
            if literal.len() < 200 && is_secret_name(literal) {
                seen += 1;
                assert!(holds_a_core(literal), "{literal:?}");
            }
        }
    }
    for name in include_str!("regression_names.txt").lines().filter_map(|l| l.strip_prefix("secret\t")) {
        seen += 1;
        assert!(holds_a_core(name), "{name:?}");
    }
    assert!(seen > 100, "{seen}");
}

#[test]
fn a_lowered_name_is_read_the_way_the_rule_reads_it() {
    // `K` (Kelvin) and `İ` lower-case to ASCII letters; the rule lower-cases, so a title with one
    // cannot be skipped by an ASCII scan.
    assert!(is_secret_name("tokens.json") && is_secret_name("TOKENS.JSON"));
    assert!(is_secret_name("\u{212A}ey4.db") == holds_a_core("\u{212A}ey4.db"));
}
