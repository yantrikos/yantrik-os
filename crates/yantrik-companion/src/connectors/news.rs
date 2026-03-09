//! News/RSS Feed Scanner — interest-based relevance filtering.
//!
//! Polls RSS/Atom feeds, extracts articles, scores them against the user's
//! Interest nodes in the PWG, and produces `LifeEvent::NewsRelevant` or
//! `LifeEvent::PriceChange` events for articles above the relevance threshold.
//!
//! This powers the "Hey, I was checking this for you" proactive insights:
//! - "US struck Iranian naval assets → oil prices may rise → top up your car"
//! - "New AI breakthrough in language models — you might find this interesting"
//! - "Your favorite band announced a tour date near you"

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use crate::graph_bridge::{LifeEvent, LifeEventKind};
use crate::world_graph::{EntityType, WorldGraph};

// ── Feed Configuration ───────────────────────────────────────────────

/// A configured RSS/Atom feed source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedSource {
    /// Human-readable name.
    pub name: String,
    /// Feed URL.
    pub url: String,
    /// Which interest categories this feed is relevant to.
    pub categories: Vec<String>,
    /// Whether this feed is enabled.
    pub enabled: bool,
}

/// Default feeds covering major news categories.
pub fn default_feeds() -> Vec<FeedSource> {
    vec![
        // General news
        FeedSource {
            name: "Reuters World".into(),
            url: "https://feeds.reuters.com/reuters/worldNews".into(),
            categories: vec!["politics".into(), "geopolitics".into()],
            enabled: true,
        },
        FeedSource {
            name: "BBC World".into(),
            url: "https://feeds.bbci.co.uk/news/world/rss.xml".into(),
            categories: vec!["politics".into(), "geopolitics".into()],
            enabled: true,
        },
        // Tech
        FeedSource {
            name: "Hacker News".into(),
            url: "https://hnrss.org/frontpage".into(),
            categories: vec!["technology".into(), "AI".into(), "startups".into()],
            enabled: true,
        },
        FeedSource {
            name: "TechCrunch".into(),
            url: "https://techcrunch.com/feed/".into(),
            categories: vec!["technology".into(), "startups".into()],
            enabled: true,
        },
        FeedSource {
            name: "Ars Technica".into(),
            url: "https://feeds.arstechnica.com/arstechnica/index".into(),
            categories: vec!["technology".into(), "science".into()],
            enabled: true,
        },
        // Finance
        FeedSource {
            name: "Yahoo Finance".into(),
            url: "https://finance.yahoo.com/news/rssindex".into(),
            categories: vec!["finance".into(), "stocks".into(), "commodities".into()],
            enabled: true,
        },
        FeedSource {
            name: "CNBC Top News".into(),
            url: "https://search.cnbc.com/rs/search/combinedcms/view.xml?partnerId=wrss01&id=100003114".into(),
            categories: vec!["finance".into(), "business".into()],
            enabled: true,
        },
        // Science
        FeedSource {
            name: "Nature News".into(),
            url: "https://www.nature.com/nature.rss".into(),
            categories: vec!["science".into(), "research".into()],
            enabled: true,
        },
        // Sports
        FeedSource {
            name: "ESPN Top Headlines".into(),
            url: "https://www.espn.com/espn/rss/news".into(),
            categories: vec!["sports".into()],
            enabled: true,
        },
    ]
}

// ── Feed Item ────────────────────────────────────────────────────────

/// A parsed article from an RSS/Atom feed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedItem {
    pub title: String,
    pub description: String,
    pub link: String,
    pub pub_date: Option<String>,
    pub source_feed: String,
    pub categories: Vec<String>,
}

// ── Lightweight RSS/Atom Parser ──────────────────────────────────────

/// Parse RSS 2.0 or Atom XML into feed items.
/// This is a lightweight parser — handles common cases without an XML library.
pub fn parse_feed(xml: &str, source_name: &str) -> Vec<FeedItem> {
    let mut items = Vec::new();

    // Detect if Atom or RSS
    let is_atom = xml.contains("<feed") && xml.contains("xmlns=\"http://www.w3.org/2005/Atom\"");

    if is_atom {
        parse_atom(xml, source_name, &mut items);
    } else {
        parse_rss(xml, source_name, &mut items);
    }

    items
}

