//! The mind panel's plain words: the model a mind runs on, said the way a person reads it, and
//! each recent action as a whole sentence. The raw id and the raw action name stay in the data,
//! for the panel's Details.
//!
//! UI punch list, 12 October (44 screens of build 1461): the panel's status line read
//! "attached · ollama-cloud:deeps…", a provider id clipped by the panel's width, on every screen;
//! Recent actions read "terminal.run", "calendar.update_event" and a bare "failed".
//! design/experience-final-2026-10-10.md: one sentence on top, the proof under it behind Details.

use crate::runs_on::identity::{self, ProviderRef};
use yantrik_ml::KNOWN_PROVIDERS;

// ── The model ───────────────────────────────────────────────────────

/// What a model id says, in a person's words: "DeepSeek V4 Flash, on Ollama Cloud".
///
/// Built from the AI-accounts catalogue's display names: the provider by
/// `runs_on::identity::from_reported` (never guessed from a model name), the model's family by
/// the same catalogue ("deepseek" → "DeepSeek"). A raw id never reaches the result — an address
/// is said as where it is, not by its host.
pub fn plain_model_name(raw: &str) -> String {
    let raw = raw.trim();
    if raw.is_empty() {
        return String::new();
    }
    let (provider, model) = identity::from_reported(raw);
    let model = model.map(|m| humanise_model(&m)).filter(|m| !m.is_empty());
    let place = match &provider {
        ProviderRef::Known(p) => Some(p.display_name.to_string()),
        ProviderRef::Local(_) => Some("this network".to_string()),
        ProviderRef::Custom(_) => Some("your own server".to_string()),
        ProviderRef::NotReported => None,
    };
    match (model, place) {
        (Some(m), Some(p)) => format!("{m}, on {p}"),
        (Some(m), None) => m,
        (None, Some(p)) => format!("a model on {p}"),
        // Neither could be told apart. A mind's own sentence ("Hermes 0.14 on node1") is words a
        // person can read; an unparsed single token is humanised like a model name.
        (None, None) if raw.contains(char::is_whitespace) => one_line(raw),
        (None, None) => humanise_model(raw),
    }
}

/// "deepseek-v4.1-flash" → "DeepSeek V4.1 Flash"; "qwen3.5:9b" → "Qwen 3.5 9B";
/// "nvidia/nemotron-3-super-120b-a12b" → "Nemotron 3 Super 120B A12B".
pub fn humanise_model(id: &str) -> String {
    // Only the last path segment names the model; the rest is an organisation or a provider.
    let id = id.trim().rsplit('/').next().unwrap_or("");
    let (name, tag) = match id.split_once(':') {
        Some((n, t)) => (n, Some(t)),
        None => (id, None),
    };
    let mut words: Vec<String> = name.split(['-', '_', ' ']).filter(|w| !w.is_empty()).map(word).collect();
    if let Some(tag) = tag.filter(|t| !t.is_empty() && !t.eq_ignore_ascii_case("latest")) {
        words.extend(tag.split(['-', '_']).filter(|w| !w.is_empty()).map(word));
    }
    words.join(" ")
}

/// Words a model id spells in capitals.
const ACRONYMS: &[&str] = &["gpt", "glm", "oss", "llm", "vl", "moe", "it", "hf", "gguf", "awq", "fp8", "fp16", "q4", "q8"];

/// One word of a model id, as a person writes it.
fn word(w: &str) -> String {
    let lower = w.to_ascii_lowercase();
    if let Some(name) = catalogue_name(&lower) {
        return name;
    }
    if ACRONYMS.contains(&lower.as_str()) {
        return lower.to_ascii_uppercase();
    }
    // A size or a version: "9b", "120b", "a12b", "v4", "v4.1", "8x7b".
    let digits = lower.chars().any(|c| c.is_ascii_digit());
    if digits && lower.len() <= 6 && lower.chars().all(|c| c.is_ascii_alphanumeric() || c == '.') {
        if lower.starts_with(|c: char| c.is_ascii_alphabetic()) && lower[1..].starts_with(|c: char| c.is_ascii_digit()) {
            return lower.to_ascii_uppercase();
        }
        if lower.starts_with(|c: char| c.is_ascii_digit()) {
            return lower.to_ascii_uppercase();
        }
    }
    // A family with its version run on: "qwen3.5" → "Qwen 3.5", "llama3" → "Llama 3".
    if let Some(at) = lower.find(|c: char| c.is_ascii_digit()) {
        let (family, version) = lower.split_at(at);
        if family.len() >= 3 && family.chars().all(|c| c.is_ascii_alphabetic()) {
            return format!("{} {}", word(family), version);
        }
    }
    capital(&lower)
}

