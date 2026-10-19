//! What a mind is doing, as one plain sentence: "Looking at your calendar", "Searching the web for
//! "flights to Goa"", "Couldn't open the web page".
//!
//! A mind's steps reach the shell as tool names and arguments (`os_act calendar.add_event {…}`)
//! and as its own status lines ("Using web_search…"). Shown as they arrive, they are a log, not an
//! account: the Lens drew `⚙️ memory_remember {"text": …}` in the middle of DeepSeek's answers on
//! VM 541, and a person cannot tell from that whether anything is happening. This is the one
//! place a step becomes words, so a reply's timeline while it runs, the same timeline once it has
//! ended and the Agents screen's run words say the same thing about the same call.
//!
//! The rule the table keeps: never an argument dump, a token, a path or JSON. The one argument
//! that is said is a search's query, and only a short plain one, because "Searching the web" for
//! two minutes tells the person less than what it is searching for.

use crate::agents::progress;
use serde_json::Value;

/// The longest query a step quotes. Longer ones are cut on a word.
const QUERY_CHARS: usize = 40;

/// The longest status line of the mind's own that is said as it wrote it.
const SAID_CHARS: usize = 60;

/// One step, in the three ways a sentence about it is needed: while it runs, once it is done, and
/// when it could not be done.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phrase {
    /// A verb and what it acts on: `look` / `looked` / `looking` + " at your calendar".
    Verb { base: String, past: String, ing: String, rest: String },
    /// Words the mind wrote itself, already plain, said as they are.
    Said(String),
}

impl Phrase {
    /// "Looking at your calendar".
    pub fn running(&self) -> String {
        match self {
            Phrase::Verb { ing, rest, .. } => capitalised(&format!("{ing}{rest}")),
            Phrase::Said(s) => s.clone(),
        }
    }

    /// "Looked at your calendar".
    pub fn done(&self) -> String {
        match self {
            Phrase::Verb { past, rest, .. } => capitalised(&format!("{past}{rest}")),
            Phrase::Said(s) => s.clone(),
        }
    }

    /// Whether it is something done, as opposed to the mind thinking or working between steps:
    /// only actions stay in a reply's timeline once it has ended.
    pub fn is_action(&self) -> bool {
        !matches!(self, Phrase::Verb { base, .. } if base == "think" || base == "work")
    }

    /// "Delete an event from your calendar": the step as something asked for, for a wait on the
    /// person's allow.
    pub fn asked(&self) -> String {
        match self {
            Phrase::Verb { base, rest, .. } => capitalised(&format!("{base}{rest}")),
            Phrase::Said(s) => s.clone(),
        }
    }

    /// "Couldn't look at your calendar".
    pub fn failed(&self) -> String {
        match self {
            Phrase::Verb { base, rest, .. } => format!("Couldn\u{2019}t {base}{rest}"),
            Phrase::Said(s) => format!("Couldn\u{2019}t finish: {}", lower_first(s)),
        }
    }
}

/// A verb from its base form, with its past and -ing forms. Irregular ones are listed; the rest
/// follow the spelling rules, which the verbs this table uses keep to.
fn verb(base: &str, rest: &str) -> Phrase {
    let (past, ing) = match base {
        "write" => ("wrote".to_string(), "writing".to_string()),
        "run" => ("ran".to_string(), "running".to_string()),
        "send" => ("sent".to_string(), "sending".to_string()),
        "find" => ("found".to_string(), "finding".to_string()),
        "get" => ("got".to_string(), "getting".to_string()),
        "make" => ("made".to_string(), "making".to_string()),
        "read" => ("read".to_string(), "reading".to_string()),
        "set" => ("set".to_string(), "setting".to_string()),
        "put" => ("put".to_string(), "putting".to_string()),
        "take" => ("took".to_string(), "taking".to_string()),
        // Said once done as what it was: "✓ Thought it through", never a bare "Thought".
        "think" => ("thought it through".to_string(), "thinking".to_string()),
        "work" => ("worked on it".to_string(), "working".to_string()),
        "stop" => ("stopped".to_string(), "stopping".to_string()),
        "plan" => ("planned".to_string(), "planning".to_string()),
        "begin" => ("began".to_string(), "beginning".to_string()),
        "buy" => ("bought".to_string(), "buying".to_string()),
        "pay" => ("paid".to_string(), "paying".to_string()),
        "draw" => ("drew".to_string(), "drawing".to_string()),
        "do" => ("did".to_string(), "doing".to_string()),
        "copy" => ("copied".to_string(), "copying".to_string()),
        _ if base.ends_with('e') && !base.ends_with("ee") => {
            (format!("{base}d"), format!("{}ing", &base[..base.len() - 1]))
        }
        _ => (format!("{base}ed"), format!("{base}ing")),
    };
    Phrase::Verb { base: base.to_string(), past, ing, rest: rest.to_string() }
}

