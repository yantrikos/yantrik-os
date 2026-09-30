//! A mind's reply, drawn: the markdown minds actually write (headings, lists, code blocks, quotes,
//! `code` and **bold**), wrapped to the width here, so the screen knows exactly how many rows a
//! message takes and can scroll by rows.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::theme;

/// One styled run of text.
pub type Run = (String, Style);

/// `text` as rows no wider than `width`, each starting with `indent`.
pub fn render(text: &str, width: usize, indent: &str) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let mut in_code = false;
    for raw in text.lines() {
        let line = raw.trim_end();
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
            if in_code {
                let lang = line.trim_start().trim_start_matches('`').trim();
                let label = if lang.is_empty() { "code".to_string() } else { lang.to_string() };
                out.push(Line::from(vec![Span::raw(indent.to_string()), Span::styled(format!("╭─ {label}"), theme::faint())]));
            } else {
                out.push(Line::from(vec![Span::raw(indent.to_string()), Span::styled("╰─", theme::faint())]));
            }
            continue;
        }
        if in_code {
            let runs = vec![("│ ".to_string(), theme::faint()), (line.to_string(), Style::default().fg(theme::CODE))];
            out.extend(wrap(runs, width, indent, &format!("{indent}│ ")));
            continue;
        }
        let trimmed = line.trim_start();
        let lead = &line[..line.len() - trimmed.len()];
        let (prefix, body, style) = if let Some(h) = trimmed.strip_prefix("### ").or_else(|| trimmed.strip_prefix("## ")) {
            (String::new(), h, theme::bold(theme::accent()))
        } else if let Some(h) = trimmed.strip_prefix("# ") {
            (String::new(), h, theme::bold(theme::accent()).add_modifier(Modifier::UNDERLINED))
        } else if let Some(b) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) {
            (format!("{lead}• "), b, theme::text())
        } else if let Some((n, rest)) = numbered(trimmed) {
            (format!("{lead}{n}. "), rest, theme::text())
        } else if let Some(q) = trimmed.strip_prefix("> ") {
            ("│ ".to_string(), q, theme::dim().add_modifier(Modifier::ITALIC))
        } else {
            (lead.to_string(), trimmed, theme::text())
        };
        if body.is_empty() && prefix.is_empty() {
            out.push(Line::from(indent.to_string()));
            continue;
        }
        let mut runs: Vec<Run> = Vec::new();
        if !prefix.is_empty() {
            runs.push((prefix.clone(), theme::accent()));
        }
        runs.extend(inline(body, style));
        let hang = format!("{indent}{}", " ".repeat(prefix.width()));
        out.extend(wrap(runs, width, indent, &hang));
    }
    out
}

fn numbered(s: &str) -> Option<(&str, &str)> {
    let dot = s.find(". ")?;
    let n = &s[..dot];
    (!n.is_empty() && n.len() <= 3 && n.chars().all(|c| c.is_ascii_digit())).then(|| (n, &s[dot + 2..]))
}

/// `code`, **bold** and [text](url) inside one line.
pub fn inline(s: &str, base: Style) -> Vec<Run> {
    let mut runs = Vec::new();
    let mut rest = s;
    while !rest.is_empty() {
        let next = [rest.find('`'), rest.find("**"), rest.find('[')].into_iter().flatten().min();
        let Some(at) = next else {
            runs.push((rest.to_string(), base));
            break;
        };
        if at > 0 {
            runs.push((rest[..at].to_string(), base));
        }
        let tail = &rest[at..];
        if let Some(inner) = tail.strip_prefix('`') {
            if let Some(end) = inner.find('`') {
                runs.push((inner[..end].to_string(), Style::default().fg(theme::CODE)));
                rest = &inner[end + 1..];
                continue;
            }
        } else if let Some(inner) = tail.strip_prefix("**") {
            if let Some(end) = inner.find("**") {
                runs.push((inner[..end].to_string(), base.add_modifier(Modifier::BOLD)));
                rest = &inner[end + 2..];
                continue;
            }
        } else if let Some(inner) = tail.strip_prefix('[') {
            if let Some(close) = inner.find("](") {
                if let Some(end) = inner[close + 2..].find(')') {
                    runs.push((inner[..close].to_string(), base.add_modifier(Modifier::UNDERLINED)));
                    rest = &inner[close + 2 + end + 1..];
                    continue;
                }
            }
        }
        // Not markup after all: the character stands as itself.
        let ch = tail.chars().next().unwrap();
        runs.push((ch.to_string(), base));
        rest = &tail[ch.len_utf8()..];
    }
    runs
}

