//! What the Agents screen keeps about each agent: who it is, what state it is in, and its session —
//! the person's prompts, the mind's text and thinking, and one card per tool call with the call's
//! own output inside it.
//!
//! Nothing here talks to a window or a socket. The store (`store.rs`) owns the rules that change
//! these values; this file is the vocabulary, plus the two pieces of mechanism a card needs to hold
//! its output: a byte buffer that keeps a head and a tail under a cap, and a terminal emulator for
//! a command's bytes.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

pub use yantrik_harness::event::{AgentId, Event, Stream};

/// Unix seconds, from the wall clock.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The most of one event's text the store takes. The host already refuses anything larger
/// (design decision 2); this is the store not relying on that.
pub const EVENT_CAP: usize = 64 * 1024;

/// What one call keeps of its output: the first [`CARD_HEAD`] bytes and the last of the rest, with
/// a marker saying how much fell between them.
pub const CARD_CAP: usize = 2 * 1024 * 1024;
pub const CARD_HEAD: usize = 256 * 1024;

/// The same for one block of the mind's text or thinking.
pub const TEXT_CAP: usize = 256 * 1024;
pub const TEXT_HEAD: usize = 32 * 1024;

/// The size a card's terminal is drawn at. A PTY whose bytes go to a card should be opened this
/// size, so a program formats its output for the screen that will show it.
pub const TERMINAL_ROWS: u16 = 24;
pub const TERMINAL_COLS: u16 = 120;

/// What an agent is doing, as the list and the tabs read it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// A turn is open and the mind is producing text or thinking.
    Thinking,
    /// A turn is open and at least one call is running.
    RunningTool,
    /// The agent cannot go on until the person answers something: an approval, a password.
    WaitingForYou,
    /// Known, and nothing is running: not started yet, or picked up again after a restart.
    Idle,
    /// Its last turn finished.
    Done,
    /// Its last turn failed.
    Failed,
    /// The harness that ran it stopped polling (design: "When a harness dies").
    HarnessGone,
}

impl State {
    pub fn key(self) -> &'static str {
        match self {
            State::Thinking => "thinking",
            State::RunningTool => "running_tool",
            State::WaitingForYou => "waiting_for_you",
            State::Idle => "idle",
            State::Done => "done",
            State::Failed => "failed",
            State::HarnessGone => "harness_gone",
        }
    }

    /// How the list says it.
    pub fn label(self) -> &'static str {
        match self {
            State::Thinking => "thinking",
            State::RunningTool => "running a tool",
            State::WaitingForYou => "waiting for you",
            State::Idle => "idle",
            State::Done => "done",
            State::Failed => "failed",
            State::HarnessGone => "harness gone",
        }
    }

    /// Doing something right now — the states a Close has to ask about.
    pub fn working(self) -> bool {
        matches!(self, State::Thinking | State::RunningTool | State::WaitingForYou)
    }

    /// Still able to move on its own or on the next prompt: the Active tab.
    pub fn live(self) -> bool {
        self.working() || self == State::Idle
    }
}

/// The four views over the one list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tab {
    Active,
    NeedsYou,
    Complete,
    All,
}

impl Tab {
    pub const EVERY: [Tab; 4] = [Tab::Active, Tab::NeedsYou, Tab::Complete, Tab::All];

    pub fn key(self) -> &'static str {
        match self {
            Tab::Active => "active",
            Tab::NeedsYou => "needs_you",
            Tab::Complete => "complete",
            Tab::All => "all",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Tab::Active => "Active",
            Tab::NeedsYou => "Needs you",
            Tab::Complete => "Complete",
            Tab::All => "All",
        }
    }

    pub fn from_key(key: &str) -> Tab {
        Tab::EVERY.into_iter().find(|t| t.key() == key).unwrap_or(Tab::Active)
    }

    /// Whether an agent in this state is listed under this tab.
    ///
    /// Active is everything that is still going or can go on: working, waiting for the person, or
    /// idle between prompts. Complete is everything that will not move without being asked again:
    /// done, failed, or its harness gone. Needs you is the part of Active that is waiting on the
    /// person. All is all of it.
    pub fn holds(self, state: State) -> bool {
        match self {
            Tab::Active => state.live(),
            Tab::NeedsYou => state == State::WaitingForYou,
            Tab::Complete => !state.live(),
            Tab::All => true,
        }
    }
}

/// Where a card came from, and so how far to trust it (design decision 2).
///
/// *Reported* is what a harness said about itself — a `harness.event`, or a line of its text.
/// *Verified* is what the shell itself did or saw: a command it ran through `agent_run`, the output
/// and the exit code it read off that command's PTY, an approval it drew. The details column counts
/// commands, files and approvals from verified cards only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    Reported,
    Verified,
}

