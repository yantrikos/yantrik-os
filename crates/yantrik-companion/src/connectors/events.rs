//! Event Discovery Connector — finds local events matching user interests.
//!
//! Powers: "There's a concert by the artist you like next month at X.
//!          Tickets are available from $XYZ."
//!
//! Sources (in order of preference):
//! 1. RSS feeds from event platforms (Eventbrite, Meetup, local venues)
//! 2. Web search queries constructed from PWG Interest nodes + location
//!
//! Produces `EventDiscovered` LifeEvents scored against user interests.
//! Scans daily (not hourly — events change slowly).

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::graph_bridge::{LifeEvent, LifeEventKind};
use crate::world_graph::{EntityType, WorldGraph};

// ── Event Discovery Types ───────────────────────────────────────────

/// A discovered event from any source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredEvent {
    /// Event title.
    pub title: String,
    /// Date/time description (may be human-readable).
    pub date_text: String,
    /// Unix timestamp if parseable, 0 if not.
    pub date_ts: f64,
    /// Venue name.
    pub venue: String,
    /// City/location.
    pub city: String,
    /// Price text (e.g., "$25-$50", "Free").
    pub price: String,
    /// URL to event page.
    pub url: String,
    /// Source platform.
    pub source: String,
    /// Keywords/tags for matching.
    pub tags: Vec<String>,
    /// Description/summary.
    pub description: String,
}

/// Configuration for event discovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventDiscoveryConfig {
    /// User's city for local event search.
    pub city: String,
    /// Maximum distance in km (used for search queries).
    pub radius_km: u32,
    /// Custom RSS feeds for local events.
    pub custom_feeds: Vec<EventFeedSource>,
    /// Minimum relevance score to surface (0.0-1.0).
    pub min_relevance: f64,
    /// Maximum events to surface per scan.
    pub max_results: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventFeedSource {
    pub name: String,
    pub url: String,
    pub event_type: String, // "music", "tech", "sports", etc.
}

impl Default for EventDiscoveryConfig {
    fn default() -> Self {
        Self {
            city: String::new(),
            radius_km: 50,
            custom_feeds: vec![],
            min_relevance: 0.3,
            max_results: 10,
        }
    }
}

// ── Event Feed Parsing ──────────────────────────────────────────────

/// Parse events from an RSS/Atom feed (same XML as news, but event-structured).
pub fn parse_event_feed(xml: &str, source: &str) -> Vec<DiscoveredEvent> {
    let mut events = Vec::new();

    // Try RSS 2.0 items first, then Atom entries
    let items = extract_items(xml, "item");
    let entries = if items.is_empty() {
        extract_items(xml, "entry")
    } else {
        vec![]
    };

    for item_xml in items.iter().chain(entries.iter()) {
        let title = extract_tag_content(item_xml, "title").unwrap_or_default();
        if title.is_empty() {
            continue;
        }

        let description = extract_tag_content(item_xml, "description")
            .or_else(|| extract_tag_content(item_xml, "summary"))
            .or_else(|| extract_tag_content(item_xml, "content"))
            .unwrap_or_default();

        let url = extract_tag_content(item_xml, "link")
            .or_else(|| extract_link_href(item_xml))
            .unwrap_or_default();

        let date_text = extract_tag_content(item_xml, "pubDate")
            .or_else(|| extract_tag_content(item_xml, "published"))
            .or_else(|| extract_tag_content(item_xml, "updated"))
            .unwrap_or_default();

        // Extract venue/location from description or category tags
        let venue = extract_venue_from_description(&description);
        let tags = extract_categories(item_xml);

        events.push(DiscoveredEvent {
            title: clean_html(&title),
            date_text,
            date_ts: 0.0, // would need date parsing
            venue,
            city: String::new(),
            price: extract_price_from_description(&description),
            url,
            source: source.to_string(),
            tags,
            description: clean_html(&truncate(&description, 500)),
        });
    }

    events
}

// ── Relevance Scoring ───────────────────────────────────────────────

/// Score a discovered event against user's PWG Interest nodes.
pub fn score_event_relevance(
    event: &DiscoveredEvent,
    conn: &Connection,
) -> f64 {
    let interests = WorldGraph::nodes_by_type(conn, EntityType::Interest);

    let mut max_score = 0.0_f64;
    let title_lower = event.title.to_lowercase();
    let desc_lower = event.description.to_lowercase();
    let combined = format!("{} {} {}", title_lower, desc_lower, event.tags.join(" ").to_lowercase());

    for node in &interests {
        let name_lower = node.name.to_lowercase();
        let mut score = 0.0_f64;

        // Direct name match in title — strong signal
        if title_lower.contains(&name_lower) {
            score += 0.6;
        }
        // Name match in description
        if desc_lower.contains(&name_lower) {
            score += 0.3;
        }

        // Keyword matching
        for keyword in &node.keywords {
            let kw_lower = keyword.to_lowercase();
            if combined.contains(&kw_lower) {
                score += 0.15;
            }
        }

        // Tag matching
        for tag in &event.tags {
            let tag_lower = tag.to_lowercase();
            if tag_lower == name_lower || node.keywords.iter().any(|k| k.to_lowercase() == tag_lower) {
                score += 0.2;
            }
        }

        // Weight by node's current salience (recently activated interests score higher)
        let salience_boost = 1.0 + node.salience * 0.5;
        score *= salience_boost;

        max_score = max_score.max(score);
    }

    max_score.min(1.0)
}

