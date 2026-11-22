use super::*;

pub(super) fn home() -> (PathBuf, PathBuf) {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let top = std::env::temp_dir().join(format!(
        "yantrik-view-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    let home = top.join("home");
    std::fs::create_dir_all(&home).unwrap();
    (top.clone(), home.canonicalize().unwrap())
}

pub(super) fn plant(home: &Path, rel: &str) {
    let p = home.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, "canary-1265").unwrap();
}

/// Whether `path` is in the view the words describe: under a bound place and under no mask laid
/// over it afterwards (a tmpfs, in the words' order).
pub(super) fn in_view(words: &[String], home: &Path, path: &Path) -> bool {
    let mut present = false;
    let mut i = 0;
    while i < words.len() {
        match words[i].as_str() {
            "--tmpfs" => {
                let at = Path::new(&words[i + 1]);
                if path.starts_with(at) {
                    present = false;
                }
                i += 2;
            }
            "--bind" | "--ro-bind" | "--bind-try" | "--ro-bind-try" => {
                // `/dev/null` over a file is a mask: the file is there and empty.
                if path.starts_with(&words[i + 2]) {
                    present = words[i + 1] != "/dev/null";
                }
                i += 3;
            }
            "--dir" => i += 2,
            "--setenv" => i += 3,
            other => panic!("unexpected word {other}"),
        }
    }
    present && path.starts_with(home)
}

#[test]
fn every_protected_place_is_absent_from_the_view() {
    let (top, home) = home();
    for place in PROTECTED {
        // The four under a visible folder are folders, with a file in them.
        match place.contains('/') && !place.starts_with('.') {
            true => plant(&home, &format!("{place}/canary-1265")),
            false => plant(&home, place),
        }
    }
    plant(&home, "Pictures/Screenshots/shot.png");
    plant(&home, "Documents/ok-1265");
    let words = view_args(&home, &home.join("Documents"), &[]).unwrap();
    for place in PROTECTED {
        let at = home.join(place);
        assert!(!in_view(&words, &home, &at), "{place} is in the view");
    }
    assert!(
        !in_view(&words, &home, &home.join("Pictures/Screenshots/shot.png")),
        "the capitalised folder is masked too"
    );
    assert!(
        in_view(&words, &home, &home.join("Documents/ok-1265")),
        "a visible folder is read"
    );
    let _ = std::fs::remove_dir_all(top);
}

#[test]
fn an_unknown_new_dot_folder_is_absent_because_the_rule_is_an_allow_list() {
    let (top, home) = home();
    plant(&home, ".newthing/canary-1265");
    plant(&home, ".cache/x");
    plant(&home, "bin/tool");
    let words = view_args(&home, &home, &[]).unwrap();
    for gone in [".newthing/canary-1265", ".cache/x", "bin/tool"] {
        assert!(!in_view(&words, &home, &home.join(gone)), "{gone}");
    }
    let _ = std::fs::remove_dir_all(top);
}

#[test]
fn a_visible_name_that_links_into_a_hidden_place_is_not_a_way_in() {
    let (top, home) = home();
    plant(&home, ".ssh/id");
    std::os::unix::fs::symlink(home.join(".ssh"), home.join("keys")).unwrap();
    plant(&home, "Notes/a");
    let words = view_args(&home, &home, &[]).unwrap();
    assert!(!words.iter().any(|w| w.ends_with("/keys")), "{words:?}");
    assert!(words.iter().any(|w| w.ends_with("/Notes")));
    let _ = std::fs::remove_dir_all(top);
}

/// The binds are the visible folders, the working directory's, the toolchain list and the reach,
/// and nothing else: the list equals the allow list exactly.
#[test]
fn the_bind_list_equals_the_allow_list() {
    let (top, home) = home();
    for d in [
        "Documents",
        "Music",
        ".ssh",
        ".config/gh",
        ".local/share/yantrik",
    ] {
        std::fs::create_dir_all(home.join(d)).unwrap();
    }
    for rel in TOOLCHAIN {
        std::fs::create_dir_all(home.join(rel)).unwrap();
    }
    let words = view_args(&home, &home.join("Documents"), &[]).unwrap();
    let mut rw = Vec::new();
    let mut ro = Vec::new();
    for w in words.chunks(1).enumerate().map(|(i, _)| i) {
        match words[w].as_str() {
            "--bind-try" => rw.push(PathBuf::from(&words[w + 1])),
            "--ro-bind-try" => ro.push(PathBuf::from(&words[w + 1])),
            _ => {}
        }
    }
    assert_eq!(rw, [home.join("Documents"), home.join("Music")]);
    let mut want: Vec<PathBuf> = TOOLCHAIN.iter().map(|t| home.join(t)).collect();
    want.sort();
    ro.sort();
    assert_eq!(ro, want);
    assert!(
        words
            .windows(2)
            .any(|w| w[0] == "--tmpfs" && w[1] == "/tmp"),
        "a private /tmp"
    );
    assert!(
        words
            .windows(2)
            .any(|w| w[0] == "--tmpfs" && Path::new(&w[1]) == home),
        "the home is a fresh tmpfs"
    );
    let _ = std::fs::remove_dir_all(top);
}

#[test]
fn no_toolchain_entry_is_or_holds_a_protected_place() {
    let home = Path::new("/home/ann");
    for rel in TOOLCHAIN {
        assert!(!is_protected(&home.join(rel)), "{rel} is protected");
        for place in PROTECTED {
            assert!(
                !Path::new(place).starts_with(rel) && !Path::new(rel).starts_with(place),
                "{rel} holds or is inside {place}"
            );
        }
    }
}

/// Adding a fifth entry of `PROTECTED` inside a visible folder without its mask fails: the masks
/// are derived, and each of them is protected.
#[test]
fn every_protected_place_under_a_visible_folder_has_a_mask() {
    let (top, home) = home();
    let inside: Vec<&&str> = PROTECTED
        .iter()
        .filter(|p| p.contains('/') && !p.starts_with('.'))
        .collect();
    assert_eq!(
        inside.len(),
        4,
        "{inside:?}: a new one needs a look at the view"
    );
    for place in &inside {
        std::fs::create_dir_all(home.join(place)).unwrap();
    }
    let masks = derived_masks(&home);
    // `snap/…` is not here: `snap` is not in the view at all, so there is nothing to mask in it.
    let masked: Vec<_> = inside.iter().filter(|p| !p.starts_with("snap/")).collect();
    assert_eq!(masks.len(), masked.len());
    for place in &masked {
        assert!(masks.contains(&home.join(place)), "{place}");
    }
    let _ = std::fs::remove_dir_all(top);
}

#[test]
fn a_working_directory_in_a_hidden_place_is_refused_never_widened() {
    let (top, home) = home();
    std::fs::create_dir_all(home.join(".config/gh")).unwrap();
    std::fs::create_dir_all(home.join(".newthing")).unwrap();
    for dir in [
        home.join(".config/gh"),
        home.join(".newthing"),
        home.join("Pictures/Screenshots"),
    ] {
        std::fs::create_dir_all(&dir).unwrap();
        let why = view_args(&home, &dir, &[]).unwrap_err();
        assert!(why.contains("Nothing was run"), "{why}");
    }
    assert!(view_args(&home, &home, &[]).is_ok());
    let _ = std::fs::remove_dir_all(top);
}

#[test]
fn reach_binds_one_place_read_only_and_only_inside_the_home() {
    let (top, home) = home();
    plant(&home, ".ssh/id");
    let ssh = home.join(".ssh");
    let words = view_args(&home, &home, std::slice::from_ref(&ssh)).unwrap();
    assert!(words
        .windows(3)
        .any(|w| w[0] == "--ro-bind" && Path::new(&w[1]) == ssh && Path::new(&w[2]) == ssh));
    assert!(!words.windows(2).any(|w| w[0] == "--bind" && Path::new(&w[1]) == ssh), "never writable");
    for bad in [
        PathBuf::from("/etc"),
        PathBuf::from("relative"),
        home.join("../x"),
        home.clone(),
        home.join(".nothere"),
    ] {
        assert!(
            view_args(&home, &home, std::slice::from_ref(&bad)).is_err(),
            "{}",
            bad.display()
        );
    }
    let _ = std::fs::remove_dir_all(top);
}

/// With bwrap that can run here: the real view. A canary in a dot-place cannot be read and the
/// startup file cannot be changed; a visible file is read.
#[test]
fn the_real_view_hides_dot_places_and_leaves_the_startup_file_alone() {
    let Some(bwrap) = super::super::workshops::find_bwrap() else {
        return;
    };
    let (top, home) = home();
    plant(&home, ".ssh/canary-1265");
    plant(&home, ".newthing/canary-1265");
    plant(&home, "Documents/ok-1265");
    std::fs::write(home.join(".bashrc"), "original\n").unwrap();
    let cmd = "cat ok-1265 >&2; ! cat ../.ssh/canary-1265 2>/dev/null; ! cat ../.newthing/canary-1265 2>/dev/null; echo x >> ../.bashrc; echo done";
    let (program, args) = super::super::workshops::outside_command(
        &home,
        &home.join("Documents"),
        &[],
        "/bin/bash",
        &["-c".into(), cmd.into()],
    )
    .unwrap();
    assert!(program.ends_with(bwrap.rsplit('/').next().unwrap()));
    let Ok(out) = std::process::Command::new(program)
        .args(args)
        .current_dir(home.join("Documents"))
        .output()
    else {
        return;
    };
    if !out.status.success() && String::from_utf8_lossy(&out.stderr).contains("bwrap:") {
        return; // no user namespaces in this sandbox
    }
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("done"),
        "{out:?}"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("canary-1265"),
        "a visible file is read: {out:?}"
    );
    assert_eq!(
        std::fs::read_to_string(home.join(".bashrc")).unwrap(),
        "original\n",
        "the host's startup file is unchanged"
    );
    let _ = std::fs::remove_dir_all(top);
}

