//! The headless half of the binary: build, verify and screenshot without a window.
//!
//! The window needs a display and the control surface needs the window, but a script —
//! or this kit's own real run on a headless box — still has to be able to turn a spec
//! into a game and prove the game plays. Same library, same compiler, same verifier;
//! only the driver is different. Exit codes: 0 done, 1 a gate failed, 2 refused.

use std::path::{Path, PathBuf};

use crate::compile;
use crate::library::Library;
use crate::spec::{parse_game, ResolvedGame};
use crate::verify;

/// Run a CLI command if the arguments ask for one. `None` means "no subcommand:
/// start the window"; `Some(code)` is the exit status the process should end with.
pub fn run(args: Vec<String>) -> Option<i32> {
    let mut it = args.into_iter();
    let _binary = it.next();
    let command = it.next()?;
    let rest: Vec<String> = it.collect();
    Some(match command.as_str() {
        "build" => build(rest),
        "verify" => verify_cmd(rest),
        "screenshot" => screenshot(rest),
        "help" | "--help" | "-h" => {
            print!("{}", USAGE);
            0
        }
        other => {
            eprintln!("arcade: unknown command `{other}`\n{USAGE}");
            2
        }
    })
}

const USAGE: &str = "\
usage: yantrik-arcade                       open the workbench window
       yantrik-arcade build <spec.json> <out.html>
       yantrik-arcade verify <game.html> [--screenshot out.png] [--json out.json]
       yantrik-arcade screenshot <game.html> <out.png>

The library lives at ~/.local/share/yantrik/arcade, or wherever YANTRIK_ARCADE_DIR
points. `build` resolves a by-name `player.character` against that library.
`verify` prints the report JSON and exits 1 when any gate fails.";

fn fail(message: String) -> i32 {
    eprintln!("arcade: {message}");
    2
}

fn build(args: Vec<String>) -> i32 {
    if args.len() != 2 {
        return fail("build wants exactly two paths: <spec.json> <out.html>".into());
    }
    let spec_path = PathBuf::from(&args[0]);
    let out_path = PathBuf::from(&args[1]);
    let text = match std::fs::read_to_string(&spec_path) {
        Ok(t) => t,
        Err(e) => return fail(format!("cannot read {}: {e}", spec_path.display())),
    };
    let result = parse_game(&text)
        .and_then(|spec| ResolvedGame::resolve(spec, &|name| Library::open().load_character(name)))
        .map(|resolved| compile::compile(&resolved))
        .and_then(|html| {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
            }
            std::fs::write(&out_path, html).map_err(|e| format!("cannot write {}: {e}", out_path.display()))
        });
    match result {
        Ok(()) => {
            println!("{}", out_path.display());
            0
        }
        Err(message) => fail(message),
    }
}

fn verify_cmd(args: Vec<String>) -> i32 {
    let mut it = args.into_iter();
    let Some(html) = it.next().map(PathBuf::from) else {
        return fail("verify wants the path of a built game.html".into());
    };
    let mut screenshot: Option<PathBuf> = None;
    let mut json_out: Option<PathBuf> = None;
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--screenshot" => {
                screenshot = it.next().map(PathBuf::from);
            }
            "--json" => {
                json_out = it.next().map(PathBuf::from);
            }
            other => return fail(format!("verify does not know `{other}`")),
        }
    }
    if !html.exists() {
        return fail(format!("{} does not exist; build the game first", html.display()));
    }
    if let Some(parent) = screenshot.as_deref().and_then(Path::parent) {
        let _ = std::fs::create_dir_all(parent);
    }
    match verify::verify_game(&html, screenshot.as_deref()) {
        Ok(report) => {
            let json = serde_json::to_string_pretty(&report).unwrap_or_default();
            println!("{json}");
            if let Some(path) = json_out {
                let _ = std::fs::write(&path, json);
            }
            if report.passed { 0 } else { 1 }
        }
        Err(message) => fail(message),
    }
}

fn screenshot(args: Vec<String>) -> i32 {
    if args.len() != 2 {
        return fail("screenshot wants exactly two paths: <game.html> <out.png>".into());
    }
    let html = PathBuf::from(&args[0]);
    let out = PathBuf::from(&args[1]);
    if !html.exists() {
        return fail(format!("{} does not exist; build the game first", html.display()));
    }
    if let Some(parent) = out.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match verify::screenshot_game(&html, &out) {
        Ok(()) => {
            println!("{}", out.display());
            0
        }
        Err(message) => fail(message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_subcommand_means_open_the_window() {
        assert!(run(vec!["yantrik-arcade".into()]).is_none());
    }

    #[test]
    fn help_is_an_exit_zero() {
        assert_eq!(run(vec!["yantrik-arcade".into(), "help".into()]), Some(0));
    }

    #[test]
    fn unknown_commands_are_refused_with_the_usage() {
        assert_eq!(run(vec!["yantrik-arcade".into(), "dance".into()]), Some(2));
    }

    #[test]
    fn build_refuses_a_missing_file_with_a_sentence() {
        let code = run(vec![
            "yantrik-arcade".into(), "build".into(),
            "/nonexistent/spec.json".into(), "/tmp/out.html".into(),
        ]);
        assert_eq!(code, Some(2));
    }

    #[test]
    fn build_refuses_a_bad_spec_with_the_validators_sentence() {
        let dir = std::env::temp_dir().join(format!("arcade-cli-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let spec = dir.join("spec.json");
        std::fs::write(&spec, r##"{ "title": "X" }"##).unwrap(); // no player
        let code = run(vec![
            "yantrik-arcade".into(), "build".into(),
            spec.display().to_string(), dir.join("out.html").display().to_string(),
        ]);
        assert_eq!(code, Some(2));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn build_compiles_a_whole_game_from_a_file() {
        let dir = std::env::temp_dir().join(format!("arcade-cli-build-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let spec = dir.join("spec.json");
        std::fs::write(
            &spec,
            r##"{
                "title": "CLI Check",
                "player": { "character": {
                    "name": "Pip",
                    "palette": { "base": "#44cc88", "belly": "#f2ead8", "accent": "#ff9f43", "nose": "#2f3640", "eye": "#2f3640" }
                } },
                "collectible": { "kind": "coin", "count": 4 }
            }"##,
        )
        .unwrap();
        let out = dir.join("game.html");
        let code = run(vec![
            "yantrik-arcade".into(), "build".into(),
            spec.display().to_string(), out.display().to_string(),
        ]);
        assert_eq!(code, Some(0));
        let html = std::fs::read_to_string(&out).unwrap();
        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains("window.ARCADE_GAME"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_refuses_a_missing_file_by_name() {
        let code = run(vec!["yantrik-arcade".into(), "verify".into(), "/nonexistent/game.html".into()]);
        assert_eq!(code, Some(2));
    }
}