fn parse_rss(xml: &str, source_name: &str, items: &mut Vec<FeedItem>) {
    // Split on <item> tags
    for item_chunk in xml.split("<item>").skip(1) {
        let end = item_chunk.find("</item>").unwrap_or(item_chunk.len());
        let chunk = &item_chunk[..end];

        let title = extract_tag(chunk, "title").unwrap_or_default();
        let description = extract_tag(chunk, "description").unwrap_or_default();
        let link = extract_tag(chunk, "link").unwrap_or_default();
        let pub_date = extract_tag(chunk, "pubDate");

        // Extract categories
        let mut categories = Vec::new();
        for cat_chunk in chunk.split("<category>").skip(1) {
            if let Some(end) = cat_chunk.find("</category>") {
                categories.push(cat_chunk[..end].trim().to_string());
            }
        }

        if !title.is_empty() {
            items.push(FeedItem {
                title: clean_html(&title),
                description: clean_html(&description),
                link,
                pub_date,
                source_feed: source_name.to_string(),
                categories,
            });
        }
    }
}

fn parse_atom(xml: &str, source_name: &str, items: &mut Vec<FeedItem>) {
    // Split on <entry> tags
    for entry_chunk in xml.split("<entry>").skip(1) {
        let end = entry_chunk.find("</entry>").unwrap_or(entry_chunk.len());
        let chunk = &entry_chunk[..end];

        let title = extract_tag(chunk, "title").unwrap_or_default();
        let summary = extract_tag(chunk, "summary")
            .or_else(|| extract_tag(chunk, "content"))
            .unwrap_or_default();

        // Atom links are in attributes: <link href="..." />
        let link = extract_atom_link(chunk).unwrap_or_default();
        let updated = extract_tag(chunk, "updated");

        // Extract categories from <category term="..."/>
        let mut categories = Vec::new();
        for cat_chunk in chunk.split("<category").skip(1) {
            if let Some(term_start) = cat_chunk.find("term=\"") {
                let rest = &cat_chunk[term_start + 6..];
                if let Some(term_end) = rest.find('"') {
                    categories.push(rest[..term_end].to_string());
                }
            }
        }

        if !title.is_empty() {
            items.push(FeedItem {
                title: clean_html(&title),
                description: clean_html(&summary),
                link,
                pub_date: updated,
                source_feed: source_name.to_string(),
                categories,
            });
        }
    }
}

/// Extract content between opening and closing XML tags.
fn extract_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{}", tag);
    let close = format!("</{}>", tag);

    let start_idx = xml.find(&open)?;
    let after_open = &xml[start_idx..];

    // Find the end of the opening tag (handle attributes)
    let content_start = after_open.find('>')? + 1;
    let content = &after_open[content_start..];

    let end_idx = content.find(&close)?;
    let value = content[..end_idx].trim();

    // Handle CDATA
    let cleaned = if value.starts_with("<![CDATA[") && value.ends_with("]]>") {
        &value[9..value.len() - 3]
    } else {
        value
    };

    Some(cleaned.to_string())
}

/// Extract href from Atom <link> tag.
fn extract_atom_link(xml: &str) -> Option<String> {
    // Look for <link ... href="..." ... />
    // Prefer rel="alternate" but fall back to first link
    let mut best_link = None;

    for link_chunk in xml.split("<link").skip(1) {
        let end = link_chunk.find("/>").or_else(|| link_chunk.find(">"))?;
        let attrs = &link_chunk[..end];

        if let Some(href_start) = attrs.find("href=\"") {
            let rest = &attrs[href_start + 6..];
            if let Some(href_end) = rest.find('"') {
                let href = rest[..href_end].to_string();
                if attrs.contains("rel=\"alternate\"") {
                    return Some(href);
                }
                if best_link.is_none() {
                    best_link = Some(href);
                }
            }
        }
    }

    best_link
}

/// Strip HTML tags from a string.
fn clean_html(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut in_tag = false;

    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(c),
            _ => {}
        }
    }

    // Decode common HTML entities
    result
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&#x27;", "'")
        .trim()
        .to_string()
}

// ── Relevance Scoring ────────────────────────────────────────────────

/// Keywords that indicate price/commodity changes (trigger PriceChange events).
const PRICE_KEYWORDS: &[&str] = &[
    "oil price", "crude oil", "gas price", "fuel price", "petrol",
    "commodity", "OPEC", "inflation", "interest rate", "stock market crash",
    "market surge", "price hike", "price drop", "tariff",
];

