//! Round 10 of #1265: the dependencies folder is out of the scripts folder and read-only to a
//! script, an install is staged and promoted through descriptors, and the script sandbox's
//! environment is the allow list. The live ones need `bwrap`: they skip without it and fail
//! instead where `YANTRIK_REQUIRE_BWRAP=1` (the project runner).

use crate::coder_deps::{deps_root_in, run_staged_for_tests, script_view, staging_in, STAGE_SCRIPT_FOR_TESTS};
use crate::spawn::{sandboxed_script, SandboxMode};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use yantrik_ipc_contracts::home_paths::workshops::skip_live_test;

fn scratch(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("yantrik-r10-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

/// Run `script` in the script sandbox over `scripts` with the dependencies of `home` laid over it, as
/// `execute_script` starts it. `None` (after the skip) where no sandbox can be made here.
fn in_sandbox(home: &Path, scripts: &Path, extra: &[(String, String)], script: &str) -> Option<String> {
    if !Path::new("/usr/bin/bwrap").exists() {
        skip_live_test("no /usr/bin/bwrap");
        return None;
    }
    let out = scripts.join("output");
    std::fs::create_dir_all(&out).unwrap();
    let (mut reads, mut named) = script_view(&out, scripts, "python");
    // The view takes the account's home from the account entry; a test names its own.
    for (source, _) in reads.iter_mut() {
        let rest = source.strip_prefix(crate::spawn::account_home().unwrap_or_else(|_| crate::spawn::home())).map(Path::to_path_buf).unwrap_or_default();
        *source = home.join(rest);
    }
    named.extend(extra.iter().cloned());
    let run = sandboxed_script(SandboxMode::Strict, scripts, &out, &reads, &named, script).output().ok()?;
    let err = String::from_utf8_lossy(&run.stderr);
    if err.contains("bwrap:") {
        skip_live_test(&err);
        return None;
    }
    Some(String::from_utf8_lossy(&run.stdout).into_owned())
}

/// A. A coder script cannot write the dependencies folder, and can still import an installed package.
#[test]
fn a_script_imports_an_installed_package_and_cannot_write_the_dependencies_folder() {
    let top = scratch("ro");
    let home = top.join("home");
    let scripts = top.join("scripts");
    std::fs::create_dir_all(deps_root_in(&home).join("python/pkgmod")).unwrap();
    std::fs::write(deps_root_in(&home).join("python/pkgmod/__init__.py"), "X = 'installed-r10'\n").unwrap();
    let probe = "if echo x > \"$YANTRIK_SCRIPTS_DIR/.deps/new\" 2>/dev/null; then echo WROTE; else echo READONLY; fi; \
                 if mkdir \"$YANTRIK_SCRIPTS_DIR/.deps/d\" 2>/dev/null; then echo MADE; else echo NODIR; fi; \
                 if rm -r \"$YANTRIK_SCRIPTS_DIR/.deps/pkgmod\" 2>/dev/null; then echo REMOVED; else echo KEPT; fi; \
                 python3 -c 'import pkgmod; print(pkgmod.X)' 2>&1 || echo NOIMPORT";
    if let Some(said) = in_sandbox(&home, &scripts, &[], probe) {
        assert!(said.contains("READONLY") && said.contains("NODIR") && said.contains("KEPT"), "{said}");
        assert!(!said.contains("WROTE") && !said.contains("MADE") && !said.contains("REMOVED"), "{said}");
        if !said.contains("NOIMPORT") && !said.contains("not found") {
            assert!(said.contains("installed-r10"), "an installed package imports: {said}");
        }
    }
    assert!(!deps_root_in(&home).join("python/pkgmod/new").exists() && deps_root_in(&home).join("python/pkgmod/__init__.py").exists());
    let _ = std::fs::remove_dir_all(top);
}

/// B. An API key and the session bus address of the shell do not reach a script; the allow list and the
/// door's own named variables do, with `HOME` the sandbox's own.
#[test]
fn a_script_does_not_see_the_shells_keys_or_session_bus() {
    let top = scratch("env");
    std::env::set_var("YANTRIK_TEST_API_KEY_R10", "hunter2");
    std::env::set_var("DBUS_SESSION_BUS_ADDRESS", "unix:path=/run/user/1000/bus");
    let said = in_sandbox(&top.join("home"), &top.join("scripts"), &[], "env");
    std::env::remove_var("YANTRIK_TEST_API_KEY_R10");
    std::env::remove_var("DBUS_SESSION_BUS_ADDRESS");
    if let Some(said) = said {
        assert!(!said.contains("hunter2") && !said.contains("DBUS_SESSION") && !said.contains("API_KEY"), "{said}");
        assert!(said.lines().any(|l| l == "HOME=/tmp") && said.contains("YANTRIK_OUTPUT_DIR=") && said.contains("PYTHONPATH="), "{said}");
    }
    let _ = std::fs::remove_dir_all(top);
}

/// C. An install end to end with a local fake installer and no network: the staged run, the
/// descriptor-relative promote into `deps/python`, the old copy kept aside, and what an earlier
/// install put there still there.
#[test]
fn a_fake_installer_is_staged_promoted_and_the_next_install_keeps_the_first() {
    let top = scratch("e2e");
    let home = top.join("home");
    let target = deps_root_in(&home).join("python");
    for (tag, package) in [("one", "alpha"), ("two", "beta")] {
        let out = format!("/tmp/yantrik-stage-r10-{}-{tag}", std::process::id());
        let fake = top.join(format!("fake-installer-{tag}"));
        std::fs::write(&fake, format!("#!/bin/sh\nmkdir -p \"$1/{package}\" && echo 'NAME = \"{package}\"' > \"$1/{package}/__init__.py\" && chmod 755 \"$1/{package}\"\n")).unwrap();
        let mut c = Command::new("/bin/sh");
        c.args(["-c", STAGE_SCRIPT_FOR_TESTS, "sh", &out, "/bin/sh", fake.to_str().unwrap(), &out]);
        c.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
        let (ok, archive, err) = run_staged_for_tests(c, Duration::from_secs(60)).unwrap();
        assert!(ok, "{err}");
        let n = crate::coder_promote::promote(&archive, &target, &staging_in(&home)).unwrap();
        assert_eq!(n, 1, "one file written");
        let _ = std::fs::remove_dir_all(&out);
    }
    assert!(target.join("alpha/__init__.py").is_file() && target.join("beta/__init__.py").is_file(), "the first install stays");
    let beside: Vec<String> = std::fs::read_dir(deps_root_in(&home)).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    assert!(beside.iter().any(|n| n.starts_with("python.replaced-")), "the old copy is kept aside: {beside:?}");
    assert_eq!(std::fs::read_dir(staging_in(&home)).unwrap().count(), 0, "nothing is left in the staging folder");
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(staging_in(&home)).unwrap().permissions().mode() & 0o777, 0o700);
    let _ = std::fs::remove_dir_all(top);
}

/// A. The staging folder is in neither the scripts folder nor the dependencies folder, and the
/// dependencies are not in the scripts folder.
#[test]
fn the_dependencies_and_the_staging_are_outside_the_scripts_folder() {
    let home = Path::new("/home/p");
    let scripts = crate::coder_deps::scripts_dir_in(home);
    for place in [deps_root_in(home), staging_in(home)] {
        assert!(place.starts_with(home.join(".config/yantrik")) && !place.starts_with(&scripts) && !scripts.starts_with(&place), "{place:?}");
    }
    assert!(!staging_in(home).starts_with(deps_root_in(home)));
}
