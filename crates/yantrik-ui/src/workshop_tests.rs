use super::*;

use super::*;

/// A home with its root made the way the installer makes it: the person's, 0700.
fn home() -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt;
    workshops::roots_under_home_for_tests();
    let h = tempfile::tempdir().unwrap();
    std::fs::create_dir(h.path().join("Documents")).unwrap();
    std::fs::create_dir_all(workshops::root(h.path())).unwrap();
    std::fs::set_permissions(workshops::root(h.path()), std::fs::Permissions::from_mode(0o700)).unwrap();
    h
}

fn root_str(h: &tempfile::TempDir) -> String {
    workshops::root(h.path()).display().to_string()
}

fn refused(h: &tempfile::TempDir, raw: &str) -> String {
    check_folder(h.path(), raw, true).expect_err(raw)
}

#[test]
fn a_new_folder_under_the_root_is_made_private_and_resolved() {
    let h = home();
    let at = root_str(&h);
    let got = check_folder(h.path(), &format!("{at}/hermes/sitcom"), true).unwrap();
    let real = h.path().canonicalize().unwrap();
    assert_eq!(got, workshops::root(&real).join("hermes/sitcom"));
    use std::os::unix::fs::PermissionsExt;
    for dir in [got.clone(), got.parent().unwrap().to_path_buf(), workshops::root(&real)] {
        assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700, "{}", dir.display());
    }
    // Run time: the same folder, now holding files, passes without the grant rules.
    std::fs::write(got.join("a.txt"), "x").unwrap();
    assert_eq!(check_folder(h.path(), got.to_str().unwrap(), false).unwrap(), got);
    assert!(refused(&h, &format!("{at}/hermes/sitcom")).contains("already holds"));
    assert_eq!(folder_for(h.path(), "hermes", "sitcom", false).unwrap(), got);
}

#[test]
fn only_the_root_is_accepted() {
    let h = home();
    let at = root_str(&h);
    let p = h.path().display().to_string();
    for raw in [
        "~", "~/", "~/Documents", "~/Documents/x", "~/Desktop/x", "~/Downloads/x", "/tmp/x", "/", "relative/dir", "", &p,
        "~/.local/share/yantrik/workshops/hermes/x", &format!("{p}/.local/share/yantrik/workshops/hermes/x"),
        &format!("{p}/Documents/x"),
        at.as_str(), &format!("{at}/"), &format!("{at}/hermes"), &format!("{at}/hermes/a/b"), &format!("{at}/hermes/"),
        &format!("{at}2/hermes/x"), &format!("{at}/../hermes/x"),
    ] {
        assert!(check_folder(h.path(), raw, true).is_err(), "{raw:?} must be refused");
    }
    assert_eq!(std::fs::read_dir(workshops::root(h.path())).unwrap().count(), 0, "a refused folder makes nothing");
}

#[test]
fn dotdot_and_hidden_names_are_refused() {
    let h = home();
    let at = root_str(&h);
    assert!(check_folder(h.path(), &format!("{at}/../../../../Documents/x"), true).is_err());
    assert!(refused(&h, &format!("{at}/hermes/../pi/x")).contains(".."));
    assert!(refused(&h, &format!("{at}/hermes/.hidden")).contains("dot"));
    assert!(refused(&h, &format!("{at}/.hermes/x")).contains("dot"));
    assert!(refused(&h, &format!("{at}/hermes/a\nb")).contains("plain path"));
    assert!(!workshops::root(h.path()).join("hermes/.hidden").exists());
    for bad in ["a/b", ".x", "..", "", "."] {
        assert!(folder_for(h.path(), "hermes", bad, true).is_err(), "{bad:?}");
    }
    assert!(folder_for(h.path(), "../pi", "x", true).is_err());
}

