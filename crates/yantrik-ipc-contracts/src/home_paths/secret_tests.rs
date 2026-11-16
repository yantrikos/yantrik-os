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

/// Review M4 of #1247: each name below was readable before the lists grew.
#[test]
fn every_credential_the_review_lists_is_refused_by_name() {
    for name in [
        // Key formats.
        "putty.ppk", "key.p8", "service.keytab", "backup.age", "vault.kdb", "login.keychain", "login.keychain-db",
        // Cloud and CLI credentials copied out of their folders.
        "application_default_credentials.json", "access_tokens.db", "credentials.db", "kubeconfig", "hosts.yml",
        ".s3cfg", ".boto", "credentials.tfrc.json",
        // Other dotfiles.
        ".envrc", ".my.cnf", ".htpasswd", ".authinfo", ".authinfo.gpg",
        // Browser stores.
        "key3.db", "signons.sqlite", "Local State", "Login Data For Account", "Cookies-journal", "Login Data-journal",
        // `id_dsa` was missing from the starts.
        "id_dsa", "id_dsa.bak",
    ] {
        assert!(is_secret_name(name), "{name}");
        assert!(is_protected(Path::new(&format!("/home/ann/old/{name}"))), "{name}");
    }
}

#[test]
fn a_backup_copy_of_a_credential_is_that_credential() {
    for name in [
        "tokens.json.bak", "tokens (1).json", "credentials.json.bak", "auth.json.old", "id_dsa.bak", "tokens.json.orig",
        "tokens.json.backup", "tokens.json.copy", "tokens.json~", "TOKENS (12).JSON", "tokens (2).json.bak", ".env.bak",
        "kubeconfig.old", "Local State.bak",
    ] {
        assert!(is_secret_name(name), "{name}");
        assert!(is_protected(Path::new(&format!("/home/ann/backup/{name}"))), "{name}");
    }
}

/// Review N1 of #1247: editor swap and recovery files, package-manager files and the copy names
/// that other systems' file managers give. Each form, for a `.json` and a `.pem` credential.
#[test]
fn every_copy_form_of_a_credential_is_that_credential() {
    for original in ["tokens.json", "server.pem"] {
        let (stem, ext) = original.split_once('.').unwrap();
        let mut forms: Vec<String> = [
            ".save", ".swp", ".swo", ".tmp", ".dpkg-old", ".dpkg-dist", ".rpmsave", ".rpmnew", ".prev", ".1", ".2", ".12",
            ".~1~", ".~12~", ".bak2", ".bak.1", ".bak.bak", ".SAVE", " (copy)", " (another copy)", " - Copy", " - copy (2)",
        ]
        .iter()
        .map(|end| format!("{original}{end}"))
        .collect();
        forms.extend([
            format!(".{original}.swp"), format!(".{original}.swo"), format!("#{original}#"), format!(".#{original}"),
            format!("{stem} (copy).{ext}"), format!("{stem} (another copy).{ext}"), format!("{stem} (3rd copy).{ext}"),
            format!("{stem} (2nd copy).{ext}"), format!("{stem} (11th copy).{ext}"), format!("{stem} - Copy.{ext}"),
            format!("{stem} - Copy (2).{ext}"), format!("{stem} copy.{ext}"), format!("{stem} copy 2.{ext}"),
            format!("{stem}(1).{ext}"), format!("{stem} (1).{ext}"), format!("Copy of {original}"),
            format!("Copy of {stem} (2).{ext}"), format!("{original}.bak (1)"),
        ]);
        for name in forms {
            assert!(is_secret_name(&name), "{name}");
            assert!(is_protected(Path::new(&format!("/home/ann/backup/{name}"))), "{name}");
        }
    }
    // A dotfile's own swap file keeps its dot: `.env.swp` is `.env`.
    for name in [".env.swp", ".env.save", ".netrc.2", ".#.env"] {
        assert!(is_secret_name(name), "{name}");
    }
}

