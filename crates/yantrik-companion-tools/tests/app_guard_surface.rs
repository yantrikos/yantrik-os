//! The reviewed list of `app_action` is honest: every pair on it exists, under the id the app binds
//! (#1313, round 6).
//!
//! The guard is an allow list decided at run time (`app_guard::admit`): an action runs only if the app
//! grades it `safe` or it is on the reviewed list (`app_reviewed`), keyed on the id the app's socket is
//! bound under. What is not listed is refused whatever it does, so this test does not look for
//! dangers; it keeps the list honest. It reads the source of every `apps/*` app (Rust intents; the
//! Blender add-on's and the browser service's Python), of every `adapters/*` adapter (Python) and of
//! the shell's `control*.rs`, takes each app's id from its `.desktop` `X-Yantrik-Surface` (the shell's
//! table, parsed by `yantrik_shell_core`) and checks it against the app's `const ID`, and fails when
//! a listed pair no longer exists under its id, when a declaration it cannot name is not one of the
//! few named below, when the Python modules that register actions are not the ones counted here, and
//! when one of the four questions' names is declared by anything but the shell's `control_questions.rs`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

// The one parser the reach scans share (`yantrik-companion/tests/common/syntax.rs`), not a copy of it.
#[path = "../../yantrik-companion/tests/common/syntax.rs"]
mod syntax;
use syntax::without_test_items;

use yantrik_companion_tools::app_guard::{INSTEAD, REVIEWED};
use yantrik_companion_tools::asks::Question;
use yantrik_shell_core::apps::parse_desktop_text;

/// `(id, action)` to its grade.
type Found = BTreeMap<(String, String), String>;

/// Constants a shell declaration names that live in another crate (`yantrik-ipc-contracts`).
const KNOWN_CONSTS: &[(&str, &str)] =
    &[("SNAPSHOT_ACTION", "mind_desk"), ("STOP_ACTION", "mind_desk_stop"), ("AGENTS_ACTION", "mind_desk_open_agents")];

/// The declarations whose name is a variable, each the one line that needs it, and why that is fine:
/// the helper of an app's intents table takes the name its callers pass as a literal (which this scan
/// reads at the call), and the shell's question helper declares the four questions, the one place their
/// names are written. A declaration with a variable name anywhere else fails the scan until it is looked at.
const UNREADABLE: &[(&str, &str)] = &[
    ("apps/music-player/src/intents.rs", "Action::new(name, sentence).risk(\"standard\")"),
    ("apps/notes/src/intents/mod.rs", "Action::new(name, sentence)"),
    ("apps/studio/src/intents.rs", "Action::new(name, sentence)"),
    ("apps/terminal/src/intents.rs", "Action::new(name, &sentence)"),
    ("apps/text-editor/src/intents/mod.rs", "Action::new(name, sentence)"),
    ("crates/yantrik-ui/src/control_files.rs", "let spec = Action::new($name, $description);"),
    ("crates/yantrik-ui/src/control_questions.rs", "Action::new(question.name, question.sentence).risk(question.grade),"),
];

/// The Python modules that register actions, and no others: a fourth fails until it is read here.
const PYTHON_MODULES: &[&str] = &[
    "adapters/libreoffice/yantrik_libreoffice/surface.py",
    "apps/blender/addon/yantrik_blender/surface.py",
    "apps/browser/yantrik_browser/service.py",
];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn relative(path: &Path) -> String {
    let full = path.to_string_lossy().replace('\\', "/");
    let base = root().to_string_lossy().replace('\\', "/");
    let base = base.trim_end_matches('/');
    full.strip_prefix(base).unwrap_or(&full).trim_start_matches('/').to_string()
}

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

/// An unreadable file or declaration fails the test, never skips: a file that cannot be read is a
/// declaration nobody looked at.
fn read(file: &Path) -> String {
    std::fs::read_to_string(file).unwrap_or_else(|e| panic!("{} cannot be read: {e}", file.display()))
}

