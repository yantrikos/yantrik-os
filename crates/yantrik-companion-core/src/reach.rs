//! Which tools reach the network, and the one rule about them (#1171).
//!
//! **With nobody there, the only tool that reaches the open web is `web_search`, and only while
//! background research is on; every other fetch, probe or browser runs in a turn a person started
//! or not at all.** The person's own turn (chat, voice, a button they pressed) is exempt: they asked.
//!
//! The reason is the card: it promises "search terms, to your own SearXNG or else DuckDuckGo", and
//! only a search keeps that promise. A page, a package, a host or a tool server's call can go
//! anywhere, so research being on does not widen to them (Fable, 11 Nov).
//!
//! Every tool has a [`Reach`], read from this one table by [`reach`], and the rule is enforced once,
//! in `ToolRegistry::execute`, through [`check_call`]. Nothing is matched by the words in an urge or
//! a model's reply: a tool that reaches the web is refused whatever the model called it for, and a
//! tool nobody listed is `Local`. That default is why the table is complete only if a test says so:
//! `yantrik-companion`'s `reach_declared` test registers every tool the companion has, and fails on
//! one that is `Local` while its source drives an HTTP client or a browser.
//!
//! The policy is a table here, not a method on each tool, for the reason `taint` gives: it can be
//! read end to end in a minute.

use crate::background_research;

/// What a tool reaches when it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// This machine only: files, processes, the clock, the person's memory, the local calendar.
    Local,
    /// A service the person set up and named (their mail, their phone, their Home Assistant, a
    /// host they SSH to, the weather they asked for, the model they chose). It is what they
    /// connected the OS to, so no research switch applies.
    Service,
    /// A web search, and nothing else: it goes to the person's SearXNG or else DuckDuckGo, which is
    /// what the research card says. Unattended only while research is on.
    Web,
    /// Anything else that reaches the open internet, or drives something that can: a browser, a page,
    /// a download, a package, a probe of someone else's address, a tool server, code with a network.
    /// Only in a turn the person started, switch or no switch.
    Browser,
}

/// In the `browser` category, but only closing or reporting on the browser. (`web_search` is in that
/// category too and is `Web`: unattended, it goes to SearXNG or DuckDuckGo and never to a browser.)
const BROWSER_LOCAL: &[&str] = &["browser_cleanup", "browser_status"];
/// Drive or open a browser from outside the `browser` category, or run something that can: `claude_code`
/// runs `claude -p` with `Bash`, and `run_background` and `docker_exec` run whatever command they are
/// given, which is `curl` of anything. (`run_command` runs only the programs of `safe_command`.)
const BROWSER_NAMES: &[&str] = &["search_sources", "open_url", "claude_code", "run_background", "docker_exec"];
/// Categories whose every tool fetches from, or posts to, the open internet. `mcp` is here because a
/// tool server the person added can be a fetch or a search (`fetch`, `brave-search`), and nothing in its
/// tool list says so.
const FETCH_CATEGORIES: &[&str] = &["network", "github", "mcp"];
/// The one tool research unlocks.
const SEARCH: &str = "web_search";
/// Single tools that fetch or probe, from categories that are otherwise local. `network_dns_set` does
/// not fetch: it points the machine's resolver at servers the model names, so every name the OS
/// looks up afterwards goes to them.
const FETCH_NAMES: &[&str] = &[
    "clipboard_fetch_url",
    "git_clone",
    "antivirus_update",
    "package_install",
    "network_ping",
    "network_traceroute",
    "network_dns",
    "network_diagnose",
    "network_dns_set",
    // Not a tool: the step the code runners take before a script, when it needs a package from pip,
    // npm or gem. They ask this gate for it (`coder_sandbox::allow_install`), outside any sandbox.
    "install_deps",
];
/// Categories that talk to a service the person set up.
const SERVICE_CATEGORIES: &[&str] =
    &["email", "calendar", "telegram", "whatsapp", "home_assistant", "ssh", "connector", "weather", "vision", "canvas", "ai"];