/// The verbs an action id may start with to be said as a sentence (`files.move_file` → "Moving
/// file in your files"). Anything else is "Using <app>", which is never wrong.
const VERBS: &[&str] = &[
    "add", "archive", "cancel", "change", "check", "clear", "close", "copy", "create", "delete",
    "download", "edit", "export", "find", "generate", "get", "import", "install", "list", "load",
    "make", "mark", "move", "open", "pause", "play", "print", "read", "remove", "rename", "render", "reply",
    "restore", "resume", "run", "save", "search", "send", "set", "share", "show", "start", "stop",
    "switch", "update", "upload", "write",
];

/// An app as the sentence names it: "your calendar", "the Editor", "Download Manager".
pub fn app_words(app: &str) -> String {
    match app.trim().to_ascii_lowercase().as_str() {
        "calendar" => "your calendar".into(),
        "notes" => "your notes".into(),
        "files" => "your files".into(),
        "documents" => "your documents".into(),
        "email" | "mail" => "your email".into(),
        "contacts" => "your contacts".into(),
        "photos" | "gallery" => "your photos".into(),
        "music" => "your music".into(),
        "editor" => "the Editor".into(),
        "terminal" => "the terminal".into(),
        "browser" => "the browser".into(),
        "settings" => "Settings".into(),
        "shell" | "desktop" => "the desktop".into(),
        other => super::pins::humanise(other),
    }
}

/// A tool name with the bridges' prefixes taken off: Hermes calls `os_act` `mcp_yantrik_os_os_act`.
fn bare(name: &str) -> &str {
    let name = name.trim();
    for prefix in ["mcp_yantrik_os_", "mcp__yantrik_os__", "yantrik_os_"] {
        if let Some(rest) = name.strip_prefix(prefix) {
            return rest;
        }
    }
    name
}

/// "memory_stats" → "memory stats": a name nothing here knows, said as words.
fn humanised(name: &str) -> String {
    let words: Vec<&str> = name.split(['_', '-', '.', ':']).filter(|w| !w.is_empty()).collect();
    if words.is_empty() {
        return "a tool".into();
    }
    words.join(" ").to_lowercase()
}

/// A search's query, when it is short plain words: quoted, cut at 40 characters on a word.
fn query_of(args: &Value, preview: &str) -> Option<String> {
    let inner = args.get("args").unwrap_or(&Value::Null);
    let raw = ["query", "q", "search", "text"]
        .iter()
        .find_map(|k| args.get(*k).or_else(|| inner.get(*k)).and_then(Value::as_str))
        .unwrap_or(preview);
    plain(raw).map(|q| progress::brief(&q, QUERY_CHARS))
}

/// `text` when it reads as words a person could have written, flattened; None when it looks like
/// data: JSON, markup, a path, a link, or nothing at all.
fn plain(text: &str) -> Option<String> {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let data = flat.is_empty()
        || flat.contains(['{', '}', '[', ']', '<', '>', '\\', '`', '='])
        || flat.starts_with(['/', '~', '.'])
        || flat.contains("://")
        || flat.split_whitespace().any(|w| w.contains('/') && w.len() > 1);
    (!data).then_some(flat)
}

