//! The tool trail — a harness's tool call as it travels in the text of an answer, and how the
//! panel reads it back.
//!
//! The harness wire carries text and nothing else (`harness.chunk` has a `delta` and no other
//! field), so a tool call reaches the shell the way the harness chose to write it down. Two
//! writers exist today, and this is what each one actually sends:
//!
//! - this OS's own harness library (`harnesses/lib/yantrik_harness.py`, `tool_trail`) writes
//!   `⚙️ os_act studio.generate {"args":{"prompt":"a red kite"}}` — the tool, what it touched,
//!   and the rest of its arguments as one JSON object on the same line;
//! - Hermes' gateway writes its tool-progress line. In its default mode that is `⚙️ name...`
//!   while a tool runs, or `⚙️ name: "preview"` when Hermes knows the tool's primary argument —
//!   which it does for its own tools and not for MCP tools, so every `mcp_yantrik_os_*` call
//!   arrives as the bare name. In its `verbose` mode it is `⚙️ name(['app', 'action'])` with the
//!   arguments as JSON on the line after. A repeated call collapses to `… (×3)`.
//!
//! Issue #125: the panel showed `mcp_yantrik_os_os_act...` and nothing else, and it read as if
//! the shell had elided the call. It had not — that is the whole line Hermes sends, and the
//! `...` is Hermes saying "running". What the shell did wrong was treat the line as prose: the
//! arguments this OS's own harnesses could have carried had nowhere to go, and a line that did
//! carry them was shown as raw JSON. This module is the trail's one reader, so every harness's
//! call renders the same way — the name, what it touched, its arguments on one line, and the
//! whole of them a click away.

use yantrik_harness::protocol::TRAIL_MARK;

/// The longest one argument's value is shown at on the trail's one line. The whole value is in
/// [`ToolCall::detail`], which the card shows on a click.
const VALUE_CAP: usize = 48;

/// One tool call, read back out of the trail.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolCall {
    /// The tool, as the harness names it: `os_act`, `mcp_yantrik_os_os_act`, `terminal`.
    pub name: String,
    /// What it touched, when the line or the arguments say: `studio.generate`, `notes`.
    pub target: String,
    /// The arguments the line carried, or `Null` when it carried none.
    pub arguments: serde_json::Value,
    /// What Hermes shows instead of arguments: the primary argument, already cut by Hermes.
    pub preview: String,
    /// How many times in a row this call was made, when the harness collapsed them.
    pub repeats: u32,
}

/// Whether a line of an answer is a tool call.
pub fn is_trail(line: &str) -> bool {
    strip_mark(line).is_some()
}

fn strip_mark(line: &str) -> Option<&str> {
    let line = line.trim_start();
    // With and without the variation selector: the harness library and Hermes both send U+2699
    // U+FE0F, but a terminal or a model copying the mark can drop the selector.
    line.strip_prefix(TRAIL_MARK)
        .or_else(|| line.strip_prefix('\u{2699}'))
        .map(|rest| rest.trim())
}

/// `text` with its trail lines left out: what a reply says while it is still arriving. The calls
/// are already said as steps above it (`wire::lens_timeline`), and drawn raw they were
/// `⚙️ memory_remember {"text": …}` in the middle of an answer. The whole text, trail and all, is
/// kept, so a reply the Lens did not follow can still find its calls at the end.
pub fn without_trail(text: &str) -> String {
    let mut kept: Vec<&str> = Vec::new();
    let mut after_call = false;
    for line in text.split('\n') {
        if is_trail(line) {
            after_call = true;
            continue;
        }
        // Hermes' verbose form writes the call's arguments on the line after it.
        if after_call && line.trim_start().starts_with('{') {
            continue;
        }
        after_call = false;
        kept.push(line);
    }
    kept.join("\n").trim_start_matches('\n').to_string()
}

/// Read one call from a trail line.
///
/// `next` is the line after it, offered because Hermes' verbose form puts the arguments there.
/// The `bool` says whether that line was the arguments and has been consumed.
pub fn parse(line: &str, next: Option<&str>) -> Option<(ToolCall, bool)> {
    let mut rest = strip_mark(line)?.to_string();
    if rest.is_empty() {
        return None;
    }
    let mut call = ToolCall::default();

    // Hermes: `… (×3)` when the same call ran three times in a row.
    if let Some(open) = rest.rfind(" (×") {
        if let Some(n) = rest[open + " (×".len()..].strip_suffix(')').and_then(|n| n.parse().ok()) {
            call.repeats = n;
            rest.truncate(open);
        }
    }
    // Hermes: `name...` means "running", not "elided".
    let rest = rest.trim_end_matches("...").trim_end_matches('…').trim_end().to_string();

    // Hermes with a primary argument: `terminal: "ls -la"`.
    if let Some((name, preview)) = rest.split_once(": \"") {
        if let Some(preview) = preview.strip_suffix('"') {
            call.name = name.trim().to_string();
            call.preview = preview.to_string();
            return Some((call, false));
        }
    }

    // Hermes verbose: `name(['app', 'action', 'args'])`, arguments on the next line.
    if let Some((name, keys)) = rest.split_once('(') {
        if keys.ends_with(')') && !name.contains(' ') {
            call.name = name.trim().to_string();
            let consumed = match next.map(str::trim).and_then(parse_object) {
                Some(args) => {
                    call.arguments = args;
                    true
                }
                None => false,
            };
            call.target = target_of(&call.arguments);
            return Some((call, consumed));
        }
    }

    // This OS's harnesses: `name [target] [{json}]`.
    let (label, args) = match rest.find(" {") {
        Some(at) => match parse_object(&rest[at + 1..]) {
            Some(args) => (&rest[..at], args),
            None => (rest.as_str(), serde_json::Value::Null),
        },
        None => (rest.as_str(), serde_json::Value::Null),
    };
    let mut words = label.split_whitespace();
    call.name = words.next().unwrap_or_default().to_string();
    call.target = words.collect::<Vec<_>>().join(" ");
    call.arguments = args;
    if call.target.is_empty() {
        call.target = target_of(&call.arguments);
    }
    Some((call, false))
}