/// The `sandbox` values of `code_execute` and `script_run` that give the script a network, or no
/// sandbox at all (`coder_sandbox::SandboxMode::from_str` reads the same ones; a test there pins it).
const CODE_NETWORK_SANDBOXES: &[&str] = &["network", "web", "none", "unsafe"];

/// The apps `app_action` can drive that are a browser, or that fetch or run whatever they are given
/// (the download manager's `add`, the terminal's `run` and `send_input`). The apps' own descriptions say
/// so too, and `yantrik-companion-tools`' `app_reach` reads them; this list is the second line, for a
/// `describe` that fails.
const BROWSER_APPS: &[&str] = &["browser", "chromium", "download-manager", "terminal"];
/// The apps with no action that fetches an address or takes a command line, which an unattended
/// `app_action` may drive when their `describe` fails. A source test holds every one to that.
pub const LOCAL_APPS: &[&str] = &[
    "shell", "notes", "calendar", "text-editor", "document-editor", "spreadsheet", "presentation", "snippet-manager",
    "image-viewer", "system-monitor",
];

/// An app as `app_action` names it: `app-notes` and `Notes` are `notes`.
pub fn app_id(app: &str) -> String {
    let app = app.trim().to_ascii_lowercase().replace('_', "-");
    app.strip_prefix("app-").unwrap_or(&app).to_string()
}

/// What this call reaches. `args` matter for three kinds of tool. `app_action` drives any app, so its
/// reach is its app's (and `AppActionTool` reads the action's own declaration besides). `ssh_check_host`
/// is a service only for a host the person configured or has been to. The code runners run code the model wrote: in the strict sandbox it has no
/// network, and asked for any other way it can fetch anything, so that is the browser's reach (only
/// in a turn the person started; "SearXNG, else DuckDuckGo" is not what code with a network does).
pub fn reach(name: &str, category: &str, args: &serde_json::Value) -> Reach {
    if matches!(name, "code_execute" | "script_run") {
        let sandbox = args.get("sandbox").and_then(serde_json::Value::as_str).unwrap_or("strict").trim().to_ascii_lowercase();
        return if CODE_NETWORK_SANDBOXES.contains(&sandbox.as_str()) { Reach::Browser } else { Reach::Local };
    }
    if name == "app_action" {
        let app = app_id(args.get("app").and_then(serde_json::Value::as_str).unwrap_or(""));
        return if BROWSER_APPS.contains(&app.as_str()) { Reach::Browser } else { Reach::Local };
    }
    if name == "ssh_check_host" {
        let host = args.get("host").and_then(serde_json::Value::as_str).unwrap_or("");
        return if crate::ssh_hosts::is_known(host) { Reach::Service } else { Reach::Browser };
    }
    if name == SEARCH {
        Reach::Web
    } else if BROWSER_NAMES.contains(&name) || FETCH_NAMES.contains(&name) || FETCH_CATEGORIES.contains(&category) {
        Reach::Browser
    } else if category == "browser" && !BROWSER_LOCAL.contains(&name) {
        Reach::Browser
    } else if SERVICE_CATEGORIES.contains(&category) || matches!(name, "telegram_send" | "whatsapp_send" | "connect_service" | "sync_service") {
        Reach::Service
    } else {
        Reach::Local
    }
}

/// Whether a call of this reach may run, for a turn the person started or not and with research on
/// or off.
pub fn permits(reach: Reach, person_started: bool, research_on: bool) -> bool {
    match reach {
        Reach::Local | Reach::Service => true,
        Reach::Web => person_started || research_on,
        Reach::Browser => person_started,
    }
}

