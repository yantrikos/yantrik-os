//! The automatic dependency install of `code_execute` and `script_run` (#1124, round 9, finding 2).
//!
//! The package names come from the Mind's own code (`import evil`, `require("x")`), and an
//! installer runs the package's own code: a `setup.py`, a `postinstall`. So the install:
//!
//! - starts through [`crate::spawn::mind_command`], in the outside view, where the workshops root
//!   does not exist (a package that does run code finds nothing to read);
//! - takes only names that match `^[A-Za-z0-9][A-Za-z0-9._-]*$`: no path (`pkgdir/`, `../x`), no
//!   `file:`/`git+`/URL spec, no `name@spec`, nothing that can be an option. Any other name is
//!   skipped, and the answer names it;
//! - runs no hook of the package: pip gets `--only-binary=:all:` (a wheel only, so no `setup.py`)
//!   and npm gets `--ignore-scripts` (no lifecycle script);
//! - **stages and promotes** (#1265, round 8): the installer runs with its working directory and
//!   its target in the job's own `/tmp`, so the dependencies folder is neither visible nor
//!   writable to it, and prints the result as a tar stream. The shell, outside the view, reads the
//!   stream with a strict reader (`coder_promote`: plain files and folders only, no link, a size
//!   limit, refuse on anything else) and writes it into the real folder as plain files;
//! - keeps the result in `~/.config/yantrik/deps` (round 10), outside the scripts folder: each install is
//!   staged in a `0700` folder of the shell's (`deps-staging`) that no view and no sandbox shows, written
//!   with descriptor-relative calls that refuse links, and swapped in whole. A script sees the result
//!   only as a **read-only** bind over `<scripts>/.deps` and `<scripts>/node_modules`, so it can import
//!   an installed package and never write one. npm runs with `--no-bin-links`, so a package with a
//!   `bin` is not refused for its links;
//! - puts a Python wheel in a private target (`pip3 install --target`), and the script sees it only through `PYTHONPATH`. Python runs a `.pth` file only from a site
//!   directory, and a `PYTHONPATH` entry is not one, so a `.pth` a wheel ships never runs, in the
//!   script or in any host Python (a `--user` install would put it where every later one reads it);
//! - has no ruby path: `gem install` runs an extension's `extconf.rb`, so a gem is never
//!   installed automatically.

use std::path::Path;
use std::process::Command;

/// The scripts folder of the account whose home is `home`: `~/.config/yantrik/scripts`. A dot-folder
/// of the home, so no view shows it.
pub fn scripts_dir_in(home: &Path) -> std::path::PathBuf {
    home.join(".config/yantrik/scripts")
}

/// The scripts folder of this process's account (`spawn::account_home`), or the reason there is none.
pub fn scripts_dir() -> Result<std::path::PathBuf, String> {
    crate::spawn::account_home().map(|h| scripts_dir_in(&h))
}

/// The dependencies folders of the account whose home is `home`: `~/.config/yantrik/deps`, a
/// dot-folder of the home that the shell owns. It is not in the scripts folder: a script sees it only
/// as a read-only bind, so it can import an installed package and never write one.
pub fn deps_root_in(home: &Path) -> std::path::PathBuf {
    home.join(".config/yantrik/deps")
}

/// Where each install is staged: a `0700` folder of the shell's, in no view and in no sandbox.
pub(crate) fn staging_in(home: &Path) -> std::path::PathBuf {
    home.join(".config/yantrik/deps-staging")
}

/// Where a script finds the Python packages (a read-only bind of `deps/python`), and where `PYTHONPATH` points.
pub(crate) fn deps_dir(scripts: &Path) -> std::path::PathBuf {
    scripts.join(".deps")
}

/// Where a language's result is promoted to.
pub(crate) fn promoted_to(home: &Path, language: &str) -> std::path::PathBuf {
    match language {
        "python" => deps_root_in(home).join("python"),
        _ => deps_root_in(home).join("node_modules"),
    }
}

