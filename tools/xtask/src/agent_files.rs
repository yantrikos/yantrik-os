//! `check-agent-files`: every action an agent can reach that names a file is held to the rule for
//! agents' files (#443), or says why it needs none.
//!
//! An app runs as the person, so a file an agent names to it is read or written with the person's
//! rights: Document Editor's `open` read the snippets store and `describe` handed it back (security
//! review of #942, finding 1). The rule is `yantrik_ipc_contracts::home_paths` (Python surfaces:
//! `yantrik_surface.files`, which asks the shell for it). This finds every published action with
//! a parameter named like a path, in every app, adapter, service and the shell, and holds each to
//! a line of `tools/xtask/agent-files.txt` naming the file that applies the rule, or the reason
//! none is needed. A new action with a path, a line for an action that is gone, and a guard file
//! that no longer applies the rule each fail.
//!
//! It reads source text, as the size rule does: a declaration it cannot see (built at run time,
//! or a parameter not named like a path) is not held, which is why the names are broad.

use std::collections::BTreeSet;
use std::path::Path;

pub const REGISTRY: &str = "tools/xtask/agent-files.txt";

/// Where actions an agent can reach are declared.
const SCANNED: &[&str] = &[
    "apps/*.rs",
    "apps/*.py",
    "adapters/*.py",
    "services/*.rs",
    "crates/yantrik-ui/src/*.rs",
    "frameworks/yantrik-app/src/*.rs",
];

/// Parameter names that carry a path somewhere on this desktop.
const PATH_PARAMS: &[&str] = &[
    "path", "paths", "file", "files", "output", "destination", "dest", "directory", "dir", "folder",
    "save_dir", "workflow", "cwd", "working_dir", "working_directory", "of", "record",
];

/// Where an action's name is declared, in each style the tree uses.
const ACTION_MARKS: &[&str] = &["Action::new(", "act(", "_action(", ".action("];
/// Where a parameter is declared.
const PARAM_MARKS: &[&str] = &["Param::text(", "Param::path(", "Param(", "arg("];
/// What a guard file has to say to be applying the rule.
const GUARD_MARKS: &[&str] = &["home_paths::", "files.may_", "may_read_file", "may_write_file", "validate_path"];

/// One action's path parameter, as found in a source file.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Found {
    pub file: String,
    pub action: String,
    pub param: String,
}

/// One line of the registry: the guard file, or `None` with the reason none is needed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub found: Found,
    pub guard: Option<String>,
}

fn is_test(file: &str) -> bool {
    file.contains("/tests/")
        || file.contains("/test_")
        || file.ends_with("tests.rs")
        || file.ends_with("_test.py")
        || file.contains("/fake_")
}

/// The quoted name right after `at` (whitespace and newlines allowed before it).
fn quoted(text: &str, at: usize) -> Option<&str> {
    let rest = text[at..].trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    let name = &rest[..end];
    (!name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'))
        .then_some(name)
}

/// Every place one of `marks` starts a call whose first argument is a quoted name, as
/// `(offset, name)`. A mark preceded by an identifier character is another word (`react(`).
fn calls<'t>(text: &'t str, marks: &[&str]) -> Vec<(usize, &'t str)> {
    let mut out = Vec::new();
    for mark in marks {
        let mut from = 0;
        while let Some(i) = text[from..].find(mark) {
            let at = from + i;
            from = at + mark.len();
            let joined = text[..at].chars().next_back().is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
            if joined && !mark.starts_with(['.', '_', 'P', 'A']) {
                continue;
            }
            if let Some(name) = quoted(text, from) {
                out.push((at, name));
            }
        }
    }
    out.sort();
    out
}

/// Python's annotated parameters: `path: Annotated[...]`, the LibreOffice adapter's style.
fn annotated(text: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = text[from..].find(": Annotated[") {
        let at = from + i;
        from = at + 1;
        let start = text[..at].rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).map_or(0, |s| s + 1);
        out.push((start, &text[start..at]));
    }
    out
}

/// Every action's path parameter in `text`, read as the file `file`.
pub fn scan(file: &str, text: &str) -> Vec<Found> {
    let actions = calls(text, ACTION_MARKS);
    let mut params = calls(text, PARAM_MARKS);
    if file.ends_with(".py") {
        params.extend(annotated(text));
    }
    let mut found = BTreeSet::new();
    for (at, param) in params {
        if !PATH_PARAMS.contains(&param) {
            continue;
        }
        if let Some((_, action)) = actions.iter().rev().find(|(a, _)| *a < at) {
            found.insert(Found { file: file.into(), action: (*action).into(), param: param.into() });
        }
    }
    found.into_iter().collect()
}

/// The registry: `<file> <action> <param> <guard file>`, or `... - <reason>`; `#` comments.
pub fn parse(text: &str) -> Result<Vec<Line>, String> {
    let mut lines = Vec::new();
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.splitn(5, char::is_whitespace).collect();
        let [file, action, param, guard, rest @ ..] = parts.as_slice() else {
            return Err(format!("{REGISTRY}:{}: want `<file> <action> <param> <guard file | - reason>`", n + 1));
        };
        let found = Found { file: (*file).into(), action: (*action).into(), param: (*param).into() };
        let guard = match *guard {
            "-" if rest.first().is_some_and(|r| !r.trim().is_empty()) => None,
            "-" => return Err(format!("{REGISTRY}:{}: `-` needs the reason no rule is needed", n + 1)),
            g => Some(g.to_string()),
        };
        lines.push(Line { found, guard });
    }
    Ok(lines)
}

