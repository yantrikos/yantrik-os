//! The Mind desk's wire: what the shell tells the bars around Mind View, and nothing more
//! (#1023, phase 1).
//!
//! The desk (`apps/mind-desk`) is a window of Mind View, so a Mind that is acting in there can
//! read it and press its buttons. Everything on this page is therefore display-safe by
//! construction: a name, a state, one tool's *name*, a
//! few counts, a model label and a kind of account. There is no field for an argument, a path, a
//! prompt, a command line, a token or an output, so none can be put on the desk by a mistake in
//! either end. The shell cleans every string with the functions below before it sends it, and the
//! desk cleans again before it draws, so one end's bug is not the other's leak.
//!
//! There is deliberately **no status line** here. What a Mind says it is doing is the person's
//! (`agents::PERSONS_FIELDS` withholds it from every agent), and the desk is a window in a room
//! every Mind can read, by screenshot, by OCR (`read_mind_view`) and by `describe`. Quoting it on
//! the desk would hand one Mind another's words. It returns with per-Mind attribution (phase 4).
//!
//! The calls are `app.act { action: "mind_desk", args: { since?, wait_seconds? } }`, answered with
//! a [`DeskSnapshot`]; `mind_desk_stop { agent }`, which asks the *person* on their own desktop; and
//! `mind_desk_open_agents`. All three are answered to the one desk process the shell itself
//! started, identified by its pid and start time, and to nothing else.

use serde::{Deserialize, Serialize};

/// The shell's action that answers with a [`DeskSnapshot`].
pub const SNAPSHOT_ACTION: &str = "mind_desk";

/// The shell's action that asks the *person*, on their own desktop, whether to stop a Mind.
pub const STOP_ACTION: &str = "mind_desk_stop";

/// The shell's action that puts the person's Agents screen up, at most once in a while.
pub const AGENTS_ACTION: &str = "mind_desk_open_agents";

/// The longest one call waits for the revision to move, in seconds. A wait is one blocked socket
/// call and no timer: the desk asks again when it returns.
pub const LONGEST_WAIT_SECS: u64 = 25;

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

/// Where a stop the desk asked for stands. The desk never stops anything: it asks the shell, which
/// asks the person on their own desktop, and only the person's yes stops the Mind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopStatus {
    /// The person has been asked, on their desktop, and has not answered.
    Asked,
    /// The person said yes and the shell stopped the Mind: the shell's own word.
    Stopped,
    /// The person said no, did not answer, or the shell could not stop it.
    NotStopped,
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
    /// A stop asked for this Mind, and how it stands.
    pub stop: Option<StopStatus>,
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

/// What the shell answered to a stop the desk asked for: it named the agent it has asked the
/// person about. That is **not** a stop: nothing stops until the person says yes on their desktop,
/// and the desk learns it from [`StopStatus::Stopped`] in a later snapshot.
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

/// Whether one word looks like something that is not for the chrome: a path (any slash inside a
/// word, in any width), a flag, `key=value`, an address (an `@`, or a scheme), or a secret.
///
/// It reads more than it should: `and/or` is private. The words it is asked about are names and
/// labels, which have no use for a slash, and a wrong "private" costs a label, where a wrong
/// "fine" costs a path.
fn word_is_private(word: &str) -> bool {
    let w = word.trim_matches(|c: char| matches!(c, '"' | '\'' | '`' | '(' | ')' | '[' | ']' | ',' | ';' | ':' | '.' | '!' | '?'));
    if w.is_empty() || w == "/" {
        return false;
    }
    // A path or an address: a slash or an at-sign anywhere in the word, whatever its width, or a
    // home-relative or drive-letter start.
    let separator = w.contains(['/', '\\', '@', '／', '＼', '＠', '∕']);
    let drive = w.len() > 2 && w.as_bytes()[1] == b':' && matches!(w.as_bytes()[2], b'\\' | b'/');
    let home = w.starts_with('~') && w.len() > 1;
    // An argument: a flag, or `key=value`.
    let argument = w.starts_with("--") || (w.contains('=') && !w.starts_with('='));
    let address = w.contains("://");
    // A secret: a known prefix, or a long unbroken run of token characters, or a long run of
    // letters and digits with no separator at all.
    let prefixed = ["sk-", "sk_", "ghp_", "gho_", "ghs_", "github_pat_", "xox", "AKIA", "ASIA", "eyJ", "Bearer", "glpat-"]
        .iter()
        .any(|p| w.starts_with(p));
    let long = w.chars().count() >= 32 && w.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '+' | '=' | '.'));
    let packed = w.chars().count() >= 24
        && w.chars().all(|c| c.is_ascii_alphanumeric())
        && w.chars().any(|c| c.is_ascii_digit())
        && w.chars().any(|c| c.is_ascii_alphabetic());
    separator || drive || home || argument || address || prefixed || long || packed
}

/// Whether a line carries anything the chrome must not show.
pub fn looks_private(text: &str) -> bool {
    text.split_whitespace().any(word_is_private)
}

