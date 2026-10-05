//! The one client that asks a SearXNG: Settings' Test button and the companion's `web_search`
//! both come through [`fetch`], so what Test reports is what a search will meet.
//!
//! `GET <url>/search?q=…&format=json`, redirects not followed (a redirect would send the query
//! somewhere the person did not check), no proxy, and a bounded body. Every way it can fail is a
//! [`Failure`] whose text says what to change.

use std::io::Read;
use std::time::Duration;

use serde::Deserialize;

use crate::SearxUrl;

/// How long Test waits, and how long a search waits before falling back.
pub const TIMEOUT: Duration = Duration::from_secs(10);

/// The most of an answer that is read. A results page is tens of KiB.
const MOST_BODY: u64 = 2 * 1024 * 1024;

/// One result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// What one search brought back.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Page {
    pub hits: Vec<Hit>,
    /// Engines that contributed a result, sorted.
    pub answered: Vec<String>,
    /// SearXNG's `unresponsive_engines`: `(engine, why)`, as it reported them.
    pub unresponsive: Vec<(String, String)>,
}

/// Why a search did not come back with a page. `Display` is the sentence for the screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// Nothing answered: refused, no such name, timed out, TLS refused.
    Unreachable(String),
    /// A web server answered, but not with SearXNG's JSON.
    NoJson,
    /// SearXNG's limiter turned this machine away.
    Limited,
    /// It redirected; the address it pointed at, if it said.
    Redirected(String),
    /// Some other HTTP status.
    Status(u16),
    /// No `/search` there.
    NotFound,
    /// The answer was longer than [`MOST_BODY`]: not read past it, and not used.
    TooLarge,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failure::Unreachable(why) => write!(f, "Not reachable: {why}."),
            Failure::NoJson => write!(
                f,
                "This SearXNG doesn't serve JSON. Add `json` under search.formats in its settings.yml, then restart it."
            ),
            Failure::Limited => write!(
                f,
                "SearXNG's limiter turned this machine away (HTTP 429). Add this machine's address under pass_ip in its limiter.toml, or set server.limiter to false in settings.yml."
            ),
            Failure::Redirected(to) if to.is_empty() => write!(f, "It redirected somewhere else. Enter the final address instead."),
            Failure::Redirected(to) => write!(f, "It redirected to {to}. Enter that address instead."),
            Failure::Status(code) => write!(f, "SearXNG answered with HTTP {code}."),
            Failure::NotFound => write!(
                f,
                "A web server answered, but there is no SearXNG search there (HTTP 404). Check the address, including any path such as /searxng."
            ),
            Failure::TooLarge => write!(f, "The answer was larger than {} MiB, far more than a results page.", MOST_BODY / (1024 * 1024)),
        }
    }
}

/// Ask `base` for `query`. A failure names the address, for the person's screen.
pub fn fetch(base: &SearxUrl, query: &str, timeout: Duration) -> Result<Page, Failure> {
    fetch_as(base, query, timeout, true)
}

/// [`fetch`]; `named` false says "the SearXNG" where the address would be, for what a model reads
/// (docs/harness.md: the address is configuration, not something to show the model).
fn fetch_as(base: &SearxUrl, query: &str, timeout: Duration, named: bool) -> Result<Page, Failure> {
    let agent = ureq::AgentBuilder::new().timeout(timeout).redirects(0).build();
    let url = format!("{}/search", base.url);
    let answer = agent
        .get(&url)
        .query("q", query)
        .query("format", "json")
        .query("pageno", "1")
        .set("Accept", "application/json")
        .call();
    let response = match answer {
        Ok(r) => r,
        Err(ureq::Error::Status(code, r)) => return Err(status_failure(code, &r)),
        Err(ureq::Error::Transport(t)) => return Err(Failure::Unreachable(transport_reason(&t, base, timeout, named))),
    };
    if (300..400).contains(&response.status()) {
        return Err(Failure::Redirected(response.header("Location").unwrap_or_default().chars().take(200).collect()));
    }
    let content_type = response.content_type().to_string();
    let mut body = Vec::new();
    // One byte past the cap is read, to tell an answer that fits from one that was cut.
    response
        .into_reader()
        .take(MOST_BODY + 1)
        .read_to_end(&mut body)
        .map_err(|e| Failure::Unreachable(format!("the answer broke off ({e})")))?;
    if body.len() as u64 > MOST_BODY {
        return Err(Failure::TooLarge);
    }
    parse(&content_type, &String::from_utf8_lossy(&body))
}

