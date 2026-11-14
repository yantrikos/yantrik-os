//! #1247: a credential copied out of its folder is refused by name, on every side.

use std::os::unix::fs::symlink;
use std::path::Path;

use super::tests::home;
use super::*;

/// Fake contents only; a refusal must never print them.
const FAKE: &str = "FAKE-SECRET-DO-NOT-PRINT";

fn put(home: &Path, rel: &str) {
    let at = home.join(rel);
    std::fs::create_dir_all(at.parent().unwrap()).unwrap();
    std::fs::write(at, FAKE).unwrap();
}

#[test]
fn every_exact_name_is_refused_wherever_it_is_and_in_any_case() {
    for name in SECRET_NAMES {
        for shown in [name.to_string(), name.to_uppercase()] {
            let p = format!("/home/ann/backup/{shown}");
            assert!(is_protected(Path::new(&p)), "{p}");
            assert!(is_secret_name(&shown), "{shown}");
        }
    }
}

#[test]
fn every_suffix_is_refused() {
    for end in SECRET_SUFFIXES {
        let p = format!("/home/ann/Downloads/server{end}");
        assert!(is_protected(Path::new(&p)), "{p}");
    }
}

#[test]
fn every_prefix_is_refused_with_a_copy_ending() {
    for start in SECRET_PREFIXES {
        for tail in ["", "_123.json", ".bak", ".local"] {
            let name = format!("{start}{tail}");
            assert!(is_secret_name(&name), "{name}");
        }
    }
}

#[test]
fn a_public_key_is_not_a_secret() {
    for name in ["id_ed25519.pub", "id_rsa.pub", "ID_ECDSA.PUB"] {
        assert!(!is_secret_name(name), "{name}");
    }
}

/// The rule is by exact name, ending or start, never by a word inside the name.
#[test]
fn decoys_stay_readable() {
    let (_d, home) = home();
    for rel in ["Documents/notes-about-tokens.md", "Documents/keynote.txt", "Documents/tokens.md", "old/id_ed25519.pub", "Documents/environment.txt", "Documents/monkey.txt", "Documents/pemberton.txt", "Documents/idea_rsa_notes.txt"] {
        put(&home, rel);
        assert_eq!(may_read_file(&format!("~/{rel}"), &home), Ok(()), "{rel}");
        assert!(!is_protected_in(&home.join(rel), Some(&home)), "{rel}");
    }
}

#[test]
fn the_issues_files_are_refused_through_the_editor_and_files_check() {
    let (_d, home) = home();
    for rel in [
        "backup/tokens.json", "x/.env", "Downloads/server.pem", "old/id_ed25519", "old/ID_RSA.bak",
        "Documents/client_secret_123.json", "hermes-copy/.hermes/config.yaml", ".mozilla/firefox/x/logins.json",
        "x/.env.local", "x/service-account-prod.json",
    ] {
        put(&home, rel);
        let refused = may_read_file(&format!("~/{rel}"), &home).unwrap_err();
        assert!(refused.ends_with(" is protected"), "{rel}: {refused}");
        assert!(may_write_file(&format!("~/{rel}"), &home).is_err(), "{rel}");
    }
}

#[test]
fn a_link_to_a_secret_is_that_secret() {
    let (_d, home) = home();
    put(&home, "backup/tokens.json");
    symlink(home.join("backup/tokens.json"), home.join("notes.txt")).unwrap();
    assert!(is_protected_in(&home.join("notes.txt"), Some(&home)));
    assert!(may_read_file("~/notes.txt", &home).is_err());
}

#[test]
fn the_browser_and_mail_profiles_are_protected_for_every_side() {
    for p in [
        ".mozilla/firefox/x/places.sqlite", ".config/chromium/Default/History", ".config/google-chrome/Default/x",
        ".config/BraveSoftware/Brave-Browser/x", ".var/app/com.google.Chrome/config/x",
        ".var/app/org.chromium.Chromium/x", ".var/app/com.brave.Browser/x", "snap/chromium/common/x",
        "snap/firefox/common/x", ".thunderbird/p/prefs.js", ".config/evolution/sources/x",
    ] {
        let at = format!("/home/ann/{p}");
        assert!(is_protected(Path::new(&at)), "{at}");
    }
    assert!(!is_protected(Path::new("/home/ann/.config/chromium-notes/x")));
}

/// The refusal names the rule and the path, and never the contents of the file.
#[test]
fn the_refusal_names_the_rule_and_never_prints_the_contents() {
    let (_d, home) = home();
    put(&home, "backup/tokens.json");
    let refused = may_read_file("~/backup/tokens.json", &home).unwrap_err();
    assert!(refused.contains("protected places") && refused.ends_with(" is protected"), "{refused}");
    assert!(!refused.contains(FAKE));
}

#[test]
fn a_search_pattern_is_refused_by_what_it_asks_for() {
    for asked in ["tokens.json", "TOKENS.JSON", "*.pem", "id_*", ".env*", "client_secret*", "id_rs?", "*.kdbx"] {
        assert!(pattern_asks_for_a_secret(asked), "{asked}");
    }
    for fine in ["*.csv", "*", "*.json", "notes*", "tokens.md", "keynote.txt", "i*", "*token*", "*.pub"] {
        assert!(!pattern_asks_for_a_secret(fine), "{fine}");
    }
}
