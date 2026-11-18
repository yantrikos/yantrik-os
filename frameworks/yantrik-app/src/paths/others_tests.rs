//! Every app id, on the framework or still to be ported, against every other component's file.

use std::path::{Path, PathBuf};

use super::*;
use crate::paths::{Paths, APPS_DIR, DEFAULT_PROFILE, SETTINGS_FILE};

/// The repository, from this crate's manifest (`frameworks/yantrik-app`).
fn repo() -> PathBuf {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    repo.canonicalize().unwrap_or(repo)
}

/// The ids the apps in `apps/` use or will use: each folder's name, and every id its sources
/// declare (`const ID`, `APP_ID`, an instance `claim`, `App::new`), plus the ports to come.
fn app_ids() -> Vec<String> {
    let mut ids: Vec<String> = [
        // The framework apps today.
        "weather", "notes", "email", "containers", "image-viewer", "network", "system-monitor",
        // The ports to come, by folder and by the ids they use now.
        "calendar", "music-player", "music", "terminal", "browser", "spreadsheet", "document-editor",
        "documents", "text-editor", "editor", "snippet-manager", "snippets", "download-manager",
        "downloads", "desktop-files", "presentation", "studio", "arcade", "blender",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let apps = repo().join("apps");
    let folders = std::fs::read_dir(&apps).unwrap_or_else(|e| panic!("{}: {e}", apps.display()));
    for folder in folders.flatten().filter(|f| f.path().is_dir()) {
        ids.push(folder.file_name().to_string_lossy().to_string());
        for source in ["src/lib.rs", "src/main.rs"] {
            let Ok(text) = std::fs::read_to_string(folder.path().join(source)) else { continue };
            for marker in ["const ID: &'static str = \"", "APP_ID: &str = \"", "claim(\"", "App::new(\""] {
                for (at, _) in text.match_indices(marker) {
                    let rest = &text[at + marker.len()..];
                    if let Some(end) = rest.find('"') {
                        ids.push(rest[..end].to_string());
                    }
                }
            }
        }
    }
    ids.sort();
    ids.dedup();
    ids.retain(|id| !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    ids
}

fn root_dir(root: Root) -> &'static str {
    match root {
        Root::Config => "config",
        Root::Data => "data",
        Root::State => "state",
        Root::Cache => "cache",
    }
}

fn entry_path(base: &Path, e: &Entry) -> PathBuf {
    base.join(root_dir(e.root)).join("yantrik").join(e.name)
}

#[test]
fn the_scan_finds_the_framework_apps() {
    let ids = app_ids();
    for id in ["weather", "notes", "email", "containers", "image-viewer", "network", "system-monitor"] {
        assert!(ids.iter().any(|i| i == id), "{id} in {ids:?}");
    }
}

/// The fault this guards: under `yantrik/<id>.json`, Email's settings were email-service's
/// account file. No app's settings may be, or sit inside, any other component's entry.
#[test]
fn no_apps_settings_file_is_another_components() {
    let base = Path::new("/r");
    assert!(is_others(Root::Config, "email.json", "email"), "the collision this rule removed");
    for id in app_ids() {
        let p = Paths::under(base, &id, DEFAULT_PROFILE);
        let expected = base.join("config/yantrik").join(APPS_DIR).join(&id).join(SETTINGS_FILE);
        assert_eq!(p.config_file(), expected);
        for e in ENTRIES {
            let other = entry_path(base, e);
            assert!(
                !p.config_file().starts_with(&other) && !p.imported_record().starts_with(&other),
                "{id}'s settings {} land on {} ({})",
                p.config_file().display(),
                other.display(),
                e.owner
            );
        }
    }
}

/// An app's data, state and cache folders are `<root>/yantrik/<id>/`: one only meets another
/// component's entry where that entry names the app as sharing it (Notes and its vault).
#[test]
fn no_apps_folders_are_another_components_unless_shared_on_purpose() {
    let base = Path::new("/r");
    for id in app_ids() {
        let p = Paths::under(base, &id, DEFAULT_PROFILE);
        for (dir, root) in [(p.data_dir(), Root::Data), (p.state_dir(), Root::State), (p.cache_dir(), Root::Cache)] {
            for e in ENTRIES.iter().filter(|e| e.root == root && e.app != Some(id.as_str())) {
                let other = entry_path(base, e);
                assert!(
                    !covers(e, &id) && !other.starts_with(dir),
                    "{id}'s {} meets {} ({})",
                    dir.display(),
                    other.display(),
                    e.owner
                );
            }
        }
    }
}

#[test]
fn the_apps_directory_is_the_frameworks_alone_and_the_table_is_one_entry_a_name() {
    for e in ENTRIES {
        assert!(!covers(e, APPS_DIR), "{} keeps `{APPS_DIR}`: the framework's own", e.owner);
        assert!(!e.name.contains('/') && !e.name.is_empty(), "{}: first component only", e.name);
        let twins = ENTRIES.iter().filter(|o| o.root == e.root && o.name == e.name).count();
        assert_eq!(twins, 1, "{:?} {} twice", e.root, e.name);
    }
}

/// The names a component makes from its own file (a backup, a marker, a temporary) are its own
/// too: a former file or an app folder by any of them is another component's.
#[test]
fn an_entry_covers_the_names_made_from_it() {
    for (root, name) in [
        (Root::Config, "deepseek.json.before-yantrik"),
        (Root::Config, "pi.json.before-yantrik"),
        (Root::Config, "openclaw.env.before-yantrik"),
        (Root::Config, "openclaw.json.before-yantrik.absent"),
        (Root::Config, "companion-models.json.before-yantrik"),
        (Root::Config, "mind-mode.json.tmp"),
        (Root::Config, "mind-panel.json.new"),
        (Root::Config, "free-ai.json.new"),
        (Root::Config, ".privacy.json.4242.123456789"),
        (Root::Config, "email.json.tmp"),
        (Root::Config, "providers.yaml.1"),
        (Root::Data, "taint-scopes.json.4242"),
        (Root::Data, "notifications.tmp4242"),
        (Root::Data, "notifications.notices.json"),
        (Root::Data, "notifications.json.unreadable-1700000000"),
        (Root::Data, "notices.json.migrated-1700000000"),
        (Root::Data, "notices-archive.3.jsonl"),
        (Root::Data, "last-brief.tmp"),
    ] {
        assert!(is_others(root, name, "any-app"), "{root:?} {name}");
    }
    assert!(!is_others(Root::Config, "weather.json", "weather"), "an app's own former file is its own");
    assert!(is_others(Root::Config, "weather.json", "notes"), "and no other app's");
    assert!(!is_others(Root::Config, "notes.json", "notes"));
}

/// The names under the four roots that the rest of the system writes, as its sources spell them
/// (`.config/yantrik/<name>`, `.local/share/yantrik/<name>`, `.local/state/yantrik/<name>`,
/// `.cache/yantrik/<name>`, `join("yantrik").join("<name>")`), are every one in the table: a
/// service that starts keeping a new file there fails this until it is added.
#[test]
fn every_name_the_rest_of_the_system_keeps_is_in_the_table() {
    let repo = repo();
    let mut sources = Vec::new();
    for top in ["services", "crates", "deploy", "harnesses", "packaging", "scripts", "sdk", "tools", "frameworks"] {
        collect(&repo.join(top), &mut sources);
    }
    // The framework's own paths are its own: the rule and its tests.
    sources.retain(|p| !p.to_string_lossy().contains("frameworks/yantrik-app/"));
    // A test's fixtures spell paths the test makes up, not files a service keeps.
    sources.retain(|p| {
        let name = p.file_name().unwrap_or_default().to_string_lossy().to_string();
        let in_tests_dir = p.components().any(|c| c.as_os_str() == "tests");
        !(in_tests_dir || name == "tests.rs" || name.ends_with("_tests.rs") || name.starts_with("test_"))
    });
    assert!(sources.len() > 100, "the sources are there to read: {}", sources.len());
    let roots = [
        (".config/yantrik/", Some(Root::Config)),
        (".local/share/yantrik/", Some(Root::Data)),
        (".local/state/yantrik/", Some(Root::State)),
        (".cache/yantrik/", Some(Root::Cache)),
        // Shell: `${XDG_CONFIG_HOME:-$HOME/.config}/yantrik/<name>` and the like.
        (".config}/yantrik/", Some(Root::Config)),
        (".local/share}/yantrik/", Some(Root::Data)),
        (".local/state}/yantrik/", Some(Root::State)),
        (".cache}/yantrik/", Some(Root::Cache)),
        ("XDG_CONFIG_HOME}/yantrik/", Some(Root::Config)),
        ("XDG_DATA_HOME}/yantrik/", Some(Root::Data)),
        ("XDG_STATE_HOME}/yantrik/", Some(Root::State)),
        ("XDG_CACHE_HOME}/yantrik/", Some(Root::Cache)),
        // Rust: `join("yantrik").join("<name>")`, or `.join(NAME)` with `const NAME: &str = "…"`.
        ("join(\"yantrik\").join(", None),
    ];
    let mut missing = Vec::new();
    for path in &sources {
        let Ok(text) = std::fs::read_to_string(path) else { continue };
        let squeezed: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        for (marker, root) in roots {
            let haystack = if marker.contains('/') && !marker.contains('}') { &text } else { &squeezed };
            for (at, _) in haystack.match_indices(marker) {
                let rest = &haystack[at + marker.len()..];
                let Some(name) = name_at(rest, marker, &squeezed) else {
                    missing.push(format!("a name not resolved after `{marker}` ({})", path.display()));
                    continue;
                };
                let name = name.as_str();
                if name.is_empty() {
                    continue;
                }
                let listed = ENTRIES.iter().any(|e| root.is_none_or(|r| e.root == r) && covers(e, name));
                if !listed {
                    missing.push(format!("{root:?} {name} ({})", path.display()));
                }
            }
        }
    }
    missing.sort();
    missing.dedup();
    assert!(missing.is_empty(), "kept under a yantrik root, not in paths/others.rs: {missing:#?}");
}

/// The name a path is built with: the characters after a `…/yantrik/` marker, or after
/// `join("yantrik").join(` the string literal, or the value of the `const` it names in the same
/// file. `None` for a `join(` whose argument is neither (a name the scan cannot see).
fn name_at(rest: &str, marker: &str, squeezed: &str) -> Option<String> {
    let word = |s: &str| -> String { s.chars().take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')).collect() };
    if !marker.ends_with("join(") {
        return Some(word(rest).trim_end_matches('.').to_string());
    }
    if let Some(quoted) = rest.strip_prefix('"') {
        return Some(word(quoted).trim_end_matches('.').to_string());
    }
    let ident: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
    if ident.is_empty() || !rest[ident.len()..].starts_with(')') || !ident.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') {
        // A variable or an expression: a run-time name, not one this scan can list.
        return Some(String::new());
    }
    ["&str=\"", "&'staticstr=\""].iter().find_map(|ty| {
        let decl = format!("const{ident}:{ty}");
        squeezed.find(&decl).map(|at| word(&squeezed[at + decl.len()..]).trim_end_matches('.').to_string())
    })
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            if !matches!(name.as_str(), "target" | "node_modules" | ".git" | "__pycache__") {
                collect(&path, out);
            }
        } else if ["rs", "sh", "py", "yaml", "yml", "toml", "service", "desktop", "ts", "js", "conf"]
            .iter()
            .any(|ext| path.extension().is_some_and(|e| e == *ext))
            || path.extension().is_none()
        {
            out.push(path);
        }
    }
}

