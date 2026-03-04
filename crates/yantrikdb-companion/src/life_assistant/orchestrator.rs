//! Multi-Source Browser Orchestration + Structured Data Extraction.
//!
//! Searches multiple websites sequentially via CDP browser automation,
//! extracts structured data from page text using LLM + heuristic fallback, and
//! deduplicates results across sources.
//!
//! Tools: `search_sources`, `extract_search_results`, `rank_results`

use std::collections::HashMap;
use std::net::TcpStream;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tungstenite::{Message, WebSocket, stream::MaybeTlsStream};

use crate::tools::{Tool, ToolContext, ToolRegistry, PermissionLevel};

// ── CDP constants (same as browser.rs) ──

const CDP_HOST: &str = "127.0.0.1";
const CDP_PORT: u16 = 9222;
const MAX_TEXT_CHARS: usize = 6000;

/// Global CDP message counter for unique IDs (separate from browser.rs to avoid conflict).
static MSG_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(10_000);

// ── Data types ──

/// A data source to search (e.g., Google Maps, Yelp, Amazon).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataSource {
    /// Human-readable name (e.g., "google_maps", "yelp").
    pub name: String,
    /// URL template with `{query}` placeholder (e.g., "https://www.google.com/maps/search/{query}").
    pub search_url: String,
    /// Priority (lower = searched first).
    pub priority: u32,
}

/// Result from searching a single source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceResult {
    pub source_name: String,
    pub source_url: String,
    pub items: Vec<ExtractedItem>,
    pub search_duration_ms: u64,
    pub success: bool,
    pub error: Option<String>,
    /// Raw page text (truncated to MAX_TEXT_CHARS).
    pub raw_text: Option<String>,
}

/// A single extracted item (restaurant, person, product, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedItem {
    pub fields: HashMap<String, Value>,
    pub source: String,
    pub confidence: f64,
    pub raw_text: Option<String>,
}

/// Merged result after deduplication across sources.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergedResult {
    pub items: Vec<ExtractedItem>,
    pub sources_searched: Vec<String>,
    pub total_raw_results: usize,
    pub search_duration_ms: u64,
}

// ── CDP helpers (inline, same approach as browser.rs) ──

/// Fetch the WebSocket URL for the first open tab from CDP.
fn cdp_first_tab_ws() -> Result<String, String> {
    let url = format!("http://{}:{}/json", CDP_HOST, CDP_PORT);
    let output = std::process::Command::new("curl")
        .args(["-s", "--max-time", "3", &url])
        .output()
        .map_err(|e| format!("Cannot reach browser (curl): {e}"))?;

    if !output.status.success() {
        return Err("Browser not reachable. Use launch_browser first.".to_string());
    }

    let body = String::from_utf8_lossy(&output.stdout);
    let tabs: Vec<Value> = serde_json::from_str(&body)
        .map_err(|e| format!("Bad CDP response: {e}"))?;

    for tab in &tabs {
        if tab.get("type").and_then(|v| v.as_str()) == Some("page") {
            if let Some(ws) = tab.get("webSocketDebuggerUrl").and_then(|v| v.as_str()) {
                if !ws.is_empty() {
                    return Ok(ws.to_string());
                }
            }
        }
    }
    Err("No browser tabs open. Use launch_browser first.".to_string())
}

/// Connect to a CDP WebSocket.
fn cdp_connect(ws_url: &str) -> Result<WebSocket<MaybeTlsStream<TcpStream>>, String> {
    let (socket, _response) = tungstenite::connect(ws_url)
        .map_err(|e| format!("WS connect failed: {e}"))?;
    Ok(socket)
}

/// Send a CDP command and wait for the matching response.
fn cdp_send(
    ws: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let id = MSG_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let msg = json!({
        "id": id,
        "method": method,
        "params": params,
    });

    ws.send(Message::Text(msg.to_string()))
        .map_err(|e| format!("WS send error: {e}"))?;

    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if Instant::now() > deadline {
            return Err("CDP response timeout (15s)".to_string());
        }

        let msg = ws.read()
            .map_err(|e| format!("WS read error: {e}"))?;

        if let Message::Text(text) = msg {
            if let Ok(resp) = serde_json::from_str::<Value>(&text) {
                if resp.get("id").and_then(|v| v.as_u64()) == Some(id as u64) {
                    if let Some(err) = resp.get("error") {
                        return Err(format!("CDP error: {}", err));
                    }
                    return Ok(resp.get("result").cloned().unwrap_or(json!({})));
                }
            }
        }
    }
}

/// Evaluate JS in the browser and return the string result.
fn cdp_eval(
    ws: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    expression: &str,
) -> Result<String, String> {
    let result = cdp_send(ws, "Runtime.evaluate", json!({
        "expression": expression,
        "returnByValue": true,
    }))?;

    if let Some(exc) = result.get("exceptionDetails") {
        let text = exc.get("text").and_then(|v| v.as_str()).unwrap_or("JS error");
        return Err(format!("JS exception: {text}"));
    }

    let value = result
        .get("result")
        .and_then(|r| r.get("value"));

    match value {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(v) => Ok(v.to_string()),
        None => Ok(String::new()),
    }
}

/// Navigate to a URL via CDP.
fn cdp_navigate(
    ws: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    url: &str,
) -> Result<(), String> {
    cdp_send(ws, "Page.navigate", json!({ "url": url }))?;
    Ok(())
}

/// Get page text content via CDP.
fn cdp_get_text(
    ws: &mut WebSocket<MaybeTlsStream<TcpStream>>,
) -> Result<String, String> {
    cdp_eval(ws, "document.body?.innerText || ''")
}

/// Check if CDP is reachable.
fn is_browser_running() -> bool {
    TcpStream::connect_timeout(
        &format!("{}:{}", CDP_HOST, CDP_PORT).parse().unwrap(),
        Duration::from_millis(500),
    )
    .is_ok()
}