/// Score how relevant a feed item is to the user's interests.
/// Returns (relevance_score, matched_interests, extracted_keywords).
pub fn score_relevance(
    conn: &Connection,
    item: &FeedItem,
) -> (f64, Vec<String>, Vec<String>) {
    let text = format!("{} {} {}", item.title, item.description, item.categories.join(" "));
    let text_lower = text.to_lowercase();

    let mut matched_interests = Vec::new();
    let mut extracted_keywords = Vec::new();
    let mut max_score: f64 = 0.0;

    // Check against all Interest nodes in the PWG
    let interests = WorldGraph::nodes_by_type(conn, EntityType::Interest);

    for interest in &interests {
        let name_lower = interest.name.to_lowercase();

        // Direct name match in text
        if text_lower.contains(&name_lower) {
            let score = 0.7 * interest.salience.max(0.3); // Weight by current salience
            max_score = max_score.max(score);
            matched_interests.push(interest.name.clone());
            extracted_keywords.push(interest.name.to_lowercase());
        }

        // Check each keyword of the interest
        for keyword in &interest.keywords {
            let kw_lower = keyword.to_lowercase();
            if text_lower.contains(&kw_lower) {
                let score = 0.5 * interest.salience.max(0.3);
                max_score = max_score.max(score);
                if !matched_interests.contains(&interest.name) {
                    matched_interests.push(interest.name.clone());
                }
                extracted_keywords.push(kw_lower);
            }
        }
    }

    // Also check feed categories against interest categories
    for cat in &item.categories {
        let cat_lower = cat.to_lowercase();
        for interest in &interests {
            if interest.name.to_lowercase() == cat_lower {
                max_score = max_score.max(0.6);
                if !matched_interests.contains(&interest.name) {
                    matched_interests.push(interest.name.clone());
                }
            }
        }
    }

    // Boost for price-related content (actionable)
    for pk in PRICE_KEYWORDS {
        if text_lower.contains(pk) {
            extracted_keywords.push(pk.to_string());
            max_score = max_score.max(0.65);
        }
    }

    (max_score, matched_interests, extracted_keywords)
}

/// Determine if a feed item is about price/commodity changes.
fn is_price_related(item: &FeedItem, keywords: &[String]) -> bool {
    keywords.iter().any(|k| {
        PRICE_KEYWORDS.iter().any(|pk| k.contains(pk))
    })
}

// ── News Scanner ─────────────────────────────────────────────────────

/// Configuration for the news scanner.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewsScannerConfig {
    /// RSS feeds to scan.
    pub feeds: Vec<FeedSource>,
    /// Minimum relevance score to produce an event (0.0–1.0).
    pub relevance_threshold: f64,
    /// Maximum articles to process per feed per scan.
    pub max_articles_per_feed: usize,
    /// Maximum total events to produce per scan cycle.
    pub max_events_per_scan: usize,
}

impl Default for NewsScannerConfig {
    fn default() -> Self {
        Self {
            feeds: default_feeds(),
            relevance_threshold: 0.3,
            max_articles_per_feed: 10,
            max_events_per_scan: 15,
        }
    }
}

/// Result of a news scan cycle.
#[derive(Debug, Clone)]
pub struct ScanResult {
    /// Total feeds fetched.
    pub feeds_fetched: usize,
    /// Total articles parsed.
    pub articles_parsed: usize,
    /// Articles that passed relevance threshold.
    pub articles_relevant: usize,
    /// Life events produced.
    pub events: Vec<LifeEvent>,
    /// Errors encountered during fetching.
    pub errors: Vec<String>,
}

