//! Round 12 of #1265 (the 07:44 and 07:49 reads of round 11): the output folder checked and bound from a
//! descriptor, the shell's listings and its file handling in the scripts folder never following a link,
//! the dependencies folders checked before the lock, and the registry's `filename` untrusted.

use crate::coder_files::{checked_filename, list_files, Scripts};
use crate::coder_promote::ensure_folders;
use crate::coder_round10_tests::scratch;
use crate::coder_script_view::script_view;
use std::collections::HashMap;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::time::{Duration, Instant};

fn scripts_in(tag: &str) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
    let top = scratch(tag);
    let (home, scripts) = (top.join("home"), top.join("scripts"));
    std::fs::create_dir_all(&scripts).unwrap();
    (top, home, scripts)
}

/// Medium 1: a link planted at the output folder refuses the run, whatever it leads to; a real folder is
/// bound from a held descriptor.
#[test]
fn a_link_at_the_output_folder_refuses_the_run_and_a_real_one_is_held_by_descriptor() {
    let (top, home, scripts) = scripts_in("r12-output-link");
    let hidden = top.join("hidden");
    std::fs::create_dir_all(&hidden).unwrap();
    for target in [hidden.clone(), Path::new("../hidden").to_path_buf()] {
        let _ = std::fs::remove_file(scripts.join("output"));
        symlink(&target, scripts.join("output")).unwrap();
        let why = script_view(&home, &scripts.join("output"), &scripts, "python", &HashMap::new()).err().expect("a link at the output folder refuses");
        assert!(why.starts_with("Script run refused:") && why.contains("output"), "{why}");
    }
    std::fs::remove_file(scripts.join("output")).unwrap();
    // Missing: made through the descriptor code, then held.
    let view = script_view(&home, &scripts.join("output"), &scripts, "python", &HashMap::new()).expect("a missing output folder is made");
    assert!(scripts.join("output").symlink_metadata().unwrap().is_dir());
    assert_eq!(view.held.len(), 1, "the output folder is held open");
    assert_eq!(view.held.numbers(), [view.output_fd]);
    // Not a folder at all.
    std::fs::remove_dir(scripts.join("output")).unwrap();
    std::fs::write(scripts.join("output"), "x").unwrap();
    assert!(script_view(&home, &scripts.join("output"), &scripts, "python", &HashMap::new()).is_err());
    let _ = std::fs::remove_dir_all(top);
}

/// Medium 2: a link to a hidden folder is neither listed nor entered, and a self-link ends the walk.
#[test]
fn the_listing_never_follows_a_link_and_a_self_link_ends() {
    let (top, _home, scripts) = scripts_in("r12-walk");
    let out = scripts.join("output");
    let hidden = top.join("hidden-place");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::create_dir_all(&hidden).unwrap();
    std::fs::write(hidden.join("secret-name.txt"), "s").unwrap();
    std::fs::write(out.join("made.csv"), "a").unwrap();
    symlink(&hidden, out.join("to-hidden")).unwrap();
    symlink(&out, out.join("self")).unwrap();
    symlink("loop-b", out.join("loop-a")).unwrap();
    symlink("loop-a", out.join("loop-b")).unwrap();
    symlink(hidden.join("secret-name.txt"), out.join("file-link.txt")).unwrap();
    let (files, more) = list_files(&out);
    assert_eq!(files, [out.join("made.csv")], "{files:?}");
    assert_eq!(more, 0);
    // The scripts folder as a whole, the way the shell lists it after a run.
    let (all, _) = list_files(&scripts);
    assert!(!all.iter().any(|p| p.to_string_lossy().contains("secret-name")), "{all:?}");
    // A link at the top itself is not entered.
    let top_link = top.join("top-link");
    symlink(&hidden, &top_link).unwrap();
    assert!(list_files(&top_link).0.is_empty());
    let _ = std::fs::remove_dir_all(top);
}

/// Medium 2: the depth is 8 and the number of entries 2,000; the rest is counted.
#[test]
fn the_listing_has_a_depth_and_an_entry_cap_and_counts_the_rest() {
    let (top, _home, scripts) = scripts_in("r12-cap");
    let wide = scripts.join("wide");
    std::fs::create_dir_all(&wide).unwrap();
    for i in 0..2_005 {
        std::fs::write(wide.join(format!("f{i:05}")), "").unwrap();
    }
    let (files, more) = list_files(&wide);
    assert_eq!((files.len(), more), (2_000, 5));
    let deep = (0..10).fold(scripts.join("deep"), |p, i| p.join(format!("d{i}")));
    std::fs::create_dir_all(&deep).unwrap();
    std::fs::write(deep.join("too-deep"), "").unwrap();
    std::fs::write(scripts.join("deep/d0/d1/d2/d3/d4/d5/d6/d7/ok"), "").unwrap();
    let (files, _) = list_files(&scripts.join("deep"));
    assert!(files.iter().any(|p| p.ends_with("ok")) && !files.iter().any(|p| p.ends_with("too-deep")), "{files:?}");
    let _ = std::fs::remove_dir_all(top);
}

