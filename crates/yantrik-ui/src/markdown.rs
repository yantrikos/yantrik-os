//! Simple block-level markdown parser for chat messages.
//!
//! Converts LLM markdown output into a flat list of styled blocks
//! that Slint can render with per-block formatting.
//!
//! Handles: code fences, headings, bullet lists, numbered lists,
//! and strips inline markdown markers.

/// A parsed content block with its type and text.
pub struct ParsedBlock {
    pub block_type: &'static str, // "text", "code", "heading", "bullet"
    pub text: String,
}

/// Parse markdown content into renderable blocks.
pub fn parse_blocks(content: &str) -> Vec<ParsedBlock> {
    let mut blocks = Vec::new();
    let mut current_type = "text";
    let mut current_text = String::new();
    let mut in_code_block = false;

    for line in content.lines() {
        // Code fence toggle
        if line.trim_start().starts_with("```") {
            if in_code_block {
                // End code block
                if !current_text.is_empty() {
                    blocks.push(ParsedBlock {
                        block_type: "code",
                        text: current_text.trim_end().to_string(),
                    });
                    current_text.clear();
                }
                in_code_block = false;
            } else {
                // Flush current text, start code block
                flush(&mut blocks, &mut current_text, current_type);
                current_type = "text";
                in_code_block = true;
            }
            continue;
        }

        if in_code_block {
            if !current_text.is_empty() {
                current_text.push('\n');
            }
            current_text.push_str(line);
            continue;
        }

        let trimmed = line.trim();

        // Empty line → paragraph break
        if trimmed.is_empty() {
            flush(&mut blocks, &mut current_text, current_type);
            current_type = "text";
            continue;
        }

        // Heading (### > ## > #)
        if let Some(heading_text) = strip_heading(trimmed) {
            flush(&mut blocks, &mut current_text, current_type);
            current_type = "text";
            blocks.push(ParsedBlock {
                block_type: "heading",
                text: strip_inline(&heading_text),
            });
            continue;
        }

        // Bullet list item (- item, * item, • item)
        if let Some(bullet_text) = strip_bullet(trimmed) {
            if current_type != "bullet" {
                flush(&mut blocks, &mut current_text, current_type);
            }
            current_type = "bullet";
            if !current_text.is_empty() {
                current_text.push('\n');
            }
            current_text.push_str(&format!("\u{2022} {}", strip_inline(&bullet_text)));
            continue;
        }

        // Numbered list (1. item, 2. item, etc.)
        if let Some((num, list_text)) = strip_numbered(trimmed) {
            if current_type != "bullet" {
                flush(&mut blocks, &mut current_text, current_type);
            }
            current_type = "bullet";
            if !current_text.is_empty() {
                current_text.push('\n');
            }
            current_text.push_str(&format!("{}. {}", num, strip_inline(&list_text)));
            continue;
        }

        // Normal text paragraph
        if current_type != "text" {
            flush(&mut blocks, &mut current_text, current_type);
        }
        current_type = "text";
        if !current_text.is_empty() {
            current_text.push(' ');
        }
        current_text.push_str(&strip_inline(trimmed));
    }

    // Flush remaining content
    if !current_text.is_empty() {
        let bt = if in_code_block { "code" } else { current_type };
        blocks.push(ParsedBlock {
            block_type: bt,
            text: current_text.trim_end().to_string(),
        });
    }

    // If no blocks were parsed, return content as single text block
    if blocks.is_empty() && !content.trim().is_empty() {
        blocks.push(ParsedBlock {
            block_type: "text",
            text: strip_inline(content.trim()),
        });
    }

    blocks
}

fn flush(blocks: &mut Vec<ParsedBlock>, text: &mut String, block_type: &'static str) {
    if !text.is_empty() {
        blocks.push(ParsedBlock {
            block_type,
            text: text.trim_end().to_string(),
        });
        text.clear();
    }
}

fn strip_heading(line: &str) -> Option<String> {
    if let Some(rest) = line.strip_prefix("### ") {
        Some(rest.trim().to_string())
    } else if let Some(rest) = line.strip_prefix("## ") {
        Some(rest.trim().to_string())
    } else if let Some(rest) = line.strip_prefix("# ") {
        Some(rest.trim().to_string())
    } else {
        None
    }
}

fn strip_bullet(line: &str) -> Option<String> {
    if let Some(rest) = line.strip_prefix("- ") {
        Some(rest.to_string())
    } else if let Some(rest) = line.strip_prefix("* ") {
        Some(rest.to_string())
    } else {
        // Also match Unicode bullet (•)
        let bullet = "\u{2022} ";
        line.strip_prefix(bullet).map(|rest| rest.to_string())
    }
}

fn strip_numbered(line: &str) -> Option<(String, String)> {
    let num_end = line.find(". ")?;
    let num_part = &line[..num_end];
    if num_part.len() <= 3 && num_part.chars().all(|c| c.is_ascii_digit()) {
        Some((num_part.to_string(), line[num_end + 2..].to_string()))
    } else {
        None
    }
}

/// Strip inline markdown markers: **bold**, *italic*, `code`.
fn strip_inline(text: &str) -> String {
    let mut result = text.to_string();

    // Bold: **text**
    while let Some(start) = result.find("**") {
        if let Some(end) = result[start + 2..].find("**") {
            let inner = result[start + 2..start + 2 + end].to_string();
            result = format!("{}{}{}", &result[..start], inner, &result[start + 2 + end + 2..]);
        } else {
            break;
        }
    }

    // Italic: *text* (single asterisks remaining after bold removal)
    while let Some(start) = result.find('*') {
        if let Some(end) = result[start + 1..].find('*') {
            if end > 0 {
                let inner = result[start + 1..start + 1 + end].to_string();
                result =
                    format!("{}{}{}", &result[..start], inner, &result[start + 1 + end + 1..]);
            } else {
                break;
            }
        } else {
            break;
        }
    }

    // Inline code: `text`
    while let Some(start) = result.find('`') {
        if let Some(end) = result[start + 1..].find('`') {
            let inner = result[start + 1..start + 1 + end].to_string();
            result = format!("{}{}{}", &result[..start], inner, &result[start + 1 + end + 1..]);
        } else {
            break;
        }
    }

    result
}