// ── URL encoding ──

/// Percent-encode a query string for use in URLs.
fn url_encode(input: &str) -> String {
    let mut encoded = String::with_capacity(input.len() * 3);
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            b' ' => encoded.push('+'),
            _ => {
                encoded.push('%');
                encoded.push_str(&format!("{:02X}", byte));
            }
        }
    }
    encoded
}

// ── Source search execution ──

/// Search a single source using the browser.
/// Called sequentially (one tab at a time to avoid resource exhaustion).
pub fn search_source(
    source_name: &str,
    search_url: &str,
) -> SourceResult {
    let start = Instant::now();

    // Check if CDP is running
    if !is_browser_running() {
        return SourceResult {
            source_name: source_name.to_string(),
            source_url: search_url.to_string(),
            items: Vec::new(),
            search_duration_ms: start.elapsed().as_millis() as u64,
            success: false,
            error: Some("Browser not running. Use launch_browser first.".to_string()),
            raw_text: None,
        };
    }

    // Connect to first tab
    let ws_url = match cdp_first_tab_ws() {
        Ok(u) => u,
        Err(e) => {
            return SourceResult {
                source_name: source_name.to_string(),
                source_url: search_url.to_string(),
                items: Vec::new(),
                search_duration_ms: start.elapsed().as_millis() as u64,
                success: false,
                error: Some(e),
                raw_text: None,
            };
        }
    };

    let mut ws = match cdp_connect(&ws_url) {
        Ok(w) => w,
        Err(e) => {
            return SourceResult {
                source_name: source_name.to_string(),
                source_url: search_url.to_string(),
                items: Vec::new(),
                search_duration_ms: start.elapsed().as_millis() as u64,
                success: false,
                error: Some(e),
                raw_text: None,
            };
        }
    };

    // Navigate to search URL
    if let Err(e) = cdp_navigate(&mut ws, search_url) {
        return SourceResult {
            source_name: source_name.to_string(),
            source_url: search_url.to_string(),
            items: Vec::new(),
            search_duration_ms: start.elapsed().as_millis() as u64,
            success: false,
            error: Some(format!("Navigation failed: {e}")),
            raw_text: None,
        };
    }

    // Wait for page load (2 seconds)
    std::thread::sleep(Duration::from_secs(2));

    // Get page text
    let page_text = match cdp_get_text(&mut ws) {
        Ok(text) => text,
        Err(e) => {
            return SourceResult {
                source_name: source_name.to_string(),
                source_url: search_url.to_string(),
                items: Vec::new(),
                search_duration_ms: start.elapsed().as_millis() as u64,
                success: false,
                error: Some(format!("Failed to read page: {e}")),
                raw_text: None,
            };
        }
    };

    // Truncate to MAX_TEXT_CHARS
    let truncated = if page_text.len() > MAX_TEXT_CHARS {
        page_text[..MAX_TEXT_CHARS].to_string()
    } else {
        page_text
    };

    SourceResult {
        source_name: source_name.to_string(),
        source_url: search_url.to_string(),
        items: Vec::new(), // Items populated later via extract tool
        search_duration_ms: start.elapsed().as_millis() as u64,
        success: true,
        error: None,
        raw_text: Some(truncated),
    }
}

/// Search multiple sources sequentially and collect results.
pub fn search_multiple_sources(
    query: &str,
    sources: &[DataSource],
    max_sources: usize,
) -> Vec<SourceResult> {
    // Sort sources by priority (lower = first)
    let mut sorted: Vec<&DataSource> = sources.iter().collect();
    sorted.sort_by_key(|s| s.priority);

    let mut results = Vec::new();
    let limit = max_sources.min(sorted.len());

    for source in sorted.iter().take(limit) {
        // Build search URL: replace {query} with URL-encoded query
        let encoded_query = url_encode(query);
        let search_url = source.search_url.replace("{query}", &encoded_query);

        tracing::info!(
            source = %source.name,
            url = %search_url,
            "Searching source"
        );

        let result = search_source(&source.name, &search_url);

        if !result.success {
            tracing::warn!(
                source = %source.name,
                error = ?result.error,
                "Source search failed, trying next"
            );
        }

        results.push(result);
    }

    results
}

// ── Text extraction heuristics ──

/// Extract structured items from page text using pattern matching.
///
/// This is a heuristic extractor. The LLM layer above will refine results.
/// It splits text into blocks, detects result items, and pulls out
/// requested fields using regex-like patterns.
fn extract_from_text(text: &str, fields: &str, source: &str, max: usize) -> String {
    let field_list: Vec<&str> = fields.split(',').map(|f| f.trim()).collect();

    // Split text into logical blocks by double newlines or separator patterns
    let blocks = split_into_blocks(text);

    let mut items: Vec<Value> = Vec::new();

    for block in blocks.iter().take(max * 3) {
        // Skip very short blocks (likely not a result item)
        if block.len() < 15 {
            continue;
        }

        let mut item = serde_json::Map::new();
        let mut has_content = false;

        for field in &field_list {
            let value = extract_field(block, field);
            if let Some(v) = value {
                item.insert(field.to_string(), v);
                has_content = true;
            }
        }

        if has_content {
            item.insert("source".to_string(), json!(source));
            item.insert("_raw_block".to_string(), json!(truncate_str(block, 200)));
            items.push(Value::Object(item));
        }

        if items.len() >= max {
            break;
        }
    }

    let result = json!({
        "extracted_items": items,
        "total_found": items.len(),
        "source": source,
        "fields_requested": field_list,
    });

    serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".to_string())
}

