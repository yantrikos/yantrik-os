//! #1247 round 3: the copies that editors, browsers and sync tools make by themselves.

use super::*;

/// Review R3-1: swap, undo and backup files kept away from the file, or ending in a letter or a
/// word the earlier lists missed. Each form, for a `.json` credential, a `.pem` one and a key.
#[test]
fn an_editors_own_copies_are_the_credential() {
    for original in ["tokens.json", "server.pem", "id_ed25519"] {
        let forms = [
            format!("%home%ann%backup%{original}.swp"), format!("%home%ann%backup%{original}"), format!("%home%ann%.ssh%{original}.swp"),
            format!("!home!ann!backup!{original}~"), format!(".{original}.swn"), format!(".{original}.swm"), format!(".{original}.swa"),
            format!(".{original}.un~"), format!("{original}.un~"), format!(".{original}.kate-swp"), format!("{original}___jb_old___"),
            format!("{original}___jb_tmp___"), format!("%home%ann%{original}.un~"),
        ];
        for name in forms {
            assert!(is_secret_name(&name), "{name}");
            assert!(is_protected(Path::new(&format!("/home/ann/.local/state/nvim/swap/{name}"))), "{name}");
        }
    }
}

/// Review R3-2: what a sync tool, a browser or Finder makes.
#[test]
fn a_sync_tools_browsers_and_finders_copies_are_the_credential() {
    for original in ["tokens.json", "server.pem"] {
        let (stem, ext) = original.split_once('.').unwrap();
        let mut forms: Vec<String> =
            [".part", ".partial", ".crdownload", ".filepart", ".download", ".sav", ".bk", "~1", "~12", ".backup1", ".sync-conflict-20261115-123456-ABCDEFG"]
                .iter()
                .map(|end| format!("{original}{end}"))
                .collect();
        forms.extend([
            format!("{stem} (conflicted copy 2026-11-15).{ext}"), format!("{stem} (Ann's conflicted copy 2026-11-15).{ext}"),
            format!("{stem} (ann's conflicted copy).{ext}"), format!("{stem}.sync-conflict-20261115-123456-ABCDEFG.{ext}"),
            format!("{stem} 2.{ext}"), format!("{stem} 12.{ext}"), format!("{original}.SYNC-CONFLICT-20261115-123456-ABCDEFG"),
        ]);
        for name in forms {
            assert!(is_secret_name(&name), "{name}");
            assert!(is_protected(Path::new(&format!("/home/ann/backup/{name}"))), "{name}");
        }
    }
}

/// Review R3-5: names that give the name away.
#[test]
fn a_windows_stream_and_a_libreoffice_lock_name_the_credential() {
    for name in ["tokens.json:Zone.Identifier", "server.pem:Zone.Identifier", ".~lock.tokens.json#", ".~lock.server.pem#"] {
        assert!(is_secret_name(name), "{name}");
    }
}

#[test]
fn the_round_three_forms_leave_their_decoys_readable() {
    for name in [
        "%home%ann%notes.md.swp", "!home!ann!notes.md~", "swap.swn", ".notes.md.un~", "notes.part", "photo (conflicted copy 2026-11-15).jpg",
        "chapter 2.md", "notes.sync-conflict-20261115-123456-ABCDEFG.md", "notes.md:Zone.Identifier", ".~lock.notes.md#", "notes.kate-swp",
        "notes___jb_old___", "notes.crdownload", "notes~1", "chapter 2", "tokens.sync-conflict-2026-123456-ABCDEFG.md", "tokens.json.swq",
        "tokens.json.swpp", "tokens.json.sync-conflict-20261115-123456-ABC", "tokens (conflicted copy of nothing).md", "notes.backup1",
        "id_rsa.pub.part", "id_rsa.pub.bak", "100% done.md", "credentials: how to rotate.md", "cookies: chocolate chip.md",
        "tokens.json: what it holds.md", "login data: export notes.txt", "photo (copy 1).jpg", "notes (conflicted copy ann 2026-11-15 101530).md",
        "notes-10:30:00.md", "notes.trashinfo", "notes.md.svz",
        // Review M1: ordinary three-letter endings on a bare secret name; L1: a URL-encoded space.
        "credentials.svg", "credentials.sql", "credentials.sig", "credentials.sha", "credentials.srt", "credentials.ssh", "cookies.svg",
        "cookies.sql", "cookies.sig", "kubeconfig.svg", "kubeconfig.sig", "cookies%20recipe.pdf", "credentials%20how-to.md",
        "cookies%2Fchocolate.md", "%home%ann%notes.md.swp", "%home%ann%documents%config.swp",
    ] {
        assert!(!is_secret_name(name), "{name}");
        assert!(!is_protected(Path::new(&format!("/home/ann/Documents/{name}"))), "{name}");
    }
}