fn status_failure(code: u16, response: &ureq::Response) -> Failure {
    match code {
        404 => Failure::NotFound,
        429 => Failure::Limited,
        // SearXNG answers 403 to a format it does not serve.
        403 if response.content_type() != "application/json" => Failure::NoJson,
        _ => Failure::Status(code),
    }
}

fn transport_reason(t: &ureq::Transport, base: &SearxUrl, timeout: Duration, named: bool) -> String {
    let (at, name) = if named {
        (format!("{}:{}", base.host, base.port), format!("the name {}", base.host))
    } else {
        ("the SearXNG".to_string(), "the SearXNG's host name".to_string())
    };
    let message = t.message().unwrap_or_default().to_string();
    let source = std::error::Error::source(t).map(|s| s.to_string()).unwrap_or_default();
    let said = format!("{message} {source}").to_ascii_lowercase();
    match t.kind() {
        ureq::ErrorKind::Dns => format!("{name} does not resolve"),
        _ if said.contains("timed out") || said.contains("would block") => {
            format!("no answer from {at} within {} seconds", timeout.as_secs())
        }
        ureq::ErrorKind::ConnectionFailed if said.contains("refused") => format!("nothing is listening at {at}"),
        ureq::ErrorKind::ConnectionFailed if said.contains("certificate") || said.contains("tls") => {
            format!("its TLS certificate was not accepted ({})", message.trim())
        }
        ureq::ErrorKind::ConnectionFailed => format!("could not connect to {at}"),
        _ if said.contains("certificate") || said.contains("tls") => {
            format!("its TLS certificate was not accepted ({})", message.trim())
        }
        _ => format!("{at} did not answer as a web server ({})", t.kind()),
    }
}

/// Read an answer. Split from [`fetch`] so every shape of answer can be tested without a server.
pub fn parse(content_type: &str, body: &str) -> Result<Page, Failure> {
    #[derive(Deserialize)]
    struct Answer {
        results: Vec<serde_json::Value>,
        #[serde(default)]
        unresponsive_engines: Vec<serde_json::Value>,
    }
    if content_type.contains("html") {
        return Err(Failure::NoJson);
    }
    let answer: Answer = serde_json::from_str(body).map_err(|_| Failure::NoJson)?;
    let text = |v: &serde_json::Value, k: &str| v.get(k).and_then(|s| s.as_str()).unwrap_or_default().to_string();
    let mut answered = std::collections::BTreeSet::new();
    let mut hits = Vec::new();
    for r in &answer.results {
        if let Some(engines) = r.get("engines").and_then(|e| e.as_array()) {
            answered.extend(engines.iter().filter_map(|e| e.as_str()).map(str::to_string));
        } else if let Some(engine) = r.get("engine").and_then(|e| e.as_str()) {
            answered.insert(engine.to_string());
        }
        let hit = Hit { title: text(r, "title"), url: text(r, "url"), snippet: text(r, "content") };
        if !hit.url.is_empty() {
            hits.push(hit);
        }
    }
    // `[["google", "CAPTCHA"], ["brave", "too many requests"]]`.
    let unresponsive = answer
        .unresponsive_engines
        .iter()
        .filter_map(|pair| {
            let pair = pair.as_array()?;
            let name = pair.first()?.as_str()?.to_string();
            let why = pair.get(1).and_then(|w| w.as_str()).unwrap_or("no answer").to_string();
            Some((name, why))
        })
        .collect();
    Ok(Page { hits, answered: answered.into_iter().collect(), unresponsive })
}

/// What Settings' Test button reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Probe {
    /// At least one result came back: the address may be saved.
    pub ok: bool,
    /// One or two sentences for the screen.
    pub summary: String,
    pub results: usize,
    pub answered: Vec<String>,
    pub unresponsive: Vec<(String, String)>,
}

/// Search `base` for "yantrik" and say what came back.
pub fn probe(base: &SearxUrl, timeout: Duration) -> Probe {
    judge(fetch(base, "yantrik", timeout))
}

