//! The two engines the browser tries, and how a page of either is read (#1439).
//!
//! DuckDuckGo first, then Google. Each is a results page the headless browser loads and a
//! read-only script that lists what is on it; [`judge`] then says whether that was results, a
//! bot check, or nothing. No page here is ever clicked: a result's address is read, not followed.

use serde::Deserialize;
use yantrik_web_search::client::Hit;

use super::encode_query;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Engine {
    DuckDuckGo,
    Google,
}

impl Engine {
    /// How the engine is named to the person and in errors.
    pub(super) fn name(self) -> &'static str {
        match self {
            Engine::DuckDuckGo => "DuckDuckGo",
            Engine::Google => "Google",
        }
    }

    pub(super) fn url(self, query: &str) -> String {
        let q = encode_query(query);
        match self {
            Engine::DuckDuckGo => format!("https://html.duckduckgo.com/html/?q={q}"),
            Engine::Google => format!("https://www.google.com/search?q={q}&num=10&hl=en"),
        }
    }

    /// A read-only expression that evaluates to a JSON string: [`Page`].
    pub(super) fn script(self) -> &'static str {
        match self {
            Engine::DuckDuckGo => DDG_SCRIPT,
            Engine::Google => GOOGLE_SCRIPT,
        }
    }
}

const DDG_SCRIPT: &str = r#"(function () {
  var hits = [];
  document.querySelectorAll('.result, .web-result').forEach(function (el) {
    if (hits.length >= 10) return;
    var a = el.querySelector('a.result__a');
    if (!a) return;
    var href = a.getAttribute('href') || '';
    try {
      var u = new URL(href, location.href);
      if (u.searchParams.get('uddg')) href = u.searchParams.get('uddg'); else href = u.href;
    } catch (e) {}
    var s = el.querySelector('.result__snippet');
    hits.push({ title: a.innerText, url: href, snippet: s ? s.innerText : '' });
  });
  return JSON.stringify({ title: document.title, url: location.href, text: (document.body ? document.body.innerText : '').slice(0, 1500), hits: hits });
})()"#;

const GOOGLE_SCRIPT: &str = r#"(function () {
  var hits = [];
  document.querySelectorAll('div.g, div[data-hveid]').forEach(function (el) {
    if (hits.length >= 10) return;
    var a = el.querySelector('a[href^="http"]');
    var h = el.querySelector('h3');
    if (!a || !h || /google\.com/.test(a.hostname)) return;
    var s = el.querySelector('div[data-sncf], div.VwiC3b, span.aCOpRe');
    hits.push({ title: h.innerText, url: a.href, snippet: s ? s.innerText : '' });
  });
  return JSON.stringify({ title: document.title, url: location.href, text: (document.body ? document.body.innerText : '').slice(0, 1500), hits: hits });
})()"#;

#[derive(Debug, Deserialize)]
struct RawHit {
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    snippet: String,
}

/// What the script read off a page.
#[derive(Debug, Deserialize)]
pub(super) struct Page {
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    hits: Vec<RawHit>,
}

/// What a page was.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Verdict {
    Hits(Vec<Hit>),
    /// A bot check, a consent wall or a "sorry" page: the search did not run.
    Challenge,
    /// A results page with no results on it.
    Nothing,
}

pub(super) fn parse(json: &str) -> Result<Page, String> {
    serde_json::from_str(json).map_err(|_| "the results page could not be read".to_string())
}

/// What `page` is, for `engine`. A page with results is results whatever else it says; a page
/// without them is a bot check when it says so, and "no results" otherwise.
pub(super) fn judge(engine: Engine, page: &Page) -> Verdict {
    let hits: Vec<Hit> = page
        .hits
        .iter()
        .filter(|h| (h.url.starts_with("https://") || h.url.starts_with("http://")) && !h.title.trim().is_empty())
        .map(|h| Hit { title: h.title.trim().to_string(), url: h.url.clone(), snippet: h.snippet.trim().chars().take(200).collect() })
        .collect();
    if !hits.is_empty() {
        return Verdict::Hits(hits);
    }
    let said = format!("{} {}", page.title, page.text).to_lowercase();
    let challenged = said.contains("captcha")
        || said.contains("unusual traffic")
        || said.contains("not a robot")
        || said.contains("select all squares")
        || said.contains("verify you are human")
        || said.contains("before you continue")
        || page.url.contains("/sorry/")
        || page.url.contains("consent.google")
        || match engine {
            Engine::DuckDuckGo => said.contains("anomaly") || said.contains("bots use duckduckgo") || said.contains("select all"),
            Engine::Google => false,
        };
    if challenged {
        Verdict::Challenge
    } else {
        Verdict::Nothing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(title: &str, url: &str, text: &str, hits: &str) -> Page {
        parse(&format!(r#"{{"title":{title:?},"url":{url:?},"text":{text:?},"hits":{hits}}}"#)).unwrap()
    }

    #[test]
    fn engines_have_urls_and_the_query_is_encoded_once() {
        assert_eq!(Engine::DuckDuckGo.url("rust & go"), "https://html.duckduckgo.com/html/?q=rust+%26+go");
        assert_eq!(Engine::Google.url("a b"), "https://www.google.com/search?q=a+b&num=10&hl=en");
        assert!(Engine::DuckDuckGo.url("x").starts_with("https://") && Engine::Google.url("x").starts_with("https://"));
    }

    #[test]
    fn results_are_results_and_only_web_addresses_count() {
        let p = page(
            "rust at DuckDuckGo",
            "https://html.duckduckgo.com/html/?q=rust",
            "",
            r#"[{"title":" Rust ","url":"https://www.rust-lang.org/","snippet":" A language "},{"title":"x","url":"javascript:alert(1)","snippet":""},{"title":"","url":"https://e.org/","snippet":""}]"#,
        );
        match judge(Engine::DuckDuckGo, &p) {
            Verdict::Hits(h) => assert_eq!(h, vec![Hit { title: "Rust".into(), url: "https://www.rust-lang.org/".into(), snippet: "A language".into() }]),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_bot_check_is_a_challenge_and_an_empty_page_is_nothing() {
        let ddg = page("DuckDuckGo", "https://html.duckduckgo.com/html/", "Unfortunately, bots use DuckDuckGo too. Select all squares containing a duck.", "[]");
        assert_eq!(judge(Engine::DuckDuckGo, &ddg), Verdict::Challenge);
        let google = page("Google", "https://www.google.com/sorry/index?continue=x", "Our systems have detected unusual traffic", "[]");
        assert_eq!(judge(Engine::Google, &google), Verdict::Challenge);
        let consent = page("Before you continue to Google", "https://consent.google.com/m", "", "[]");
        assert_eq!(judge(Engine::Google, &consent), Verdict::Challenge);
        let none = page("zzzz at DuckDuckGo", "https://html.duckduckgo.com/html/?q=zzzz", "No results.", "[]");
        assert_eq!(judge(Engine::DuckDuckGo, &none), Verdict::Nothing);
        assert!(parse("not json").is_err());
    }

    /// The scripts read and never act.
    #[test]
    fn the_scripts_only_read() {
        for script in [DDG_SCRIPT, GOOGLE_SCRIPT] {
            for acting in [".click(", ".submit(", ".value =", "location.href =", "location.assign", "window.open", "fetch(", "XMLHttpRequest", "dispatchEvent", "focus("] {
                assert!(!script.contains(acting), "`{acting}` in a search script");
            }
        }
    }
}
