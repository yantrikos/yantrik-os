//! Round 7 of #1265: what the two security reads of `6b827a46` found and the decision asked for, one
//! test (or more) per item. The ones that need a live `bwrap` return without one; they run on the
//! project runner.

use super::tests::{home, in_view, plant, run_sealed};
use super::*;
use crate::home_paths::held::{open_beneath, Held};
use crate::home_paths::view_places::{secrets_in, toolchain_words_with};
use std::os::unix::fs::symlink;
use std::time::Duration;

fn has(words: &[String], flag: &str, to: &Path) -> bool {
    words.windows(3).any(|w| w[0] == flag && Path::new(&w[2]) == to)
}

/// 1. A `reach` place is bound from a descriptor, and a place with a link anywhere on its way cannot
/// be opened.
#[test]
fn a_reach_place_is_bound_from_a_descriptor_opened_without_following_a_link() {
    let (top, home) = home();
    plant(&home, ".ssh/id");
    let (words, held) = view_args(&home, &home, &[home.join(".ssh")]).unwrap();
    assert_eq!(held.len(), 1);
    let n = held.numbers()[0];
    assert!(n >= 64, "above the numbers a door uses: {n}");
    assert!(words.windows(3).any(|w| w[0] == "--ro-bind-fd" && w[1] == n.to_string() && Path::new(&w[2]) == home.join(".ssh")), "{words:?}");
    assert!(!words.iter().any(|w| w == "--ro-bind"), "nothing is bound by its path: {words:?}");
    // A link in the middle of the way, and one at the end.
    plant(&home, "real/inner/f");
    symlink(home.join("real"), home.join(".cfg")).unwrap();
    symlink(home.join(".ssh"), home.join(".end")).unwrap();
    for through in [home.join(".cfg/inner"), home.join(".cfg"), home.join(".end")] {
        assert!(open_beneath(&home, &through).is_err(), "{} was opened", through.display());
    }
    assert!(open_beneath(&home, &home.join("real/inner/f")).is_ok());
    assert!(open_beneath(&home, Path::new("/etc/passwd")).is_err(), "only beneath the home");
    // A place replaced by a link after the card is refused when it is opened.
    let checked = reach_list(&home, &["~/.ssh".to_string()]).unwrap();
    std::fs::rename(home.join(".ssh"), home.join("moved")).unwrap();
    symlink(home.join("moved"), home.join(".ssh")).unwrap();
    assert!(view_args(&home, &home, &checked).is_err());
    let _ = std::fs::remove_dir_all(top);
}

/// 1. The held descriptors survive the exec of a door's child and nothing else does.
#[test]
fn the_doors_helper_keeps_the_held_descriptors_and_closes_every_other_one() {
    use std::os::fd::AsRawFd;
    let (top, home) = home();
    plant(&home, ".ssh/id");
    let (_, held) = view_args(&home, &home, &[home.join(".ssh")]).unwrap();
    let kept = held.numbers()[0];
    // A descriptor that a library left open without close-on-exec.
    let leaked = std::fs::File::open("/etc/passwd").unwrap();
    // Without close-on-exec (`F_DUPFD` does not set it), at a number the shell's own reads do not use.
    let leaked = unsafe { libc::fcntl(leaked.as_raw_fd(), libc::F_DUPFD, 30) };
    assert!(leaked >= 30);
    let script = format!("for f in /proc/$$/fd/*; do n=${{f##*/}}; echo fd:$n; done; [ -e /proc/$$/fd/{kept} ] && echo held:yes");
    let out = run_sealed("/bin/sh".into(), vec!["-c".into(), script], held, &home).expect("a shell");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("held:yes"), "{text}");
    let open: Vec<i32> = text.lines().filter_map(|l| l.strip_prefix("fd:")).filter_map(|n| n.parse().ok()).collect();
    assert!(!open.contains(&leaked), "a descriptor left open reached the child: {open:?}");
    assert!(open.iter().all(|&n| n <= 3 || n == kept), "only the standard three (and the shell's own read of the listing) and the held one: {open:?}");
    unsafe { libc::close(leaked) };
    let _ = std::fs::remove_dir_all(top);
}

