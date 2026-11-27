//! DuckDuckGo's pages as fixtures: no test here reaches the network (#1437).

use super::*;

const CHALLENGE: &str = include_str!("fixtures/ddg_challenge.html");
const HTML_RESULTS: &str = include_str!("fixtures/ddg_html_results.html");
const HTML_EMPTY: &str = include_str!("fixtures/ddg_html_empty.html");
const LITE_RESULTS: &str = include_str!("fixtures/ddg_lite_results.html");

fn page(status: u16, body: &str) -> Result<(u16, String), String> {
    Ok((status, body.to_string()))
}

#[test]
fn a_challenge_is_an_error_not_no_results() {
    let out = search_with("rust", |_| page(202, CHALLENGE));
    assert!(out.starts_with("Error:"), "{out}");
    assert!(!out.contains("No results"), "{out}");
}

#[test]
fn lite_answers_when_html_is_challenged() {
    let out = search_with("rust", |url| {
        if url.starts_with(HTML_URL) { page(202, CHALLENGE) } else { page(200, LITE_RESULTS) }
    });
    assert!(out.contains("First & best"), "{out}");
    assert!(out.contains("https://example.org/one?q=a+b"), "{out}");
    assert!(out.contains("https://example.com/two"), "{out}");
    assert!(!out.contains("uddg"), "{out}");
}

#[test]
fn a_challenge_on_both_pages_is_the_error() {
    assert_eq!(search_with("rust", |_| page(202, CHALLENGE)), CHALLENGE_ERROR);
    assert!(is_challenge(200, CHALLENGE));
}

#[test]
fn html_results_parse_with_the_redirect_decoded() {
    let hits = parse_html_results(HTML_RESULTS);
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].1, "https://example.org/café?a=1");
    assert!(hits[0].2.contains("snippet"));
    assert_eq!(hits[1].1, "https://example.com/two");
}

#[test]
fn lite_results_parse_to_title_and_url() {
    let hits = parse_lite_results(LITE_RESULTS);
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].0, "First & best");
    assert_eq!(hits[0].1, "https://example.org/one?q=a+b");
    assert_eq!(hits[0].2, "The first snippet.");
}

#[test]
fn a_real_empty_result_still_says_no_results() {
    assert_eq!(search_with("zzz", |_| page(200, HTML_EMPTY)), "No results found for: zzz");
}

#[test]
fn a_network_failure_is_an_error() {
    assert!(search_with("x", |_| Err("Error: DuckDuckGo search failed: down".into())).starts_with("Error:"));
}

#[test]
fn queries_are_encoded_as_utf8() {
    assert_eq!(encode_query("café au lait"), "caf%C3%A9+au+lait");
    assert_eq!(encode_query("日本"), "%E6%97%A5%E6%9C%AC");
    assert_eq!(encode_query("a-b_c.d~e"), "a-b_c.d~e");
    assert_eq!(url_decode("caf%C3%A9%2"), "café%2");
}
