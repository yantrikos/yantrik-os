//! Every action every `apps/*` app publishes, and which of them an unattended `app_action` may call (#1175).
//!
//! `app_action` is one tool name for every app's every action, so what it reaches is in the action,
//! and the apps declare it: `.fetches_address(param)` for an action that takes an address,
//! `.open_ended()` for one that runs a line it is given. This reads the source of every app, builds
//! each action with the real `Action` builder (so its description carries exactly the sentence the app
//! publishes), and asks the code that decides: the actions that declare either are refused with
//! nobody there, and every other action is not. An app that gains such an action, or a declaration
//! this scan cannot attach to an action, fails here until the change is looked at.
//!
//! The shell is read the same way (`crates/yantrik-ui/src/control*.rs`, over a hundred actions): it is
//! the app `app_action` drives most, and it declares `open_ended` on its own actions. The actions
//! `reach::PERSON_ONLY_ACTIONS` holds person-only, whose description cannot say what they reach, must
//! exist in one of these surfaces, and an action whose own source names a network client, a browser or an
//! installer must be declared, person-only, or listed here as known to stay on the machine.

use std::path::{Path, PathBuf};

use yantrik_companion_core::background_research;
use yantrik_companion_core::reach::{reach, Reach, LOCAL_APPS, PERSON_ONLY_ACTIONS};
use yantrik_companion_tools::app_reach::refused_unattended;
use yantrik_ipc_contracts::control_surface::Action;

/// What one app's source declares: each action by name, and which of them say they reach out.
struct Surface {
    app: String,
    actions: Vec<(String, Option<Declared>)>,
    /// Each action's own source, from its `Action::new` to the next one's.
    spans: Vec<(String, String)>,
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
    surface_from(&app_dir.file_name().unwrap().to_string_lossy(), &files)
}

/// The shell's actions are declared across `control*.rs` (and `caller_class.rs`, for the desktop's own services).
fn shell_surface() -> Surface {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../yantrik-ui/src");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&src)
        .expect("yantrik-ui/src")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| (n.starts_with("control") || n == "caller_class.rs") && n.ends_with(".rs") && !n.ends_with("tests.rs") && !n.ends_with("_test.rs")))
        .collect();
    files.sort();
    surface_from("shell", &files)
}

fn surface_from(app: &str, files: &[PathBuf]) -> Surface {
    let mut actions: Vec<(String, Option<Declared>)> = Vec::new();
    let mut spans: Vec<(String, String)> = Vec::new();
    let mut declarations = 0;
    for file in files {
        // Comments talk about the declarations without making them. Whatever follows `#[cfg(test)]`
        // is a test's own surface, not the app's.
        let whole = std::fs::read_to_string(file).unwrap_or_default();
        let src: String = whole
            .split("\n#[cfg(test)]\nmod ")
            .next()
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
        for (i, (at, name)) in names.iter().enumerate() {
            let end = names.get(i + 1).map_or(src.len(), |(next, _)| *next);
            spans.push((name.clone(), src[*at..end].to_string()));
        }
        actions.extend(here);
    }
    let attached = actions.iter().filter(|(_, d)| d.is_some()).count();
    assert_eq!(attached, declarations, "{app}: a declaration was attached to an action twice, or to none");
    Surface { app: app.to_string(), actions, spans }
}

fn surfaces() -> Vec<Surface> {
    let apps = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&apps).expect("apps/").flatten().map(|e| e.path()).filter(|p| p.join("src").is_dir()).collect();
    dirs.sort();
    let mut all: Vec<Surface> = dirs.iter().map(|d| surface_of(d)).collect();
    all.push(shell_surface());
    all
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
    assert_eq!(
        refused,
        ["download-manager.add", "shell.agent_input", "shell.agent_run", "shell.send_message", "terminal.run", "terminal.send_input"],
        "an app now declares a reach: is it meant to?"
    );
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
    for app in ["download-manager", "terminal", "shell"] {
        assert!(!LOCAL_APPS.contains(&app), "{app}");
    }
    let shell = surfaces.iter().find(|s| s.app == "shell").expect("the shell's surface is scanned");
    for open_ended in ["agent_run", "agent_input", "send_message"] {
        assert!(shell.actions.iter().any(|(n, d)| n == open_ended && matches!(d, Some(Declared::OpenEnded))), "shell.{open_ended}");
    }
}

/// What `reach::PERSON_ONLY_ACTIONS` names exists: a renamed action must not leave its guard behind, and
/// an app that cannot say what an action reaches (free text to a geocoder, an install from GitHub, a
/// sign-in page) is held by name.
#[test]
fn every_person_only_action_exists_and_is_refused_unattended() {
    let surfaces = surfaces();
    for (app, action) in PERSON_ONLY_ACTIONS {
        let surface = surfaces.iter().find(|s| s.app == *app).unwrap_or_else(|| panic!("no app {app}"));
        assert!(surface.actions.iter().any(|(n, _)| n == action), "{app}.{action} is not an action any more");
        let call = serde_json::json!({ "app": app, "action": action, "args": {} });
        assert_eq!(reach("app_action", "", &call), Reach::Browser, "{app}.{action}");
        assert!(yantrik_companion_tools::app_reach::refused_before_start(&call).is_some(), "{app}.{action}");
    }
}

/// An action whose own source names a network client, a browser or an installer, and that
/// is neither declared nor held person-only, is a reach nobody said: the scan cannot see through a helper
/// it calls, so this is the part of the problem it can catch. What stays here stays on the machine, or is
/// the person's to ask for.
#[test]
fn an_action_that_names_the_network_in_its_own_source_declares_it() {
    const REACHING: [&str; 7] = ["ureq::", "reqwest", "TcpStream", "open::that", "xdg-open", "Command::new(\"curl\"", "Command::new(\"npm\""];
    // Known, with what each does: none sends anything with nobody there.
    const KNOWN: [&str; 0] = [];
    let mut undeclared = Vec::new();
    for surface in surfaces() {
        for ((name, declared), (_, span)) in surface.actions.iter().zip(surface.spans.iter()) {
            let held = PERSON_ONLY_ACTIONS.contains(&(surface.app.as_str(), name.as_str()));
            let names_one = REACHING.iter().any(|needle| span.contains(needle));
            let id = format!("{}.{name}", surface.app);
            if names_one && declared.is_none() && !held && !KNOWN.contains(&id.as_str()) {
                undeclared.push(id);
            }
        }
    }
    undeclared.sort();
    assert!(undeclared.is_empty(), "actions whose source reaches out and that declare nothing: {undeclared:?}");
}