/// Review H1: a name with a colon, a `%` or a leading `!` is judged as it stands as well as by the
/// part a copy rule keeps, so no copy rule makes a key readable.
#[test]
fn a_key_with_a_colon_or_a_percent_in_its_name_is_still_the_credential() {
    for name in [
        "deploy-2026-11-15T10:30:00.pem", "notes:server.pem", "id_rsa%20old", ".env%20prod", "client_secret_1%2B.json", "backup:id_ed25519",
        "!home!ann!backup!id_rsa", "tokens.json:Zone.Identifier", "tokens.json::$DATA", "TOKENS.JSON:zone.identifier",
    ] {
        assert!(is_secret_name(name), "{name}");
        assert!(is_protected(Path::new(&format!("/home/ann/backup/{name}"))), "{name}");
    }
}

/// Review M1: Nextcloud's conflict name ends in a date and a time, Thunar's copy in a number.
#[test]
fn nextcloud_and_thunar_copies_are_the_credential() {
    for name in [
        "tokens (conflicted copy 2026-11-15 101530).json", "tokens (conflicted copy ann 2026-11-15 101530).json",
        "server (conflicted copy 2026-11-15 101530).pem", "tokens (copy 1).json", "tokens (copy 12).json", "server (copy 1).pem",
        "tokens (case clash 1).json",
    ] {
        assert!(is_secret_name(name), "{name}");
        assert!(is_protected(Path::new(&format!("/home/ann/backup/{name}"))), "{name}");
    }
}

/// Review L2 and L7: the trash's record of a name, and Vim's swap names past `.swa`.
#[test]
fn a_trash_record_and_vims_later_swap_names_are_the_credential() {
    for name in ["tokens.json.trashinfo", "server.pem.trashinfo", ".tokens.json.svz", ".tokens.json.sva", ".tokens.json.suz", ".tokens.json.saa"] {
        assert!(is_secret_name(name), "{name}");
    }
}

/// Review M1: Vim's swap range past `.swp` is for a dotfile's own swap and for a path spelled into a
/// name; a bare name keeps `.swp`, `.swo` and `.swn` only.
#[test]
fn vims_wide_swap_range_is_for_dotted_and_encoded_names() {
    for name in [".tokens.json.svz", ".tokens.json.saa", "%home%ann%backup%tokens.json.svz", "tokens.json.swp", "tokens.json.swo", "tokens.json.swn"] {
        assert!(is_secret_name(name), "{name}");
    }
    for name in ["credentials.svg", "cookies.sql", "kubeconfig.sig", "tokens.json.svz", "tokens.json.saa"] {
        assert!(!is_secret_name(name), "{name}");
    }
}

/// Review R5-H1: Neovim's and Emacs's copies of a file in a protected place spell its path into the
/// name, so the place is judged, not only the last part `config`.
#[test]
fn an_editors_copy_of_a_file_in_a_protected_place_is_protected() {
    for p in [
        "/home/ann/.local/state/nvim/swap/%home%ann%.kube%config.swp",
        "/home/ann/.local/state/nvim/undo/%home%ann%.kube%config",
        "/home/ann/.local/state/nvim/swap/%home%ann%.docker%config.json.swp",
        "/home/ann/.local/state/nvim/swap/%home%ann%.hermes%config.yaml.swp",
        "/home/ann/.local/state/nvim/swap/%home%ann%.hermes%config.yaml.svz",
        "/home/ann/.emacs.d/backups/!home!ann!.kube!config~",
        "/home/ann/.emacs.d/backups/!home!ann!.kube!config.~3~",
        "/home/ann/.local/state/nvim/swap/%HOME%ann%.Kube%Config.swp",
        // Review R6-H1: Emacs's auto-save wraps the same spelling in `#…#`.
        "/home/ann/.emacs.d/.cache/auto-save/#!home!ann!.kube!config#",
        "/home/ann/.emacs.d/.cache/auto-save/#!home!ann!.docker!config.json#",
        "/home/ann/.cache/doom/autosave/#!home!ann!.hermes!config.yaml#",
        "/home/ann/.cache/doom/autosave/#%home%ann%.kube%config#",
        "/home/ann/.cache/doom/autosave/#!HOME!ann!.Kube!Config#",
    ] {
        assert!(is_protected(Path::new(p)), "{p}");
        assert!(names_a_protected_copy(p.rsplit('/').next().unwrap_or(p)), "{p}");
    }
    for p in [
        "/home/ann/.local/state/nvim/swap/%home%ann%notes.md.swp",
        "/home/ann/.local/state/nvim/swap/%home%ann%documents%config.swp",
        "/home/ann/.local/state/nvim/undo/%home%ann%documents%config",
        "/home/ann/.emacs.d/backups/!home!ann!documents!config~",
        "/home/ann/.emacs.d/backups/!home!ann!notes.md~",
        "/home/ann/.emacs.d/.cache/auto-save/#!home!ann!documents!notes.md#",
        "/home/ann/.cache/doom/autosave/#!home!ann!documents!config#",
    ] {
        assert!(!is_protected(Path::new(p)), "{p} stays readable");
        assert!(!names_a_protected_copy(p.rsplit('/').next().unwrap_or(p)), "{p}");
    }
}

