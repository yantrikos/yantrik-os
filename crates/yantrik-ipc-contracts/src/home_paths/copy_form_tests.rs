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
        "id_rsa.pub.part", "100% done.md",
    ] {
        assert!(!is_secret_name(name), "{name}");
        assert!(!is_protected(Path::new(&format!("/home/ann/Documents/{name}"))), "{name}");
    }
}