fn parse_object(text: &str) -> Option<serde_json::Value> {
    if !text.starts_with('{') {
        return None;
    }
    serde_json::from_str::<serde_json::Value>(text).ok().filter(|v| v.is_object())
}

/// `app.action`, `app`, or nothing — from the arguments, the way the harness library labels it.
fn target_of(arguments: &serde_json::Value) -> String {
    let field = |k: &str| arguments.get(k).and_then(|v| v.as_str()).unwrap_or_default().trim().to_string();
    let app = field("app");
    // `os_describe` lists actions as `new_note()`; a model copying that sends the punctuation.
    let action = field("action").split('(').next().unwrap_or_default().trim().to_string();
    match (app.is_empty(), action.is_empty()) {
        (false, false) => format!("{app}.{action}"),
        (false, true) => app,
        _ => String::new(),
    }
}

impl ToolCall {
    /// The one line the panel shows: the name, what it touched, and the arguments as
    /// `key=value`, each value cut to [`VALUE_CAP`]. Keys come in `serde_json`'s order, which is
    /// alphabetical — stable from one call to the next, which is what a reader scanning a column
    /// of calls needs.
    pub fn summary(&self) -> String {
        let mut out = self.name.clone();
        if !self.target.is_empty() {
            out.push(' ');
            out.push_str(&self.target);
        }
        if !self.preview.is_empty() {
            out.push_str(&format!(" \"{}\"", elide(&self.preview)));
        }
        for (key, value) in self.shown_arguments() {
            out.push_str(&format!(" {key}={}", short(&value)));
        }
        if self.repeats > 0 {
            out.push_str(&format!(" ×{}", self.repeats));
        }
        out
    }

    /// The arguments in full, for the click that opens the line. Empty when the line carried
    /// none — then the summary already is everything the harness sent.
    pub fn detail(&self) -> String {
        if !self.arguments.is_object() {
            return String::new();
        }
        serde_json::to_string_pretty(&self.arguments).unwrap_or_default()
    }

    /// The call as the Slint card takes it — the one `ToolCallCard` every view of a mind's
    /// calls draws.
    ///
    /// `status` and `output` are left empty: the trail says neither whether a call succeeded nor
    /// what it returned, and a card that guessed would be worse than one that says nothing.
    pub fn to_card(&self) -> crate::ToolCallData {
        crate::ToolCallData {
            name: self.name.as_str().into(),
            target: self.target.as_str().into(),
            summary: self.summary().into(),
            arguments: self.detail().into(),
            status: Default::default(),
            output: Default::default(),
        }
    }

    /// The arguments worth a place on the line: everything but `app` and `action` once the
    /// target says them, with an `args` object opened up so `prompt=…` reads as itself rather
    /// than as `args={"prompt":…}`.
    fn shown_arguments(&self) -> Vec<(String, serde_json::Value)> {
        let Some(object) = self.arguments.as_object() else {
            return Vec::new();
        };
        let mut shown = Vec::new();
        for (key, value) in object {
            if !self.target.is_empty() && (key == "app" || key == "action") {
                continue;
            }
            match value.as_object() {
                Some(inner) if key == "args" || key == "arguments" => {
                    shown.extend(inner.iter().map(|(k, v)| (k.clone(), v.clone())));
                }
                _ => shown.push((key.clone(), value.clone())),
            }
        }
        shown
    }
}

fn short(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => format!("\"{}\"", elide(s)),
        other => elide(&other.to_string()),
    }
}

fn elide(text: &str) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= VALUE_CAP {
        return flat;
    }
    let head: String = flat.chars().take(VALUE_CAP - 1).collect();
    format!("{}…", head.trim_end())
}