/// Styled runs as rows no wider than `width`: the first starting with `first`, the rest with
/// `hang`. Breaks at spaces where it can, and inside a word only when the word is longer than a row.
pub fn wrap(runs: Vec<Run>, width: usize, first: &str, hang: &str) -> Vec<Line<'static>> {
    let width = width.max(8);
    let mut lines = Vec::new();
    let mut current: Vec<Span<'static>> = vec![Span::raw(first.to_string())];
    let mut used = first.width();
    // Words keep their style; spaces between them are where a row may end.
    let mut words: Vec<(String, Style, bool)> = Vec::new();
    for (text, style) in runs {
        let mut word = String::new();
        for ch in text.chars() {
            if ch == ' ' {
                if !word.is_empty() {
                    words.push((std::mem::take(&mut word), style, false));
                }
                words.push((" ".to_string(), style, true));
            } else {
                word.push(ch);
            }
        }
        if !word.is_empty() {
            words.push((word, style, false));
        }
    }
    for (word, style, space) in words {
        let w = word.width();
        if space {
            if used + 1 <= width && used > hang.width().min(first.width()) {
                current.push(Span::styled(word, style));
                used += 1;
            }
            continue;
        }
        if used + w > width && used > hang.width() {
            lines.push(Line::from(std::mem::take(&mut current)));
            current.push(Span::raw(hang.to_string()));
            used = hang.width();
        }
        if w > width.saturating_sub(used) {
            // A word longer than a row: cut it where the row ends.
            let mut chunk = String::new();
            for ch in word.chars() {
                let cw = ch.to_string().width();
                if used + cw > width {
                    current.push(Span::styled(std::mem::take(&mut chunk), style));
                    lines.push(Line::from(std::mem::take(&mut current)));
                    current.push(Span::raw(hang.to_string()));
                    used = hang.width();
                }
                chunk.push(ch);
                used += cw;
            }
            if !chunk.is_empty() {
                current.push(Span::styled(chunk, style));
            }
        } else {
            current.push(Span::styled(word, style));
            used += w;
        }
    }
    lines.push(Line::from(current));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(lines: &[Line]) -> Vec<String> {
        lines.iter().map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect::<String>()).collect()
    }

    #[test]
    fn rows_never_run_past_the_width() {
        let text = "The quick brown fox jumps over the lazy dog and keeps running far beyond the edge.";
        for width in [12, 20, 40] {
            for row in plain(&render(text, width, "  ")) {
                assert!(row.width() <= width, "{width}: {row:?}");
            }
        }
        let long = "x".repeat(50);
        for row in plain(&render(&long, 16, "")) {
            assert!(row.width() <= 16, "{row:?}");
        }
    }

    #[test]
    fn markdown_minds_write_is_drawn_not_printed() {
        let rows = plain(&render("## Plan\n- **pedestal** at `0,0,0.5`\n1. render\n```py\nimport bpy\n```", 60, ""));
        assert_eq!(rows[0], "Plan");
        assert_eq!(rows[1], "• pedestal at 0,0,0.5");
        assert_eq!(rows[2], "1. render");
        assert_eq!(rows[3], "╭─ py");
        assert_eq!(rows[4], "│ import bpy");
        assert_eq!(rows[5], "╰─");
        assert_eq!(plain(&render("a [link](http://x) b", 40, ""))[0], "a link b");
        assert_eq!(plain(&render("unclosed `tick and * star", 40, ""))[0], "unclosed `tick and * star");
    }
}
