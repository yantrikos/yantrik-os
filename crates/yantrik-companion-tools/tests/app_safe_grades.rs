//! Nothing the built-in companion may run unasked is added without somebody reading it (#1313, M4).
//!
//! `app_action` lets the companion run what an app grades `safe` (`app_guard::admit`), so a `safe`
//! grade is a promise that the handler changes nothing the person keeps, starts nothing and sends
//! nothing. Nothing at run time checks the promise. This test lists every `safe`-graded declaration in
//! the repository, `.risk("safe")` in Rust, `grade="safe"` in Python, the Blender add-on's positional
//! grade, a `permission: "safe"` field, against the checked-in `safe_grades.allow`, so a new `safe`
//! grade fails CI until a person has read the handler and added its line (and says why in the commit).
//!
//! Name the safe, not the dangerous: anything this scan cannot classify fails too. A declaration it
//! cannot read (an `Action { .. }` struct literal, a grade that is not a string literal, a Python
//! grade that is a variable) fails until it is either written the way the scan reads or named in
//! [`UNREADABLE`] with the reason it is fine. Test code is not scanned (`#[cfg(test)]` items, a file
//! that a `#[cfg(test)] mod` declares, a `tests` folder); every other `.rs` and `.py` file under the
//! roots is, whatever it is called. A grade written as a constant of the same file is read through it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

// The one parser the reach scans share (`yantrik-companion/tests/common/syntax.rs`), not a copy of it.
#[path = "../../yantrik-companion/tests/common/syntax.rs"]
mod syntax;
use syntax::without_test_items;

/// The checked-in list: `file :: action`, one line per `safe` grade, `#` for comments.
const ALLOW: &str = include_str!("safe_grades.allow");

/// Where declarations live. `sdk/` is the library the declarations are written with, not a declaration.
const ROOTS: &[&str] = &["apps", "adapters", "frameworks", "services", "examples", "templates", "crates", "harnesses"];

/// Declarations the scan cannot read, each the one line that needs it, and why that is fine.
///
/// The four questions of the shell take their grade from the `grade:` fields of `QUESTIONS` in the same
/// file (`sensitive`, `sensitive`, `sensitive`, `standard`), and a `grade: "safe"` field is read as a
/// safe grade by this scan, so a question graded `safe` would fail here.
const UNREADABLE: &[(&str, &str)] = &[(
    "crates/yantrik-ui/src/control_questions.rs",
    "Action::new(question.name, question.sentence).risk(question.grade),",
)];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn relative(path: &Path) -> String {
    let full = path.to_string_lossy().replace('\\', "/");
    let base = root().to_string_lossy().replace('\\', "/");
    let base = base.trim_end_matches('/');
    full.strip_prefix(base).unwrap_or(&full).trim_start_matches('/').to_string()
}

fn read(file: &Path) -> String {
    std::fs::read_to_string(file).unwrap_or_else(|e| panic!("{} cannot be read: {e}", file.display()))
}

/// What one file declares as `safe`, and what in it could not be read.
#[derive(Default, Debug)]
struct Found {
    /// `file :: action`.
    safe: Vec<String>,
    /// `file: the line`.
    unreadable: Vec<String>,
}

impl Found {
    fn add(&mut self, other: Found) {
        self.safe.extend(other.safe);
        self.unreadable.extend(other.unreadable);
    }
}

