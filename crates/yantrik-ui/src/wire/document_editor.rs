//! Document Editor wiring — load, save, format, find/replace, outline, autosave,
//! comments, track changes, version snapshots, tables, images, undo/redo,
//! templates, text statistics, advanced formatting, HTML export, document
//! properties, auto-versioning, outline navigation, and project-wide search.
//!
//! Documents stored as `.md` files. Supports markdown import/export,
//! heading outline extraction, word/char count, and autosave.
//!
//! ## Slint structs required (add to document_editor.slint):
//!
//! ```slint
//! export struct DocCommentEntry {
//!     id: int,
//!     text: string,
//!     line-hint: int,
//!     resolved: bool,
//!     timestamp: string,
//! }
//!
//! export struct DocChangeEntry {
//!     id: int,
//!     change-type: string,   // "insert", "delete", "modify"
//!     old-text: string,
//!     new-text: string,
//!     line-hint: int,
//!     timestamp: string,
//! }
//!
//! export struct DocVersionEntry {
//!     label: string,
//!     timestamp: string,
//! }
//!
//! export struct DocSearchResult {
//!     file-name: string,
//!     snippet: string,
//!     line-number: int,
//! }
//! ```

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use slint::{ComponentHandle, ModelRc, Timer, TimerMode, VecModel};

use crate::app_context::AppContext;
use crate::bridge::CompanionBridge;
use crate::App;
use crate::DocHeadingEntry;

// ─── Slint struct imports (uncomment once added to .slint) ───
use crate::DocCommentEntry;
use crate::DocChangeEntry;
use crate::DocVersionEntry;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Internal data structures
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Clone, Debug)]
struct DocComment {
    id: i32,
    text: String,
    line_hint: i32,
    resolved: bool,
    timestamp: String,
}

#[derive(Clone, Debug)]
struct TrackedChange {
    id: i32,
    change_type: String, // "insert", "delete", "modify"
    old_text: String,
    new_text: String,
    line_hint: i32,
    timestamp: String,
}

#[derive(Clone, Debug)]
struct DocVersion {
    label: String,
    content: String,
    timestamp: String,
}

#[derive(Clone, Debug)]
struct UndoEntry {
    content: String,
    cursor_hint: i32,
}

/// Centralised mutable state for the document editor.
struct DocState {
    current_file: Option<String>,
    is_modified: bool,

    // Comments
    comments: Vec<DocComment>,
    next_comment_id: i32,

    // Track changes
    track_changes_on: bool,
    changes: Vec<TrackedChange>,
    next_change_id: i32,

    // Versions
    versions: Vec<DocVersion>,

    // Undo / Redo
    undo_stack: Vec<UndoEntry>,
    redo_stack: Vec<UndoEntry>,
    /// Guard to avoid recording undo entries while we are performing undo/redo.
    undo_in_progress: bool,

    // Document properties
    properties: HashMap<String, String>,

    // Auto-version tracking (epoch secs of last auto-version)
    last_auto_version: u64,
    // Auto-save tracking (epoch secs of last save)
    last_save_epoch: u64,
}

impl DocState {
    fn new() -> Self {
        Self {
            current_file: None,
            is_modified: false,
            comments: Vec::new(),
            next_comment_id: 1,
            track_changes_on: false,
            changes: Vec::new(),
            next_change_id: 1,
            versions: Vec::new(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            undo_in_progress: false,
            properties: HashMap::new(),
            last_auto_version: 0,
            last_save_epoch: 0,
        }
    }

    fn reset(&mut self) {
        self.current_file = None;
        self.is_modified = false;
        self.comments.clear();
        self.next_comment_id = 1;
        self.track_changes_on = false;
        self.changes.clear();
        self.next_change_id = 1;
        self.versions.clear();
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.undo_in_progress = false;
        self.properties.clear();
        self.last_auto_version = 0;
        self.last_save_epoch = 0;
    }

    /// Push current content onto the undo stack (max 100 entries).
    fn push_undo(&mut self, content: &str) {
        if self.undo_in_progress {
            return;
        }
        // Avoid duplicate entries for identical content
        if self.undo_stack.last().map_or(false, |e| e.content == content) {
            return;
        }
        self.undo_stack.push(UndoEntry {
            content: content.to_string(),
            cursor_hint: 0,
        });
        if self.undo_stack.len() > 100 {
            self.undo_stack.remove(0);
        }
        // Any new edit clears the redo stack
        self.redo_stack.clear();
    }

    /// Record a tracked change (if tracking is enabled).
    fn record_change(&mut self, change_type: &str, old_text: &str, new_text: &str, line_hint: i32) {
        if !self.track_changes_on {
            return;
        }
        let id = self.next_change_id;
        self.next_change_id += 1;
        self.changes.push(TrackedChange {
            id,
            change_type: change_type.to_string(),
            old_text: old_text.to_string(),
            new_text: new_text.to_string(),
            line_hint,
            timestamp: now_iso(),
        });
    }

    /// Save a named version snapshot.
    fn save_version(&mut self, label: &str, content: &str) {
        self.versions.push(DocVersion {
            label: label.to_string(),
            content: content.to_string(),
            timestamp: now_iso(),
        });
        tracing::info!(label, "Version snapshot saved");
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Utility functions
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Current timestamp as an ISO-like string.
fn now_iso() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Simple human-readable timestamp (no chrono dependency)
    let mins = (secs / 60) % 60;
    let hours = (secs / 3600) % 24;
    let days = secs / 86400;
    format!("d{}:{:02}:{:02}", days, hours, mins)
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Parse headings from markdown content.
fn parse_headings(content: &str) -> Vec<DocHeadingEntry> {
    let mut headings = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        let (level, title) = if let Some(t) = trimmed.strip_prefix("### ") {
            (3, t.trim())
        } else if let Some(t) = trimmed.strip_prefix("## ") {
            (2, t.trim())
        } else if let Some(t) = trimmed.strip_prefix("# ") {
            (1, t.trim())
        } else {
            continue;
        };
        if !title.is_empty() {
            headings.push(DocHeadingEntry {
                title: title.into(),
                level: level as i32,
                block_index: idx as i32,
            });
        }
    }
    headings
}

/// Count words in content.
fn count_words(content: &str) -> i32 {
    content.split_whitespace().count() as i32
}

/// Count characters in content.
fn count_chars(content: &str) -> i32 {
    content.chars().count() as i32
}

/// Count sentences (rough: split on `.` `!` `?` followed by whitespace or end).
fn count_sentences(content: &str) -> i32 {
    if content.trim().is_empty() {
        return 0;
    }
    let mut count = 0i32;
    let chars: Vec<char> = content.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if c == '.' || c == '!' || c == '?' {
            // Count if followed by whitespace, end-of-string, or a quote character
            let next = chars.get(i + 1);
            if next.is_none() || next.map_or(false, |n| n.is_whitespace() || *n == '"' || *n == '\'') {
                count += 1;
            }
        }
    }
    count.max(if content.trim().is_empty() { 0 } else { 1 })
}

/// Count paragraphs (blocks separated by blank lines).
fn count_paragraphs(content: &str) -> i32 {
    if content.trim().is_empty() {
        return 0;
    }
    let mut count = 0i32;
    let mut in_para = false;
    for line in content.lines() {
        if line.trim().is_empty() {
            if in_para {
                in_para = false;
            }
        } else if !in_para {
            in_para = true;
            count += 1;
        }
    }
    count
}

/// Estimate reading time in minutes (200 wpm).
fn reading_time_mins(word_count: i32) -> i32 {
    ((word_count as f64) / 200.0).ceil().max(1.0) as i32
}

/// Estimate pages (~250 words per page).
fn estimate_pages(word_count: i32) -> i32 {
    ((word_count as f64) / 250.0).ceil().max(1.0) as i32
}

/// Update all stats on the UI.
fn update_stats(ui: &App, content: &str) {
    let wc = count_words(content);
    let cc = count_chars(content);
    let pages = estimate_pages(wc);
    let headings = parse_headings(content);

    ui.set_doc_word_count(wc);
    ui.set_doc_char_count(cc);
    ui.set_doc_page_estimate(pages);
    ui.set_doc_headings(ModelRc::new(VecModel::from(headings)));

    // Enhanced stats
    ui.set_doc_reading_time(reading_time_mins(wc));
    ui.set_doc_sentence_count(count_sentences(content));
    ui.set_doc_paragraph_count(count_paragraphs(content));
}

/// Sync comments state to UI (serialised as a string model for now).
fn sync_comments_to_ui(ui: &App, state: &DocState) {
    let entries: Vec<DocCommentEntry> = state
        .comments
        .iter()
        .map(|c| DocCommentEntry {
            id: c.id as i32,
            text: c.text.clone().into(),
            line_hint: c.line_hint as i32,
            resolved: c.resolved,
            timestamp: c.timestamp.clone().into(),
        })
        .collect();
    ui.set_doc_comments(ModelRc::new(VecModel::from(entries)));
}

/// Sync tracked changes state to UI.
fn sync_changes_to_ui(ui: &App, state: &DocState) {
    let entries: Vec<DocChangeEntry> = state
        .changes
        .iter()
        .map(|c| DocChangeEntry {
            id: c.id as i32,
            change_type: c.change_type.clone().into(),
            old_text: c.old_text.clone().into(),
            new_text: c.new_text.clone().into(),
            line_hint: c.line_hint as i32,
            timestamp: c.timestamp.clone().into(),
        })
        .collect();
    ui.set_doc_changes(ModelRc::new(VecModel::from(entries)));
}

/// Sync versions state to UI.
fn sync_versions_to_ui(ui: &App, state: &DocState) {
    let entries: Vec<DocVersionEntry> = state
        .versions
        .iter()
        .enumerate()
        .map(|(i, v)| DocVersionEntry {
            label: v.label.clone().into(),
            timestamp: v.timestamp.clone().into(),
            index: i as i32,
        })
        .collect();
    ui.set_doc_versions(ModelRc::new(VecModel::from(entries)));
}

/// Sync document properties to UI.
fn sync_properties_to_ui(_ui: &App, _state: &DocState) {
    // doc-tags, doc-category, doc-summary properties not yet wired in Slint UI.
    // Will be added when document metadata panel is implemented.
}

/// Count occurrences of a query in the content (case-insensitive).
fn count_matches(content: &str, query: &str) -> i32 {
    if query.is_empty() {
        return 0;
    }
    let lower_content = content.to_lowercase();
    let lower_query = query.to_lowercase();
    lower_content.matches(&lower_query).count() as i32
}

/// Get the documents directory.
fn documents_directory() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    PathBuf::from(home).join(".local/share/yantrik/documents")
}

