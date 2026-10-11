//! The screen: a header naming the mind, the conversation with each call of the current turn as
//! a live card, the input box, and a status line with what is open in Mind View.

use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use crate::app::App;
use crate::markdown;
use crate::theme;
use crate::view::{ChatView, Step};

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let input_rows = input_lines(app, area.width.saturating_sub(4) as usize).len().clamp(1, 6) as u16;
    let [header, body, input, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(input_rows + 2),
        Constraint::Length(1),
    ])
    .areas(area);
    draw_header(f, header, app);
    draw_body(f, body, app);
    draw_input(f, input, app);
    draw_footer(f, footer, app);
}

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let v = &app.view;
    let mut left = vec![
        Span::styled(" ◎ YANTRIK ", Style::default().fg(ratatui::style::Color::Black).bg(theme::ACCENT).add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(if v.mind.name.is_empty() { "no mind".to_string() } else { v.mind.name.clone() }, theme::bold(theme::text())),
    ];
    if !v.mind.detail.is_empty() {
        left.push(Span::styled(format!("  {}", v.mind.detail), theme::dim()));
    }
    let mode = if v.mode.is_empty() { "?".to_string() } else { v.mode.clone() };
    let right = vec![
        Span::styled(format!(" {mode} "), Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD)),
        Span::styled(if app.connected { "● " } else { "○ " }, Style::default().fg(if app.connected { theme::GREEN } else { theme::RED })),
    ];
    let right_w: usize = right.iter().map(|s| s.content.width()).sum();
    f.render_widget(Paragraph::new(Line::from(left)), area);
    let r = Rect { x: area.x + area.width.saturating_sub(right_w as u16), width: right_w as u16, ..area };
    f.render_widget(Paragraph::new(Line::from(right)), r);
}

fn draw_body(f: &mut Frame, area: Rect, app: &mut App) {
    let width = area.width.saturating_sub(2) as usize;
    if app.view.messages.is_empty() && !app.view.busy() {
        draw_welcome(f, area, app);
        return;
    }
    let lines = transcript(&app.view, width, app.tick);
    let height = area.height as usize;
    let max_scroll = lines.len().saturating_sub(height);
    app.scroll = app.scroll.min(max_scroll);
    let top = max_scroll - app.scroll;
    let shown: Vec<Line> = lines.into_iter().skip(top).take(height).collect();
    let inner = Rect { x: area.x + 1, width: area.width.saturating_sub(2), ..area };
    f.render_widget(Paragraph::new(shown), inner);
    if app.scroll > 0 {
        let note = format!(" ↓ {} more ", app.scroll);
        let r = Rect { x: area.x + area.width.saturating_sub(note.width() as u16 + 1), y: area.y + area.height.saturating_sub(1), width: note.width() as u16, height: 1 };
        f.render_widget(Paragraph::new(Span::styled(note, Style::default().fg(theme::AMBER))), r);
    }
}

fn draw_welcome(f: &mut Frame, area: Rect, app: &App) {
    let n = theme::WORDMARK.len();
    let mut lines: Vec<Line> = Vec::new();
    let wide = area.width as usize >= theme::WORDMARK[0].width() + 2;
    let top_pad = area.height.saturating_sub(if wide { 12 } else { 6 }) / 2;
    lines.extend((0..top_pad).map(|_| Line::from("")));
    if wide {
        for (i, row) in theme::WORDMARK.iter().enumerate() {
            lines.push(Line::from(Span::styled(*row, Style::default().fg(theme::gradient(i, n)).add_modifier(Modifier::BOLD))));
        }
    } else {
        lines.push(Line::from(Span::styled("◎ YANTRIK", theme::bold(theme::accent()))));
    }
    lines.push(Line::from(""));
    let who = if app.view.mind.name.is_empty() { "a mind".to_string() } else { app.view.mind.name.clone() };
    lines.push(Line::from(Span::styled(format!("You are talking to {who}. Ask anything; it works in Mind View."), theme::dim())));
    lines.push(Line::from(Span::styled("/help  ·  /mind  ·  /new  ·  /view  ·  Ctrl+C to leave", theme::faint())));
    f.render_widget(Paragraph::new(lines).alignment(Alignment::Center), area);
}