fn line_of(src: &str, at: usize) -> String {
    let start = src[..at].rfind('\n').map_or(0, |n| n + 1);
    let end = src[at..].find('\n').map_or(src.len(), |n| at + n);
    src[start..end].trim().to_string()
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The name an `Action::new(` (or, in an app's `intents`, the `act(` helper) before `at` gives its
/// action: the literal, or the expression it is written as (`ACTION`, `question.name`).
fn owner_before(code: &str, at: usize, helper: bool) -> String {
    let mut best: Option<(usize, usize)> = code[..at].rfind("Action::new(").map(|i| (i, "Action::new(".len()));
    if helper {
        let mut end = at;
        while let Some(i) = code[..end].rfind("act(") {
            let before = code[..i].chars().next_back().unwrap_or(' ');
            if !(is_ident(before) || before == '.' || code[..i].ends_with("fn ")) {
                if best.map_or(true, |(b, _)| i > b) {
                    best = Some((i, "act(".len()));
                }
                break;
            }
            end = i;
        }
    }
    let Some((i, len)) = best else { return "?".into() };
    let rest = code[i + len..].trim_start();
    match rest.strip_prefix('"') {
        Some(quoted) => quoted.split('"').next().unwrap_or("?").to_string(),
        None => rest.chars().take_while(|c| !matches!(c, ',' | ')' | '\n')).collect::<String>().trim().to_string(),
    }
}

/// `const NAME: &str = "value"` of this source, by name.
fn constants(src: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (at, _) in src.match_indices("const ") {
        let Some((name, after)) = src[at + 6..].split_once(':') else { continue };
        let Some((ty, value)) = after.split_once('=') else { continue };
        let name = name.trim();
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') || !ty.contains("str") || ty.len() > 20 {
            continue;
        }
        if let Some(text) = value.trim_start().strip_prefix('"').and_then(|v| v.split('"').next()) {
            out.insert(name.to_string(), text.to_string());
        }
    }
    out
}

/// Rust: every `.risk("safe")`, every `permission: "safe"`, and what cannot be read.
fn scan_rust(rel: &str, code: &str) -> Found {
    let mut out = Found::default();
    let helper = rel.contains("/intents");
    let consts = constants(code);
    for (at, _) in code.match_indices(".risk(") {
        let rest = code[at + 6..].trim_start();
        if rest.starts_with("\\\"") {
            continue; // the text of a call inside a string, not a call
        }
        let grade = match rest.strip_prefix('"') {
            Some(literal) => literal.split('"').next().map(str::to_string),
            None => consts.get(rest.split(')').next().unwrap_or("").trim()).cloned(),
        };
        match grade.as_deref() {
            Some("safe") => out.safe.push(format!("{rel} :: {}", owner_before(code, at, helper))),
            Some(_) => {}
            None => out.unreadable.push(format!("{rel}: {}", line_of(code, at))),
        }
    }
    for needle in ["permission: \"safe\"", "permission = \"safe\"", "grade: \"safe\""] {
        for (at, _) in code.match_indices(needle) {
            out.safe.push(format!("{rel} :: <permission literal at `{}`>", line_of(code, at)));
        }
    }
    // A struct literal of the declaration type is a declaration whose grade this scan does not read.
    for (at, _) in code.match_indices("Action {") {
        let before = &code[..at];
        let prior = before.chars().next_back().unwrap_or(' ');
        if is_ident(prior) {
            continue; // `PendingAction {`, `SomeAction {`
        }
        let head = before.trim_end();
        if ["->", "struct", "impl", "for", "trait", "enum", "mod", "&mut", "&", ":"].iter().any(|k| head.ends_with(k)) {
            continue; // a return type, an item, a type in a signature
        }
        out.unreadable.push(format!("{rel}: {}", line_of(code, at)));
    }
    out
}

/// Python: `grade="safe"` anywhere, the Blender add-on's `_action("name", "purpose", "safe", ..)`
/// rows, and an `.action(` decorator whose grade is not a string literal.
fn scan_python(rel: &str, src: &str) -> Found {
    let mut out = Found::default();
    for (at, _) in src.match_indices("grade") {
        let rest = src[at + 5..].trim_start();
        let Some(rest) = rest.strip_prefix('=') else { continue };
        if src[..at].chars().next_back().is_some_and(is_ident) {
            continue; // `upgrade=`, `my_grade=`
        }
        let rest = rest.trim_start();
        let line = line_of(src, at);
        if let Some(quoted) = rest.strip_prefix('"').or_else(|| rest.strip_prefix('\'')) {
            if quoted.starts_with("safe\"") || quoted.starts_with("safe'") {
                out.safe.push(format!("{rel} :: {}", python_name(src, at)));
            }
        } else if line.contains(".action(") {
            out.unreadable.push(format!("{rel}: {line}"));
        }
    }
    let starts: Vec<usize> = src.match_indices("_action(\"").filter(|(at, _)| !src[..*at].chars().next_back().is_some_and(is_ident)).map(|(at, _)| at).collect();
    for (i, at) in starts.iter().enumerate() {
        let span = &src[*at..starts.get(i + 1).copied().unwrap_or(src.len())];
        let name = span["_action(\"".len()..].split('"').next().unwrap_or("?").to_string();
        let first = ["safe", "standard", "sensitive", "dangerous"].iter().filter_map(|g| span.find(&format!("\"{g}\"")).map(|p| (p, *g))).min();
        match first {
            Some((_, "safe")) => out.safe.push(format!("{rel} :: {name}")),
            Some(_) => {}
            None => out.unreadable.push(format!("{rel}: _action(\"{name}\" has no grade this scan reads")),
        }
    }
    out
}

/// The action a Python grade belongs to: the decorator's first argument when it is a string, else the
/// name of the function it decorates.
fn python_name(src: &str, at: usize) -> String {
    let line_start = src[..at].rfind('\n').map_or(0, |n| n + 1);
    let head = &src[line_start..at];
    if !head.trim_start().starts_with('@') {
        return format!("<grade at `{}`>", line_of(src, at));
    }
    if let Some(after) = head.split(".action(").nth(1) {
        if let Some(quoted) = after.trim_start().strip_prefix('"') {
            return quoted.split('"').next().unwrap_or("?").to_string();
        }
    }
    src[at..].split("def ").nth(1).and_then(|f| f.split('(').next()).map_or("?".into(), |f| f.trim().to_string())
}

/// The attribute lines directly above the line `at` is on, trimmed: the ones that belong to the item.
fn attributes_before(text: &str, at: usize) -> Vec<String> {
    let line_start = text[..at].rfind('\n').map_or(0, |n| n + 1);
    let mut out = Vec::new();
    for line in text[..line_start].lines().rev() {
        let line = line.trim();
        if line.starts_with("#[") {
            out.push(line.to_string());
        } else if !line.starts_with("//") {
            break;
        }
    }
    // The attribute may be on the same line as the item, `#[cfg(test)] mod x;`.
    out.extend(text[line_start..at].split("#[").skip(1).map(|a| format!("#[{}", a.trim())));
    out
}

/// Whether a sibling or the parent module declares `file` as `#[cfg(test)] mod <stem>;`.
fn is_a_test_module(file: &Path) -> bool {
    let (Some(stem), Some(dir)) = (file.file_stem().and_then(|s| s.to_str()), file.parent()) else { return false };
    let mut homes: Vec<PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).collect();
    if let (Some(up), Some(name)) = (dir.parent(), dir.file_name().and_then(|n| n.to_str())) {
        homes.push(up.join(format!("{name}.rs")));
        homes.push(up.join("lib.rs"));
    }
    let wanted = [format!("mod {stem};"), format!("{stem}.rs\"")];
    homes.iter().filter(|p| p.extension().is_some_and(|e| e == "rs") && p.as_path() != file).any(|p| {
        let Ok(text) = std::fs::read_to_string(p) else { return false };
        wanted.iter().any(|w| text.match_indices(w.as_str()).any(|(at, _)| attributes_before(&text, at).iter().any(|a| a == "#[cfg(test)]")))
    })
}

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
        if path.is_dir() {
            if !matches!(name.as_str(), "tests" | "target" | "__pycache__" | "node_modules" | ".git" | ".venv") {
                files(&path, out);
            }
        } else if name.ends_with(".rs") || (name.ends_with(".py") && !name.starts_with("test_") && name != "conftest.py") {
            out.push(path);
        }
    }
}