#[test]
fn a_failed_dot_path_gets_the_one_line_hint_and_nothing_else_does() {
    let miss = "cat: /home/ann/.ssh/id: No such file or directory\n";
    assert_eq!(hint_for(1, miss), Some(MISSING_HINT));
    assert_eq!(hint_for(0, miss), None);
    assert_eq!(
        hint_for(1, "cat: /home/ann/Documents/x: No such file or directory\n"),
        None
    );
    assert_eq!(hint_for(1, "boom\n"), None);
    assert!(!MISSING_HINT.contains('\n'));
}

/// With bwrap that can run here: without `reach` the place is absent; with it, readable and not writable;
/// a place that holds other protected places or is not there is refused.
#[test]
fn the_real_view_with_reach_reads_the_place_and_cannot_write_it() {
    let Some(_) = super::super::workshops::find_bwrap() else { return };
    let (top, home) = home();
    plant(&home, ".ssh/canary-1265");
    plant(&home, ".config/gh/canary-1265");
    std::fs::create_dir_all(home.join("Documents")).unwrap();
    let cwd = home.join("Documents");
    let cmd = "if cat ../.ssh/canary-1265 >/dev/null 2>&1; then echo READ; else echo MISSING; fi; if echo x >> ../.ssh/canary-1265 2>/dev/null; then echo WROTE; else echo READONLY; fi";
    let run = |reach: &[PathBuf]| -> Option<String> {
        let (program, args) = super::super::workshops::outside_command(&home, &cwd, reach, "/bin/bash", &["-c".into(), cmd.into()]).unwrap();
        let out = std::process::Command::new(program).args(args).current_dir(&cwd).output().ok()?;
        match String::from_utf8_lossy(&out.stderr).contains("bwrap:") {
            true => None, // no user namespaces in this sandbox
            false => Some(String::from_utf8_lossy(&out.stdout).into_owned()),
        }
    };
    let Some(without) = run(&[]) else { return };
    assert!(without.contains("MISSING"), "without reach the place is absent: {without}");
    let with = run(&[home.join(".ssh")]).expect("the same sandbox");
    assert!(with.contains("READ") && with.contains("READONLY"), "with reach: readable, not writable: {with}");
    assert_eq!(std::fs::read_to_string(home.join(".ssh/canary-1265")).unwrap(), "canary-1265", "the host file is unchanged");
    assert!(reach_list(&home, &["~/.config".to_string()]).is_err(), "a place above other protected places is refused");
    assert!(reach_list(&home, &["~/.nope".to_string()]).is_err());
    let _ = std::fs::remove_dir_all(top);
}