/// Split page text into logical blocks (candidate result items).
fn split_into_blocks(text: &str) -> Vec<String> {
    let mut blocks = Vec::new();

    // Primary split: double newlines
    for chunk in text.split("\n\n") {
        let trimmed = chunk.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Secondary split: if a chunk is very long, split by single newlines
        // and group consecutive short lines into blocks
        if trimmed.len() > 500 {
            let mut current_block = String::new();
            for line in trimmed.lines() {
                current_block.push_str(line);
                current_block.push('\n');
                // Heuristic: a blank-ish line or a line that looks like a separator
                // starts a new block
                if line.trim().is_empty()
                    || line.starts_with("---")
                    || line.starts_with("===")
                {
                    if !current_block.trim().is_empty() {
                        blocks.push(current_block.trim().to_string());
                    }
                    current_block = String::new();
                }
            }
            if !current_block.trim().is_empty() {
                blocks.push(current_block.trim().to_string());
            }
        } else {
            blocks.push(trimmed.to_string());
        }
    }

    blocks
}

/// Extract a single field from a text block using heuristic patterns.
fn extract_field(block: &str, field: &str) -> Option<Value> {
    match field.to_lowercase().as_str() {
        "name" | "title" => extract_name(block),
        "rating" | "stars" => extract_rating(block),
        "price" | "cost" => extract_price(block),
        "phone" | "telephone" | "tel" => extract_phone(block),
        "address" | "location" => extract_address(block),
        "url" | "link" | "href" => extract_url(block),
        "description" | "desc" | "summary" => extract_description(block),
        "reviews" | "review_count" => extract_review_count(block),
        "hours" | "open_hours" => extract_hours(block),
        _ => {
            // Generic: look for "field: value" or "field value" patterns
            extract_generic(block, field)
        }
    }
}

/// Extract name: first non-empty line, or first chunk before a number/rating.
fn extract_name(block: &str) -> Option<Value> {
    for line in block.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Skip lines that are just numbers or ratings
        if trimmed.chars().all(|c| c.is_ascii_digit() || c == '.' || c == '/' || c == ' ') {
            continue;
        }
        // Skip lines that look like metadata (all lowercase, short)
        if trimmed.len() < 3 {
            continue;
        }
        // Take the first meaningful line as the name, truncate to 100 chars
        let name = truncate_str(trimmed, 100);
        return Some(json!(name));
    }
    None
}

/// Extract rating: patterns like "4.5", "4.5/5", "4.5 stars", "X out of 5".
fn extract_rating(block: &str) -> Option<Value> {
    // Pattern: X.X/5 or X.X stars or (X.X) or X out of 5
    let text = block.to_lowercase();

    // "X.X/5" or "X/5"
    for word in text.split_whitespace() {
        if let Some(slash_pos) = word.find('/') {
            let num_part = &word[..slash_pos];
            let denom_part = word[slash_pos + 1..].trim_end_matches(|c: char| !c.is_ascii_digit() && c != '.');
            if let (Ok(num), Ok(_denom)) = (num_part.parse::<f64>(), denom_part.parse::<f64>()) {
                if (0.0..=5.0).contains(&num) {
                    return Some(json!(num));
                }
            }
        }
    }

    // "X.X stars" or "X stars"
    for (i, word) in text.split_whitespace().enumerate() {
        if word.starts_with("star") {
            // Look at previous word for the number
            let words: Vec<&str> = text.split_whitespace().collect();
            if i > 0 {
                if let Ok(num) = words[i - 1].parse::<f64>() {
                    if (0.0..=5.0).contains(&num) {
                        return Some(json!(num));
                    }
                }
            }
        }
    }

    // "X out of 5"
    if let Some(pos) = text.find("out of") {
        let before = &text[..pos].trim();
        if let Some(num_str) = before.split_whitespace().last() {
            if let Ok(num) = num_str.parse::<f64>() {
                if (0.0..=5.0).contains(&num) {
                    return Some(json!(num));
                }
            }
        }
    }

    // Standalone decimal that looks like a rating (e.g., "4.5" surrounded by non-digit chars)
    for word in text.split_whitespace() {
        let clean: String = word.chars().filter(|c| c.is_ascii_digit() || *c == '.').collect();
        if let Ok(num) = clean.parse::<f64>() {
            if (1.0..=5.0).contains(&num) && clean.contains('.') {
                return Some(json!(num));
            }
        }
    }

    None
}

/// Extract price: "$", "$$$$", "$XX.XX", price ranges.
fn extract_price(block: &str) -> Option<Value> {
    // Dollar sign patterns: "$$", "$$$", "$$$$"
    for word in block.split_whitespace() {
        let trimmed = word.trim_matches(|c: char| !c.is_ascii() && c != '$');
        if trimmed.starts_with('$') && trimmed.chars().all(|c| c == '$') && trimmed.len() <= 4 {
            return Some(json!(trimmed));
        }
    }

    // "$XX.XX" pattern
    for word in block.split_whitespace() {
        if word.starts_with('$') {
            let num_part: String = word[1..].chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == ',')
                .collect();
            if !num_part.is_empty() {
                return Some(json!(format!("${}", num_part)));
            }
        }
    }

    // "XX dollars" or price keywords
    let lower = block.to_lowercase();
    for keyword in &["price:", "cost:", "from $", "starting at $"] {
        if let Some(pos) = lower.find(keyword) {
            let after = &block[pos + keyword.len()..];
            let value: String = after.trim().chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '$' || *c == ',')
                .collect();
            if !value.is_empty() {
                let formatted = if value.starts_with('$') { value } else { format!("${}", value) };
                return Some(json!(formatted));
            }
        }
    }

    None
}

