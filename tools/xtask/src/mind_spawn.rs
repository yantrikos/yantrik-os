//! Rule `mind-spawn` of `check-architecture` (#1124, round 9; Fable's decision of 10 Nov).
//!
//! A program whose name, path or executed content a Mind chooses, or that asks another manager to
//! start one, starts only through one of three doors: `spawn::mind_command`, `browser_spawn`, or
//! the shell's `open_app` (the companion's tools never build the last one). `safe_command` is the
//! door for allowlisted utilities with protected paths. So in `yantrik-companion-tools/src` and
//! `yantrik-companion/src` (nested files too), outside `spawn.rs`, `browser_spawn.rs`,
//! `safe_command.rs` (the doors) and test-only code (exactly `#[cfg(test)]` or
//! `#[cfg(all(test, ..))]`; `any(test, ..)` can compile into a release), this fails on:
//!
//! - a `Command::new(` whose argument is not a string literal;
//! - a literal on the [`DENIED`] list (the shells, `env`, launchers, installers, interpreters,
//!   browsers and `claude`), however it is spelled (`/bin/sh` is `sh`);
//! - `Command` renamed on import (`use ... Command as C`), which would hide the two above;
//! - a `"-c"` handed to a program: `.arg("-c")`, or `"-c"` inside `.args(`.
//!
//! A line may carry `// fixed-program: a|b`, naming every program it can start; that is accepted
//! for a non-literal `Command::new(` argument, and for a `"-c"` that is a fixed program's own flag
//! (`ping -c`), only if none of the names is denied. There is no baseline: a finding is never
//! kept, only fixed.

use std::collections::HashSet;
use std::path::Path;

/// The crates the rule holds (a git pathspec: `*` crosses `/`, so nested files are in).
const SCOPES: [&str; 2] = ["crates/yantrik-companion-tools/src/*.rs", "crates/yantrik-companion/src/*.rs"];
/// The doors themselves: they are where the denied programs are started, on purpose.
const DOORS: [&str; 3] = ["spawn.rs", "browser_spawn.rs", "safe_command.rs"];
const DOOR_DIR: &str = "crates/yantrik-companion-tools/src";

/// The programs a Mind's choice must never reach except through a door.
pub const DENIED: [&str; 34] = [
    "sh", "bash", "dash", "env", "nohup", "setsid", "swaymsg", "xdg-open", "gio", "systemd-run", "pip", "pip3", "npm", "npx", "gem",
    "python", "python3", "node", "ruby", "perl", "chromium", "chromium-browser", "firefox", "claude",
    "busybox", "timeout", "nice", "xargs", "sudo", "doas", "nsenter", "unshare", "flock", "script",
];

const MARKER: &str = "fixed-program:";

/// A file of the crate: its path from the workspace root and its text.
pub struct Source {
    pub path: String,
    pub text: String,
}

/// The findings (`path:line: what`) for the whole tree, empty when the rule holds.
pub fn check(root: &Path) -> Result<Vec<String>, String> {
    let files = crate::workspace::tracked(root, &SCOPES)?;
    let sources: Vec<Source> = files
        .into_iter()
        .filter_map(|path| Some(Source { text: std::fs::read_to_string(root.join(&path)).ok()?, path }))
        .collect();
    Ok(scan(&sources))
}

/// The findings in `sources`.
pub fn scan(sources: &[Source]) -> Vec<String> {
    let test_files = test_only_files(sources);
    let mut out = Vec::new();
    for source in sources {
        let is_door = DOORS.iter().any(|d| source.path == format!("{DOOR_DIR}/{d}"));
        if is_door || test_files.contains(&source.path) {
            continue;
        }
        for (line_no, line) in live_lines(&source.text) {
            for why in judge(&line) {
                out.push(format!("{}:{}: {}", source.path, line_no, why));
            }
        }
    }
    out
}