/// What the model is told when a call is refused, in words it can say to the person.
fn refusal(name: &str, reach: Reach) -> String {
    let why = match reach {
        Reach::Browser => "A browser, a page or package fetched, a host probed, or code run with a network is used only in a turn the person started, so it did not run",
        _ => "Searching the web with nobody asking is off on this machine until the person allows it, so it did not run",
    };
    format!(
        "Refused: `{name}` reaches the internet. {why}, and nothing was sent. \
         Do not look for another way to do this. If the person is here, the desktop is offering them a card to \
         allow background research; a search or page they ask for themselves in chat is not held back."
    )
}

/// The refusal for an unattended call, logged; a search held back by research being off also raises
/// the offer card, when the person is here (nothing else would be let through by it).
fn deny(name: &str, reach: Reach) -> String {
    tracing::info!(tool = name, "an unattended call that reaches the internet was refused");
    if reach == Reach::Web {
        background_research::wanted(name);
    }
    refusal(name, reach)
}

/// The gate `ToolRegistry::execute` runs before every tool: `Ok` to run it, or what to tell the
/// model.
pub fn check_call(name: &str, category: &str, args: &serde_json::Value) -> Result<(), String> {
    let reach = reach(name, category, args);
    if permits(reach, background_research::person_started(), background_research::is_on()) {
        return Ok(());
    }
    Err(deny(name, reach))
}