/// Extract phone number: common formats (XXX-XXX-XXXX, (XXX) XXX-XXXX, etc.)
fn extract_phone(block: &str) -> Option<Value> {
    // Collect all digit sequences and reconstruct
    let chars: Vec<char> = block.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // Look for phone-like patterns: starts with ( or digit
        if chars[i] == '(' || chars[i].is_ascii_digit() {
            // Collect a window of chars that could be a phone number
            let start = i;
            let mut digits = 0;
            let mut j = i;
            while j < len && j - start < 20 {
                if chars[j].is_ascii_digit() {
                    digits += 1;
                } else if !matches!(chars[j], '-' | '(' | ')' | ' ' | '.') {
                    break;
                }
                j += 1;
            }
            // US phone numbers have 10 or 11 digits
            if digits >= 10 && digits <= 11 {
                let phone: String = chars[start..j].iter().collect();
                return Some(json!(phone.trim()));
            }
        }
        i += 1;
    }

    None
}

/// Extract address: look for street number + street name patterns.
fn extract_address(block: &str) -> Option<Value> {
    // Address keywords
    let lower = block.to_lowercase();
    for keyword in &["address:", "located at", "location:"] {
        if let Some(pos) = lower.find(keyword) {
            let after = &block[pos + keyword.len()..];
            let addr: String = after.lines().next().unwrap_or("").trim().to_string();
            if addr.len() > 5 {
                return Some(json!(truncate_str(&addr, 150)));
            }
        }
    }

    // Pattern: line starting with a number followed by text (likely a street address)
    for line in block.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Check if line starts with digits (house number) and has typical address words
        let first_char = trimmed.chars().next().unwrap_or(' ');
        if first_char.is_ascii_digit() {
            let lower_line = trimmed.to_lowercase();
            let address_words = ["st", "ave", "blvd", "rd", "dr", "ln", "ct", "way",
                                 "street", "avenue", "boulevard", "road", "drive",
                                 "lane", "court", "plaza", "pkwy", "hwy"];
            if address_words.iter().any(|w| lower_line.contains(w)) {
                return Some(json!(truncate_str(trimmed, 150)));
            }
        }
    }

    None
}

/// Extract URL from block text.
fn extract_url(block: &str) -> Option<Value> {
    for word in block.split_whitespace() {
        if word.starts_with("http://") || word.starts_with("https://") {
            let clean: String = word.chars()
                .take_while(|c| !c.is_whitespace() && *c != '>' && *c != '"' && *c != '\'')
                .collect();
            return Some(json!(clean));
        }
    }
    // Also look for "www." prefixed URLs
    for word in block.split_whitespace() {
        if word.starts_with("www.") {
            let clean: String = word.chars()
                .take_while(|c| !c.is_whitespace() && *c != '>' && *c != '"')
                .collect();
            return Some(json!(format!("https://{}", clean)));
        }
    }
    None
}

/// Extract description: first substantial line that is not a name.
fn extract_description(block: &str) -> Option<Value> {
    let lines: Vec<&str> = block.lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();

    // Skip the first line (likely name), take the longest remaining line
    if lines.len() > 1 {
        let desc = lines[1..].iter()
            .max_by_key(|l| l.len())
            .unwrap_or(&"");
        if desc.len() > 10 {
            return Some(json!(truncate_str(desc, 300)));
        }
    }

    None
}

/// Extract review count: "X reviews", "X ratings", "(X)".
fn extract_review_count(block: &str) -> Option<Value> {
    let lower = block.to_lowercase();
    let words: Vec<&str> = lower.split_whitespace().collect();

    for (i, word) in words.iter().enumerate() {
        if word.starts_with("review") || word.starts_with("rating") {
            if i > 0 {
                let num_str: String = words[i - 1].chars()
                    .filter(|c| c.is_ascii_digit())
                    .collect();
                if let Ok(n) = num_str.parse::<u64>() {
                    return Some(json!(n));
                }
            }
        }
    }

    // "(123)" pattern often used for review counts
    if let Some(open) = block.find('(') {
        if let Some(close) = block[open..].find(')') {
            let inner = &block[open + 1..open + close];
            let num_str: String = inner.chars().filter(|c| c.is_ascii_digit() || *c == ',').collect();
            let clean = num_str.replace(',', "");
            if let Ok(n) = clean.parse::<u64>() {
                if n > 0 && n < 1_000_000 {
                    return Some(json!(n));
                }
            }
        }
    }

    None
}

/// Extract hours: look for time patterns.
fn extract_hours(block: &str) -> Option<Value> {
    let lower = block.to_lowercase();
    for keyword in &["hours:", "open:", "hours of operation", "open now", "closed"] {
        if let Some(pos) = lower.find(keyword) {
            let after = &block[pos..];
            let hours: String = after.lines().next().unwrap_or("").trim().to_string();
            if !hours.is_empty() {
                return Some(json!(truncate_str(&hours, 100)));
            }
        }
    }
    None
}

/// Generic field extraction: look for "field: value" or "field value" patterns.
fn extract_generic(block: &str, field: &str) -> Option<Value> {
    let lower = block.to_lowercase();
    let field_lower = field.to_lowercase();

    // "field: value" pattern
    let pattern = format!("{}:", field_lower);
    if let Some(pos) = lower.find(&pattern) {
        let after = &block[pos + pattern.len()..];
        let value: String = after.lines().next().unwrap_or("").trim().to_string();
        if !value.is_empty() {
            return Some(json!(truncate_str(&value, 200)));
        }
    }

    // "field = value" pattern
    let pattern = format!("{} =", field_lower);
    if let Some(pos) = lower.find(&pattern) {
        let after = &block[pos + pattern.len()..];
        let value: String = after.lines().next().unwrap_or("").trim().to_string();
        if !value.is_empty() {
            return Some(json!(truncate_str(&value, 200)));
        }
    }

    None
}

/// Truncate a string at a char boundary.
fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        return s.to_string();
    }
    let mut boundary = max_len;
    while boundary > 0 && !s.is_char_boundary(boundary) {
        boundary -= 1;
    }
    format!("{}...", &s[..boundary])
}

