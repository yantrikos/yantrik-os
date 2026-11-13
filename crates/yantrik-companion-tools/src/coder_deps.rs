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
//! - puts a Python wheel in a private target, `<scripts>/.deps` (`pip3 install --target`), and the
//!   script sees it only through `PYTHONPATH`. Python runs a `.pth` file only from a site
//!   directory, and a `PYTHONPATH` entry is not one, so a `.pth` a wheel ships never runs, in the
//!   script or in any host Python (a `--user` install would put it where every later one reads it);
//! - has no ruby path: `gem install` runs an extension's `extconf.rb`, so a gem is never
//!   installed automatically.

use std::path::Path;
use std::process::Command;

/// The private target a Python install goes to, and the one `PYTHONPATH` names for the script.
fn deps_dir() -> String {
    format!("{SCRIPTS_DIR}/.deps")
}

/// The environment of a script run: where it writes, where its scripts are, and for Python the
/// private `.deps` its installs went to.
pub fn script_env(command: &mut Command, output: &Path, scripts: &Path, language: &str) {
    command.env("YANTRIK_OUTPUT_DIR", output.to_str().unwrap_or("/tmp"));
    command.env("YANTRIK_SCRIPTS_DIR", scripts.to_str().unwrap_or("/tmp"));
    if language == "python" {
        command.env("PYTHONPATH", deps_dir());
    }
}

/// Where the scripts live, and so where an install runs.
const SCRIPTS_DIR: &str = "/home/yantrik/.config/yantrik/scripts";

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
        "node" | "javascript" | "typescript" => Some(("npm", &["install", "--no-save", "--silent", "--ignore-scripts"])),
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
    let mut argv: Vec<String> = fixed.iter().map(|a| a.to_string()).collect();
    if language == "python" {
        argv.extend(["--target".to_string(), deps_dir()]);
    }
    argv.extend(plain);
    let mut command = crate::spawn::mind_command(home, program, &argv)?;
    command.current_dir(SCRIPTS_DIR);
    Ok(Some((program, command, skipped)))
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
    let planned = plan(&crate::spawn::home(), language, deps).map_err(|why| format!("Dependency install refused: {why}"))?;
    let Some((program, mut command, skipped)) = planned else {
        // Nothing is installed: ruby, or no name was plain. A name left out is still named.
        return Ok(if installer(language).is_some() { skipped_note(&vet(deps).1) } else { String::new() });
    };
    // pip and npm reach their public registries, outside the sandbox: that is a fetch from the open
    // web, and it waits for a turn the person started or for research to be on. Asked only when an
    // install would really run, so a refusal still leaves nothing written.
    crate::coder_sandbox::allow_install()?;
    tracing::info!(lang = language, deps = ?deps, "Auto-installing dependencies");
    match command.output() {
        Ok(output) if output.status.success() => Ok(skipped_note(&skipped)),
        Ok(output) => {
            let err = String::from_utf8_lossy(&output.stderr);
            Err(format!("Dependency install failed ({program}): {}", err.chars().take(500).collect::<String>()))
        }
        Err(e) => Err(format!("Could not run {program}: {e}")),
    }
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
        if let Ok(Some((_, command, skipped))) = plan(&home, "python", &names(&["requests", "pkgdir/"])) {
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
        if let Ok(Some((_, command, _))) = plan(&crate::spawn::home(), "python", &names(&["requests"])) {
            let w = words(&command);
            let at = w.iter().position(|x| x == "--only-binary=:all:").expect("the flag reaches the command");
            assert!(w.iter().position(|x| x == "requests").unwrap() > at, "{w:?}");
        }
    }

    /// The wheel goes to a private target, not to `~/.local`, where a `.pth` would run in every Python.
    #[test]
    fn pip_installs_into_a_private_target() {
        if let Ok(Some((_, command, _))) = plan(&crate::spawn::home(), "python", &names(&["requests"])) {
            let w = words(&command);
            let at = w.iter().position(|x| x == "--target").expect("--target reaches the command");
            assert_eq!(w[at + 1], format!("{SCRIPTS_DIR}/.deps"), "{w:?}");
            assert!(w.iter().position(|x| x == "requests").unwrap() > at + 1, "{w:?}");
        }
        if let Ok(Some((_, command, _))) = plan(&crate::spawn::home(), "node", &names(&["left-pad"])) {
            assert!(!words(&command).contains(&"--target".to_string()));
        }
    }

    /// The script's environment carries `PYTHONPATH` for Python, and only for Python.
    #[test]
    fn a_python_script_gets_pythonpath_to_the_private_target() {
        let env = |language: &str| {
            let mut c = Command::new("x");
            script_env(&mut c, Path::new("/o"), Path::new(SCRIPTS_DIR), language);
            c.get_envs().map(|(k, v)| (k.to_string_lossy().into_owned(), v.map(|v| v.to_string_lossy().into_owned()))).collect::<Vec<_>>()
        };
        let py = env("python");
        assert!(py.contains(&("PYTHONPATH".into(), Some(format!("{SCRIPTS_DIR}/.deps")))), "{py:?}");
        assert!(py.iter().any(|(k, _)| k == "YANTRIK_OUTPUT_DIR") && py.iter().any(|(k, _)| k == "YANTRIK_SCRIPTS_DIR"));
        assert!(!env("node").iter().any(|(k, _)| k == "PYTHONPATH"));
    }

    /// npm runs no lifecycle script of a package.
    #[test]
    fn npm_gets_ignore_scripts() {
        for language in ["node", "javascript", "typescript"] {
            let (program, fixed) = installer(language).unwrap();
            assert_eq!(program, "npm");
            assert!(fixed.contains(&"--ignore-scripts"), "{fixed:?}");
        }
        if let Ok(Some((_, command, _))) = plan(&crate::spawn::home(), "node", &names(&["left-pad"])) {
            assert!(words(&command).contains(&"--ignore-scripts".to_string()));
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