impl Provenance {
    pub fn key(self) -> &'static str {
        match self {
            Provenance::Reported => "reported",
            Provenance::Verified => "verified",
        }
    }
}

/// How one call went.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallState {
    Running,
    Ok,
    Failed,
    /// Still open when its turn ended, or when its harness went: it never said how it went.
    Interrupted,
    /// Read out of the mind's text (a `⚙️` trail line). The trail says a call was made and nothing
    /// about how it went, so the card claims no outcome at all rather than guess one.
    Untold,
}

impl CallState {
    pub fn key(self) -> &'static str {
        match self {
            CallState::Running => "running",
            CallState::Ok => "ok",
            CallState::Failed => "failed",
            CallState::Interrupted => "interrupted",
            CallState::Untold => "untold",
        }
    }

    /// The word `ToolCallCard` shows on the call's line, in its vocabulary: `done` and `failed` are
    /// coloured, anything else is shown as it is, and empty says nothing.
    pub fn status(self) -> &'static str {
        match self {
            CallState::Running => "running",
            CallState::Ok => "done",
            CallState::Failed => "failed",
            CallState::Interrupted => "interrupted",
            CallState::Untold => "",
        }
    }
}

/// A card the lifecycle had to make on its own, and why.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mark {
    /// A `tool_end` for a call that never started. Kept as a card of its own, so the claim is
    /// visible and does not settle some other call.
    EndWithoutStart,
    /// Output for a call that never started.
    OutputWithoutStart,
}

impl Mark {
    pub fn label(self) -> &'static str {
        match self {
            Mark::EndWithoutStart => "ended without a start",
            Mark::OutputWithoutStart => "output without a start",
        }
    }
}

/// Who an agent is. What pieces 1 and 2 hand the store through `upsert_agent`.
#[derive(Clone, Debug, PartialEq)]
pub struct AgentMeta {
    pub id: AgentId,
    /// The mind's name as a person reads it: "pi", "Hermes".
    pub mind: String,
    /// The model, when known. A `usage` event fills it in later if it is empty here.
    pub model: String,
    /// What the agent is for: its first prompt, cut. Left empty, the first turn sets it.
    pub title: String,
    /// The agent that started this one, for a child (`shell.new_agent`).
    pub parent: Option<AgentId>,
    /// Unix seconds. Zero means "now".
    pub started: u64,
    /// Whether its harness holds more than one conversation. One that does not says so in the
    /// pane (design decision 1) rather than pretending.
    pub conversations: bool,
    /// The catalog role it was started as (`hand_off`), as the role stood then. `None` for an
    /// agent started on a mind alone.
    pub role: Option<RoleMeta>,
    /// The recipe that handed it the work (an Agent step, design/desk-and-mind-2026-09-23.md
    /// section 6), when a recipe did: what its row and its approval cards say it works for —
    /// "Council recipe → Reviewer". An agent a recipe started is a child: it starts no agents.
    pub recipe: Option<RecipeOrigin>,
}

/// The recipe run that started an agent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecipeOrigin {
    /// The run's id in the recipe store (`rcp_…`).
    pub id: String,
    /// Its name as the Recipes screen shows it (`Council`).
    pub name: String,
}

impl RecipeOrigin {
    /// "Council recipe".
    pub fn label(&self) -> String {
        format!("{} recipe", self.name)
    }
}

/// The catalog role an agent was started as, kept with the agent: what its row, its details and
/// `describe shell` say, and the budget it is held to — as the role was when it started, so a role
/// edited later does not rewrite what an older agent was given.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleMeta {
    /// The role's id in the catalog (`reviewer`).
    pub id: String,
    /// The role's name as a person reads it (`Reviewer`).
    pub name: String,
    /// Its reach in words: "editor, documents and notes · at most safe".
    pub reach: String,
    /// Its budget: turns, and minutes from its start.
    pub turns: u32,
    pub minutes: u32,
}

impl AgentMeta {
    pub fn new(id: AgentId, mind: impl Into<String>) -> Self {
        AgentMeta {
            id,
            mind: mind.into(),
            model: String::new(),
            title: String::new(),
            parent: None,
            started: 0,
            conversations: false,
            role: None,
            recipe: None,
        }
    }

    /// Who this agent works for, when it is not simply the person's: "Council recipe → Reviewer"
    /// for a recipe's agent, "Reviewer" for a role the person handed work to, and nothing for an
    /// agent started on a mind alone.
    pub fn on_behalf(&self) -> String {
        match (&self.recipe, &self.role) {
            (Some(recipe), Some(role)) => format!("{} → {}", recipe.label(), role.name),
            (Some(recipe), None) => recipe.label(),
            (None, Some(role)) => role.name.clone(),
            (None, None) => String::new(),
        }
    }
}

