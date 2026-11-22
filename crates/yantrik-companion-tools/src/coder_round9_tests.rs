//! Round 9 of #1265 (the decision on the round 8 read): where the dependencies folder is, the
//! installer's output and deadline, and the variables an installer gets. The live ones skip without
//! `bwrap` and fail instead where `YANTRIK_REQUIRE_BWRAP=1` (the project runner).

use crate::coder_deps::{check_target, network_env_for_tests, run_staged_for_tests, scripts_dir_in, stage_view_for_tests};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};
use yantrik_ipc_contracts::home_paths::workshops::{find_bwrap, skip_live_test};

fn pairs(v: &[(&str, &str)]) -> Vec<(String, String)> {
    v.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

/// Item 1: the scripts folder comes from the account's home, and an install whose target is not
/// under it is refused with a sentence that says why.
#[test]
fn the_scripts_folder_is_under_the_accounts_home_and_a_target_elsewhere_refuses() {
    let home = Path::new("/home/someone-else");
    assert_eq!(scripts_dir_in(home), Path::new("/home/someone-else/.config/yantrik/scripts"));
    assert!(check_target(home, &scripts_dir_in(home).join(".deps")).is_ok());
    for bad in ["/home/yantrik/.config/yantrik/scripts/.deps", "/home/someone-else", "/home/someone-else2/x", "/srv/scripts/.deps", "/"] {
        let why = check_target(home, Path::new(bad)).unwrap_err();
        assert!(why.contains("not under this account's home") && why.contains("nothing was installed"), "{bad}: {why}");
    }
    let src = include_str!("coder.rs").to_string() + include_str!("coder_deps.rs").split("#[cfg(all(test").next().unwrap();
    assert!(!src.contains("/home/yantrik"), "no fixed account name is left in the code");
}

/// Item 1, live: the install's own view has neither the folder nor anything in it (not only: the
/// command's words do not name it).
#[test]
fn the_installs_own_view_has_neither_the_scripts_folder_nor_anything_in_it() {
    let top = std::env::temp_dir().join(format!("yantrik-r9-view-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&top);
    let home = top.join("home");
    let scripts = scripts_dir_in(&home);
    std::fs::create_dir_all(scripts.join(".deps/pkg")).unwrap();
    std::fs::write(scripts.join(".deps/pkg/mod.py"), "x = 1").unwrap();
    std::fs::write(scripts.join("registry.json"), "{}").unwrap();
    std::fs::create_dir_all(home.join("Documents")).unwrap();
    yantrik_ipc_contracts::home_paths::outside_view::account_home_for_tests(Some(home.clone()));
    let probe = "for p in \"$@\"; do if [ -e \"$p\" ]; then echo \"VISIBLE $p\"; fi; done; ls -A \"$HOME\"; echo END";
    let mut argv: Vec<String> = ["-c", probe, "sh"].map(String::from).to_vec();
    argv.extend([scripts.clone(), scripts.join(".deps"), scripts.join(".deps/pkg/mod.py"), scripts.join("registry.json"), home.join(".config")].map(|p| p.display().to_string()));
    let run = if find_bwrap().is_some() { stage_view_for_tests(&home, &argv).ok().map(|mut c| c.output()) } else { None };
    yantrik_ipc_contracts::home_paths::outside_view::account_home_for_tests(None);
    match run {
        Some(Ok(out)) if String::from_utf8_lossy(&out.stdout).contains("END") => {
            let said = String::from_utf8_lossy(&out.stdout);
            assert!(!said.contains("VISIBLE"), "the install's view shows the dependencies folder: {said}");
            assert!(!said.lines().any(|l| l == ".config"), "{said}");
        }
        Some(Ok(out)) => skip_live_test(&String::from_utf8_lossy(&out.stderr)),
        _ => skip_live_test("no bwrap, or the view did not start"),
    }
    let _ = std::fs::remove_dir_all(&top);
}

/// Item 3: an installer that writes far more than the kept part of its standard error is not killed
/// by a broken pipe; the first 64 KiB are kept and the rest is thrown away.
#[test]
fn a_chatty_installer_is_not_killed_and_only_the_first_64_kib_are_kept() {
    let mut c = Command::new("sh");
    c.args(["-c", "head -c 400000 /dev/zero | tr '\\0' e >&2; printf archive"]).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    let (ok, out, err) = run_staged_for_tests(c, Duration::from_secs(60)).unwrap();
    assert!(ok, "it finished and succeeded");
    assert_eq!(out, b"archive");
    assert_eq!(err.len(), 64 << 10);
}

/// Item 3: a stalled install is stopped at the deadline, the whole process group with it, and the
/// answer says so.
#[test]
fn a_stalled_installer_and_its_children_are_killed_at_the_deadline() {
    let marker = std::env::temp_dir().join(format!("yantrik-r9-stall-{}", std::process::id()));
    let _ = std::fs::remove_file(&marker);
    let script = format!("(sleep 3; echo alive > {}) & sleep 60", marker.display());
    let mut c = Command::new("sh");
    c.args(["-c", &script]).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    let started = Instant::now();
    let (ok, _, err) = run_staged_for_tests(c, Duration::from_secs(1)).unwrap();
    assert!(!ok && err.contains("ran longer than 1 seconds"), "{err}");
    assert!(started.elapsed() < Duration::from_secs(30), "it was stopped at the deadline, not at the end");
    std::thread::sleep(Duration::from_secs(4));
    assert!(!marker.exists(), "a child of the installer outlived the deadline");
}

/// Item 7: the proxy and certificate variables, and no other, reach the installer.
#[test]
fn only_the_proxy_and_certificate_variables_reach_the_installer() {
    let src = pairs(&[
        ("HTTP_PROXY", "p"), ("HTTPS_PROXY", "p"), ("NO_PROXY", "n"), ("http_proxy", "p"), ("https_proxy", "p"), ("no_proxy", "n"),
        ("SSL_CERT_FILE", "/c"), ("SSL_CERT_DIR", "/d"), ("ANTHROPIC_API_KEY", "k"), ("SSH_AUTH_SOCK", "s"), ("PATH", "/x"), ("ALL_PROXY", "a"),
    ]);
    let got = network_env_for_tests(src);
    assert_eq!(got.len(), 8, "{got:?}");
    assert!(got.iter().all(|(k, _)| k.contains("PROXY") && k != "ALL_PROXY" || k.contains("proxy") || k.starts_with("SSL_CERT")), "{got:?}");
    let env = yantrik_ipc_contracts::home_paths::view_env::allowed(pairs(&[("ANTHROPIC_API_KEY", "k")]), Path::new("/home/p"), &got);
    let names: Vec<&str> = env.iter().map(|(k, _)| k.as_str()).collect();
    assert!(names.contains(&"HTTPS_PROXY") && names.contains(&"SSL_CERT_FILE") && !names.contains(&"ANTHROPIC_API_KEY"), "{names:?}");
}