/// Every row of the conversation, wrapped to `width`.
pub fn transcript(v: &ChatView, width: usize, tick: u64) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();
    let last_user = v.messages.iter().rposition(|m| m.role == "user");
    for (i, m) in v.messages.iter().enumerate() {
        out.push(Line::from(""));
        match m.role.as_str() {
            "user" => {
                let runs = vec![("❯ ".to_string(), theme::bold(theme::accent())), (m.text.clone(), theme::bold(theme::text()))];
                out.extend(markdown::wrap(runs, width, "", "  "));
                if Some(i) == last_user {
                    out.extend(turn_cards(v, width, tick));
                }
            }
            _ => {
                let name = if v.mind.name.is_empty() { m.role.clone() } else { v.mind.name.clone() };
                out.push(Line::from(vec![Span::styled("◎ ", theme::accent()), Span::styled(name, theme::dim())]));
                if m.text.trim().is_empty() && m.streaming {
                    out.push(Line::from(Span::styled(format!("  {} thinking…", theme::spinner(tick)), theme::dim())));
                } else {
                    out.extend(markdown::render(&m.text, width, "  "));
                    if m.streaming {
                        out.push(Line::from(Span::styled(format!("  {}", theme::spinner(tick)), theme::accent())));
                    }
                }
            }
        }
    }
    // Asked, and nothing said back yet: the calls so far, and that it is at work.
    if last_user == Some(v.messages.len().saturating_sub(1)) && v.busy() {
        out.push(Line::from(Span::styled(format!("  {} {}", theme::spinner(tick), working_word(&v.state)), theme::dim())));
    }
    out
}

fn working_word(state: &str) -> &'static str {
    match state {
        "runningtool" => "working…",
        "waitingforyou" => "waiting for you on the desktop…",
        _ => "thinking…",
    }
}

/// The current turn's calls, approvals and questions, as a tree under the question that began it.
fn turn_cards(v: &ChatView, width: usize, tick: u64) -> Vec<Line<'static>> {
    let Some(turn) = &v.turn else { return Vec::new() };
    let n = turn.steps.len();
    let mut out = Vec::new();
    for (i, step) in turn.steps.iter().enumerate() {
        let branch = if i + 1 == n { "  └ " } else { "  ├ " };
        let (glyph, gstyle, head, rest, tail): (String, Style, String, String, String) = match step {
            Step::Call { name, label, target, args, state, summary, seconds, repeats } => {
                let (g, st) = match state.as_str() {
                    "running" => (theme::spinner(tick).to_string(), theme::accent()),
                    "ok" => ("✓".to_string(), Style::default().fg(theme::GREEN)),
                    "failed" => ("✗".to_string(), Style::default().fg(theme::RED)),
                    "interrupted" => ("!".to_string(), Style::default().fg(theme::AMBER)),
                    _ => ("·".to_string(), theme::dim()),
                };
                let head = if !label.is_empty() { label.clone() } else if !target.is_empty() { format!("{name} {target}") } else { name.clone() };
                let times = if *repeats > 1 { format!(" ×{repeats}") } else { String::new() };
                let detail = if state == "failed" && !summary.is_empty() { summary.clone() } else { args.clone() };
                let secs = seconds.map(|s| format!("  {s}s")).unwrap_or_default();
                (g, st, format!("{head}{times}"), detail, secs)
            }
            Step::Approval { what, outcome } => match outcome.as_str() {
                "pending" => ("⚠".into(), Style::default().fg(theme::AMBER), what.clone(), "waiting for you: Allow or Deny on the desktop card".into(), String::new()),
                "allowed" => ("✓".into(), Style::default().fg(theme::GREEN), what.clone(), "allowed".into(), String::new()),
                other => ("✗".into(), Style::default().fg(theme::RED), what.clone(), other.to_string(), String::new()),
            },
            Step::Question { prompt, answered } => (
                "?".into(),
                Style::default().fg(theme::AMBER),
                prompt.clone(),
                if *answered { "answered".into() } else { "answer it on the desktop".into() },
                String::new(),
            ),
            Step::Note(text) => ("·".into(), theme::faint(), text.clone(), String::new(), String::new()),
        };
        let mut runs = vec![
            (branch.to_string(), theme::faint()),
            (format!("{glyph} "), gstyle),
            (head, theme::bold(theme::text())),
        ];
        if !rest.is_empty() {
            runs.push(("  ".to_string(), theme::dim()));
            runs.push((rest, theme::dim()));
        }
        if !tail.is_empty() {
            runs.push((tail, theme::faint()));
        }
        let mut rows = markdown::wrap(runs, width, "", "  │   ");
        rows.truncate(2); // a card is a glance; the Agents pane has the rest
        out.extend(rows);
    }
    out
}

/// The input as rows, for sizing the box.
fn input_lines(app: &App, width: usize) -> Vec<String> {
    let mut rows = Vec::new();
    for (i, line) in app.input.split('\n').enumerate() {
        let lead = if i == 0 { "❯ " } else { "  " };
        let text = format!("{lead}{line}");
        let chars: Vec<char> = text.chars().collect();
        if chars.is_empty() {
            rows.push(String::new());
            continue;
        }
        for chunk in chars.chunks(width.max(4)) {
            rows.push(chunk.iter().collect());
        }
    }
    rows
}