/// How long a title is: enough to recognise the task by, short enough for a row.
pub const TITLE_CHARS: usize = 60;

/// A prompt as a title: one line, cut at a word, with an ellipsis when cut.
pub fn title_of(prompt: &str) -> String {
    let flat: String = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= TITLE_CHARS {
        return flat;
    }
    let head: String = flat.chars().take(TITLE_CHARS).collect();
    let cut = match head.rfind(' ') {
        Some(at) if at > TITLE_CHARS / 2 => &head[..at],
        _ => head.as_str(),
    };
    format!("{}…", cut.trim_end())
}

// ── A buffer with a cap ─────────────────────────────────────────────

/// Bytes kept under a cap: the first `head_cap` of them, and the newest of the rest, so a call that
/// prints four megabytes keeps how it started and how it ended, and says how much fell between.
#[derive(Clone, Debug)]
pub struct Capped {
    head: Vec<u8>,
    tail: VecDeque<u8>,
    head_cap: usize,
    tail_cap: usize,
    dropped: u64,
    total: u64,
    lines: u64,
}

impl Capped {
    pub fn new(head_cap: usize, cap: usize) -> Self {
        Capped {
            head: Vec::new(),
            tail: VecDeque::new(),
            head_cap,
            tail_cap: cap.saturating_sub(head_cap),
            dropped: 0,
            total: 0,
            lines: 0,
        }
    }

    pub fn push(&mut self, bytes: &[u8]) {
        self.total += bytes.len() as u64;
        self.lines += bytes.iter().filter(|b| **b == b'\n').count() as u64;
        let room = self.head_cap.saturating_sub(self.head.len());
        let (into_head, rest) = bytes.split_at(room.min(bytes.len()));
        self.head.extend_from_slice(into_head);
        if rest.is_empty() {
            return;
        }
        self.tail.extend(rest);
        if self.tail.len() > self.tail_cap {
            let mut excess = self.tail.len() - self.tail_cap;
            // Never start the kept tail in the middle of a character.
            while excess < self.tail.len() && (self.tail[excess] & 0xC0) == 0x80 {
                excess += 1;
            }
            self.tail.drain(..excess);
            self.dropped += excess as u64;
        }
    }

    /// Everything ever pushed, in bytes, kept or not.
    pub fn total(&self) -> u64 {
        self.total
    }

    /// How much fell between the head and the tail.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// How many lines have gone through, kept or not.
    pub fn lines(&self) -> u64 {
        self.lines
    }

    pub fn is_empty(&self) -> bool {
        self.total == 0
    }

    /// The kept bytes, with the marker where the middle was dropped.
    pub fn text(&self) -> String {
        let mut out = String::from_utf8_lossy(&self.head).into_owned();
        if self.dropped > 0 {
            out.push_str(&format!("\n… {} not kept …\n", bytes(self.dropped)));
        }
        let (a, b) = self.tail.as_slices();
        let mut tail = Vec::with_capacity(a.len() + b.len());
        tail.extend_from_slice(a);
        tail.extend_from_slice(b);
        out.push_str(&String::from_utf8_lossy(&tail));
        out
    }

    /// At most the last `max` bytes of what is kept, cut on a character.
    pub fn last(&self, max: usize) -> String {
        // Straight from the tail when it holds enough: a running command is redrawn four times a
        // second, and copying two megabytes to show its last few lines each time is waste.
        if self.tail.len() >= max {
            let mut bytes: Vec<u8> = self.tail.range(self.tail.len() - max..).copied().collect();
            let skip = bytes.iter().take_while(|b| (**b & 0xC0) == 0x80).count();
            bytes.drain(..skip);
            return String::from_utf8_lossy(&bytes).into_owned();
        }
        let text = self.text();
        if text.len() <= max {
            return text;
        }
        let mut from = text.len() - max;
        while !text.is_char_boundary(from) {
            from += 1;
        }
        text[from..].to_string()
    }

    /// Put back what a saved session held: the newest of it, and the counts of what went through.
    /// What was not kept is marked at the front, which is where it was.
    pub fn restore(cap: usize, kept: &str, total: u64, lines: u64) -> Self {
        let mut buffer = Capped::new(0, cap);
        buffer.push(kept.as_bytes());
        let kept_len = buffer.total;
        buffer.dropped += total.saturating_sub(kept_len);
        buffer.total = total.max(kept_len);
        buffer.lines = lines.max(buffer.lines);
        buffer
    }
}