/// Nothing but this crate writes under `yantrik/apps/`: a component that started to would put a
/// file where an app's settings go.
#[test]
fn no_other_component_names_the_apps_directory() {
    let repo = repo();
    let mut found = Vec::new();
    for top in ["services", "crates", "deploy", "harnesses", "packaging", "scripts", "apps"] {
        scan(&repo.join(top), &mut found);
    }
    assert!(found.is_empty(), "these name `yantrik/apps/`, the framework's own directory: {found:?}");
}

fn scan(dir: &Path, found: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            if !matches!(name.as_str(), "target" | "node_modules" | ".git") {
                scan(&path, found);
            }
            continue;
        }
        let text_like = ["rs", "sh", "py", "yaml", "yml", "toml", "json", "service", "desktop"]
            .iter()
            .any(|ext| path.extension().is_some_and(|e| e == *ext))
            || path.extension().is_none();
        if !text_like {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        // Code, not prose: a comment may say where the settings are.
        let code: String = text
            .lines()
            .filter(|l| !["//", "#", "*", "/*"].iter().any(|c| l.trim_start().starts_with(c)))
            .collect::<Vec<_>>()
            .join("
");
        let text = code;
        let squeezed: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        if text.contains("yantrik/apps/") || text.contains("yantrik/apps\"") || squeezed.contains(r#"join("yantrik").join("apps")"#) {
            found.push(path.display().to_string());
        }
    }
}
