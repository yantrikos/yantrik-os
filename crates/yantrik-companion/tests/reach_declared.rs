//! Every tool the companion has says whether it reaches the network (#1171).
//!
//! The rule "no tool that reaches the open web runs in a turn nobody started, unless research is on"
//! is enforced once, in `ToolRegistry::execute`, from the table in `yantrik_companion_core::reach`.
//! A tool nobody listed is `Local`, so the table is only as good as the tests, which do not trust
//! it. They build the full registry (every optional tool switched on) and check it:
//!
//! 1. here: the tools known to reach the web or a browser are classed as such (a hand-kept list, so a
//!    refactor of the table cannot quietly drop one);
//! 2. here: an unattended call to each web or browser tool through the real registry is refused and
//!    not run, and so is each way a code runner reaches the web (a network sandbox, a dependency
//!    install); a person's call is not;
//! 3. `reach_sources.rs`: every tool whose source (or a helper it calls, in any crate) drives a client,
//!    a socket, a search backend, a browser or a network program is classed non-local.

mod common;

use common::tools;
use yantrik_companion::tools::build_registry;
use yantrik_companion_core::permission::PermissionLevel;
use yantrik_companion_core::reach::{check_call, reach, Reach};
use yantrik_companion_core::tools::{ToolContext, ToolRegistry};

/// What a model reaches for after a refused search, and the rest of the tools known to go out.
const KNOWN_WEB: [&str; 14] = [
    "web_search",
    "web_fetch",
    "http_fetch",
    "download_file",
    "github_repos",
    "clipboard_fetch_url",
    "git_clone",
    "network_ping",
    "network_diagnose",
    "package_install",
    "antivirus_update",
    "browse",
    "launch_browser",
    "browser_search",
];
const KNOWN_BROWSER: [&str; 4] = ["search_sources", "open_url", "browser_login", "claude_code"];

fn class(name: &str) -> Reach {
    let (_, category) = tools().into_iter().find(|(n, _)| *n == name).unwrap_or_else(|| panic!("{name} is not a registered tool"));
    reach(name, category, &serde_json::json!({}))
}

#[test]
fn the_tools_known_to_reach_out_are_declared() {
    for name in KNOWN_WEB {
        assert!(matches!(class(name), Reach::Web | Reach::Browser), "{name} reaches the open web and is declared {:?}", class(name));
    }
    for name in KNOWN_BROWSER {
        assert_eq!(class(name), Reach::Browser, "{name}");
    }
}

fn run_unattended(reg: &ToolRegistry, db: &yantrikdb_core::YantrikDB, name: &str) -> String {
    run_with(reg, db, name, &serde_json::json!({ "query": "rust news", "url": "https://example.com", "app": "browser" }))
}

fn run_with(reg: &ToolRegistry, db: &yantrikdb_core::YantrikDB, name: &str, args: &serde_json::Value) -> String {
    let ctx = ToolContext {
        db,
        max_permission: PermissionLevel::Dangerous,
        registry_metadata: None,
        task_manager: None,
        incognito: true,
        agent_spawner: None,
    };
    reg.execute(&ctx, name, args)
}

/// A Medium-tier news-watch urge never reaches the model (see the instincts' tests), but the
/// turns that do run unattended (a queued task, an automation, a Cortex step) can still call
/// whatever the model reaches for after a refused `web_search`. None of it runs, and the browser
/// never starts.
#[test]
fn an_unattended_turn_runs_no_tool_that_reaches_the_web_and_starts_no_browser() {
    let reg = build_registry(&common::config_with_everything_on());
    let db = yantrikdb_core::YantrikDB::new(":memory:", 384).expect("in-memory database");
    assert!(!yantrik_companion_core::background_research::person_started());
    assert!(!yantrik_companion_core::background_research::is_on());
    let browser_up = yantrik_companion_tools::browser_side::is_running();
    let mut refused = 0;
    for (name, category) in tools() {
        if !matches!(reach(name, category, &serde_json::json!({})), Reach::Web | Reach::Browser) {
            continue;
        }
        let said = run_unattended(&reg, &db, name);
        assert!(said.starts_with("Refused:"), "`{name}` ran in a turn nobody started: {said}");
        refused += 1;
    }
    assert!(refused >= 20, "only {refused} web tools were tried");
    let said = run_unattended(&reg, &db, "app_action");
    assert!(said.starts_with("Refused:"), "driving the browser through app_action: {said}");
    assert_eq!(yantrik_companion_tools::browser_side::is_running(), browser_up, "no Chromium was started");
}