/// Fetch and scan all configured feeds, producing life events for relevant articles.
pub fn scan_feeds(conn: &Connection, config: &NewsScannerConfig) -> ScanResult {
    let mut result = ScanResult {
        feeds_fetched: 0,
        articles_parsed: 0,
        articles_relevant: 0,
        events: Vec::new(),
        errors: Vec::new(),
    };

    let now = now_ts();
    let mut seen_titles: HashSet<String> = HashSet::new();

    // Get user's active interest categories to filter feeds
    let user_interests: Vec<String> = WorldGraph::nodes_by_type(conn, EntityType::Interest)
        .iter()
        .filter(|n| n.salience > 0.1)
        .map(|n| n.name.to_lowercase())
        .collect();

    for feed in &config.feeds {
        if !feed.enabled {
            continue;
        }

        // Skip feeds not matching any user interest (optimization)
        let feed_relevant = feed.categories.iter().any(|cat| {
            user_interests.iter().any(|ui| ui.contains(&cat.to_lowercase()) || cat.to_lowercase().contains(ui))
        }) || user_interests.is_empty(); // scan all if no interests yet

        if !feed_relevant {
            continue;
        }

        match fetch_feed(&feed.url) {
            Ok(xml) => {
                result.feeds_fetched += 1;
                let items = parse_feed(&xml, &feed.name);

                for item in items.iter().take(config.max_articles_per_feed) {
                    // Deduplicate by title
                    if seen_titles.contains(&item.title) {
                        continue;
                    }
                    seen_titles.insert(item.title.clone());
                    result.articles_parsed += 1;

                    // Score relevance
                    let (score, matched, keywords) = score_relevance(conn, item);

                    if score >= config.relevance_threshold {
                        result.articles_relevant += 1;

                        let kind = if is_price_related(item, &keywords) {
                            LifeEventKind::PriceChange
                        } else {
                            LifeEventKind::NewsRelevant
                        };

                        let event = LifeEvent {
                            kind,
                            summary: format!("{}: {}", item.title, truncate(&item.description, 200)),
                            keywords,
                            entities: matched,
                            importance: score.min(1.0),
                            source: format!("news:{}", feed.name),
                            data: serde_json::json!({
                                "title": item.title,
                                "link": item.link,
                                "feed": feed.name,
                                "pub_date": item.pub_date,
                                "relevance_score": score,
                            }),
                            timestamp: now,
                        };

                        result.events.push(event);

                        if result.events.len() >= config.max_events_per_scan {
                            return result;
                        }
                    }
                }
            }
            Err(e) => {
                result.errors.push(format!("{}: {}", feed.name, e));
            }
        }
    }

    // Sort by relevance (highest first)
    result.events.sort_by(|a, b| b.importance.partial_cmp(&a.importance).unwrap_or(std::cmp::Ordering::Equal));

    result
}

/// Fetch a feed URL using ureq.
fn fetch_feed(url: &str) -> Result<String, String> {
    ureq::get(url)
        .set("User-Agent", "YantrikOS/1.0 (RSS Reader)")
        .timeout(std::time::Duration::from_secs(15))
        .call()
        .map_err(|e| format!("HTTP error: {}", e))
        .and_then(|resp| {
            resp.into_string()
                .map_err(|e| format!("Read error: {}", e))
        })
}