// ── Result deduplication ──

/// Deduplicate results from multiple sources.
///
/// Uses simple name-based matching: normalize names (lowercase, remove articles,
/// trim punctuation), then check if Levenshtein distance < 3 or one contains the other.
pub fn deduplicate_results(all_results: &[SourceResult]) -> MergedResult {
    let start = Instant::now();
    let mut all_items: Vec<ExtractedItem> = Vec::new();
    let mut sources_searched: Vec<String> = Vec::new();
    let mut total_raw = 0;

    // Flatten all items from all sources
    for result in all_results {
        if !sources_searched.contains(&result.source_name) {
            sources_searched.push(result.source_name.clone());
        }
        total_raw += result.items.len();
        for item in &result.items {
            all_items.push(item.clone());
        }
    }

    // Group by similarity
    let mut merged: Vec<ExtractedItem> = Vec::new();
    let mut used: Vec<bool> = vec![false; all_items.len()];

    for i in 0..all_items.len() {
        if used[i] {
            continue;
        }
        used[i] = true;

        let mut group = vec![&all_items[i]];

        // Find duplicates
        for j in (i + 1)..all_items.len() {
            if used[j] {
                continue;
            }
            if items_are_similar(&all_items[i], &all_items[j]) {
                used[j] = true;
                group.push(&all_items[j]);
            }
        }

        // Merge group into a single item
        let merged_item = merge_group(&group);
        merged.push(merged_item);
    }

    // Sort by confidence (items found in multiple sources rank higher)
    merged.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal));

    MergedResult {
        items: merged,
        sources_searched,
        total_raw_results: total_raw,
        search_duration_ms: start.elapsed().as_millis() as u64,
    }
}