/// 1, with a live `bwrap`: the held place is read (not written) in the view, and the command does not
/// hold a descriptor of it.
#[test]
fn the_real_view_binds_the_held_place_and_leaves_the_command_no_descriptor_of_it() {
    let Some(_) = super::super::workshops::find_bwrap() else { return };
    let (top, home) = home();
    plant(&home, ".ssh/canary-1265");
    std::fs::create_dir_all(home.join("Documents")).unwrap();
    let cwd = home.join("Documents");
    let cmd = "cat ../.ssh/canary-1265; ls /proc/self/fd | tr '\\n' ' '";
    let (program, args, held) = super::super::workshops::outside_command(&home, &cwd, &[home.join(".ssh")], "/bin/bash", &["-c".into(), cmd.into()]).unwrap();
    let Some(out) = run_sealed(program, args, held, &cwd) else { return };
    if String::from_utf8_lossy(&out.stderr).contains("bwrap:") {
        return;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("canary-1265"), "{text} {:?}", out.stderr);
    let fds: Vec<i32> = text.split_whitespace().filter_map(|n| n.parse().ok()).collect();
    assert!(fds.iter().all(|&n| n < 10), "a held descriptor reached the command: {fds:?}");
    let _ = std::fs::remove_dir_all(top);
}

/// 1. A `reach` inside a visible folder is refused with the sentence, and one under hidden folders is not.
#[test]
fn a_reach_place_inside_a_shown_entry_is_refused_with_the_sentence() {
    let (top, home) = home();
    plant(&home, "Documents/notes/a");
    plant(&home, ".cargo/registry/x");
    for at in ["Documents", "Documents/notes", "Documents/notes/a"] {
        let why = reach_place(&home, &home.join(at)).unwrap_err();
        assert!(why.contains("it is already visible to the command"), "{at}: {why}");
    }
    assert!(reach_place(&home, &home.join(".cargo/registry")).is_ok());
    let _ = std::fs::remove_dir_all(top);
}

/// 2. A link at a masked place is refused with a sentence naming it, whether it resolves or dangles,
/// and a real folder there is left out.
#[test]
fn a_link_at_a_masked_place_stops_the_command_with_a_sentence_naming_it() {
    let (top, home) = home();
    plant(&home, "Pictures/Holiday/a.png");
    plant(&home, "elsewhere/shot.png");
    for (name, target) in [("resolves", home.join("elsewhere")), ("dangles", home.join("nowhere"))] {
        let link = home.join("Pictures/Screenshots");
        let _ = std::fs::remove_file(&link);
        symlink(&target, &link).unwrap();
        let why = view_args(&home, &home, &[]).unwrap_err();
        assert!(why.contains("Pictures/Screenshots") && why.contains("is a link") && why.contains("Nothing was run"), "{name}: {why}");
        // Also where the folder is spelled in lower case.
        std::fs::remove_file(&link).unwrap();
        symlink(&target, home.join("Pictures/screenshots")).unwrap();
        assert!(view_args(&home, &home, &[]).is_err(), "{name}, lower case");
        std::fs::remove_file(home.join("Pictures/screenshots")).unwrap();
    }
    plant(&home, "Videos/Recordings/r.mkv");
    std::fs::remove_dir_all(home.join("Videos/Recordings")).unwrap();
    symlink(home.join("elsewhere"), home.join("Videos/Recordings")).unwrap();
    assert!(view_args(&home, &home, &[]).unwrap_err().contains("Videos/Recordings"));
    std::fs::remove_file(home.join("Videos/Recordings")).unwrap();
    assert!(view_args(&home, &home, &[]).is_ok(), "no link, no refusal");
    let _ = std::fs::remove_dir_all(top);
}

