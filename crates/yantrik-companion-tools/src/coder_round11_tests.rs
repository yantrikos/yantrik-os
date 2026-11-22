//! Round 11 of #1265 (the 07:11 and 07:13 reads of round 9 and 10): a failed archive move, the fallback
//! and the lock of `promote`, strict and swept staging folders, the script sandbox's places, scripts
//! folder, capabilities and environment. The live ones need `bwrap`: they skip without it and fail
//! instead where `YANTRIK_REQUIRE_BWRAP=1` (the project runner).

use crate::coder_promote::{hook, promote};
use crate::coder_round10_tests::{in_sandbox, in_sandbox_as, scratch};
use crate::coder_script_view::{mind_env, script_view};
use crate::coder_deps::{deps_root_in, staging_in};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn header(name: &str, size: usize) -> [u8; 512] {
    let mut h = [0u8; 512];
    h[..name.len()].copy_from_slice(name.as_bytes());
    h[100..107].copy_from_slice(b"0000644");
    h[124..135].copy_from_slice(format!("{size:011o}").as_bytes());
    h[156] = b'0';
    h[257..263].copy_from_slice(b"ustar\0");
    h[148..156].fill(32);
    let sum: u64 = h.iter().map(|b| *b as u64).sum();
    h[148..155].copy_from_slice(format!("{sum:06o}\0").as_bytes());
    h
}