#[test]
fn symlinks_are_refused_at_every_step() {
    let h = home();
    let at = root_str(&h);
    let outside = tempfile::tempdir().unwrap();
    let real = h.path().canonicalize().unwrap();
    let root = workshops::root(&real);
    std::fs::create_dir_all(root.join("hermes")).unwrap();
    std::fs::set_permissions(root.join("hermes"), std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
    std::os::unix::fs::symlink(outside.path(), root.join("hermes/link")).unwrap();
    std::os::unix::fs::symlink(outside.path(), root.join("pi")).unwrap();
    assert!(refused(&h, &format!("{at}/hermes/link")).contains("symlink"));
    assert!(refused(&h, &format!("{at}/pi/new")).contains("symlink"));
    // The root itself a link to somewhere else.
    let h2 = home();
    std::fs::remove_dir(workshops::root(h2.path())).unwrap();
    std::os::unix::fs::symlink(outside.path(), workshops::root(h2.path())).unwrap();
    assert!(check_folder(h2.path(), &format!("{}/hermes/x", root_str(&h2)), true).is_err());
    // Granted, then swapped for a link: the run-time check refuses.
    let shop = check_folder(h.path(), &format!("{at}/hermes/shop"), true).unwrap();
    std::fs::remove_dir(&shop).unwrap();
    std::os::unix::fs::symlink(outside.path(), &shop).unwrap();
    assert!(check_folder(h.path(), shop.to_str().unwrap(), false).unwrap_err().contains("symlink"));
}

#[test]
fn a_folder_with_files_is_not_new() {
    use std::os::unix::fs::PermissionsExt;
    let h = home();
    let at = root_str(&h);
    let shop = check_folder(h.path(), &format!("{at}/hermes/old"), true).unwrap();
    std::fs::write(shop.join(".env"), "k=v").unwrap();
    assert!(refused(&h, &format!("{at}/hermes/old")).contains("already holds"));
    let empty = workshops::root(&h.path().canonicalize().unwrap()).join("hermes/empty");
    std::fs::create_dir(&empty).unwrap();
    std::fs::set_permissions(&empty, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(check_folder(h.path(), &format!("{at}/hermes/empty"), true).is_ok());
    // Not made for a card's sentence, or for a run.
    assert!(check_folder(h.path(), &format!("{at}/hermes/never"), false).unwrap_err().contains("does not exist"));
}

#[test]
fn the_record_grants_per_mind_and_revoking_deletes() {
    let h = home();
    let rec = h.path().join("workshops.json");
    assert!(load(&rec).unwrap().is_empty());
    let shop = folder_for(h.path(), "hermes", "shop", true).unwrap();
    grant(&rec, "hermes", &shop, 5).unwrap();
    assert_eq!(granted_for(&rec, h.path(), "hermes", shop.to_str().unwrap()).unwrap(), shop);
    // Another Mind has none, and a different folder is not the grant.
    assert!(granted_for(&rec, h.path(), "pi", shop.to_str().unwrap()).unwrap_err().contains("no workshop"));
    let other = folder_for(h.path(), "hermes", "other", true).unwrap();
    assert!(granted_for(&rec, h.path(), "hermes", other.to_str().unwrap()).unwrap_err().contains("not the workshop"));
    // A new grant replaces; the file is private.
    grant(&rec, "hermes", &other, 6).unwrap();
    assert_eq!(load(&rec).unwrap().len(), 1);
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(&rec).unwrap().permissions().mode() & 0o777, 0o600);
    assert!(revoke(&rec, "hermes").unwrap());
    assert!(!revoke(&rec, "hermes").unwrap());
    assert!(granted_for(&rec, h.path(), "hermes", other.to_str().unwrap()).is_err());
}

#[test]
fn a_record_that_cannot_be_read_grants_nothing() {
    let h = home();
    let rec = h.path().join("workshops.json");
    std::fs::write(&rec, "{not json").unwrap();
    let shop = folder_for(h.path(), "hermes", "shop", true).unwrap();
    assert!(granted_for(&rec, h.path(), "hermes", shop.to_str().unwrap()).unwrap_err().contains("no workshop is granted"));
    assert!(grant(&rec, "hermes", &shop, 1).is_err(), "a corrupt record is not overwritten");
}

#[test]
fn cwd_stays_inside_the_workshop() {
    let h = home();
    let shop = folder_for(h.path(), "hermes", "shop", true).unwrap();
    std::fs::create_dir(shop.join("app")).unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), shop.join("escape")).unwrap();
    assert_eq!(cwd_inside(&shop, None).unwrap(), shop);
    assert_eq!(cwd_inside(&shop, Some(Path::new("app"))).unwrap(), shop.join("app"));
    assert_eq!(cwd_inside(&shop, Some(&shop.join("app"))).unwrap(), shop.join("app"));
    for bad in ["..", "app/../..", "escape", "escape/x", "/", "/tmp", "missing"] {
        assert!(cwd_inside(&shop, Some(Path::new(bad))).is_err(), "{bad}");
    }
    assert!(cwd_inside(&shop, Some(h.path())).is_err());
    // A sibling that merely shares the prefix.
    let sibling = folder_for(h.path(), "hermes", "shop2", true).unwrap();
    assert!(cwd_inside(&shop, Some(&sibling)).is_err());
}