/// What is wrong with one line of live code, if anything.
fn judge(raw: &str) -> Vec<String> {
    let (code, comment) = split_comment(raw);
    let mut out = Vec::new();
    let marker = comment.find(MARKER).map(|at| {
        comment[at + MARKER.len()..].trim().split('|').map(|n| n.trim().to_string()).collect::<Vec<String>>()
    });
    let fixed_flag = marker.as_ref().is_some_and(|names| names.iter().all(|n| !n.is_empty() && !is_denied(n)));
    if hands_dash_c(&code) && !fixed_flag {
        out.push("`\"-c\"` hands a string to a program that may run it as a command: use argv, work in-process, go through a door (spawn::mind_command), or name the fixed program with `// fixed-program: name`".to_string());
    }
    if code.contains("Command as ") {
        out.push("`Command as` renames the type, which hides every `Command::new(` from this rule: use it by name".to_string());
    }
    let mut rest = code.as_str();
    while let Some(at) = rest.find("Command::new(") {
        let arg = rest[at + "Command::new(".len()..].trim_start();
        rest = &rest[at + "Command::new(".len()..];
        match literal(arg) {
            Some(name) if is_denied(&name) => {
                out.push(format!("`Command::new(\"{name}\")` starts a program a Mind can steer: use spawn::mind_command or browser_spawn"));
            }
            Some(_) => {}
            None => match &marker {
                None => out.push(
                    "`Command::new(` with a program that is not a string literal: use spawn::mind_command, or name every program it can take with `// fixed-program: a|b`"
                        .to_string(),
                ),
                Some(names) => {
                    for denied in names.iter().filter(|n| n.is_empty() || is_denied(n)) {
                        out.push(format!("`// fixed-program:` names `{denied}`, which only a door may start"));
                    }
                }
            },
        }
    }
    out
}

/// Whether `code` passes a `"-c"` argument: `.arg("-c")`, or `"-c"` after `.args(`.
fn hands_dash_c(code: &str) -> bool {
    code.contains(".arg(\"-c\")") || code.find(".args(").is_some_and(|at| code[at..].contains("\"-c\""))
}

/// The string literal `arg` starts with, when the call is exactly `Command::new("lit")`.
fn literal(arg: &str) -> Option<String> {
    let body = arg.strip_prefix('"')?;
    let end = body.find('"')?;
    body[end + 1..].trim_start().starts_with(')').then(|| body[..end].to_string())
}

/// Whether `program` (or the last part of its path) is on the denylist.
fn is_denied(program: &str) -> bool {
    let base = program.rsplit('/').next().unwrap_or(program);
    DENIED.contains(&base)
}

/// `line` as its code and its `//` comment, not splitting inside a string literal.
fn split_comment(line: &str) -> (String, String) {
    let bytes: Vec<char> = line.chars().collect();
    let (mut i, mut in_str) = (0, false);
    while i < bytes.len() {
        match bytes[i] {
            '\\' if in_str => i += 1,
            '"' => in_str = !in_str,
            '/' if !in_str && bytes.get(i + 1) == Some(&'/') => {
                return (bytes[..i].iter().collect(), bytes[i..].iter().collect());
            }
            _ => {}
        }
        i += 1;
    }
    (line.to_string(), String::new())
}

/// `code` with every string and char literal emptied, for counting braces.
fn without_literals(code: &str) -> String {
    let chars: Vec<char> = code.chars().collect();
    let (mut out, mut i, mut in_str) = (String::new(), 0, false);
    while i < chars.len() {
        let c = chars[i];
        if in_str {
            match c {
                '\\' => i += 1,
                '"' => in_str = false,
                _ => {}
            }
        } else if c == '"' {
            in_str = true;
        } else if c == '\'' && chars.get(i + 2) == Some(&'\'') {
            i += 2;
        } else if c == '\'' && chars.get(i + 1) == Some(&'\\') && chars.get(i + 3) == Some(&'\'') {
            i += 3;
        } else {
            out.push(c);
        }
        i += 1;
    }
    out
}

/// Whether `line` opens an item that only a test build has: exactly `#[cfg(test)]` or
/// `#[cfg(all(test, ..))]`. `any(test, not(test))` always compiles, so it is live code.
fn is_cfg_test(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("#[cfg(test)]") || t.starts_with("#[cfg(all(test")
}

/// The lines (1-based) that are not inside a `#[cfg(test)]` item, whole comment lines left out.
fn live_lines(text: &str) -> Vec<(usize, String)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if is_cfg_test(lines[i]) {
            i = end_of_item(&lines, i);
            continue;
        }
        if !lines[i].trim_start().starts_with("//") {
            out.push((i + 1, lines[i].to_string()));
        }
        i += 1;
    }
    out
}