/// "2.1 MB", "340 KB", "812 bytes".
pub fn bytes(n: u64) -> String {
    if n >= 1024 * 1024 {
        format!("{:.1} MB", n as f64 / (1024.0 * 1024.0))
    } else if n >= 1024 {
        format!("{} KB", n / 1024)
    } else {
        format!("{n} bytes")
    }
}

// ── A call's output ────────────────────────────────────────────────

/// What a call printed has been, so far.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OutputKind {
    #[default]
    None,
    /// A tool's result, or a command's plain output: shown as text.
    Text,
    /// A command's terminal: bytes with escape sequences, drawn by an emulator from cells.
    Terminal,
}

/// One run of cells with the same look, as the card's canvas draws it.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub text: String,
    pub row: u16,
    pub col: u16,
    pub width: u16,
    pub fg: (u8, u8, u8),
    pub bg: (u8, u8, u8),
    pub bold: bool,
}

/// The card's background, and the colour of text nothing has coloured.
pub const TERMINAL_BG: (u8, u8, u8) = (16, 23, 30);
pub const TERMINAL_FG: (u8, u8, u8) = (222, 230, 239);

/// One call's output: the bytes, kept under [`CARD_CAP`], and — for a terminal — the emulator
/// that turns them into cells.
///
/// The emulator lives while the call runs. When it ends, the screen it last drew is kept as runs
/// and the emulator is dropped, so a session of a hundred commands holds a hundred small
/// snapshots, not a hundred emulators. Everything a command printed is still in the bytes, for
/// "all of it on a click".
///
/// Only cells are drawn: a command's OSC 52 clipboard write, its hyperlinks and its title changes
/// reach the emulator and go no further (design decision 4).
pub struct Output {
    pub kind: OutputKind,
    pub bytes: Capped,
    term: Option<Box<vt100::Parser>>,
    screen: Vec<Run>,
}

impl Default for Output {
    fn default() -> Self {
        Output { kind: OutputKind::None, bytes: Capped::new(CARD_HEAD, CARD_CAP), term: None, screen: Vec::new() }
    }
}

impl std::fmt::Debug for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Output")
            .field("kind", &self.kind)
            .field("total", &self.bytes.total())
            .field("live", &self.term.is_some())
            .finish()
    }
}

impl Output {
    /// Take more of the call's output.
    pub fn push(&mut self, stream: Stream, delta: &[u8]) {
        match (stream, self.kind) {
            (Stream::Terminal, OutputKind::None | OutputKind::Text) => {
                // A terminal from here on. Anything printed as text before it is replayed into
                // the emulator, so the screen is the whole of what the call said.
                self.kind = OutputKind::Terminal;
                let mut parser = Box::new(vt100::Parser::new(TERMINAL_ROWS, TERMINAL_COLS, 0));
                parser.process(&crlf(&self.bytes.text()));
                self.term = Some(parser);
            }
            (_, OutputKind::None) => self.kind = OutputKind::Text,
            _ => {}
        }
        self.bytes.push(delta);
        if self.kind == OutputKind::Terminal {
            let parser = self.term.get_or_insert_with(|| Box::new(vt100::Parser::new(TERMINAL_ROWS, TERMINAL_COLS, 0)));
            if stream == Stream::Terminal {
                parser.process(delta);
            } else {
                // Plain text has no carriage returns of its own; a PTY would have added them.
                parser.process(&crlf(&String::from_utf8_lossy(delta)));
            }
        }
    }

    /// The call ended: keep the last screen, drop the emulator.
    pub fn settle(&mut self) {
        if let Some(parser) = self.term.take() {
            self.screen = runs_of(parser.screen());
        }
    }

    /// Whether the emulator is still attached.
    pub fn live(&self) -> bool {
        self.term.is_some()
    }

    /// The terminal as cells: live while the call runs, the last screen after. Blank rows below
    /// the last one with anything on it are left out.
    pub fn runs(&self) -> Vec<Run> {
        match &self.term {
            Some(parser) => runs_of(parser.screen()),
            None => self.screen.clone(),
        }
    }

    /// How many rows of the terminal have anything on them.
    pub fn rows(&self) -> u16 {
        self.runs().iter().map(|r| r.row + 1).max().unwrap_or(0)
    }

    /// The last `n` lines, as text. A terminal's are read off its bytes with the escapes taken out.
    pub fn tail_lines(&self, n: usize) -> String {
        let text = self.plain(64 * 1024);
        let lines: Vec<&str> = text.trim_end_matches('\n').lines().collect();
        let from = lines.len().saturating_sub(n);
        lines[from..].join("\n")
    }

    /// Up to `max` bytes of the output as plain text, the newest kept.
    pub fn plain(&self, max: usize) -> String {
        let raw = self.bytes.last(max);
        match self.kind {
            OutputKind::Terminal => strip_escapes(&raw),
            _ => raw,
        }
    }