fn scan_all() -> Found {
    let mut paths = Vec::new();
    for dir in ROOTS {
        files(&root().join(dir), &mut paths);
    }
    paths.sort();
    let mut all = Found::default();
    for path in &paths {
        let rel = relative(path);
        if rel.ends_with(".py") {
            all.add(scan_python(&rel, &read(path)));
            continue;
        }
        if is_a_test_module(path) {
            continue;
        }
        let text = read(path);
        let code = without_test_items(&text).unwrap_or_else(|e| panic!("{rel} does not parse: {e}"));
        let code: String = code.lines().map(|l| if l.trim_start().starts_with("//") { "" } else { l }).collect::<Vec<_>>().join("\n");
        all.add(scan_rust(&rel, &code));
    }
    all
}

fn counted(lines: impl IntoIterator<Item = String>) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for line in lines {
        *out.entry(line).or_insert(0) += 1;
    }
    out
}

fn allowed() -> BTreeMap<String, usize> {
    counted(ALLOW.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(str::to_string))
}

/// Every `safe` grade in the repository is on the allow file, and every line of it is still there.
#[test]
fn every_safe_grade_is_on_the_allow_file_and_nothing_else_is() {
    let found = counted(scan_all().safe);
    let allow = allowed();
    let new: Vec<String> = found.iter().filter(|(k, n)| allow.get(*k) != Some(*n)).map(|(k, n)| format!("{k}  (x{n}, listed x{})", allow.get(k).unwrap_or(&0))).collect();
    let gone: Vec<&String> = allow.keys().filter(|k| !found.contains_key(*k)).collect();
    assert!(
        new.is_empty(),
        "a `safe` grade nobody has listed: read what its handler does (`safe` lets the built-in companion run it through `app_action` unasked: it must change nothing the person keeps, start nothing and send nothing), then add its line to tests/safe_grades.allow, or grade it `standard`:\n{}",
        new.join("\n")
    );
    assert!(gone.is_empty(), "tests/safe_grades.allow lists a `safe` grade that is not declared any more (remove the line): {gone:#?}");
}