/// The index just past the item whose `#[cfg(test)]` attribute is at `start`: through its
/// closing brace, or its `;` for `mod x;`, `use ...;` and the like.
fn end_of_item(lines: &[&str], start: usize) -> usize {
    let mut depth = 0i32;
    let mut opened = false;
    for (at, line) in lines.iter().enumerate().skip(start) {
        // `#[cfg(test)] mod x;` on one line: the item is what follows the attribute.
        let line = if at == start { line.split_once(']').map_or("", |(_, rest)| rest) } else { line };
        let t = line.trim_start();
        if !opened && (t.starts_with("#[") || t.starts_with("//") || t.is_empty()) {
            continue;
        }
        for c in without_literals(&split_comment(line).0).chars() {
            match c {
                '{' => {
                    depth += 1;
                    opened = true;
                }
                '}' => depth -= 1,
                ';' if !opened && depth == 0 => return at + 1,
                _ => {}
            }
        }
        if opened && depth <= 0 {
            return at + 1;
        }
    }
    lines.len()
}

/// The files only a test build has: each `mod x;` under `#[cfg(test)]` (with its `#[path]`).
fn test_only_files(sources: &[Source]) -> HashSet<String> {
    let mut out = HashSet::new();
    for source in sources {
        let dir = source.path.rsplit_once('/').map_or("", |(d, _)| d);
        let lines: Vec<&str> = source.text.lines().collect();
        for (at, line) in lines.iter().enumerate() {
            if !is_cfg_test(line) {
                continue;
            }
            let mut path_attr: Option<String> = None;
            for next in lines.iter().skip(at + 1) {
                let t = next.trim();
                if let Some(rest) = t.strip_prefix("#[path") {
                    path_attr = rest.split('"').nth(1).map(str::to_string);
                } else if t.starts_with("#[") || t.starts_with("//") {
                } else {
                    let declared = t.trim_start_matches("pub ").trim_start_matches("pub(crate) ");
                    if let Some(name) = declared.strip_prefix("mod ").and_then(|r| r.strip_suffix(';')) {
                        let file = path_attr.unwrap_or_else(|| format!("{}.rs", name.trim()));
                        out.insert(format!("{dir}/{file}"));
                        out.insert(format!("{dir}/{}/mod.rs", name.trim()));
                    }
                    break;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src(path: &str, text: &str) -> Source {
        Source { path: format!("crates/yantrik-companion-tools/src/{path}"), text: text.to_string() }
    }

    fn found(path: &str, text: &str) -> Vec<String> {
        scan(&[src(path, text)])
    }

    #[test]
    fn a_fixed_utility_by_literal_is_fine() {
        assert!(found("a.rs", "fn f() { std::process::Command::new(\"ps\").arg(\"aux\").output(); }").is_empty());
        assert!(found("a.rs", "let c = Command::new(\"/usr/bin/bwrap\");").is_empty());
    }

    #[test]
    fn a_non_literal_program_is_found_unless_every_name_is_listed_and_none_is_denied() {
        assert_eq!(found("a.rs", "let c = Command::new(program);").len(), 1);
        assert_eq!(found("a.rs", "let c = Command::new(&binary);").len(), 1);
        assert!(found("a.rs", "let c = Command::new(cmd); // fixed-program: apk|apt-get|pacman").is_empty());
        let bad = found("a.rs", "let c = Command::new(cmd); // fixed-program: apk|sh");
        assert_eq!(bad.len(), 1, "{bad:?}");
        assert!(found("a.rs", "let c = Command::new(cmd); // fixed-program: ").len() == 1);
    }

    /// The launchers that start another program (round 11, Low 1) are denied as literals too.
    #[test]
    fn the_launchers_are_denied() {
        for program in ["busybox", "timeout", "nice", "xargs", "sudo", "doas", "nsenter", "unshare", "flock", "script"] {
            assert!(DENIED.contains(&program), "{program}");
            let line = format!("Command::new(\"{program}\").arg(\"x\");");
            assert_eq!(found("a.rs", &line).len(), 1, "{program}");
            assert_eq!(found("a.rs", &format!("Command::new(cmd); // fixed-program: ls|{program}")).len(), 1, "{program}");
        }
    }

    #[test]
    fn every_denied_literal_is_found_in_any_spelling() {
        for program in DENIED {
            let line = format!("let c = std::process::Command::new(\"{program}\");");
            assert_eq!(found("a.rs", &line).len(), 1, "{program}");
        }
        assert_eq!(found("a.rs", "Command::new(\"/bin/sh\")").len(), 1);
        assert_eq!(found("a.rs", "Command::new(\"/usr/bin/env\")").len(), 1);
        // A marker does not excuse a denied literal.
        assert_eq!(found("a.rs", "Command::new(\"claude\") // fixed-program: claude").len(), 1);
    }

    #[test]
    fn a_shell_c_argument_is_found() {
        assert_eq!(found("a.rs", "cmd.arg(\"-c\").arg(line);").len(), 1);
        assert_eq!(found("a.rs", "cmd.args([\"-c\", line]);").len(), 1);
        assert_eq!(found("a.rs", "Command::new(\"ping\").args([\"-W\", \"5\", \"-c\", n]);").len(), 1);
        assert_eq!(found("a.rs", "cmd.args(&[\"-c\", line]);").len(), 1);
        // A fixed program's own flag, named: accepted. Naming a denied program is not.
        assert!(found("a.rs", "cmd.args([\"-c\", n]); // fixed-program: ping").is_empty());
        assert_eq!(found("a.rs", "cmd.args([\"-c\", n]); // fixed-program: sh").len(), 1);
        assert_eq!(found("a.rs", "cmd.args([\"-c\", n]); // ping count").len(), 1);
        assert!(found("a.rs", "cmd.args([\"--color\", c]);").is_empty());
    }

    #[test]
    fn renaming_command_on_import_is_found() {
        assert_eq!(found("a.rs", "use std::process::Command as C;").len(), 1);
        assert_eq!(found("a.rs", "use std::process::{Command as C, Stdio};").len(), 1);
        assert!(found("a.rs", "use std::process::{Command, Stdio};").is_empty());
    }

    #[test]
    fn only_an_exact_cfg_test_or_cfg_all_test_is_test_code() {
        let held = |attr: &str| found("a.rs", &format!("{attr}
fn f() {{ Command::new(\"sh\"); }}
"));
        assert!(held("#[cfg(test)]").is_empty());
        assert!(held("#[cfg(all(test, unix))]").is_empty());
        assert_eq!(held("#[cfg(any(test, not(test)))]").len(), 1);
        assert_eq!(held("#[cfg(any(test, unix))]").len(), 1);
        assert_eq!(held("#[cfg(not(test))]").len(), 1);
    }

    #[test]
    fn both_crates_and_nested_files_are_in_scope() {
        assert!(SCOPES.iter().any(|s| s.contains("yantrik-companion/src")));
        assert!(SCOPES.iter().any(|s| s.contains("yantrik-companion-tools/src")));
        let nested = Source { path: "crates/yantrik-companion/src/connectors/x.rs".into(), text: "Command::new(\"sh\");".into() };
        assert_eq!(scan(&[nested]).len(), 1);
        // The door exemption is the tools crate's files only: a companion file named spawn.rs is held.
        let lookalike = Source { path: "crates/yantrik-companion/src/spawn.rs".into(), text: "Command::new(\"sh\");".into() };
        assert_eq!(scan(&[lookalike]).len(), 1);
    }

    #[test]
    fn test_code_comments_and_the_doors_are_not_held() {
        let text = "fn live() {}\n// Command::new(x) in prose\n#[cfg(test)]\nmod tests {\n    fn t() { let s = \"}\"; Command::new(\"sh\"); }\n}\nfn after() { Command::new(arg); }\n";
        let found = found("a.rs", text);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].ends_with(":7: `Command::new(` with a program that is not a string literal: use spawn::mind_command, or name every program it can take with `// fixed-program: a|b`"), "{found:?}");
        for door in DOORS {
            assert!(scan(&[src(door, "Command::new(\"sh\").arg(\"-c\");")]).is_empty(), "{door}");
        }
    }

    #[test]
    fn a_file_declared_under_cfg_test_is_test_code() {
        let lib = src("lib.rs", "#[cfg(test)]\n#[path = \"x_tests.rs\"]\nmod t;\n#[cfg(test)]\nmod capture_tests;\n");
        let held = [
            lib,
            src("x_tests.rs", "Command::new(\"sh\");"),
            src("capture_tests.rs", "Command::new(\"sh\");"),
            src("other.rs", "Command::new(\"sh\");"),
        ];
        let found = scan(&held);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("other.rs"));
    }

    /// The tree itself: the rule holds, with no baseline to hide behind.
    #[test]
    fn the_companion_crates_hold_the_rule() {
        let found = check(&crate::workspace::root()).expect("git ls-files");
        assert!(found.is_empty(), "{}", found.join("\n"));
    }
}
