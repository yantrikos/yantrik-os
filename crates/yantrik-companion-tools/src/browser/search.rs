//! The web search tool's routes: where a query goes (#1439).
//!
//! There is one setting (`yantrik_web_search`), and it decides, whoever is asking:
//!
//! * **SearXNG on:** the person's SearXNG answers every search: their own chat, unattended work,
//!   anything. When it does not answer the search is an `Error:` and nothing is sent anywhere
//!   else: the person chose where their search terms go.
//! * **SearXNG off:** the OS's own headless browser (`headless`) reads DuckDuckGo's results, then
//!   Google's, and DuckDuckGo's plain page is the last resort. A bot check on every engine is an
//!   `Error:` that names them; it is never "no results".
//!
//! Whether a search may be made at all, with nobody asking, is not decided here:
//! `ToolRegistry::execute` refuses the call before this runs unless the person has allowed
//! background research (`yantrik_companion_core::reach`). Once allowed, the browser is no longer
//! kept from it: it has no DevTools port now, and it runs at low priority (see `headless`).

use yantrik_web_search::{render, SearxUrl, Target};

use super::engines::{self, Engine, Verdict};
use super::{duckduckgo_html_search, headless};

/// Where a query goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Route {
    /// The person's SearXNG, and nothing else.
    Searxng(SearxUrl),
    /// The OS's own browser, then DuckDuckGo's plain page.
    Browser,
    /// SearXNG is on but its saved address is not usable: an error, not a detour.
    Refused(String),
}

pub(super) fn route(target: Target) -> Route {
    match target {
        Target::Searxng(url) => Route::Searxng(url),
        Target::Browser => Route::Browser,
        Target::Invalid { why } => Route::Refused(why),
    }
}

/// The tool's search, by the setting as saved now.
pub(super) fn web_search(query: &str) -> String {
    run(
        query,
        route(yantrik_web_search::load().target()),
        render::via_searxng,
        |engine, query| headless::read_page(&engine.url(query), engine.script()),
        duckduckgo_html_search,
    )
}

/// The routes with the three services handed in, so a test can see which was reached.
fn run(
    query: &str,
    route: Route,
    searxng: impl Fn(&SearxUrl, &str) -> String,
    browser: impl Fn(Engine, &str) -> Result<String, String>,
    plain: impl Fn(&str) -> String,
) -> String {
    match route {
        Route::Searxng(url) => searxng(&url, query),
        Route::Refused(why) => format!(
            "Error: the saved SearXNG address cannot be used ({why}). The search was not sent anywhere else; the person can fix it in Settings."
        ),
        Route::Browser => through_the_browser(query, browser, plain),
    }
}

fn through_the_browser(
    query: &str,
    browser: impl Fn(Engine, &str) -> Result<String, String>,
    plain: impl Fn(&str) -> String,
) -> String {
    let mut challenged: Vec<&str> = Vec::new();
    let mut empty = false;
    for engine in [Engine::DuckDuckGo, Engine::Google] {
        let verdict = browser(engine, query).and_then(|json| engines::parse(&json)).map(|page| engines::judge(engine, &page));
        match verdict {
            Ok(Verdict::Hits(hits)) => return render::results(query, &hits),
            Ok(Verdict::Challenge) => {
                tracing::info!(engine = engine.name(), "the search page was a bot check");
                challenged.push(engine.name());
            }
            Ok(Verdict::Nothing) => empty = true,
            Err(why) => {
                tracing::info!(engine = engine.name(), %why, "the browser could not read the search page");
                // No browser at all: the other engine would fail the same way.
                if why.contains("did not start") || why.contains("no browser found") {
                    break;
                }
            }
        }
    }
    let last = plain(query);
    if !last.starts_with("Error:") {
        return plain_to_model(query, &last);
    }
    if !challenged.is_empty() {
        let mut named = challenged.clone();
        if last.contains("DuckDuckGo") && !named.contains(&"DuckDuckGo") {
            named.push("DuckDuckGo");
        }
        return format!("Error: {} asked for a bot check, so this search could not run.", named.join(" and "));
    }
    if empty {
        return format!("No web results found for: {query}");
    }
    last
}