/// A declaration the scan cannot read is one nobody has looked at: written the way the scan reads it,
/// or named above with the reason it is fine.
#[test]
fn a_declaration_the_scan_cannot_read_fails_until_it_is_named() {
    let found = scan_all();
    let stray: Vec<&String> = found.unreadable.iter().filter(|u| !UNREADABLE.iter().any(|(f, l)| **u == format!("{f}: {l}"))).collect();
    assert!(stray.is_empty(), "declared in a way this scan cannot read (write the grade as a string literal, or list the line in UNREADABLE with why):\n{stray:#?}");
    for (f, l) in UNREADABLE {
        assert!(found.unreadable.contains(&format!("{f}: {l}")), "UNREADABLE names a line that is not there any more: {f}: {l}");
    }
}

/// The scan reads what it is meant to, so an empty result cannot pass for a clean one.
#[test]
fn the_scan_finds_the_safe_grades_of_every_kind_of_source() {
    let found = counted(scan_all().safe);
    let has = |needle: &str| found.keys().any(|k| k.contains(needle));
    for needle in [
        "frameworks/yantrik-app/src/present.rs :: ",
        "crates/yantrik-ui/src/control_approvals.rs :: ",
        "apps/browser/yantrik_browser/service.py :: tabs",
        "adapters/libreoffice/yantrik_libreoffice/surface.py :: read_text",
        "apps/snippet-manager/src/intents.rs :: ",
    ] {
        assert!(has(needle), "the scan found no safe grade for {needle}");
    }
    assert!(found.len() >= 60, "the scan found {} safe grades", found.len());
    assert!(!has("launch"), "the framework's `launch` is graded standard (it opens files as the person)");
}

/// The probes of the round-5 review, as sources: a safe grade in a Python module under any `src/`
/// and a struct literal that carries a grade are both caught.
#[test]
fn the_probes_that_passed_the_old_test_are_caught() {
    let py = "@surface.action(\"wipe_everything\", grade=\"safe\")\ndef wipe(): pass\n\n@surface.action(grade='safe')\ndef look(): pass\n@s.action(grade=GRADE)\ndef x(): pass\n";
    let found = scan_python("apps/notes/src/zz_probe/py/surface.py", py);
    assert_eq!(found.safe, ["apps/notes/src/zz_probe/py/surface.py :: wipe_everything", "apps/notes/src/zz_probe/py/surface.py :: look"]);
    assert_eq!(found.unreadable.len(), 1, "{:?}", found.unreadable);
    let rs = "fn x() -> Vec<Action> { vec![Action { name: \"wipe_all\".into(), permission: \"safe\".into(), ..Default::default() }] }";
    let found = scan_rust("apps/notes/src/zz_probe.rs", rs);
    assert_eq!(found.unreadable.len(), 1, "{:?}", found.unreadable);
    assert!(found.safe.iter().any(|s| s.contains("permission literal")), "{:?}", found.safe);
    // The declarations of the type itself are not literals.
    let items = "pub struct Action { a: u8 }\nimpl Action { fn f(&self) {} }\nfn g() -> Action { todo!() }\nimpl Tool for Action {}\nfn h(a: PendingAction) {}\nlet x = PendingAction { a: 1 };";
    assert!(scan_rust("x.rs", items).unreadable.is_empty());
    // A grade that is a variable, and one in a helper, are read or fail.
    let var = "Action::new(\"a\", \"b\").risk(GRADE)";
    assert_eq!(scan_rust("x.rs", var).unreadable.len(), 1);
    let helper = "act(\"look\", \"b\").risk(\"safe\")\nact(\"change\", \"b\").risk(\"standard\")";
    assert_eq!(scan_rust("apps/x/src/intents.rs", helper).safe, ["apps/x/src/intents.rs :: look"]);
    let blender = "\n    _action(\"look\", \"Look\", \"safe\", [])\n    _action(\"do\", \"Do\", \"standard\", [\"safe\"])\n";
    assert_eq!(scan_python("apps/blender/addon/surface.py", blender).safe, ["apps/blender/addon/surface.py :: look"]);
}

/// A file is a test module only when `#[cfg(test)]` is an attribute of its own `mod` line, not of the
/// declaration next to it.
#[test]
fn a_neighbours_cfg_test_does_not_hide_a_module() {
    let text = "#[cfg(test)]\nmod a;\nmod b;\n#[cfg(test)]\n#[path = \"c_tests.rs\"]\nmod tests;\n#[cfg(test)] mod d;\nmod e;\n";
    let on = |needle: &str| attributes_before(text, text.find(needle).unwrap()).iter().any(|a| a == "#[cfg(test)]");
    assert!(on("mod a;") && !on("mod b;") && on("c_tests.rs\"") && on("mod tests;") && on("mod d;") && !on("mod e;"));
}