#[test]
fn the_hint_names_the_tool_that_asks() {
    assert!(MISSING_HINT.contains("agent_run_reach") && MISSING_HINT.contains("run_command_with_reach"), "{MISSING_HINT}");
    assert!(!MISSING_HINT.contains("name it in `reach`"));
}

/// A database's side files in the home are the database's: not in the view, and a file that is gone
/// when the view is made does not stop a command (the `-try` binds).
#[test]
fn a_protected_names_side_files_are_absent_and_a_vanished_file_stops_nothing() {
    let (top, home) = home();
    for name in ["memory.db", "memory.db-wal", "memory.db-shm", "memory.db-journal", "Memory.DB-SHM"] {
        plant(&home, name);
    }
    plant(&home, "memory.dbx");
    let words = view_args(&home, &home, &[]).unwrap();
    for gone in ["memory.db", "memory.db-wal", "memory.db-shm", "memory.db-journal", "Memory.DB-SHM"] {
        assert!(!in_view(&words, &home, &home.join(gone)), "{gone}");
    }
    assert!(in_view(&words, &home, &home.join("memory.dbx")), "a different name is an ordinary file");
    assert!(words.iter().any(|w| w == "--bind-try") && !words.iter().any(|w| w == "--bind"), "entries are bound with -try");
    let _ = std::fs::remove_dir_all(top);
}
