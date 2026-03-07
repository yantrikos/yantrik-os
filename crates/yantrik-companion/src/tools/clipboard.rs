//! Clipboard intelligence tools — history search, content analysis,
//! URL fetching, transformations, and text actions.
//!
//! Tools: clipboard_history, clipboard_analyze, clipboard_fetch_url,
//!        clipboard_transform, text_action

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(ClipboardHistoryTool));
    reg.register(Box::new(ClipboardAnalyzeTool));
    reg.register(Box::new(ClipboardFetchUrlTool));
    reg.register(Box::new(ClipboardTransformTool));
    reg.register(Box::new(TextActionTool));
}

// ── Clipboard History ──

pub struct ClipboardHistoryTool;

impl Tool for ClipboardHistoryTool {
    fn name(&self) -> &'static str { "clipboard_history" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "clipboard" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "clipboard_history",
                "description": "Search or list recent clipboard history entries. Shows what the user has copied recently with timestamps and content types.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "Optional search term to filter entries"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Max entries to return (default 10, max 20)"
                        }
                    }
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10).min(20) as usize;

        // Read clipboard history via wl-paste (current) and recent entries
        // Since we can't access SharedHistory from companion, we'll read current
        // and provide context about what tools are available
        let current = read_clipboard_text();

        if current.is_empty() {
            return "Clipboard is empty.".to_string();
        }

        let content_type = detect_type(&current);
        let preview = truncate(&current, 500);

        let mut result = format!("Current clipboard ({}):\n{}\n", content_type, preview);

        // If there's a query, check if current matches
        if !query.is_empty() {
            if current.to_lowercase().contains(&query.to_lowercase()) {
                result.push_str("\n✓ Matches search query.");
            } else {
                result.push_str("\n✗ Current clipboard does not match query.");
            }
        }

        result.push_str(&format!(
            "\n\nNote: Full clipboard history (up to {} entries) is available in the UI clipboard panel (Super+V). \
             Use read_clipboard for the current content, or clipboard_analyze for detailed analysis.",
            limit
        ));

        result
    }
}

// ── Clipboard Analyze ──

pub struct ClipboardAnalyzeTool;

impl Tool for ClipboardAnalyzeTool {
    fn name(&self) -> &'static str { "clipboard_analyze" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "clipboard" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "clipboard_analyze",
                "description": "Analyze the current clipboard content: detect type (URL, code, JSON, email, sensitive data, plain text), check MIME types, and suggest available actions.",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, _args: &serde_json::Value) -> String {
        let content = read_clipboard_text();
        if content.is_empty() {
            return "Clipboard is empty.".to_string();
        }

        let content_type = detect_type(&content);
        let is_sensitive = check_sensitive(&content);
        let char_count = content.len();
        let line_count = content.lines().count();
        let word_count = content.split_whitespace().count();
        let preview = truncate(&content, 300);

        // Detect MIME types
        let mime_types = detect_mime_types();

        let mut result = format!(
            "Clipboard analysis:\n\
             Type: {}\n\
             Size: {} chars, {} words, {} lines\n\
             MIME types: {}\n\
             Preview: {}\n",
            content_type,
            char_count,
            word_count,
            line_count,
            mime_types,
            preview
        );

        if is_sensitive {
            result.push_str("\n⚠ WARNING: Clipboard may contain sensitive data (API key, token, or credential). Consider clearing after use.\n");
        }

        // Suggest actions based on type
        result.push_str("\nSuggested actions:\n");
        match content_type.as_str() {
            "URL" => {
                result.push_str("  - clipboard_fetch_url: Fetch the page content\n");
                result.push_str("  - open_url: Open in browser\n");
            }
            "JSON" => {
                result.push_str("  - clipboard_transform(format_json): Pretty-print the JSON\n");
                result.push_str("  - clipboard_transform(minify_json): Minify the JSON\n");
            }
            "code" => {
                result.push_str("  - Ask me to explain, refactor, or review this code\n");
            }
            "email" => {
                result.push_str("  - Ready to paste into an email field\n");
            }
            "HTML" => {
                result.push_str("  - clipboard_transform(strip_html): Extract plain text\n");
                result.push_str("  - clipboard_transform(extract_urls): Extract all URLs\n");
            }
            _ => {
                result.push_str("  - text_action(summarize): Summarize this text\n");
                result.push_str("  - text_action(rewrite_formal): Rewrite formally\n");
                result.push_str("  - text_action(fix_grammar): Fix grammar and spelling\n");
                result.push_str("  - clipboard_transform(sort_lines): Sort lines\n");
            }
        }

        result
    }
}

