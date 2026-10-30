//! The Mind desk's wire: what the shell tells the bars around Mind View, and nothing more
//! (#1023, phase 1).
//!
//! The desk (`apps/mind-desk`) is a window of Mind View, so a Mind that is acting in there can
//! read it and press its buttons. Everything on this page is therefore display-safe by
//! construction: a name, a state, a short quoted line the Mind chose to say, one tool's *name*, a
//! few counts, a model label and a kind of account. There is no field for an argument, a path, a
//! prompt, a command line, a token or an output, so none can be put on the desk by a mistake in
//! either end. The shell cleans every string with the functions below before it sends it, and the
//! desk cleans again before it draws, so one end's bug is not the other's leak.
//!
//! The call is `app.act { action: "mind_desk", args: { since?, wait_seconds? } }` on the shell's
//! control surface, answered with a [`DeskSnapshot`]. It reads and changes nothing, and it is
//! answered to the person's processes only: a Mind's own call is refused.

use serde::{Deserialize, Serialize};

/// The shell's action that answers with a [`DeskSnapshot`].
pub const ACTION: &str = "mind_desk";

/// The longest one call waits for the revision to move, in seconds. A wait is one blocked socket
/// call and no timer: the desk asks again when it returns.
pub const LONGEST_WAIT_SECS: u64 = 25;

/// How many characters of the Mind's own line the desk shows.
pub const NARRATION_MAX_CHARS: usize = 140;
/// The longest tool name shown.
pub const TOOL_NAME_MAX_CHARS: usize = 48;
/// The longest Mind or app name shown.
pub const NAME_MAX_CHARS: usize = 40;
/// The longest model label shown.
pub const MODEL_MAX_CHARS: usize = 64;
/// The most apps the footer names.
pub const APPS_MAX: usize = 8;

/// What a Mind is doing, as the Agents list says it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MindState {
    Thinking,
    RunningTool,
    WaitingForYou,
    Idle,
    Done,
    Failed,
    HarnessGone,
}

impl MindState {
    /// Doing something now, so there is something to stop.
    pub fn working(self) -> bool {
        matches!(self, MindState::Thinking | MindState::RunningTool | MindState::WaitingForYou)
    }
}

/// Who vouches for a fact: the shell, which saw it happen, or the Mind's own harness.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Evidence {
    Verified,
    Reported,
}

/// How one call went.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Running,
    Ok,
    Failed,
    Interrupted,
    Untold,
}

/// Which account a Mind's harness runs under, as the kernel named it when the harness attached.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Account {
    /// Its own (`yantrik-mind`): it cannot reach the person's files or sockets.
    Own,
    /// The person's: whatever the person can do, the harness can.
    Person,
    /// The shell does not know.
    Unknown,
}

/// The tool a Mind is running, or ran last. Its name and nothing it was called with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeskTool {
    pub name: String,
    pub evidence: Evidence,
    pub outcome: Outcome,
}

/// The Mind's own line, quoted on the desk and marked Reported.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Narration {
    pub text: String,
    /// When the shell last heard from the Mind, local `HH:MM`.
    pub heard: String,
}

/// Calls in the Mind's current run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    pub calls: u32,
    pub failed: u32,
    pub interrupted: u32,
}

/// The Mind the desk is showing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeskMind {
    /// The Agents id, for Stop. Never drawn, never spoken.
    pub agent: String,
    pub name: String,
    pub state: MindState,
    /// Unix seconds the state last changed.
    pub since: u64,
    pub narration: Option<Narration>,
    pub tool: Option<DeskTool>,
    pub counts: Counts,
    /// Seconds the current run has lasted, as of the snapshot. A snapshot, not a clock.
    pub elapsed_secs: Option<u64>,
    /// The model, as the harness said it. `None` when it said nothing.
    pub model: Option<String>,
    pub account: Account,
}

/// What the desk draws from.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeskSnapshot {
    /// Stands for what the desk would draw, bar the age of the run: two snapshots with the same
    /// tag draw the same words. The desk hands the last one it drew back, and a call returns as
    /// soon as the shell has something else to say (or at the end of its wait).
    pub tag: u64,
    /// The local `HH:MM` the snapshot was made, which the run's age is "as of".
    pub as_of: String,
    /// `None` when no Mind is known.
    pub mind: Option<DeskMind>,
    /// The apps open in Mind View, by the names a person reads.
    pub apps: Vec<String>,
}