/// The file's code: without its test items and without comment lines.
fn code_of(file: &Path) -> String {
    without_test_items(&read(file))
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
/// helper the apps wrap it in (read as unreadable in an app's `intents` only: elsewhere `act(` is
/// another function). A name that is neither a literal nor a constant this scan can read is pushed to
/// `unreadable` with its line.
fn declared(src: &str, helper: bool, in_intents: bool, consts: &BTreeMap<String, String>, unreadable: &mut Vec<String>) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    let openers: &[&str] = if helper { &["Action::new(", "act("] } else { &["Action::new("] };
    for opener in openers {
        for (at, _) in src.match_indices(opener) {
            let before = src[..at].chars().next_back().unwrap_or(' ');
            if *opener == "act(" && (before.is_alphanumeric() || before == '_' || before == '.' || src[..at].ends_with("fn ")) {
                continue;
            }
            let rest = src[at + opener.len()..].trim_start();
            if let Some(name) = rest.strip_prefix('"').and_then(|r| r.split('"').next()) {
                found.push((at, name.to_string()));
                continue;
            }
            // Outside an app's intents, `act(` is some other function (`act(m, fx, action)`).
            if *opener == "act(" && !in_intents {
                continue;
            }
            let ident: String = rest.chars().take_while(|c| !matches!(c, ',' | ')' | '\n')).collect::<String>().trim().to_string();
            match consts.get(&ident).cloned().or_else(|| KNOWN_CONSTS.iter().find(|(k, _)| *k == ident).map(|(_, v)| v.to_string())) {
                Some(name) => found.push((at, name)),
                None => {
                    let line_start = src[..at].rfind('\n').map_or(0, |n| n + 1);
                    let line_end = src[at..].find('\n').map_or(src.len(), |n| at + n);
                    unreadable.push(src[line_start..line_end].trim().to_string());
                }
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

/// The scan's findings: every declared action, the unreadable declarations, and who declared what.
#[derive(Default)]
struct Scan {
    found: Found,
    /// `(file, line)` of each declaration whose name is a variable.
    unreadable: Vec<(String, String)>,
    /// `(id, action, file name)`.
    owners: Vec<(String, String, String)>,
}

fn scan_rust(id: &str, files: &[PathBuf], helper: bool, scan: &mut Scan) {
    for file in files {
        let src = code_of(file);
        let consts = constants(&src);
        let in_intents = relative(file).contains("/intents");
        let mut unreadable = Vec::new();
        let names = declared(&src, helper, in_intents, &consts, &mut unreadable);
        scan.unreadable.extend(unreadable.into_iter().map(|line| (relative(file), line)));
        for (i, (at, name)) in names.iter().enumerate() {
            let end = names.get(i + 1).map_or(src.len(), |(next, _)| *next);
            scan.found.insert((id.to_string(), name.clone()), grade_in(&src[*at..end], &consts));
            scan.owners.push((id.to_string(), name.clone(), file.file_name().unwrap().to_string_lossy().to_string()));
        }
    }
}

/// The surface id a `.desktop` file declares (`X-Yantrik-Surface`), parsed as the shell parses it.
fn desktop_surface(file: &Path) -> Option<String> {
    let stem = file.file_stem()?.to_string_lossy().to_string();
    parse_desktop_text(&stem, &read(file)).and_then(|entry| entry.surface)
}

/// An app's id: its `.desktop` file's `X-Yantrik-Surface`, which the `const ID` of its `lib.rs` must
/// say too when it has one. Neither is a failure.
fn app_id(dir: &Path) -> String {
    let folder = dir.file_name().unwrap().to_string_lossy().to_string();
    let desktop = desktop_surface(&root().join("apps/desktop-files").join(format!("yantrik-{folder}.desktop")));
    let lib = dir.join("src/lib.rs");
    let constant = lib.is_file().then(|| constants(&code_of(&lib)).get("ID").cloned()).flatten();
    match (desktop, constant) {
        (Some(d), Some(c)) => {
            assert_eq!(d, c, "apps/{folder}: the .desktop file's X-Yantrik-Surface and `const ID` disagree");
            d
        }
        (Some(id), None) | (None, Some(id)) => id,
        (None, None) => panic!("apps/{folder} declares no id: no X-Yantrik-Surface in its .desktop file and no `const ID`"),
    }
}

/// The Python modules under `dir` that register actions: a `_action("name", ...)` row (the Blender
/// add-on) or an `@x.action(...)` decorator.
fn python_modules(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
        if path.is_dir() {
            // `src/` is walked too: a module under any `src/` that registers an action is counted (#1313, M4).
            if !matches!(name.as_str(), "tests" | "__pycache__" | "target") {
                python_modules(&path, out);
            }
        } else if name.ends_with(".py") && read(&path).lines().any(registers_an_action) {
            out.push(path);
        }
    }
}

fn registers_an_action(line: &str) -> bool {
    let line = line.trim_start();
    line.starts_with("_action(\"") || (line.starts_with('@') && line.contains(".action("))
}

/// The Blender add-on's `_action("name", "purpose", "grade", ...)` rows.
fn scan_blender(id: &str, file: &Path, scan: &mut Scan) {
    let src = read(file);
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
        scan.found.insert((id.to_string(), name), grade.into());
    }
}

/// An `@x.action(...)` decorator over a function: the name is its first argument when that is a
/// string (the adapters), else the function's name (the browser service); the grade is `grade="x"`,
/// else `standard`.
fn scan_decorated(id: &str, file: &Path, scan: &mut Scan) {
    let src = read(file);
    for line_start in src.match_indices('@').map(|(at, _)| at).filter(|at| src[*at..].lines().next().is_some_and(registers_an_action)) {
        let rest = &src[line_start..];
        let def = rest.find("def ").unwrap_or_else(|| panic!("{}: a decorator with no function after it", file.display()));
        let head = &rest[..def];
        let after = head.split(".action(").nth(1).unwrap_or("").trim_start();
        let name = match after.strip_prefix('"') {
            Some(quoted) => quoted.split('"').next().unwrap_or("").to_string(),
            None => rest[def + 4..].split('(').next().unwrap_or("").trim().to_string(),
        };
        let grade = head.split("grade=\"").nth(1).and_then(|g| g.split('"').next()).unwrap_or("standard");
        scan.found.insert((id.to_string(), name), grade.to_string());
    }
}

/// The surface id a Python service builds: `Surface("browser", ...)`.
fn python_surface_id(file: &Path) -> String {
    let src = read(file);
    let at = src.find("Surface(\"").unwrap_or_else(|| panic!("{} builds no Surface(\"id\")", file.display()));
    src[at + 9..].split('"').next().unwrap_or("").to_string()
}

fn surfaces() -> Scan {
    let mut scan = Scan::default();
    let apps = root().join("apps");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&apps).expect("apps/").flatten().map(|e| e.path()).filter(|p| p.join("src").is_dir()).collect();
    dirs.sort();
    for dir in &dirs {
        let mut files = Vec::new();
        rust_files(&dir.join("src"), &mut files);
        scan_rust(&app_id(dir), &files, true, &mut scan);
    }
    let blender = desktop_surface(&apps.join("desktop-files/yantrik-blender.desktop")).expect("Blender's .desktop declares its surface");
    scan_blender(&blender, &apps.join("blender/addon/yantrik_blender/surface.py"), &mut scan);
    let service = apps.join("browser/yantrik_browser/service.py");
    scan_decorated(&python_surface_id(&service), &service, &mut scan);
    let office = root().join("adapters/libreoffice");
    let office_id = desktop_surface(&office.join("yantrik-libreoffice.desktop")).expect("the LibreOffice adapter's .desktop declares its surface");
    scan_decorated(&office_id, &office.join("yantrik_libreoffice/surface.py"), &mut scan);
    let src = root().join("crates/yantrik-ui/src");
    let mut shell: Vec<PathBuf> = std::fs::read_dir(&src)
        .expect("yantrik-ui/src")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| (n.starts_with("control") || n == "caller_class.rs") && n.ends_with(".rs") && !n.ends_with("tests.rs") && !n.ends_with("_test.rs")))
        .collect();
    shell.sort();
    scan_rust("shell", &shell, false, &mut scan);
    scan
}