/// Scan for events and produce LifeEvents above relevance threshold.
pub fn scan_events(
    config: &EventDiscoveryConfig,
    conn: &Connection,
) -> Vec<LifeEvent> {
    let mut discovered = Vec::new();

    // 1. Fetch from custom feeds
    for feed in &config.custom_feeds {
        match fetch_feed(&feed.url) {
            Ok(xml) => {
                let events = parse_event_feed(&xml, &feed.name);
                discovered.extend(events);
            }
            Err(e) => {
                tracing::warn!(feed = %feed.name, error = %e, "Event feed fetch failed");
            }
        }
    }

    // 2. Score against interests
    let mut scored: Vec<(DiscoveredEvent, f64)> = discovered
        .into_iter()
        .map(|event| {
            let score = score_event_relevance(&event, conn);
            (event, score)
        })
        .filter(|(_, score)| *score >= config.min_relevance)
        .collect();

    // Sort by relevance
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(config.max_results);

    // 3. Convert to LifeEvents
    scored
        .into_iter()
        .map(|(event, score)| {
            let mut summary = format!("Event: {}", event.title);
            if !event.venue.is_empty() {
                summary.push_str(&format!(" at {}", event.venue));
            }
            if !event.date_text.is_empty() {
                summary.push_str(&format!(" — {}", event.date_text));
            }
            if !event.price.is_empty() {
                summary.push_str(&format!(". Tickets: {}", event.price));
            }

            let mut keywords = vec!["event".into(), "discovery".into()];
            keywords.extend(event.tags.iter().cloned());

            LifeEvent {
                kind: LifeEventKind::EventDiscovered,
                summary,
                keywords,
                entities: vec![event.title.clone()],
                importance: (score * 0.8).min(0.9), // scale to importance
                source: format!("events:{}", event.source),
                data: serde_json::json!({
                    "title": event.title,
                    "venue": event.venue,
                    "city": event.city,
                    "date": event.date_text,
                    "price": event.price,
                    "url": event.url,
                    "relevance_score": score,
                }),
                timestamp: now_ts(),
            }
        })
        .collect()
}

// ── Search Query Builder ────────────────────────────────────────────

/// Build web search queries from user interests + location for event discovery.
/// These can be used with a search engine or Brave Search API.
pub fn build_search_queries(
    conn: &Connection,
    city: &str,
) -> Vec<String> {
    let interests = WorldGraph::nodes_by_type(conn, EntityType::Interest);
    let mut queries = Vec::new();

    for node in &interests {
        if node.salience < 0.1 {
            continue; // Skip low-salience interests
        }

        let name = &node.name;

        // Build event-specific search queries
        if node.keywords.iter().any(|k| {
            let lower = k.to_lowercase();
            lower.contains("music") || lower.contains("band") || lower.contains("artist")
        }) {
            queries.push(format!("{} concert tickets {} 2026", name, city));
        } else if node.keywords.iter().any(|k| {
            let lower = k.to_lowercase();
            lower.contains("tech") || lower.contains("software") || lower.contains("AI")
        }) {
            queries.push(format!("{} meetup conference {} upcoming", name, city));
        } else if node.keywords.iter().any(|k| {
            let lower = k.to_lowercase();
            lower.contains("sport") || lower.contains("game") || lower.contains("match")
        }) {
            queries.push(format!("{} tickets {} schedule 2026", name, city));
        } else {
            queries.push(format!("{} events {} upcoming", name, city));
        }
    }

    queries.truncate(10); // Don't generate too many queries
    queries
}

// ── XML Helpers (shared with news connector) ────────────────────────

fn extract_items(xml: &str, tag: &str) -> Vec<String> {
    let open = format!("<{}", tag);
    let close = format!("</{}>", tag);
    let mut items = Vec::new();
    let mut search_from = 0;

    while let Some(start) = xml[search_from..].find(&open) {
        let abs_start = search_from + start;
        if let Some(end) = xml[abs_start..].find(&close) {
            let abs_end = abs_start + end + close.len();
            items.push(xml[abs_start..abs_end].to_string());
            search_from = abs_end;
        } else {
            break;
        }
    }

    items
}