/// What a call is doing, from its tool name, what it touched (`calendar.add_event`, `notes`), its
/// arguments, and the preview a harness sends instead of arguments.
pub fn of_call(name: &str, target: &str, args: &Value, preview: &str) -> Phrase {
    let name = bare(name);
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        "os_describe" | "describe" => {
            let app = app_of(target, args);
            if app.is_empty() {
                return verb("look", " around the desktop");
            }
            return verb("look", &format!(" at {}", app_words(&app)));
        }
        "os_act" | "act" => return of_action(target, args, preview),
        "os_apps" | "apps" => return verb("check", " which apps are open"),
        "os_open_app" | "open_app" => {
            let app = app_of(target, args);
            return if app.is_empty() { verb("open", " an app") } else { verb("open", &format!(" {}", app_words(&app))) };
        }
        _ => {}
    }
    if is_memory_recall(&lower) {
        return verb("check", " what I remember");
    }
    if is_memory_save(&lower) {
        return verb("save", " that to memory");
    }
    if matches!(lower.as_str(), "web_search" | "browser_search" | "search_web" | "websearch" | "search" | "life_search") {
        return searching(args, preview);
    }
    if matches!(lower.as_str(), "web_fetch" | "http_fetch" | "fetch" | "browse" | "browser_snapshot" | "browser_see" | "browser_open" | "open_url") {
        return verb("open", " the web page");
    }
    if matches!(lower.as_str(), "bash" | "terminal" | "run_command" | "shell" | "exec" | "process") {
        return verb("run", " a command");
    }
    if matches!(lower.as_str(), "read" | "read_file" | "view") {
        return verb("read", " a file");
    }
    if matches!(lower.as_str(), "write" | "write_file" | "create_file") {
        return verb("write", " a file");
    }
    if matches!(lower.as_str(), "edit" | "edit_file" | "patch" | "str_replace") {
        return verb("edit", " a file");
    }
    if matches!(lower.as_str(), "ls" | "glob" | "grep" | "list_files" | "search_files" | "find") {
        return verb("look", " through files");
    }
    if lower.starts_with("calendar") {
        return verb("look", " at your calendar");
    }
    if lower.starts_with("email") {
        return verb("check", " your email");
    }
    // A tool named for an app's action, as some harnesses name them: `files.move`.
    if name.contains('.') && !name.contains(char::is_whitespace) {
        return of_action(name, args, preview);
    }
    verb("use", &format!(" {}", humanised(name)))
}

/// `os_act <app>.<action>`, or an app's own act: "Adding an event to your calendar".
fn of_action(target: &str, args: &Value, preview: &str) -> Phrase {
    let target = if target.trim().is_empty() {
        let app = args.get("app").and_then(Value::as_str).unwrap_or_default();
        let action = args.get("action").and_then(Value::as_str).unwrap_or_default();
        if action.is_empty() { app.to_string() } else { format!("{app}.{action}") }
    } else {
        target.trim().to_string()
    };
    let (app, action) = target.split_once('.').unwrap_or((target.as_str(), ""));
    let (app, action) = (app.to_ascii_lowercase(), action.to_ascii_lowercase());
    match (app.as_str(), action.as_str()) {
        ("calendar", "add_event" | "create_event" | "new_event") => verb("add", " an event to your calendar"),
        ("calendar", "delete_event" | "remove_event") => verb("delete", " an event from your calendar"),
        ("calendar", "update_event" | "edit_event" | "move_event") => verb("change", " an event in your calendar"),
        ("calendar", _) => verb("look", " at your calendar"),
        ("notes", "new_note" | "create_note" | "add_note" | "write_note") => verb("write", " a note"),
        ("notes", "delete_note" | "remove_note") => verb("delete", " a note"),
        ("notes", "edit_note" | "update_note" | "append") => verb("edit", " a note"),
        ("notes", _) => verb("look", " through your notes"),
        ("household" | "browser" | "web", a) if a.contains("search") => searching(args, preview),
        ("browser" | "web", "open" | "navigate" | "visit" | "fetch") => verb("open", " the web page"),
        ("terminal", _) | ("shell", "agent_run") => verb("run", " a command"),
        ("memory", a) if is_memory_recall(a) => verb("check", " what I remember"),
        ("memory", a) if is_memory_save(a) => verb("save", " that to memory"),
        ("", _) => verb("use", " an app"),
        (app, action) => generic_action(app, action),
    }
}

/// An action this table has no sentence for, from its id: `files.move_file` → "Moving file in your
/// files"; `clock.start` → "Starting in Clock"; anything else "Using Clock".
fn generic_action(app: &str, action: &str) -> Phrase {
    let mut words = action.split(['_', '-']).filter(|w| !w.is_empty());
    let first = words.next().unwrap_or_default();
    let object: Vec<&str> = words.collect();
    let plain_words = object.iter().all(|w| w.chars().all(|c| c.is_ascii_alphabetic())) && object.len() <= 3;
    if !VERBS.contains(&first) || !plain_words {
        return verb("use", &format!(" {}", app_words(app)));
    }
    let object = if object.is_empty() { String::new() } else { format!(" {}", object.join(" ")) };
    verb(first, &format!("{object} in {}", app_words(app)))
}