/// A tool's name, or `None` when what was given is not a plain name (a command line put where a
/// name goes).
pub fn tool_name(raw: &str) -> Option<String> {
    let name = raw.trim();
    let plain = !name.is_empty()
        && name.chars().count() <= TOOL_NAME_MAX_CHARS
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
        && !looks_private(name);
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

    fn mind() -> DeskMind {
        DeskMind {
            agent: "hermes:main".into(),
            name: "Hermes".into(),
            state: MindState::RunningTool,
            since: 5,
            stop: None,
            tool: None,
            counts: Counts::default(),
            elapsed_secs: Some(10),
            model: None,
            account: Account::Person,
        }
    }

    #[test]
    fn a_line_is_one_short_plain_line() {
        assert_eq!(one_line("  Shaping\tthe\n last   ascent. ", 140), "Shaping the last ascent.");
        assert_eq!(one_line("a\u{1b}[31mb", 140), "a [31mb");
        let long = "word ".repeat(60);
        let cut = one_line(&long, 20);
        assert!(cut.chars().count() <= 20 && cut.ends_with('…'), "{cut}");
    }

    #[test]
    fn a_tool_is_a_name_and_never_a_command_or_a_secret() {
        assert_eq!(tool_name("blender.add_object").as_deref(), Some("blender.add_object"));
        assert_eq!(tool_name(" agent_run ").as_deref(), Some("agent_run"));
        assert_eq!(tool_name("mcp__files__write").as_deref(), Some("mcp__files__write"));
        for not in [
            "",
            "ls -la /etc",
            "cat ~/.ssh/id_rsa",
            "a b",
            "x".repeat(49).as_str(),
            "echo $TOKEN",
            // What a review put through it: token-shaped names, and a name that is an argument.
            "ghp_0123456789abcdefghijABCDEFGHIJ0123456789",
            "sk-proj-abc123def456",
            "AKIAIOSFODNN7EXAMPLE",
            "read:id_rsa",
            "xoxa-2-1234-5678-abcd",
            "0123456789abcdefABCDEF0123",
        ] {
            assert_eq!(tool_name(not), None, "{not}");
        }
    }

    #[test]
    fn a_model_label_is_a_label() {
        assert_eq!(model_label("deepseek-v4.1-flash via ollama-cloud").as_deref(), Some("deepseek-v4.1-flash via ollama-cloud"));
        assert_eq!(model_label("claude-opus-5").as_deref(), Some("claude-opus-5"));
        for not in [
            "",
            "key=abc",
            "/home/me/model.gguf",
            "user@internal-host:8080/private-model",
            "ollama/llama3",
            "ｈｏｍｅ／ｍｅ／model",
            "http://localhost:11434/api",
        ] {
            assert_eq!(model_label(not), None, "{not}");
        }
    }

    #[test]
    fn a_name_that_is_a_path_or_an_address_is_not_a_name() {
        assert_eq!(display_name("Yantrik Mind"), "Yantrik Mind");
        for not in ["/usr/bin/x", "~/x", "sam.patel@example.com", "C:\\Users\\me", "https://intranet.corp/hr", "Reading ／home／me"] {
            assert_eq!(display_name(not), "", "{not}");
        }
        // The one separator the footer uses between apps is not a word of its own to refuse.
        assert!(!looks_private("Blender / Terminal / Files"));
    }

    #[test]
    fn the_tag_moves_with_what_is_drawn_and_not_with_the_clock() {
        let mut snap = DeskSnapshot { mind: Some(mind()), ..Default::default() }.tagged();
        let first = snap.tag;
        assert!(first <= 0xFFFF_FFFF_FFFF);
        assert_eq!(snap.tag_of(), first);
        // Time passing is not a change.
        snap.as_of = "14:33".into();
        snap.mind.as_mut().unwrap().elapsed_secs = Some(400);
        assert_eq!(snap.tag_of(), first);
        // A tool starting is, and so is a stop being asked.
        snap.mind.as_mut().unwrap().tool = Some(DeskTool { name: "t".into(), evidence: Evidence::Verified, outcome: Outcome::Running });
        let with_tool = snap.tag_of();
        assert_ne!(with_tool, first);
        snap.mind.as_mut().unwrap().stop = Some(StopStatus::Asked);
        assert_ne!(snap.tag_of(), with_tool);
    }

    /// The wire is the whole of what the desk can learn. A field for what a Mind said it is doing,
    /// for an argument, a path, a prompt, a command or a token would be a way to put one on a
    /// window every Mind can read, by picture, by OCR or by `describe`.
    #[test]
    fn the_snapshot_has_no_field_for_what_the_chrome_must_not_show() {
        let mut m = mind();
        m.tool = Some(DeskTool { name: "t".into(), evidence: Evidence::Reported, outcome: Outcome::Ok });
        m.stop = Some(StopStatus::Asked);
        let wire = serde_json::to_string(&DeskSnapshot { mind: Some(m), ..Default::default() }).unwrap();
        for forbidden in [
            "args", "arguments", "path", "prompt", "command", "token", "output", "target", "title", "cwd", "status", "narration",
            "headline", "text", "said", "words", "message", "heard",
        ] {
            assert!(!wire.contains(forbidden), "`{forbidden}` is on the wire: {wire}");
        }
    }
}