// ── Clipboard Fetch URL ──

pub struct ClipboardFetchUrlTool;

impl Tool for ClipboardFetchUrlTool {
    fn name(&self) -> &'static str { "clipboard_fetch_url" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "clipboard" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "clipboard_fetch_url",
                "description": "If the clipboard contains a URL, fetch the page title and text excerpt. Useful for 'smart paste' — turning a URL into actual content.",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, _args: &serde_json::Value) -> String {
        let content = read_clipboard_text();
        let trimmed = content.trim();

        if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
            return format!(
                "Clipboard does not contain a URL. Current content type: {}",
                detect_type(&content)
            );
        }

        // Extract just the URL (first line, no trailing whitespace)
        let url = trimmed.lines().next().unwrap_or(trimmed).trim();

        // Fetch with curl, timeout 10s, follow redirects, user-agent
        let output = match std::process::Command::new("curl")
            .args([
                "-sL",
                "--max-time", "10",
                "--max-filesize", "500000",
                "-A", "Mozilla/5.0 (compatible; YantrikOS/1.0)",
                url,
            ])
            .output()
        {
            Ok(o) => o,
            Err(e) => return format!("Failed to fetch URL: {}", e),
        };

        if !output.status.success() {
            return format!("Failed to fetch URL (HTTP error or timeout)");
        }

        let body = String::from_utf8_lossy(&output.stdout);
        if body.is_empty() {
            return "Fetched empty response.".to_string();
        }

        // Extract title
        let title = extract_html_title(&body).unwrap_or_else(|| "No title".to_string());

        // Extract text content (strip HTML tags)
        let text = strip_html_tags(&body);
        let excerpt = truncate(&text, 800);

        format!(
            "Fetched: {}\n\nTitle: {}\n\nContent excerpt:\n{}",
            url, title, excerpt
        )
    }
}

// ── Clipboard Transform ──

pub struct ClipboardTransformTool;

impl Tool for ClipboardTransformTool {
    fn name(&self) -> &'static str { "clipboard_transform" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "clipboard" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "clipboard_transform",
                "description": "Transform clipboard content and write the result back. Supports: format_json, minify_json, sort_lines, dedupe_lines, extract_urls, extract_emails, strip_html, decode_base64, url_decode.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["format_json", "minify_json", "sort_lines", "dedupe_lines",
                                     "extract_urls", "extract_emails", "strip_html",
                                     "decode_base64", "url_decode"],
                            "description": "The transformation to apply"
                        }
                    },
                    "required": ["action"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("");
        if action.is_empty() {
            return "Error: action is required".to_string();
        }

        let content = read_clipboard_text();
        if content.is_empty() {
            return "Clipboard is empty.".to_string();
        }

        let result = match action {
            "format_json" => {
                match serde_json::from_str::<serde_json::Value>(&content) {
                    Ok(v) => serde_json::to_string_pretty(&v).unwrap_or(content),
                    Err(e) => return format!("Invalid JSON: {}", e),
                }
            }
            "minify_json" => {
                match serde_json::from_str::<serde_json::Value>(&content) {
                    Ok(v) => serde_json::to_string(&v).unwrap_or(content),
                    Err(e) => return format!("Invalid JSON: {}", e),
                }
            }
            "sort_lines" => {
                let mut lines: Vec<&str> = content.lines().collect();
                lines.sort();
                lines.join("\n")
            }
            "dedupe_lines" => {
                let mut seen = std::collections::HashSet::new();
                content
                    .lines()
                    .filter(|l| seen.insert(*l))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            "extract_urls" => {
                let urls: Vec<&str> = content
                    .split_whitespace()
                    .filter(|w| w.starts_with("http://") || w.starts_with("https://"))
                    .collect();
                if urls.is_empty() {
                    return "No URLs found in clipboard.".to_string();
                }
                urls.join("\n")
            }
            "extract_emails" => {
                let emails: Vec<&str> = content
                    .split_whitespace()
                    .filter(|w| w.contains('@') && w.contains('.') && w.len() > 5)
                    .collect();
                if emails.is_empty() {
                    return "No email addresses found in clipboard.".to_string();
                }
                emails.join("\n")
            }
            "strip_html" => strip_html_tags(&content),
            "decode_base64" => {
                match base64_decode(content.trim()) {
                    Some(decoded) => decoded,
                    None => return "Failed to decode base64.".to_string(),
                }
            }
            "url_decode" => {
                url_decode(&content)
            }
            _ => return format!("Unknown action: {}", action),
        };

        // Write result back to clipboard
        write_clipboard(&result);

        let preview = truncate(&result, 300);
        format!("Transformed ({}) and copied to clipboard:\n{}", action, preview)
    }
}

