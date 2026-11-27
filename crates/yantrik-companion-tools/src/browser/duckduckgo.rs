//! DuckDuckGo as a keyless search backend: the `/html/` page, then Lite when that is challenged.
//!
//! DuckDuckGo answers a client it distrusts with HTTP 202 and a bot-check page. That is a failed
//! search, not an empty one, and it is reported as an `Error:` so no caller reads it as "the web
//! had nothing" (#1437).

use std::time::Duration;

const USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36";

const HTML_URL: &str = "https://html.duckduckgo.com/html/?q=";
const LITE_URL: &str = "https://lite.duckduckgo.com/lite/?q=";

/// What a search ends with when DuckDuckGo asks for a bot check on both pages.
pub(super) const CHALLENGE_ERROR: &str =
    "Error: DuckDuckGo asked for a bot check, so this search could not run.";

/// Percent-encode a query for a URL: spaces as `+`, every other byte outside the unreserved set as
/// `%XX` of its UTF-8 encoding.
pub(super) fn encode_query(query: &str) -> String {
    let mut out = String::with_capacity(query.len());
    for &b in query.as_bytes() {
        match b {
            b' ' => out.push('+'),
            b if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') => out.push(b as char),
            b => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Percent-decode (UTF-8) with `+` as a space.
fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let hex = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok());
                match hex {
                    Some(v) => {
                        out.push(v);
                        i += 3;
                    }
                    None => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Tag stripper with the common entities decoded.
fn strip_html_basic(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        if ch == '<' {
            in_tag = true;
        } else if ch == '>' {
            in_tag = false;
        } else if !in_tag {
            result.push(ch);
        }
    }
    result
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .trim()
        .to_string()
}

/// One page's answer: its status and body.
struct Page {
    status: u16,
    body: String,
}

fn fetch(url: &str) -> Result<Page, String> {
    let resp = match ureq::get(url)
        .set("User-Agent", USER_AGENT)
        .set("Accept", "text/html")
        .set("Accept-Language", "en-US,en;q=0.5")
        .timeout(Duration::from_secs(15))
        .call()
    {
        Ok(r) => r,
        // A 4xx/5xx still has a status and a body worth reading.
        Err(ureq::Error::Status(_, r)) => r,
        Err(e) => return Err(format!("Error: DuckDuckGo search failed: {e}")),
    };
    let status = resp.status();
    let body = resp
        .into_string()
        .map_err(|e| format!("Error: Failed to read DuckDuckGo response: {e}"))?;
    Ok(Page { status, body })
}

/// Whether DuckDuckGo answered with its bot check instead of results.
fn is_challenge(status: u16, body: &str) -> bool {
    status == 202
        || body.contains("anomaly-modal")
        || body.contains("bots use DuckDuckGo")
        || (body.contains("anomaly") && !body.contains("result__a") && !body.contains("result-link"))
}

type Hit = (String, String, String);

/// The real address behind a DuckDuckGo redirect (`//duckduckgo.com/l/?uddg=ENCODED&...`).
fn clean_url(href: &str) -> String {
    match href.split("uddg=").nth(1) {
        Some(rest) if href.contains("uddg=") => url_decode(rest.split('&').next().unwrap_or("")),
        _ => href.to_string(),
    }
}

/// The `href` of the anchor tag that contains `marker_abs`, whichever side of the class it is on.
fn tag_href(body: &str, marker_abs: usize) -> Option<&str> {
    let tag_start = body[..marker_abs].rfind('<')?;
    let tag_end = body[marker_abs..].find('>').map(|i| marker_abs + i)?;
    let tag = &body[tag_start..tag_end];
    let hs = tag.find("href=\"")? + 6;
    let he = tag[hs..].find('"')?;
    Some(&tag[hs..hs + he])
}

fn anchor_text(body: &str, from: usize) -> (String, usize) {
    let start = body[from..].find('>').map(|i| from + i + 1).unwrap_or(from);
    let end = body[start..].find("</a>").map(|i| start + i).unwrap_or(start);
    (strip_html_basic(&body[start..end]), end)
}

fn cut_snippet(raw: String) -> String {
    if raw.len() > 200 {
        format!("{}...", &raw[..raw.floor_char_boundary(200)])
    } else {
        raw
    }
}

/// Parse the `/html/` page: `<a class="result__a" href=…>title</a>` with a `result__snippet`.
pub(super) fn parse_html_results(body: &str) -> Vec<Hit> {
    let mut results = Vec::new();
    let mut pos = 0;
    let marker = "class=\"result__a\"";
    while results.len() < 10 {
        let Some(found) = body[pos..].find(marker) else { break };
        let marker_abs = pos + found;
        pos = marker_abs + marker.len();
        let Some(href) = tag_href(body, marker_abs) else { continue };
        let (title, _) = anchor_text(body, pos);

        let snippet_marker = "class=\"result__snippet\"";
        let next_result = body[pos..].find(marker).map(|i| pos + i).unwrap_or(body.len());
        let snippet = match body[pos..next_result].find(snippet_marker) {
            Some(sp) => {
                let sp_abs = pos + sp + snippet_marker.len();
                let start = body[sp_abs..].find('>').map(|i| sp_abs + i + 1).unwrap_or(sp_abs);
                let end = body[start..]
                    .find("</a>")
                    .or_else(|| body[start..].find("</td>"))
                    .map(|i| start + i)
                    .unwrap_or(start);
                cut_snippet(strip_html_basic(&body[start..end]))
            }
            None => String::new(),
        };
        let url = clean_url(href);
        if !title.is_empty() && !url.is_empty() {
            results.push((title, url, snippet));
        }
    }
    results
}

/// Parse the Lite page: `<a class="result-link" href=…>title</a>`, snippet in `result-snippet`.
pub(super) fn parse_lite_results(body: &str) -> Vec<Hit> {
    let mut results = Vec::new();
    let mut pos = 0;
    let marker = "result-link";
    while results.len() < 10 {
        let Some(found) = body[pos..].find(marker) else { break };
        let marker_abs = pos + found;
        pos = marker_abs + marker.len();
        let Some(href) = tag_href(body, marker_abs) else { continue };
        let (title, _) = anchor_text(body, marker_abs);

        let next_result = body[pos..].find(marker).map(|i| pos + i).unwrap_or(body.len());
        let snippet = match body[pos..next_result].find("result-snippet") {
            Some(sp) => {
                let sp_abs = pos + sp;
                let start = body[sp_abs..].find('>').map(|i| sp_abs + i + 1).unwrap_or(sp_abs);
                let end = body[start..].find("</td>").map(|i| start + i).unwrap_or(start);
                cut_snippet(strip_html_basic(&body[start..end]))
            }
            None => String::new(),
        };
        let url = clean_url(href);
        let url = if url.starts_with("//") { format!("https:{url}") } else { url };
        if !title.is_empty() && !url.is_empty() {
            results.push((title, url, snippet));
        }
    }
    results
}

fn format_results(query: &str, results: &[Hit]) -> String {
    if results.is_empty() {
        return format!("No results found for: {query}");
    }
    let mut output = format!("Search results for: {query}\n\n");
    for (i, (title, url, snippet)) in results.iter().enumerate() {
        output.push_str(&format!("{}. {}\n   {}\n   {}\n\n", i + 1, title, url, snippet));
    }
    output
}

/// The search with the two pages handed in, so a test can run it without the network.
fn search_with(query: &str, get: impl Fn(&str) -> Result<(u16, String), String>) -> String {
    let encoded = encode_query(query);
    let html = match get(&format!("{HTML_URL}{encoded}")) {
        Ok((status, body)) if !is_challenge(status, &body) => {
            return format_results(query, &parse_html_results(&body));
        }
        Ok(_) => None,
        Err(e) => Some(e),
    };
    if let Some(e) = html {
        return e;
    }
    tracing::info!("DuckDuckGo /html/ was challenged, trying Lite");
    match get(&format!("{LITE_URL}{encoded}")) {
        Ok((status, body)) if !is_challenge(status, &body) => {
            format_results(query, &parse_lite_results(&body))
        }
        Ok(_) => CHALLENGE_ERROR.to_string(),
        Err(e) => e,
    }
}

/// Search DuckDuckGo over plain HTTP: no browser, no JS.
pub(super) fn duckduckgo_html_search(query: &str) -> String {
    search_with(query, |url| fetch(url).map(|p| (p.status, p.body)))
}

#[cfg(test)]
#[path = "duckduckgo_tests.rs"]
mod tests;
