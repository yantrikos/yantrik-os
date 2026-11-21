//! The line-level pieces of `markdown::parse_blocks`: what one line says on its own (a heading, a
//! list item, a rule, a quote) and how a table's lines split into cells. Kept apart so the
//! parser stays a loop over lines.

/// A list item as it was written: how deep it sits, what its marker was, and its text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListItem {
    /// 0 for the outermost list; two more spaces of indent is one deeper.
    pub depth: usize,
    /// `Some(n)` for a numbered item (the number as written), `None` for a bullet.
    pub number: Option<u64>,
    /// The item's inline markdown, its wrapped continuation lines joined in with a space.
    pub markdown: String,
}

/// `# ` to `###### `: the heading's level and its text.
pub fn heading(line: &str) -> Option<(u8, String)> {
    let hashes = line.bytes().take_while(|b| *b == b'#').count();
    let rest = line[hashes..].strip_prefix(' ')?;
    ((1..=6).contains(&hashes)).then(|| (hashes as u8, rest.trim().to_string()))
}

/// `---`, `***` or `___` (three or more, nothing else on the line).
pub fn is_rule(line: &str) -> bool {
    let t = line.trim();
    t.len() >= 3
        && ["-", "*", "_"]
            .iter()
            .any(|c| t.chars().all(|x| c.starts_with(x)))
}

/// A block quote's line, `> ` removed.
pub fn quote(line: &str) -> Option<&str> {
    line.trim_start().strip_prefix('>').map(str::trim_start)
}

/// How far a line is indented, a tab counting four.
pub fn indent_of(line: &str) -> usize {
    line.chars()
        .take_while(|c| c.is_whitespace())
        .map(|c| if c == '\t' { 4 } else { 1 })
        .sum()
}

/// A list item's line, with its indent read as depth: `- a`, `* a`, `• a`, `3. a`.
pub fn list_item(line: &str) -> Option<ListItem> {
    let depth = indent_of(line) / 2;
    let t = line.trim();
    if let Some(rest) = ["- ", "* ", "\u{2022} "]
        .iter()
        .find_map(|m| t.strip_prefix(m))
    {
        return Some(ListItem {
            depth,
            number: None,
            markdown: rest.trim().to_string(),
        });
    }
    let end = t.find(". ")?;
    let digits = &t[..end];
    (digits.len() <= 3 && !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())).then(
        || ListItem {
            depth,
            number: digits.parse().ok(),
            markdown: t[end + 2..].trim().to_string(),
        },
    )
}

/// A table's cells on one line: split on `|` that is not escaped or inside backticks, the outer
/// pipes dropped.
pub fn cells(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t
        .strip_suffix('|')
        .filter(|_| !t.ends_with("\\|"))
        .unwrap_or(t);
    let (mut out, mut cell, mut code, mut escaped) = (Vec::new(), String::new(), false, false);
    for c in t.chars() {
        match c {
            '|' if !code && !escaped => out.push(std::mem::take(&mut cell).trim().to_string()),
            '\\' if !escaped => {
                escaped = true;
                continue;
            }
            '|' if escaped && !code => cell.push('|'),
            _ => {
                if escaped {
                    cell.push('\\');
                }
                if c == '`' {
                    code = !code;
                }
                cell.push(c);
            }
        }
        escaped = false;
    }
    out.push(cell.trim().to_string());
    out
}

/// A table's separator row: `|---|:--:|` — every cell dashes with optional colons.
pub fn is_separator(line: &str) -> bool {
    line.contains('-')
        && cells(line).iter().all(|c| {
            let c = c.trim_matches(':');
            !c.is_empty() && c.chars().all(|x| x == '-')
        })
}

/// Whether `header` and the `separator` under it open a GFM table.
pub fn opens_table(header: &str, separator: Option<&str>) -> bool {
    header.contains('|')
        && separator.is_some_and(|s| is_separator(s) && cells(s).len() == cells(header).len())
}