/// Every call in an answer, in order — for `describe`, which reads the transcript as text.
pub fn calls_in(text: &str) -> Vec<ToolCall> {
    let mut calls = Vec::new();
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        if let Some((call, took_next)) = parse(line, lines.peek().copied()) {
            if took_next {
                lines.next();
            }
            calls.push(call);
        }
    }
    calls
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The line this OS's own harnesses send, with the arguments on it. Before #125 the library
    /// left the arguments off, and the panel had no reader for them anyway.
    #[test]
    fn a_call_from_this_oss_harness_shows_what_it_touched_and_with_what() {
        let (call, took) = parse(
            r#"⚙️ os_act studio.generate {"args":{"prompt":"a red kite over the sea at dusk","count":1}}"#,
            Some("Done."),
        )
        .unwrap();
        assert!(!took, "the arguments were on the line; the answer's next line is prose");
        assert_eq!(call.name, "os_act");
        assert_eq!(call.target, "studio.generate");
        assert_eq!(
            call.summary(),
            r#"os_act studio.generate count=1 prompt="a red kite over the sea at dusk""#
        );
        assert!(call.detail().contains("\"prompt\": \"a red kite over the sea at dusk\""));
    }

    #[test]
    fn a_call_with_nothing_but_a_name_is_shown_as_that_and_nothing_is_invented() {
        // Hermes' default mode for an MCP tool. The `...` is Hermes' "running", not an elision,
        // and the panel must not present a name-only line as if there were more to open.
        let (call, took) = parse("⚙️ mcp_yantrik_os_os_act...", Some("")).unwrap();
        assert!(!took);
        assert_eq!(call.summary(), "mcp_yantrik_os_os_act");
        assert_eq!(call.detail(), "", "nothing was sent, so there is nothing to expand");
        assert_eq!(parse("⚙️ os_apps", None).unwrap().0.summary(), "os_apps");
    }

    #[test]
    fn hermes_primary_argument_preview_is_kept_as_the_argument() {
        let (call, _) = parse(r#"⚙️ terminal: "ls -la ~/Pictures""#, None).unwrap();
        assert_eq!(call.name, "terminal");
        assert_eq!(call.summary(), r#"terminal "ls -la ~/Pictures""#);
    }

    #[test]
    fn hermes_verbose_form_takes_its_arguments_from_the_next_line() {
        let (call, took) = parse(
            "⚙️ mcp_yantrik_os_os_act(['app', 'action', 'args'])",
            Some(r#"{"app": "studio", "action": "generate", "args": {"prompt": "a red kite"}}"#),
        )
        .unwrap();
        assert!(took, "the JSON line is the arguments, not prose");
        assert_eq!(call.target, "studio.generate");
        assert_eq!(call.summary(), r#"mcp_yantrik_os_os_act studio.generate prompt="a red kite""#);
        assert_eq!(call.arguments["app"], "studio");
    }

    #[test]
    fn a_repeated_call_says_how_many_times() {
        let (call, _) = parse(r#"⚙️ terminal: "make" (×3)"#, None).unwrap();
        assert_eq!(call.repeats, 3);
        assert_eq!(call.summary(), r#"terminal "make" ×3"#);
    }

    #[test]
    fn a_long_value_is_cut_on_the_line_and_whole_in_the_detail() {
        let body = "word ".repeat(40);
        let line = format!(r#"⚙️ os_act notes.new_note {{"args":{{"body":{}}}}}"#, serde_json::json!(body));
        let (call, _) = parse(&line, None).unwrap();
        let summary = call.summary();
        assert!(summary.ends_with("…\""), "{summary}");
        assert!(summary.chars().count() < 100, "{summary}");
        assert!(call.detail().contains(body.trim_end()), "the whole value is a click away");
    }

    #[test]
    fn prose_and_code_are_not_calls() {
        assert!(!is_trail("The gear icon ⚙️ opens settings."));
        assert!(!is_trail("Done."));
        assert!(parse("⚙️", None).is_none());
        // A brace that is not an object stays part of the label rather than crashing the line.
        let (call, _) = parse("⚙️ os_act notes.new_note {not json", None).unwrap();
        assert_eq!(call.name, "os_act");
        assert!(call.arguments.is_null());
    }

    /// The card is what the bubble — and any later view of a mind's work — draws, so what the
    /// person can open has to be exactly what the harness sent.
    #[test]
    fn the_card_carries_the_line_and_the_whole_arguments_and_claims_no_outcome() {
        let (call, _) = parse(r#"⚙️ os_act studio.generate {"args":{"prompt":"a red kite"}}"#, None).unwrap();
        let card = call.to_card();
        assert_eq!(card.name, "os_act");
        assert_eq!(card.target, "studio.generate");
        assert_eq!(card.summary, r#"os_act studio.generate prompt="a red kite""#);
        assert!(card.arguments.contains(r#""prompt": "a red kite""#), "{}", card.arguments);
        assert_eq!(card.status, "", "the trail does not say how a call went");
        assert_eq!(card.output, "", "the trail does not carry what a call returned");
    }

    #[test]
    fn every_call_in_an_answer_is_found_in_order() {
        let text = "Looking.\n⚙️ os_apps\n\n⚙️ os_act calendar.add_event {\"args\":{\"title\":\"Dentist\"}}\n\nDone.";
        let names: Vec<String> = calls_in(text).iter().map(|c| c.summary()).collect();
        assert_eq!(names, vec!["os_apps", "os_act calendar.add_event title=\"Dentist\""]);
    }
}