    /// All of it that is kept, as plain text.
    pub fn all(&self) -> String {
        match self.kind {
            OutputKind::Terminal => strip_escapes(&self.bytes.text()),
            _ => self.bytes.text(),
        }
    }

    pub fn lines(&self) -> u64 {
        self.bytes.lines()
    }

    /// Put back a saved call's output. A terminal's last screen is redrawn from the bytes kept.
    pub fn restore(kind: OutputKind, kept: &str, total: u64, lines: u64) -> Output {
        let mut output = Output { kind, bytes: Capped::restore(CARD_CAP, kept, total, lines), term: None, screen: Vec::new() };
        if kind == OutputKind::Terminal {
            let mut parser = vt100::Parser::new(TERMINAL_ROWS, TERMINAL_COLS, 0);
            parser.process(kept.as_bytes());
            output.screen = runs_of(parser.screen());
        }
        output
    }

    /// Keep only the newest `keep` bytes — for an old card, once newer ones hold the attention.
    pub fn trim_to(&mut self, keep: usize) {
        if self.bytes.total() as usize <= keep && self.bytes.dropped() == 0 {
            return;
        }
        let kept = self.bytes.last(keep);
        let (total, lines) = (self.bytes.total(), self.bytes.lines());
        self.bytes = Capped::restore(keep.max(1), &kept, total, lines);
    }
}

fn crlf(text: &str) -> Vec<u8> {
    text.replace("\r\n", "\n").replace('\n', "\r\n").into_bytes()
}

/// The screen as runs of same-looking cells, blank default-coloured stretches left out.
fn runs_of(screen: &vt100::Screen) -> Vec<Run> {
    let (rows, cols) = screen.size();
    let mut runs: Vec<Run> = Vec::new();
    for row in 0..rows {
        for col in 0..cols {
            let Some(cell) = screen.cell(row, col) else { continue };
            if cell.is_wide_continuation() {
                continue;
            }
            let mut fg = color(cell.fgcolor(), TERMINAL_FG);
            let mut bg = color(cell.bgcolor(), TERMINAL_BG);
            if cell.inverse() {
                std::mem::swap(&mut fg, &mut bg);
            }
            if cell.dim() {
                fg = (fg.0 / 2, fg.1 / 2, fg.2 / 2);
            }
            let text = if cell.contents().is_empty() { " " } else { cell.contents() };
            let width = if cell.is_wide() { 2 } else { 1 };
            if let Some(last) = runs.last_mut().filter(|r| {
                r.row == row && r.col + r.width == col && r.fg == fg && r.bg == bg && r.bold == cell.bold()
            }) {
                last.text.push_str(text);
                last.width += width;
            } else {
                runs.push(Run { text: text.to_string(), row, col, width, fg, bg, bold: cell.bold() });
            }
        }
    }
    // Nothing to draw in a run of spaces on the canvas's own colour.
    runs.retain(|r| !(r.bg == TERMINAL_BG && r.text.trim().is_empty()));
    for run in &mut runs {
        if run.bg == TERMINAL_BG {
            let trimmed = run.text.trim_end().to_string();
            let cut = run.text.chars().count() - trimmed.chars().count();
            run.width = run.width.saturating_sub(cut as u16);
            run.text = trimmed;
        }
    }
    runs
}

/// A terminal colour as RGB — the palette the Terminal app draws with, so a command looks the same
/// in a card as it does there (apps/terminal/src/session.rs).
fn color(value: vt100::Color, default: (u8, u8, u8)) -> (u8, u8, u8) {
    const ANSI: [(u8, u8, u8); 16] = [
        (24, 31, 40),
        (240, 115, 125),
        (130, 207, 156),
        (238, 203, 129),
        (126, 176, 244),
        (195, 158, 237),
        (110, 211, 207),
        (222, 230, 239),
        (116, 131, 150),
        (255, 151, 157),
        (163, 229, 182),
        (250, 223, 164),
        (163, 199, 255),
        (219, 185, 255),
        (156, 234, 230),
        (247, 249, 252),
    ];
    match value {
        vt100::Color::Default => default,
        vt100::Color::Rgb(r, g, b) => (r, g, b),
        vt100::Color::Idx(i @ 0..=15) => ANSI[i as usize],
        vt100::Color::Idx(i @ 16..=231) => {
            let n = i - 16;
            let c = |v| if v == 0 { 0 } else { 55 + 40 * v };
            (c(n / 36), c((n / 6) % 6), c(n % 6))
        }
        vt100::Color::Idx(i) => {
            let v = 8 + 10 * (i - 232);
            (v, v, v)
        }
    }
}