impl DeskSnapshot {
    /// This snapshot with its [`tag`](DeskSnapshot::tag) set from what it says.
    pub fn tagged(mut self) -> DeskSnapshot {
        self.tag = self.tag_of();
        self
    }

    /// A number for what the desk would draw: everything but the tag, the time it was made and
    /// the age of the run, which change with the clock and not with anything the Mind did. Whole
    /// in 48 bits, so it crosses a JSON number as it is.
    pub fn tag_of(&self) -> u64 {
        use std::hash::{Hash as _, Hasher as _};
        let mut like = self.clone();
        like.tag = 0;
        like.as_of.clear();
        if let Some(mind) = like.mind.as_mut() {
            mind.elapsed_secs = None;
        }
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        serde_json::to_string(&like).unwrap_or_default().hash(&mut hasher);
        hasher.finish() & 0xFFFF_FFFF_FFFF
    }
}

/// What a stop answered, as far as the desk reads it: the shell named the agent it stopped.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct StopAck {
    pub agent: String,
}

// ── Cleaning ──────────────────────────────────────────────────────────

/// `text` as one plain line: control characters gone, runs of space made one, cut to `max`
/// characters with an ellipsis when it was longer.
pub fn one_line(text: &str, max: usize) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    let cut: String = flat.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

/// Whether one word of a line looks like something that is not for the chrome: a path, a flag, an
/// address with a query, or a secret.
fn word_is_private(word: &str) -> bool {
    let w = word.trim_matches(|c: char| matches!(c, '"' | '\'' | '`' | '(' | ')' | '[' | ']' | ',' | ';' | ':' | '.' | '!' | '?'));
    if w.is_empty() {
        return false;
    }
    // A path: absolute, home-relative, relative with a parent, or a Windows drive.
    let path = w.starts_with('/') && w.len() > 1 && w[1..].contains(|c: char| c.is_alphanumeric())
        || w.starts_with("~/")
        || w.starts_with("./")
        || w.starts_with("../")
        || w.contains("/home/")
        || w.contains("/Users/")
        || (w.len() > 2 && w.as_bytes()[1] == b':' && matches!(w.as_bytes()[2], b'\\' | b'/'));
    // An argument: a flag, or `key=value`.
    let argument = w.starts_with("--") || (w.contains('=') && !w.starts_with('='));
    // An address that carries a query or credentials.
    let address = w.contains("://") && (w.contains('?') || w.contains('@'));
    // A secret: a known prefix, or a long unbroken run of token characters.
    let prefixed = ["sk-", "ghp_", "gho_", "xoxb-", "xoxp-", "AKIA", "eyJ", "Bearer"].iter().any(|p| w.starts_with(p));
    let long = w.chars().count() >= 32 && w.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '+' | '/' | '=' | '.'));
    path || argument || address || prefixed || long
}

/// Whether a line carries anything the chrome must not show.
pub fn looks_private(text: &str) -> bool {
    text.split_whitespace().any(word_is_private)
}

/// The Mind's own line, cleaned for the desk, or `None` when there is none or it carries
/// something that is not for the chrome. The desk then says the state in its own words.
pub fn narration_text(raw: &str) -> Option<String> {
    let line = one_line(raw, NARRATION_MAX_CHARS);
    (!line.is_empty() && !looks_private(&line)).then_some(line)
}

/// A tool's name, or `None` when what was given is not a plain name (a command line put where a
/// name goes).
pub fn tool_name(raw: &str) -> Option<String> {
    let name = raw.trim();
    let plain = !name.is_empty()
        && name.chars().count() <= TOOL_NAME_MAX_CHARS
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '-'));
    plain.then(|| name.to_string())
}

/// A Mind's or an app's name as one short line.
pub fn display_name(raw: &str) -> String {
    let line = one_line(raw, NAME_MAX_CHARS);
    if looks_private(&line) {
        String::new()
    } else {
        line
    }
}

