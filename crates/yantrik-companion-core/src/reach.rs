//! Which tools reach the network, and the one rule about them (#1171).
//!
//! **No tool that reaches the open web runs in a turn nobody started, unless background research
//! is on; and none that drives a browser runs in one at all.** The person's own turn (chat, voice,
//! a button they pressed) is exempt: they asked.
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
    /// The open internet, with no host the person named: a search, a page, a download, a probe of
    /// someone else's address. Unattended only while research is on (a search goes to their
    /// SearXNG or else DuckDuckGo).
    Web,
    /// A browser: it loads whatever page it is sent to, runs its scripts and carries its sign-ins.
    /// Only in a turn the person started, switch or no switch.
    Browser,
}

/// In the `browser` category, but only closing or reporting on the browser. (`web_search` is in that
/// category too and is `Web`: unattended, it goes to SearXNG or DuckDuckGo and never to a browser.)
const BROWSER_LOCAL: &[&str] = &["browser_cleanup", "browser_status"];
/// Drive or open a browser from outside the `browser` category.
const BROWSER_NAMES: &[&str] = &["search_sources", "open_url"];
/// Categories whose every tool fetches from, or posts to, the open internet.
const WEB_CATEGORIES: &[&str] = &["network", "github"];
/// Single tools that do the same, from categories that are otherwise local.
const WEB_NAMES: &[&str] = &[
    "web_search",
    "clipboard_fetch_url",
    "git_clone",
    "antivirus_update",
    "package_install",
    "network_ping",
    "network_traceroute",
    "network_dns",
    "network_diagnose",
];
/// Categories that talk to a service the person set up.
const SERVICE_CATEGORIES: &[&str] =
    &["email", "calendar", "telegram", "whatsapp", "home_assistant", "ssh", "connector", "mcp", "weather", "vision", "canvas", "ai"];

/// The apps `app_action` can drive that are a browser.
const BROWSER_APPS: &[&str] = &["browser", "chromium"];

/// What this call reaches. `args` matter for one tool: `app_action` drives any app, so its reach
/// is its app's.
pub fn reach(name: &str, category: &str, args: &serde_json::Value) -> Reach {
    if name == "app_action" {
        let app = args.get("app").and_then(serde_json::Value::as_str).unwrap_or("").trim().to_ascii_lowercase().replace('_', "-");
        let app = app.strip_prefix("app-").unwrap_or(&app);
        return if BROWSER_APPS.contains(&app) { Reach::Browser } else { Reach::Local };
    }
    if BROWSER_NAMES.contains(&name) {
        Reach::Browser
    } else if WEB_NAMES.contains(&name) || WEB_CATEGORIES.contains(&category) {
        Reach::Web
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
        Reach::Browser => "A browser is used only in a turn the person started, so it did not run",
        _ => "Searching or fetching from the web with nobody asking is off on this machine until the person allows it, so it did not run",
    };
    format!(
        "Refused: `{name}` reaches the internet. {why}, and nothing was sent. \
         Do not look for another way to do this. If the person is here, the desktop is offering them a card to \
         allow background research; a search or page they ask for themselves in chat is not held back."
    )
}

/// The gate `ToolRegistry::execute` runs before every tool: `Ok` to run it, or what to tell the
/// model. A refusal that research being off caused also raises the offer card, when the person is here.
pub fn check_call(name: &str, category: &str, args: &serde_json::Value) -> Result<(), String> {
    let reach = reach(name, category, args);
    if permits(reach, background_research::person_started(), background_research::is_on()) {
        return Ok(());
    }
    tracing::info!(tool = name, "an unattended call that reaches the internet was refused");
    background_research::wanted(name);
    Err(refusal(name, reach))
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
        for (name, category) in [("browse", "browser"), ("launch_browser", "browser"), ("browser_search", "browser"), ("search_sources", "life_assistant")] {
            assert_eq!(of(name, category), Reach::Browser, "{name}");
        }
        for (name, category) in [("http_fetch", "network"), ("web_fetch", "network"), ("download_file", "network"), ("web_search", "browser"), ("github_repos", "github")] {
            assert_eq!(of(name, category), Reach::Web, "{name}");
        }
        assert_eq!(of("browser_cleanup", "browser"), Reach::Local, "closing the browser sends nothing");
        assert_eq!(of("read_file", "files"), Reach::Local);
        assert_eq!(of("email_send", "email"), Reach::Service);
    }

    #[test]
    fn driving_the_browser_through_app_action_is_the_browser() {
        for app in ["browser", "Chromium", "app-browser"] {
            assert_eq!(reach("app_action", "app", &json!({ "app": app, "action": "go" })), Reach::Browser, "{app}");
        }
        assert_eq!(reach("app_action", "app", &json!({ "app": "notes", "action": "new" })), Reach::Local);
        assert_eq!(reach("app_action", "app", &json!({})), Reach::Local);
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