/// A ustar stream of one plain file.
fn tar(name: &str, body: &str) -> Vec<u8> {
    let mut out = header(name, body.len()).to_vec();
    out.extend(body.as_bytes());
    out.extend(vec![0u8; (512 - body.len() % 512) % 512 + 1024]);
    out
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

/// A target holding `old` (a file of that name), in a scratch folder with a staging root beside it.
fn with_old(tag: &str) -> (PathBuf, PathBuf, PathBuf) {
    let top = scratch(tag);
    let dest = top.join("deps");
    std::fs::create_dir_all(&dest).unwrap();
    std::fs::write(dest.join("old"), "old").unwrap();
    let stage = top.join("stage");
    (top, dest, stage)
}

/// 1. The archive move fails after the exchange: the old folder is back in place, the failure is told as
/// one, and nothing of the old copy is removed.
#[test]
fn a_failed_archive_move_puts_the_old_folder_back() {
    let (top, dest, stage) = with_old("r11-archive");
    hook::set(hook::FAIL_ARCHIVE);
    let why = promote(&tar("new", "n"), &dest, &stage).unwrap_err();
    hook::set(0);
    assert!(why.contains("back in place") && why.contains("nothing was changed"), "{why}");
    assert_eq!(std::fs::read_to_string(dest.join("old")).unwrap(), "old", "the old folder is in place again");
    assert!(!dest.join("new").exists(), "the failed install is not in place");
    assert!(names(&top).iter().all(|n| !n.contains("replaced")), "no archive was made");
    assert_eq!(names(&stage).len(), 0, "the staging folder holds no old copy, so it was cleared");
    let _ = std::fs::remove_dir_all(top);
}

/// 1. If the exchange back fails too, the staging folder is left untouched and the report names the old copy.
#[test]
fn if_the_exchange_back_fails_the_staging_folder_stays_and_names_the_old_copy() {
    let (top, dest, stage) = with_old("r11-back");
    hook::set(hook::FAIL_ARCHIVE | hook::FAIL_BACK);
    let why = promote(&tar("new", "n"), &dest, &stage).unwrap_err();
    hook::set(0);
    assert!(why.contains("the old copy is at ") && why.contains("could not be put back"), "{why}");
    let at = why.rsplit("the old copy is at ").next().unwrap().trim().to_string();
    assert!(at.starts_with(stage.to_str().unwrap()), "{at}");
    assert_eq!(std::fs::read_to_string(Path::new(&at).join("old")).unwrap(), "old", "the old copy is where it says");
    assert!(dest.join("new").exists(), "the new folder is in place");
    let _ = std::fs::remove_dir_all(top);
}

/// 5. The two-rename path is taken for `EINVAL` (no exchange here): the old folder is kept aside, the new in place.
#[test]
fn the_two_rename_path_runs_when_the_file_system_has_no_exchange() {
    let (top, dest, stage) = with_old("r11-two");
    hook::set(hook::NO_EXCHANGE);
    let n = promote(&tar("new", "n"), &dest, &stage);
    hook::set(0);
    assert_eq!(n.unwrap(), 1);
    assert!(dest.join("new").exists() && dest.join("old").exists(), "the new folder holds both");
    let archive = names(&top).into_iter().find(|n| n.starts_with("deps.replaced-")).expect("the old folder is kept aside");
    assert_eq!(std::fs::read_to_string(top.join(archive).join("old")).unwrap(), "old");
    let _ = std::fs::remove_dir_all(top);
}

/// 5. Any other refusal of the exchange (a full disk) does not take the fallback: the old folder stays whole
/// and nothing is moved aside.
#[test]
fn a_full_disk_does_not_take_the_fallback() {
    let (top, dest, stage) = with_old("r11-full");
    hook::set(hook::FULL_DISK);
    let why = promote(&tar("new", "n"), &dest, &stage).unwrap_err();
    hook::set(0);
    assert!(why.contains("no space left"), "{why}");
    assert_eq!(names(&dest), ["old"]);
    assert!(names(&top).iter().all(|n| !n.contains("replaced")), "the target was never absent or moved");
    let _ = std::fs::remove_dir_all(top);
}

/// 5. Two promotes of one target at once: neither loses the other's result.
#[test]
fn two_promotes_of_one_target_at_once_keep_both_results() {
    let (top, dest, stage) = with_old("r11-race");
    let threads: Vec<_> = ["alpha", "beta", "gamma", "delta"]
        .map(|n| {
            let (dest, stage) = (dest.clone(), stage.clone());
            std::thread::spawn(move || promote(&tar(n, n), &dest, &stage).unwrap())
        })
        .into();
    threads.into_iter().for_each(|t| {
        t.join().unwrap();
    });
    assert_eq!(names(&dest), ["alpha", "beta", "delta", "gamma", "old"], "every install is in the live folder");
    let _ = std::fs::remove_dir_all(top);
}

/// 5. The lock is waited for with a deadline, never forever.
#[test]
fn the_staging_lock_has_a_deadline() {
    let top = scratch("r11-lock");
    let a = crate::coder_promote::staging_root(&top.join("stage")).unwrap();
    let b = crate::coder_promote::staging_root(&top.join("stage")).unwrap();
    let held = a.lock(std::time::Duration::from_secs(1)).unwrap();
    let t = std::time::Instant::now();
    assert!(b.lock(std::time::Duration::from_millis(150)).unwrap_err().contains("nothing was changed"));
    assert!(t.elapsed() < std::time::Duration::from_secs(5));
    drop(held);
    assert!(b.lock(std::time::Duration::from_secs(1)).is_ok(), "free again once released");
    let _ = std::fs::remove_dir_all(top);
}

/// 6. A per-call folder is made strictly (an existing name is an error), and a stale one older than a day is
/// swept: its `in` and the folder go, but one that holds a `whole` (maybe the only old copy) stays.
#[test]
fn staging_folders_are_strict_and_stale_ones_are_swept_but_never_an_old_copy() {
    let top = scratch("r11-stale");
    let stage = top.join("stage");
    let dest = top.join("deps");
    let root = crate::coder_promote::staging_root(&stage).unwrap();
    root.mkdir_new(std::ffi::OsStr::new("x"), 0o700).unwrap();
    assert!(root.mkdir_new(std::ffi::OsStr::new("x"), 0o700).is_err(), "an existing name is not reused as success");
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3 * 24 * 3600);
    let age = |p: &Path| std::fs::File::open(p).unwrap().set_modified(old).unwrap();
    std::fs::create_dir_all(stage.join(".deps.call-1-1/in")).unwrap();
    std::fs::write(stage.join(".deps.call-1-1/in/leftover"), "l").unwrap();
    std::fs::create_dir_all(stage.join(".deps.call-2-2/whole")).unwrap();
    std::fs::write(stage.join(".deps.call-2-2/whole/precious"), "p").unwrap();
    std::fs::create_dir_all(stage.join(".deps.call-3-3/in")).unwrap();
    for n in [".deps.call-1-1", ".deps.call-2-2"] {
        age(&stage.join(n));
    }
    promote(&tar("a", "1"), &dest, &stage).unwrap();
    assert!(!stage.join(".deps.call-1-1").exists(), "a stale folder with no old copy is swept");
    assert_eq!(std::fs::read_to_string(stage.join(".deps.call-2-2/whole/precious")).unwrap(), "p", "one that may hold the old copy stays");
    assert!(stage.join(".deps.call-3-3").exists(), "a young one is left");
    let _ = std::fs::remove_dir_all(top);
}