/// What looks like a marker on something that is no credential stays readable (review N1).
#[test]
fn the_new_copy_forms_leave_their_decoys_readable() {
    for name in [
        "notes copy.md", "photo (copy).jpg", "chapter.2.md", "report.json.1.txt", "copy of notes.md", "swap.swp", "notes.save",
        "notes (another copy).md", "notes - Copy.md", "notes.~1~", "#notes#", ".#notes", ".notes.swp", "notes.dpkg-old",
        "tokens.json.1.txt", "tokens.json (copy).txt", "copy of tokens-notes.md", "tokens copy notes.json", "tokens.2x",
        "mytokens.json.2", "tokens.json.bakx", "tokens.json.2b", "tokens.jsonn.save",
    ] {
        assert!(!is_secret_name(name), "{name}");
        assert!(!is_protected(Path::new(&format!("/home/ann/Documents/{name}"))), "{name}");
    }
}

#[test]
fn the_oldest_copy_forms_still_hold() {
    for name in [
        "tokens.json.bak",
    ] {
        assert!(is_secret_name(name), "{name}");
        assert!(is_protected(Path::new(&format!("/home/ann/backup/{name}"))), "{name}");
    }
}

/// Only an ending or a number before the extension makes a copy: a word in the middle does not.
#[test]
fn what_only_looks_like_a_copy_stays_readable() {
    for name in [
        "notes.bak.md", "old-tokens-notes.txt", "tokens.md.bak", "tokens (a).json", "tokens ().json", "tokens (1)x.json",
        "keynote.txt", "id_rsa.pub", "id_rsa.pub.bak", "id_dsa.pub", "api.key.txt", "notes (1).md", ".bak", "~", "image.agent",
        "hosts.yaml", "state.json", "local-state.md",
    ] {
        assert!(!is_secret_name(name), "{name}");
        assert!(!is_protected(Path::new(&format!("/home/ann/Documents/{name}"))), "{name}");
    }
}

#[test]
fn gpgs_private_key_folder_is_protected_wherever_it_is_copied() {
    for p in ["/home/ann/.gnupg/private-keys-v1.d/x.key", "/home/ann/backup/private-keys-v1.d/abc", "/home/ann/backup/private-keys-v1.d"] {
        assert!(is_protected(Path::new(p)), "{p}");
    }
    assert!(!is_protected(Path::new("/home/ann/backup/private-keys-v1.d-notes.md")));
}

/// Plain `*.gpg` and `*.asc` are ciphertext or public keys more often than not (the review).
#[test]
fn ciphertext_and_public_keys_stay_readable() {
    for name in ["report.gpg", "public.asc", "release.agekey.md"] {
        assert!(!is_secret_name(name), "{name}");
    }
}

#[test]
fn a_search_for_a_backup_copy_is_a_search_for_the_secret() {
    for asked in ["tokens.json.bak", "tokens (1).json", "*.ppk", "id_dsa*", "kubeconfig"] {
        assert!(pattern_asks_for_a_secret(asked), "{asked}");
    }
}

/// Review N4 of #1247: the refusal to read a folder whole does not name the protected child.
#[test]
fn a_folder_with_a_secret_inside_is_refused_without_naming_it() {
    let (_d, home) = home();
    for rel in ["credentials.json", "deep/er/tokens (1).json", "keys/private-keys-v1.d/ABC.key"] {
        put(&home, &format!("tree/{rel}"));
        let refused = may_read_tree(&home.join("tree")).unwrap_err();
        let last = rel.rsplit('/').next().unwrap();
        assert!(refused.contains("something protected is inside"), "{rel}: {refused}");
        assert!(!refused.contains(last) && !refused.contains("private-keys"), "{rel}: {refused}");
        std::fs::remove_dir_all(home.join("tree")).unwrap();
    }
}

