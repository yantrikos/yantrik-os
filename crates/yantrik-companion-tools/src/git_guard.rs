//! Git, run on the person's folders and never on a Mind's workshop (#1124, M1).
//!
//! A workshop holds anything a Mind's sealed job wrote, `.git/config` with a `core.fsmonitor` or
//! a filter included, and `git` runs the programs such a config names. `validate_path` and
//! `Root::holds` look at the folder a tool is given, not at where git then goes: a link outside
//! the root that points into it, or a `.git` *file* (`gitdir: <workshop>/.git`), sends git into
//! the workshop from a folder that is not in it. So before any git runs, [`vet`] asks git where
//! the repository really is, with nothing to run: the answer is checked, and the real command is
//! built by [`command`], which also turns off the two settings that start a program on their
//! own.

use std::path::Path;
use std::process::Command;

/// Settings that start a program by themselves, turned off for every git the tools run.
const NO_PROGRAMS: [&str; 4] = ["-c", "core.fsmonitor=false", "-c", "core.hooksPath=/dev/null"];

/// `git` in `dir` for a tool, with the settings that run programs off and discovery kept from
/// climbing into the workshops root.
pub fn command(dir: &str) -> Command {
    let mut cmd = Command::new("git");
    cmd.current_dir(dir).args(NO_PROGRAMS);
    if let Some(home) = std::env::var_os("HOME") {
        let root = yantrik_ipc_contracts::home_paths::workshops::base(Path::new(&home));
        cmd.env("GIT_CEILING_DIRECTORIES", root);
    }
    cmd
}

/// `Ok` when the repository `dir` is in has its git directory and work tree outside every
/// workshop, as `held` says; `Err(why)` for a tool to return. A folder that is no repository is
/// `Ok`: git says so itself.
pub fn vet_with(dir: &str, held: &dyn Fn(&Path) -> bool) -> Result<(), String> {
    let mut probe = command(dir);
    probe
        .args(["rev-parse", "--absolute-git-dir", "--show-toplevel"])
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_OPTIONAL_LOCKS", "0");
    let Ok(out) = probe.output() else { return Ok(()) };
    if !out.status.success() {
        return Ok(());
    }
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if held(Path::new(line.trim())) {
            return Err("that repository lives in a Mind's workshop (it is reached through a link or a .git file); git is not run there.".into());
        }
    }
    Ok(())
}

/// [`vet_with`] against the workshops root and the other protected places.
pub fn vet(dir: &str) -> Result<(), String> {
    vet_with(dir, &yantrik_ipc_contracts::home_paths::is_protected)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn git_here() -> bool {
        Command::new("git").arg("--version").output().is_ok_and(|o| o.status.success())
    }

    fn git(dir: &Path, args: &[&str]) {
        assert!(Command::new("git").current_dir(dir).args(args).output().unwrap().status.success(), "git {args:?}");
    }

    #[test]
    fn a_link_or_a_git_file_into_a_workshop_is_not_run_and_a_plain_repository_is() {
        if !git_here() {
            return;
        }
        let tmp = std::env::temp_dir().join(format!("yantrik-git-guard-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let base = tmp.canonicalize().unwrap();
        let root = base.join("workshops");
        let shop = root.join("hermes/shop");
        std::fs::create_dir_all(&shop).unwrap();
        git(&shop, &["init", "-q"]);
        let marker = base.join("PWNED");
        git(&shop, &["config", "core.fsmonitor", &format!("touch {}; false", marker.display())]);
        std::fs::create_dir_all(base.join("Projects/plain")).unwrap();
        git(&base.join("Projects/plain"), &["init", "-q"]);
        let held = |p: &Path| p.starts_with(&root);

        // ~/Projects/shop is a link to the workshop.
        std::os::unix::fs::symlink(&shop, base.join("Projects/shop")).unwrap();
        // ~/Projects/app/.git is a file that points into it.
        std::fs::create_dir_all(base.join("Projects/app")).unwrap();
        std::fs::write(base.join("Projects/app/.git"), format!("gitdir: {}\n", shop.join(".git").display())).unwrap();
        for bad in ["Projects/shop", "Projects/app"] {
            let why = vet_with(base.join(bad).to_str().unwrap(), &held).unwrap_err();
            assert!(why.contains("workshop"), "{bad}: {why}");
        }
        assert!(vet_with(base.join("Projects/plain").to_str().unwrap(), &held).is_ok());
        assert!(vet_with(base.join("Projects").to_str().unwrap(), &held).is_ok(), "no repository: git says so itself");
        // And what was asked never ran a thing.
        assert!(!marker.exists());
        // The real command has the two program-starting settings off, even when vetting is wrong.
        let status = command(shop.to_str().unwrap()).args(["status", "--short"]).output().unwrap();
        assert!(status.status.success());
        assert!(!marker.exists(), "core.fsmonitor ran");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
