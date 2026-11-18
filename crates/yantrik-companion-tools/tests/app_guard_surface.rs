//! Every action every app and the shell publishes, graded above `safe`, is either refused on the
//! `app_action` road or reviewed as not a guarded effect (#1313, round 5, H1).
//!
//! `app_action` reaches every action of every app, so a new one that deletes a file, changes a mail
//! or an event, sends a message, plans work or forgets a memory would be a second road to an effect
//! the person must press Allow on a card for. This reads the source of every `apps/*` app (Rust
//! intents, the Blender add-on's and the browser service's Python) and of the shell's
//! `control*.rs`, grades each action as its declaration does (`.risk("..")`, else `standard`, the
//! grade `Action::new` gives), and fails when an action above `safe` is on neither list of
//! `app_guard`, when a listed pair no longer exists, when a name is on both lists, and when one of
//! the four questions' names is declared by anything but the shell's `control_questions.rs`.
//! A declaration this scan cannot name fails here until it is looked at.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

// The one parser the reach scans share (`yantrik-companion/tests/common/syntax.rs`), not a copy of it.
#[path = "../../yantrik-companion/tests/common/syntax.rs"]
mod syntax;
use syntax::without_test_items;

use yantrik_companion_tools::app_guard::{REFUSED, REVIEWED};
use yantrik_companion_tools::asks::Question;

/// `(app, action)` to its grade.
type Found = BTreeMap<(String, String), String>;

/// Constants a shell declaration names that live in another crate (`yantrik-ipc-contracts`).
const KNOWN_CONSTS: &[(&str, &str)] =
    &[("SNAPSHOT_ACTION", "mind_desk"), ("STOP_ACTION", "mind_desk_stop"), ("AGENTS_ACTION", "mind_desk_open_agents")];

/// Declarations whose name is not a literal or a constant this scan can read, and why that is fine:
/// the macro's own body (its invocations name the actions), and the question helper (the four
/// questions are the one place their names are declared).
const UNREADABLE: &[&str] = &["$name", "question.name"];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
        if path.is_dir() {
            if name != "tests" && name != "target" {
                rust_files(&path, out);
            }
        } else if name.ends_with(".rs") && !name.ends_with("tests.rs") && !name.ends_with("_test.rs") {
            out.push(path);
        }
    }
}

/// The file's code: without its test items and without comment lines.
fn code_of(file: &Path) -> String {
    let whole = std::fs::read_to_string(file).unwrap_or_default();
    without_test_items(&whole)
        .unwrap_or_else(|e| panic!("{} does not parse: {e}", file.display()))
        .lines()
        .map(|l| if l.trim_start().starts_with("//") { "" } else { l })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `const NAME: &str = "value"` of this source, by name.
fn constants(src: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (at, _) in src.match_indices("const ") {
        let rest = &src[at + 6..];
        let Some((name, after)) = rest.split_once(':') else { continue };
        let name = name.trim();
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') {
            continue;
        }
        let Some((ty, value)) = after.split_once('=') else { continue };
        if !ty.contains("str") || ty.len() > 20 {
            continue;
        }
        if let Some(text) = value.trim_start().strip_prefix('"').and_then(|v| v.split('"').next()) {
            out.insert(name.to_string(), text.to_string());
        }
    }
    out
}

/// What one Rust file declares: `(position, name)` for `Action::new(` and, in an app, the `act(`
/// helper the apps wrap it in; and the name's identifier when it is not a literal.
fn declared(src: &str, helper: bool, consts: &BTreeMap<String, String>, unreadable: &mut Vec<String>) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    let openers: &[&str] = if helper { &["Action::new(", "act("] } else { &["Action::new("] };
    for opener in openers {
        for (at, _) in src.match_indices(opener) {
            let before = src[..at].chars().next_back().unwrap_or(' ');
            if *opener == "act(" && (before.is_alphanumeric() || before == '_' || before == '.') {
                continue;
            }
            let rest = src[at + opener.len()..].trim_start();
            if let Some(name) = rest.strip_prefix('"').and_then(|r| r.split('"').next()) {
                found.push((at, name.to_string()));
                continue;
            }
            if *opener == "act(" {
                // The helper's own definition and calls that pass a variable name no action.
                continue;
            }
            let ident: String = rest.chars().take_while(|c| !matches!(c, ',' | ')' | '\n')).collect::<String>().trim().to_string();
            match consts.get(&ident).cloned().or_else(|| KNOWN_CONSTS.iter().find(|(k, _)| *k == ident).map(|(_, v)| v.to_string())) {
                Some(name) => found.push((at, name)),
                None => unreadable.push(ident),
            }
        }
    }
    found.sort();
    found
}