fn truncate(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        s
    } else {
        &s[..s.char_indices().take(max_len).last().map(|(i, c)| i + c.len_utf8()).unwrap_or(max_len)]
    }
}

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world_graph::WorldGraph;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        WorldGraph::ensure_tables(&conn);
        WorldGraph::seed_interests(&conn, &["Technology", "Finance"]);
        WorldGraph::seed_defaults(&conn);
        conn
    }

    #[test]
    fn parse_rss_feed() {
        let xml = r#"<?xml version="1.0"?>
        <rss version="2.0">
          <channel>
            <title>Test Feed</title>
            <item>
              <title>Oil Prices Surge After Iran Strike</title>
              <description>Crude oil futures jumped 5% following military action</description>
              <link>https://example.com/oil-prices</link>
              <pubDate>Mon, 09 Mar 2026 12:00:00 GMT</pubDate>
              <category>Finance</category>
              <category>Commodities</category>
            </item>
            <item>
              <title>New AI Model Beats Human Performance</title>
              <description>Researchers announce breakthrough in language understanding</description>
              <link>https://example.com/ai-breakthrough</link>
              <pubDate>Mon, 09 Mar 2026 10:00:00 GMT</pubDate>
              <category>Technology</category>
            </item>
          </channel>
        </rss>"#;

        let items = parse_feed(xml, "Test Feed");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].title, "Oil Prices Surge After Iran Strike");
        assert_eq!(items[0].source_feed, "Test Feed");
        assert!(items[0].categories.contains(&"Finance".to_string()));
        assert_eq!(items[1].title, "New AI Model Beats Human Performance");
    }

    #[test]
    fn parse_atom_feed() {
        let xml = r#"<?xml version="1.0"?>
        <feed xmlns="http://www.w3.org/2005/Atom">
          <title>Test Atom Feed</title>
          <entry>
            <title>SpaceX Launches New Satellite</title>
            <summary>Latest Starlink mission succeeds</summary>
            <link href="https://example.com/spacex" rel="alternate"/>
            <updated>2026-03-09T12:00:00Z</updated>
            <category term="Space"/>
          </entry>
        </feed>"#;

        let items = parse_feed(xml, "Atom Test");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "SpaceX Launches New Satellite");
        assert_eq!(items[0].link, "https://example.com/spacex");
    }

    #[test]
    fn parse_cdata_content() {
        let xml = r#"<?xml version="1.0"?>
        <rss version="2.0">
          <channel>
            <item>
              <title><![CDATA[Breaking: Major Tech Acquisition]]></title>
              <description><![CDATA[<p>Company A buys Company B for $10B</p>]]></description>
              <link>https://example.com/acquisition</link>
            </item>
          </channel>
        </rss>"#;

        let items = parse_feed(xml, "Test");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Breaking: Major Tech Acquisition");
        assert_eq!(items[0].description, "Company A buys Company B for $10B");
    }

    #[test]
    fn relevance_scoring_finance() {
        let conn = setup();

        let item = FeedItem {
            title: "Oil Prices Surge After Iran Strike".into(),
            description: "Crude oil futures jumped 5% following military action in the Strait of Hormuz".into(),
            link: "https://example.com".into(),
            pub_date: None,
            source_feed: "Reuters".into(),
            categories: vec!["Finance".into()],
        };

        let (score, matched, keywords) = score_relevance(&conn, &item);
        assert!(score > 0.3, "oil/finance article should score above threshold, got {}", score);
        assert!(!keywords.is_empty(), "should extract keywords");

        // Should detect this as price-related
        assert!(is_price_related(&item, &keywords), "should detect price-related content");
    }

    #[test]
    fn relevance_scoring_tech() {
        let conn = setup();

        let item = FeedItem {
            title: "New AI Breakthrough in Language Models".into(),
            description: "Researchers achieve new state of the art in natural language processing".into(),
            link: "https://example.com".into(),
            pub_date: None,
            source_feed: "HN".into(),
            categories: vec!["Technology".into()],
        };

        let (score, matched, _) = score_relevance(&conn, &item);
        assert!(score > 0.3, "AI/tech article should score above threshold, got {}", score);
        assert!(matched.iter().any(|m| m == "AI" || m == "Technology"),
                "should match AI or Technology interest");
    }

    #[test]
    fn relevance_scoring_irrelevant() {
        let conn = setup();

        // User has Technology and Finance interests, not Sports
        let item = FeedItem {
            title: "Local Cricket Team Wins Championship".into(),
            description: "The hometown cricket team celebrated their victory".into(),
            link: "https://example.com".into(),
            pub_date: None,
            source_feed: "ESPN".into(),
            categories: vec!["Sports".into(), "Cricket".into()],
        };

        let (score, matched, _) = score_relevance(&conn, &item);
        assert!(score < 0.3, "sports article should score low for tech/finance user, got {}", score);
    }

    #[test]
    fn clean_html_entities() {
        assert_eq!(clean_html("Hello &amp; World"), "Hello & World");
        assert_eq!(clean_html("<b>Bold</b> text"), "Bold text");
        assert_eq!(clean_html("It&#39;s a test"), "It's a test");
    }

    #[test]
    fn default_feeds_populated() {
        let feeds = default_feeds();
        assert!(feeds.len() >= 5, "should have at least 5 default feeds");
        assert!(feeds.iter().any(|f| f.name.contains("Hacker News")));
        assert!(feeds.iter().all(|f| f.enabled));
    }

    #[test]
    fn feed_filtering_by_interest() {
        let conn = setup(); // Has Technology and Finance interests

        let config = NewsScannerConfig {
            feeds: vec![
                FeedSource {
                    name: "Tech Feed".into(),
                    url: "https://invalid.example.com/tech".into(),
                    categories: vec!["technology".into()],
                    enabled: true,
                },
                FeedSource {
                    name: "Sports Feed".into(),
                    url: "https://invalid.example.com/sports".into(),
                    categories: vec!["sports".into()],
                    enabled: true,
                },
            ],
            relevance_threshold: 0.3,
            max_articles_per_feed: 5,
            max_events_per_scan: 10,
        };

        // scan_feeds will try to fetch and fail (invalid URLs)
        // but it should only attempt feeds matching user interests
        let result = scan_feeds(&conn, &config);

        // Tech feed should have been attempted (matches Technology interest)
        // Sports feed should NOT have been attempted (no Sports interest)
        // Both will error since URLs are invalid, but we can check which were tried
        let tech_error = result.errors.iter().any(|e| e.contains("Tech Feed"));
        let sports_error = result.errors.iter().any(|e| e.contains("Sports Feed"));

        assert!(tech_error, "should attempt Tech Feed (matches user interest)");
        assert!(!sports_error, "should skip Sports Feed (no matching interest)");
    }
}