fn searching(args: &Value, preview: &str) -> Phrase {
    match query_of(args, preview) {
        Some(q) => verb("search", &format!(" the web for \u{201c}{q}\u{201d}")),
        None => verb("search", " the web"),
    }
}

fn app_of(target: &str, args: &Value) -> String {
    let from_args = args.get("app").or_else(|| args.get("name")).and_then(Value::as_str).unwrap_or_default();
    let app = if target.trim().is_empty() { from_args } else { target.trim() };
    app.split('.').next().unwrap_or_default().to_string()
}

fn is_memory_recall(name: &str) -> bool {
    matches!(name, "recall" | "memory_recall" | "memory_search" | "search_memory" | "recall_memory" | "memory_get" | "memory_read")
}

fn is_memory_save(name: &str) -> bool {
    matches!(name, "remember" | "memory_remember" | "save_user_fact" | "memory_save" | "memory_write" | "memory_add" | "save_memory")
}

/// The mind's own status line as a step: "Using web_search…" through the same table as a call,
/// "Grounding from memory…" as "Checking what I remember", "Thinking… (60 s)" as "Thinking". None
/// for nothing to say, and for a wait, which the Lens says from the approval itself.
pub fn of_status(text: &str) -> Option<Phrase> {
    let mut line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    // The heartbeat's counter: "Thinking… (60 s)". The Lens counts the time itself.
    if line.ends_with(')') {
        if let Some(open) = line.rfind(" (") {
            line.truncate(open);
        }
    }
    let line = line.trim_end_matches(['…', '.', ' ']).trim();
    let lower = line.to_lowercase();
    // A question the mind asked ("asks: Which days…", as the store puts it) is drawn as its own
    // card in the reply, not said again as a step.
    if line.is_empty() || lower.starts_with("waiting") || lower.starts_with("asks:") {
        return None;
    }
    if let Some(tool) = line.strip_prefix("Using ").or_else(|| line.strip_prefix("using ")) {
        let tool = tool.split_whitespace().next().unwrap_or_default();
        return Some(of_call(tool, "", &Value::Null, ""));
    }
    if lower.starts_with("grounding") || lower.starts_with("recalling") || lower.contains("memory") {
        return Some(verb("check", " what I remember"));
    }
    if lower.starts_with("thinking") {
        return Some(verb("think", ""));
    }
    // Hermes's heartbeat ("Working — 3 min"): the same one step however long it runs, never a new
    // one each time its minutes change, and not an action.
    if lower.starts_with("working") {
        return Some(verb("work", ""));
    }
    match plain(line).filter(|s| s.chars().count() <= SAID_CHARS) {
        Some(said) => Some(Phrase::Said(capitalised(&said))),
        None => Some(verb("work", "")),
    }
}