/// The grade a declaration's source says: `.risk("x")`, or `.risk(CONST)` read from the file, else
/// `standard`. Anything this cannot read is `unknown`, which is above `safe`.
fn grade_in(span: &str, consts: &BTreeMap<String, String>) -> String {
    let Some(at) = span.find(".risk(") else { return "standard".into() };
    let rest = span[at + 6..].trim_start();
    if let Some(grade) = rest.strip_prefix('"').and_then(|r| r.split('"').next()) {
        return grade.to_string();
    }
    let ident: String = rest.chars().take_while(|c| !matches!(c, ')' | ',')).collect();
    consts.get(ident.trim()).cloned().unwrap_or_else(|| "unknown".into())
}

fn scan_rust(app: &str, files: &[PathBuf], helper: bool, found: &mut Found, unreadable: &mut Vec<String>, owners: &mut Vec<(String, String, String)>) {
    for file in files {
        let src = code_of(file);
        let consts = constants(&src);
        let names = declared(&src, helper, &consts, unreadable);
        for (i, (at, name)) in names.iter().enumerate() {
            let end = names.get(i + 1).map_or(src.len(), |(next, _)| *next);
            let grade = grade_in(&src[*at..end], &consts);
            found.insert((app.to_string(), name.clone()), grade);
            owners.push((app.to_string(), name.clone(), file.file_name().unwrap().to_string_lossy().to_string()));
        }
    }
}

/// The Blender add-on's `_action("name", "purpose", "grade", ...)` rows.
fn scan_blender(file: &Path, found: &mut Found) {
    let src = std::fs::read_to_string(file).unwrap_or_default();
    let starts: Vec<usize> = src.match_indices("_action(\"").map(|(at, _)| at).collect();
    for (i, at) in starts.iter().enumerate() {
        let end = starts.get(i + 1).copied().unwrap_or(src.len());
        let span = &src[*at..end];
        let name = span["_action(\"".len()..].split('"').next().unwrap_or("").to_string();
        let grade = ["safe", "standard", "sensitive", "dangerous"]
            .iter()
            .filter_map(|g| span.find(&format!("\"{g}\"")).map(|p| (p, *g)))
            .min()
            .map_or("unknown", |(_, g)| g);
        found.insert(("blender".into(), name), grade.into());
    }
}

/// The browser service's `@s.action(grade="x", ...)` over `def name(`.
fn scan_browser(file: &Path, found: &mut Found) {
    let src = std::fs::read_to_string(file).unwrap_or_default();
    for (at, _) in src.match_indices("@s.action(") {
        let rest = &src[at..];
        let Some(def) = rest.find("def ") else { continue };
        let head = &rest[..def];
        let grade = head.split("grade=\"").nth(1).and_then(|g| g.split('"').next()).unwrap_or("unknown");
        let name = rest[def + 4..].split('(').next().unwrap_or("").trim().to_string();
        found.insert(("browser".into(), name), grade.to_string());
    }
}