fn extract_tag_content(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{}", tag);
    let close = format!("</{}>", tag);

    let start_pos = xml.find(&open)?;
    let tag_end = xml[start_pos..].find('>')? + start_pos + 1;
    let end_pos = xml[tag_end..].find(&close)? + tag_end;

    let content = xml[tag_end..end_pos].trim();

    // Handle CDATA
    let content = if content.starts_with("<![CDATA[") && content.ends_with("]]>") {
        &content[9..content.len() - 3]
    } else {
        content
    };

    if content.is_empty() {
        None
    } else {
        Some(content.to_string())
    }
}

fn extract_link_href(xml: &str) -> Option<String> {
    let start = xml.find("<link")?;
    let chunk = &xml[start..xml[start..].find('>')? + start + 1];
    let href_start = chunk.find("href=\"")? + 6;
    let href_end = chunk[href_start..].find('"')? + href_start;
    Some(chunk[href_start..href_end].to_string())
}

fn extract_categories(xml: &str) -> Vec<String> {
    let mut cats = Vec::new();
    let mut pos = 0;
    while let Some(start) = xml[pos..].find("<category") {
        let abs = pos + start;
        if let Some(end) = xml[abs..].find("</category>") {
            let tag_end = xml[abs..].find('>').unwrap_or(0) + abs + 1;
            let content = xml[tag_end..abs + end].trim();
            if !content.is_empty() {
                cats.push(clean_html(content));
            }
            pos = abs + end + 11;
        } else {
            break;
        }
    }
    cats
}

fn extract_venue_from_description(desc: &str) -> String {
    // Look for common venue patterns in event descriptions
    let lower = desc.to_lowercase();

    // "at Venue Name" or "@ Venue"
    for prefix in &[" at ", " @ ", "venue: ", "location: "] {
        if let Some(idx) = lower.find(prefix) {
            let start = idx + prefix.len();
            let rest = &desc[start..];
            // Take until next period, comma, or newline
            let end = rest.find(|c: char| c == '.' || c == ',' || c == '\n' || c == '<')
                .unwrap_or(rest.len().min(100));
            let venue = rest[..end].trim();
            if !venue.is_empty() && venue.len() < 100 {
                return clean_html(venue);
            }
        }
    }

    String::new()
}

fn extract_price_from_description(desc: &str) -> String {
    // Look for price patterns: $XX, $XX-$XX, €XX, £XX, "Free", "from $XX"
    let lower = desc.to_lowercase();

    if lower.contains("free admission") || lower.contains("free entry") || lower.contains("free event") {
        return "Free".to_string();
    }

    // Find currency symbols followed by numbers
    for (i, c) in desc.char_indices() {
        if c == '$' || c == '€' || c == '£' {
            let rest = &desc[i..];
            let end = rest.find(|c: char| c.is_whitespace() || c == ',' || c == '.' || c == '<')
                .unwrap_or(rest.len().min(30));
            let price = rest[..end].trim();
            if price.len() >= 2 && price.len() <= 20 {
                return price.to_string();
            }
        }
    }

    String::new()
}

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
    result
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        s.char_indices()
            .take(max_len)
            .last()
            .map(|(i, c)| s[..i + c.len_utf8()].to_string())
            .unwrap_or_else(|| s[..max_len].to_string())
    }
}

