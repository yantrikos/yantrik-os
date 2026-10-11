//! The desktop's colours, in the terminal: the teal accent, slate text, amber for anything waiting
//! on the person, red for a failure.

use ratatui::style::{Color, Modifier, Style};

pub const ACCENT: Color = Color::Rgb(45, 212, 191);
pub const ACCENT_DEEP: Color = Color::Rgb(20, 184, 166);
pub const VIOLET: Color = Color::Rgb(167, 139, 250);
pub const TEXT: Color = Color::Rgb(226, 232, 240);
pub const DIM: Color = Color::Rgb(148, 163, 184);
pub const FAINT: Color = Color::Rgb(100, 116, 139);
pub const AMBER: Color = Color::Rgb(245, 158, 11);
pub const RED: Color = Color::Rgb(248, 113, 113);
pub const GREEN: Color = Color::Rgb(74, 222, 128);
pub const CODE: Color = Color::Rgb(125, 211, 252);

pub fn text() -> Style {
    Style::default().fg(TEXT)
}
pub fn dim() -> Style {
    Style::default().fg(DIM)
}
pub fn faint() -> Style {
    Style::default().fg(FAINT)
}
pub fn accent() -> Style {
    Style::default().fg(ACCENT)
}
pub fn bold(style: Style) -> Style {
    style.add_modifier(Modifier::BOLD)
}

/// The wordmark shown on an empty chat, a line at a time from teal to violet.
pub const WORDMARK: [&str; 6] = [
    "██╗   ██╗ █████╗ ███╗   ██╗████████╗██████╗ ██╗██╗  ██╗",
    "╚██╗ ██╔╝██╔══██╗████╗  ██║╚══██╔══╝██╔══██╗██║██║ ██╔╝",
    " ╚████╔╝ ███████║██╔██╗ ██║   ██║   ██████╔╝██║█████╔╝ ",
    "  ╚██╔╝  ██╔══██║██║╚██╗██║   ██║   ██╔══██╗██║██╔═██╗ ",
    "   ██║   ██║  ██║██║ ╚████║   ██║   ██║  ██║██║██║  ██╗",
    "   ╚═╝   ╚═╝  ╚═╝╚═╝  ╚═══╝   ╚═╝   ╚═╝  ╚═╝╚═╝╚═╝  ╚═╝",
];

/// Line `i` of `n` along the wordmark's gradient.
pub fn gradient(i: usize, n: usize) -> Color {
    let t = if n <= 1 { 0.0 } else { i as f32 / (n - 1) as f32 };
    let (a, b) = ((45.0, 212.0, 191.0), (167.0, 139.0, 250.0));
    let mix = |x: f32, y: f32| (x + (y - x) * t).round() as u8;
    Color::Rgb(mix(a.0, b.0), mix(a.1, b.1), mix(a.2, b.2))
}

/// A spinner frame for a tick.
pub fn spinner(tick: u64) -> &'static str {
    const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    FRAMES[(tick as usize) % FRAMES.len()]
}
