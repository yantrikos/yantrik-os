//! `git` as the companion runs it in a folder it did not make (#1124, N1).
//!
//! git runs a program for several settings in a repository's own `.git/config` (and for the
//! attributes it names). A repository a Mind wrote is such a folder, and so is any clone. Every
//! key below runs a program, so each is set to nothing here, and the environment git also reads
//! is cleaned. `workshop_gate` keeps these calls out of a workshop in the first place; this is
//! the second layer, for a folder that is not a workshop yet.
//!
//! Not covered, because the key's name is in the repository: `filter.<name>.clean`,
//! `diff.<name>.textconv` and `merge.<name>.driver` run only for a path whose attributes name
//! them. `--no-textconv` and `--no-ext-diff` are given to the commands that print a diff.

use std::process::Command;

/// Every config key that makes git run a program, set to nothing (or the null path).
const NO_PROGRAM: &[&str] = &[
    "core.fsmonitor=false",
    "core.hooksPath=/dev/null",
    "core.pager=cat",
    "core.editor=true",
    "core.askPass=true",
    "core.sshCommand=false",
    "core.gitProxy=false",
    "core.attributesFile=/dev/null",
    "diff.external=",
    "sequence.editor=true",
    "credential.helper=",
    "gpg.program=false",
    "gpg.openpgp.program=false",
    "gpg.x509.program=false",
    "gpg.ssh.program=false",
    "gpg.ssh.defaultKeyCommand=",
    "uploadpack.packObjectsHook=",
    "protocol.ext.allow=never",
];

/// A `git` command with the program-running keys blanked and a clean environment. The caller
/// adds `current_dir` and the arguments.
pub fn command() -> Command {
    let mut cmd = Command::new("git");
    for kv in NO_PROGRAM {
        cmd.arg("-c").arg(kv);
    }
    cmd.env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_PAGER", "cat")
        .env("GIT_EDITOR", "true")
        .env("GIT_ASKPASS", "true")
        .env("GIT_SSH_COMMAND", "false")
        .env_remove("GIT_EXTERNAL_DIFF")
        .env_remove("GIT_SSH")
        .env_remove("GIT_PROXY_COMMAND")
        .env_remove("GIT_CONFIG_PARAMETERS")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE");
    cmd
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn git_ok(dir: &std::path::Path, args: &[&str]) {
        let ok = Command::new("git").current_dir(dir).args(args).status().map(|s| s.success()).unwrap_or(false);
        assert!(ok, "git {args:?}");
    }

    /// A repository whose config would run `touch <marker>` when git asks for the status.
    fn planted_repo(root: &std::path::Path) -> std::path::PathBuf {
        let repo = root.join("shop");
        std::fs::create_dir_all(&repo).unwrap();
        git_ok(&repo, &["init", "-q"]);
        let marker = root.join("PWNED");
        let hook = format!("touch {} #", marker.display());
        git_ok(&repo, &["config", "core.fsmonitor", &hook]);
        git_ok(&repo, &["config", "diff.external", &format!("touch {} #", marker.display())]);
        std::fs::write(repo.join("a.txt"), "a").unwrap();
        repo
    }

    #[test]
    fn a_planted_fsmonitor_never_runs_from_a_companion_git_call() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let root = std::env::temp_dir().join(format!("yantrik-safe-git-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let repo = planted_repo(&root);
        let marker = root.join("PWNED");

        for args in [&["status", "--short"][..], &["diff", "--no-ext-diff"], &["log", "--oneline", "-1"]] {
            let _ = command().current_dir(&repo).args(args).output();
            assert!(!marker.exists(), "git {args:?} ran the planted program");
        }

        // The gate: with the shell's rule installed, the tools do not even start git there.
        let shop = repo.to_string_lossy().to_string();
        let shop2 = shop.clone();
        crate::workshop_gate::install(move |p| p.starts_with(&shop2).then(|| "It is a workshop.".to_string()));
        assert!(crate::workshop_gate::refuse(&shop).is_err());
        assert!(crate::git::run_git(&shop, &["status"]).starts_with("Error:"));
        assert!(!marker.exists());
        assert!(crate::workshop_gate::refuse("/tmp").is_ok());
        let _ = std::fs::remove_dir_all(&root);
    }
}