fn view(home: &Path, scripts: &Path, language: &str, mind: &HashMap<String, String>) -> Result<crate::coder_script_view::ScriptView, String> {
    std::fs::create_dir_all(scripts.join("output")).unwrap();
    script_view(home, &scripts.join("output"), scripts, language, mind)
}

/// Addendum 1: the empty dependency folders are made before the first script, through the descriptor code,
/// and a link where one belongs refuses.
#[test]
fn the_empty_dependency_folders_are_made_and_a_link_in_their_place_refuses() {
    let top = scratch("r11-ensure");
    let (home, scripts) = (top.join("home"), top.join("scripts"));
    let v = view(&home, &scripts, "python", &HashMap::new()).unwrap();
    for (source, dest) in &v.reads {
        assert!(source.is_dir() && source.starts_with(deps_root_in(&home)), "{source:?}");
        assert!(dest.is_dir() && dest.starts_with(&scripts), "the destination is a folder of the workspace: {dest:?}");
    }
    assert!(names(&deps_root_in(&home)).contains(&"python".to_string()) && names(&deps_root_in(&home)).contains(&"node_modules".to_string()));
    assert_eq!(names(&staging_in(&home)).len(), 0);
    // a link in a source's place
    let out = std::env::temp_dir().join(format!("yantrik-r11-elsewhere-{}", std::process::id()));
    std::fs::create_dir_all(&out).unwrap();
    std::fs::remove_dir(deps_root_in(&home).join("python")).unwrap();
    std::os::unix::fs::symlink(&out, deps_root_in(&home).join("python")).unwrap();
    assert!(view(&home, &scripts, "python", &HashMap::new()).is_err(), "a link where the python folder belongs refuses");
    let _ = std::fs::remove_dir_all(top);
    let _ = std::fs::remove_dir_all(out);
}

/// Addendum 5: a link at either bind destination of the workspace refuses the run, and so does a file there.
#[test]
fn a_link_at_a_bind_destination_refuses_the_run() {
    for dest in [".deps", "node_modules"] {
        let top = scratch("r11-link");
        let (home, scripts) = (top.join("home"), top.join("scripts"));
        std::fs::create_dir_all(&scripts).unwrap();
        let target = top.join("elsewhere");
        std::fs::create_dir_all(&target).unwrap();
        std::os::unix::fs::symlink(&target, scripts.join(dest)).unwrap();
        let why = view(&home, &scripts, "python", &HashMap::new()).err().expect("a link refuses");
        assert!(why.contains("Script run refused") && why.contains("link"), "{why}");
        std::fs::remove_file(scripts.join(dest)).unwrap();
        std::fs::write(scripts.join(dest), "x").unwrap();
        assert!(view(&home, &scripts, "python", &HashMap::new()).is_err(), "a file there refuses too");
        let _ = std::fs::remove_dir_all(top);
    }
}