fn draw_input(f: &mut Frame, area: Rect, app: &App) {
    let border = if app.view.busy() { theme::ACCENT_DEEP } else { theme::FAINT };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let rows = input_lines(app, inner.width as usize);
    let shown: Vec<Line> = if app.input.is_empty() {
        vec![Line::from(vec![
            Span::styled("❯ ", theme::bold(theme::accent())),
            Span::styled(if app.view.busy() { "the mind is at work · you can still type" } else { "Ask anything · /help" }, theme::faint()),
        ])]
    } else {
        let skip = rows.len().saturating_sub(inner.height as usize);
        rows.iter()
            .skip(skip)
            .enumerate()
            .map(|(i, r)| {
                if i == 0 && skip == 0 {
                    Line::from(vec![Span::styled("❯ ", theme::bold(theme::accent())), Span::styled(r.chars().skip(2).collect::<String>(), theme::text())])
                } else {
                    Line::from(Span::styled(r.clone(), theme::text()))
                }
            })
            .collect()
    };
    f.render_widget(Paragraph::new(shown), inner);
    // The cursor at the end of what is typed.
    if let Some(last) = rows.last() {
        let visible = rows.len().min(inner.height as usize);
        let x = inner.x + (last.width() as u16).min(inner.width.saturating_sub(1));
        let y = inner.y + visible.saturating_sub(1) as u16;
        f.set_cursor_position((x, y));
    }
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let v = &app.view;
    let mut spans: Vec<Span> = Vec::new();
    if let Some(note) = &app.note {
        spans.push(Span::styled(format!(" {note} "), Style::default().fg(theme::AMBER)));
    } else {
        let mv = if v.mind_view_apps.is_empty() {
            if v.mind_view_running { "Mind View: empty".to_string() } else { "Mind View: closed".to_string() }
        } else {
            format!("Mind View: {}", v.mind_view_apps.join(", "))
        };
        spans.push(Span::styled(format!(" ◎ {mv}"), theme::dim()));
        if let Some(t) = &v.turn {
            let calls = t.steps.iter().filter(|s| matches!(s, Step::Call { .. })).count();
            if calls > 0 {
                spans.push(Span::styled(format!("  ·  {calls} calls"), theme::faint()));
            }
        }
        if v.waiting_on_you > 0 {
            spans.push(Span::styled(format!("  ·  ⚠ {} waiting for you on the desktop", v.waiting_on_you), Style::default().fg(theme::AMBER)));
        }
    }
    let hint = " /help · PgUp/PgDn · Ctrl+C ";
    f.render_widget(Paragraph::new(Line::from(spans)), area);
    let r = Rect { x: area.x + area.width.saturating_sub(hint.width() as u16), width: hint.width() as u16, ..area };
    f.render_widget(Paragraph::new(Span::styled(hint, theme::faint())), r);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::{Message, Mind, Turn};

    fn plain(lines: &[Line]) -> String {
        lines.iter().map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect::<String>()).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn the_turns_calls_sit_under_the_question_that_began_it() {
        let v = ChatView {
            messages: vec![
                Message { index: 0, role: "user".into(), text: "make a scene".into(), streaming: false },
                Message { index: 1, role: "assistant".into(), text: "Done: **rendered**.".into(), streaming: false },
            ],
            mind: Mind { name: "Yantrik Mind".into(), ..Default::default() },
            turn: Some(Turn {
                n: 1,
                prompt: "make a scene".into(),
                ended: true,
                ok: Some(true),
                steps: vec![
                    Step::Call { name: "os_act".into(), label: "blender.add_primitive".into(), target: String::new(), args: "kind=monkey".into(), state: "ok".into(), summary: String::new(), seconds: Some(1), repeats: 0 },
                    Step::Approval { what: "blender.new_scene".into(), outcome: "pending".into() },
                ],
            }),
            ..Default::default()
        };
        let text = plain(&transcript(&v, 80, 0));
        let q = text.find("❯ make a scene").unwrap();
        let call = text.find("✓ blender.add_primitive  kind=monkey  1s").unwrap();
        let ask = text.find("⚠ blender.new_scene  waiting for you").unwrap();
        let reply = text.find("Done: rendered.").unwrap();
        assert!(q < call && call < ask && ask < reply, "{text}");
        assert!(text.contains("├ ") && text.contains("└ "), "{text}");
    }
}
