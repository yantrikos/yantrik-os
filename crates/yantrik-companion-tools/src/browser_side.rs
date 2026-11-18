//! The one browser the built-in companion's browser tools used to reach (#1085), and no longer can.
//!
//! Mind View's. These tools drove it over a DevTools port (`http://127.0.0.1:<port>/json`, then a
//! WebSocket to the tab). A port is reachable by every process of the machine, and a job that spoke
//! raw DevTools to it went round the fence the browser service puts on a browser's tabs (#1200): it
//! loaded an image from the person's protected `~/.ssh` into a page. Mind View's browser now runs
//! with `--remote-debugging-pipe`, held by the browser service, and has no port; these tools have
//! no road to it and say so. A mind's `browser` tool is the road: it goes through the service.
//!
//! Every reader of the built-in tools asks this module and nothing else, so none of them can fall
//! back to a port: there is no number here to fall back to.

use serde_json::Value;

pub const CDP_HOST: &str = "127.0.0.1";

/// Why a built-in `browser_*` tool has no browser to drive.
pub const GONE: &str = "Mind View's browser has no DevTools port any more, so the built-in browser_* tools cannot reach it. \
Use the `browser` tool instead: it drives Mind View's browser through the browser service (go, read, tabs, click).";

/// Whether the tools may use Mind View's browser: never; the reason.
pub fn guard() -> Result<(), String> {
    Err(GONE.to_string())
}

/// Whether Mind View's browser is up for these tools: it is not theirs to use.
pub fn is_running() -> bool {
    false
}

/// Mind View's browser's targets, for these tools: none, they have no road to it.
pub fn targets() -> Result<Vec<Value>, String> {
    guard().map(|()| Vec::new())
}

/// Close one of Mind View's browser's targets by id: refused, for the same reason.
pub fn close_target(_id: &str) -> Result<(), String> {
    guard()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_builtin_tools_have_no_road_to_mind_views_browser_and_say_where_the_road_is() {
        assert!(!is_running());
        for refused in [guard(), targets().map(|_| ()), close_target("T1")] {
            let why = refused.unwrap_err();
            assert!(why.contains("no DevTools port") && why.contains("`browser` tool"), "{why}");
        }
    }

    #[test]
    fn nothing_here_spells_a_port() {
        let source = include_str!("browser_side.rs");
        let code: String = source.split("#[cfg(test)]").next().unwrap().lines().filter(|l| !l.trim_start().starts_with("//")).collect();
        assert!(!code.contains("port:") && !code.contains("PORT") && !code.contains("curl"), "a way back to a port");
    }
}