/// Get the images directory.
fn images_directory() -> PathBuf {
    documents_directory().join("images")
}

/// Append text to the current document content.
fn append_to_content(ui: &App, insertion: &str) {
    let content = ui.get_doc_content().to_string();
    let new_content = format!("{}{}", content, insertion);
    ui.set_doc_content(new_content.clone().into());
    ui.set_doc_is_modified(true);
    ui.set_doc_save_status("Modified".into());
    update_stats(ui, &new_content);
}

/// Extract title from markdown content (first # heading or first non-empty line).
fn extract_title(content: &str) -> String {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(heading) = trimmed.strip_prefix("# ") {
            return heading.trim().to_string();
        }
        return trimmed.chars().take(50).collect();
    }
    "Untitled".to_string()
}

/// Convert a title to a filesystem-safe slug.
fn slugify(title: &str) -> String {
    let slug: String = title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    let mut result = String::new();
    let mut last_dash = false;
    for c in slug.chars() {
        if c == '-' {
            if !last_dash && !result.is_empty() {
                result.push('-');
            }
            last_dash = true;
        } else {
            result.push(c);
            last_dash = false;
        }
    }
    result.trim_end_matches('-').to_string()
}

/// Generate a markdown table with the given dimensions.
fn generate_markdown_table(rows: i32, cols: i32) -> String {
    let cols = cols.max(1) as usize;
    let rows = rows.max(1) as usize;

    let mut table = String::new();
    // Header row
    table.push('|');
    for c in 0..cols {
        table.push_str(&format!(" H{} |", c + 1));
    }
    table.push('\n');
    // Separator row
    table.push('|');
    for _ in 0..cols {
        table.push_str("---|");
    }
    table.push('\n');
    // Data rows
    for _ in 0..rows {
        table.push('|');
        for _ in 0..cols {
            table.push_str("  |");
        }
        table.push('\n');
    }
    table
}

/// Detect if the cursor is inside a markdown table and add a row.
fn add_table_row(content: &str) -> String {
    // Find the last table in the content and append a row
    let lines: Vec<&str> = content.lines().collect();
    let mut last_table_end = None;
    let mut cols = 0usize;

    for (i, line) in lines.iter().enumerate().rev() {
        let trimmed = line.trim();
        if trimmed.starts_with('|') && trimmed.ends_with('|') {
            if last_table_end.is_none() {
                last_table_end = Some(i);
            }
            // Count columns
            let c = trimmed.matches('|').count().saturating_sub(1);
            if c > cols {
                cols = c;
            }
        } else if last_table_end.is_some() {
            break;
        }
    }

    if let Some(end) = last_table_end {
        if cols == 0 {
            cols = 3;
        }
        let mut new_row = String::from("|");
        for _ in 0..cols {
            new_row.push_str("  |");
        }
        let mut result_lines: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
        result_lines.insert(end + 1, new_row);
        result_lines.join("\n")
    } else {
        content.to_string()
    }
}

/// Detect the last table and add a column.
fn add_table_col(content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let mut in_table = false;
    let mut table_ranges: Vec<(usize, usize)> = Vec::new();
    let mut start = 0;

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let is_table_line = trimmed.starts_with('|') && trimmed.ends_with('|');
        if is_table_line && !in_table {
            in_table = true;
            start = i;
        } else if !is_table_line && in_table {
            in_table = false;
            table_ranges.push((start, i - 1));
        }
    }
    if in_table {
        table_ranges.push((start, lines.len() - 1));
    }

    if let Some(&(tstart, tend)) = table_ranges.last() {
        let mut result_lines: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
        for i in tstart..=tend {
            let line = &result_lines[i];
            let trimmed = line.trim();
            if trimmed.contains("---") {
                // Separator row
                result_lines[i] = format!("{}---|", trimmed);
            } else {
                // Header or data row — add empty cell
                result_lines[i] = format!("{}  |", trimmed);
            }
        }
        result_lines.join("\n")
    } else {
        content.to_string()
    }
}

