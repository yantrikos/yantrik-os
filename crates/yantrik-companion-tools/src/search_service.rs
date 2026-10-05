//! Which service `web_search` asks, from the person's one setting (crates/yantrik-web-search).
//!
//! Settings → Network → Web search decides it. `SEARXNG_URL` is still read, as an explicit
//! override for tests, and goes through the same address check. With built-in chosen nothing is
//! tried on this machine's port 8888 or anywhere else: the configured address is the only one.
//! When the configured SearXNG fails, the search falls back to DuckDuckGo and the result says so,
//! so the model and the person both know which service answered.

use yantrik_web_search::{client, Target, WebSearch};

/// What `web_search` should use now.
pub fn target() -> Target {
    target_from(std::env::var("SEARXNG_URL").ok(), yantrik_web_search::load())
}

/// [`target`], from what it reads: the override, if set, wins over the setting.
pub fn target_from(override_url: Option<String>, setting: WebSearch) -> Target {
    match override_url {
        Some(url) if !url.trim().is_empty() => match yantrik_web_search::check(&url) {
            Ok(u) => Target::Searxng(u),
            Err(why) => Target::Invalid { url, why },
        },
        _ => setting.target(),
    }
}

/// Search through the configured SearXNG. `None` when built-in is chosen: the caller searches its
/// own way. Otherwise the results, or `fallback`'s with a first line saying why SearXNG was not
/// used.
pub fn search(target: &Target, query: &str, fallback: &dyn Fn(&str) -> String) -> Option<String> {
    match target {
        Target::Builtin => None,
        Target::Searxng(url) => Some(match client::search(url, query) {
            Ok(hits) => {
                tracing::info!(count = hits.len().min(10), "SearXNG search");
                let mut out = format!("Search results for: {query}\n\n");
                for (i, h) in hits.iter().take(10).enumerate() {
                    out.push_str(&format!("{}. {}\n   {}\n   {}\n\n", i + 1, h.title, h.url, h.snippet));
                }
                out
            }
            Err(reason) => {
                tracing::info!(%reason, "SearXNG unavailable, used DuckDuckGo");
                format!("SearXNG at {} unavailable: {reason}; used DuckDuckGo.\n\n{}", url.url, fallback(query))
            }
        }),
        // Not echoed: an address that failed the check may hold a password.
        Target::Invalid { why, .. } => Some(format!(
            "SearXNG unavailable: the address saved in Settings is not valid ({why}); used DuckDuckGo.\n\n{}",
            fallback(query)
        )),
    }
}

/// What to do instead when a page refused the browser, in terms of the service the person set up.
pub fn advice() -> String {
    advice_for(&target())
}

pub fn advice_for(target: &Target) -> String {
    match target {
        Target::Searxng(url) => format!(
            "Use web_search(query=\"...\") instead: it asks the person's own SearXNG at {}, which is not blocked like this.",
            url.url
        ),
        _ => "Use web_search(query=\"...\") instead: it searches DuckDuckGo's plain HTML page, without a browser.".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use yantrik_web_search::Service;

    fn searxng(url: &str) -> WebSearch {
        WebSearch { service: Service::Searxng, searxng_url: url.into(), saved_at: String::new() }
    }

    /// A SearXNG on a loopback port that answers one search; the request line it was asked.
    fn stub(body: &'static str) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let handle = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let n = s.read(&mut buf).unwrap_or(0);
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            s.write_all(reply.as_bytes()).unwrap();
            String::from_utf8_lossy(&buf[..n]).lines().next().unwrap_or_default().to_string()
        });
        (url, handle)
    }

    #[test]
    fn the_configured_searxng_is_the_one_asked() {
        let (url, server) = stub(r#"{"results":[{"title":"Kite","url":"https://example.org/kite","content":"red","engines":["brave"]}]}"#);
        let target = target_from(None, searxng(&url));
        let ddg = Cell::new(0);
        let out = search(&target, "red kite", &|_| {
            ddg.set(ddg.get() + 1);
            String::new()
        })
        .unwrap();
        assert!(server.join().unwrap().starts_with("GET /search?q=red+kite&format=json"));
        assert_eq!(ddg.get(), 0);
        assert_eq!(out, "Search results for: red kite\n\n1. Kite\n   https://example.org/kite\n   red\n\n");
    }

    #[test]
    fn a_failing_searxng_falls_back_to_duckduckgo_and_says_so() {
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let url = format!("http://127.0.0.1:{port}");
        let out = search(&target_from(None, searxng(&url)), "q", &|q| format!("DDG RESULTS for {q}")).unwrap();
        assert!(
            out.starts_with(&format!("SearXNG at {url} unavailable: Not reachable: nothing is listening at 127.0.0.1:{port}; used DuckDuckGo.")),
            "{out}"
        );
        assert!(out.ends_with("DDG RESULTS for q"), "{out}");
    }

    #[test]
    fn builtin_asks_no_searxng_at_all() {
        let builtin = WebSearch { service: Service::Builtin, searxng_url: "http://127.0.0.1:8888".into(), saved_at: String::new() };
        let target = target_from(None, builtin);
        assert_eq!(target, Target::Builtin);
        assert_eq!(search(&target, "q", &|_| panic!("built-in is the caller's own search")), None);
        assert_eq!(target_from(None, WebSearch::default()), Target::Builtin, "no setting is built-in, not localhost:8888");
        // The old default is gone from the tool's source, so nothing can reach for it again.
        let browser = include_str!("browser.rs");
        assert!(!browser.contains("8888") && !browser.contains("SEARXNG_URL"), "browser.rs reads the setting through this module only");
    }

    #[test]
    fn the_env_override_wins_and_is_checked_the_same_way() {
        let t = target_from(Some("http://127.0.0.1:9999/".into()), WebSearch::default());
        assert!(matches!(t, Target::Searxng(ref u) if u.url == "http://127.0.0.1:9999"));
        let t = target_from(Some("http://search.example.com".into()), WebSearch::default());
        assert!(matches!(t, Target::Invalid { .. }));
        let out = search(&t, "q", &|_| "ddg".into()).unwrap();
        assert!(out.starts_with("SearXNG unavailable: the address saved in Settings is not valid") && out.ends_with("ddg"), "{out}");
    }

    #[test]
    fn advice_names_the_service_the_person_set_up() {
        let t = target_from(None, searxng("http://192.168.4.42:8888"));
        assert!(advice_for(&t).contains("SearXNG at http://192.168.4.42:8888"));
        let builtin = advice_for(&Target::Builtin);
        assert!(builtin.contains("DuckDuckGo") && !builtin.contains("Bing"));
    }
}