/// What is wrong, one sentence each; empty when every action is held.
pub fn judge(found: &[Found], lines: &[Line], read: impl Fn(&str) -> Option<String>) -> Vec<String> {
    let mut problems = Vec::new();
    for f in found {
        if !lines.iter().any(|l| &l.found == f) {
            problems.push(format!(
                "{} `{}` takes `{}`, a path, and {REGISTRY} does not say what holds it to the rule for agents' files",
                f.file, f.action, f.param
            ));
        }
    }
    for l in lines {
        let f = &l.found;
        if !found.contains(f) {
            problems.push(format!("{REGISTRY} names {} `{}` `{}`, which is not there any more", f.file, f.action, f.param));
            continue;
        }
        if let Some(guard) = &l.guard {
            match read(guard) {
                None => problems.push(format!("{} `{}`: its guard {guard} is not a file", f.file, f.action)),
                Some(text) if !GUARD_MARKS.iter().any(|m| text.contains(m)) => problems.push(format!(
                    "{} `{}`: its guard {guard} does not apply the rule (none of {})",
                    f.file,
                    f.action,
                    GUARD_MARKS.join(", ")
                )),
                Some(_) => {}
            }
        }
    }
    problems
}

/// Every `.rs` and `.py` file under the folders [`SCANNED`] names, `/`-separated, skipping build
/// output and hidden folders.
fn walked(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut folders: Vec<String> = SCANNED
        .iter()
        .map(|p| p.trim_end_matches("*.rs").trim_end_matches("*.py").trim_end_matches('/').to_string())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    while let Some(folder) = folders.pop() {
        let Ok(entries) = std::fs::read_dir(root.join(&folder)) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let rel = format!("{folder}/{name}");
            match entry.file_type() {
                Ok(t) if t.is_dir() && !name.starts_with('.') && name != "target" && name != "node_modules" => folders.push(rel),
                Ok(t) if t.is_file() && (name.ends_with(".rs") || name.ends_with(".py")) => out.push(rel),
                _ => {}
            }
        }
    }
    out.sort();
    out
}

/// The whole check over the tree at `root`.
pub fn check(root: &Path) -> Result<Vec<String>, String> {
    let read = |rel: &str| std::fs::read_to_string(root.join(rel)).ok();
    let mut found = Vec::new();
    // The files git tracks; in a copy of the tree with no git (a build host's mirror), every file
    // under the same folders.
    let files = crate::workspace::tracked(root, SCANNED).or_else(|_| Ok::<_, String>(walked(root)))?;
    for file in files {
        if is_test(&file) {
            continue;
        }
        if let Some(text) = read(&file) {
            found.extend(scan(&file, &text));
        }
    }
    let lines = parse(&read(REGISTRY).ok_or_else(|| format!("{REGISTRY} is missing"))?)?;
    Ok(judge(&found, &lines, read))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_an_agent_can_reach_that_names_a_file_is_held() {
        let problems = check(&crate::workspace::root()).expect("the check runs");
        assert!(problems.is_empty(), "\n{}", problems.join("\n"));
    }

    #[test]
    fn declarations_are_found_in_each_style() {
        let rust = r#"
            Action::new("open", "Open").arg(Param::text("path"))
            act(
                "import",
                "Read it",
            )
            .arg(arg(
                "path",
                "Where",
            ))
            Action::new("find", "Find").arg(Param::text("query"))
        "#;
        let names = |found: Vec<Found>| found.into_iter().map(|f| format!("{}.{}", f.action, f.param)).collect::<Vec<_>>();
        assert_eq!(names(scan("a.rs", rust)), ["import.path", "open.path"]);
        let python = r#"
    _action("save", "Save it", "sensitive", [Param("path", description="where")])
    @surface.action("export_pdf", expected_seconds=20)
    def export_pdf(path: Annotated[str, "the PDF"], document: Annotated[str, "which"] = None):
        """react("path") is prose."""
"#;
        assert_eq!(names(scan("a.py", python)), ["export_pdf.path", "save.path"]);
    }

    #[test]
    fn an_unheld_action_a_stale_line_and_a_guard_that_applies_nothing_fail() {
        let found = vec![
            Found { file: "a.rs".into(), action: "open".into(), param: "path".into() },
            Found { file: "a.rs".into(), action: "save".into(), param: "path".into() },
        ];
        let lines = parse(
            "a.rs open path rule.rs\n# a comment\na.rs gone path rule.rs\na.rs save path plain.rs\n",
        )
        .unwrap();
        let read = |p: &str| match p {
            "rule.rs" => Some("home_paths::may_read_file(...)".to_string()),
            "plain.rs" => Some("fn save() {}".to_string()),
            _ => None,
        };
        let problems = judge(&found, &lines, read);
        assert_eq!(problems.len(), 2, "{problems:#?}");
        assert!(problems[0].contains("`gone`") && problems[0].contains("not there any more"));
        assert!(problems[1].contains("does not apply the rule"));
        let unlisted = judge(&found, &parse("a.rs open path rule.rs\n").unwrap(), read);
        assert!(unlisted[0].contains("`save` takes `path`"), "{unlisted:?}");
        assert!(parse("a.rs open path -\n").is_err(), "an exemption says why");
        assert!(parse("a.rs open path - the shell's own folder\n").unwrap()[0].guard.is_none());
    }
}