/// Terminal bytes as text: escape sequences out, carriage returns folded.
///
/// Good enough for reading what a command said, which is all it is for. The cells are what is
/// drawn; this is the "all of it" view and the text a caller of `describe` reads.
pub fn strip_escapes(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => match chars.next() {
                // CSI: parameters, then one final byte in @..~.
                Some('[') => {
                    while let Some(&n) = chars.peek() {
                        chars.next();
                        if ('@'..='~').contains(&n) {
                            break;
                        }
                    }
                }
                // OSC, DCS, APC, PM, SOS: up to BEL or ST.
                Some(']' | 'P' | '_' | '^' | 'X') => {
                    while let Some(n) = chars.next() {
                        if n == '\u{7}' {
                            break;
                        }
                        if n == '\u{1b}' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                // Charset selection takes one more character.
                Some('(' | ')' | '*' | '+') => {
                    chars.next();
                }
                _ => {}
            },
            '\r' => {
                if chars.peek() != Some(&'\n') {
                    // A lone CR rewrites the line; keep what came after it.
                    if let Some(at) = out.rfind('\n') {
                        out.truncate(at + 1);
                    } else {
                        out.clear();
                    }
                }
            }
            '\u{7}' | '\u{8}' => {}
            c => out.push(c),
        }
    }
    out
}

// ── The session ────────────────────────────────────────────────────

/// One tool call, as a card.
#[derive(Debug)]
pub struct Card {
    /// The harness's id for the call (or the shell's job id, for a verified command).
    pub call: String,
    pub name: String,
    pub target: String,
    pub args: serde_json::Value,
    /// Hermes' primary-argument preview, for a call read off a trail line.
    pub preview: String,
    /// How many times in a row, when the trail collapsed repeats.
    pub repeats: u32,
    pub state: CallState,
    /// One line on how it went, from the end.
    pub summary: String,
    pub exit_code: Option<i32>,
    pub provenance: Provenance,
    pub mark: Option<Mark>,
    pub output: Output,
    pub started: u64,
    pub ended: Option<u64>,
}

impl Card {
    pub fn new(call: &str, name: &str, target: &str, args: serde_json::Value, provenance: Provenance, at: u64) -> Card {
        Card {
            call: call.to_string(),
            name: name.to_string(),
            target: target.to_string(),
            args,
            preview: String::new(),
            repeats: 0,
            state: CallState::Running,
            summary: String::new(),
            exit_code: None,
            provenance,
            mark: None,
            output: Output::default(),
            started: at,
            ended: None,
        }
    }

    /// The call as the trail reader describes one, for its one-line summary and its arguments.
    pub fn as_call(&self) -> crate::trail::ToolCall {
        crate::trail::ToolCall {
            name: self.name.clone(),
            target: self.target.clone(),
            arguments: self.args.clone(),
            preview: self.preview.clone(),
            repeats: self.repeats,
        }
    }

    pub fn running(&self) -> bool {
        self.state == CallState::Running
    }

    /// Whether this is a command — something with a process, a PTY and an exit code.
    pub fn is_command(&self) -> bool {
        const COMMANDS: &[&str] = &["agent_run", "terminal.run", "bash", "run", "shell"];
        self.exit_code.is_some()
            || COMMANDS.contains(&self.name.as_str())
            || COMMANDS.contains(&self.target.as_str())
    }

    /// The command line, for a command.
    pub fn command_line(&self) -> String {
        for key in ["command", "cmd"] {
            if let Some(text) = self.args.get(key).and_then(|v| v.as_str()) {
                return text.to_string();
            }
            if let Some(text) = self.args.get("args").and_then(|a| a.get(key)).and_then(|v| v.as_str()) {
                return text.to_string();
            }
        }
        if !self.preview.is_empty() {
            return self.preview.clone();
        }
        self.as_call().summary()
    }

    /// Paths the call's arguments name. Only asked of verified cards: a harness's claim about
    /// what it touched is not evidence that it touched it.
    pub fn paths(&self) -> Vec<String> {
        const KEYS: &[&str] = &[
            "path", "paths", "file", "files", "src", "dst", "source", "destination", "from", "to",
            "dir", "directory", "folder",
        ];
        let mut found = Vec::new();
        fn walk(value: &serde_json::Value, keyed: bool, found: &mut Vec<String>) {
            match value {
                serde_json::Value::String(s) if keyed => {
                    let s = s.trim();
                    if s.starts_with('/') || s.starts_with('~') || s.starts_with("./") {
                        found.push(s.to_string());
                    }
                }
                serde_json::Value::Array(items) => {
                    for item in items {
                        walk(item, keyed, found);
                    }
                }
                serde_json::Value::Object(map) => {
                    for (key, value) in map {
                        walk(value, KEYS.contains(&key.as_str()), found);
                    }
                }
                _ => {}
            }
        }
        walk(&self.args, false, &mut found);
        found
    }
}