/// Refuse an install whose `target` is not under the account's `home`: only there is the folder a
/// dot-folder of the home that no command's view shows or can write.
pub fn check_target(home: &Path, target: &Path) -> Result<(), String> {
    match target.strip_prefix(home) {
        Ok(rest) if rest.components().next().is_some_and(|c| matches!(c, std::path::Component::Normal(_))) => Ok(()),
        _ => Err(format!(
            "the dependencies folder {} is not under this account's home {}, where no command's view shows it; nothing was installed",
            target.display(),
            home.display()
        )),
    }
}

/// Where the install runs and puts its result, in the job's own `/tmp` (never the host's).
const STAGE: &str = "/tmp/yantrik-stage";

/// The folder of `STAGE` that holds the result, and the folder of the scripts' folder it is
/// promoted to.
fn staged_name(language: &str) -> &'static str {
    if language == "python" { "deps" } else { "node_modules" }
}

/// What the install runs: `$1` is the folder of the result, the rest is the installer and its words
/// (positional, never part of this text). The installer's own output goes to stderr so the standard
/// output is only the archive.
const STAGE_SCRIPT: &str = "set -e; out=\"$1\"; shift; mkdir -p /tmp/yantrik-stage \"$out\"; cd /tmp/yantrik-stage; \"$@\" >&2; \
tar --format=ustar --numeric-owner --owner=0 --group=0 -cf - -C \"$out\" .";

/// Whether `name` is a plain package name, as `^[A-Za-z0-9][A-Za-z0-9._-]*$`.
pub fn is_plain_package_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// The names that are plain, and the others, in order.
pub fn vet(deps: &[String]) -> (Vec<String>, Vec<String>) {
    deps.iter().cloned().partition(|d| is_plain_package_name(d))
}

/// The installer and its fixed arguments for `language`, or `None` where nothing is installed
/// automatically (ruby, and every language with no installer here).
fn installer(language: &str) -> Option<(&'static str, &'static [&'static str])> {
    match language {
        "python" => Some(("pip3", &["install", "-q", "--break-system-packages", "--only-binary=:all:"])),
        "node" | "javascript" | "typescript" => Some(("npm", &["install", "--no-save", "--silent", "--ignore-scripts", "--no-bin-links"])),
        _ => None,
    }
}

/// The install of `deps` for `language`, with the command that runs it in the outside view.
/// `Ok(None)` when there is nothing to install; `Err` is the view's refusal. `Some` is an install
/// that is ready to run: the installer's name, the command, the names left out. (The tuple is
/// spelled out here and not behind a `type` alias: the reach scan refuses an alias of a `Command`.)
pub fn plan(home: &Path, language: &str, deps: &[String]) -> Result<Option<(&'static str, Command, Vec<String>)>, String> {
    let Some((program, fixed)) = installer(language) else { return Ok(None) };
    let (plain, skipped) = vet(deps);
    if plain.is_empty() {
        return Ok(None);
    }
    let out = format!("{STAGE}/{}", staged_name(language));
    let mut argv: Vec<String> = ["-c", STAGE_SCRIPT, "sh", out.as_str(), program].map(String::from).to_vec();
    argv.extend(fixed.iter().map(|a| a.to_string()));
    if language == "python" {
        argv.extend(["--target".to_string(), out.clone()]);
    }
    argv.extend(plain);
    // The working directory is the job's own `/tmp`: the scripts folder is a protected place and is
    // not in the view.
    Ok(Some((program, stage_view(home, &argv)?, skipped)))
}

/// The proxy and certificate variables an installer needs to reach a registry (and nothing else of
/// the shell's own): passed to the installer only, through `extra`.
const NETWORK_VARS: &[&str] =
    &["HTTP_PROXY", "HTTPS_PROXY", "NO_PROXY", "http_proxy", "https_proxy", "no_proxy", "SSL_CERT_FILE", "SSL_CERT_DIR"];

fn network_env(source: impl IntoIterator<Item = (String, String)>) -> Vec<(String, String)> {
    source.into_iter().filter(|(k, _)| NETWORK_VARS.contains(&k.as_str())).collect()
}

/// The install's own view: `sh argv` in the outside view, in the job's own `/tmp`, with the proxy and
/// certificate variables of this process.
fn stage_view(home: &Path, argv: &[String]) -> Result<Command, String> {
    let extra = network_env(std::env::vars());
    let mut command = crate::spawn::mind_command(home, Path::new("/tmp"), "/bin/sh", argv, &extra)?;
    command.current_dir("/tmp").stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    Ok(command)
}

/// Make the folder an install starts in (and its `.deps` target's parent) when it is missing.
fn ensure_install_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)
}