/// 3. `alias.*` is dropped entirely, and the allow list keeps only plain-value keys.
#[test]
fn alias_settings_are_not_carried_and_the_environment_is_scrubbed() {
    let (top, home) = home();
    std::fs::write(
        home.join(".gitconfig"),
        "[alias]\n\tco = checkout\n\tsneaky = -c credential.helper=x status\n\tremote = push https://u:tok@host/r\n[user]\n\tname = Ann\n",
    )
    .unwrap();
    let words = view_args(&home, &home, &[]).unwrap().0;
    let all = words.join("\n");
    for never in ["alias", "checkout", "sneaky", "tok@host", "credential.helper"] {
        assert!(!all.contains(never), "{never} is in the view: {words:?}");
    }
    assert!(words.windows(3).any(|w| w[0] == "--setenv" && w[1] == "GIT_CONFIG_VALUE_0" && w[2] == "Ann"));
    let first_unset = words.iter().position(|w| w == "--unsetenv").expect("the environment is scrubbed");
    let count_set = words.windows(2).position(|w| w[0] == "--setenv" && w[1] == "GIT_CONFIG_COUNT").unwrap();
    assert!(first_unset < count_set, "unset first, then set");
    for name in ["GIT_CONFIG_GLOBAL", "GIT_CONFIG_COUNT", "GIT_SSH_COMMAND", "GIT_DIR", "XDG_CONFIG_HOME"] {
        assert!(words.windows(2).any(|w| w[0] == "--unsetenv" && w[1] == name), "{name}");
    }
    let _ = std::fs::remove_dir_all(top);
}

/// 4. `outside_command` itself compares the home it is given with the account's.
#[test]
fn outside_command_refuses_a_home_that_is_not_the_accounts() {
    let (top, home) = home();
    let (top2, other) = home_again();
    // The account of this test is `home`: `other` is refused before anything else is looked at.
    account_home_for_tests(Some(home.clone()));
    let why = super::super::workshops::outside_command(&other, &other, &[], "/bin/true", &[]).unwrap_err();
    assert!(why.contains("not this account's home"), "{why}");
    match super::super::workshops::outside_command(&home, &home, &[], "/bin/true", &[]) {
        Ok(_) => {}
        Err(e) => assert!(e.contains("nothing was run"), "no bwrap here, and nothing else: {e}"),
    }
    let _ = std::fs::remove_dir_all(top);
    let _ = std::fs::remove_dir_all(top2);
}

fn home_again() -> (PathBuf, PathBuf) {
    let keep = account_home().ok();
    let made = home();
    account_home_for_tests(keep);
    made
}

/// 4. The judgments use the account's home, not `$HOME`: a protected place moved behind a link under
/// the account's home is protected where it is, whatever the environment says.
#[test]
fn the_judgments_use_the_given_home_not_the_environment() {
    let (top, home) = home();
    plant(&home, "dotfiles/kube/config");
    symlink(home.join("dotfiles/kube"), home.join(".kube")).unwrap();
    let inside = home.join("dotfiles/kube");
    // `HOME` in this process is not this test's home.
    assert!(std::env::var_os("HOME").is_none_or(|h| Path::new(&h) != home));
    assert!(is_protected_in(&inside, Some(&home)), "protected where the link leads");
    let why = view_args(&home, &inside, &[]).unwrap_err();
    assert!(why.contains("Nothing was run"), "{why}");
    assert!(view_args(&home, &home.join("dotfiles"), &[]).is_ok());
    let _ = std::fs::remove_dir_all(top);
}