/// Basic markdown to HTML conversion.
fn markdown_to_html(md: &str) -> String {
    let mut html = String::from(
        "<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n\
         <style>\n  body { font-family: sans-serif; max-width: 800px; margin: 40px auto; padding: 0 20px; }\n\
         pre { background: #f4f4f4; padding: 12px; border-radius: 4px; overflow-x: auto; }\n\
         code { background: #f4f4f4; padding: 2px 4px; border-radius: 3px; }\n\
         blockquote { border-left: 4px solid #ddd; margin: 0; padding: 0 16px; color: #666; }\n\
         table { border-collapse: collapse; }\n\
         th, td { border: 1px solid #ddd; padding: 8px 12px; text-align: left; }\n\
         th { background: #f4f4f4; }\n\
         hr { border: none; border-top: 1px solid #ddd; margin: 24px 0; }\n\
         img { max-width: 100%; }\n\
         </style>\n</head>\n<body>\n",
    );

    let mut in_code_block = false;
    let mut in_list = false;
    let mut in_table = false;
    let mut in_blockquote = false;

    for line in md.lines() {
        let trimmed = line.trim();

        // Code blocks
        if trimmed.starts_with("```") {
            if in_code_block {
                html.push_str("</code></pre>\n");
                in_code_block = false;
            } else {
                in_code_block = true;
                html.push_str("<pre><code>");
            }
            continue;
        }
        if in_code_block {
            html.push_str(&html_escape(trimmed));
            html.push('\n');
            continue;
        }

        // Blank line
        if trimmed.is_empty() {
            if in_list {
                html.push_str("</ul>\n");
                in_list = false;
            }
            if in_blockquote {
                html.push_str("</blockquote>\n");
                in_blockquote = false;
            }
            if in_table {
                html.push_str("</tbody></table>\n");
                in_table = false;
            }
            continue;
        }

        // Horizontal rule
        if trimmed == "---" || trimmed == "***" || trimmed == "___" {
            html.push_str("<hr>\n");
            continue;
        }

        // Headings
        if let Some(t) = trimmed.strip_prefix("### ") {
            html.push_str(&format!("<h3>{}</h3>\n", inline_format(t.trim())));
            continue;
        }
        if let Some(t) = trimmed.strip_prefix("## ") {
            html.push_str(&format!("<h2>{}</h2>\n", inline_format(t.trim())));
            continue;
        }
        if let Some(t) = trimmed.strip_prefix("# ") {
            html.push_str(&format!("<h1>{}</h1>\n", inline_format(t.trim())));
            continue;
        }

        // Blockquote
        if let Some(t) = trimmed.strip_prefix("> ") {
            if !in_blockquote {
                html.push_str("<blockquote>\n");
                in_blockquote = true;
            }
            html.push_str(&format!("<p>{}</p>\n", inline_format(t)));
            continue;
        }

        // Unordered list
        if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
            if !in_list {
                html.push_str("<ul>\n");
                in_list = true;
            }
            let item = &trimmed[2..];
            // Checkbox
            if let Some(rest) = item.strip_prefix("[ ] ") {
                html.push_str(&format!(
                    "<li><input type=\"checkbox\" disabled> {}</li>\n",
                    inline_format(rest)
                ));
            } else if let Some(rest) = item.strip_prefix("[x] ") {
                html.push_str(&format!(
                    "<li><input type=\"checkbox\" checked disabled> {}</li>\n",
                    inline_format(rest)
                ));
            } else {
                html.push_str(&format!("<li>{}</li>\n", inline_format(item)));
            }
            continue;
        }

        // Table
        if trimmed.starts_with('|') && trimmed.ends_with('|') {
            // Check if separator row
            let inner = &trimmed[1..trimmed.len() - 1];
            let is_sep = inner
                .split('|')
                .all(|cell| cell.trim().chars().all(|c| c == '-' || c == ':'));

            if is_sep {
                // Already handled header above, skip separator
                continue;
            }

            if !in_table {
                html.push_str("<table><thead><tr>\n");
                let cells: Vec<&str> = inner.split('|').collect();
                for cell in &cells {
                    html.push_str(&format!("<th>{}</th>", inline_format(cell.trim())));
                }
                html.push_str("</tr></thead><tbody>\n");
                in_table = true;
            } else {
                html.push_str("<tr>");
                let cells: Vec<&str> = inner.split('|').collect();
                for cell in &cells {
                    html.push_str(&format!("<td>{}</td>", inline_format(cell.trim())));
                }
                html.push_str("</tr>\n");
            }
            continue;
        }

        // Close any open list/blockquote before paragraph
        if in_list {
            html.push_str("</ul>\n");
            in_list = false;
        }
        if in_blockquote {
            html.push_str("</blockquote>\n");
            in_blockquote = false;
        }

        // Paragraph
        html.push_str(&format!("<p>{}</p>\n", inline_format(trimmed)));
    }

    // Close any open blocks
    if in_code_block {
        html.push_str("</code></pre>\n");
    }
    if in_list {
        html.push_str("</ul>\n");
    }
    if in_blockquote {
        html.push_str("</blockquote>\n");
    }
    if in_table {
        html.push_str("</tbody></table>\n");
    }

    html.push_str("</body>\n</html>\n");
    html
}

/// Inline markdown formatting (bold, italic, code, links, images, strikethrough, highlight).
fn inline_format(text: &str) -> String {
    let mut s = html_escape(text);

    // Images: ![alt](src)
    while let Some(start) = s.find("![") {
        if let Some(mid) = s[start..].find("](") {
            if let Some(end) = s[start + mid..].find(')') {
                let alt = &s[start + 2..start + mid].to_string();
                let src = &s[start + mid + 2..start + mid + end].to_string();
                let img = format!("<img src=\"{}\" alt=\"{}\">", src, alt);
                s = format!("{}{}{}", &s[..start], img, &s[start + mid + end + 1..]);
                continue;
            }
        }
        break;
    }

    // Links: [text](url)
    while let Some(start) = s.find('[') {
        // Make sure it's not an image
        if start > 0 && s.as_bytes()[start - 1] == b'!' {
            break;
        }
        if let Some(mid) = s[start..].find("](") {
            if let Some(end) = s[start + mid..].find(')') {
                let text_part = &s[start + 1..start + mid].to_string();
                let url = &s[start + mid + 2..start + mid + end].to_string();
                let link = format!("<a href=\"{}\">{}</a>", url, text_part);
                s = format!("{}{}{}", &s[..start], link, &s[start + mid + end + 1..]);
                continue;
            }
        }
        break;
    }

    // Bold: **text**
    while let Some(start) = s.find("**") {
        if let Some(end) = s[start + 2..].find("**") {
            let inner = &s[start + 2..start + 2 + end].to_string();
            s = format!(
                "{}<strong>{}</strong>{}",
                &s[..start],
                inner,
                &s[start + 2 + end + 2..]
            );
        } else {
            break;
        }
    }

    // Strikethrough: ~~text~~
    while let Some(start) = s.find("~~") {
        if let Some(end) = s[start + 2..].find("~~") {
            let inner = &s[start + 2..start + 2 + end].to_string();
            s = format!(
                "{}<del>{}</del>{}",
                &s[..start],
                inner,
                &s[start + 2 + end + 2..]
            );
        } else {
            break;
        }
    }

    // Highlight: ==text==
    while let Some(start) = s.find("==") {
        if let Some(end) = s[start + 2..].find("==") {
            let inner = &s[start + 2..start + 2 + end].to_string();
            s = format!(
                "{}<mark>{}</mark>{}",
                &s[..start],
                inner,
                &s[start + 2 + end + 2..]
            );
        } else {
            break;
        }
    }

    // Italic: *text* (single asterisk, after bold is processed)
    while let Some(start) = s.find('*') {
        if let Some(end) = s[start + 1..].find('*') {
            let inner = &s[start + 1..start + 1 + end].to_string();
            s = format!(
                "{}<em>{}</em>{}",
                &s[..start],
                inner,
                &s[start + 1 + end + 1..]
            );
        } else {
            break;
        }
    }

    // Inline code: `code`
    while let Some(start) = s.find('`') {
        if let Some(end) = s[start + 1..].find('`') {
            let inner = &s[start + 1..start + 1 + end].to_string();
            s = format!(
                "{}<code>{}</code>{}",
                &s[..start],
                inner,
                &s[start + 1 + end + 1..]
            );
        } else {
            break;
        }
    }

    // Superscript: ^text^
    while let Some(start) = s.find('^') {
        if let Some(end) = s[start + 1..].find('^') {
            let inner = &s[start + 1..start + 1 + end].to_string();
            s = format!(
                "{}<sup>{}</sup>{}",
                &s[..start],
                inner,
                &s[start + 1 + end + 1..]
            );
        } else {
            break;
        }
    }

    s
}

fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Generate a table of contents from headings in the content.
fn generate_toc(content: &str) -> String {
    let headings = parse_headings(content);
    if headings.is_empty() {
        return String::from("*No headings found.*\n");
    }
    let mut toc = String::from("## Table of Contents\n\n");
    for h in &headings {
        let indent = "  ".repeat((h.level - 1) as usize);
        let title = h.title.to_string();
        // Create anchor-like link
        let anchor = title
            .to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() || c == ' ' { c } else { '-' })
            .collect::<String>()
            .replace(' ', "-");
        toc.push_str(&format!("{}- [{}](#{})\n", indent, title, anchor));
    }
    toc.push('\n');
    toc
}

/// Get template content by index.
fn get_template(idx: i32) -> (String, String) {
    match idx {
        0 => ("Untitled".to_string(), String::new()),
        1 => (
            "Meeting Notes".to_string(),
            "## Date\n\n\n\n## Attendees\n\n- \n\n## Agenda\n\n1. \n\n## Discussion\n\n\n\n\
             ## Action Items\n\n- [ ] \n\n## Next Meeting\n\n"
                .to_string(),
        ),
        2 => (
            "Project Status".to_string(),
            "## Overview\n\nBrief description of the project and its current state.\n\n\
             ## Completed This Week\n\n- \n\n## In Progress\n\n- \n\n\
             ## Blocked / Risks\n\n- \n\n## Upcoming\n\n- \n\n\
             ## Metrics\n\n| Metric | Value | Trend |\n|---|---|---|\n|  |  |  |\n"
                .to_string(),
        ),
        3 => (
            "Weekly Update".to_string(),
            "## Week of \n\n### Highlights\n\n- \n\n\
             ### Accomplishments\n\n- \n\n### Challenges\n\n- \n\n\
             ### Goals for Next Week\n\n- [ ] \n- [ ] \n- [ ] \n\n### Notes\n\n"
                .to_string(),
        ),
        4 => (
            "Technical Spec".to_string(),
            "## Summary\n\nOne-paragraph summary of the proposed change.\n\n\
             ## Motivation\n\nWhy is this change needed? What problem does it solve?\n\n\
             ## Design\n\n### Architecture\n\n\n\n### API Changes\n\n```\n\n```\n\n\
             ### Data Model\n\n\n\n## Alternatives Considered\n\n\n\n\
             ## Testing Strategy\n\n- [ ] Unit tests\n- [ ] Integration tests\n- [ ] Manual testing\n\n\
             ## Rollout Plan\n\n1. \n\n## Open Questions\n\n- \n"
                .to_string(),
        ),
        5 => (
            "Proposal".to_string(),
            "## Executive Summary\n\n\n\n## Problem Statement\n\n\n\n\
             ## Proposed Solution\n\n\n\n## Benefits\n\n- \n\n\
             ## Cost / Effort Estimate\n\n| Item | Estimate |\n|---|---|\n|  |  |\n\n\
             ## Timeline\n\n| Phase | Duration | Deliverable |\n|---|---|---|\n|  |  |  |\n\n\
             ## Risks & Mitigations\n\n| Risk | Mitigation |\n|---|---|\n|  |  |\n\n\
             ## Success Criteria\n\n- [ ] \n\n## Approval\n\n- [ ] Reviewed by: \n- [ ] Approved by: \n"
                .to_string(),
        ),
        _ => ("Untitled".to_string(), String::new()),
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Wire function
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Wire document editor callbacks.
pub fn wire(ui: &App, ctx: &AppContext) {
    let docs_dir = documents_directory();
    let _ = std::fs::create_dir_all(&docs_dir);
    let _ = std::fs::create_dir_all(images_directory());

    let state = Rc::new(RefCell::new(DocState::new()));

    // ── Content changed ──────────────────────────────────────
    {
        let ui_weak = ui.as_weak();
        let st = Rc::clone(&state);
        ui.on_doc_content_changed(move |text| {
            let content = text.to_string();

            // Push undo before mutation
            {
                let mut s = st.borrow_mut();
                s.push_undo(&content);
                s.is_modified = true;
            }

            if let Some(ui) = ui_weak.upgrade() {
                update_stats(&ui, &content);
                ui.set_doc_is_modified(true);
                ui.set_doc_save_status("Modified".into());

                // Update find match count if find is active
                let query = ui.get_doc_find_query().to_string();
                if !query.is_empty() {
                    let matches = count_matches(&content, &query);
                    ui.set_doc_find_count(matches);
                }
            }
        });
    }

    // ── New document ─────────────────────────────────────────
    {
        let ui_weak = ui.as_weak();
        let st = Rc::clone(&state);
        ui.on_doc_new(move || {
            st.borrow_mut().reset();
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_doc_title("Untitled".into());
                ui.set_doc_content("".into());
                ui.set_doc_file_path("".into());
                ui.set_doc_is_modified(false);
                ui.set_doc_save_status("New document".into());
                ui.set_doc_track_changes_on(false);
                update_stats(&ui, "");
                sync_comments_to_ui(&ui, &st.borrow());
                sync_changes_to_ui(&ui, &st.borrow());
                sync_versions_to_ui(&ui, &st.borrow());
                sync_properties_to_ui(&ui, &st.borrow());
            }
        });
    }

    // ── Save document ────────────────────────────────────────
    {
        let ui_weak = ui.as_weak();
        let st = Rc::clone(&state);
        let dd = docs_dir.clone();
        ui.on_doc_save(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let title = ui.get_doc_title().to_string();
                let content = ui.get_doc_content().to_string();

                let full_content = if content.starts_with("# ") {
                    content.clone()
                } else {
                    format!("# {}\n\n{}", title, content)
                };

                let current_file = st.borrow().current_file.clone();
                let path = if let Some(existing) = current_file {
                    PathBuf::from(existing)
                } else {
                    let _ = std::fs::create_dir_all(&dd);
                    let slug = slugify(&title);
                    let ts = now_epoch();
                    dd.join(format!("{}-{}.md", ts, slug))
                };

                match std::fs::write(&path, &full_content) {
                    Ok(()) => {
                        let mut s = st.borrow_mut();
                        s.current_file = Some(path.display().to_string());
                        s.is_modified = false;
                        s.last_save_epoch = now_epoch();

                        ui.set_doc_file_path(path.display().to_string().into());
                        ui.set_doc_is_modified(false);
                        ui.set_doc_save_status("Saved".into());
                        tracing::info!(path = %path.display(), "Document saved");
                    }
                    Err(e) => {
                        ui.set_doc_save_status(format!("Error: {}", e).into());
                        tracing::error!(error = %e, "Failed to save document");
                    }
                }
            }
        });
    }

    // ── Open document ────────────────────────────────────────
    {
        let ui_weak = ui.as_weak();
        let st = Rc::clone(&state);
        let dd = docs_dir.clone();
        ui.on_doc_open(move || {
            let mut files: Vec<_> = std::fs::read_dir(&dd)
                .into_iter()
                .flatten()
                .flatten()
                .filter(|e| e.path().extension().map_or(false, |ext| ext == "md"))
                .collect();

            files.sort_by(|a, b| {
                let ma = a.metadata().ok().and_then(|m| m.modified().ok());
                let mb = b.metadata().ok().and_then(|m| m.modified().ok());
                mb.cmp(&ma)
            });

            if let Some(entry) = files.first() {
                let path = entry.path();
                match std::fs::read_to_string(&path) {
                    Ok(content) => {
                        {
                            let mut s = st.borrow_mut();
                            s.reset();
                            s.current_file = Some(path.display().to_string());
                            s.last_save_epoch = now_epoch();
                        }

                        if let Some(ui) = ui_weak.upgrade() {
                            let title = extract_title(&content);
                            let body = content
                                .lines()
                                .skip_while(|l| l.trim().is_empty())
                                .skip(1)
                                .collect::<Vec<_>>()
                                .join("\n")
                                .trim_start_matches('\n')
                                .to_string();

                            ui.set_doc_title(title.into());
                            ui.set_doc_content(body.clone().into());
                            ui.set_doc_file_path(path.display().to_string().into());
                            ui.set_doc_is_modified(false);
                            ui.set_doc_save_status("Loaded".into());
                            ui.set_doc_track_changes_on(false);
                            update_stats(&ui, &body);
                            sync_comments_to_ui(&ui, &st.borrow());
                            sync_changes_to_ui(&ui, &st.borrow());
                            sync_versions_to_ui(&ui, &st.borrow());
                            sync_properties_to_ui(&ui, &st.borrow());
                        }
                        tracing::info!(path = %path.display(), "Document opened");
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "Failed to open document");
                    }
                }
            } else {
                tracing::info!("No documents found in {}", dd.display());
            }
        });
    }

    // ── Export PDF (markdown fallback) ──────────────────────
    {
        let ui_weak = ui.as_weak();
        let dd = docs_dir.clone();
        ui.on_doc_export_pdf(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let title = ui.get_doc_title().to_string();
                let content = ui.get_doc_content().to_string();
                let safe_title = title
                    .chars()
                    .map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' { c } else { '_' })
                    .collect::<String>();
                let filename = if safe_title.is_empty() { "Untitled".to_string() } else { safe_title };
                let path = dd.join(format!("{}.md", filename));
                match std::fs::write(&path, &content) {
                    Ok(()) => {
                        let msg = format!("Exported as Markdown: {}", path.display());
                        tracing::info!("{}", msg);
                        ui.set_doc_export_status(msg.into());
                    }
                    Err(e) => {
                        let msg = format!("Export failed: {}", e);
                        tracing::error!("{}", msg);
                        ui.set_doc_export_status(msg.into());
                    }
                }
            }
        });
    }

    // ── Page layout ──────────────────────────────────────────
    {
        let ui_weak = ui.as_weak();
        ui.on_doc_set_page_layout(move |size| {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_doc_page_size(size);
                tracing::debug!(page_size = %ui.get_doc_page_size(), "Page layout changed");
            }
        });
    }

    // ── Print preview (stub) ─────────────────────────────────
    {
        let ui_weak = ui.as_weak();
        ui.on_doc_print_preview(move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_doc_export_status("Print preview not yet available".into());
            }
        });
    }

    // ── Import markdown ──────────────────────────────────────
    {
        let ui_weak = ui.as_weak();
        let st = Rc::clone(&state);
        let dd = docs_dir.clone();
        ui.on_doc_import_md(move || {
            let mut files: Vec<_> = std::fs::read_dir(&dd)
                .into_iter()
                .flatten()
                .flatten()
                .filter(|e| e.path().extension().map_or(false, |ext| ext == "md"))
                .collect();

            files.sort_by(|a, b| {
                let ma = a.metadata().ok().and_then(|m| m.modified().ok());
                let mb = b.metadata().ok().and_then(|m| m.modified().ok());
                mb.cmp(&ma)
            });

            if let Some(entry) = files.first() {
                let path = entry.path();
                match std::fs::read_to_string(&path) {
                    Ok(content) => {
                        // Auto-version before import
                        if let Some(ui) = ui_weak.upgrade() {
                            let old_content = ui.get_doc_content().to_string();
                            if !old_content.is_empty() {
                                st.borrow_mut()
                                    .save_version("Before import", &old_content);
                                sync_versions_to_ui(&ui, &st.borrow());
                            }
                        }

                        {
                            let mut s = st.borrow_mut();
                            s.current_file = Some(path.display().to_string());
                            s.is_modified = false;
                        }

                        if let Some(ui) = ui_weak.upgrade() {
                            let title = extract_title(&content);
                            ui.set_doc_title(title.into());
                            ui.set_doc_content(content.clone().into());
                            ui.set_doc_file_path(path.display().to_string().into());
                            ui.set_doc_is_modified(false);
                            ui.set_doc_save_status("Imported".into());
                            update_stats(&ui, &content);
                        }
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "Failed to import markdown");
                    }
                }
            }
        });
    }

    // ── Export markdown ──────────────────────────────────────
    {
        let ui_weak = ui.as_weak();
        let dd2 = docs_dir.clone();
        ui.on_doc_export_md(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let title = ui.get_doc_title().to_string();
                let content = ui.get_doc_content().to_string();
                let full = format!("# {}\n\n{}", title, content);

                let slug = slugify(&title);
                let path = dd2.join(format!("{}-export.md", slug));

                match std::fs::write(&path, &full) {
                    Ok(()) => {
                        ui.set_doc_save_status(
                            format!("Exported to {}", path.display()).into(),
                        );
                        tracing::info!(path = %path.display(), "Markdown exported");
                    }
                    Err(e) => {
                        ui.set_doc_save_status(format!("Export error: {}", e).into());
                        tracing::error!(error = %e, "Failed to export markdown");
                    }
                }
            }
        });
    }

    // ── Export HTML ───────────────────────────────────────────
    {
        let ui_weak = ui.as_weak();
        let dd = docs_dir.clone();
        ui.on_doc_export_html(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let title = ui.get_doc_title().to_string();
                let content = ui.get_doc_content().to_string();
                let full_md = format!("# {}\n\n{}", title, content);
                let html = markdown_to_html(&full_md);

                let slug = slugify(&title);
                let path = dd.join(format!("{}-export.html", slug));

                match std::fs::write(&path, &html) {
                    Ok(()) => {
                        ui.set_doc_save_status(
                            format!("HTML exported to {}", path.display()).into(),
                        );
                        tracing::info!(path = %path.display(), "HTML exported");
                    }
                    Err(e) => {
                        ui.set_doc_save_status(format!("HTML export error: {}", e).into());
                        tracing::error!(error = %e, "Failed to export HTML");
                    }
                }
            }
        });
    }

    // ━━ Formatting callbacks ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    {
        let ui_weak = ui.as_weak();
        let st = Rc::clone(&state);
        ui.on_doc_format_bold(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let content = ui.get_doc_content().to_string();
                st.borrow_mut().record_change("insert", "", "**bold text**", count_lines(&content));
                append_to_content(&ui, "**bold text**");
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        let st = Rc::clone(&state);
        ui.on_doc_format_italic(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let content = ui.get_doc_content().to_string();
                st.borrow_mut().record_change("insert", "", "*italic text*", count_lines(&content));
                append_to_content(&ui, "*italic text*");
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        let st = Rc::clone(&state);
        ui.on_doc_format_underline(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let content = ui.get_doc_content().to_string();
                st.borrow_mut().record_change("insert", "", "<u>underlined text</u>", count_lines(&content));
                append_to_content(&ui, "<u>underlined text</u>");
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        let st = Rc::clone(&state);
        ui.on_doc_format_heading(move |level| {
            if let Some(ui) = ui_weak.upgrade() {
                let prefix = "#".repeat(level as usize);
                let insertion = format!("\n{} Heading\n", prefix);
                let content = ui.get_doc_content().to_string();
                st.borrow_mut().record_change("insert", "", &insertion, count_lines(&content));
                append_to_content(&ui, &insertion);
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        ui.on_doc_format_bullet(move || {
            if let Some(ui) = ui_weak.upgrade() {
                append_to_content(&ui, "\n- ");
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        ui.on_doc_format_checklist(move || {
            if let Some(ui) = ui_weak.upgrade() {
                append_to_content(&ui, "\n- [ ] ");
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        ui.on_doc_format_quote(move || {
            if let Some(ui) = ui_weak.upgrade() {
                append_to_content(&ui, "\n> ");
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        ui.on_doc_format_code(move || {
            if let Some(ui) = ui_weak.upgrade() {
                append_to_content(&ui, "\n```\ncode\n```\n");
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        ui.on_doc_format_divider(move || {
            if let Some(ui) = ui_weak.upgrade() {
                append_to_content(&ui, "\n---\n");
            }
        });
    }

    // ── Advanced formatting ──────────────────────────────────

    {
        let ui_weak = ui.as_weak();
        ui.on_doc_format_strikethrough(move || {
            if let Some(ui) = ui_weak.upgrade() {
                append_to_content(&ui, "~~strikethrough~~");
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        ui.on_doc_format_highlight(move || {
            if let Some(ui) = ui_weak.upgrade() {
                append_to_content(&ui, "==highlighted==");
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        ui.on_doc_format_link(move |url| {
            if let Some(ui) = ui_weak.upgrade() {
                let link = format!("[link]({})", url);
                append_to_content(&ui, &link);
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        ui.on_doc_format_inline_code(move || {
            if let Some(ui) = ui_weak.upgrade() {
                append_to_content(&ui, "`code`");
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        ui.on_doc_insert_footnote(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let content = ui.get_doc_content().to_string();
                // Count existing footnotes to determine the next number
                let footnote_count = content.matches("[^").count() / 2 + 1;
                let ref_text = format!("[^{}]", footnote_count);
                let def_text = format!("\n\n[^{}]: ", footnote_count);
                let insertion = format!("{}{}", ref_text, def_text);
                append_to_content(&ui, &insertion);
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        ui.on_doc_insert_toc(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let content = ui.get_doc_content().to_string();
                let toc = generate_toc(&content);
                // Prepend TOC to content
                let new_content = format!("{}\n{}", toc, content);
                ui.set_doc_content(new_content.clone().into());
                ui.set_doc_is_modified(true);
                ui.set_doc_save_status("TOC inserted".into());
                update_stats(&ui, &new_content);
            }
        });
    }

    // ━━ Undo / Redo ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    {
        let ui_weak = ui.as_weak();
        let st = Rc::clone(&state);
        ui.on_doc_undo(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let current_content = ui.get_doc_content().to_string();

                let prev = {
                    let mut s = st.borrow_mut();
                    s.undo_in_progress = true;

                    // Push current state to redo stack
                    s.redo_stack.push(UndoEntry {
                        content: current_content,
                        cursor_hint: 0,
                    });

                    s.undo_stack.pop()
                };

                if let Some(entry) = prev {
                    ui.set_doc_content(entry.content.clone().into());
                    ui.set_doc_save_status("Undo".into());
                    update_stats(&ui, &entry.content);
                } else {
                    ui.set_doc_save_status("Nothing to undo".into());
                    // Remove the redo entry we just pushed since there was nothing to undo
                    st.borrow_mut().redo_stack.pop();
                }

                st.borrow_mut().undo_in_progress = false;
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        let st = Rc::clone(&state);
        ui.on_doc_redo(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let current_content = ui.get_doc_content().to_string();

                let next = {
                    let mut s = st.borrow_mut();
                    s.undo_in_progress = true;

                    // Push current state to undo stack
                    s.undo_stack.push(UndoEntry {
                        content: current_content,
                        cursor_hint: 0,
                    });

                    s.redo_stack.pop()
                };

                if let Some(entry) = next {
                    ui.set_doc_content(entry.content.clone().into());
                    ui.set_doc_save_status("Redo".into());
                    update_stats(&ui, &entry.content);
                } else {
                    ui.set_doc_save_status("Nothing to redo".into());
                    // Remove the undo entry we just pushed
                    st.borrow_mut().undo_stack.pop();
                }

                st.borrow_mut().undo_in_progress = false;
            }
        });
    }

    // ━━ Find / Replace ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    {
        let ui_weak = ui.as_weak();
        ui.on_doc_find_next(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let content = ui.get_doc_content().to_string();
                let query = ui.get_doc_find_query().to_string();
                let matches = count_matches(&content, &query);
                ui.set_doc_find_count(matches);
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        ui.on_doc_find_prev(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let content = ui.get_doc_content().to_string();
                let query = ui.get_doc_find_query().to_string();
                let matches = count_matches(&content, &query);
                ui.set_doc_find_count(matches);
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        let st = Rc::clone(&state);
        ui.on_doc_replace_one(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let content = ui.get_doc_content().to_string();
                let query = ui.get_doc_find_query().to_string();
                let replacement = ui.get_doc_replace_text().to_string();
                if !query.is_empty() {
                    if let Some(pos) = content.to_lowercase().find(&query.to_lowercase()) {
                        let old_fragment = &content[pos..pos + query.len()];
                        st.borrow_mut().record_change(
                            "modify",
                            old_fragment,
                            &replacement,
                            content[..pos].lines().count() as i32,
                        );

                        let new_content = format!(
                            "{}{}{}",
                            &content[..pos],
                            replacement,
                            &content[pos + query.len()..]
                        );
                        ui.set_doc_content(new_content.clone().into());
                        ui.set_doc_is_modified(true);
                        let matches = count_matches(&new_content, &query);
                        ui.set_doc_find_count(matches);
                        update_stats(&ui, &new_content);
                    }
                }
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        let st = Rc::clone(&state);
        ui.on_doc_replace_all(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let content = ui.get_doc_content().to_string();
                let query = ui.get_doc_find_query().to_string();
                let replacement = ui.get_doc_replace_text().to_string();
                if !query.is_empty() {
                    // Auto-version before replace-all
                    st.borrow_mut()
                        .save_version("Before replace-all", &content);

                    st.borrow_mut().record_change("modify", &query, &replacement, 0);

                    let lower_content = content.to_lowercase();
                    let lower_query = query.to_lowercase();
                    let mut result = String::new();
                    let mut last = 0;
                    for (start, _) in lower_content.match_indices(&lower_query) {
                        result.push_str(&content[last..start]);
                        result.push_str(&replacement);
                        last = start + query.len();
                    }
                    result.push_str(&content[last..]);
                    ui.set_doc_content(result.clone().into());
                    ui.set_doc_is_modified(true);
                    ui.set_doc_find_count(0);
                    update_stats(&ui, &result);

                    if let Some(ui2) = ui_weak.upgrade() {
                        sync_versions_to_ui(&ui2, &st.borrow());
                        sync_changes_to_ui(&ui2, &st.borrow());
                    }
                }
            }
        });
    }

    // ── Heading clicked (outline navigation) ─────────────────
    {
        let ui_weak = ui.as_weak();
        ui.on_doc_heading_clicked(move |block_index| {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_doc_save_status(format!("Line {}", block_index + 1).into());
                tracing::info!(line = block_index, "Outline navigation");
            }
        });
    }

    // ━━ Comments ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    {
        let ui_weak = ui.as_weak();
        let st = Rc::clone(&state);
        ui.on_doc_add_comment(move |text| {
            if let Some(ui) = ui_weak.upgrade() {
                let content = ui.get_doc_content().to_string();
                let line_hint = content.lines().count() as i32;

                let mut s = st.borrow_mut();
                let id = s.next_comment_id;
                s.next_comment_id += 1;
                s.comments.push(DocComment {
                    id,
                    text: text.to_string(),
                    line_hint,
                    resolved: false,
                    timestamp: now_iso(),
                });
                drop(s);

                sync_comments_to_ui(&ui, &st.borrow());
                ui.set_doc_save_status(format!("Comment #{} added", id).into());
                tracing::info!(id, "Comment added");
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        let st = Rc::clone(&state);
        ui.on_doc_delete_comment(move |index| {
            let mut s = st.borrow_mut();
            let idx = index as usize;
            if idx < s.comments.len() {
                let removed_id = s.comments[idx].id;
                s.comments.remove(idx);
                drop(s);

                if let Some(ui) = ui_weak.upgrade() {
                    sync_comments_to_ui(&ui, &st.borrow());
                    ui.set_doc_save_status(format!("Comment #{} deleted", removed_id).into());
                }
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        let st = Rc::clone(&state);
        ui.on_doc_resolve_comment(move |index| {
            let mut s = st.borrow_mut();
            let idx = index as usize;
            if idx < s.comments.len() {
                s.comments[idx].resolved = true;
                let id = s.comments[idx].id;
                drop(s);

                if let Some(ui) = ui_weak.upgrade() {
                    sync_comments_to_ui(&ui, &st.borrow());
                    ui.set_doc_save_status(format!("Comment #{} resolved", id).into());
                }
            }
        });
    }

    // ━━ Track Changes ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    {
        let ui_weak = ui.as_weak();
        let st = Rc::clone(&state);
        ui.on_doc_toggle_track_changes(move || {
            let new_state;
            {
                let mut s = st.borrow_mut();
                s.track_changes_on = !s.track_changes_on;
                new_state = s.track_changes_on;
            }
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_doc_track_changes_on(new_state);
                ui.set_doc_save_status(
                    if new_state {
                        "Track changes: ON"
                    } else {
                        "Track changes: OFF"
                    }
                    .into(),
                );
                tracing::info!(enabled = new_state, "Track changes toggled");
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        let st = Rc::clone(&state);
        ui.on_doc_accept_change(move |index| {
            let mut s = st.borrow_mut();
            let idx = index as usize;
            if idx < s.changes.len() {
                let change_id = s.changes[idx].id;
                s.changes.remove(idx);
                drop(s);

                if let Some(ui) = ui_weak.upgrade() {
                    sync_changes_to_ui(&ui, &st.borrow());
                    ui.set_doc_save_status(format!("Change #{} accepted", change_id).into());
                }
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        let st = Rc::clone(&state);
        ui.on_doc_reject_change(move |index| {
            if let Some(ui) = ui_weak.upgrade() {
                let idx = index as usize;
                let maybe_change = {
                    let s = st.borrow();
                    s.changes.get(idx).cloned()
                };

                if let Some(change) = maybe_change {
                    // Attempt to revert: if old_text is non-empty and new_text was inserted,
                    // replace new_text with old_text in the content.
                    if !change.new_text.is_empty() {
                        let content = ui.get_doc_content().to_string();
                        if let Some(pos) = content.find(&change.new_text) {
                            let reverted = format!(
                                "{}{}{}",
                                &content[..pos],
                                change.old_text,
                                &content[pos + change.new_text.len()..]
                            );
                            ui.set_doc_content(reverted.clone().into());
                            update_stats(&ui, &reverted);
                        }
                    }

                    {
                        let mut s = st.borrow_mut();
                        if idx < s.changes.len() {
                            s.changes.remove(idx);
                        }
                    }

                    sync_changes_to_ui(&ui, &st.borrow());
                    ui.set_doc_save_status(
                        format!("Change #{} rejected", change.id).into(),
                    );
                }
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        let st = Rc::clone(&state);
        ui.on_doc_accept_all_changes(move || {
            {
                let mut s = st.borrow_mut();
                s.changes.clear();
            }
            if let Some(ui) = ui_weak.upgrade() {
                sync_changes_to_ui(&ui, &st.borrow());
                ui.set_doc_save_status("All changes accepted".into());
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        let st = Rc::clone(&state);
        ui.on_doc_reject_all_changes(move || {
            // Reject all by reverting to earliest tracked version if available,
            // otherwise just clear the changes list.
            {
                let mut s = st.borrow_mut();
                s.changes.clear();
            }
            if let Some(ui) = ui_weak.upgrade() {
                sync_changes_to_ui(&ui, &st.borrow());
                ui.set_doc_save_status("All changes rejected (cleared)".into());
            }
        });
    }

    // ━━ Version Snapshots ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    {
        let ui_weak = ui.as_weak();
        let st = Rc::clone(&state);
        ui.on_doc_save_version(move |label| {
            if let Some(ui) = ui_weak.upgrade() {
                let content = ui.get_doc_content().to_string();
                st.borrow_mut().save_version(&label.to_string(), &content);
                sync_versions_to_ui(&ui, &st.borrow());
                ui.set_doc_save_status(format!("Version '{}' saved", label).into());
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        let st = Rc::clone(&state);
        ui.on_doc_list_versions(move || {
            if let Some(ui) = ui_weak.upgrade() {
                sync_versions_to_ui(&ui, &st.borrow());
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        let st = Rc::clone(&state);
        ui.on_doc_restore_version(move |index| {
            if let Some(ui) = ui_weak.upgrade() {
                let idx = index as usize;
                let version_content = {
                    let s = st.borrow();
                    s.versions.get(idx).map(|v| v.content.clone())
                };

                if let Some(content) = version_content {
                    // Save current state as a version before restoring
                    let current = ui.get_doc_content().to_string();
                    st.borrow_mut()
                        .save_version("Before restore", &current);

                    ui.set_doc_content(content.clone().into());
                    ui.set_doc_is_modified(true);
                    ui.set_doc_save_status(format!("Restored version #{}", index + 1).into());
                    update_stats(&ui, &content);
                    sync_versions_to_ui(&ui, &st.borrow());
                } else {
                    ui.set_doc_save_status("Version not found".into());
                }
            }
        });
    }

    // ━━ Tables ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    {
        let ui_weak = ui.as_weak();
        ui.on_doc_insert_table(move |rows, cols| {
            if let Some(ui) = ui_weak.upgrade() {
                let table = generate_markdown_table(rows, cols);
                append_to_content(&ui, &format!("\n{}", table));
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        ui.on_doc_add_table_row(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let content = ui.get_doc_content().to_string();
                let new_content = add_table_row(&content);
                if new_content != content {
                    ui.set_doc_content(new_content.clone().into());
                    ui.set_doc_is_modified(true);
                    ui.set_doc_save_status("Table row added".into());
                    update_stats(&ui, &new_content);
                } else {
                    ui.set_doc_save_status("No table found to add row".into());
                }
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        ui.on_doc_add_table_col(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let content = ui.get_doc_content().to_string();
                let new_content = add_table_col(&content);
                if new_content != content {
                    ui.set_doc_content(new_content.clone().into());
                    ui.set_doc_is_modified(true);
                    ui.set_doc_save_status("Table column added".into());
                    update_stats(&ui, &new_content);
                } else {
                    ui.set_doc_save_status("No table found to add column".into());
                }
            }
        });
    }

    // ━━ Images ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    {
        let ui_weak = ui.as_weak();
        ui.on_doc_insert_image(move |path| {
            if let Some(ui) = ui_weak.upgrade() {
                let img_ref = format!("![]({})", path);
                append_to_content(&ui, &format!("\n{}\n", img_ref));
                ui.set_doc_save_status("Image inserted".into());
            }
        });
    }

    // ━━ Templates ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    {
        let ui_weak = ui.as_weak();
        let st = Rc::clone(&state);
        ui.on_doc_use_template(move |template_idx| {
            if let Some(ui) = ui_weak.upgrade() {
                let (title, content) = get_template(template_idx);

                // Reset state for new template
                st.borrow_mut().reset();

                ui.set_doc_title(title.into());
                ui.set_doc_content(content.clone().into());
                ui.set_doc_file_path("".into());
                ui.set_doc_is_modified(false);
                ui.set_doc_save_status("Template applied".into());
                ui.set_doc_track_changes_on(false);
                update_stats(&ui, &content);
                sync_comments_to_ui(&ui, &st.borrow());
                sync_changes_to_ui(&ui, &st.borrow());
                sync_versions_to_ui(&ui, &st.borrow());
                sync_properties_to_ui(&ui, &st.borrow());
            }
        });
    }

    // ━━ Autosave timer (30 seconds) + auto-version (10 minutes) ━━

    let autosave_timer = Timer::default();
    {
        let ui_weak = ui.as_weak();
        let st = Rc::clone(&state);
        autosave_timer.start(TimerMode::Repeated, Duration::from_secs(30), move || {
            let (is_mod, has_file) = {
                let s = st.borrow();
                (s.is_modified, s.current_file.is_some())
            };

            if is_mod && has_file {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.invoke_doc_save();
                }
            }

            // Auto-version every 10 minutes (600 seconds)
            let now = now_epoch();
            let should_auto_version = {
                let s = st.borrow();
                now - s.last_auto_version >= 600 && s.current_file.is_some()
            };

            if should_auto_version {
                if let Some(ui) = ui_weak.upgrade() {
                    let content = ui.get_doc_content().to_string();
                    if !content.is_empty() {
                        st.borrow_mut().last_auto_version = now;
                        st.borrow_mut()
                            .save_version("Auto-snapshot", &content);
                        sync_versions_to_ui(&ui, &st.borrow());
                    }
                }
            }

            // Update "Last saved" status
            if let Some(ui) = ui_weak.upgrade() {
                let last_save = st.borrow().last_save_epoch;
                if last_save > 0 {
                    let ago = now.saturating_sub(last_save);
                    if ago < 60 {
                        ui.set_doc_save_status("Just now".into());
                    } else {
                        ui.set_doc_save_status(format!("{}m ago", ago / 60).into());
                    }
                }
            }
        });
    }

    // Keep the timer alive
    std::mem::forget(autosave_timer);

    // ── AI Assist ──
    wire_ai(ui, ctx);
}

/// Wire AI assist callbacks for yDoc.
fn wire_ai(ui: &App, ctx: &AppContext) {
    let bridge = ctx.bridge.clone();
    let ai_state = super::ai_assist::AiAssistState::new();

    // Free-form AI submit
    {
        let bridge = bridge.clone();
        let ai_st = ai_state.clone();
        let ui_weak = ui.as_weak();
        ui.on_doc_ai_submit(move |prompt| {
            let prompt_str = prompt.to_string();
            if prompt_str.trim().is_empty() { return; }
            let ui = match ui_weak.upgrade() { Some(u) => u, None => return };
            let content = ui.get_doc_content().to_string();
            let full_prompt = super::ai_assist::office_freeform_prompt("document editor", &content, &prompt_str);
            super::ai_assist::ai_request(
                &ui_weak,
                &bridge,
                &ai_st,
                super::ai_assist::AiAssistRequest {
                    prompt: full_prompt,
                    timeout_secs: 45,
                    set_working: Box::new(|ui, v| ui.set_doc_ai_working(v)),
                    set_response: Box::new(|ui, s| ui.set_doc_ai_response(s.into())),
                    get_response: Box::new(|ui| ui.get_doc_ai_response().to_string()),
                },
            );
        });
    }

    // Draft from instruction
    {
        let bridge = bridge.clone();
        let ai_st = ai_state.clone();
        let ui_weak = ui.as_weak();
        ui.on_doc_ai_draft(move |instruction| {
            let instr = instruction.to_string();
            let desc = if instr.trim().is_empty() {
                "Continue writing based on the existing content".to_string()
            } else {
                instr
            };
            let ui = match ui_weak.upgrade() { Some(u) => u, None => return };
            let content = ui.get_doc_content().to_string();
            let full_prompt = super::ai_assist::doc_draft_prompt(&desc, &content);
            super::ai_assist::ai_request(
                &ui_weak,
                &bridge,
                &ai_st,
                super::ai_assist::AiAssistRequest {
                    prompt: full_prompt,
                    timeout_secs: 60,
                    set_working: Box::new(|ui, v| ui.set_doc_ai_working(v)),
                    set_response: Box::new(|ui, s| ui.set_doc_ai_response(s.into())),
                    get_response: Box::new(|ui| ui.get_doc_ai_response().to_string()),
                },
            );
        });
    }

    // Summarize
    {
        let bridge = bridge.clone();
        let ai_st = ai_state.clone();
        let ui_weak = ui.as_weak();
        ui.on_doc_ai_summarize(move || {
            let ui = match ui_weak.upgrade() { Some(u) => u, None => return };
            let content = ui.get_doc_content().to_string();
            if content.trim().is_empty() { return; }
            let full_prompt = super::ai_assist::doc_summarize_prompt(&content);
            super::ai_assist::ai_request(
                &ui_weak,
                &bridge,
                &ai_st,
                super::ai_assist::AiAssistRequest {
                    prompt: full_prompt,
                    timeout_secs: 45,
                    set_working: Box::new(|ui, v| ui.set_doc_ai_working(v)),
                    set_response: Box::new(|ui, s| ui.set_doc_ai_response(s.into())),
                    get_response: Box::new(|ui| ui.get_doc_ai_response().to_string()),
                },
            );
        });
    }

    // Improve
    {
        let bridge = bridge.clone();
        let ai_st = ai_state.clone();
        let ui_weak = ui.as_weak();
        ui.on_doc_ai_improve(move || {
            let ui = match ui_weak.upgrade() { Some(u) => u, None => return };
            let content = ui.get_doc_content().to_string();
            if content.trim().is_empty() { return; }
            let full_prompt = super::ai_assist::doc_improve_prompt(&content);
            super::ai_assist::ai_request(
                &ui_weak,
                &bridge,
                &ai_st,
                super::ai_assist::AiAssistRequest {
                    prompt: full_prompt,
                    timeout_secs: 45,
                    set_working: Box::new(|ui, v| ui.set_doc_ai_working(v)),
                    set_response: Box::new(|ui, s| ui.set_doc_ai_response(s.into())),
                    get_response: Box::new(|ui| ui.get_doc_ai_response().to_string()),
                },
            );
        });
    }

    // Translate
    {
        let bridge = bridge.clone();
        let ai_st = ai_state.clone();
        let ui_weak = ui.as_weak();
        ui.on_doc_ai_translate(move |lang| {
            let ui = match ui_weak.upgrade() { Some(u) => u, None => return };
            let content = ui.get_doc_content().to_string();
            if content.trim().is_empty() { return; }
            let full_prompt = super::ai_assist::doc_translate_prompt(&content, &lang.to_string());
            super::ai_assist::ai_request(
                &ui_weak,
                &bridge,
                &ai_st,
                super::ai_assist::AiAssistRequest {
                    prompt: full_prompt,
                    timeout_secs: 60,
                    set_working: Box::new(|ui, v| ui.set_doc_ai_working(v)),
                    set_response: Box::new(|ui, s| ui.set_doc_ai_response(s.into())),
                    get_response: Box::new(|ui| ui.get_doc_ai_response().to_string()),
                },
            );
        });
    }

    // Apply AI response (append to document)
    {
        let ui_weak = ui.as_weak();
        ui.on_doc_ai_apply(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let response = ui.get_doc_ai_response().to_string();
                if !response.is_empty() {
                    let content = ui.get_doc_content().to_string();
                    let new_content = if content.is_empty() {
                        response
                    } else {
                        format!("{}\n\n{}", content, response)
                    };
                    ui.set_doc_content(new_content.clone().into());
                    ui.set_doc_is_modified(true);
                    ui.set_doc_save_status("Modified".into());
                    update_stats(&ui, &new_content);
                }
                ui.set_doc_ai_response("".into());
            }
        });
    }

    // Dismiss AI response
    {
        let ui_weak = ui.as_weak();
        ui.on_doc_ai_dismiss(move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_doc_ai_response("".into());
            }
        });
    }

    // Contextual insights — uses companion's recall to find related emails/calendar/docs
    {
        let bridge = bridge.clone();
        let ai_st = ai_state.clone();
        let ui_weak = ui.as_weak();
        ui.on_doc_ai_insights(move || {
            let ui = match ui_weak.upgrade() { Some(u) => u, None => return };
            let content = ui.get_doc_content().to_string();
            let title = ui.get_doc_title().to_string();
            let context = format!("Title: {}\n\n{}", title, content);
            let full_prompt = super::ai_assist::contextual_insights_prompt(&context, "document");
            super::ai_assist::ai_request(
                &ui_weak,
                &bridge,
                &ai_st,
                super::ai_assist::AiAssistRequest {
                    prompt: full_prompt,
                    timeout_secs: 60,
                    set_working: Box::new(|ui, v| ui.set_doc_ai_working(v)),
                    set_response: Box::new(|ui, s| ui.set_doc_ai_response(s.into())),
                    get_response: Box::new(|ui| ui.get_doc_ai_response().to_string()),
                },
            );
        });
    }
}

/// Count lines in content.
fn count_lines(content: &str) -> i32 {
    content.lines().count() as i32
}
