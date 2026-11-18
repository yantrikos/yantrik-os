use super::*;
use std::os::unix::fs::{symlink, PermissionsExt};

/// A home with a workshop in it; `Videos` is the destination.
struct Fixture {
    _h: tempfile::TempDir,
    home: PathBuf,
    shop: PathBuf,
    dest: PathBuf,
}

fn fixture() -> Fixture {
    let h = tempfile::tempdir().unwrap();
    let home = h.path().canonicalize().unwrap();
    let shop = home.join("shop");
    let dest = home.join("Videos");
    std::fs::create_dir(&shop).unwrap();
    std::fs::create_dir(&dest).unwrap();
    Fixture { _h: h, home, shop, dest }
}

fn run(f: &Fixture, path: &str) -> Result<(PathBuf, Exported), String> {
    export(&f.shop, Path::new(path), &f.home, &f.dest)
}

fn fifo(at: &Path) {
    let c = CString::new(at.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

/// `git init --bare meta`, then `core.bare=false`, `core.worktree=..`, `core.fsmonitor`.
fn bare(at: &Path) {
    std::fs::create_dir_all(at.join("objects")).unwrap();
    std::fs::create_dir_all(at.join("refs/heads")).unwrap();
    std::fs::write(at.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    std::fs::write(at.join("config"), "[core]\n\tbare = false\n\tworktree = ..\n\tfsmonitor = touch /tmp/PWNED; false\n").unwrap();
}

#[test]
fn export_copies_regular_files_by_content_and_nothing_else() {
    let f = fixture();
    let secret = f.home.join("secret");
    std::fs::write(&secret, "host secret").unwrap();
    let src = f.shop.join("pkg");
    std::fs::create_dir_all(src.join("sub/deep")).unwrap();
    std::fs::write(src.join("a.txt"), "alpha").unwrap();
    std::fs::write(src.join("sub/deep/b.txt"), "beta").unwrap();
    std::fs::write(src.join("notes.csv"), "x,y\n").unwrap();
    std::fs::set_permissions(src.join("notes.csv"), std::fs::Permissions::from_mode(0o4755)).unwrap();
    symlink(&secret, src.join("leak")).unwrap();
    symlink(&f.home, src.join("dir-leak")).unwrap();
    fifo(&src.join("pipe"));
    // Not on the allow list: a script, a startup file, tool config (hidden, at depth too).
    std::fs::write(src.join("run.sh"), "#!/bin/sh\n").unwrap();
    std::fs::write(src.join(".npmrc"), "script-shell=/tmp/x").unwrap();
    std::fs::create_dir_all(src.join(".cargo")).unwrap();
    std::fs::write(src.join(".cargo/config.toml"), "[build]\nrustc-wrapper='x'").unwrap();
    // An extended attribute, where the filesystem takes one.
    let name = CString::new("user.mark").unwrap();
    let path = CString::new(src.join("a.txt").as_os_str().as_bytes()).unwrap();
    let tagged = unsafe { libc::setxattr(path.as_ptr(), name.as_ptr(), b"1".as_ptr().cast(), 1, 0) } == 0;
    let (at, done) = run(&f, "pkg").unwrap();
    assert_eq!(at, f.dest.join("pkg"));
    assert_eq!(done, Exported { files: 3, bytes: 5 + 4 + 4, dropped: 3 }, "run.sh, .npmrc and .cargo are left out");
    assert_eq!(std::fs::read_to_string(at.join("a.txt")).unwrap(), "alpha");
    assert_eq!(std::fs::read_to_string(at.join("sub/deep/b.txt")).unwrap(), "beta");
    for gone in ["leak", "dir-leak", "pipe", "run.sh", ".npmrc", ".cargo"] {
        assert!(at.join(gone).symlink_metadata().is_err(), "{gone} must not be copied");
    }
    // Plain bytes: no setuid or execute bit, no attribute, a regular file.
    let mode = std::fs::metadata(at.join("notes.csv")).unwrap().permissions().mode();
    assert_eq!(mode & 0o7000, 0, "{mode:o}");
    assert_eq!(mode & 0o111, 0, "{mode:o}");
    if tagged {
        let copy = CString::new(at.join("a.txt").as_os_str().as_bytes()).unwrap();
        assert!(unsafe { libc::getxattr(copy.as_ptr(), name.as_ptr(), std::ptr::null_mut(), 0) } < 0, "the attribute was copied");
    }
    // The landing folder is an ordinary 0755 one and nothing of the scratch name is left.
    assert_eq!(std::fs::metadata(&at).unwrap().permissions().mode() & 0o777, 0o755);
    assert_eq!(names(&f.dest), ["pkg"]);
    // It never replaces what is there.
    assert!(run(&f, "pkg").unwrap_err().contains("could not be made"));
    assert_eq!(std::fs::read_to_string(at.join("a.txt")).unwrap(), "alpha");
}

/// Finding 6: an allow list. Tool config is not carried at any depth, whatever it is called.
#[test]
fn only_a_kind_of_file_that_is_read_is_carried() {
    for yes in ["a.txt", "Notes.MD", "clip.mp4", "song.flac", "pic.JPG", "data.csv", "main.rs", "README", "LICENSE", "Cargo.lock"] {
        assert!(carried(yes.as_bytes()), "{yes}");
    }
    for no in [
        ".mise.toml", "mise.toml", ".tool-versions", ".envrc", ".direnv", ".idea", ".devcontainer", ".vscode", "x.desktop", "run.sh",
        "Makefile", "justfile", "a.service", "x.so", "", ".", "txt", ".txt", "a.TXT.exe", "page.html",
    ] {
        assert!(!carried(no.as_bytes()), "{no:?}");
    }
    let f = fixture();
    let pkg = f.shop.join("pkg");
    std::fs::create_dir_all(pkg.join("a/b/.devcontainer")).unwrap();
    std::fs::create_dir_all(pkg.join("a/.idea")).unwrap();
    std::fs::write(pkg.join("a/b/.devcontainer/devcontainer.json"), "{}").unwrap();
    std::fs::write(pkg.join("a/.idea/x.xml"), "<a/>").unwrap();
    std::fs::write(pkg.join("a/b/mise.toml"), "[tasks]").unwrap();
    std::fs::write(pkg.join("a/b/.mise.toml"), "[tasks]").unwrap();
    std::fs::write(pkg.join("a/b/.tool-versions"), "node 20").unwrap();
    std::fs::write(pkg.join("a/b/ok.md"), "fine").unwrap();
    let (at, done) = run(&f, "pkg").unwrap();
    assert_eq!((done.files, done.dropped), (1, 5));
    assert_eq!(names(&at.join("a/b")), ["ok.md"]);
    assert_eq!(names(&at.join("a")), ["b"]);
    // A single file is judged the same.
    std::fs::write(f.shop.join("go.sh"), "x").unwrap();
    assert!(run(&f, "go.sh").unwrap_err().contains("not a kind of file"));
}

#[test]
fn export_refuses_links_pipes_outside_paths_and_a_missing_folder() {
    let f = fixture();
    std::fs::write(f.home.join("outside.txt"), "x").unwrap();
    symlink(f.home.join("outside.txt"), f.shop.join("link.txt")).unwrap();
    symlink(&f.home, f.shop.join("dir-link")).unwrap();
    fifo(&f.shop.join("pipe"));
    for bad in ["link.txt", "dir-link", "dir-link/outside.txt", "pipe", "../outside.txt", "missing", "", "/etc/passwd"] {
        assert!(run(&f, bad).is_err(), "{bad:?}");
    }
    assert!(names(&f.dest).is_empty(), "nothing landed");
    std::fs::write(f.shop.join("ok.txt"), "ok").unwrap();
    assert!(export(&f.shop, Path::new("ok.txt"), &f.home, &f.dest.join("nowhere")).is_err());
    assert!(run(&f, "ok.txt").is_ok());
}

/// H2: a repository is what it looks like, not what it is called. Every one of these is
/// refused whole, and nothing is left behind.
#[test]
fn export_refuses_a_repository_by_its_shape_at_every_level_and_by_name_in_any_case() {
    let f = fixture();
    bare(&f.shop.join("proj/meta"));
    std::fs::write(f.shop.join("proj/readme"), "hi").unwrap();
    bare(&f.shop.join("whole"));
    bare(&f.shop.join("deep/er/still/inside"));
    std::fs::create_dir_all(f.shop.join("worktree/sub")).unwrap();
    std::fs::write(f.shop.join("worktree/sub/gitdir"), "/elsewhere/.git\n").unwrap();
    std::fs::create_dir_all(f.shop.join("linked")).unwrap();
    std::fs::write(f.shop.join("linked/commondir"), "../..\n").unwrap();
    for (i, n) in [".git", ".GIT", ".Hg", ".jj", ".svn", ".bzr"].iter().enumerate() {
        std::fs::create_dir_all(f.shop.join(format!("named{i}"))).unwrap();
        std::fs::write(f.shop.join(format!("named{i}/{n}")), "gitdir: /x\n").unwrap();
        std::fs::create_dir_all(f.shop.join(format!("dirnamed{i}/{n}"))).unwrap();
    }
    for bad in ["proj", "proj/meta", "whole", "deep", "worktree", "linked", "named0", "named1", "named2", "named3", "named4", "named5", "dirnamed0", "dirnamed2", "dirnamed3"] {
        let why = run(&f, bad).unwrap_err();
        assert!(why.contains("repository") || why.contains("version-control"), "{bad}: {why}");
    }
    for bad in [".git", ".HG", ".jj"] {
        assert!(run(&f, bad).is_err(), "{bad}");
    }
    assert!(names(&f.dest).is_empty(), "a refused export leaves nothing behind, not even a scratch name");
    // A folder that only has a file called HEAD is not a repository.
    std::fs::create_dir_all(f.shop.join("notes")).unwrap();
    std::fs::write(f.shop.join("notes/HEAD"), "head").unwrap();
    std::fs::write(f.shop.join("notes/a.txt"), "head").unwrap();
    assert!(run(&f, "notes").is_ok());
}

/// Finding 5: the shape is judged on the source before one byte lands, so there is no window in
/// which `~/Projects/<name>/meta` is a working repository. And a source that turns into one after
/// that is still caught on the copy, which is only a hidden scratch name until it is whole.
#[test]
fn a_repository_is_refused_before_any_byte_lands_and_never_exists_under_its_name() {
    let f = fixture();
    // A big file first, so the copy of a naive implementation would run a while.
    std::fs::create_dir_all(f.shop.join("thesis")).unwrap();
    std::fs::write(f.shop.join("thesis/objects.csv"), vec![b'x'; 4 << 20]).unwrap();
    bare(&f.shop.join("thesis/meta"));
    std::fs::create_dir_all(f.shop.join("thesis/meta/info")).unwrap();
    std::fs::write(f.shop.join("thesis/meta/info/attributes"), "* filter=x\n").unwrap();
    let saw = std::cell::Cell::new(false);
    let seen = |dest: &Path| {
        // Hit by the hook below if it ever ran: the source was refused before it.
        saw.set(true);
        let _ = dest;
    };
    let err = export_with(&f.shop, Path::new("thesis"), &f.home, &f.dest, &|| seen(&f.dest)).unwrap_err();
    assert!(err.contains("repository"), "{err}");
    assert!(!saw.get(), "refused at the preflight, before the copy began");
    assert!(names(&f.dest).is_empty());
    // Turned into a repository after the preflight: caught on the copy, and removed by descriptor.
    std::fs::remove_dir_all(f.shop.join("thesis/meta")).unwrap();
    let turn = || std::fs::create_dir_all(f.shop.join("thesis/late/.git")).unwrap();
    let err = export_with(&f.shop, Path::new("thesis"), &f.home, &f.dest, &turn).unwrap_err();
    assert!(err.contains("version-control"), "{err}");
    assert!(names(&f.dest).is_empty(), "the half copy is gone: {:?}", names(&f.dest));
    // Whole and clean, it appears under its name in one step: nothing else was ever there.
    std::fs::remove_dir_all(f.shop.join("thesis/late")).unwrap();
    let during = || assert!(!f.dest.join("thesis").exists(), "not under its name while it is built");
    let (at, _) = export_with(&f.shop, Path::new("thesis"), &f.home, &f.dest, &during).unwrap();
    assert_eq!(names(&f.dest), ["thesis"]);
    assert!(at.join("objects.csv").is_file());
}

/// H2, end to end where git is installed: what is not refused is not a repository to git.
#[test]
fn a_refused_export_leaves_nothing_for_git_to_run() {
    let git = |args: &[&str], dir: &Path| std::process::Command::new("git").args(args).current_dir(dir).output();
    let f = fixture();
    std::fs::create_dir_all(f.shop.join("pkg")).unwrap();
    let marker = f.home.join("PWNED");
    let init = git(&["init", "--bare", "-q", "meta"], &f.shop.join("pkg"));
    if init.map_or(true, |o| !o.status.success()) {
        return; // no git here
    }
    let cfg = format!("touch {}; false", marker.display());
    for kv in [["core.bare", "false"], ["core.worktree", ".."], ["core.fsmonitor", cfg.as_str()]] {
        git(&["config", kv[0], kv[1]], &f.shop.join("pkg/meta")).unwrap();
    }
    assert!(run(&f, "pkg").is_err());
    let _ = git(&["status"], &f.dest);
    assert!(!marker.exists(), "git ran the marker");
}

/// M3: the name that lands is judged like the folder it lands in, and a link in the place of
/// the folder is not followed.
#[test]
fn export_refuses_a_startup_name_and_a_destination_that_is_a_link() {
    let f = fixture();
    for n in [".bash_profile", ".zshenv", ".gitconfig", ".pam_environment", ".npmrc"] {
        std::fs::write(f.shop.join(n), "x").unwrap();
        assert!(run(&f, n).unwrap_err().contains("starts with a dot"), "{n}");
        assert!(f.dest.join(n).symlink_metadata().is_err(), "{n} landed");
    }
    std::fs::write(f.shop.join("x.txt"), "x").unwrap();
    let autostart = f.home.join("autostart");
    std::fs::create_dir(&autostart).unwrap();
    let videos_link = f.home.join("Videos-link");
    symlink(&autostart, &videos_link).unwrap();
    let why = export(&f.shop, Path::new("x.txt"), &f.home, &videos_link).unwrap_err();
    assert!(why.contains("not a folder that is there"), "{why}");
    assert!(names(&autostart).is_empty(), "nothing landed through the link");
    // A name that is a link made after the card is not written through either.
    symlink(f.home.join("elsewhere"), f.dest.join("x.txt")).unwrap();
    assert!(run(&f, "x.txt").is_err());
    assert!(!f.home.join("elsewhere").exists());
}

/// Finding 2, delete: the destination is swapped for a link to another folder while the copy
/// runs, and the copy then fails. The failed copy is removed from the folder it was made in, by
/// descriptor, and the other folder's `thesis/keep` is not touched.
#[test]
fn a_destination_swapped_for_a_link_mid_copy_deletes_and_plants_nothing_elsewhere() {
    let f = fixture();
    let docs = f.home.join("Documents");
    std::fs::create_dir_all(docs.join("thesis")).unwrap();
    std::fs::write(docs.join("thesis/keep"), "mine").unwrap();
    std::fs::create_dir_all(f.shop.join("thesis")).unwrap();
    std::fs::write(f.shop.join("thesis/a.txt"), "a").unwrap();
    // Between the card and the copy: the job moves ~/Videos away and puts a link to Documents in its place,
    // and the source turns into a repository so that the copy fails.
    let swap = || {
        std::fs::rename(&f.dest, f.home.join("V")).unwrap();
        symlink(&docs, &f.dest).unwrap();
        std::fs::create_dir_all(f.shop.join("thesis/late/.git")).unwrap();
    };
    let err = export_with(&f.shop, Path::new("thesis"), &f.home, &f.dest, &swap).unwrap_err();
    assert!(err.contains("version-control"), "{err}");
    assert_eq!(std::fs::read_to_string(docs.join("thesis/keep")).unwrap(), "mine", "keep survives");
    assert_eq!(names(&docs.join("thesis")), ["keep"]);
    assert!(names(&f.home.join("V")).is_empty(), "the half copy was removed where it was made: {:?}", names(&f.home.join("V")));
}

/// Finding 2, plant: `to=~/Projects` is swapped for a link to a startup folder after the card. The
/// folder `autostart` lands nowhere in it: the copy is refused when the destination has moved.
#[test]
fn a_destination_swapped_for_a_startup_folder_plants_nothing_there() {
    let f = fixture();
    let projects = f.home.join("Projects");
    let config = f.home.join(".config");
    std::fs::create_dir_all(&projects).unwrap();
    std::fs::create_dir_all(&config).unwrap();
    std::fs::create_dir_all(f.shop.join("autostart")).unwrap();
    std::fs::write(f.shop.join("autostart/x.desktop"), "[Desktop Entry]\nExec=sh").unwrap();
    std::fs::write(f.shop.join("autostart/notes.txt"), "n").unwrap();
    let swap = || {
        std::fs::rename(&projects, f.home.join("P")).unwrap();
        symlink(&config, &projects).unwrap();
    };
    let result = export_with(&f.shop, Path::new("autostart"), &f.home, &projects, &swap);
    assert!(result.is_err(), "the destination moved while the copy ran: {result:?}");
    assert!(names(&config).is_empty(), "nothing in ~/.config: {:?}", names(&config));
    assert!(names(&f.home.join("P")).is_empty(), "and the moved folder holds no leftover: {:?}", names(&f.home.join("P")));
    // A destination moved INTO a startup place (not through a link) is caught at the commit too.
    let b = fixture();
    let autostart = b.home.join(".config/autostart");
    std::fs::create_dir_all(&autostart).unwrap();
    std::fs::create_dir(b.home.join("Stage")).unwrap();
    std::fs::write(b.shop.join("a.txt"), "x").unwrap();
    let into = || {
        std::fs::remove_dir(&autostart).unwrap();
        std::fs::rename(b.home.join("Stage"), &autostart).unwrap();
    };
    assert!(export_with(&b.shop, Path::new("a.txt"), &b.home, &b.home.join("Stage"), &into).is_err());
    assert!(names(&autostart).is_empty(), "{:?}", names(&autostart));
}