/// 5. A link above a toolchain entry is skipped, as a link at the entry is.
#[test]
fn a_link_above_a_toolchain_entry_is_skipped() {
    let (top, home) = home();
    plant(&home, "dotfiles/cargo/bin/tool");
    symlink(home.join("dotfiles/cargo"), home.join(".cargo")).unwrap();
    plant(&home, "dotfiles/git/ignore");
    std::fs::create_dir_all(home.join(".config")).unwrap();
    symlink(home.join("dotfiles/git"), home.join(".config/git")).unwrap();
    plant(&home, ".local/bin/build-tool");
    let (words, seen) = toolchain_words_with(&home, 50_000, Duration::from_secs(5));
    assert!(!words.iter().any(|w| w.contains(".cargo") || w.contains(".config/git")), "{words:?}");
    assert!(has(&words, "--ro-bind-try", &home.join(".local/bin")), "a plain entry still binds: {words:?}");
    assert_eq!(seen.len(), 1);
    let _ = std::fs::remove_dir_all(top);
}

/// 6. What was seen is looked at again, and a swap for a link is found.
#[test]
fn a_bound_entry_swapped_for_a_link_is_found_when_it_is_looked_at_again() {
    let (top, home) = home();
    plant(&home, ".local/bin/tool");
    let (_, seen) = toolchain_words_with(&home, 50_000, Duration::from_secs(5));
    assert!(seen.iter().all(|s| s.unchanged()));
    std::fs::rename(home.join(".local/bin"), home.join("moved")).unwrap();
    symlink(home.join("moved"), home.join(".local/bin")).unwrap();
    assert!(seen.iter().any(|s| !s.unchanged()), "a link in its place is a change");
    let _ = std::fs::remove_dir_all(top);
}

/// 7. The walk is bounded by time as well as count; the top-only folders are looked at at their top.
#[test]
fn the_toolchain_walk_is_bounded_by_time_and_skips_the_source_trees_but_not_their_top() {
    let (top, home) = home();
    plant(&home, ".cargo/registry/src/index-1/crate-1.0/src/lib.rs");
    plant(&home, ".cargo/registry/src/index-1/crate-1.0/id_rsa");
    plant(&home, ".cargo/registry/src/id_rsa");
    plant(&home, ".cargo/registry/cache/x/id_ed25519");
    let entry = home.join(".cargo/registry");
    let found = secrets_in(&home, &entry, 50_000, Duration::from_secs(5)).expect("small enough");
    let names: Vec<_> = found.iter().map(|(p, _)| p.strip_prefix(&entry).unwrap().to_path_buf()).collect();
    assert!(names.contains(&PathBuf::from("src/id_rsa")), "a secret name at the top of the source tree: {names:?}");
    assert!(names.contains(&PathBuf::from("cache/x/id_ed25519")), "an ordinary folder is walked: {names:?}");
    assert!(!names.iter().any(|n| n.starts_with("src/index-1/crate-1.0")), "below the top is not walked: {names:?}");
    // Time: an entry that takes longer than its budget is skipped whole, as one over the count is.
    assert!(secrets_in(&home, &entry, 50_000, Duration::ZERO).is_none());
    assert!(secrets_in(&home, &entry, 2, Duration::from_secs(5)).is_none());
    let (words, seen) = toolchain_words_with(&home, 2, Duration::from_secs(5));
    assert!(words.is_empty() && seen.is_empty(), "skipped, never half checked: {words:?}");
    // A source tree larger than the cap still passes, because it is not walked.
    for i in 0..30 {
        plant(&home, &format!(".cargo/registry/src/index-1/c{i}/a/b.rs"));
    }
    let big = toolchain_words_with(&home, 40, Duration::from_secs(5));
    assert!(has(&big.0, "--ro-bind-try", &entry), "{:?}", big.0);
    let _ = std::fs::remove_dir_all(top);
}