/// What to tell about names that were not installed (empty when there are none).
fn skipped_note(skipped: &[String]) -> String {
    if skipped.is_empty() {
        return String::new();
    }
    let shown: Vec<String> = skipped.iter().take(10).map(|s| format!("`{}`", s.chars().take(60).collect::<String>())).collect();
    format!(
        "Not installed (not a plain package name): {}. Nothing but a plain name is installed automatically.\n",
        shown.join(", ")
    )
}

/// Install `deps` before a script runs. `Ok` carries the note to put before the output (names that
/// were skipped, or empty); `Err` is the message to answer with, and the script is not run.
pub fn install_deps(language: &str, deps: &[String]) -> Result<String, String> {
    if deps.is_empty() {
        return Ok(String::new());
    }
    let home = crate::spawn::account_home().map_err(|why| format!("Dependency install refused: {why}"))?;
    let scripts = scripts_dir_in(&home);
    let target = promoted_to(&home, language);
    check_target(&home, &target).map_err(|why| format!("Dependency install refused: {why}"))?;
    let planned = plan(&home, language, deps).map_err(|why| format!("Dependency install refused: {why}"))?;
    let Some((program, command, skipped)) = planned else {
        // Nothing is installed: ruby, or no name was plain. A name left out is still named.
        return Ok(if installer(language).is_some() { skipped_note(&vet(deps).1) } else { String::new() });
    };
    // pip and npm reach their public registries, outside the sandbox: that is a fetch from the open
    // web, and it waits for a turn the person started or for research to be on. Asked only when an
    // install would really run, so a refusal still leaves nothing written.
    crate::coder_sandbox::allow_install()?;
    // The result is promoted into the scripts folder, and on a machine that never ran a script it is
    // not there (#1175).
    ensure_install_dir(&deps_root_in(&home)).map_err(|e| format!("Could not make the dependencies folder: {e}"))?;
    ensure_install_dir(&scripts).map_err(|e| format!("Could not make the scripts folder {}: {e}", scripts.display()))?;
    tracing::info!(lang = language, deps = ?deps, "Auto-installing dependencies");
    let (status, archive, err) = run_staged(command, INSTALL_DEADLINE).map_err(|e| format!("Could not run {program}: {e}"))?;
    if !status {
        return Err(format!("Dependency install failed ({program}): {}", err.chars().take(500).collect::<String>()));
    }
    crate::coder_promote::promote(&archive, &target, &staging_in(&home)).map_err(|why| format!("Dependency install failed ({program}): {why}"))?;
    Ok(skipped_note(&skipped))
}

#[cfg(test)]
pub(crate) const STAGE_SCRIPT_FOR_TESTS: &str = STAGE_SCRIPT;

#[cfg(test)]
pub(crate) fn stage_view_for_tests(home: &Path, argv: &[String]) -> Result<Command, String> {
    stage_view(home, argv)
}

#[cfg(test)]
pub(crate) fn run_staged_for_tests(command: Command, deadline: std::time::Duration) -> std::io::Result<(bool, Vec<u8>, String)> {
    run_staged(command, deadline)
}

#[cfg(test)]
pub(crate) fn network_env_for_tests(source: Vec<(String, String)>) -> Vec<(String, String)> {
    network_env(source)
}

/// The longest an install may run.
const INSTALL_DEADLINE: std::time::Duration = std::time::Duration::from_secs(600);

/// The first bytes of the installer's standard error that are kept; the rest is read and thrown away.
const KEEP_ERR: u64 = 64 << 10;

