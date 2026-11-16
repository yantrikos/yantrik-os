//! The file tools' path roots (#1120): workshops and the home. Moved out of `tools.rs` (#1247, review R11-L5).

use crate::tools::{under_any, validate_path, with_canonical};
use std::path::PathBuf;

/// #1120: the companion's file tools (`read_file`, `grep`, `git` and the rest all validate their
/// path here) refuse a Mind's workshops, the part not made yet included.
#[test]
fn a_minds_workshop_is_out_of_reach_of_the_companions_tools() {
    for p in [
        "~/.local/share/yantrik/workshops/hermes/shop/notes.md",
        "~/.local/share/yantrik/workshops/hermes",
        "~/.local/share/yantrik/workshops/hermes/shop/../shop/notes.md",
        "/var/lib/yantrik/workshops/1000/hermes/shop/notes.md",
        "/var/lib/yantrik/workshops/1000/hermes",
        "/var/lib/yantrik/workshops/1000/hermes/shop/../shop/notes.md",
        "/var/lib/yantrik/workshops",
    ] {
        assert!(validate_path(p).is_err(), "{p}");
    }
}

/// Gate 12 (G7): outside the home, a workshop is refused in the rule's own words, and an ordinary
/// folder outside it still gets the sentence about the home.
#[test]
fn a_workshop_outside_the_home_is_refused_in_the_rules_words() {
    for p in ["/var/lib/yantrik/workshops/1000/hermes/shop/notes.md", "/var/lib/yantrik/workshops"] {
        let err = validate_path(p).unwrap_err();
        assert!(err.contains("Access denied") && err.contains("protected") && !err.contains("home directory"), "{p}: {err}");
    }
    assert!(validate_path("/usr/share/doc").unwrap_err().contains("under your home directory"));
}

#[test]
fn the_work_dir_is_out_of_reach_in_both_spellings() {
    // Other programs write there while following links; the model must not be able to plant
    // anything in it, even through the home fallback that sits under $HOME.
    let work_in_home = format!("~/{}/voice-0123/voice.wav", yantrik_ml::private_dir::WORK_HOME_REL);
    let err = validate_path(&work_in_home).unwrap_err();
    assert!(err.contains("not allowed"), "{err}");
    let work_in_runtime = format!("/run/user/1000/{}/voice-0123/voice.wav", yantrik_ml::private_dir::WORK_NAME);
    assert!(validate_path(&work_in_runtime).is_err());
    if let Ok(work) = yantrik_ml::private_dir::work_dir() {
        assert!(validate_path(work.join("x.txt").to_str().unwrap()).is_err());
    }
}

