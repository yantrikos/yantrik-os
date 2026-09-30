//! The Yantrik terminal: the desktop's chat with any mind, in a terminal.
//!
//! The same conversation as the Lens, read and written through the shell's own surface
//! (`chat_view`, `send_message`), so a question asked here is answered on the desktop too, and the
//! other way round. Each call a mind makes is a live card under the question that began it. What
//! needs a person's Allow is shown as waiting and answered on the desktop's card: this client runs
//! as the person, but a keypress in a terminal is not the card a person was shown.
//!
//! Run it on the desktop, or over `ssh -t` as the person.

mod app;
mod client;
mod markdown;
mod theme;
mod ui;
mod view;

use std::io;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

const TICK: Duration = Duration::from_millis(90);

fn main() -> io::Result<()> {
    if std::env::args().any(|a| a == "--help" || a == "-h") {
        println!("yantrik-term — the desktop's chat with any mind, in a terminal\n\n  Enter send · Alt+Enter new line · Up/Down history · PgUp/PgDn scroll · Ctrl+C quit\n  /help /mind /new /view /agents /mode /quit");
        return Ok(());
    }
    // Put the terminal back however this ends, a panic included.
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = restore();
        hook(info);
    }));
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let result = run(&mut terminal);
    restore()?;
    result
}

fn restore() -> io::Result<()> {
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)
}

fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
    let mut app = app::App::new();
    app.start_polling();
    let mut last_tick = Instant::now();
    while !app.quit {
        while let Ok(update) = app.rx.try_recv() {
            app.apply(update);
        }
        terminal.draw(|f| ui::draw(f, &mut app))?;
        let wait = TICK.saturating_sub(last_tick.elapsed());
        if event::poll(wait)? {
            if let Event::Key(k) = event::read()? {
                if k.kind == KeyEventKind::Press {
                    app.key(k);
                }
            }
        }
        if last_tick.elapsed() >= TICK {
            app.tick += 1;
            last_tick = Instant::now();
        }
    }
    Ok(())
}