/// Run the staged install: whether it succeeded, its standard output (the archive, cut at the
/// limit, which then refuses) and what it said on standard error (the first 64 KiB; the rest is
/// read and discarded, so a chatty installer is not killed by a broken pipe). The installer runs in
/// its own process group, and the whole group is killed when `deadline` passes.
fn run_staged(mut command: Command, deadline: std::time::Duration) -> std::io::Result<(bool, Vec<u8>, String)> {
    use std::io::Read;
    use std::os::unix::process::CommandExt;
    command.process_group(0);
    let mut child = command.spawn()?;
    let pgid = child.id() as i32;
    let kill_group = move || {
        // SAFETY: `kill` with a negative pid signals the group made above; no memory is touched.
        unsafe { libc::kill(-pgid, libc::SIGKILL) };
    };
    let mut errs = child.stderr.take();
    let reader = std::thread::spawn(move || {
        let mut kept = Vec::new();
        if let Some(e) = errs.as_mut() {
            let _ = e.by_ref().take(KEEP_ERR).read_to_end(&mut kept);
            let _ = std::io::copy(e, &mut std::io::sink());
        }
        String::from_utf8_lossy(&kept).into_owned()
    });
    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
    let timed_out = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let watchdog = {
        let timed_out = timed_out.clone();
        std::thread::spawn(move || {
            if done_rx.recv_timeout(deadline).is_err() {
                timed_out.store(true, std::sync::atomic::Ordering::SeqCst);
                kill_group();
            }
        })
    };
    let mut archive = Vec::new();
    let read = match child.stdout.as_mut() {
        Some(out) => out.take(crate::coder_promote::MAX_BYTES + 1).read_to_end(&mut archive).map(|_| ()),
        None => Ok(()),
    };
    if archive.len() as u64 > crate::coder_promote::MAX_BYTES {
        kill_group();
    }
    let status = child.wait();
    let _ = done_tx.send(());
    let _ = watchdog.join();
    let err = reader.join().unwrap_or_default();
    read?;
    let status = status?;
    if timed_out.load(std::sync::atomic::Ordering::SeqCst) {
        return Ok((false, archive, format!("it ran longer than {} seconds and was stopped", deadline.as_secs())));
    }
    Ok((status.success(), archive, err))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use yantrik_ipc_contracts::home_paths::workshops;

    fn argv(c: &Command) -> Vec<String> {
        c.get_args().map(OsStr::to_string_lossy).map(|s| s.into_owned()).collect()
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    /// The whole command, program first, whichever way the view wraps it.
    fn words(c: &Command) -> Vec<String> {
        std::iter::once(c.get_program().to_string_lossy().into_owned()).chain(argv(c)).collect()
    }

    /// The install of plain names, or the skip: a plan that errs is a failure wherever `bwrap` is required
    /// (`YANTRIK_REQUIRE_BWRAP=1`), and passes quietly nowhere; a plan of nothing is a failure.
    fn planned(home: &Path, language: &str, deps: &[String]) -> Option<(&'static str, Command, Vec<String>)> {
        match plan(home, language, deps) {
            Ok(Some(p)) => Some(p),
            Ok(None) => panic!("a plain name is installed ({language} {deps:?})"),
            Err(why) => {
                workshops::skip_live_test(&why);
                None
            }
        }
    }

    /// Bar 8: a command whose working folder is missing fails with "No such file or directory", as
    /// the first install on a machine with no `scripts/` did; after `ensure_install_dir` it runs.
    #[test]
    fn a_missing_scripts_folder_is_made_before_the_install_starts() {
        let root = std::env::temp_dir().join(format!("yantrik-bar8-{}", std::process::id()));
        let dir = root.join("a/b/scripts");
        let _ = std::fs::remove_dir_all(&root);
        let run = |d: &Path| Command::new("true").current_dir(d).output();
        assert_eq!(run(&dir).unwrap_err().kind(), std::io::ErrorKind::NotFound, "before: the spawn fails");
        ensure_install_dir(&dir).unwrap();
        assert!(run(&dir).unwrap().status.success(), "after: it starts");
        ensure_install_dir(&dir).unwrap(); // already there: fine
        let _ = std::fs::remove_dir_all(&root);
        // and install_deps calls it before the spawn
        let src = include_str!("coder_deps.rs").split("#[cfg(all(test").next().unwrap();
        let (before, after) = src.split_once("ensure_install_dir(&scripts)").unwrap();
        assert!(before.contains("allow_install()?") && after.contains("run_staged(command") && after.contains("promote("));
    }

    #[test]
    fn a_one_line_import_is_an_import() {
        assert_eq!(crate::coder::detect_python_deps("import requests; print(requests.__version__)\n"), ["requests"]);
        assert_eq!(crate::coder::detect_python_deps("x = 1; import yaml; import os\n"), ["pyyaml"]);
        assert!(crate::coder::detect_python_deps("print('a;b')\n# import zzz; y\n").is_empty());
        // The review's two cases: a trailing comment and a string are not statements.
        assert!(crate::coder::detect_python_deps("x = 1  # note; import foo\n").is_empty());
        assert!(crate::coder::detect_python_deps("s = \"a; import foo \"\n").is_empty());
        assert!(crate::coder::detect_python_deps("s = 'it\\'s; import foo'\n").is_empty());
        assert_eq!(crate::coder::detect_python_deps("s = '#'; import yaml  # import foo\n"), ["pyyaml"]);
    }

    #[test]
    fn only_a_plain_name_is_a_package() {
        for ok in ["requests", "PyJWT", "python-docx", "a.b_c-d", "x1"] {
            assert!(is_plain_package_name(ok), "{ok}");
        }
        for bad in [
            "", "pkgdir/", "x/", "../x", "-e", "--index-url", "x@file:..", "x@1.0", "@scope/pkg", "git+https://h/r", "file:..", "x y", "x;y",
            ".hidden", "_x", "http://h/p",
        ] {
            assert!(!is_plain_package_name(bad), "{bad}");
        }
    }

    /// The reviewer's test: `import x/` and `require("x@file:..")` give no install, and the name is told.
    #[test]
    fn a_path_or_a_spec_produces_no_install_and_is_named() {
        let home = crate::spawn::home();
        for dep in ["pkgdir/", "x/", "x@file:.."] {
            for language in ["python", "node"] {
                assert!(plan(&home, language, &names(&[dep])).unwrap().is_none(), "{language} {dep}");
                let note = install_deps(language, &names(&[dep])).unwrap();
                assert!(note.contains(dep), "{note}");
            }
        }
        // Mixed: the plain one is installed, the other is left out and named.
        if let Some((_, command, skipped)) = planned(&home, "python", &names(&["requests", "pkgdir/"])) {
            assert!(words(&command).contains(&"requests".to_string()));
            assert!(!words(&command).iter().any(|w| w.contains("pkgdir")));
            assert_eq!(skipped, ["pkgdir/"]);
        }
        // Through the code detector: `import pkgdir/` names a dependency that is skipped.
        let detected = crate::coder::detect_python_deps("import pkgdir/\nimport x/y\n");
        assert!(vet(&detected).0.is_empty(), "{detected:?}");
        let node = crate::coder::detect_node_deps("const a = require(\"x@file:..\");\n");
        assert!(vet(&node).0.is_empty(), "{node:?}");
    }

    /// The reviewer's test: the install starts behind `--tmpfs` over the base when it exists.
    #[test]
    fn the_install_starts_behind_a_tmpfs_over_the_workshops_base() {
        let home = crate::spawn::home();
        match plan(&home, "python", &names(&["requests"])) {
            Ok(Some((_, command, _))) => {
                if workshops::find_bwrap().is_some() {
                    assert!(command.get_program().to_string_lossy().ends_with("bwrap"));
                    if Path::new(workshops::BASE).is_dir() {
                        let a = argv(&command);
                        assert_eq!(a[3..5], ["--tmpfs", workshops::BASE], "{a:?}");
                    }
                } else {
                    // Without a view the plan cannot be a bare `pip3`: where the project requires bwrap this fails.
                    workshops::skip_live_test("no bwrap");
                    assert_eq!(command.get_program(), "pip3");
                }
            }
            Ok(None) => panic!("a plain name is installed"),
            Err(e) => assert!(e.contains("bwrap") && e.contains("nothing was run"), "{e}"),
        }
    }

    /// pip installs wheels only, so no `setup.py` of a package runs.
    #[test]
    fn pip_gets_only_binary_all() {
        let (program, fixed) = installer("python").unwrap();
        assert_eq!(program, "pip3");
        assert!(fixed.contains(&"--only-binary=:all:"), "{fixed:?}");
        if let Some((_, command, _)) = planned(&crate::spawn::home(), "python", &names(&["requests"])) {
            let w = words(&command);
            let at = w.iter().position(|x| x == "--only-binary=:all:").expect("the flag reaches the command");
            assert!(w.iter().position(|x| x == "requests").unwrap() > at, "{w:?}");
        }
    }

    /// The wheel goes to a private target, not to `~/.local`, where a `.pth` would run in every Python.
    #[test]
    fn pip_installs_into_a_private_target() {
        if let Some((_, command, _)) = planned(&crate::spawn::home(), "python", &names(&["requests"])) {
            let w = words(&command);
            let at = w.iter().position(|x| x == "--target").expect("--target reaches the command");
            // The install writes into the job's own `/tmp`; the shell promotes it afterwards.
            assert_eq!(w[at + 1], format!("{STAGE}/deps"), "{w:?}");
            let scripts = scripts_dir_in(&crate::spawn::home());
            assert!(!w.iter().any(|x| x.contains(scripts.to_str().unwrap())), "the scripts folder is not in the job's words: {w:?}");
            assert!(w.iter().position(|x| x == "requests").unwrap() > at + 1, "{w:?}");
        }
        if let Some((_, command, _)) = planned(&crate::spawn::home(), "node", &names(&["left-pad"])) {
            assert!(!words(&command).contains(&"--target".to_string()));
        }
    }

    /// Dependency installs work: the words are staged in `/tmp`, the result is promoted to the real
    /// folder, and the installer's own words are positional, never part of the shell text.
    #[test]
    fn an_install_is_staged_in_tmp_and_promoted_as_plain_files() {
        let home = Path::new("/home/p");
        assert_eq!(promoted_to(home, "python"), home.join(".config/yantrik/deps/python"));
        assert_eq!(promoted_to(home, "node"), home.join(".config/yantrik/deps/node_modules"));
        assert!(!promoted_to(home, "python").starts_with(scripts_dir_in(home)), "the dependencies are not in the scripts folder");
        assert!(!STAGE_SCRIPT.contains(".config") && STAGE_SCRIPT.contains("\"$@\"") && STAGE_SCRIPT.contains("tar --format=ustar"));
        // The stream a stage prints is what the shell writes into the folder (the reader's own tests
        // are in `coder_promote`): here, one that ends the way the script's does.
        let dest = std::env::temp_dir().join(format!("yantrik-deps-promote-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dest);
        let mut stream = vec![0u8; 0];
        stream.extend(vec![0u8; 1024]);
        assert_eq!(crate::coder_promote::promote(&stream, &dest.join(".deps"), &dest.join("stage")).unwrap(), 0);
        assert!(dest.join(".deps").is_dir());
        let _ = std::fs::remove_dir_all(&dest);
        if let Some((_, command, _)) = planned(&crate::spawn::home(), "python", &names(&["requests"])) {
            let w = words(&command);
            // The shell text is the stage script, the install's own folder is under the staging
            // folder in `/tmp`, and the working directory is the job's `/tmp`.
            assert!(w.contains(&STAGE_SCRIPT.to_string()) && w.contains(&format!("{STAGE}/deps")), "{w:?}");
            assert_eq!(command.get_current_dir(), Some(Path::new("/tmp")));
        }
    }

    /// npm runs no lifecycle script of a package.
    #[test]
    fn npm_gets_ignore_scripts() {
        for language in ["node", "javascript", "typescript"] {
            let (program, fixed) = installer(language).unwrap();
            assert_eq!(program, "npm");
            assert!(fixed.contains(&"--ignore-scripts") && fixed.contains(&"--no-bin-links"), "{fixed:?}");
        }
        if let Some((_, command, _)) = planned(&crate::spawn::home(), "node", &names(&["left-pad"])) {
            assert!(words(&command).contains(&"--ignore-scripts".to_string()) && words(&command).contains(&"--no-bin-links".to_string()));
        }
    }

    /// `gem install` runs an extension's `extconf.rb`: ruby is not installed automatically.
    #[test]
    fn ruby_is_never_installed_automatically() {
        assert!(installer("ruby").is_none());
        assert!(plan(&crate::spawn::home(), "ruby", &names(&["nokogiri"])).unwrap().is_none());
        assert!(install_deps("ruby", &names(&["nokogiri"])).unwrap().is_empty());
    }
}