/// A call whose reach only its target can say (`app_action`: the app's own description of the action
/// declares that it fetches an address or runs a command line): the same refusal, for a turn nobody
/// started.
pub fn check_declared(name: &str) -> Result<(), String> {
    if background_research::person_started() {
        return Ok(());
    }
    Err(deny(name, Reach::Browser))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn of(name: &str, category: &str) -> Reach {
        reach(name, category, &json!({}))
    }

    #[test]
    fn the_tools_a_model_reaches_for_after_a_refused_search_are_all_classed() {
        for (name, category) in [
            ("browse", "browser"),
            ("launch_browser", "browser"),
            ("browser_search", "browser"),
            ("search_sources", "life_assistant"),
            ("http_fetch", "network"),
            ("web_fetch", "network"),
            ("download_file", "network"),
            ("github_repos", "github"),
        ] {
            assert_eq!(of(name, category), Reach::Browser, "{name}");
        }
        assert_eq!(of("web_search", "browser"), Reach::Web, "the one tool research unlocks");
        assert_eq!(of("browser_cleanup", "browser"), Reach::Local, "closing the browser sends nothing");
        assert_eq!(of("read_file", "files"), Reach::Local);
        assert_eq!(of("email_send", "email"), Reach::Service);
    }

    #[test]
    fn driving_the_browser_through_app_action_is_the_browser() {
        for app in ["browser", "Chromium", "app-browser", "download-manager", "app-download_manager", "Terminal"] {
            assert_eq!(reach("app_action", "app", &json!({ "app": app, "action": "go" })), Reach::Browser, "{app}");
        }
        assert_eq!(reach("app_action", "app", &json!({ "app": "notes", "action": "new" })), Reach::Local);
        assert_eq!(reach("app_action", "app", &json!({})), Reach::Local);
    }

    #[test]
    fn code_that_is_given_a_network_is_the_browsers_reach_and_strict_code_is_local() {
        for tool in ["code_execute", "script_run"] {
            for sandbox in ["network", "web", "none", "unsafe", "Network", " NONE "] {
                assert_eq!(reach(tool, "coder", &json!({ "sandbox": sandbox })), Reach::Browser, "{tool} {sandbox}");
                assert!(check_call(tool, "coder", &json!({ "sandbox": sandbox })).is_err(), "{tool} {sandbox} unattended");
            }
            for args in [json!({ "sandbox": "strict" }), json!({}), json!({ "sandbox": 3 })] {
                assert_eq!(reach(tool, "coder", &args), Reach::Local, "{tool} {args}");
            }
        }
        let _mine = background_research::person_turn();
        assert_eq!(check_call("code_execute", "coder", &json!({ "sandbox": "network" })), Ok(()), "the person asked");
    }

    #[test]
    fn what_can_fetch_for_a_model_that_was_told_no_is_not_a_service() {
        assert_eq!(of("install_deps", "coder"), Reach::Browser, "pip, npm and gem fetch from public registries");
        assert_eq!(of("claude_code", "ai"), Reach::Browser, "claude -p runs Bash, so curl");
        assert_eq!(of("claude_think", "ai"), Reach::Service, "the model the person chose");
        for (name, category) in [("run_background", "background_tasks"), ("docker_exec", "docker")] {
            assert_eq!(of(name, category), Reach::Browser, "{name} runs any command, curl included");
        }
        assert_eq!(of("run_command", "desktop"), Reach::Local, "only the programs on safe_command's list");
        for name in ["mcp__fetch_fetch", "mcp__brave-search_search", "mcp__anything_at_all"] {
            assert_eq!(of(name, "mcp"), Reach::Browser, "{name}: a tool server can fetch, and its tool list does not say");
        }
    }

    /// Round 5, medium 3: the card says "SearXNG, else DuckDuckGo", which only a search keeps.
    #[test]
    fn research_unlocks_a_search_and_nothing_that_can_go_anywhere() {
        for (name, category) in [
            ("http_fetch", "network"),
            ("web_fetch", "network"),
            ("download_file", "network"),
            ("git_clone", "git"),
            ("package_install", "system"),
            ("antivirus_update", "security"),
            ("network_ping", "networking"),
            ("network_dns", "networking"),
            ("install_deps", "coder"),
            ("mcp__fetch_fetch", "mcp"),
            ("github_repos", "github"),
        ] {
            assert_eq!(of(name, category), Reach::Browser, "{name}");
            assert!(!permits(of(name, category), false, true), "{name} runs with research on and nobody there");
        }
        assert!(permits(of("web_search", "browser"), false, true));
    }

    #[test]
    fn what_reaches_out_without_fetching_is_held_too() {
        assert_eq!(of("network_dns_set", "networking"), Reach::Browser, "it repoints every name the OS looks up");
        assert_eq!(of("network_status", "networking"), Reach::Local);
        // A host nobody configured is a probe of someone else's address: person-only.
        let probe = reach("ssh_check_host", "ssh", &json!({ "host": "a.host-the-model-made-up.example" }));
        assert_eq!(probe, Reach::Browser);
        assert_eq!(reach("ssh_check_host", "ssh", &json!({})), Reach::Browser, "no host, no service");
        assert_eq!(of("ssh_list_hosts", "ssh"), Reach::Service);
    }

    #[test]
    fn a_declared_reach_is_refused_the_same_way() {
        let said = check_declared("app_action").expect_err("unattended");
        assert!(said.starts_with("Refused:") && said.contains("app_action"), "{said}");
        let _mine = background_research::person_turn();
        assert_eq!(check_declared("app_action"), Ok(()));
    }

    #[test]
    fn the_rule_in_four_lines() {
        // (reach, person started, research on) -> may run
        for (r, person, on, runs) in [
            (Reach::Web, false, false, false),
            (Reach::Web, false, true, true),
            (Reach::Web, true, false, true),
            (Reach::Browser, false, false, false),
            (Reach::Browser, false, true, false),
            (Reach::Browser, true, false, true),
            (Reach::Local, false, false, true),
            (Reach::Service, false, false, true),
        ] {
            assert_eq!(permits(r, person, on), runs, "{r:?} person={person} on={on}");
        }
    }

    #[test]
    fn an_unattended_call_is_refused_and_a_persons_is_not() {
        // The test process never turns research on.
        for (name, category) in [("browser_search", "browser"), ("http_fetch", "network"), ("web_search", "browser")] {
            let said = check_call(name, category, &json!({})).expect_err(name);
            assert!(said.starts_with("Refused:") && said.contains(name), "{said}");
            assert!(!said.contains("  "), "no stray runs of spaces: {said}");
            let _mine = background_research::person_turn();
            assert_eq!(check_call(name, category, &json!({})), Ok(()), "{name} in a turn the person started");
        }
    }
}