/// The report for one test search's outcome.
pub fn judge(outcome: Result<Page, Failure>) -> Probe {
    let page = match outcome {
        Ok(page) => page,
        Err(failure) => {
            return Probe { ok: false, summary: failure.to_string(), results: 0, answered: vec![], unresponsive: vec![] }
        }
    };
    let blocked = || {
        page.unresponsive.iter().map(|(n, why)| format!("{n} ({why})")).collect::<Vec<_>>().join(", ")
    };
    let summary = if !page.hits.is_empty() {
        let mut s = format!(
            "{} result{} from {}.",
            page.hits.len(),
            if page.hits.len() == 1 { "" } else { "s" },
            if page.answered.is_empty() { "SearXNG".to_string() } else { page.answered.join(", ") }
        );
        if !page.unresponsive.is_empty() {
            s.push_str(&format!(" Unresponsive: {}.", blocked()));
        }
        s
    } else if !page.unresponsive.is_empty() {
        format!("Reached it, but every engine is blocked or rate-limited: {}.", blocked())
    } else {
        "Reached it, but the test search found nothing. Check that at least one engine is enabled in its settings.yml.".to_string()
    };
    Probe { ok: !page.hits.is_empty(), summary, results: page.hits.len(), answered: page.answered, unresponsive: page.unresponsive }
}