/// How an approval the shell asked for this agent came out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalOutcome {
    /// On the person's screen now: in this pane and in the Lens, one request id.
    Pending,
    Allowed,
    Denied,
    /// Nobody answered before the request ran out.
    Expired,
    /// Taken back by the shell: the agent was stopped, its harness went, or the shell restarted
    /// while it was waiting. Refused, never granted.
    Withdrawn,
}

impl ApprovalOutcome {
    pub fn key(self) -> &'static str {
        match self {
            ApprovalOutcome::Pending => "pending",
            ApprovalOutcome::Allowed => "allowed",
            ApprovalOutcome::Denied => "denied",
            ApprovalOutcome::Expired => "expired",
            ApprovalOutcome::Withdrawn => "withdrawn",
        }
    }
}

/// An approval the shell drew for this agent (design decision 4). Only the shell makes one — from
/// the agent its token names — so it is verified by construction, and what it draws is the shell's
/// own card, never anything the agent said: `request` is the shell's request id, and the card's
/// words and buttons come from the shell's approval store under that id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Approval {
    /// The shell's request id (`appr-7`). Allow and Deny in the pane answer exactly this one.
    pub request: String,
    /// What was asked, as `app.action`.
    pub what: String,
    pub outcome: ApprovalOutcome,
    /// The line it leaves once settled — the approval store's own record when it still had one
    /// ("Allowed once: shell.agent_run — 21:04").
    pub record: String,
    pub asked: u64,
    pub settled: Option<u64>,
}

/// One thing in a turn, in the order it happened.
#[derive(Debug)]
pub enum Item {
    Text(Capped),
    /// The mind's reasoning, when its harness shares it. Shown folded.
    Thinking(Capped),
    Card(Card),
    /// Something the shell says about the session: a stop asked for, a turn cut short.
    Note(String),
    /// An approval the shell asked the person for, on this agent's behalf.
    Approval(Approval),
}

/// One prompt and everything that came of it.
#[derive(Debug)]
pub struct Turn {
    /// Counts up for the life of the agent, so a turn keeps its name after older ones are let go.
    pub n: u64,
    pub prompt: String,
    pub started: u64,
    pub ended: Option<u64>,
    pub ok: Option<bool>,
    pub items: Vec<Item>,
    /// Whether any structured `harness.event` arrived in this turn. Once one has, trail lines in
    /// the text are the same calls told twice and are not made into cards.
    pub events: bool,
    /// Counts trail cards in this turn, for their ids.
    pub trail_seq: u32,
}

impl Turn {
    pub fn open(&self) -> bool {
        self.ended.is_none()
    }

    pub fn cards(&self) -> impl Iterator<Item = &Card> {
        self.items.iter().filter_map(|i| match i {
            Item::Card(c) => Some(c),
            _ => None,
        })
    }

    pub fn cards_mut(&mut self) -> impl Iterator<Item = &mut Card> {
        self.items.iter_mut().filter_map(|i| match i {
            Item::Card(c) => Some(c),
            _ => None,
        })
    }

    /// The mind's final message in this turn: what it said after its last call — its answer, not
    /// the narration it wrote on the way ("I'll start by seeing what's on this desktop…"). When it
    /// said nothing after its last call, the last thing it said before it. Its thinking, the
    /// shell's notes and approvals are not what it said. (#194: a recipe kept the whole turn, so
    /// the Chair read each seat's narration as its answer.)
    pub fn final_text(&self) -> String {
        let mut runs: Vec<String> = vec![String::new()];
        for item in &self.items {
            match item {
                Item::Text(t) => runs.last_mut().expect("never empty").push_str(&t.text()),
                Item::Card(_) => runs.push(String::new()),
                Item::Thinking(_) | Item::Note(_) | Item::Approval(_) => {}
            }
        }
        runs.iter().rev().map(|r| r.trim()).find(|r| !r.is_empty()).unwrap_or_default().to_string()
    }
}

/// What a turn cost, when the harness says.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Usage {
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    /// Whether any usage was reported at all — "0 tokens" and "not reported" are different.
    pub reported: bool,
}