fn capitalised(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn lower_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(first) => first.to_lowercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn call(name: &str, target: &str, args: Value) -> String {
        of_call(name, target, &args, "").running()
    }

    #[test]
    fn looking_at_an_app_names_the_app_as_a_person_would() {
        assert_eq!(call("os_describe", "", json!({"app": "calendar"})), "Looking at your calendar");
        assert_eq!(call("os_describe", "calendar", Value::Null), "Looking at your calendar");
        assert_eq!(call("mcp_yantrik_os_os_describe", "notes", Value::Null), "Looking at your notes");
        assert_eq!(call("os_describe", "download-manager", Value::Null), "Looking at Download Manager");
    }

    #[test]
    fn acting_in_an_app_says_what_is_being_done_there() {
        assert_eq!(call("os_act", "calendar.add_event", json!({"args": {"title": "Dentist"}})), "Adding an event to your calendar");
        assert_eq!(call("os_act", "notes.new_note", Value::Null), "Writing a note");
        assert_eq!(call("os_act", "", json!({"app": "notes", "action": "new_note"})), "Writing a note");
        assert_eq!(call("os_act", "files.move_file", Value::Null), "Moving file in your files");
        assert_eq!(call("os_act", "clock.frobnicate", Value::Null), "Using Clock");
        assert_eq!(call("files.move", "", Value::Null), "Moving in your files", "a tool named for an app's action");
    }

    #[test]
    fn memory_is_checking_what_i_remember_or_saving_to_it() {
        assert_eq!(call("memory_recall", "", Value::Null), "Checking what I remember");
        assert_eq!(call("memory_remember", "", json!({"text": "Pranab likes tea"})), "Saving that to memory");
        assert_eq!(of_call("memory_remember", "", &Value::Null, "").done(), "Saved that to memory");
    }

    #[test]
    fn a_search_quotes_a_short_query_and_cuts_a_long_one() {
        assert_eq!(call("web_search", "", json!({"query": "weather in Bentonville"})), "Searching the web for \u{201c}weather in Bentonville\u{201d}");
        let long = call("web_search", "", json!({"query": "the best way to get from Bentonville to Goa with two children in December"}));
        assert!(long.starts_with("Searching the web for \u{201c}the best way"), "{long}");
        let quoted = long.split('\u{201c}').nth(1).unwrap().trim_end_matches('\u{201d}');
        assert!(quoted.chars().count() <= QUERY_CHARS + 1, "cut at 40 characters: {quoted}");
        assert!(quoted.ends_with('…'));
        assert_eq!(call("os_act", "household.search", json!({"args": {"query": "milk"}})), "Searching the web for \u{201c}milk\u{201d}");
        assert_eq!(call("web_search", "", Value::Null), "Searching the web");
    }

    #[test]
    fn a_step_never_shows_json_a_path_or_a_token() {
        for (name, args, preview) in [
            ("web_search", json!({"query": "{\"q\": \"x\"}"}), ""),
            ("web_search", json!({"query": "/home/pranab/.ssh/id_rsa"}), ""),
            ("web_search", json!({"query": "https://example.com/?token=abc"}), ""),
            ("web_search", Value::Null, "~/Documents/secret.txt"),
            ("read_file", json!({"path": "/etc/shadow"}), ""),
            ("frob_tool", json!({"api_key": "sk-123", "payload": {"a": 1}}), ""),
        ] {
            let p = of_call(name, "", &args, preview);
            for said in [p.running(), p.done(), p.failed()] {
                assert!(!said.contains(['{', '}', '/', '~', '=']), "{name}: {said}");
                assert!(!said.contains("sk-123") && !said.contains("shadow"), "{name}: {said}");
            }
        }
    }

    #[test]
    fn an_unknown_tool_is_used_by_its_name_in_words() {
        assert_eq!(call("image_resize", "", json!({"w": 10})), "Using image resize");
        assert_eq!(of_call("image_resize", "", &Value::Null, "").failed(), "Couldn\u{2019}t use image resize");
    }

    #[test]
    fn a_failed_step_says_what_could_not_be_done() {
        assert_eq!(of_call("web_fetch", "", &json!({"url": "https://x.y"}), "").failed(), "Couldn\u{2019}t open the web page");
        assert_eq!(of_call("os_describe", "calendar", &Value::Null, "").done(), "Looked at your calendar");
    }

    #[test]
    fn the_minds_status_lines_go_through_the_same_table() {
        let said = |s: &str| of_status(s).map(|p| p.running());
        assert_eq!(said("Using web_search…").as_deref(), Some("Searching the web"));
        assert_eq!(said("Grounding from memory…").as_deref(), Some("Checking what I remember"));
        assert_eq!(said("Thinking…").as_deref(), Some("Thinking"));
        assert_eq!(said("Thinking… (60 s)").as_deref(), Some("Thinking"), "the counter is the Lens's to say");
        assert_eq!(said("Reading your inbox").as_deref(), Some("Reading your inbox"), "plain words are said as written");
        assert_eq!(said("{\"step\": 3}").as_deref(), Some("Working"), "data is never said");
        assert_eq!(said("waiting for your approval"), None);
        assert_eq!(said("   "), None);
    }

    /// Hermes's heartbeat changed its minutes every few, and each change was a new "done" step
    /// in the reply ("Working — 3 min", "Working — 6 min"); the question a mind asks was a step
    /// too, beside its own card.
    #[test]
    fn a_heartbeat_is_one_unchanging_step_and_a_question_is_no_step_at_all() {
        let three = of_status("Working — 3 min").expect("a step");
        assert_eq!(three, of_status("Working — 6 min").unwrap(), "the same step whatever its minutes");
        assert!(!three.is_action(), "and not something done, so no reply lists it");
        assert_eq!(of_status("asks: Which days is this weekend for you?"), None);
    }
}