/// The model label the harness gave, or `None` when it gave none, or something that is not one.
pub fn model_label(raw: &str) -> Option<String> {
    let line = one_line(raw, MODEL_MAX_CHARS);
    let plain = !line.is_empty()
        && !looks_private(&line)
        && line.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '_' | '-' | ':' | '/' | '@' | '+'));
    plain.then_some(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_is_one_short_plain_line() {
        assert_eq!(one_line("  Shaping\tthe\n last   ascent. ", 140), "Shaping the last ascent.");
        assert_eq!(one_line("a\u{1b}[31mb", 140), "a [31mb");
        let long = "word ".repeat(60);
        let cut = one_line(&long, 20);
        assert!(cut.chars().count() <= 20 && cut.ends_with('…'), "{cut}");
    }

    #[test]
    fn the_minds_own_words_pass_and_its_paths_flags_and_secrets_do_not() {
        for fine in [
            "Shaping the last ascent.",
            "Ready to export. May I replace the file?",
            "Reading the docs, then trying again (twice).",
            "Waiting while you drive",
        ] {
            assert_eq!(narration_text(fine).as_deref(), Some(fine), "{fine}");
        }
        for private in [
            "Writing /home/pranab/Documents/Ridge/notes.md now",
            "Opening ~/Documents/ridge",
            "Running ./build.sh",
            "Reading ../secrets/env",
            "Saving C:\\Users\\me\\a.txt",
            "Running ls --all --color",
            "Setting token=abc123",
            "Fetching https://example.com/x?key=1",
            "Using sk-live-9f8a7b6c5d4e3f2a1b0c",
            "Authorization eyJhbGciOiJIUzI1NiJ9",
            "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghij",
        ] {
            assert_eq!(narration_text(private), None, "{private}");
        }
        assert_eq!(narration_text("   \n "), None, "nothing said is nothing shown");
    }

    #[test]
    fn a_tool_is_a_name_and_never_a_command() {
        assert_eq!(tool_name("blender.add_object").as_deref(), Some("blender.add_object"));
        assert_eq!(tool_name(" agent_run ").as_deref(), Some("agent_run"));
        assert_eq!(tool_name("mcp__files__write").as_deref(), Some("mcp__files__write"));
        for not in ["", "ls -la /etc", "cat ~/.ssh/id_rsa", "a b", "x".repeat(49).as_str(), "echo $TOKEN"] {
            assert_eq!(tool_name(not), None, "{not}");
        }
    }

    #[test]
    fn a_model_label_is_a_label() {
        assert_eq!(model_label("deepseek-v4.1-flash via ollama-cloud").as_deref(), Some("deepseek-v4.1-flash via ollama-cloud"));
        assert_eq!(model_label("").as_deref(), None);
        assert_eq!(model_label("key=abc").as_deref(), None);
        assert_eq!(model_label("/home/me/model.gguf").as_deref(), None);
    }

    #[test]
    fn the_tag_moves_with_what_is_drawn_and_not_with_the_clock() {
        let mut snap = DeskSnapshot {
            mind: Some(DeskMind {
                agent: "hermes:main".into(),
                name: "Hermes".into(),
                state: MindState::RunningTool,
                since: 5,
                narration: None,
                tool: None,
                counts: Counts::default(),
                elapsed_secs: Some(10),
                model: None,
                account: Account::Person,
            }),
            ..Default::default()
        }
        .tagged();
        let first = snap.tag;
        assert!(first <= 0xFFFF_FFFF_FFFF);
        assert_eq!(snap.tag_of(), first);
        // Time passing is not a change.
        snap.as_of = "14:33".into();
        snap.mind.as_mut().unwrap().elapsed_secs = Some(400);
        assert_eq!(snap.tag_of(), first);
        // A tool starting is.
        snap.mind.as_mut().unwrap().tool = Some(DeskTool { name: "t".into(), evidence: Evidence::Verified, outcome: Outcome::Running });
        assert_ne!(snap.tag_of(), first);
    }

    #[test]
    fn the_snapshot_has_no_field_for_what_the_chrome_must_not_show() {
        // The wire is the whole of what the desk can learn. A field named for an argument, a
        // path, a prompt, a command or a token would be a way to put one on the desk.
        let wire = serde_json::to_string(&DeskSnapshot::default()).unwrap()
            + &serde_json::to_string(&DeskMind {
                agent: "a".into(),
                name: "n".into(),
                state: MindState::Idle,
                since: 0,
                narration: Some(Narration { text: "t".into(), heard: "00:00".into() }),
                tool: Some(DeskTool { name: "t".into(), evidence: Evidence::Reported, outcome: Outcome::Ok }),
                counts: Counts::default(),
                elapsed_secs: None,
                model: None,
                account: Account::Unknown,
            })
            .unwrap();
        for forbidden in ["args", "arguments", "path", "prompt", "command", "token", "output", "target", "title", "cwd"] {
            assert!(!wire.contains(forbidden), "`{forbidden}` is on the wire: {wire}");
        }
    }
}