/// Gate 12: the person's old harness folder, kept by an update as `<home>.yantrik-moved` or by hand
/// under any other suffix, is as protected as the live one: every file inside it, whatever the suffix,
/// and the folder itself when its name reads as a copy. A name that only starts like one is not.
#[test]
fn a_harness_home_is_protected_under_any_suffix() {
    for home in [".hermes", ".openclaw", ".pi", ".claude"] {
        for suffix in ["", ".yantrik-moved", ".bak", "-old", "_2", "~", ".2026-11-15", "2", ".anything.at-all"] {
            // `.pi` is also the start of ordinary names (N-L3): only a backup suffix makes it a copy.
            if home == ".pi" && ["2", ".anything.at-all"].contains(&suffix) {
                continue;
            }
            for file in ["config.yaml", "sessions/a.json", "outside-home/hermes.db"] {
                let p = format!("/home/ann/{home}{suffix}/{file}");
                assert!(is_protected(Path::new(&p)), "{p}");
            }
        }
        for copy in [".yantrik-moved", ".bak", ".old", ".backup", "~"] {
            let p = format!("/home/ann/{home}{copy}");
            assert!(is_protected(Path::new(&p)), "the folder itself: {p}");
        }
    }
    assert!(is_protected(Path::new("/home/ann/.claude.json.bak")), "a copy of the file the list names");
    assert!(is_protected(Path::new("/home/ann/.claude.json.yantrik-moved")));
    for fine in [".pi-hole/x.conf", ".pi2/x", ".pi.anything/x", ".pip/cache/x", ".pidgin/accounts.xml", ".pki/nssdb/x", ".hermesx/x", ".claudette/x", "pi/x", "hermes/x", "pictures/.pi-day.png", ".pi-day.png"] {
        let p = format!("/home/ann/{fine}");
        assert!(!is_protected(Path::new(&p)), "{p}");
    }
}

/// Gate 12: a repository's object store holds every committed file; the git tools are the way in.
#[test]
fn a_git_object_store_is_protected_and_the_rest_of_a_repository_is_not() {
    for p in [
        "/home/ann/proj/.git/objects/0b/0123456789abcdef", "/home/ann/proj/.git/objects/pack/pack-1.pack", "/home/ann/proj/.git/objects",
        "/home/ann/proj/.git/modules/sub/objects/ab/cd", "/home/ann/bare.git/objects/ab/cd", "/home/ann/proj/.GIT/OBJECTS/ab/cd",
    ] {
        assert!(is_protected(Path::new(p)), "{p}");
    }
    for p in ["/home/ann/proj/.git/HEAD", "/home/ann/proj/.git/config", "/home/ann/proj/objects/notes.md", "/home/ann/proj/src/main.rs", "/home/ann/objects/x"] {
        assert!(!is_protected(Path::new(p)), "{p}");
    }
}

/// R11-M2: a copy of a protected folder is protected, whoever made it and however a file manager names
/// it: GNOME Files `.hermes (copy)`, Finder `.hermes copy`, a person's `.kube.bak`; one rule for every
/// listed place, the harness homes and the rest.
#[test]
fn a_copy_of_a_protected_folder_is_protected_under_any_marker() {
    for p in [
        "/home/ann/.hermes (copy)/config.yaml", "/home/ann/.hermes copy/config.yaml", "/home/ann/.kube.bak/config", "/home/ann/.kube (copy)/config",
        "/home/ann/.docker.bak/config.json", "/home/ann/.config/labwc (copy)/autostart", "/home/ann/.ssh~/id_x", "/home/ann/.hermes-2026",
        "/home/ann/.hermes (copy)", "/home/ann/.kube.bak",
        // N-M3: a folder copied by hand under a backup word or a date.
        "/home/ann/.kube-old/config", "/home/ann/.kube_old/config", "/home/ann/.kube-backup/config", "/home/ann/.kube.2026-11-15/config", "/home/ann/.docker-old/config.json",
    ] {
        assert!(is_protected(Path::new(p)), "{p}");
        assert!(is_listed_place(Path::new(p)), "the text rule alone, for a title: {p}");
    }
    for fine in ["/home/ann/.kube-tools/readme.md", "/home/ann/.pip/pip.conf", "/home/ann/.hermesx/notes.md", "/home/ann/.kubernetes/x", "/home/ann/.docker/other.json", "/home/ann/.sshd/x", "/home/ann/.kubectl/x", "/home/ann/.docker-compose.yml", "/home/ann/Pictures/Screenshots-old/a.png"] {
        assert!(!is_protected(Path::new(fine)), "{fine}");
    }
    assert!(title_names_a_secret("config.yaml (~/.hermes-2026) - gedit"), "L1: the title names a kept harness folder");
}

