//! #1247 through `run_command`: a credential outside its folder is refused by name, a listing
//! never prints one, and the refusal never carries the file's contents. Fake files only.

use std::path::{Path, PathBuf};

use crate::safe_command::{check, expand, protected, run, words};
use crate::safe_command_listing::{stderr_without_protected, without_protected};

const FAKE: &str = "FAKE-SECRET-DO-NOT-PRINT";

struct Home(PathBuf);

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn home(tag: &str) -> Home {
    let dir = std::env::temp_dir().join(format!("yantrik-1247-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    Home(dir.canonicalize().unwrap())
}

fn put(home: &Home, rel: &str) -> String {
    let at = home.0.join(rel);
    std::fs::create_dir_all(at.parent().unwrap()).unwrap();
    std::fs::write(&at, FAKE).unwrap();
    at.to_string_lossy().into_owned()
}

fn verdict(line: &str, home: &Path) -> Result<(), String> {
    let h = home.to_string_lossy();
    check(&expand(&words(line)?, &h), &h)
}

#[test]
fn a_secret_copied_out_of_its_folder_is_refused_by_every_reader() {
    let h = home("read");
    for rel in [
        "backup/tokens.json", "x/.env", "Downloads/server.pem", "old/id_ed25519", "old/ID_RSA.bak",
        "Documents/client_secret_123.json", "hermes-copy/.hermes/config.yaml", ".mozilla/firefox/x/logins.json",
    ] {
        let at = put(&h, rel);
        for program in ["cat", "head", "tail", "grep x", "wc", "file", "sha256sum", "sort", "stat", "ls"] {
            let refused = verdict(&format!("{program} {at}"), &h.0).unwrap_err();
            assert!(refused.contains("a place the AI never reads"), "{program} {rel}: {refused}");
            assert!(!refused.contains(FAKE));
        }
    }
}

#[test]
fn a_link_to_a_secret_is_refused() {
    let h = home("link");
    let real = put(&h, "backup/tokens.json");
    let link = h.0.join("notes.txt");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    assert!(verdict(&format!("cat {}", link.display()), &h.0).is_err());
}

#[test]
fn decoys_and_public_keys_stay_readable() {
    let h = home("decoy");
    for rel in ["old/id_ed25519.pub", "Documents/tokens.md", "Documents/keynote.txt", "Documents/notes-about-tokens.md"] {
        let at = put(&h, rel);
        assert_eq!(verdict(&format!("cat {at}"), &h.0), Ok(()), "{rel}");
    }
}

/// With no shell, the ways a command line hides a file name are only text: nothing to resolve,
/// and any program outside the list is refused whole.
#[test]
fn the_ways_to_name_a_file_indirectly_are_refused() {
    let h = home("indirect");
    let at = put(&h, "backup/tokens.json");
    for line in [
        format!("cd {} && cat tokens.json", h.0.join("backup").display()),
        format!("cat $(echo {at})"),
        format!("xargs cat {at}"),
        format!("find {} -exec cat {{}} ;", h.0.display()),
        format!("find {} -path *tokens.json", h.0.display()),
        format!("find {} -ipath *tokens.json", h.0.display()),
        format!("grep -r x {}", h.0.display()),
        format!("cp {at} /tmp/x"),
        format!("tar cf /tmp/x.tar {at}"),
        format!("less {at}"),
        format!("cat {}/backup/*", h.0.display()),
    ] {
        assert!(verdict(&line, &h.0).is_err(), "{line}");
    }
}

/// `echo path | xargs cat` is one `echo` whose words are text: nothing is piped and nothing read.
#[test]
fn a_pipe_to_xargs_is_only_text() {
    let h = home("pipe");
    let at = put(&h, "backup/tokens.json");
    let argv = words(&format!("echo {at} | xargs cat")).unwrap();
    let (out, _) = run(&argv).unwrap();
    assert!(!out.contains(FAKE), "{out}");
}

#[test]
fn find_may_not_search_for_a_secret_by_name() {
    let h = home("findname");
    put(&h, "backup/tokens.json");
    let root = h.0.display();
    for name in ["tokens.json", "'*.pem'", "'id_*'", ".env", "'client_secret*'"] {
        for test in ["-name", "-iname"] {
            let refused = verdict(&format!("find {root} {test} {name}"), &h.0).unwrap_err();
            assert!(refused.contains("never reads"), "{test} {name}: {refused}");
        }
    }
    for fine in ["'*.csv'", "tokens.md", "keynote.txt"] {
        assert_eq!(verdict(&format!("find {root} -name {fine}"), &h.0), Ok(()), "{fine}");
    }
}

#[test]
fn find_does_not_print_a_secret_it_walked_past() {
    let h = home("findout");
    put(&h, "Documents/a.csv");
    put(&h, "Documents/tokens.json");
    put(&h, "Documents/server.pem");
    put(&h, "Documents/keynote.txt");
    let argv = words(&format!("find {} -type f", h.0.join("Documents").display())).unwrap();
    let (out, _) = run(&argv).unwrap();
    assert!(out.contains("tokens.json"), "the raw output holds it, so the filter has work to do");
    let shown = without_protected(&argv, &out);
    assert!(shown.contains("a.csv") && shown.contains("keynote.txt"), "{shown}");
    assert!(!shown.contains("tokens.json") && !shown.contains("server.pem") && !shown.contains(FAKE), "{shown}");
    assert!(shown.contains("2 entries are not shown"), "{shown}");
}

#[test]
fn ls_recursive_long_and_du_do_not_print_a_secret() {
    let h = home("ls");
    put(&h, "Documents/a.csv");
    put(&h, "Documents/sub/tokens.json");
    put(&h, "Documents/sub/b.txt");
    put(&h, "Documents/.env");
    let root = h.0.join("Documents");
    for line in [format!("ls -R {}", root.display()), format!("ls -laR {}", root.display()), format!("ls -la {}", root.display()), format!("du -a {}", root.display())] {
        let argv = words(&line).unwrap();
        let (out, _) = run(&argv).unwrap();
        let shown = without_protected(&argv, &out);
        assert!(!shown.contains("tokens.json") && !shown.contains(".env"), "{line}: {shown}");
        assert!(shown.contains("a.csv") || shown.contains("b.txt"), "{line}: {shown}");
        assert!(shown.contains("not shown"), "{line}: {shown}");
    }
}

#[test]
fn a_listing_of_a_link_to_a_secret_hides_it() {
    let h = home("lslink");
    let real = put(&h, "backup/tokens.json");
    std::os::unix::fs::symlink(&real, h.0.join("backup/notes.txt")).unwrap();
    let argv = words(&format!("ls -l {}", h.0.join("backup").display())).unwrap();
    let (out, _) = run(&argv).unwrap();
    let shown = without_protected(&argv, &out);
    assert!(!shown.contains("notes.txt") && !shown.contains("tokens.json"), "{shown}");
}

#[test]
fn other_programs_come_back_whole() {
    let argv = words("echo hi").unwrap();
    assert_eq!(without_protected(&argv, "hi\n"), "hi\n");
}

#[test]
fn browser_profiles_are_refused_by_the_one_shared_list() {
    for p in [".mozilla/x", ".config/chromium/x", ".config/BraveSoftware/x", ".var/app/com.google.Chrome/x", "snap/firefox/x", ".thunderbird/x"] {
        assert!(protected(&format!("/home/ann/{p}")), "{p}");
    }
}

/// Review H1: `-print0` joins names with NUL, which a filter that splits lines never sees.
#[test]
fn find_print0_is_refused_and_every_allowed_print_form_hides_a_secret() {
    let h = home("print0");
    put(&h, "backup/tokens.json");
    put(&h, "backup/id_rsa");
    put(&h, "backup/notes.md");
    let dir = h.0.join("backup");
    let refused = verdict(&format!("find {} -print0", dir.display()), &h.0).unwrap_err();
    assert!(refused.contains("-print0"), "{refused}");
    for line in [format!("find {}", dir.display()), format!("find {} -print", dir.display()), format!("find {} -type f -print", dir.display())] {
        assert_eq!(verdict(&line, &h.0), Ok(()), "{line}");
        let argv = words(&line).unwrap();
        let (out, _) = run(&argv).unwrap();
        assert!(out.contains("tokens.json"), "{line}: the raw output holds it");
        let shown = without_protected(&argv, &out);
        assert!(shown.contains("notes.md"), "{line}: {shown}");
        assert!(!shown.contains("tokens.json") && !shown.contains("id_rsa") && !shown.contains(FAKE), "{line}: {shown}");
    }
}

/// Review M2: a classified name is `tokens.json*` to a filter that matches whole names.
#[test]
fn ls_does_not_decorate_names_and_hides_an_executable_secret() {
    use std::os::unix::fs::PermissionsExt;
    let h = home("classify");
    for rel in ["backup/tokens.json", "backup/server.pem"] {
        let at = put(&h, rel);
        std::fs::set_permissions(&at, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    put(&h, "backup/notes.md");
    std::fs::create_dir_all(h.0.join("backup/credentials")).unwrap();
    let dir = h.0.join("backup");
    for flags in ["-F", "-lF", "--classify", "-aF1"] {
        let refused = verdict(&format!("ls {flags} {}", dir.display()), &h.0).unwrap_err();
        assert!(refused.contains("not one of the options"), "{flags}: {refused}");
    }
    for flags in ["-l", "-la", "-1", "-R"] {
        let argv = words(&format!("ls {flags} {}", dir.display())).unwrap();
        let (out, _) = run(&argv).unwrap();
        let shown = without_protected(&argv, &out);
        assert!(shown.contains("notes.md"), "{flags}: {shown}");
        assert!(!shown.contains("tokens.json") && !shown.contains("server.pem") && !shown.contains("credentials"), "{flags}: {shown}");
    }
}

/// Review M1: the other name of a hard-linked secret is no secret name, and no link to follow.
#[test]
fn a_hard_link_to_a_secret_is_refused_by_every_reader() {
    let h = home("hard");
    let real = put(&h, "backup/tokens.json");
    let second = h.0.join("Documents/data.bin");
    std::fs::create_dir_all(second.parent().unwrap()).unwrap();
    std::fs::hard_link(&real, &second).unwrap();
    for program in ["cat", "head", "tail", "grep -l FAKE", "grep FAKE", "sha256sum", "md5sum", "wc", "file", "sort", "diff"] {
        let refused = verdict(&format!("{program} {}", second.display()), &h.0).unwrap_err();
        assert!(refused.contains("hard link"), "{program}: {refused}");
        assert!(!refused.contains(FAKE));
    }
    // A link to the file reaches the same two names.
    let soft = h.0.join("Documents/via-link.txt");
    std::os::unix::fs::symlink(&second, &soft).unwrap();
    assert!(verdict(&format!("cat {}", soft.display()), &h.0).is_err());
    // A file with one name is read as before.
    let own = put(&h, "Documents/plain.txt");
    assert_eq!(verdict(&format!("cat {own}"), &h.0), Ok(()));
}

/// Review L1: an error that quotes a protected path says which one.
#[test]
fn stderr_does_not_name_a_protected_path() {
    let h = home("stderr");
    let missing = h.0.join("backup/tokens.json");
    let argv = words(&format!("ls {}", missing.display())).unwrap();
    let (_, err) = run(&argv).unwrap();
    assert!(err.contains("tokens.json"), "the raw error holds the name, so the filter has work to do: {err}");
    let shown = stderr_without_protected(&err);
    assert!(!shown.contains("tokens.json") && shown.contains("1 lines of error text are not shown"), "{shown}");
    for line in [
        "find: '/home/ann/backup/id_rsa': Permission denied",
        "find: \u{2018}/home/ann/old/credentials.json\u{2019}: Permission denied",
        "ls: cannot access `server.pem': No such file or directory",
        "find: '/home/ann/.ssh/x': Permission denied",
    ] {
        assert!(!stderr_without_protected(line).contains("/home/ann"), "{line}");
    }
    let fine = "find: '/home/ann/Documents/keynote.txt': Permission denied\n";
    assert_eq!(stderr_without_protected(fine), fine);
}