#[test]
fn a_link_deep_in_the_path_is_followed_to_where_it_leads() {
    use super::resolves_within;
    let root = std::env::temp_dir().join(format!("yantrik-resolves-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("notes")).unwrap();
    let root = root.canonicalize().unwrap();
    std::os::unix::fs::symlink("/etc", root.join("escape")).unwrap();
    std::os::unix::fs::symlink("/etc/yantrik-no-such-thing", root.join("dangling")).unwrap();
    let roots = [root.clone()];
    let at = |p: &str| resolves_within(&root.join(p), &roots);

    assert!(at("notes/new.txt").is_ok(), "a new file in a real folder");
    assert!(at("notes/new/deeper/file.txt").is_ok(), "new folders under a real one");
    for out in ["escape/passwd", "escape/new.txt", "escape/no-such-dir/new.txt", "escape/a/b/c/d.txt"] {
        assert!(at(out).is_err(), "{out} leads out through the link");
    }
    for nowhere in ["dangling", "dangling/new.txt"] {
        assert!(at(nowhere).is_err(), "{nowhere} goes through a link that leads nowhere");
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_protected_name_split_across_a_link_is_refused() {
    use super::resolves_within;
    let root = std::env::temp_dir().join(format!("yantrik-resolves-split-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join(".config")).unwrap();
    std::fs::create_dir_all(root.join(".ssh")).unwrap();
    std::fs::create_dir_all(root.join("x")).unwrap();
    let root = root.canonicalize().unwrap();
    std::os::unix::fs::symlink(root.join(".config"), root.join("x/c")).unwrap();
    std::os::unix::fs::symlink(root.join(".ssh"), root.join("keys")).unwrap();
    let roots = [root.clone()];
    let at = |p: &str| resolves_within(&root.join(p), &roots);

    for refused in ["x/c/labwc/autostart", "x/c/yantrik/config.yaml", "x/c/autostart/a.desktop", "keys/authorized_keys", "keys/new/deeper"] {
        let err = at(refused).unwrap_err();
        assert!(err.contains("protected location"), "{refused}: {err}");
    }
    assert!(at("x/c/gtk-3.0/settings.ini").is_ok(), "an ordinary folder under the link");
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn a_second_name_for_a_file_is_refused() {
    use super::resolves_within;
    let root = std::env::temp_dir().join(format!("yantrik-resolves-hard-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("Documents")).unwrap();
    std::fs::create_dir_all(root.join(".local/share/yantrik")).unwrap();
    let root = root.canonicalize().unwrap();
    let store = root.join(".local/share/yantrik").join("store.json");
    std::fs::write(&store, "x").unwrap();
    std::fs::write(root.join("Documents/plain.txt"), "x").unwrap();
    std::fs::hard_link(&store, root.join("Documents/hl.json")).unwrap();
    let roots = [root.clone()];
    let err = resolves_within(&root.join("Documents/hl.json"), &roots).unwrap_err();
    assert!(err.contains("hard link"), "{err}");
    assert!(resolves_within(&root.join("Documents/plain.txt"), &roots).is_ok());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn with_no_root_at_all_the_refusal_says_there_is_no_home() {
    let err = super::resolves_within(std::path::Path::new("/home/ann/notes.txt"), &[]).unwrap_err();
    assert!(err.ends_with("there is no home directory to work in"), "{err}");
}

#[test]
fn a_write_goes_nowhere_hidden_in_the_home_but_a_read_may() {
    for hidden in ["~/.vimrc", "~/.gitconfig", "~/.config/nvim/init.lua", "~/.local/bin/x", "~/bin/x", "~/.cargo/config.toml"] {
        let err = super::validate_write_path(hidden).unwrap_err();
        assert!(err.contains("hidden folders or dotfiles"), "{hidden}: {err}");
    }
    assert!(super::validate_path("~/.gitconfig").is_ok(), "reading a dotfile that is not protected");
    assert!(super::validate_write_path("~/yantrik-write-rule-test.txt").is_ok());
    if let Ok(scratch) = yantrik_ml::private_dir::scratch_dir() {
        let own = scratch.join("diagram.svg");
        assert!(super::validate_write_path(own.to_str().unwrap()).is_ok(), "the tools' own scratch");
    }
}

/// Every place the shared list protects is refused here: by `resolves_within`'s whole-component
/// `is_protected` check, after links are followed. Asserted on behaviour rather than on string
/// membership of BLOCKED_SEGMENTS, whose matching is by substring: copying a short name like
/// `.pi` into it would refuse `photo.pic` and `server.pid` too.
#[test]
fn every_place_every_side_protects_is_blocked_here_too() {
    for place in yantrik_ipc_contracts::home_paths::PROTECTED {
        let inside = format!("~/{place}/x");
        assert!(super::validate_path(&inside).is_err(), "{inside} must be refused");
    }
    assert!(super::validate_path("~/Pictures/photo.pic").is_ok() || super::validate_path("~/Pictures/photo.pic").unwrap_err().contains("home"),
        "a name that merely contains a protected one is not refused for it");
}

#[test]
fn a_root_admits_itself_and_what_is_inside_it_only() {
    let roots = [PathBuf::from("/home/ann"), PathBuf::from("/run/user/1000/yantrik-scratch")];
    assert!(under_any("/home/ann", &roots));
    assert!(under_any("/home/ann/notes.txt", &roots));
    assert!(under_any("/run/user/1000/yantrik-scratch/diagram.png", &roots));
    assert!(!under_any("/run/user/1000/yantrik/companion.sock", &roots), "the socket dir is not scratch");
    assert!(!under_any("/home/anne/notes.txt", &roots), "a name that starts the same is not inside");
    assert!(!under_any("/tmp/planted.txt", &roots), "the shared /tmp is not ours");
    assert!(!under_any("/etc/shadow", &roots));
}

#[test]
fn no_roots_admit_nothing() {
    // An unset or empty HOME used to be the empty prefix, which every path starts with.
    assert!(!under_any("/etc/shadow", &[]));
    assert!(!under_any("/", &[]));
}

#[cfg(unix)]
fn scratch_base(tag: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!("yantrik-roots-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    base
}

#[cfg(unix)]
#[test]
fn a_root_that_is_a_link_admits_what_is_really_inside_it() {
    let base = scratch_base("link");
    let real = base.join("data-ann");
    std::fs::create_dir(&real).unwrap();
    let home = base.join("home-ann");
    std::os::unix::fs::symlink(&real, &home).unwrap();
    let roots = with_canonical(vec![home.clone()]);
    // The resolved path, as validate_path checks it after canonicalizing.
    let resolved = real.canonicalize().unwrap().join("notes.txt");
    assert!(under_any(resolved.to_str().unwrap(), &roots));
    // And the path as written, as it is checked before.
    assert!(under_any(home.join("notes.txt").to_str().unwrap(), &roots));
    let _ = std::fs::remove_dir_all(&base);
}

#[cfg(unix)]
#[test]
fn a_root_that_resolves_to_slash_is_dropped() {
    let base = scratch_base("slash");
    let home = base.join("home");
    std::os::unix::fs::symlink("/", &home).unwrap();
    assert!(with_canonical(vec![home]).is_empty());
    let _ = std::fs::remove_dir_all(&base);
}