/// R11-L2: what else holds a repository's contents or names, and what is not an object store.
#[test]
fn a_bundle_an_index_and_a_pack_in_flight_are_protected_and_a_folder_named_objects_is_not() {
    for p in [
        "/home/ann/gate/proj.bundle", "/home/ann/proj/.git/index", "/home/ann/bare.git/index", "/home/ann/gitdirs/p/objects/pack/tmp_pack_AbC123",
        "/home/ann/gitdirs/p/objects/incoming-xyz/ab", "/home/ann/proj/.git/modules/a/b/objects/pack/pack-1.pack",
        "/home/ann/proj/.git/index.lock", "/home/ann/proj/.git/modules/m/index",
    ] {
        assert!(is_protected(Path::new(p)), "{p}");
    }
    for fine in ["/home/ann/src/dotfiles.git/src/objects/model.py", "/home/ann/proj/.git/HEAD", "/home/ann/proj/.git/config", "/home/ann/games/objects/player.rs", "/home/ann/proj/index",
        // N-L3: an `objects` folder deeper under `modules/`, and Ruby's `.bundle/` folder.
        "/home/ann/src/x.git/modules/m/src/objects/model.py", "/home/ann/proj/.bundle/config", "/home/ann/proj/.bundle"] {
        assert!(!is_protected(Path::new(fine)), "{fine}");
    }
}

/// R11-L5: the installers' markers are endings, not any word inside a name; and R11-H1: the shells' and
/// REPLs' history files are protected places.
#[test]
fn a_kept_copy_marker_is_an_ending_and_every_history_file_is_a_place() {
    for fine in ["/home/ann/Documents/before-yantrik-notes.md", "/home/ann/Documents/why-yantrik-moved.txt", "/home/ann/Documents/before-yantrik.md"] {
        assert!(!is_protected(Path::new(fine)), "{fine}");
    }
    for p in ["/home/ann/cfg/config.yaml.before-yantrik", "/home/ann/.model-before-yantrik.json", "/home/ann/.hermes.yantrik-moved"] {
        assert!(is_protected(Path::new(p)), "{p}");
    }
    for h in [".ash_history", ".sh_history", ".local/share/zsh/history", ".sqlite_history", ".irb_history", ".rediscli_history"] {
        assert!(is_protected(Path::new(&format!("/home/ann/{h}"))), "{h}");
    }
}

/// N-L3: a project's own `.claude/` folder is ordinary; the one directly under a home is the harness's.
#[test]
fn a_projects_claude_folder_is_readable_and_the_one_in_the_home_is_not() {
    for fine in ["/home/ann/src/app/.claude/commands/fix.md", "/home/ann/src/app/.claude/settings.json", "/home/ann/src/app/.claude.json"] {
        assert!(!is_protected(Path::new(fine)), "{fine}");
        assert!(!title_names_a_secret(&format!("fix.md ({fine}) - gedit")), "{fine}");
    }
    for p in ["/home/ann/.claude/settings.json", "/home/ann/.claude.json", "/home/ann/.claude.bak/x", "/root/.claude/x"] {
        assert!(is_protected(Path::new(p)), "{p}");
    }
}