/// A catalogue display name for a word that is a provider's id or one word of its name:
/// "deepseek" → "DeepSeek", "qwen" → "Qwen", "kimi" → "Kimi".
fn catalogue_name(lower: &str) -> Option<String> {
    KNOWN_PROVIDERS.iter().filter(|p| !p.display_name.is_empty()).find_map(|p| {
        if p.id == lower {
            return Some(p.display_name.split_whitespace().next().unwrap_or(p.display_name).to_string());
        }
        p.display_name.split_whitespace().find(|w| w.eq_ignore_ascii_case(lower) && w.len() >= 4).map(str::to_string)
    })
}

fn capital(w: &str) -> String {
    let mut chars = w.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ── Recent actions ──────────────────────────────────────────────────

/// One recent action, said: the sentence on the row, whether it went wrong, and the proof for
/// Details (the raw `app.action` and the status as recorded).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ActWords {
    pub sentence: String,
    pub failed: bool,
    pub detail: String,
}

/// The sentence for one act. `description` is the action's own published description when the
/// surface is in reach — the shell's, read from its registry — and the sentence is made from its
/// first clause; otherwise, or when that clause is not plain enough to show, from the action's
/// name, humanised.
pub fn act_words(app: &str, action: &str, outcome: &str, description: Option<&str>) -> ActWords {
    let doing = description.and_then(clause).unwrap_or_else(|| from_name(app, action));
    let outcome = outcome.trim();
    let lower = outcome.to_ascii_lowercase();
    let failed = lower.starts_with("fail") || lower.starts_with("error");
    let sentence = if failed {
        format!("Tried to {}, but it did not work. Ask the mind what went wrong, or try it yourself.", lower_first(&doing))
    } else if lower == "ok" {
        format!("{}.", past(&doing))
    } else if lower.is_empty() || lower == "not reported" {
        format!("{}. Whether it worked was not reported.", past(&doing))
    } else {
        format!("{}.", past(&doing))
    };
    let status = if outcome.is_empty() { "not reported" } else { outcome };
    ActWords { sentence, failed, detail: format!("{app}.{action} · {status}") }
}

/// The first clause of a description, as an imperative ("Set the machine's output volume"), or
/// `None` when it is not one a person can read on its own: it carries a code word, or does not
/// open with a verb.
pub fn clause(description: &str) -> Option<String> {
    let text = one_line(description);
    let end = [": ", "; ", ". ", " - ", " \u{2014} ", " ("]
        .iter()
        .filter_map(|sep| text.find(sep))
        .min()
        .unwrap_or(text.len());
    let first = text[..end].trim().trim_end_matches('.').trim();
    let verb = first.split_whitespace().next()?.to_ascii_lowercase();
    let plain = !first.is_empty()
        && first.len() <= 80
        && !first.contains(['`', '_', '{', '<', '/', '#'])
        && is_verb(&verb);
    plain.then(|| first.to_string())
}

/// Verbs this module conjugates: the irregular ones below and these. A description or an action
/// name that does not open with one is not made a sentence by guessing at its first word ("Asked
/// by…" would become "Askeded", `chat_view` "Chated a view").
const REGULAR: &[&str] = &[
    "accept", "add", "answer", "apply", "approve", "archive", "ask", "assign", "attach", "cancel",
    "change", "check", "clear", "click", "close", "configure", "confirm", "connect", "convert", "copy",
    "create", "crop", "decide", "delete", "deny", "disable", "disconnect", "download", "edit", "empty",
    "enable", "enter", "export", "fetch", "fill", "flag", "focus", "follow", "forward", "generate",
    "import", "install", "kill", "launch", "list", "load", "lock", "loosen", "mark", "maximise",
    "maximize", "merge", "minimise", "minimize", "move", "mute", "name", "open", "paste", "pause",
    "pick", "play", "post", "power", "press", "print", "publish", "reboot", "record", "refresh",
    "reject", "release", "reload", "remove", "rename", "render", "reply", "report", "request",
    "restart", "restore", "resume", "return", "revert", "rotate", "save", "schedule", "scroll",
    "search", "select", "share", "sort", "start", "suspend", "switch", "sync", "tidy", "tighten",
    "toggle", "trash", "type", "uninstall", "unlock", "unmute", "update", "upload", "use", "verify",
    "view", "wait", "watch", "turn", "erase", "fold", "unfold", "hibernate",
];