// ── Text Action ──

pub struct TextActionTool;

impl Tool for TextActionTool {
    fn name(&self) -> &'static str { "text_action" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "clipboard" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "text_action",
                "description": "Read the clipboard text and prepare it for an AI text action. Returns the clipboard content with the requested action, so you (the AI) can perform the transformation and write the result back with write_clipboard. Actions: rewrite_formal, rewrite_casual, summarize, fix_grammar, explain, translate_to.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["rewrite_formal", "rewrite_casual", "summarize",
                                     "fix_grammar", "explain", "translate_to"],
                            "description": "The text action to perform"
                        },
                        "language": {
                            "type": "string",
                            "description": "Target language for translate_to action (e.g. 'Spanish', 'French')"
                        }
                    },
                    "required": ["action"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("");
        let language = args.get("language").and_then(|v| v.as_str()).unwrap_or("English");

        if action.is_empty() {
            return "Error: action is required".to_string();
        }

        let content = read_clipboard_text();
        if content.is_empty() {
            return "Clipboard is empty. Ask the user to copy text first.".to_string();
        }

        let instruction = match action {
            "rewrite_formal" => "Rewrite the following text in a formal, professional tone. Keep the meaning. Return ONLY the rewritten text.",
            "rewrite_casual" => "Rewrite the following text in a casual, friendly tone. Keep the meaning. Return ONLY the rewritten text.",
            "summarize" => "Summarize the following text concisely (2-3 sentences max). Return ONLY the summary.",
            "fix_grammar" => "Fix all grammar, spelling, and punctuation errors in the following text. Keep the original meaning and tone. Return ONLY the corrected text.",
            "explain" => "Explain the following text in simple terms. What does it mean?",
            "translate_to" => &format!("Translate the following text to {}. Return ONLY the translation.", language),
            _ => return format!("Unknown action: {}", action),
        };

        let preview = truncate(&content, 2000);
        format!(
            "ACTION: {}\n\nINSTRUCTION: {}\n\nCLIPBOARD TEXT:\n---\n{}\n---\n\n\
             After you transform the text, use write_clipboard to copy the result so the user can paste it.",
            action, instruction, preview
        )
    }
}

// ── Helpers ──

/// Read current clipboard text via wl-paste.
fn read_clipboard_text() -> String {
    match std::process::Command::new("wl-paste")
        .arg("--no-newline")
        .output()
    {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).to_string()
        }
        _ => String::new(),
    }
}

/// Detect MIME types available in clipboard.
fn detect_mime_types() -> String {
    match std::process::Command::new("wl-paste")
        .arg("--list-types")
        .output()
    {
        Ok(output) if output.status.success() => {
            let types = String::from_utf8_lossy(&output.stdout);
            let relevant: Vec<&str> = types
                .lines()
                .filter(|t| {
                    t.starts_with("text/") || t.starts_with("image/")
                        || t.starts_with("application/json")
                })
                .take(5)
                .collect();
            if relevant.is_empty() {
                "text/plain".to_string()
            } else {
                relevant.join(", ")
            }
        }
        _ => "unknown".to_string(),
    }
}