/// Low 3: with the folders in place, a held lock does not delay a call; a missing one waits for the lock.
#[test]
fn the_dependencies_folders_are_checked_before_the_lock() {
    let top = scratch("r12-lock");
    let (parent, stage) = (top.join("deps"), top.join("stage"));
    ensure_folders(&parent, &["python", "node_modules"], &stage).unwrap();
    let held = crate::coder_promote::staging_root(&stage).unwrap();
    let _lock = held.lock(Duration::from_secs(1)).unwrap();
    let t = Instant::now();
    ensure_folders(&parent, &["python", "node_modules"], &stage).expect("nothing is made, so no lock is needed");
    assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
    drop(_lock);
    // A link where a folder belongs is still refused (the check does not follow it).
    std::fs::remove_dir(parent.join("python")).unwrap();
    symlink(&top, parent.join("python")).unwrap();
    assert!(ensure_folders(&parent, &["python"], &stage).is_err());
    let _ = std::fs::remove_dir_all(top);
}

/// Addendum: a link at a script's name and at `registry.json` is refused for reading and writing, and the
/// file it leads to is neither read nor changed.
#[test]
fn a_link_at_a_script_name_or_at_the_registry_is_never_followed() {
    let (top, _home, scripts) = scripts_in("r12-links");
    let outside = top.join("protected.txt");
    std::fs::write(&outside, "do not touch").unwrap();
    symlink(&outside, scripts.join("evil.py")).unwrap();
    symlink(&outside, scripts.join("registry.json")).unwrap();
    let s = Scripts::open_at(scripts.clone()).unwrap();
    for file in ["evil.py", "registry.json"] {
        assert!(s.read(file).is_err(), "{file} is not read");
        assert!(s.write(file, "pwned", 0o755, false).is_err(), "{file} is not written");
        assert!(s.write(file, "pwned", 0o755, true).is_err(), "{file} is not made over");
        assert!(!s.is_plain_file(file), "{file} is not a plain file");
    }
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), "do not touch");
    // A plain file works, is made executable, and a name with a path is refused.
    s.write("ok.py", "print(1)", 0o755, false).unwrap();
    assert_eq!(s.read("ok.py").unwrap(), "print(1)");
    assert_eq!(std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(scripts.join("ok.py")).unwrap().permissions()) & 0o777, 0o755);
    s.write("ok.py", "print(2)", 0o755, false).unwrap();
    assert!(s.write("ok.py", "x", 0o755, true).is_err(), "an exclusive write never replaces");
    for bad in ["../protected.txt", "sub/x.py", "..", ".", ""] {
        assert!(s.read(bad).is_err() && s.write(bad, "x", 0o755, false).is_err(), "{bad:?}");
    }
    // A folder, a pipe or a file with another name is not a plain file to write.
    std::fs::create_dir(scripts.join("dir.py")).unwrap();
    assert!(s.write("dir.py", "x", 0o755, false).is_err() && s.read("dir.py").is_err());
    std::fs::hard_link(&outside, scripts.join("hard.py")).unwrap();
    assert!(s.write("hard.py", "pwned", 0o755, false).is_err());
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), "do not touch");
    // Removing a link removes the link, not what it leads to.
    s.remove("evil.py");
    assert!(scripts.join("evil.py").symlink_metadata().is_err() && outside.exists());
    // The scripts folder itself as a link is not used.
    let link = top.join("scripts-link");
    symlink(&scripts, &link).unwrap();
    assert!(Scripts::open_at(link).is_err());
    let _ = std::fs::remove_dir_all(top);
}

/// Addendum: the registry's `filename` is exactly the sanitized name plus the language's extension.
#[test]
fn a_registry_filename_with_a_path_in_it_is_refused() {
    assert_eq!(checked_filename("tool", "tool.py", ".py"), Ok("tool.py"));
    for bad in ["../tool.py", "/etc/passwd", "sub/tool.py", "..", "tool.py/", "other.py", "tool.sh", "tool.py\0", ""] {
        assert!(checked_filename("tool", bad, ".py").is_err(), "{bad:?}");
    }
    assert!(checked_filename("a/b", "a/b.py", ".py").is_err(), "a key with a slash is no plain name");
    let src = include_str!("coder.rs");
    // Every use of an entry's file goes through the check; none joins `entry.filename` to a path.
    assert!(!src.contains("join(&entry.filename)") && !src.contains("scripts_dir()"), "a path is built from the registry again");
}