fn surfaces() -> (Found, Vec<String>, Vec<(String, String, String)>) {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let apps = manifest.join("../../apps");
    let (mut found, mut unreadable, mut owners) = (Found::new(), Vec::new(), Vec::new());
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&apps).expect("apps/").flatten().map(|e| e.path()).filter(|p| p.join("src").is_dir()).collect();
    dirs.sort();
    for dir in &dirs {
        let mut files = Vec::new();
        rust_files(&dir.join("src"), &mut files);
        scan_rust(&dir.file_name().unwrap().to_string_lossy(), &files, true, &mut found, &mut unreadable, &mut owners);
    }
    scan_blender(&apps.join("blender/addon/yantrik_blender/surface.py"), &mut found);
    scan_browser(&apps.join("browser/yantrik_browser/service.py"), &mut found);
    let src = manifest.join("../yantrik-ui/src");
    let mut shell: Vec<PathBuf> = std::fs::read_dir(&src)
        .expect("yantrik-ui/src")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| (n.starts_with("control") || n == "caller_class.rs") && n.ends_with(".rs") && !n.ends_with("tests.rs") && !n.ends_with("_test.rs")))
        .collect();
    shell.sort();
    scan_rust("shell", &shell, false, &mut found, &mut unreadable, &mut owners);
    (found, unreadable, owners)
}

fn listed() -> Vec<(String, String)> {
    let mut all: Vec<(String, String)> = REFUSED.iter().map(|r| (r.app.to_string(), r.action.to_string())).collect();
    for group in REVIEWED {
        for pair in group.actions {
            let (app, action) = pair.split_once('.').unwrap_or_else(|| panic!("`{pair}` is not app.action"));
            all.push((app.to_string(), action.to_string()));
        }
    }
    all
}

#[test]
fn the_scan_reads_the_surfaces_it_is_meant_to() {
    let (found, unreadable, _) = surfaces();
    let apps: std::collections::BTreeSet<&str> = found.keys().map(|(a, _)| a.as_str()).collect();
    for app in ["calendar", "text-editor", "notes", "email", "document-editor", "shell", "blender", "browser", "spreadsheet"] {
        assert!(apps.contains(app), "the scan found nothing of {app}: {apps:?}");
    }
    assert!(found.len() >= 250, "the scan found {} actions", found.len());
    assert_eq!(found.get(&("calendar".into(), "delete_event".into())).map(String::as_str), Some("sensitive"));
    assert_eq!(found.get(&("calendar".into(), "update_event".into())).map(String::as_str), Some("sensitive"));
    assert_eq!(found.get(&("calendar".into(), "add_event".into())).map(String::as_str), Some("standard"));
    for ident in &unreadable {
        assert!(UNREADABLE.contains(&ident.as_str()), "a declaration named `{ident}` cannot be read by this scan: name it in KNOWN_CONSTS, or read it");
    }
}

#[test]
fn every_action_graded_above_safe_is_refused_or_reviewed() {
    let (found, _, _) = surfaces();
    let on_a_list: std::collections::BTreeSet<(String, String)> = listed().into_iter().collect();
    let missing: Vec<String> = found
        .iter()
        .filter(|(_, grade)| grade.as_str() != "safe")
        .filter(|(key, _)| !on_a_list.contains(*key))
        .map(|((app, action), grade)| format!("{app}.{action} ({grade})"))
        .collect();
    assert!(
        missing.is_empty(),
        "these actions are graded above safe and are on neither list of `app_guard`: put each in REFUSED (its effect is one of the four \
         questions') or in REVIEWED with the reason it is not: {missing:#?}"
    );
}

#[test]
fn every_listed_pair_exists() {
    let (found, _, _) = surfaces();
    let stale: Vec<String> = listed().into_iter().filter(|key| !found.contains_key(key)).map(|(a, n)| format!("{a}.{n}")).collect();
    assert!(stale.is_empty(), "listed, and no longer declared by the app (renamed or removed?): {stale:?}");
}

#[test]
fn no_pair_is_on_both_lists_or_twice() {
    let all = listed();
    let mut seen = std::collections::BTreeSet::new();
    for key in &all {
        assert!(seen.insert(key.clone()), "{}.{} is listed twice or on both lists", key.0, key.1);
    }
}

/// The four questions' names belong to the shell's `control_questions.rs` alone: no app and no
/// other shell file declares an action of that name (#1313, M1: `send_message` was already the
/// Lens's).
#[test]
fn nothing_else_declares_a_questions_name() {
    let (_, _, owners) = surfaces();
    for q in Question::ALL {
        for (app, name, file) in &owners {
            assert!(name != q.action(), "{app} declares `{name}` in {file}: that name is a question of the shell's");
        }
    }
}