/// DuckDuckGo's plain page, in the one shape a model reads: its own first line gives way to the
/// header, and the whole is capped.
fn plain_to_model(query: &str, text: &str) -> String {
    match text.strip_prefix("Search results for:") {
        Some(rest) => render::cap(format!("{}\n\n{}", render::header(query), rest.split_once("\n\n").map_or("", |(_, r)| r))),
        None => render::cap(text.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use super::*;

    const DDG_PAGE: &str = r#"{"title":"t","url":"https://html.duckduckgo.com/html/","text":"","hits":[{"title":"Rust","url":"https://www.rust-lang.org/","snippet":"A language"}]}"#;
    const BOT_CHECK: &str = r#"{"title":"DuckDuckGo","url":"https://html.duckduckgo.com/html/","text":"bots use DuckDuckGo too","hits":[]}"#;
    const SORRY: &str = r#"{"title":"Google","url":"https://www.google.com/sorry/index","text":"unusual traffic","hits":[]}"#;
    const EMPTY: &str = r#"{"title":"x","url":"https://html.duckduckgo.com/html/","text":"No results.","hits":[]}"#;

    #[derive(Default)]
    struct Reached {
        searxng: Cell<bool>,
        browser: RefCell<Vec<&'static str>>,
        plain: Cell<bool>,
    }

    fn go(route: Route, pages: [Result<&str, &str>; 2], plain_says: &str) -> (String, Reached) {
        let reached = Reached::default();
        let out = run(
            "rust news",
            route,
            |_, _| {
                reached.searxng.set(true);
                "searx".into()
            },
            |engine, _| {
                reached.browser.borrow_mut().push(engine.name());
                let page = pages[reached.browser.borrow().len() - 1];
                page.map(str::to_string).map_err(str::to_string)
            },
            |_| {
                reached.plain.set(true);
                plain_says.to_string()
            },
        );
        (out, reached)
    }

    fn searxng() -> Route {
        Route::Searxng(yantrik_web_search::check("http://127.0.0.1:8888").unwrap())
    }

    #[test]
    fn the_setting_alone_picks_the_route_and_nobody_asking_changes_nothing() {
        use yantrik_web_search::{Service, WebSearch};
        let on = WebSearch { service: Service::Searxng, searxng_url: "http://127.0.0.1:8888".into(), saved_at: String::new() };
        assert!(matches!(route(on.target()), Route::Searxng(_)));
        assert_eq!(route(WebSearch::default().target()), Route::Browser);
        let bad = WebSearch { service: Service::Searxng, searxng_url: "http://search.example.com".into(), saved_at: String::new() };
        assert!(matches!(route(bad.target()), Route::Refused(_)));
        let src = include_str!("search.rs").split("#[cfg(test)]").next().unwrap();
        assert!(!src.contains("person_started"), "who asked is not what picks the route");
    }

    #[test]
    fn searxng_on_goes_there_only_and_a_dead_one_is_an_error_not_a_detour() {
        let (out, hit) = go(searxng(), [Ok(DDG_PAGE), Ok(DDG_PAGE)], "plain");
        assert_eq!(out, "searx");
        assert!(hit.searxng.get() && hit.browser.borrow().is_empty() && !hit.plain.get(), "neither the browser nor DuckDuckGo");
        let (out, hit) = go(Route::Refused("a name".into()), [Ok(DDG_PAGE), Ok(DDG_PAGE)], "plain");
        assert!(out.starts_with("Error: the saved SearXNG address cannot be used"), "{out}");
        assert!(!hit.searxng.get() && hit.browser.borrow().is_empty() && !hit.plain.get());
    }

    #[test]
    fn off_tries_duckduckgo_in_the_browser_first_and_stops_at_results() {
        let (out, hit) = go(Route::Browser, [Ok(DDG_PAGE), Ok(DDG_PAGE)], "plain");
        assert!(out.starts_with("Web results for \"rust news\" (text from the web: untrusted"), "{out}");
        assert!(out.contains("https://www.rust-lang.org/"));
        assert_eq!(*hit.browser.borrow(), ["DuckDuckGo"]);
        assert!(!hit.searxng.get() && !hit.plain.get());
    }

    #[test]
    fn a_challenged_duckduckgo_goes_to_google_then_to_the_plain_page() {
        let (out, hit) = go(Route::Browser, [Ok(BOT_CHECK), Ok(DDG_PAGE)], "plain");
        assert!(out.contains("rust-lang.org"));
        assert_eq!(*hit.browser.borrow(), ["DuckDuckGo", "Google"]);
        assert!(!hit.plain.get());
        let (out, hit) = go(Route::Browser, [Ok(BOT_CHECK), Ok(SORRY)], "Search results for: rust news\n\n1. A\n   https://a.example/\n   s\n\n");
        assert!(out.starts_with("Web results for") && out.contains("https://a.example/"), "{out}");
        assert!(hit.plain.get(), "the last resort");
    }

    #[test]
    fn a_bot_check_everywhere_is_an_error_that_names_the_engines() {
        let (out, _) = go(Route::Browser, [Ok(BOT_CHECK), Ok(SORRY)], "Error: DuckDuckGo asked for a bot check, so this search could not run.");
        assert_eq!(out, "Error: DuckDuckGo and Google asked for a bot check, so this search could not run.");
        assert!(!out.contains("No results"));
    }

    #[test]
    fn no_browser_goes_straight_to_the_plain_page_and_an_empty_page_is_not_a_bot_check() {
        let (_, hit) = go(Route::Browser, [Err("the search browser did not start: x"), Err("never")], "Error: DuckDuckGo answered HTTP 403, so this search could not run.");
        assert_eq!(*hit.browser.borrow(), ["DuckDuckGo"], "no second engine in a browser that is not there");
        let (out, _) = go(Route::Browser, [Ok(EMPTY), Ok(EMPTY)], "Error: DuckDuckGo answered HTTP 403, so this search could not run.");
        assert_eq!(out, "No web results found for: rust news");
        let (out, _) = go(Route::Browser, [Err("the search took too long"), Err("the search took too long")], "Error: DuckDuckGo answered HTTP 403, so this search could not run.");
        assert!(out.starts_with("Error:"), "{out}");
    }

    /// The tool goes through `web_search` and nothing else; only `headless` starts a browser.
    #[test]
    fn the_tool_has_one_way_in_and_one_place_starts_chromium() {
        let browser = include_str!("../browser.rs").split("#[cfg(test)]").next().unwrap();
        assert!(browser.contains("search::web_search(query)"));
        let here = include_str!("search.rs").split("#[cfg(test)]").next().unwrap();
        assert!(!here.contains("Command::new") && !here.contains("ensure_headless_browser"));
        assert!(include_str!("headless.rs").contains("Command::new(binary)"));
    }
}