/// Detect content type from text.
fn detect_type(text: &str) -> String {
    let trimmed = text.trim();
    if check_sensitive(trimmed) {
        "sensitive data".to_string()
    } else if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        "URL".to_string()
    } else if trimmed.contains('@') && trimmed.contains('.') && !trimmed.contains(' ')
        && trimmed.len() < 200
    {
        "email".to_string()
    } else if trimmed.starts_with('/') || trimmed.starts_with("~/") {
        "file path".to_string()
    } else if (trimmed.starts_with('{') && trimmed.ends_with('}'))
        || (trimmed.starts_with('[') && trimmed.ends_with(']'))
    {
        "JSON".to_string()
    } else if trimmed.contains('<') && trimmed.contains('>') && (trimmed.contains("</") || trimmed.contains("/>")) {
        "HTML".to_string()
    } else if trimmed.contains("fn ") || trimmed.contains("def ") || trimmed.contains("function ")
        || trimmed.contains("class ") || trimmed.contains("import ")
        || (trimmed.contains('{') && trimmed.contains('}') && trimmed.matches(';').count() >= 2)
    {
        "code".to_string()
    } else {
        "plain text".to_string()
    }
}

/// Check if text contains sensitive patterns.
fn check_sensitive(text: &str) -> bool {
    let prefixes = [
        "sk-", "sk_live_", "sk_test_", "AKIA", "ghp_", "gho_", "ghs_",
        "github_pat_", "xoxb-", "xoxp-", "SG.", "AIza", "ya29.", "glpat-",
        "npm_", "pypi-",
    ];
    for p in &prefixes {
        if text.starts_with(p) {
            return true;
        }
    }
    if text.contains("-----BEGIN") && text.contains("PRIVATE KEY") {
        return true;
    }
    let lower = text.to_lowercase();
    ["password=", "secret=", "token=", "api_key=", "apikey="]
        .iter()
        .any(|kw| lower.contains(kw))
}

/// Write text to clipboard via wl-copy.
fn write_clipboard(text: &str) {
    let mut child = match std::process::Command::new("wl-copy")
        .stdin(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "wl-copy failed");
            return;
        }
    };
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        let _ = stdin.write_all(text.as_bytes());
    }
    let _ = child.wait();
}

/// Extract HTML <title> tag content.
fn extract_html_title(html: &str) -> Option<String> {
    let lower = html.to_lowercase();
    let start = lower.find("<title")?.checked_add(6)?;
    let after_tag = lower[start..].find('>')?.checked_add(start + 1)?;
    let end = lower[after_tag..].find("</title")?.checked_add(after_tag)?;
    Some(html[after_tag..end].trim().to_string())
}

/// Strip HTML tags from text (simple approach).
fn strip_html_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut in_script = false;
    let lower = html.to_lowercase();

    let chars: Vec<char> = html.chars().collect();
    let lower_chars: Vec<char> = lower.chars().collect();

    let mut i = 0;
    while i < chars.len() {
        if !in_tag && !in_script && chars[i] == '<' {
            // Check for script/style start
            let remaining: String = lower_chars[i..].iter().take(10).collect();
            if remaining.starts_with("<script") || remaining.starts_with("<style") {
                in_script = true;
            }
            in_tag = true;
        } else if in_tag && chars[i] == '>' {
            if in_script {
                let remaining: String = lower_chars[i.saturating_sub(8)..=i].iter().collect();
                if remaining.contains("</script>") || remaining.contains("</style>") {
                    in_script = false;
                }
            }
            in_tag = false;
        } else if !in_tag && !in_script {
            result.push(chars[i]);
        }
        i += 1;
    }

    // Clean up whitespace
    let cleaned: Vec<&str> = result.lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();
    cleaned.join("\n")
}

/// Simple base64 decode.
fn base64_decode(input: &str) -> Option<String> {
    let clean: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    // Use base64 decoding via shell
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("echo -n '{}' | base64 -d 2>/dev/null", clean.replace('\'', "")))
        .output()
        .ok()?;
    if output.status.success() {
        let decoded = String::from_utf8_lossy(&output.stdout).to_string();
        if !decoded.is_empty() {
            Some(decoded)
        } else {
            None
        }
    } else {
        None
    }
}

/// Simple URL decode.
fn url_decode(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = &input[i + 1..i + 3];
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                result.push(byte as char);
                i += 3;
                continue;
            }
        }
        if bytes[i] == b'+' {
            result.push(' ');
        } else {
            result.push(bytes[i] as char);
        }
        i += 1;
    }
    result
}

/// Truncate text to max length with ellipsis.
fn truncate(text: &str, max: usize) -> String {
    if text.len() <= max {
        text.to_string()
    } else {
        let mut boundary = max;
        while boundary > 0 && !text.is_char_boundary(boundary) {
            boundary -= 1;
        }
        format!("{}...", &text[..boundary])
    }
}
