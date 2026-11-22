//! Round 6 of #1265: what the security reads of `7c79cd73` found, one test per item.

use super::tests::{home, in_view, plant};
use super::*;
use std::os::unix::fs::symlink;

fn tmp_dir(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let d = PathBuf::from("/tmp").join(format!("yantrik-r6-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn has(words: &[String], flag: &str, to: &Path) -> bool {
    words.windows(3).any(|w| w[0] == flag && Path::new(&w[2]) == to)
}

/// 1. A working directory is a place the view shows, or inside the job's own `/tmp`.
#[test]
fn a_working_directory_never_binds_the_hosts_tmp() {
    let (top, home) = home();
    let words = view_args(&home, Path::new("/tmp"), &[]).unwrap().0;
    assert!(words.windows(2).any(|w| w[0] == "--tmpfs" && w[1] == "/tmp"), "the job's own /tmp");
    assert!(!words.iter().any(|w| w == "--bind" || w == "--ro-bind"), "nothing of the host's /tmp is bound: {words:?}");
    let below = tmp_dir("below");
    let words = view_args(&home, &below, &[]).unwrap().0;
    assert!(has(&words, "--dir", &below) || words.windows(2).any(|w| w[0] == "--dir" && Path::new(&w[1]) == below), "made in the job's /tmp");
    assert!(!words.iter().any(|w| w == "--bind"), "{words:?}");
    let _ = std::fs::remove_dir_all(&below);
    let _ = std::fs::remove_dir_all(top);
}

#[test]
fn a_tmp_path_that_leads_elsewhere_is_refused_whatever_its_text_says() {
    let (top, home) = home();
    let shop = home.join(".test-workshops/1000");
    // The person's runtime folder, or any folder that is not under /tmp.
    let run_user = [format!("/run/user/{}", unsafe { libc::geteuid() }), "/run".into(), "/usr".into()].into_iter().map(PathBuf::from).find(|p| p.is_dir()).unwrap();
    plant(&home, "Pictures/Screenshots/shot.png");
    std::fs::create_dir_all(&shop).unwrap();
    let out = tmp_dir("out");
    let mut refused = Vec::new();
    for (name, target) in [("shop", shop.clone()), ("run", run_user.clone()), ("shots", home.join("Pictures/Screenshots"))] {
        let link = out.join(name);
        symlink(&target, &link).unwrap();
        refused.push(link.clone());
        refused.push(link.join("sub-that-is-not-there"));
    }
    refused.push(out.join("../../etc"));
    refused.push(PathBuf::from("/tmp/x/../../run"));
    let dangling = out.join("dangling");
    symlink(home.join("nowhere"), &dangling).unwrap();
    refused.push(dangling);
    for cwd in &refused {
        let why = view_args(&home, cwd, &[]).unwrap_err();
        assert!(why.contains("Nothing was run"), "{}: {why}", cwd.display());
    }
    // A real directory in /tmp, and a link to another place in /tmp, are the job's own.
    let fine = out.join("fine");
    std::fs::create_dir_all(&fine).unwrap();
    assert!(view_args(&home, &fine, &[]).is_ok());
    let _ = std::fs::remove_dir_all(&out);
    let _ = std::fs::remove_dir_all(top);
}

fn git_fixture(home: &Path) {
    plant(home, ".config/git/credentials");
    plant(home, ".config/git/ignore");
    plant(home, ".config/git/attributes");
    std::fs::write(
        home.join(".config/git/config"),
        "[user]\n\tname = Ann Lee\n\temail = ann@example.org\n[credential]\n\thelper = store\n",
    )
    .unwrap();
    std::fs::write(
        home.join(".gitconfig"),
        "[user]\n\tsigningkey = ABCDEF\n[init]\n\tdefaultBranch = main\n[core]\n\tautocrlf = input\n\teditor = vim\n\tsshCommand = ssh -i /secret\n\
         [pull]\n\trebase = true\n[merge]\n\tconflictstyle = zdiff3\n[color]\n\tui = auto\n[diff \"pdf\"]\n\ttextconv = pdftotext\n\tcolorMoved = zebra\n\
         [alias]\n\tco = checkout\n\tdanger = !sh -c 'cat ~/.ssh/id'\n\tquoted = \"!sh\"\n[url \"https://tok123@github.com/\"]\n\tinsteadOf = https://github.com/\n\
         [http]\n\textraheader = AUTHORIZATION: bearer tok456\n[credential]\n\thelper = store\n[include]\n\tpath = /root/.secret\n[includeIf \"gitdir:~/\"]\n\tpath = x\n",
    )
    .unwrap();
}

/// 2. The view never carries git's files; it carries a synthesized config of the keys that may be shown.
#[test]
fn git_credentials_and_url_rewrites_are_not_in_the_view_and_the_allowed_keys_are() {
    let (top, home) = home();
    git_fixture(&home);
    let words = view_args(&home, &home, &[]).unwrap().0;
    let all = words.join("\n");
    for never in ["credential", "tok123", "tok456", "extraheader", "insteadof", "sshcommand", "/secret", "include", "signingkey", "danger", "pdftotext", "\"!sh\"", "alias", "checkout"] {
        assert!(!all.to_lowercase().contains(never), "{never} is in the view: {words:?}");
    }
    for whole in [".gitconfig", ".config/git", ".config/git/credentials", ".config/git/config"] {
        assert!(!words.iter().any(|w| Path::new(w) == home.join(whole)), "{whole} is bound");
    }
    assert!(!in_view(&words, &home, &home.join(".config/git/credentials")));
    for kept in [".config/git/ignore", ".config/git/attributes"] {
        assert!(has(&words, "--ro-bind-try", &home.join(kept)), "{kept}");
    }
    let pairs: Vec<(String, String)> = words
        .windows(3)
        .filter(|w| w[0] == "--setenv" && w[1].starts_with("GIT_CONFIG_"))
        .map(|w| (w[1].clone(), w[2].clone()))
        .collect();
    let value = |name: &str| pairs.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone());
    let keys: Vec<String> = (0..).map_while(|i| value(&format!("GIT_CONFIG_KEY_{i}"))).collect();
    let vals: Vec<String> = (0..keys.len()).filter_map(|i| value(&format!("GIT_CONFIG_VALUE_{i}"))).collect();
    assert_eq!(value("GIT_CONFIG_COUNT").as_deref(), Some(keys.len().to_string().as_str()));
    let got: Vec<(&str, &str)> = keys.iter().map(String::as_str).zip(vals.iter().map(String::as_str)).collect();
    for want in [
        ("user.name", "Ann Lee"),
        ("user.email", "ann@example.org"),
        ("init.defaultbranch", "main"),
        ("core.autocrlf", "input"),
        ("core.editor", "vim"),
        ("pull.rebase", "true"),
        ("merge.conflictstyle", "zdiff3"),
        ("color.ui", "auto"),
        ("diff.pdf.colormoved", "zebra"),
    ] {
        assert!(got.contains(&want), "{want:?} missing from {got:?}");
    }
    assert_eq!(got.len(), 9, "nothing else arrives: {got:?}");
    let _ = std::fs::remove_dir_all(top);
}

#[test]
fn a_secret_named_file_inside_a_toolchain_entry_is_masked_and_a_linked_entry_is_skipped() {
    let (top, home) = home();
    plant(&home, ".local/bin/build-tool");
    plant(&home, ".local/bin/id_rsa");
    plant(&home, ".local/bin/deep/tokens.json");
    plant(&home, ".local/bin/deep/ok.sh");
    plant(&home, ".rustup/settings.toml");
    plant(&home, "elsewhere/x");
    std::fs::create_dir_all(home.join(".cargo")).unwrap();
    symlink(home.join("elsewhere"), home.join(".cargo/bin")).unwrap();
    let words = view_args(&home, &home, &[]).unwrap().0;
    assert!(has(&words, "--ro-bind-try", &home.join(".local/bin")));
    assert!(in_view(&words, &home, &home.join(".local/bin/build-tool")));
    assert!(in_view(&words, &home, &home.join(".local/bin/deep/ok.sh")));
    assert!(!in_view(&words, &home, &home.join(".local/bin/id_rsa")), "a secret's name is masked");
    assert!(!in_view(&words, &home, &home.join(".local/bin/deep/tokens.json")));
    assert!(in_view(&words, &home, &home.join(".rustup/settings.toml")));
    assert!(!words.iter().any(|w| Path::new(w) == home.join(".cargo/bin")), "a linked entry is skipped: {words:?}");
    let _ = std::fs::remove_dir_all(top);
}

/// 3. The card and the bind use one function; a link is refused with the place it leads to, and a
/// place swapped for a link after the check is refused at bind time.
#[test]
fn a_reach_place_that_is_a_link_is_refused_and_so_is_one_swapped_after_the_check() {
    let (top, home) = home();
    plant(&home, ".ssh/id");
    plant(&home, "Documents/a");
    symlink(home.join(".ssh"), home.join("Documents/looks-harmless")).unwrap();
    let why = reach_place(&home, &home.join("Documents/looks-harmless")).unwrap_err();
    assert!(why.contains("link") && why.contains(&home.join(".ssh").display().to_string()), "{why}");
    let checked = reach_list(&home, &["~/.ssh".to_string()]).unwrap();
    assert_eq!(checked, [home.join(".ssh")]);
    assert!(view_args(&home, &home, &checked).is_ok());
    // Swapped for a link between the card and the run.
    std::fs::create_dir_all(home.join("Other")).unwrap();
    std::fs::rename(home.join(".ssh"), home.join("moved")).unwrap();
    symlink(home.join("moved"), home.join(".ssh")).unwrap();
    assert!(view_args(&home, &home, &checked).is_err(), "bound from the place it still is, or not at all");
    let _ = std::fs::remove_dir_all(top);
}

#[test]
fn a_visible_top_level_entry_that_is_a_link_is_skipped_not_followed() {
    let (top, home) = home();
    plant(&home, "Real/a");
    plant(&home, "target/a");
    symlink(home.join("target"), home.join("Shortcut")).unwrap();
    let words = view_args(&home, &home, &[]).unwrap().0;
    assert!(has(&words, "--bind-try", &home.join("Real")) && has(&words, "--bind-try", &home.join("target")));
    assert!(!words.iter().any(|w| Path::new(w) == home.join("Shortcut")));
    let _ = std::fs::remove_dir_all(top);
}

/// 4 (changed in round 7). A `reach` of a place inside a visible folder is refused: it is already
/// visible, or protected and never shown. `Pictures` is made child by child, with nothing masked.
#[test]
fn a_reach_inside_a_visible_folder_is_refused_and_a_protected_place_in_one_is_not_shown() {
    let (top, home) = home();
    plant(&home, ".ssh/id");
    plant(&home, "Pictures/Screenshots/shot.png");
    plant(&home, "Pictures/Holiday/a.png");
    let words = view_args(&home, &home, &[home.join(".ssh")]).unwrap().0;
    assert!(!in_view(&words, &home, &home.join("Pictures/Screenshots/shot.png")));
    assert!(in_view(&words, &home, &home.join("Pictures/Holiday/a.png")));
    for (at, said) in [
        ("Pictures/Holiday", "already visible"),
        ("Pictures", "protected place inside a visible folder"),
        ("Pictures/Screenshots", "protected place inside a visible folder"),
        ("Pictures/Screenshots/shot.png", "protected place inside a visible folder"),
    ] {
        let why = reach_place(&home, &home.join(at)).unwrap_err();
        assert!(why.contains(said), "{at}: {why}");
    }
    let _ = std::fs::remove_dir_all(top);
}

/// 5 (changed in round 7). A protected folder that is made after the view started is hidden because
/// its parent is a tmpfs of the view's own: nothing is mounted over it, and nothing is made in the
/// person's folder.
#[test]
fn a_protected_folder_that_does_not_exist_yet_stays_hidden_and_nothing_is_made_for_it() {
    let (top, home) = home();
    std::fs::create_dir_all(home.join("Pictures")).unwrap();
    std::fs::create_dir_all(home.join("Videos")).unwrap();
    let words = view_args(&home, &home, &[]).unwrap().0;
    for made_later in ["Pictures/Screenshots", "Videos/Recordings"] {
        assert!(!words.windows(2).any(|w| w[0] == "--tmpfs" && Path::new(&w[1]) == home.join(made_later)), "no mount point: {words:?}");
        plant(&home, &format!("{made_later}/later.png"));
        assert!(!in_view(&words, &home, &home.join(made_later).join("later.png")), "{made_later} shows through");
    }
    // A folder that is not there at all is not in the view, so nothing is made for it.
    let (top2, bare) = super::tests::home();
    let words = view_args(&bare, &bare, &[]).unwrap().0;
    assert!(!words.iter().any(|w| w.contains("Pictures") || w.contains("Videos")), "{words:?}");
    let _ = std::fs::remove_dir_all(top);
    let _ = std::fs::remove_dir_all(top2);
}

/// 6. The honest sentence.
#[test]
fn the_tool_text_says_a_command_can_change_files_that_run_later() {
    for word in ["change files in the visible folders", "`.git/hooks`", "`.envrc`", "editor tasks", "run later as you", "sees your visible folders", "app data are not there"] {
        assert!(VIEW_TEXT.contains(word), "{word}");
    }
    assert_eq!(super::super::workshops::STARTUP_REACH, VIEW_TEXT);
}

/// 7. `~/snap` is hidden as a whole.
#[test]
fn snap_is_hidden_as_a_whole() {
    let (top, home) = home();
    plant(&home, "snap/thunderbird/common/mail.db");
    plant(&home, "snap/chromium/x");
    plant(&home, "Documents/a");
    let words = view_args(&home, &home, &[]).unwrap().0;
    assert!(!in_view(&words, &home, &home.join("snap/thunderbird/common/mail.db")));
    assert!(!in_view(&words, &home, &home.join("snap/chromium/x")));
    assert!(!words.iter().any(|w| w.contains("/snap")), "nothing is made for it: {words:?}");
    assert!(in_view(&words, &home, &home.join("Documents/a")));
    let _ = std::fs::remove_dir_all(top);
}

/// 10. The home is the account's, not `$HOME`.
#[test]
fn the_home_comes_from_the_account_and_a_different_home_variable_is_refused() {
    let account = account_home().expect("this test account has a home");
    assert!(account.is_absolute() && account.parent().is_some());
    let check = super::super::view_places::check_home;
    assert_eq!(check(PathBuf::from("/home/ann"), None).unwrap(), Path::new("/home/ann"));
    assert_eq!(check(PathBuf::from("/home/ann"), Some("/home/ann".as_ref())).unwrap(), Path::new("/home/ann"));
    for said in ["/home/bob", "", "/", "relative"] {
        assert!(check(PathBuf::from("/home/ann"), Some(said.as_ref())).is_err(), "{said:?}");
    }
}