/// Search for the companion's `web_search`: the page, or why there is none to use. A page with no
/// results is a failure here, with SearXNG's own reasons, so the caller falls back. The reason is
/// for the model, so it never names the address.
pub fn search(base: &SearxUrl, query: &str) -> Result<Vec<Hit>, String> {
    match fetch_as(base, query, TIMEOUT, false) {
        Ok(page) if !page.hits.is_empty() => Ok(page.hits),
        Ok(page) if !page.unresponsive.is_empty() => Err(format!(
            "no results; unresponsive engines: {}",
            page.unresponsive.iter().map(|(n, w)| format!("{n} ({w})")).collect::<Vec<_>>().join(", ")
        )),
        Ok(_) => Err("no results".to_string()),
        Err(f) => Err(f.to_string().trim_end_matches('.').to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::TcpListener;

    const GOOD: &str = r#"{"query":"yantrik","results":[
        {"title":"Yantrik OS","url":"https://example.org/a","content":"A desktop","engines":["duckduckgo","brave"]},
        {"title":"Yantrik","url":"https://example.org/b","content":"Two","engine":"wikipedia"}
      ],"unresponsive_engines":[["google","CAPTCHA"]]}"#;

    /// One canned HTTP answer on a loopback port, for one request. Returns the address and the
    /// request line it was asked, once the thread is joined.
    fn stub(status: &str, content_type: &str, body: &str) -> (SearxUrl, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let reply = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let handle = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let n = std::io::Read::read(&mut s, &mut buf).unwrap_or(0);
            s.write_all(reply.as_bytes()).unwrap();
            String::from_utf8_lossy(&buf[..n]).lines().next().unwrap_or_default().to_string()
        });
        (crate::check(&format!("http://127.0.0.1:{port}/")).unwrap(), handle)
    }

    #[test]
    fn a_good_answer_counts_results_and_names_the_engines() {
        let (url, server) = stub("200 OK", "application/json", GOOD);
        let p = probe(&url, Duration::from_secs(5));
        let asked = server.join().unwrap();
        assert!(asked.starts_with("GET /search?q=yantrik&format=json"), "{asked}");
        assert!(p.ok);
        assert_eq!(p.results, 2);
        assert_eq!(p.answered, vec!["brave", "duckduckgo", "wikipedia"]);
        assert_eq!(p.unresponsive, vec![("google".to_string(), "CAPTCHA".to_string())]);
        assert_eq!(p.summary, "2 results from brave, duckduckgo, wikipedia. Unresponsive: google (CAPTCHA).");
    }

    #[test]
    fn html_only_says_how_to_turn_json_on() {
        let (url, server) = stub("200 OK", "text/html; charset=utf-8", "<html><body>SearXNG</body></html>");
        let p = probe(&url, Duration::from_secs(5));
        server.join().unwrap();
        assert!(!p.ok);
        assert!(p.summary.contains("doesn't serve JSON") && p.summary.contains("search.formats"), "{}", p.summary);
        // SearXNG's own refusal of a format it does not serve.
        let (url, server) = stub("403 FORBIDDEN", "text/html", "<html>Forbidden</html>");
        let p = probe(&url, Duration::from_secs(5));
        server.join().unwrap();
        assert!(p.summary.contains("search.formats"), "{}", p.summary);
    }

    #[test]
    fn every_engine_unresponsive_is_named() {
        let body = r#"{"results":[],"unresponsive_engines":[["google","CAPTCHA"],["duckduckgo","too many requests"]]}"#;
        let (url, server) = stub("200 OK", "application/json", body);
        let p = probe(&url, Duration::from_secs(5));
        server.join().unwrap();
        assert!(!p.ok);
        assert_eq!(
            p.summary,
            "Reached it, but every engine is blocked or rate-limited: google (CAPTCHA), duckduckgo (too many requests)."
        );
    }

    #[test]
    fn unreachable_says_so() {
        let port = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let url = crate::check(&format!("http://127.0.0.1:{port}")).unwrap();
        let p = probe(&url, Duration::from_secs(5));
        assert!(!p.ok);
        assert!(p.summary.starts_with("Not reachable"), "{}", p.summary);
        assert!(p.summary.contains(&format!("127.0.0.1:{port}")), "{}", p.summary);
    }

    #[test]
    fn a_redirect_is_not_followed() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = [0u8; 2048];
            let _ = std::io::Read::read(&mut s, &mut buf);
            s.write_all(b"HTTP/1.1 302 Found\r\nLocation: https://elsewhere.example/search\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        });
        let url = crate::check(&format!("http://127.0.0.1:{port}")).unwrap();
        let p = probe(&url, Duration::from_secs(5));
        server.join().unwrap();
        assert!(!p.ok);
        assert!(p.summary.contains("https://elsewhere.example/search"), "{}", p.summary);
    }

    #[test]
    fn other_shapes_parse_as_they_should() {
        assert_eq!(parse("application/json", "{}"), Err(Failure::NoJson));
        assert_eq!(parse("application/json", "not json"), Err(Failure::NoJson));
        let page = parse("application/json", r#"{"results":[]}"#).unwrap();
        assert!(judge(Ok(page)).summary.contains("found nothing"));
        let (url, server) = stub("429 Too Many Requests", "text/plain", "Too Many Requests");
        let p = probe(&url, Duration::from_secs(5));
        server.join().unwrap();
        assert!(p.summary.contains("limiter"), "{}", p.summary);
    }

    #[test]
    fn search_hands_back_hits_or_a_reason() {
        let (url, server) = stub("200 OK", "application/json", GOOD);
        let hits = search(&url, "rust async").unwrap();
        assert!(server.join().unwrap().starts_with("GET /search?q=rust+async&format=json"));
        assert_eq!(hits[0], Hit { title: "Yantrik OS".into(), url: "https://example.org/a".into(), snippet: "A desktop".into() });
        let (url, server) = stub("200 OK", "application/json", r#"{"results":[],"unresponsive_engines":[["google","CAPTCHA"]]}"#);
        assert_eq!(search(&url, "x").unwrap_err(), "no results; unresponsive engines: google (CAPTCHA)");
        server.join().unwrap();
    }

    /// An answer is read up to the cap and no further, and one past it is not used: a stub that
    /// streams more than the cap of valid JSON gets no result through.
    #[test]
    fn an_answer_past_the_size_cap_is_refused() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = [0u8; 2048];
            let _ = std::io::Read::read(&mut s, &mut buf);
            let one = r#"{"title":"t","url":"https://example.org/","content":"c"},"#;
            let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"results\":[");
            let mut sent = 0u64;
            // Past the cap, then a well-formed end: only the cap keeps it out.
            while sent <= MOST_BODY {
                if s.write_all(one.as_bytes()).is_err() {
                    return;
                }
                sent += one.len() as u64;
            }
            let _ = s.write_all(one.trim_end_matches(',').as_bytes());
            let _ = s.write_all(b"]}");
        });
        let url = crate::check(&format!("http://127.0.0.1:{port}")).unwrap();
        let p = probe(&url, Duration::from_secs(10));
        let _ = server.join();
        assert!(!p.ok, "{}", p.summary);
        assert_eq!(p.summary, "The answer was larger than 2 MiB, far more than a results page.");
    }

    /// What a model reads about a failed search says "the SearXNG", never the address.
    #[test]
    fn a_failed_search_does_not_name_the_address() {
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let url = crate::check(&format!("http://127.0.0.1:{port}")).unwrap();
        let why = search(&url, "q").unwrap_err();
        assert_eq!(why, "Not reachable: nothing is listening at the SearXNG");
        assert!(probe(&url, Duration::from_secs(5)).summary.contains(&format!("127.0.0.1:{port}")), "the person's screen names it");
    }
}
