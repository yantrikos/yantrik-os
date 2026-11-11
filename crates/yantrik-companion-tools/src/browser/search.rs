//! The web search tool's routes: which service a query goes to, for whom.
//!
//! A person's own turn may use the browser (SearXNG first, then Google in the headless Chromium,
//! then DuckDuckGo). A search with nobody asking (an instinct's urge, a queued task, a program on
//! the socket) goes to the person's SearXNG, else DuckDuckGo's HTML page, never to Chromium, and
//! only once the person has allowed background research (`yantrik_companion_core::background_research`).

use std::time::Duration;

use serde_json::json;
use yantrik_companion_core::background_research;

use super::{
    cdp_send, cleanup_after_data_extraction, connect_first_tab, detect_captcha, duckduckgo_html_search,
    ensure_headless_browser, eval_js, searxng_search,
};

/// What a search without a person behind it is told while research is off.
const REFUSED: &str = "Background web search is off on this machine, so nothing was searched. The person can     allow it on the card the desktop shows; a search they ask for in chat is not held back.";

/// Where a query may go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Route {
    /// Nowhere: research is off and nobody asked.
    Refused,
    /// SearXNG, else DuckDuckGo's HTML page. Never a browser.
    Quiet,
    /// SearXNG, else the headless Chromium on Google, else DuckDuckGo.
    Browser,
}

pub(super) fn route(person_started: bool, research_on: bool) -> Route {
    match (person_started, research_on) {
        (true, _) => Route::Browser,
        (false, true) => Route::Quiet,
        (false, false) => Route::Refused,
    }
}

/// The tool's search, for the turn this thread is running.
pub(super) fn web_search(query: &str) -> String {
    let route = route(background_research::person_started(), background_research::is_on());
    search_with(query, route, searxng_search, duckduckgo_html_search, google_search)
}

/// The routes with the three services handed in, so a test can see which was reached.
fn search_with(
    query: &str,
    route: Route,
    searxng: impl Fn(&str) -> Result<String, String>,
    duckduckgo: impl Fn(&str) -> String,
    google: impl Fn(&str) -> String,
) -> String {
    if route == Route::Refused {
        return REFUSED.to_string();
    }
    // SearXNG first (local, fast, no rate limits)
    match searxng(query) {
        Ok(results) => return results,
        Err(e) => tracing::debug!("SearXNG unavailable ({e}), trying browser/DDG"),
    }
    match route {
        Route::Browser => google(query),
        _ => duckduckgo(query),
    }
}