fn fetch_feed(url: &str) -> Result<String, String> {
    ureq::get(url)
        .set("User-Agent", "YantrikOS/1.0 (Event Discovery)")
        .timeout(std::time::Duration::from_secs(15))
        .call()
        .map_err(|e| format!("HTTP error: {}", e))
        .and_then(|resp| {
            resp.into_string()
                .map_err(|e| format!("Read error: {}", e))
        })
}

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_event_rss_feed() {
        let xml = r#"<?xml version="1.0"?>
<rss version="2.0">
<channel>
<title>Events</title>
<item>
<title>Jazz Night at Blue Note</title>
<description>Live jazz performance at Blue Note. Tickets from $30. Join us for an evening of music.</description>
<link>https://example.com/jazz-night</link>
<pubDate>Sat, 15 Mar 2026 20:00:00 GMT</pubDate>
<category>Music</category>
<category>Jazz</category>
</item>
<item>
<title>Tech Meetup: AI in Healthcare</title>
<description>Monthly tech meetup at Startup Hub. Free entry. Networking and talks.</description>
<link>https://example.com/ai-meetup</link>
<pubDate>Wed, 12 Mar 2026 18:30:00 GMT</pubDate>
<category>Technology</category>
</item>
</channel>
</rss>"#;

        let events = parse_event_feed(xml, "test_feed");
        assert_eq!(events.len(), 2);

        let jazz = &events[0];
        assert_eq!(jazz.title, "Jazz Night at Blue Note");
        assert_eq!(jazz.price, "$30");
        assert!(jazz.venue.contains("Blue Note"));
        assert!(jazz.tags.contains(&"Music".to_string()));

        let meetup = &events[1];
        assert_eq!(meetup.title, "Tech Meetup: AI in Healthcare");
        assert!(meetup.tags.contains(&"Technology".to_string()));
    }

    #[test]
    fn price_extraction() {
        assert_eq!(extract_price_from_description("Tickets from $25"), "$25");
        assert_eq!(extract_price_from_description("Entry: €15"), "€15");
        assert_eq!(extract_price_from_description("Free admission for all"), "Free");
        assert_eq!(extract_price_from_description("No price info here"), "");
    }

    #[test]
    fn venue_extraction() {
        assert_eq!(
            extract_venue_from_description("Live music at Blue Note Club. Great vibes."),
            "Blue Note Club"
        );
        assert_eq!(
            extract_venue_from_description("Location: Madison Square Garden"),
            "Madison Square Garden"
        );
        assert_eq!(
            extract_venue_from_description("No venue mentioned"),
            ""
        );
    }

    #[test]
    fn relevance_scoring_with_pwg() {
        let conn = Connection::open_in_memory().unwrap();
        WorldGraph::ensure_tables(&conn);

        // Seed music interest — seed_interests takes &[&str] of category names
        WorldGraph::seed_interests(&conn, &["Music"]);

        let event = DiscoveredEvent {
            title: "Jazz Festival 2026".into(),
            date_text: "March 15".into(),
            date_ts: 0.0,
            venue: "Central Park".into(),
            city: "New York".into(),
            price: "$50".into(),
            url: "https://example.com".into(),
            source: "test".into(),
            tags: vec!["Music".into(), "Jazz".into()],
            description: "Annual jazz festival with live music performances".into(),
        };

        let score = score_event_relevance(&event, &conn);
        assert!(score > 0.3, "Jazz event should be relevant to music interest, got {}", score);

        // Irrelevant event
        let boring = DiscoveredEvent {
            title: "Plumbing Workshop".into(),
            date_text: "March 20".into(),
            date_ts: 0.0,
            venue: "Community Center".into(),
            city: "New York".into(),
            price: "Free".into(),
            url: "https://example.com/plumbing".into(),
            source: "test".into(),
            tags: vec!["Home Improvement".into()],
            description: "Learn basic plumbing repairs".into(),
        };

        let boring_score = score_event_relevance(&boring, &conn);
        assert!(boring_score < score, "Plumbing should score lower than jazz");
    }

    #[test]
    fn search_query_builder() {
        let conn = Connection::open_in_memory().unwrap();
        WorldGraph::ensure_tables(&conn);

        // seed_interests creates nodes from predefined categories
        WorldGraph::seed_interests(&conn, &["Music", "Technology"]);

        // Activate both parent interests
        if let Some(music) = WorldGraph::find_node(&conn, EntityType::Interest, "Music") {
            WorldGraph::activate(&conn, music.id, 0.5, "test");
        }
        if let Some(tech) = WorldGraph::find_node(&conn, EntityType::Interest, "Technology") {
            WorldGraph::activate(&conn, tech.id, 0.5, "test");
        }

        let queries = build_search_queries(&conn, "Bangalore");
        assert!(!queries.is_empty());

        // Should have music-specific query (Music sub-interests include "concerts")
        let has_concert = queries.iter().any(|q| q.to_lowercase().contains("concert"));
        // Should have tech-specific query
        let has_meetup = queries.iter().any(|q| q.to_lowercase().contains("meetup") || q.to_lowercase().contains("conference"));

        // At least one of these should be true since we seeded both categories
        assert!(has_concert || has_meetup, "Should generate relevant queries, got: {:?}", queries);
    }

    #[test]
    fn clean_html_entities() {
        assert_eq!(clean_html("<b>Bold</b> &amp; <i>italic</i>"), "Bold & italic");
        assert_eq!(clean_html("&lt;script&gt;alert(1)&lt;/script&gt;"), "<script>alert(1)</script>");
    }

    #[test]
    fn life_event_production() {
        let conn = Connection::open_in_memory().unwrap();
        WorldGraph::ensure_tables(&conn);
        WorldGraph::seed_interests(&conn, &["Technology"]);

        let xml = r#"<?xml version="1.0"?>
<rss version="2.0"><channel>
<item>
<title>AI Startup Demo Day</title>
<description>Tech startups showcase AI products at Innovation Hub. Free entry.</description>
<link>https://example.com/ai-demo</link>
<pubDate>Sat, 20 Mar 2026</pubDate>
<category>Technology</category>
</item>
</channel></rss>"#;

        let events = parse_event_feed(xml, "test");
        assert_eq!(events.len(), 1);

        let score = score_event_relevance(&events[0], &conn);
        assert!(score > 0.0, "Tech event should match technology interest");
    }
}