fn is_verb(word: &str) -> bool {
    let w = word.to_ascii_lowercase();
    REGULAR.contains(&w.as_str()) || IRREGULAR.iter().any(|(v, _)| *v == w)
}

/// An imperative made from an action's name: `run` in Terminal → "run something in Terminal",
/// `update_event` in Calendar → "update an event in Calendar".
fn from_name(app: &str, action: &str) -> String {
    let words: Vec<String> = action.split(['_', '-', '.']).filter(|w| !w.is_empty()).map(str::to_ascii_lowercase).collect();
    let place = app_name(app);
    let Some((verb, rest)) = words.split_first() else {
        return format!("use {place}");
    };
    let on = if app.trim().is_empty() || app == "shell" { "on the desktop".to_string() } else { format!("in {place}") };
    if !is_verb(verb) {
        // A name that does not open with a verb ("chat_view", "reach_of"): said as what was used.
        return format!("use {} {on}", words.join(" "));
    }
    let object = if rest.is_empty() {
        "something".to_string()
    } else {
        let thing = rest.join(" ");
        let article = if thing.starts_with(['a', 'e', 'i', 'o', 'u']) { "an" } else { "a" };
        format!("{article} {thing}")
    };
    format!("{verb} {object} {on}")
}

/// An app's id as a name: "system-monitor" → "System Monitor".
fn app_name(app: &str) -> String {
    let words: Vec<String> = app.split(['-', '_', '.']).filter(|w| !w.is_empty()).map(|w| capital(&w.to_ascii_lowercase())).collect();
    if words.is_empty() { "an app".to_string() } else { words.join(" ") }
}

/// The imperative's first word in the past: "Set the volume" → "Set the volume", "Run a command"
/// → "Ran a command", "Update an event" → "Updated an event".
pub fn past(imperative: &str) -> String {
    // "Mute or unmute the speaker" → "Muted or unmuted the speaker": a verb joined to the first
    // by "or", "and" or a comma is in the past too.
    let mut out: Vec<String> = Vec::new();
    let mut joined = true;
    for (i, w) in imperative.split(' ').enumerate() {
        let bare = w.trim_end_matches(',');
        let next = if i == 0 || (joined && is_verb(bare)) {
            let lower = bare.to_ascii_lowercase();
            let p = IRREGULAR.iter().find(|(v, _)| *v == lower).map(|(_, p)| p.to_string()).unwrap_or_else(|| regular_past(&lower));
            let p = if i == 0 { capital(&p) } else { p };
            format!("{p}{}", &w[bare.len()..])
        } else {
            w.to_string()
        };
        joined = matches!(bare, "or" | "and") || w.ends_with(',');
        out.push(next);
    }
    out.join(" ")
}

const IRREGULAR: &[(&str, &str)] = &[
    ("run", "ran"), ("set", "set"), ("put", "put"), ("read", "read"), ("send", "sent"), ("make", "made"),
    ("find", "found"), ("write", "wrote"), ("take", "took"), ("give", "gave"), ("choose", "chose"),
    ("get", "got"), ("begin", "began"), ("build", "built"), ("bring", "brought"), ("keep", "kept"),
    ("leave", "left"), ("let", "let"), ("cut", "cut"), ("shut", "shut"), ("hide", "hid"), ("show", "showed"),
    ("draw", "drew"), ("speak", "spoke"), ("tell", "told"), ("say", "said"), ("pay", "paid"), ("lay", "laid"),
    ("go", "went"), ("do", "did"), ("undo", "undid"), ("redo", "redid"), ("rerun", "reran"), ("reset", "reset"),
    ("hold", "held"), ("feed", "fed"), ("buy", "bought"), ("sell", "sold"), ("spend", "spent"), ("swap", "swapped"),
    ("stop", "stopped"), ("drop", "dropped"), ("plan", "planned"), ("pin", "pinned"), ("unpin", "unpinned"),
    ("tag", "tagged"), ("log", "logged"), ("snap", "snapped"), ("zip", "zipped"), ("unzip", "unzipped"),
    ("submit", "submitted"), ("commit", "committed"), ("admit", "admitted"), ("drag", "dragged"), ("scan", "scanned"),
    ("ping", "pinged"), ("wake", "woke"), ("forget", "forgot"), ("rescan", "rescanned"),
    ("shop", "shopped"), ("ship", "shipped"), ("strip", "stripped"), ("trim", "trimmed"),
];