/// Check if two extracted items are similar (likely the same entity).
fn items_are_similar(a: &ExtractedItem, b: &ExtractedItem) -> bool {
    let name_a = a.fields.get("name")
        .or_else(|| a.fields.get("title"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let name_b = b.fields.get("name")
        .or_else(|| b.fields.get("title"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if name_a.is_empty() || name_b.is_empty() {
        return false;
    }

    let norm_a = normalize_name(name_a);
    let norm_b = normalize_name(name_b);

    if norm_a == norm_b {
        return true;
    }

    // One contains the other
    if norm_a.contains(&norm_b) || norm_b.contains(&norm_a) {
        return true;
    }

    // Levenshtein distance < 3
    levenshtein(&norm_a, &norm_b) < 3
}

/// Normalize a name for comparison: lowercase, remove common articles,
/// trim punctuation and whitespace.
fn normalize_name(name: &str) -> String {
    let lower = name.to_lowercase();
    let stripped: String = lower.chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect();
    let words: Vec<&str> = stripped.split_whitespace()
        .filter(|w| !matches!(*w, "the" | "a" | "an" | "and" | "&"))
        .collect();
    words.join(" ")
}

/// Simple Levenshtein distance (edit distance) between two strings.
fn levenshtein(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let m = a_chars.len();
    let n = b_chars.len();

    if m == 0 { return n; }
    if n == 0 { return m; }

    // Use two-row optimization to save memory
    let mut prev = vec![0usize; n + 1];
    let mut curr = vec![0usize; n + 1];

    for j in 0..=n {
        prev[j] = j;
    }

    for i in 1..=m {
        curr[0] = i;
        for j in 1..=n {
            let cost = if a_chars[i - 1] == b_chars[j - 1] { 0 } else { 1 };
            curr[j] = (prev[j] + 1)
                .min(curr[j - 1] + 1)
                .min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[n]
}

/// Merge a group of similar items into one, preferring the source with more data.
fn merge_group(group: &[&ExtractedItem]) -> ExtractedItem {
    if group.len() == 1 {
        return group[0].clone();
    }

    // Pick the item with the most fields as the base
    let base = group.iter()
        .max_by_key(|item| item.fields.len())
        .unwrap();

    let mut merged_fields = base.fields.clone();

    // Merge fields from other items (fill in missing fields)
    for item in group {
        for (key, value) in &item.fields {
            if !merged_fields.contains_key(key) || merged_fields[key].is_null() {
                merged_fields.insert(key.clone(), value.clone());
            }
        }
    }

    // Average ratings if multiple sources have them
    let ratings: Vec<f64> = group.iter()
        .filter_map(|item| item.fields.get("rating").and_then(|v| v.as_f64()))
        .collect();
    if ratings.len() > 1 {
        let avg = ratings.iter().sum::<f64>() / ratings.len() as f64;
        let rounded = (avg * 10.0).round() / 10.0;
        merged_fields.insert("rating".to_string(), json!(rounded));
    }

    // Note all sources
    let sources: Vec<String> = group.iter()
        .map(|item| item.source.clone())
        .collect();
    merged_fields.insert("_sources".to_string(), json!(sources));

    // Confidence boost for items found in multiple sources
    let base_confidence = base.confidence;
    let multi_source_boost = (group.len() as f64 - 1.0) * 0.15;
    let confidence = (base_confidence + multi_source_boost).min(1.0);

    ExtractedItem {
        fields: merged_fields,
        source: sources.join(", "),
        confidence,
        raw_text: base.raw_text.clone(),
    }
}

// ── Tool: search_sources ──

pub struct SearchSourcesTool;

impl Tool for SearchSourcesTool {
    fn name(&self) -> &'static str { "search_sources" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "life_assistant" }

    fn definition(&self) -> serde_json::Value {
        json!({
            "type": "function",
            "function": {
                "name": "search_sources",
                "description": "Search multiple websites for a query using the browser. Navigates to each source URL sequentially, waits for page load, and returns the raw page text from each. Use this for real-world lookups: restaurants, products, people, services. After getting results, use extract_search_results to parse structured data from the page text.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "What to search for (e.g., 'best pizza near downtown Seattle')"
                        },
                        "sources": {
                            "type": "array",
                            "description": "Sources to search. Each has name, search_url (with {query} placeholder), and priority (lower=first).",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "name": {"type": "string", "description": "Source identifier (e.g., 'google_maps')"},
                                    "search_url": {"type": "string", "description": "URL template with {query} placeholder"},
                                    "priority": {"type": "integer", "description": "Priority (1=highest, searched first)"}
                                },
                                "required": ["name", "search_url"]
                            }
                        },
                        "max_sources": {
                            "type": "integer",
                            "description": "Max number of sources to search (default: 3)"
                        }
                    },
                    "required": ["query", "sources"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let query = match args.get("query").and_then(|v| v.as_str()) {
            Some(q) if !q.is_empty() => q,
            _ => return json!({"error": "query is required"}).to_string(),
        };

        let sources: Vec<DataSource> = match args.get("sources").and_then(|v| v.as_array()) {
            Some(arr) => {
                arr.iter().filter_map(|s| {
                    let name = s.get("name").and_then(|v| v.as_str())?;
                    let search_url = s.get("search_url").and_then(|v| v.as_str())?;
                    let priority = s.get("priority").and_then(|v| v.as_u64()).unwrap_or(10) as u32;
                    Some(DataSource {
                        name: name.to_string(),
                        search_url: search_url.to_string(),
                        priority,
                    })
                }).collect()
            }
            None => return json!({"error": "sources array is required"}).to_string(),
        };

        if sources.is_empty() {
            return json!({"error": "at least one source is required"}).to_string();
        }

        let max_sources = args.get("max_sources")
            .and_then(|v| v.as_u64())
            .unwrap_or(3) as usize;

        let start = Instant::now();
        let results = search_multiple_sources(query, &sources, max_sources);

        // Build response
        let response: Vec<Value> = results.iter().map(|r| {
            json!({
                "source": r.source_name,
                "url": r.source_url,
                "success": r.success,
                "duration_ms": r.search_duration_ms,
                "error": r.error,
                "page_text": r.raw_text,
                "text_length": r.raw_text.as_ref().map(|t| t.len()).unwrap_or(0),
            })
        }).collect();

        let output = json!({
            "query": query,
            "total_sources_searched": results.len(),
            "successful": results.iter().filter(|r| r.success).count(),
            "total_duration_ms": start.elapsed().as_millis() as u64,
            "results": response,
        });

        serde_json::to_string_pretty(&output).unwrap_or_else(|_| "{}".to_string())
    }
}

// ── LLM-powered extraction ──

/// Extract structured data from page text using an Ollama LLM call.
/// Falls back to heuristic `extract_from_text()` if the LLM is unavailable or fails.
fn llm_extract(
    ollama_base: &str,
    model: &str,
    page_text: &str,
    fields: &str,
    source: &str,
    max: usize,
) -> String {
    // Truncate page text to avoid token limits
    let truncated = if page_text.len() > 4000 {
        &page_text[..page_text.floor_char_boundary(4000)]
    } else {
        page_text
    };

    let prompt = format!(
        r#"Extract structured data from this web page text. Return a JSON array of items.
Each item should be a JSON object with these fields: {fields}
Source: {source}
Return ONLY valid JSON: [{{"field1": "value", ...}}, ...]
Max {max} items. Skip items with no useful data. If no items can be extracted, return [].

Page text:
{truncated}"#,
        fields = fields,
        source = source,
        max = max,
        truncated = truncated,
    );

    let payload = json!({
        "model": model,
        "messages": [{
            "role": "user",
            "content": prompt
        }],
        "stream": false,
        "options": {
            "temperature": 0.0
        }
    });

    let payload_path = "/tmp/yantrik-extract-payload.json";
    if let Err(e) = std::fs::write(payload_path, payload.to_string()) {
        tracing::warn!("Failed to write LLM payload: {e}, falling back to heuristic");
        return extract_from_text(page_text, fields, source, max);
    }

    let url = format!("{}/api/chat", ollama_base);
    let output = match std::process::Command::new("curl")
        .args([
            "-fsSL",
            "--max-time", "60",
            "-H", "Content-Type: application/json",
            "-d", &format!("@{payload_path}"),
            &url,
        ])
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!("LLM curl failed: {e}, falling back to heuristic");
            let _ = std::fs::remove_file(payload_path);
            return extract_from_text(page_text, fields, source, max);
        }
    };

    let _ = std::fs::remove_file(payload_path);

    if !output.status.success() {
        tracing::warn!("LLM request failed, falling back to heuristic");
        return extract_from_text(page_text, fields, source, max);
    }

    let response: Value = match serde_json::from_slice(&output.stdout) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("LLM response not JSON: {e}, falling back to heuristic");
            return extract_from_text(page_text, fields, source, max);
        }
    };

    let content = match response["message"]["content"].as_str() {
        Some(c) => c.trim(),
        None => {
            tracing::warn!("No content in LLM response, falling back to heuristic");
            return extract_from_text(page_text, fields, source, max);
        }
    };

    // Strip markdown code block if present
    let json_str = if content.starts_with("```") {
        content.lines()
            .skip(1)
            .take_while(|l| !l.starts_with("```"))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        content.to_string()
    };

    // Validate it's a JSON array
    let items: Vec<Value> = match serde_json::from_str(&json_str) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("LLM returned invalid JSON array: {e}, falling back to heuristic");
            return extract_from_text(page_text, fields, source, max);
        }
    };

    // Format into structured output matching the heuristic format
    let count = items.len().min(max);
    let taken: Vec<Value> = items.into_iter().take(max).collect();
    let result = json!({
        "extracted_items": taken,
        "total_found": count,
        "source": source,
        "fields_requested": fields.split(',').map(|f| f.trim()).collect::<Vec<_>>(),
        "extraction_method": "llm",
    });

    serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".to_string())
}

