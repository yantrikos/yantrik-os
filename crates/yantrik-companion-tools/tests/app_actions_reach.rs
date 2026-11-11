//! Every action every `apps/*` app publishes, and which of them an unattended `app_action` may call (#1175).
//!
//! `app_action` is one tool name for every app's every action, so what it reaches is in the action,
//! and the apps declare it: `.fetches_address(param)` for an action that takes an address,
//! `.open_ended()` for one that runs a line it is given. This reads the source of every app, builds
//! each action with the real `Action` builder (so its description carries exactly the sentence the app
//! publishes), and asks the code that decides: the actions that declare either are refused with
//! nobody there, and every other action is not. An app that gains such an action, or a declaration
//! this scan cannot attach to an action, fails here until the change is looked at.

use std::path::{Path, PathBuf};

use yantrik_companion_core::background_research;
use yantrik_companion_core::reach::LOCAL_APPS;
use yantrik_companion_tools::app_reach::refused_unattended;
use yantrik_ipc_contracts::control_surface::Action;

/// What one app's source declares: each action by name, and which of them say they reach out.
struct Surface {
    app: String,
    actions: Vec<(String, Option<Declared>)>,
}

#[derive(Clone)]
enum Declared {
    Address(String),
    OpenEnded,
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

/// The names of the actions declared in `src`, as `(position, name)`: `Action::new("name"` and the
/// `act("name"` helpers the apps wrap it in.
fn declared_names(src: &str) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    for opener in ["Action::new(", "act("] {
        for (at, _) in src.match_indices(opener) {
            let before = src[..at].chars().next_back().unwrap_or(' ');
            if opener == "act(" && (before.is_alphanumeric() || before == '_' || before == '.') {
                continue;
            }
            let rest = src[at + opener.len()..].trim_start();
            if let Some(name) = rest.strip_prefix('"').and_then(|r| r.split('"').next()) {
                found.push((at, name.to_string()));
            }
        }
    }
    found.sort();
    found
}

fn surface_of(app_dir: &Path) -> Surface {
    let mut files = Vec::new();
    rust_files(&app_dir.join("src"), &mut files);
    let mut actions: Vec<(String, Option<Declared>)> = Vec::new();
    let mut declarations = 0;
    for file in files {
        // Comments talk about the declarations without making them.
        let src: String = std::fs::read_to_string(&file)
            .unwrap_or_default()
            .lines()
            .map(|l| if l.trim_start().starts_with("//") { "" } else { l })
            .collect::<Vec<_>>()
            .join("\n");
        let names = declared_names(&src);
        let mut here: Vec<(String, Option<Declared>)> = names.iter().map(|(_, n)| (n.clone(), None)).collect();
        for (marker, open_ended) in [(".fetches_address(", false), (".open_ended()", true)] {
            for (at, _) in src.match_indices(marker) {
                declarations += 1;
                let owner = names.iter().rposition(|(pos, _)| *pos < at).unwrap_or_else(|| panic!("{}: `{marker}` with no action before it", file.display()));
                let param = src[at + marker.len()..].trim_start().strip_prefix('"').and_then(|r| r.split('"').next()).unwrap_or("url");
                here[owner].1 = Some(if open_ended { Declared::OpenEnded } else { Declared::Address(param.to_string()) });
            }
        }
        actions.extend(here);
    }
    let attached = actions.iter().filter(|(_, d)| d.is_some()).count();
    assert_eq!(attached, declarations, "{}: a declaration was attached to an action twice, or to none", app_dir.display());
    Surface { app: app_dir.file_name().unwrap().to_string_lossy().into_owned(), actions }
}

fn surfaces() -> Vec<Surface> {
    let apps = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&apps).expect("apps/").flatten().map(|e| e.path()).filter(|p| p.join("src").is_dir()).collect();
    dirs.sort();
    dirs.iter().map(|d| surface_of(d)).collect()
}

/// What the app would publish for this action, from the real builder.
fn published(app: &str, name: &str, declared: &Option<Declared>) -> serde_json::Value {
    let base = Action::new(name, &format!("{app} action {name}, as its app describes it"));
    let action = match declared {
        None => base,
        Some(Declared::Address(param)) => base.fetches_address(param),
        Some(Declared::OpenEnded) => base.open_ended(),
    };
    serde_json::json!({ "actions": [{ "name": action.name, "description": action.description }] })
}

#[test]
fn exactly_the_actions_that_declare_an_address_or_a_command_line_are_refused_with_nobody_there() {
    assert!(!background_research::person_started());
    let surfaces = surfaces();
    let total: usize = surfaces.iter().map(|s| s.actions.len()).sum();
    assert!(surfaces.len() >= 15 && total >= 100, "the scan found {} apps and {total} actions", surfaces.len());

    let mut refused = Vec::new();
    for surface in &surfaces {
        for (name, declared) in &surface.actions {
            let view = published(&surface.app, name, declared);
            let said = refused_unattended(&surface.app, name, Some(&view));
            assert_eq!(said.is_some(), declared.is_some(), "{}.{name}: refused {said:?}, declared {}", surface.app, declared.is_some());
            if let Some(said) = said {
                assert!(said.starts_with("Refused:"), "{said}");
                refused.push(format!("{}.{name}", surface.app));
            }
        }
    }
    refused.sort();
    refused.dedup();
    assert_eq!(refused, ["download-manager.add", "terminal.run", "terminal.send_input"], "an app now declares a reach: is it meant to?");
}

#[test]
fn a_person_asking_is_never_refused_for_any_of_them() {
    let _mine = background_research::person_turn();
    for surface in surfaces() {
        for (name, declared) in &surface.actions {
            assert_eq!(refused_unattended(&surface.app, name, Some(&published(&surface.app, name, declared))), None, "{}.{name}", surface.app);
        }
    }
}

#[test]
fn the_apps_an_unattended_call_may_drive_without_a_describe_declare_no_such_action() {
    let surfaces = surfaces();
    for app in LOCAL_APPS.iter().filter(|a| surfaces.iter().any(|s| s.app == **a)) {
        let surface = surfaces.iter().find(|s| s.app == *app).unwrap();
        let reaching: Vec<&str> = surface.actions.iter().filter(|(_, d)| d.is_some()).map(|(n, _)| n.as_str()).collect();
        assert!(reaching.is_empty(), "{app} is on the local list and declares {reaching:?}");
    }
    for app in ["download-manager", "terminal"] {
        assert!(!LOCAL_APPS.contains(&app), "{app}");
    }
}