/// #1124 M2: the grant is the folder. Deleting it revokes the grant, and a new folder that
/// takes the same name, with the person's own files in it, gets nothing.
#[test]
fn a_folder_deleted_and_made_again_under_the_same_name_is_not_the_granted_folder() {
    let h = home();
    let rec = h.path().join("workshops.json");
    let shop = folder_for(h.path(), "hermes", "project", true).unwrap();
    grant(&rec, "hermes", &shop, 1).unwrap();
    assert!(granted_for(&rec, h.path(), "hermes", shop.to_str().unwrap()).is_ok());
    std::fs::remove_dir(&shop).unwrap();
    assert!(granted_for(&rec, h.path(), "hermes", shop.to_str().unwrap()).is_err());
    assert!(prune(&rec).unwrap().is_empty(), "a gone folder is not a workshop");
    // Replaced by another folder with the same name.
    let shop = folder_for(h.path(), "hermes", "project", true).unwrap();
    grant(&rec, "hermes", &shop, 2).unwrap();
    std::fs::remove_dir(&shop).unwrap();
    std::fs::create_dir(&shop).unwrap();
    std::fs::write(shop.join(".env"), "KEY=secret").unwrap();
    let why = granted_for(&rec, h.path(), "hermes", shop.to_str().unwrap()).unwrap_err();
    assert!(why.contains("no workshop"), "{why}");
}

#[test]
fn a_record_without_an_inode_matches_no_folder() {
    let h = home();
    let rec = h.path().join("workshops.json");
    let shop = folder_for(h.path(), "hermes", "old", true).unwrap();
    let old = format!(r#"{{"grants":[{{"agent":"hermes","folder":"{}","granted_unix":1}}]}}"#, shop.display());
    std::fs::write(&rec, old).unwrap();
    assert!(granted_for(&rec, h.path(), "hermes", shop.to_str().unwrap()).is_err());
}

/// #1124 L2: a folder belongs to one Mind.
#[test]
fn a_second_mind_cannot_take_a_folder_the_first_holds() {
    let h = home();
    let rec = h.path().join("workshops.json");
    let shop = folder_for(h.path(), "hermes", "shared", true).unwrap();
    grant(&rec, "hermes", &shop, 1).unwrap();
    assert!(grant(&rec, "pi", &shop, 2).unwrap_err().contains("another Mind's"));
    assert!(grant(&rec, "hermes", &shop, 3).is_ok(), "the holder may ask again");
}

/// #1124 L4: nothing that draws differently from how it reads.
#[test]
fn bidi_and_other_format_characters_in_a_folder_name_are_refused() {
    let h = home();
    for bad in ["\u{202E}gpj.x", "a\u{200B}b", "a\u{2066}b", "a\u{FEFF}b", "a\u{00AD}b"] {
        assert!(folder_for(h.path(), "hermes", bad, true).is_err(), "{bad:?}");
    }
    assert!(folder_for(h.path(), "hermes", "sitcom-\u{e9}pisode", true).is_ok());
}

/// #1124 M4: the record is inside the root, so every view that hides the root hides it, and
/// every in-process tool that refuses the root refuses it. No name a Mind chooses can be it.
#[test]
fn the_grant_record_is_inside_the_root_where_no_view_of_a_mind_reaches() {
    let h = home();
    let home = h.path().canonicalize().unwrap();
    let rec = record_in(&home);
    assert_eq!(rec, workshops::root(&home).join(".grants.json"));
    let shop = folder_for(&home, "hermes", "shop", true).unwrap();
    grant(&rec, "hermes", &shop, 1).unwrap();
    assert!(rec.is_file() && workshops::holds(&rec, &home), "the tools that refuse the root refuse the record");
    // The outside view is a tmpfs over the root, so the record is not in it; the sealed view has
    // only the one workshop.
    let masked = workshops::outside_args(&[workshops::base(&home)], true);
    assert!(masked.windows(2).any(|w| w[0] == "--tmpfs" && rec.starts_with(&w[1])), "the record is under the masked base");
    assert!(!masked.iter().any(|a| a.contains(".grants")), "nothing re-exposes it");
    for name in [".grants.json", ".grants"] {
        assert!(plain_name("the Mind's id", name).is_err() && folder_for(&home, name, "x", true).is_err(), "{name}");
        assert!(folder_for(&home, "hermes", name, true).is_err(), "{name}");
    }
    assert!(!workshops::base(&home).join("workshops.json").exists(), "not beside the root");
}

/// Round 6, finding 3: a Mind's folder or a workshop that is already there is taken only when it
/// is the person's alone; one a Mind's job made before the grant (or another account made) is not.
#[test]
fn a_folder_that_is_already_there_must_be_the_persons_and_private() {
    use std::os::unix::fs::PermissionsExt;
    let h = home();
    let at = root_str(&h);
    let root = workshops::root(&h.path().canonicalize().unwrap());
    std::fs::create_dir_all(root.join("hermes/open")).unwrap();
    for what in ["hermes", "hermes/open"] {
        std::fs::set_permissions(root.join(what), std::fs::Permissions::from_mode(0o755)).unwrap();
        for for_grant in [true, false] {
            let why = check_folder(h.path(), &format!("{at}/hermes/open"), for_grant).unwrap_err();
            assert!(why.contains("not the person's alone"), "{what} {for_grant}: {why}");
        }
        std::fs::set_permissions(root.join(what), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    assert!(check_folder(h.path(), &format!("{at}/hermes/open"), true).is_ok());
}