// ── Tool: extract_search_results ──

pub struct ExtractResultsTool {
    pub ollama_base: String,
    pub model: String,
}

impl Tool for ExtractResultsTool {
    fn name(&self) -> &'static str { "extract_search_results" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "life_assistant" }

    fn definition(&self) -> serde_json::Value {
        json!({
            "type": "function",
            "function": {
                "name": "extract_search_results",
                "description": "Extract structured data from web page text using AI. Used after browsing a search results page (via search_sources or browse). Provide the raw page text and the fields you want to extract. Returns a JSON array of items with the requested fields.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "page_text": {
                            "type": "string",
                            "description": "Raw text from the web page"
                        },
                        "source_name": {
                            "type": "string",
                            "description": "Which source this is from (e.g., 'google_maps', 'yelp')"
                        },
                        "extract_fields": {
                            "type": "string",
                            "description": "Comma-separated field names to extract. Common fields: name, rating, price, address, phone, description, reviews, hours, url"
                        },
                        "max_items": {
                            "type": "integer",
                            "description": "Max items to extract (default: 5)"
                        }
                    },
                    "required": ["page_text", "extract_fields"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let page_text = match args.get("page_text").and_then(|v| v.as_str()) {
            Some(t) if !t.is_empty() => t,
            _ => return json!({"error": "page_text is required"}).to_string(),
        };

        let fields = match args.get("extract_fields").and_then(|v| v.as_str()) {
            Some(f) if !f.is_empty() => f,
            _ => return json!({"error": "extract_fields is required"}).to_string(),
        };

        let source = args.get("source_name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        let max = args.get("max_items")
            .and_then(|v| v.as_u64())
            .unwrap_or(5) as usize;

        // Try LLM extraction first, falls back to heuristic internally
        llm_extract(&self.ollama_base, &self.model, page_text, fields, source, max)
    }
}

// ── Tool: deduplicate_search_results ──

pub struct DeduplicateResultsTool;

impl Tool for DeduplicateResultsTool {
    fn name(&self) -> &'static str { "deduplicate_results" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "life_assistant" }

    fn definition(&self) -> serde_json::Value {
        json!({
            "type": "function",
            "function": {
                "name": "deduplicate_results",
                "description": "Merge and deduplicate extracted items from multiple sources. Items with similar names are merged, ratings are averaged, and multi-source items get a confidence boost. Pass the JSON array of items from extract_search_results calls.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "items": {
                            "type": "string",
                            "description": "JSON array of extracted items (from extract_search_results). Each item must have at least a 'name' or 'title' field and a 'source' field."
                        }
                    },
                    "required": ["items"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let items_json = match args.get("items").and_then(|v| v.as_str()) {
            Some(j) if !j.is_empty() => j,
            _ => return json!({"error": "items JSON string is required"}).to_string(),
        };

        // Parse the items array
        let items_array: Vec<Value> = match serde_json::from_str(items_json) {
            Ok(v) => v,
            Err(e) => return json!({"error": format!("Invalid JSON: {e}")}).to_string(),
        };

        // Convert to ExtractedItem structs
        let mut source_results: Vec<SourceResult> = Vec::new();

        // Group items by source
        let mut by_source: HashMap<String, Vec<ExtractedItem>> = HashMap::new();
        for item_val in &items_array {
            let source = item_val.get("source")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();

            let fields: HashMap<String, Value> = match item_val.as_object() {
                Some(obj) => obj.iter()
                    .filter(|(k, _)| k.as_str() != "_raw_block")
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
                None => continue,
            };

            let item = ExtractedItem {
                fields,
                source: source.clone(),
                confidence: 0.5, // Base confidence for heuristic extraction
                raw_text: item_val.get("_raw_block").and_then(|v| v.as_str()).map(|s| s.to_string()),
            };

            by_source.entry(source).or_default().push(item);
        }

        for (name, items) in by_source {
            source_results.push(SourceResult {
                source_name: name.clone(),
                source_url: String::new(),
                items,
                search_duration_ms: 0,
                success: true,
                error: None,
                raw_text: None,
            });
        }

        let merged = deduplicate_results(&source_results);

        let output = json!({
            "merged_items": merged.items.iter().map(|item| {
                let mut obj = serde_json::Map::new();
                for (k, v) in &item.fields {
                    obj.insert(k.clone(), v.clone());
                }
                obj.insert("confidence".to_string(), json!(item.confidence));
                obj.insert("source".to_string(), json!(item.source));
                Value::Object(obj)
            }).collect::<Vec<_>>(),
            "sources_searched": merged.sources_searched,
            "total_raw_results": merged.total_raw_results,
            "merged_count": merged.items.len(),
        });

        serde_json::to_string_pretty(&output).unwrap_or_else(|_| "{}".to_string())
    }
}

// ── Tool: rank_results ──

pub struct RankResultsTool;

impl Tool for RankResultsTool {
    fn name(&self) -> &'static str { "rank_results" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "life_assistant" }

    fn definition(&self) -> serde_json::Value {
        json!({
            "type": "function",
            "function": {
                "name": "rank_results",
                "description": "Rank and score extracted search results using weighted criteria. Pass the items from extract_search_results or deduplicate_results. Optionally specify the task type (to use template ranking config) and user priorities to boost specific factors.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "items": {
                            "type": "string",
                            "description": "JSON array of items to rank (from extraction/dedup)"
                        },
                        "task_type": {
                            "type": "string",
                            "description": "Task type for template ranking config (e.g., 'find_restaurant')"
                        },
                        "user_priorities": {
                            "type": "string",
                            "description": "Comma-separated priorities to boost (e.g., 'rating,price')"
                        },
                        "max_results": {
                            "type": "integer",
                            "description": "Max results to return (default: 5)"
                        }
                    },
                    "required": ["items"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let items_json = match args.get("items").and_then(|v| v.as_str()) {
            Some(j) if !j.is_empty() => j,
            _ => return json!({"error": "items JSON string is required"}).to_string(),
        };

        let items: Vec<Value> = match serde_json::from_str(items_json) {
            Ok(v) => v,
            Err(e) => return json!({"error": format!("Invalid JSON: {e}")}).to_string(),
        };

        let task_type = args.get("task_type").and_then(|v| v.as_str());
        let user_priorities: Vec<&str> = args.get("user_priorities")
            .and_then(|v| v.as_str())
            .map(|s| s.split(',').map(|p| p.trim()).collect())
            .unwrap_or_default();

        let max_results = args.get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(5) as usize;

        // Get ranking config from template if available
        let registry = super::TaskTemplateRegistry::new();
        let template_ranking = task_type.and_then(|tt| registry.get(tt)).map(|t| &t.ranking);

        // Default ranking factors
        let default_factors: Vec<(&str, f64, bool)> = vec![
            ("rating", 0.4, false),    // higher is better
            ("price", 0.2, true),      // lower is better (inverted)
            ("reviews", 0.2, false),   // higher is better
            ("distance", 0.1, true),   // lower is better (inverted)
            ("confidence", 0.1, false), // higher is better
        ];

        // Score each item
        let mut scored: Vec<(Value, f64, Vec<String>)> = items.into_iter().map(|item| {
            let mut total_score = 0.0;
            let mut total_weight = 0.0;
            let mut explanations: Vec<String> = Vec::new();

            // Use template factors if available, otherwise defaults
            if let Some(ranking) = template_ranking {
                for factor in &ranking.factors {
                    let is_inverted = matches!(factor.order, super::SortOrder::Ascending);
                    let weight = if user_priorities.contains(&factor.field.as_str()) {
                        factor.weight * 1.5
                    } else {
                        factor.weight
                    };

                    if let Some(score) = score_field(&item, &factor.field, is_inverted) {
                        total_score += score * weight;
                        total_weight += weight;
                        explanations.push(format!("{}={:.1}×{:.1}", factor.field, score, weight));
                    }
                }
            } else {
                for (field, weight, inverted) in &default_factors {
                    let w = if user_priorities.contains(field) {
                        weight * 1.5
                    } else {
                        *weight
                    };

                    if let Some(score) = score_field(&item, field, *inverted) {
                        total_score += score * w;
                        total_weight += w;
                        explanations.push(format!("{}={:.1}×{:.1}", field, score, w));
                    }
                }
            }

            let final_score = if total_weight > 0.0 {
                total_score / total_weight
            } else {
                0.5 // neutral if no factors matched
            };

            (item, final_score, explanations)
        }).collect();

        // Sort by score descending
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Build output
        let ranked: Vec<Value> = scored.into_iter().take(max_results).enumerate().map(|(i, (item, score, expl))| {
            let mut obj = item.as_object().cloned().unwrap_or_default();
            obj.insert("_rank".to_string(), json!(i + 1));
            obj.insert("_score".to_string(), json!((score * 100.0).round() / 100.0));
            obj.insert("_scoring".to_string(), json!(expl.join(", ")));
            Value::Object(obj)
        }).collect();

        let output = json!({
            "ranked_items": ranked,
            "total_scored": ranked.len(),
            "task_type": task_type.unwrap_or("generic"),
            "user_priorities": user_priorities,
        });

        serde_json::to_string_pretty(&output).unwrap_or_else(|_| "{}".to_string())
    }
}

/// Score a single field from an item, normalizing to 0.0-1.0.
fn score_field(item: &Value, field: &str, inverted: bool) -> Option<f64> {
    let val = item.get(field)?;

    let raw_score = if let Some(n) = val.as_f64() {
        match field {
            "rating" | "stars" => n / 5.0,          // normalize X/5 to 0-1
            "reviews" | "review_count" => {
                // Log-scale: 1 review = 0, 1000 reviews = ~1.0
                (n.max(1.0).ln() / 1000_f64.ln()).min(1.0)
            }
            "confidence" => n,
            _ => n.min(1.0),
        }
    } else if let Some(s) = val.as_str() {
        match field {
            "price" => {
                // Dollar signs: $ = 1.0, $$ = 0.75, $$$ = 0.5, $$$$ = 0.25
                let dollars = s.chars().filter(|c| *c == '$').count();
                if dollars > 0 {
                    Some(1.0 - (dollars as f64 - 1.0) * 0.25).map(|v| v.max(0.0))
                        .unwrap_or(0.5)
                } else if let Ok(n) = s.trim_start_matches('$').replace(',', "").parse::<f64>() {
                    // Numeric price: normalize to 0-1 where $0=1.0, $100=0.0
                    (1.0 - n / 100.0).max(0.0).min(1.0)
                } else {
                    return None;
                }
            }
            "rating" => {
                // String rating like "4.5"
                s.parse::<f64>().ok().map(|n| n / 5.0)?
            }
            _ => return None,
        }
    } else {
        return None;
    };

    Some(if inverted { 1.0 - raw_score } else { raw_score })
}

// ── Registration ──

pub fn register(reg: &mut ToolRegistry, ollama_base: &str, model: &str) {
    reg.register(Box::new(SearchSourcesTool));
    reg.register(Box::new(ExtractResultsTool {
        ollama_base: ollama_base.to_string(),
        model: model.to_string(),
    }));
    reg.register(Box::new(DeduplicateResultsTool));
    reg.register(Box::new(RankResultsTool));
}