/// Every pair the guard names: the reviewed list and the pairs that have an asking tool instead.
fn listed() -> Vec<(String, String)> {
    let mut all: Vec<(String, String)> = INSTEAD.iter().map(|i| (i.id.to_string(), i.action.to_string())).collect();
    for group in REVIEWED {
        for pair in group.actions {
            let (id, action) = pair.split_once('.').unwrap_or_else(|| panic!("`{pair}` is not id.action"));
            all.push((id.to_string(), action.to_string()));
        }
    }
    all
}

#[test]
fn the_scan_reads_the_surfaces_it_is_meant_to() {
    let scan = surfaces();
    let ids: std::collections::BTreeSet<&str> = scan.found.keys().map(|(a, _)| a.as_str()).collect();
    for id in ["calendar", "editor", "documents", "snippets", "containers", "music", "network", "notes", "email", "shell", "blender", "browser", "spreadsheet", "libreoffice"] {
        assert!(ids.contains(id), "the scan found nothing under the id {id}: {ids:?}");
    }
    for folder in ["text-editor", "document-editor", "snippet-manager", "container-manager", "music-player", "network-manager"] {
        assert!(!ids.contains(folder), "{folder} is a folder name, not an id the scan should key on: {ids:?}");
    }
    assert!(scan.found.len() >= 250, "the scan found {} actions", scan.found.len());
    assert_eq!(scan.found.get(&("calendar".into(), "delete_event".into())).map(String::as_str), Some("sensitive"));
    assert_eq!(scan.found.get(&("editor".into(), "save".into())).map(String::as_str), Some("standard"));
    assert!(scan.found.contains_key(&("libreoffice".into(), "write_text".into())), "the adapters are scanned");
    assert!(scan.found.contains_key(&("blender".into(), "run_python".into())), "the Blender add-on is scanned");
    assert!(scan.found.contains_key(&("browser".into(), "commit".into())), "the browser service is scanned");
}