/// Review R5-L1: the part before the first `%` is a file's name only when it looks like one.
#[test]
fn a_url_encoded_name_is_judged_by_its_first_part_only_when_that_is_a_file_name() {
    for name in ["id_rsa%20old", ".env%20prod", "client_secret_1%2B.json"] {
        assert!(is_secret_name(name), "{name}");
    }
    for name in ["cookies%20recipe.pdf", "credentials%20how-to.md", "cookies%2Fchocolate.md"] {
        assert!(!is_secret_name(name), "{name}");
        assert!(!is_protected(Path::new(&format!("/home/ann/Documents/{name}"))), "{name}");
    }
}

/// Review R6-M2: a name of stacked markers cost cubic time, so a 4 KB page title froze the shell.
/// Judged as it stands above NAME_MAX, and each original once below. The bound is the best of three
/// runs under 50 ms: the fixed code takes microseconds, so a busy CI machine does not trip it, and
/// the unfixed code takes seconds (it took 12 s for `is_protected`), which three runs cannot hide.
#[test]
fn a_long_name_of_stacked_markers_is_judged_in_no_time() {
    let best_of_three = |work: &dyn Fn() -> bool| {
        (0..3)
            .map(|_| {
                let start = std::time::Instant::now();
                let answer = work();
                (start.elapsed(), answer)
            })
            .min_by_key(|(took, _)| *took)
            .unwrap_or_default()
    };
    for lead in ["%", "!"] {
        let name = format!("{lead}{}", "~".repeat(4095));
        let (took, secret) = best_of_three(&|| is_secret_name(&name));
        assert!(!secret && took < std::time::Duration::from_millis(50), "{lead}: {took:?}");
        let (took, protected) = best_of_three(&|| is_protected(Path::new(&format!("/home/ann/{name}"))));
        assert!(!protected && took < std::time::Duration::from_millis(50), "{lead} as a path: {took:?}");
    }
    let name = format!("%{}", ":x".repeat(512));
    assert!(best_of_three(&|| is_secret_name(&name)).0 < std::time::Duration::from_millis(50));
    // A name at NAME_MAX is still judged as a copy, once per original: about 2 ms in a debug build
    // with the seen-set, 100 ms without it, so 30 ms tells the two apart in a debug build (CI's).
    let at_the_limit = format!("%{}", "~".repeat(NAME_MAX - 1));
    let (took, secret) = best_of_three(&|| is_secret_name(&at_the_limit));
    assert!(!secret && took < std::time::Duration::from_millis(30), "{took:?}");
    // Above it, a name is judged as it stands: a long key name is still a key, and a long copy is no one's.
    let long = "a".repeat(NAME_MAX);
    assert!(is_secret_name(&format!("{long}.pem")) && !is_secret_name(&format!("{long}.pem.bak")));
}

/// Review R6-L2: a copy marker, whatever it is a copy of.
#[test]
fn a_name_with_a_copy_marker_is_a_copy_name() {
    for name in ["client_secret.pub - Copy.json", "tokens.json.bak", "notes (copy 1).md", "TOKENS (1).json"] {
        assert!(is_a_copy_name(name), "{name}");
    }
    for name in ["id_rsa.pub - gedit", "notes.md", "id_rsa.pub"] {
        assert!(!is_a_copy_name(name), "{name}");
    }
}