/// One agent: one conversation with one mind.
#[derive(Debug)]
pub struct Agent {
    pub meta: AgentMeta,
    pub state: State,
    /// When the state last changed, for "running · 2m".
    pub since: u64,
    /// The agent's own account of what it is doing (a reported `status` event).
    pub status: String,
    pub turns: Vec<Turn>,
    pub usage: Usage,
    /// Events the lifecycle refused: for a turn that had ended, output after a call's end, a
    /// second start or a second end. Counted, never silently lost.
    pub refused: u32,
    pub approvals_asked: u32,
    pub approvals_answered: u32,
    /// Approval requests asked and not yet answered, by request id.
    pub pending_approvals: Vec<String>,
    /// Creation order, to break ties between agents started in the same second.
    pub seq: u64,
    /// Last time anything happened to it.
    pub touched: u64,
    pub next_turn: u64,
}

impl Agent {
    pub fn open_turn(&self) -> Option<&Turn> {
        self.turns.last().filter(|t| t.open())
    }

    pub fn cards(&self) -> impl Iterator<Item = &Card> {
        self.turns.iter().flat_map(|t| t.cards())
    }

    /// Whether anything of it is still running: its turn, or a command the shell owns.
    pub fn busy(&self) -> bool {
        self.state.working() || self.open_turn().is_some() || self.cards().any(|c| c.running())
    }
}

/// The details column, counted.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Details {
    pub turns: usize,
    /// Every call, reported and verified.
    pub calls: usize,
    pub failed_calls: usize,
    /// Verified commands, newest last: the command line, its exit code, how it went.
    pub commands: Vec<(String, Option<i32>, CallState)>,
    /// Paths named by verified calls, each once.
    pub files: Vec<String>,
    pub approvals_asked: u32,
    pub approvals_answered: u32,
    pub usage: Usage,
    pub refused: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_prompt_becomes_a_title_cut_at_a_word() {
        assert_eq!(title_of("tidy   the photos\nfolder"), "tidy the photos folder");
        let long = "find every duplicate photo in the pictures folder and move the copies into the trash please";
        let title = title_of(long);
        assert!(title.ends_with('…'), "{title}");
        assert!(title.chars().count() <= TITLE_CHARS + 1, "{title}");
        assert!(!title.contains("  "));
    }

    #[test]
    fn a_capped_buffer_keeps_its_head_and_its_tail_and_says_what_it_dropped() {
        let mut buffer = Capped::new(4, 10);
        buffer.push(b"abcd");
        buffer.push(b"efghijklmnop");
        assert_eq!(buffer.total(), 16);
        assert_eq!(buffer.dropped(), 6);
        let text = buffer.text();
        assert!(text.starts_with("abcd\n… 6 bytes not kept …\n"), "{text}");
        assert!(text.ends_with("klmnop"), "{text}");
        assert_eq!(buffer.last(3), "nop", "the newest bytes, read off the tail");
        assert_eq!(buffer.last(1000), text, "or everything kept, when that is less");
    }

    #[test]
    fn the_kept_tail_never_starts_inside_a_character() {
        let mut buffer = Capped::new(0, 5);
        buffer.push("aé€é".as_bytes()); // 1 + 2 + 3 + 2 bytes
        assert!(!buffer.text().contains('\u{fffd}'), "{}", buffer.text());
    }

    #[test]
    fn escapes_come_out_and_a_carriage_return_rewrites_its_line() {
        assert_eq!(strip_escapes("\u{1b}[1;31merror\u{1b}[0m: no\r\n"), "error: no\n");
        assert_eq!(strip_escapes("10%\r50%\r100%\ndone"), "100%\ndone");
        assert_eq!(strip_escapes("\u{1b}]0;title\u{7}ok"), "ok");
        assert_eq!(strip_escapes("\u{1b}]52;c;aGVsbG8=\u{1b}\\x"), "x", "an OSC 52 clipboard write is not text");
    }

    #[test]
    fn a_terminals_bytes_are_drawn_as_cells_and_its_screen_outlives_the_emulator() {
        let mut output = Output::default();
        output.push(Stream::Terminal, b"\x1b[32mgreen\x1b[0m plain\r\nsecond line\r\n");
        assert_eq!(output.kind, OutputKind::Terminal);
        let runs = output.runs();
        let green = runs.iter().find(|r| r.text == "green").expect("the coloured word is its own run");
        assert_eq!(green.fg, (130, 207, 156));
        assert_eq!(output.rows(), 2);
        output.settle();
        assert!(!output.live());
        assert_eq!(output.runs(), runs, "the last screen is kept after the emulator goes");
        assert_eq!(output.all(), "green plain\nsecond line\n");
    }

    #[test]
    fn a_title_change_or_clipboard_write_from_a_command_draws_nothing() {
        let mut output = Output::default();
        output.push(Stream::Terminal, b"\x1b]0;pwned\x07\x1b]52;c;aGk=\x07ok\r\n");
        let text: String = output.runs().iter().map(|r| r.text.clone()).collect();
        assert_eq!(text, "ok");
    }
}
