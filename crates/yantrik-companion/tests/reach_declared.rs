//! Every tool the companion has says whether it reaches the network (#1171).
//!
//! The rule "no tool that reaches the open web runs in a turn nobody started, unless research is on"
//! is enforced once, in `ToolRegistry::execute`, from the table in `yantrik_companion_core::reach`.
//! A tool nobody listed is `Local`, so the table is only as good as this test, which does not trust
//! it. It builds the full registry (every optional tool switched on) and checks it three ways:
//!
//! 1. the tools known to reach the web or a browser are classed as such (a hand-kept list, so a
//!    refactor of the table cannot quietly drop one);
//! 2. every tool whose `impl Tool` block drives an HTTP client, a socket, a search backend or the
//!    browser is classed non-local, or is in the reviewed list of tools that do and send nothing out;
//! 3. an unattended call to each web or browser tool through the real registry is refused and not run.
//!
//! (2) is what catches a new tool: add `curl` to a file and its tools must be declared.

use std::path::{Path, PathBuf};

use yantrik_companion::config::CompanionConfig;
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
const KNOWN_BROWSER: [&str; 3] = ["search_sources", "open_url", "browser_login"];

/// What in a tool's code means "this drives the network or a browser".
const MARKERS: [&str; 12] = [
    "ureq",
    "reqwest",
    "curl",
    "TcpStream",
    "UdpSocket",
    "http_get",
    "ensure_headless_browser",
    "browser_side",
    "connect_first_tab",
    "cdp_send",
    "searxng",
    "duckduckgo",
];

/// Tools whose code holds one of those markers and send nothing out, and why. Everything else with
/// a marker must be declared.
const LOCAL_DESPITE_MARKERS: [(&str, &str); 0] = [];

fn config_with_everything_on() -> CompanionConfig {
    serde_yaml::from_str(
        r#"
llm: { backend: ollama, api_base_url: "http://localhost:11434/v1", api_model: "m" }
home_assistant: { enabled: true, base_url: "http://ha.local", token: "t" }
telegram: { enabled: true, bot_token: "t", chat_id: "1" }
whatsapp: { enabled: true, phone_number_id: "p", access_token: "t" }
email: { enabled: true, accounts: [ { name: "n", email: "a@b.c", provider: "gmail" } ] }
"#,
    )
    .expect("a config with every optional tool on")
}

fn tools() -> Vec<(&'static str, &'static str)> {
    static TOOLS: std::sync::OnceLock<Vec<(&'static str, &'static str)>> = std::sync::OnceLock::new();
    TOOLS
        .get_or_init(|| {
            build_registry(&config_with_everything_on())
                .list_metadata(PermissionLevel::Dangerous)
                .into_iter()
                .map(|m| (m.name, m.category))
                .collect()
        })
        .clone()
}

fn class(name: &str) -> Reach {
    let (_, category) = tools().into_iter().find(|(n, _)| *n == name).unwrap_or_else(|| panic!("{name} is not a registered tool"));
    reach(name, category, &serde_json::json!({}))
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// The tool names a file registers: the string literal after each `fn name(&self) -> &'static str`.
fn tool_names(code: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = code;
    while let Some(at) = rest.find("fn name(&self) -> &'static str") {
        rest = &rest[at + 30..];
        let window = &rest[..rest.len().min(120)];
        if let Some(open) = window.find('"') {
            if let Some(len) = window[open + 1..].find('"') {
                names.push(window[open + 1..open + 1 + len].to_string());
            }
        }
    }
    names
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

#[test]
fn every_tool_whose_source_drives_a_client_or_a_browser_is_declared() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    rust_files(&crates.join("yantrik-companion-tools/src"), &mut files);
    rust_files(&crates.join("yantrik-companion/src/tools"), &mut files);
    rust_files(&crates.join("yantrik-companion/src/life_assistant"), &mut files);
    let registered = tools();
    let mut checked = 0;
    for file in files {
        let source = std::fs::read_to_string(&file).unwrap();
        let code = source.split("#[cfg(test)]").next().unwrap();
        for block in code.split("impl Tool for ").skip(1) {
            let Some(marker) = MARKERS.iter().find(|m| block.contains(**m)) else { continue };
            for name in tool_names(block) {
                let Some((_, category)) = registered.iter().find(|(n, _)| *n == name) else { continue };
                checked += 1;
                let declared = reach(&name, category, &serde_json::json!({})) != Reach::Local;
                let reviewed = LOCAL_DESPITE_MARKERS.iter().any(|(n, _)| *n == name);
                assert!(
                    declared || reviewed,
                    "`{name}` ({}) uses `{marker}` and is declared Local: declare what it reaches in yantrik_companion_core::reach, or list it in LOCAL_DESPITE_MARKERS with a reason",
                    file.display()
                );
            }
        }
    }
    assert!(checked > 25, "the scan found {checked} tools that reach out: it is not reading the sources");
}

#[test]
fn the_reviewed_local_tools_are_registered_and_local() {
    for (name, _) in LOCAL_DESPITE_MARKERS {
        assert_eq!(class(name), Reach::Local, "{name}");
    }
}

fn run_unattended(reg: &ToolRegistry, db: &yantrikdb_core::YantrikDB, name: &str) -> String {
    let ctx = ToolContext {
        db,
        max_permission: PermissionLevel::Dangerous,
        registry_metadata: None,
        task_manager: None,
        incognito: true,
        agent_spawner: None,
    };
    reg.execute(&ctx, name, &serde_json::json!({ "query": "rust news", "url": "https://example.com", "app": "browser" }))
}

/// A Medium-tier news-watch urge never reaches the model (see the instincts' tests), but the
/// turns that do run unattended (a queued task, an automation, a Cortex step) can still call
/// whatever the model reaches for after a refused `web_search`. None of it runs, and the browser
/// never starts.
#[test]
fn an_unattended_turn_runs_no_tool_that_reaches_the_web_and_starts_no_browser() {
    let reg = build_registry(&config_with_everything_on());
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