/// Addendum 3: a Mind's loader names and `PATH` do not reach the words, the door's own names win, and the
/// rest passes; `PYTHONSAFEPATH` is set for Python only.
#[test]
fn a_minds_loader_names_do_not_reach_the_sandbox_and_the_doors_names_win() {
    let top = scratch("r11-env");
    let (home, scripts) = (top.join("home"), top.join("scripts"));
    let mind: HashMap<String, String> = [("LD_PRELOAD", "/evil.so"), ("PYTHONPATH", "/evil"), ("PATH", "/evil"), ("NODE_OPTIONS", "--require /evil"), ("YANTRIK_OUTPUT_DIR", "/evil"), ("FRIENDLY", "yes")]
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .into();
    let kept: Vec<String> = mind_env(&mind).into_iter().map(|(k, _)| k).collect();
    assert_eq!(kept.iter().filter(|k| k.as_str() != "YANTRIK_OUTPUT_DIR").collect::<Vec<_>>(), ["FRIENDLY"], "{kept:?}");
    let v = view(&home, &scripts, "python", &mind).unwrap();
    let c = crate::spawn::sandboxed_script(crate::spawn::SandboxMode::Strict, &scripts, &scripts.join("output"), &v.reads, &v.extra, "x");
    let words: Vec<String> = c.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
    assert!(!words.iter().any(|w| w.contains("/evil") && w != &scripts.join("output").to_string_lossy()) || words.windows(3).all(|w| !(w[0] == "--setenv" && w[2].contains("evil"))), "{words:?}");
    let set = |k: &str| words.windows(3).find(|w| w[0] == "--setenv" && w[1] == k).map(|w| w[2].clone());
    assert_eq!(set("LD_PRELOAD"), None);
    assert_eq!(set("NODE_OPTIONS"), None);
    assert_eq!(set("FRIENDLY").as_deref(), Some("yes"));
    assert_eq!(set("PYTHONPATH"), Some(scripts.join(".deps").to_string_lossy().into_owned()), "the door's PYTHONPATH, not the Mind's");
    assert_eq!(set("YANTRIK_OUTPUT_DIR"), Some(scripts.join("output").to_string_lossy().into_owned()), "the door's name wins");
    assert_ne!(set("PATH").as_deref(), Some("/evil"));
    assert_eq!(set("PYTHONSAFEPATH").as_deref(), Some("1"));
    let node = view(&home, &scripts, "node", &mind).unwrap();
    assert!(!node.extra.iter().any(|(k, _)| k == "PYTHONSAFEPATH" || k == "PYTHONPATH"));
    let _ = std::fs::remove_dir_all(top);
}

/// Addendum 1, live: with no dependencies folder at all, a script cannot write either destination, nor the
/// scripts folder; it can write the output folder.
#[test]
fn with_no_dependencies_folder_a_script_cannot_write_either_destination() {
    let top = scratch("r11-none");
    let (home, scripts) = (top.join("home"), top.join("scripts"));
    assert!(!deps_root_in(&home).exists());
    let probe = "for p in .deps node_modules .; do if echo x > \"$YANTRIK_SCRIPTS_DIR/$p/planted\" 2>/dev/null; then echo \"WROTE $p\"; else echo \"READONLY $p\"; fi; done; \
                 if echo ok > \"$YANTRIK_OUTPUT_DIR/made\" 2>/dev/null; then echo OUTPUT_OK; fi";
    if let Some(said) = in_sandbox(&home, &scripts, &[], probe) {
        assert!(!said.contains("WROTE"), "{said}");
        assert!(said.contains("READONLY .deps") && said.contains("READONLY node_modules") && said.contains("READONLY ."), "{said}");
        assert!(said.contains("OUTPUT_OK") && scripts.join("output/made").is_file(), "the output folder is writable: {said}");
        assert!(!scripts.join(".deps/planted").exists() && !scripts.join("node_modules/planted").exists() && !scripts.join("planted").exists());
    }
    let _ = std::fs::remove_dir_all(top);
}