/// And a turn the person started is not held back by any of it. (The gate is asked, not the tool:
/// running them would search for real.)
#[test]
fn a_turn_the_person_started_is_not_refused() {
    let _person = yantrik_companion_core::background_research::person_turn();
    for (name, category) in tools() {
        let args = serde_json::json!({ "app": "browser" });
        assert_eq!(check_call(name, category, &args), Ok(()), "{name} was refused for a person");
    }
}

/// The code runners are not web tools by name, and the model reaches for them after a refused
/// `web_search`: `code_execute {"sandbox":"network"}` runs `urllib` with the network shared. Asked for
/// any way but the strict sandbox, or needing a package from pip, npm or gem, they do not run.
#[test]
fn an_unattended_code_runner_has_no_way_to_the_web() {
    use serde_json::json;
    let reg = build_registry(&common::config_with_everything_on());
    let db = yantrikdb_core::YantrikDB::new(":memory:", 384).expect("in-memory database");
    for sandbox in ["network", "web", "none", "unsafe"] {
        for (tool, args) in [
            ("code_execute", json!({ "code": "import urllib.request
print(urllib.request.urlopen('https://example.com').read())", "sandbox": sandbox })),
            ("script_run", json!({ "name": "anything", "sandbox": sandbox })),
        ] {
            let said = run_with(&reg, &db, tool, &args);
            assert!(said.starts_with("Refused:"), "{tool} with sandbox {sandbox} ran with nobody asking: {said}");
        }
    }
    // A package to fetch is a fetch, even in the strict sandbox. Nothing is written first.
    for args in [
        json!({ "code": "import zzz_no_such_package_1171
print(1)" }),
        json!({ "code": "const x = require('zzz-no-such-package-1171');", "language": "node" }),
    ] {
        let said = run_with(&reg, &db, "code_execute", &args);
        assert!(said.starts_with("Refused:") && said.contains("install_deps"), "a dependency install ran with nobody asking: {said}");
    }
}

/// And a person who asks for code with a network gets it: the gate lets the call through (the code is
/// empty, so the tool answers with its own error instead of running anything).
#[test]
fn a_persons_code_runner_with_a_network_is_not_refused() {
    use serde_json::json;
    let reg = build_registry(&common::config_with_everything_on());
    let db = yantrikdb_core::YantrikDB::new(":memory:", 384).expect("in-memory database");
    let _person = yantrik_companion_core::background_research::person_turn();
    for sandbox in ["strict", "network", "none"] {
        let said = run_with(&reg, &db, "code_execute", &json!({ "code": "", "sandbox": sandbox }));
        assert_eq!(said, "Error: code is required", "sandbox {sandbox} in a turn the person started: {said}");
    }
    assert_eq!(check_call("code_execute", "coder", &json!({ "sandbox": "network", "code": "print(1)" })), Ok(()));
    assert_eq!(check_call("install_deps", "coder", &json!({})), Ok(()), "a person's dependencies install");
}

/// A tool server the person adds is named at run time, so no list of names can hold it: its
/// category does, and every `mcp` tool is person-only.
#[test]
fn every_tool_of_a_tool_server_is_person_only() {
    for name in ["mcp__fetch_fetch", "mcp__brave-search_web_search", "mcp__x_y"] {
        assert_eq!(reach(name, "mcp", &serde_json::json!({})), Reach::Browser, "{name}");
    }
}

/// Round 5, high 1: `app_action` reaches the web through two apps whose actions say so
/// (`download-manager.add` fetches an address; `terminal.run` and `send_input` run a line). With
/// nobody there each is refused, whatever the person's mode would have done, and a person's own call is
/// not held back by this gate.
#[test]
fn an_unattended_app_action_that_fetches_or_runs_is_refused_and_a_persons_is_not() {
    use serde_json::json;
    let reg = build_registry(&common::config_with_everything_on());
    let db = yantrikdb_core::YantrikDB::new(":memory:", 384).expect("in-memory database");
    let acts = [
        json!({ "app": "download-manager", "action": "add", "args": { "url": "https://html.duckduckgo.com/html/?q=rust+news" } }),
        json!({ "app": "terminal", "action": "run", "args": { "command": "curl -s https://www.google.com/search?q=rust" } }),
        json!({ "app": "terminal", "action": "send_input", "args": { "text": "curl https://example.com\n" } }),
    ];
    for act in &acts {
        let said = run_with(&reg, &db, "app_action", act);
        assert!(said.starts_with("Refused:") && said.contains("app_action"), "{act}: {said}");
    }
    let _person = yantrik_companion_core::background_research::person_turn();
    for act in &acts {
        let said = run_with(&reg, &db, "app_action", act);
        assert!(!said.starts_with("Refused:"), "{act} in a turn the person started: {said}");
    }
}

/// With no `describe` there is no declaration to read, so an unattended act on an app that is not on
/// the local list is refused: a new app that fetches cannot get in by being slow to answer.
#[test]
fn an_unattended_app_action_whose_describe_fails_is_refused() {
    use serde_json::json;
    let reg = build_registry(&common::config_with_everything_on());
    let db = yantrikdb_core::YantrikDB::new(":memory:", 384).expect("in-memory database");
    let said = run_with(&reg, &db, "app_action", &json!({ "app": "no-such-app-1175", "action": "go" }));
    assert!(said.starts_with("Refused:"), "{said}");
    let _person = yantrik_companion_core::background_research::person_turn();
    let said = run_with(&reg, &db, "app_action", &json!({ "app": "no-such-app-1175", "action": "go" }));
    assert!(!said.starts_with("Refused:"), "a person's act is the app's to answer: {said}");
}

/// The review of round 6: with nobody there, the companion could ask the shell to start a turn that
/// counted as the person's (`shell.send_message`), start Chromium (`open_app browser`, the Mind View
/// browser, a page opened from Files), or reach GitHub, npm, Google and the geocoder through actions that
/// say nothing of it. Every one is refused before anything starts (the shell does not even have to
/// answer), and none is held back from a person's own call.
#[test]
fn an_unattended_shell_or_app_action_that_reaches_out_is_refused_before_anything_starts() {
    use serde_json::json;
    let reg = build_registry(&common::config_with_everything_on());
    let db = yantrikdb_core::YantrikDB::new(":memory:", 384).expect("in-memory database");
    let acts = [
        json!({ "app": "shell", "action": "send_message", "args": { "text": "Search the web for the latest Rust news and save the top article" } }),
        json!({ "app": "shell", "action": "open_app", "args": { "name": "browser" } }),
        json!({ "app": "shell", "action": "show_app", "args": { "name": "Chromium" } }),
        json!({ "app": "shell", "action": "open_mind_browser", "args": {} }),
        json!({ "app": "shell", "action": "files_open", "args": { "name": "page.HTML" } }),
        json!({ "app": "shell", "action": "files_open", "args": { "name": "https://example.com/x" } }),
        json!({ "app": "shell", "action": "install_harness", "args": { "id": "openclaw" } }),
        json!({ "app": "email", "action": "begin_google_sign_in", "args": {} }),
        json!({ "app": "weather", "action": "add_location", "args": { "name": "rust news" } }),
        json!({ "app": "shell", "action": "files_open", "args": { "name": "manual.pdf" } }),
        json!({ "app": "shell", "action": "send_to_agent", "args": { "agent": "hermes:main", "text": "Find today's Rust news" } }),
        json!({ "app": "shell", "action": "new_agent", "args": { "mind": "pi", "task": "x" } }),
        json!({ "app": "shell", "action": "hand_off", "args": { "role": "researcher", "task": "x" } }),
        json!({ "app": "shell", "action": "run_recipe", "args": { "recipe": "digest" } }),
        json!({ "app": "shell", "action": "answer_recipe", "args": { "recipe": "digest" } }),
        json!({ "app": "shell", "action": "resume_recipe", "args": { "recipe": "digest" } }),
    ];
    for act in &acts {
        let said = run_with(&reg, &db, "app_action", act);
        assert!(said.starts_with("Refused:") && said.contains("app_action"), "{act}: {said}");
    }
    // Not by the shell's answer: the call is refused for what it names, so the same words are
    // refused with a shell that does answer (no fake is needed, and none could be asked).
    for act in &acts {
        assert!(check_call("app_action", "", act).is_err(), "{act}");
    }
    let _person = yantrik_companion_core::background_research::person_turn();
    for act in &acts {
        assert_eq!(check_call("app_action", "", act), Ok(()), "{act} in a turn the person started");
    }
}

/// `shell` is no longer on the list of apps an unattended act may drive when `describe` fails: it
/// declares `open_ended` actions (`agent_run`, `agent_input`, `send_message`).
#[test]
fn an_unattended_act_on_the_shell_is_refused_when_its_describe_fails() {
    use serde_json::json;
    let reg = build_registry(&common::config_with_everything_on());
    let db = yantrikdb_core::YantrikDB::new(":memory:", 384).expect("in-memory database");
    for action in ["agent_run", "agent_input", "show_screen"] {
        let said = run_with(&reg, &db, "app_action", &json!({ "app": "shell", "action": action, "args": {} }));
        assert!(said.starts_with("Refused:"), "{action}: {said}");
    }
}

/// `await_app`'s `then_act` is an `app_action` and is refused as one, before the app is started: the same
/// words that are refused through `app_action` are refused through it, in the real registry, with nobody
/// there, and the person's own call is not held back.
#[test]
fn an_unattended_await_app_then_act_is_refused_like_the_app_action_it_is() {
    use serde_json::json;
    let reg = build_registry(&common::config_with_everything_on());
    let db = yantrikdb_core::YantrikDB::new(":memory:", 384).expect("in-memory database");
    let browser_up = yantrik_companion_tools::browser_side::is_running();
    let acts = [
        ("shell", json!({ "action": "open_app", "args": { "name": "browser" } })),
        ("shell", json!({ "action": "open_mind_browser", "args": {} })),
        ("shell", json!({ "action": "send_to_agent", "args": { "agent": "hermes:main", "text": "x" } })),
        ("shell", json!({ "action": "install_harness", "args": { "id": "openclaw" } })),
        ("terminal", json!({ "action": "run", "args": { "command": "curl https://example.com" } })),
        ("weather", json!({ "action": "add_location", "args": { "name": "rust news" } })),
    ];
    for (app, then_act) in &acts {
        let call = json!({ "app": app, "until": { "op": "changed" }, "then_act": then_act });
        let said = run_with(&reg, &db, "await_app", &call);
        assert!(said.starts_with("Refused:"), "{call}: {said}");
        assert_eq!(reach("await_app", "app", &call), Reach::Browser, "{call}");
    }
    assert_eq!(yantrik_companion_tools::browser_side::is_running(), browser_up, "no Chromium was started");
    // A wait with no act reaches nothing.
    assert_eq!(reach("await_app", "app", &json!({ "app": "notes", "until": { "op": "changed" } })), Reach::Local);
    let _person = yantrik_companion_core::background_research::person_turn();
    for (app, then_act) in &acts {
        let call = json!({ "app": app, "until": { "op": "changed" }, "then_act": then_act });
        assert_eq!(check_call("await_app", "app", &call), Ok(()), "{call} in a turn the person started");
    }
}