/// Google in the headless Chromium; DuckDuckGo when the browser cannot start or Google asks for a
/// CAPTCHA.
fn google_search(query: &str) -> String {
    // Auto-launch headless browser if not running — fall back to DuckDuckGo if unavailable
    if let Err(e) = ensure_headless_browser() {
        tracing::info!("Browser unavailable ({e}), using DuckDuckGo HTML fallback");
        return duckduckgo_html_search(query);
    }

    // URL-encode the query
    let encoded: String = query
        .chars()
        .map(|c| match c {
            ' ' => '+'.to_string(),
            c if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') => {
                c.to_string()
            }
            c => format!("%{:02X}", c as u32),
        })
        .collect();

    let url = format!("https://www.google.com/search?q={}&num=10&hl=en", encoded);

    let (mut ws, _tab) = match connect_first_tab() {
        Ok(v) => v,
        Err(e) => return format!("Error connecting to browser: {e}"),
    };

    // Navigate to Google search
    if let Err(e) = cdp_send(&mut ws, "Page.navigate", json!({ "url": url })) {
        return format!("Navigation error: {e}");
    }

    // Wait for results to load
    std::thread::sleep(Duration::from_secs(3));

    // Extract structured search results via JS
    let js = r#"
        (function() {
            var results = [];
            // Google search result blocks
            var items = document.querySelectorAll('div.g, div[data-hveid]');
            for (var i = 0; i < items.length && results.length < 10; i++) {
                var el = items[i];
                var link = el.querySelector('a[href^="http"]');
                if (!link) continue;
                var url = link.href;
                if (url.includes('google.com') || url.includes('accounts.google')) continue;
                var titleEl = el.querySelector('h3');
                var title = titleEl ? titleEl.innerText : '';
                if (!title) continue;
                var snippetEl = el.querySelector('div[data-sncf], span.aCOpRe, div.VwiC3b, div[style*="line"]');
                var snippet = snippetEl ? snippetEl.innerText : '';
                if (!snippet) {
                    // Try getting text from the result block minus the title
                    var allText = el.innerText || '';
                    var parts = allText.split('\n').filter(function(l) { return l.length > 20 && l !== title; });
                    snippet = parts.slice(0, 2).join(' ');
                }
                results.push(title + '\n' + url + '\n' + snippet.substring(0, 200));
            }
            if (results.length === 0) {
                // Fallback: just get page text
                return 'NO_STRUCTURED_RESULTS\n' + (document.body ? document.body.innerText.substring(0, 4000) : '');
            }
            return results.join('\n---\n');
        })()
    "#;

    let raw = eval_js(&mut ws, js).unwrap_or_default();

    // Check for CAPTCHA on Google search
    let page_title = eval_js(&mut ws, "document.title").unwrap_or_default();
    if let Some(captcha_msg) = detect_captcha(&page_title, &raw, "") {
        cleanup_after_data_extraction(&mut ws);
        tracing::info!("Google CAPTCHA detected, falling back to DuckDuckGo: {captcha_msg}");
        return duckduckgo_html_search(query);
    }

    if raw.starts_with("NO_STRUCTURED_RESULTS") {
        // Fallback to plain text results
        let text = raw.strip_prefix("NO_STRUCTURED_RESULTS\n").unwrap_or(&raw);
        let truncated = if text.len() > 4000 { &text[..text.floor_char_boundary(4000)] } else { text };
        cleanup_after_data_extraction(&mut ws);
        return format!("Search results for: {query}\n\n{truncated}");
    }

    // Parse structured results
    let mut output = format!("Search results for: {query}\n\n");
    for (i, block) in raw.split("\n---\n").enumerate() {
        let lines: Vec<&str> = block.lines().collect();
        if lines.len() >= 2 {
            let title = lines[0];
            let url = lines[1];
            let snippet = if lines.len() > 2 { lines[2..].join(" ") } else { String::new() };
            output.push_str(&format!("{}. {}\n   {}\n   {}\n\n", i + 1, title, url, snippet));
        }
    }

    if output.len() > 5000 {
        let boundary = output.floor_char_boundary(5000);
        output.truncate(boundary);
        output.push_str("\n... (truncated)");
    }

    // Clear page so stale content isn't mistaken for user activity
    cleanup_after_data_extraction(&mut ws);

    output
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    /// Runs `search_with` and says which services were reached.
    fn reached(route: Route, searxng_up: bool) -> (String, [bool; 3]) {
        let hit = [Cell::new(false), Cell::new(false), Cell::new(false)];
        let out = search_with(
            "rust news",
            route,
            |_| {
                hit[0].set(true);
                if searxng_up { Ok("searx".into()) } else { Err("down".into()) }
            },
            |_| {
                hit[1].set(true);
                "ddg".into()
            },
            |_| {
                hit[2].set(true);
                "google".into()
            },
        );
        (out, [hit[0].get(), hit[1].get(), hit[2].get()])
    }

    #[test]
    fn who_asked_and_whether_research_is_on_pick_the_route() {
        assert_eq!(route(false, false), Route::Refused, "off, nobody asked");
        assert_eq!(route(false, true), Route::Quiet, "on, nobody asked");
        assert_eq!(route(true, false), Route::Browser, "a person asked: research being off does not hold it back");
        assert_eq!(route(true, true), Route::Browser);
    }

    #[test]
    fn research_off_and_nobody_asking_runs_no_search() {
        let (out, hit) = reached(Route::Refused, true);
        assert_eq!(out, REFUSED);
        assert_eq!(hit, [false, false, false], "not even SearXNG is asked");
        // And the tool itself, on this thread, with no person's turn marked and the switch unset.
        assert!(!background_research::person_started() && !background_research::is_on());
        assert_eq!(web_search("anything"), REFUSED);
    }

    #[test]
    fn on_but_unattended_never_uses_chromium() {
        let (out, hit) = reached(Route::Quiet, false);
        assert_eq!(out, "ddg");
        assert_eq!(hit, [true, true, false], "SearXNG, then DuckDuckGo; the browser is never started");
        let (out, hit) = reached(Route::Quiet, true);
        assert_eq!(out, "searx");
        assert_eq!(hit, [true, false, false]);
    }

    #[test]
    fn a_turn_the_person_started_may_use_the_browser() {
        let (out, hit) = reached(Route::Browser, false);
        assert_eq!(out, "google");
        assert_eq!(hit, [true, false, true]);
        let _turn = background_research::person_turn();
        assert_eq!(route(background_research::person_started(), false), Route::Browser);
    }

    /// The tool goes through `web_search` and nothing else, and the two launches carry both flags.
    #[test]
    fn the_tool_has_one_way_in_and_chromium_starts_without_phoning_home() {
        let browser = include_str!("../browser.rs").split("#[cfg(test)]").next().unwrap();
        assert!(browser.contains("search::web_search(query)"));
        assert_eq!(browser.matches("--disable-background-networking").count(), 2, "both launches");
        assert_eq!(browser.matches("--disable-component-update").count(), 2, "both launches");
        let here = include_str!("search.rs").split("#[cfg(test)]").next().unwrap();
        let (before_google, google) = here.split_once("fn google_search").unwrap();
        assert!(!before_google.contains("ensure_headless_browser()"), "only google_search may start the browser");
        assert!(google.contains("ensure_headless_browser()"));
    }
}