/// Addendum 2, live: a script cannot create a `.py` that a later script imports, and a module in the
/// scripts folder does not shadow anything (Python runs with `PYTHONSAFEPATH=1`).
#[test]
fn a_script_cannot_plant_code_for_a_later_script() {
    let top = scratch("r11-plant");
    let (home, scripts) = (top.join("home"), top.join("scripts"));
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(scripts.join("shadowme.py"), "X = 'from-the-scripts-folder'\n").unwrap();
    std::fs::write(scripts.join("main.py"), "import shadowme\nprint(shadowme.X)\n").unwrap();
    let first = "echo 'X = 1' > \"$YANTRIK_SCRIPTS_DIR/json.py\" 2>/dev/null && echo PLANTED; echo 'X = 1' > \"$YANTRIK_SCRIPTS_DIR/planted_mod.py\" 2>/dev/null && echo PLANTED2; echo END";
    if let Some(said) = in_sandbox(&home, &scripts, &[], first) {
        assert!(said.contains("END") && !said.contains("PLANTED"), "{said}");
        assert!(!scripts.join("json.py").exists() && !scripts.join("planted_mod.py").exists());
        let second = "command -v python3 >/dev/null || echo NOPYTHON; python3 -c 'import planted_mod' 2>&1 | tail -1; python3 \"$YANTRIK_SCRIPTS_DIR/main.py\" 2>&1 | tail -1";
        let said = in_sandbox(&home, &scripts, &[], second).unwrap();
        if said.contains("NOPYTHON") {
            skip_live_test_here("no python3 in the sandbox's reach");
        } else {
            assert!(said.matches("ModuleNotFoundError").count() == 2, "neither the planted module nor a neighbour of the script imports: {said}");
            assert!(!said.contains("from-the-scripts-folder"), "{said}");
        }
    }
    let _ = std::fs::remove_dir_all(top);
}

fn skip_live_test_here(why: &str) {
    yantrik_ipc_contracts::home_paths::workshops::skip_live_test(why);
}

/// 3, live: from inside, the dependencies cannot be unmounted or remounted writable (no capabilities).
#[test]
fn the_read_only_dependencies_cannot_be_unmounted_from_inside() {
    let top = scratch("r11-umount");
    let (home, scripts) = (top.join("home"), top.join("scripts"));
    let probe = "command -v umount >/dev/null || echo NOUMOUNT; \
                 umount \"$YANTRIK_SCRIPTS_DIR/.deps\" 2>/dev/null && echo UNMOUNTED || echo UMOUNT_REFUSED; \
                 mount -o remount,rw \"$YANTRIK_SCRIPTS_DIR/.deps\" 2>/dev/null && echo REMOUNTED || echo REMOUNT_REFUSED; \
                 if echo x > \"$YANTRIK_SCRIPTS_DIR/.deps/after\" 2>/dev/null; then echo WROTE; else echo READONLY; fi; \
                 grep -c CapEff /proc/self/status >/dev/null; grep CapEff /proc/self/status";
    if let Some(said) = in_sandbox(&home, &scripts, &[], probe) {
        assert!(!said.contains("UNMOUNTED") && !said.contains("REMOUNTED") && !said.contains("WROTE"), "{said}");
        assert!(said.contains("READONLY") && said.contains("UMOUNT_REFUSED"), "{said}");
        assert!(said.contains("CapEff:\t0000000000000000"), "no capability is held: {said}");
        let _ = in_sandbox_as; // the node variant is in the round 10 import test
    }
    let _ = std::fs::remove_dir_all(top);
}