fn regular_past(verb: &str) -> String {
    if verb.ends_with('e') {
        format!("{verb}d")
    } else if verb.ends_with('y') && !["ay", "ey", "oy", "uy"].iter().any(|v| verb.ends_with(v)) {
        format!("{}ied", &verb[..verb.len() - 1])
    } else {
        format!("{verb}ed")
    }
}

fn lower_first(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        // Keep a proper noun's capital: only the opening verb is lowered.
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_model_id_is_said_by_its_plain_name_and_the_raw_id_never_reaches_it() {
        // The punch list's status line, 12 October, and the other spellings VM 520 carried.
        for raw in [
            "ollama-cloud:deepseek-v4-flash",
            "ollamacloud/deepseek-v4-flash · pi 0.87.0",
            "deepseek-v4-flash · ollama.com",
        ] {
            let plain = plain_model_name(raw);
            assert_eq!(plain, "DeepSeek V4 Flash, on Ollama Cloud", "{raw}");
        }
        assert_eq!(plain_model_name("ollama-cloud:deepseek-v4.1-flash"), "DeepSeek V4.1 Flash, on Ollama Cloud");
        assert_eq!(plain_model_name("qwen3.5:9b"), "Qwen 3.5 9B");
        assert_eq!(plain_model_name("nvidia/nemotron-3-super-120b-a12b"), "Nemotron 3 Super 120B A12B");
        assert_eq!(plain_model_name("gpt-oss:20b"), "GPT OSS 20B");
        assert_eq!(plain_model_name("http://192.168.4.35:11434"), "a model on this network");
        assert_eq!(plain_model_name("https://aig.mycluster.cyou/v1"), "a model on your own server");
        assert_eq!(plain_model_name(""), "");

        // Whatever the id, nothing of its spelling is left: no colon, no slash, no hyphenated
        // run, no host, no lower-case provider id.
        for raw in [
            "ollama-cloud:deepseek-v4-flash",
            "ollama-cloud:deeps",
            "openrouter/anthropic/claude-sonnet-5",
            "llama.cpp:Qwen3-8B-GGUF",
            "deepseek-v4-flash · ollama.com",
            "gpt-6 · aig.mycluster.cyou",
            "nvidia-nim:nvidia/nemotron-3-super-120b-a12b",
            "hf.co/unsloth/gemma-3-27b-it-GGUF:Q4_K_M",
        ] {
            let plain = plain_model_name(raw);
            assert!(!plain.is_empty(), "{raw}");
            assert!(!plain.contains([':', '/', '_']), "{raw} → {plain}");
            assert!(!plain.contains("ollama-cloud") && !plain.contains("ollama.com") && !plain.contains("mycluster"), "{raw} → {plain}");
            assert!(!plain.split_whitespace().any(|w| w.contains('-')), "{raw} → {plain}");
            assert!(!plain.contains(raw), "{raw} → {plain}");
        }
    }

    #[test]
    fn an_action_is_a_whole_sentence_with_what_happened_when_it_failed() {
        // The punch list's rows, 12 October.
        let run = act_words("terminal", "run", "failed", None);
        assert_eq!(run.sentence, "Tried to run something in Terminal, but it did not work. Ask the mind what went wrong, or try it yourself.");
        assert!(run.failed);
        assert_eq!(run.detail, "terminal.run · failed");
        assert_eq!(act_words("calendar", "update_event", "ok", None).sentence, "Updated an event in Calendar.");
        assert_eq!(act_words("blender", "render", "ok", None).sentence, "Rendered something in Blender.");
        assert_eq!(act_words("files", "move", "", None).sentence, "Moved something in Files. Whether it worked was not reported.");
        assert_eq!(act_words("files", "move", "", None).detail, "files.move · not reported");
        // From the action's own description, when the surface is in reach.
        let volume = act_words("shell", "set_volume", "ok", Some("Set the machine's output volume on the default speaker: `level` goes to 0 to 100"));
        assert_eq!(volume.sentence, "Set the machine's output volume on the default speaker.");
        let pictures = act_words("studio", "set_backend", "failed", Some("Choose where pictures are made"));
        assert_eq!(pictures.sentence, "Tried to choose where pictures are made, but it did not work. Ask the mind what went wrong, or try it yourself.");
        // A description that does not open as a plain instruction is not conjugated.
        assert_eq!(act_words("shell", "open_lens", "ok", Some("The `lens` overlay, opened")).sentence, "Opened a lens on the desktop.");
        assert_eq!(act_words("shell", "memory_validate", "ok", Some("Asked by the person's memory server")).sentence, "Used memory validate on the desktop.");
        assert_eq!(past("Mute or unmute the machine's default speaker"), "Muted or unmuted the machine's default speaker");
        assert_eq!(past("Download, verify and install a newer build"), "Downloaded, verified and installed a newer build");
        assert_eq!(past("Switch between dark and light"), "Switched between dark and light");
        assert_eq!(past("Rescan the installed applications"), "Rescanned the installed applications");
    }

    /// Every action the shell publishes, through its own description: each gets a sentence a
    /// person can read, with no code word in it. Read from the sources the surface is built from,
    /// the way `control_levels_actions` checks its own.
    #[test]
    fn every_action_the_shell_describes_gets_a_plain_sentence() {
        let actions = shell_actions();
        assert!(actions.len() > 60, "found the shell's actions ({})", actions.len());
        for (name, description) in &actions {
            for outcome in ["ok", "failed", ""] {
                let words = act_words("shell", name, outcome, Some(description));
                let s = &words.sentence;
                assert!(s.starts_with(|c: char| c.is_ascii_uppercase()), "{name}: {s}");
                assert!(s.ends_with('.'), "{name}: {s}");
                assert!(!s.contains(['`', '_', '{']), "{name}: {s}");
                assert!(!s.contains(name.as_str()) || !name.contains('_'), "{name}: {s}");
            }
        }
    }

    /// `(name, description)` for every `Action::new("name", "description…")` in the shell's
    /// control sources.
    fn shell_actions() -> Vec<(String, String)> {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut found = Vec::new();
        let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).collect();
        files.sort();
        for path in files {
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            if !(name.starts_with("control") && name.ends_with(".rs")) {
                continue;
            }
            let src = std::fs::read_to_string(&path).unwrap();
            let src = src.split("#[cfg(test)]").next().unwrap();
            let mut rest = src;
            while let Some(at) = rest.find("Action::new(") {
                rest = &rest[at + "Action::new(".len()..];
                let Some((action, after)) = literal(rest) else { continue };
                let after = after.trim_start().strip_prefix(',').unwrap_or(after);
                let Some((description, _)) = literal(after) else { continue };
                found.push((action, description));
            }
        }
        found
    }

    /// The string literal at the start of `text` (after whitespace), with `\`-newline joins.
    fn literal(text: &str) -> Option<(String, &str)> {
        let text = text.trim_start();
        let body = text.strip_prefix('"')?;
        let mut out = String::new();
        let mut chars = body.char_indices();
        while let Some((i, c)) = chars.next() {
            match c {
                '"' => return Some((out, &body[i + 1..])),
                '\\' => match chars.next() {
                    Some((_, '\n')) => {
                        while let Some((_, n)) = chars.clone().next() {
                            if !n.is_whitespace() { break; }
                            chars.next();
                        }
                    }
                    Some((_, 'n')) => out.push('\n'),
                    Some((_, other)) => out.push(other),
                    None => return None,
                },
                _ => out.push(c),
            }
        }
        None
    }
}