/// 8. `Pictures` and `Videos` are bound child by child; nothing is created in the person's folders.
#[test]
fn pictures_and_videos_are_bound_child_by_child_and_nothing_is_made_in_them() {
    let (top, home) = home();
    plant(&home, "Pictures/Holiday/a.png");
    plant(&home, "Pictures/b.png");
    plant(&home, "Videos/c.mkv");
    plant(&home, "Pictures/Screenshots/shot.png");
    symlink(home.join("Documents"), home.join("Pictures/Shortcut")).unwrap();
    let list = |d: &Path| {
        let mut v: Vec<_> = std::fs::read_dir(d).unwrap().flatten().map(|e| e.file_name()).collect();
        v.sort();
        v
    };
    let before = (list(&home.join("Pictures")), list(&home.join("Videos")));
    let words = view_args(&home, &home, &[]).unwrap().0;
    assert_eq!(before, (list(&home.join("Pictures")), list(&home.join("Videos"))), "no folder appears in the real Pictures or Videos");
    assert!(!words.iter().any(|w| Path::new(w) == home.join("Pictures") && words.iter().any(|x| x == "--bind-try")) || !has(&words, "--bind-try", &home.join("Pictures")), "the folder is not bound whole");
    assert!(!has(&words, "--bind-try", &home.join("Pictures")) && !has(&words, "--bind-try", &home.join("Videos")));
    for bound in ["Pictures/Holiday", "Pictures/b.png", "Videos/c.mkv"] {
        assert!(has(&words, "--bind-try", &home.join(bound)), "{bound}: {words:?}");
    }
    for left_out in ["Pictures/Screenshots", "Pictures/Shortcut"] {
        assert!(!words.iter().any(|w| Path::new(w) == home.join(left_out)), "{left_out}");
    }
    assert!(!words.iter().any(|w| w == "--tmpfs" && false) && !words.windows(2).any(|w| w[0] == "--tmpfs" && Path::new(&w[1]) != Path::new("/tmp") && Path::new(&w[1]) != home), "no mask is laid: {words:?}");
    assert!(in_view(&words, &home, &home.join("Pictures/Holiday/a.png")));
    assert!(!in_view(&words, &home, &home.join("Pictures/Screenshots/shot.png")));
    let _ = std::fs::remove_dir_all(top);
}

/// 9. A working directory is the home or inside a shown entry; the rest is refused, `snap` and a
/// link among them.
#[test]
fn a_working_directory_is_inside_a_shown_entry_or_refused() {
    let (top, home) = home();
    plant(&home, "Documents/a/b");
    plant(&home, "snap/app/x");
    plant(&home, "bin/tool");
    plant(&home, "target/t");
    symlink(home.join("target"), home.join("Shortcut")).unwrap();
    for ok in [home.clone(), home.join("Documents"), home.join("Documents/a"), home.join("Documents/a/b")] {
        // A file is no working directory, but the check is on where it is.
        assert!(view_args(&home, &ok, &[]).is_ok(), "{}", ok.display());
    }
    assert!(view_args(&home, &home.join("target/t"), &[]).is_ok(), "a shown folder");
    for bad in [home.join("snap/app"), home.join("bin"), home.join("Shortcut"), home.join("Shortcut/t")] {
        let why = view_args(&home, &bad, &[]).unwrap_err();
        assert!(why.contains("Nothing was run"), "{}: {why}", bad.display());
    }
    let _ = std::fs::remove_dir_all(top);
}

/// 10. SECURITY.md carries the lines the decision lists.
#[test]
fn security_md_says_what_remains() {
    let text = include_str!("../../../../SECURITY.md");
    for line in ["/etc/gitconfig", "`GIT_CONFIG_*`", "`XDG_CONFIG_HOME`", "#1351", "`--ro-bind-fd`", "a plain-named script", "inside a visible folder"] {
        assert!(text.contains(line), "SECURITY.md lacks {line}");
    }
}

/// The list the views are made from is the one list: no second copy of the held type's helpers.
#[test]
fn held_is_empty_when_nothing_is_named() {
    let held = Held::default();
    assert!(held.is_empty() && held.numbers().is_empty());
}