/// A declaration whose name is a variable is a declaration nobody read: only the lines named above.
#[test]
fn a_declaration_the_scan_cannot_name_is_one_of_the_few_named() {
    let scan = surfaces();
    let stray: Vec<&(String, String)> =
        scan.unreadable.iter().filter(|(file, line)| !UNREADABLE.iter().any(|(f, l)| f == file && l == line)).collect();
    assert!(stray.is_empty(), "declared under a name this scan cannot read (name it in KNOWN_CONSTS, make it a literal, or list the line in UNREADABLE with why): {stray:#?}");
    for (file, line) in UNREADABLE {
        assert!(scan.unreadable.iter().any(|(f, l)| f == file && l == line), "UNREADABLE names a line that is not there any more: {file}: {line}");
    }
}

#[test]
fn the_python_modules_that_register_actions_are_the_ones_counted() {
    let mut found = Vec::new();
    python_modules(&root().join("apps"), &mut found);
    python_modules(&root().join("adapters"), &mut found);
    let mut found: Vec<String> = found.iter().map(|p| relative(p)).collect();
    found.sort();
    assert_eq!(found, PYTHON_MODULES, "a module that registers actions is not counted here (or one counted no longer does): read it, then list it");
}

#[test]
fn every_listed_pair_exists_under_the_id_the_app_binds() {
    let scan = surfaces();
    let stale: Vec<String> = listed().into_iter().filter(|key| !scan.found.contains_key(key)).map(|(a, n)| format!("{a}.{n}")).collect();
    assert!(stale.is_empty(), "listed, and not declared under that id (renamed, removed, or keyed on a folder name?): {stale:?}");
}

/// The four questions' names belong to the shell's `control_questions.rs` alone: no app and no
/// other shell file declares an action of that name (#1313, M1: `send_message` was already the
/// Lens's).
#[test]
fn nothing_else_declares_a_questions_name() {
    let scan = surfaces();
    for q in Question::ALL {
        for (app, name, file) in &scan.owners {
            assert!(name != q.action(), "{app} declares `{name}` in {file}: that name is a question of the shell's");
        }
    }
}

/// `app_guard::RESERVED` is the framework's own list less `present`: a name the framework reserves
/// for itself is read here before it is allowed, and `launch` is the one that opens files.
#[test]
fn the_reserved_names_are_the_frameworks() {
    use yantrik_companion_tools::app_guard::RESERVED;
    let src = read(&root().join("frameworks/yantrik-app/src/intents.rs"));
    let at = src.find("pub const RESERVED").expect("the framework declares its reserved names");
    let line = src[at..].lines().next().unwrap();
    let names: Vec<&str> = line.split('"').skip(1).step_by(2).collect();
    assert!(names.contains(&"launch"), "{line}");
    for name in &names {
        assert!(RESERVED.contains(name) || *name == "present", "the framework reserves `{name}`: refuse it in app_guard::RESERVED, or say here why it may stay");
    }
    for name in RESERVED {
        assert!(names.contains(name), "`{name}` is refused as reserved and the framework does not reserve it");
    }
    // `launch` is graded above `safe`, and every app gets it from the one place.
    let launch = read(&root().join("frameworks/yantrik-app/src/launch.rs"));
    let at = launch.find("pub(crate) fn action()").expect("launch's declaration");
    assert!(launch[at..].contains(".risk(\"standard\")") && !launch[at..].lines().take(14).any(|l| l.contains(".risk(\"safe\")")), "launch is graded standard");
}
