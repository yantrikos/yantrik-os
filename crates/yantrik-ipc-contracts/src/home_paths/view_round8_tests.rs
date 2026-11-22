//! Round 8 of #1265: the decision on the round 7 security read, one test (or more) per item. The
//! ones that need a live `bwrap` skip without one, and fail instead where `YANTRIK_REQUIRE_BWRAP=1`
//! (the project runner), so a skipped test cannot count as passed.

use super::tests::{home, plant, run_sealed};
use super::*;
use crate::home_paths::view_env;
use crate::home_paths::view_places::{Seen, Verdict};
use crate::home_paths::workshops::{outside_command, outside_command_with, skip_live_test};
use std::os::unix::fs::symlink;

/// Run `cmd` in the view over `home`, in `cwd`, as a door starts it. `None` (after the skip) when no
/// view can be made here.
fn live(home: &Path, cwd: &Path, reach: &[PathBuf], cmd: &str) -> Option<String> {
    live_with(home, cwd, reach, None, cmd)
}

fn live_with(home: &Path, cwd: &Path, reach: &[PathBuf], env: Option<&[(String, String)]>, cmd: &str) -> Option<String> {
    if super::super::workshops::find_bwrap().is_none() {
        skip_live_test("no bwrap");
        return None;
    }
    let words = ["-c".to_string(), cmd.to_string()];
    let (program, args, held) = match env {
        Some(env) => outside_command_with(home, cwd, reach, env, "/bin/bash", &words),
        None => outside_command(home, cwd, reach, "/bin/bash", &words),
    }
    .unwrap();
    let out = run_sealed(program, args, held, cwd)?;
    let err = String::from_utf8_lossy(&out.stderr);
    if err.contains("bwrap:") {
        skip_live_test(&err);
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// 1. A `reach` of a regular file (`~/.npmrc`) is bound and read, not written.
#[test]
fn a_reach_of_a_regular_file_is_bound_and_read_only() {
    let (top, home) = home();
    plant(&home, ".npmrc");
    std::fs::create_dir_all(home.join("Documents")).unwrap();
    let cmd = "cat ../.npmrc; echo; if echo x >> ../.npmrc 2>/dev/null; then echo WROTE; else echo READONLY; fi";
    if let Some(out) = live(&home, &home.join("Documents"), &[home.join(".npmrc")], cmd) {
        assert!(out.contains("canary-1265") && out.contains("READONLY"), "{out}");
    }
    assert_eq!(std::fs::read_to_string(home.join(".npmrc")).unwrap(), "canary-1265");
    let _ = std::fs::remove_dir_all(top);
}

/// 2. The environment is an allow list: nothing but the short list, `HOME` the account's, in the view.
#[test]
fn the_command_sees_only_the_allow_listed_environment() {
    let (top, home) = home();
    std::fs::create_dir_all(home.join("Documents")).unwrap();
    std::env::set_var("YANTRIK_TEST_API_KEY_1265", "hunter2");
    std::env::set_var("GIT_SSH_COMMAND", "evil");
    std::env::set_var("SSH_AUTH_SOCK", "/tmp/agent");
    std::env::set_var("LC_ALL", "C");
    let cwd = home.join("Documents");
    if let Some(out) = live(&home, &cwd, &[], "env") {
        let names: Vec<&str> = out.lines().filter_map(|l| l.split_once('=')).map(|(k, _)| k).collect();
        for never in ["YANTRIK_TEST_API_KEY_1265", "GIT_SSH_COMMAND", "SSH_AUTH_SOCK", "XDG_RUNTIME_DIR", "DBUS_SESSION_BUS_ADDRESS", "GNUPGHOME"] {
            assert!(!names.contains(&never), "{never} reached the command: {out}");
        }
        let allowed = |n: &&str| matches!(*n, "PATH" | "HOME" | "LANG" | "TERM" | "USER" | "PWD" | "OLDPWD" | "SHLVL" | "_") || n.starts_with("LC_");
        assert!(names.iter().all(allowed), "only the allow list (and what bash makes): {out}");
        assert!(out.lines().any(|l| l == format!("HOME={}", home.display())), "{out}");
        assert!(out.contains("LC_ALL=C"));
    }
    std::env::remove_var("YANTRIK_TEST_API_KEY_1265");
    std::env::remove_var("GIT_SSH_COMMAND");
    std::env::remove_var("SSH_AUTH_SOCK");
    let _ = std::fs::remove_dir_all(top);
}

/// 2. Every door builds its environment in one place, and none keeps a deny list.
#[test]
fn every_door_takes_its_environment_from_the_one_function() {
    for (name, src) in [
        ("job.rs", include_str!("../../../yantrik-agent-terminal/src/job.rs")),
        ("spawn.rs", include_str!("../../../yantrik-companion-tools/src/spawn.rs")),
    ] {
        assert!(src.contains("view_env::"), "{name} does not use view_env");
    }
    let git = include_str!("view_git.rs");
    assert!(!git.contains("unsetenv") && !git.contains("GIT_VARS"), "a deny list is back in view_git");
    let words = view_env::words(&view_env::allowed(Vec::new(), Path::new("/home/p"), &[]));
    assert_eq!(words[0], "--clearenv");
}

/// 3. A photo folder with many files is never a reason to refuse: past the cap the loose files are
/// hidden, the folders stay.
#[test]
fn a_big_pictures_folder_hides_its_loose_files_and_keeps_its_folders() {
    let (top, home) = home();
    plant(&home, "Pictures/Holiday/a.png");
    plant(&home, "Pictures/Trip/b.png");
    for i in 0..MAX_CHILD_FILES {
        plant(&home, &format!("Pictures/f{i:04}.jpg"));
    }
    let words = view_args(&home, &home, &[]).unwrap().0;
    let bound = |p: &str| words.windows(3).any(|w| w[0] == "--bind-try" && Path::new(&w[2]) == home.join(p));
    assert!(bound("Pictures/f0000.jpg") && bound(&format!("Pictures/f{:04}.jpg", MAX_CHILD_FILES - 1)), "at the cap the files are shown");
    plant(&home, "Pictures/over.jpg");
    let words = view_args(&home, &home, &[]).unwrap().0;
    let bound = |p: &str| words.windows(3).any(|w| w[0] == "--bind-try" && Path::new(&w[2]) == home.join(p));
    assert!(bound("Pictures/Holiday") && bound("Pictures/Trip"), "the folders stay: {} words", words.len());
    assert!(!bound("Pictures/f0000.jpg") && !bound("Pictures/over.jpg"), "over the cap the loose files are hidden");
    assert!(words.len() < 40, "the words are the folders and the rest, not the files: {}", words.len());
    let _ = std::fs::remove_dir_all(top);
}

/// 4. A child that is gone is dropped; a link or a change of kind refuses.
#[test]
fn a_vanished_child_is_dropped_and_only_a_link_or_a_new_kind_refuses() {
    let (top, home) = home();
    plant(&home, "Pictures/a.png");
    let file = home.join("Pictures/a.png");
    let seen = Seen::of(&file).unwrap();
    assert_eq!(seen.verdict(), Verdict::Same);
    std::fs::remove_file(&file).unwrap();
    assert_eq!(seen.verdict(), Verdict::Gone, "a download that finished or a temporary file");
    plant(&home, "Pictures/a.png");
    assert_eq!(seen.verdict(), Verdict::Same, "a file made new under its name is that name");
    std::fs::remove_file(&file).unwrap();
    symlink(home.join("Documents"), &file).unwrap();
    assert_eq!(seen.verdict(), Verdict::Changed, "a link");
    std::fs::remove_file(&file).unwrap();
    std::fs::create_dir(&file).unwrap();
    assert_eq!(seen.verdict(), Verdict::Changed, "a file is a folder now");
    let _ = std::fs::remove_dir_all(top);
}

/// 4. In the live view a file written straight into `Pictures` fails loudly; one in a child folder works.
#[test]
fn a_file_written_directly_in_pictures_fails_loudly() {
    let (top, home) = home();
    plant(&home, "Pictures/Holiday/a.png");
    plant(&home, "Videos/c.mkv");
    let cmd = "if echo x > ~/Pictures/new.png 2>/dev/null; then echo PICS-WROTE; else echo PICS-FAILED; fi; \
               if echo x > ~/Videos/new.mkv 2>/dev/null; then echo VIDS-WROTE; else echo VIDS-FAILED; fi; \
               echo y > ~/Pictures/Holiday/b.png && echo CHILD-WROTE";
    let env = view_env::allowed(Vec::new(), &home, &[]);
    if let Some(out) = live_with(&home, &home, &[], Some(&env), cmd) {
        assert!(out.contains("PICS-FAILED") && out.contains("VIDS-FAILED") && out.contains("CHILD-WROTE"), "{out}");
        assert!(home.join("Pictures/Holiday/b.png").exists(), "a child folder is the person's own");
        assert!(!home.join("Pictures/new.png").exists() && !home.join("Videos/new.mkv").exists());
    }
    let _ = std::fs::remove_dir_all(top);
}

/// 7. The test seam is not in a release build.
#[test]
fn the_test_seam_is_gated() {
    let places = include_str!("view_places.rs");
    let outside = include_str!("outside_view.rs");
    assert!(places.contains("#[cfg(any(test, feature = \"test-seams\"))]\npub fn account_home_for_tests"));
    assert!(outside.contains("#[cfg(any(test, feature = \"test-seams\"))]\npub use super::view_places::account_home_for_tests"));
}

/// The doors that need a live view have to fail, not skip, on the project runner.
#[test]
fn ci_requires_bwrap_on_the_test_job() {
    let ci = include_str!("../../../../.github/workflows/ci.yml");
    assert!(ci.contains("YANTRIK_REQUIRE_BWRAP: \"1\""));
    assert_eq!(super::super::workshops::REQUIRE_BWRAP, "YANTRIK_REQUIRE_BWRAP");
}
