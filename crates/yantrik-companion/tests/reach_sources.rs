//! A tool that reaches the network must say so, however it reaches it (#1171).
//!
//! `reach_declared.rs` checks the table against the tools the model is known to reach for. This file
//! checks it against the sources, so that a tool nobody thought of is caught when it is written. It
//! does not look only at the text of a tool's own `impl Tool` block, which a tool passes by calling out:
//!
//! - **a helper in the same file**, outside the block;
//! - **a helper in another module or crate** (`crate::pipe::post_json`, `yantrik_chat::…`, an
//!   `yantrik_ml` client), by a path or by a name the file imported;
//! - **a subprocess**: `Command::new("wget")`, `pip3`, `xdg-open`, `git clone`, or `bwrap --share-net`.
//!
//! So it indexes every function outside a tool block, in the crates tools call into, and follows the
//! calls of each tool block a few levels down, looking for [`MARKERS`] and network programs. A tool
//! that does is declared non-local in `yantrik_companion_core::reach`, or listed in
//! [`LOCAL_DESPITE_MARKERS`] with the reason it is not.
//!
//! The reading is approximate on purpose (braces are counted, strings are not parsed; a call is
//! resolved by the module or crate it is written with, or the name the file imported), and errs
//! towards finding more: a function of the same name in the same module counts as the one called.
//! The tests at the bottom run it on small sources shaped like each way a tool can pass the old scan.

mod common;

use std::collections::{HashMap, HashSet};
use std::path::Path;

use common::{rust_files, tool_names, tools};
use yantrik_companion_core::reach::{reach, Reach};

/// What in code means "this drives the network or a browser".
const MARKERS: &[&str] = &[
    "ureq",
    "reqwest",
    "curl",
    "TcpStream",
    "UdpSocket",
    "http_get",
    "post_json",
    "run_with_stdin",
    "ensure_headless_browser",
    "browser_side",
    "connect_first_tab",
    "cdp_send",
    "searxng",
    "duckduckgo",
    "--share-net",
    "pip3",
    "npm",
    "xdg-open",
];

/// Programs that fetch, or install, or open a URL, when a tool starts one with `Command::new("…")`.
const NETWORK_PROGRAMS: &[&str] = &[
    "curl", "wget", "ssh", "scp", "rsync", "nc", "ncat", "socat", "telnet", "ftp", "pip", "pip3", "npm", "gem", "yt-dlp",
    "flatpak", "xdg-open", "claude", "apt", "apt-get", "nmap", "ping", "traceroute", "dig", "nslookup", "chromium",
];

/// `git` is local until it is given one of these.
const GIT_NETWORK_WORDS: &[&str] = &["clone", "fetch", "pull", "push", "ls-remote", "submodule"];

/// Tools whose code holds a marker and which are `Local` for the arguments the test gives, and why.
/// Everything else with a marker must be declared in `yantrik_companion_core::reach`.
const LOCAL_DESPITE_MARKERS: &[(&str, &str)] = &[
    ("code_execute", "Local in the strict sandbox; its reach reads `sandbox`, and reach_declared.rs pins the rest"),
    ("script_run", "Local in the strict sandbox; its reach reads `sandbox`, and reach_declared.rs pins the rest"),
    ("run_command", "runs only the programs on safe_command's list; `curl` is in the deny-list it checks against"),
    ("clipboard_history", "`npm_` and `curl` are in the sensitive-text patterns it reads for; it sends nothing"),
    ("clipboard_analyze", "the same patterns as clipboard_history"),
    ("browser_cleanup", "reads a running browser's local debugging port to close it; loads no page"),
    ("browser_status", "reads a running browser's local debugging port to report on it; loads no page"),
    ("extract_search_results", "sends the page text it was handed to the person's own model, not to the web"),
];

/// How many calls deep a tool's helpers are followed.
const DEPTH: usize = 4;

/// The crates whose functions a tool can call into (directories under `crates/`).
const CRATES: &[&str] = &[
    "yantrik-companion-tools",
    "yantrik-companion",
    "yantrik-companion-core",
    "yantrik-companion-cortex",
    "yantrik-chat",
    "yantrik-ml",
    "yantrik-egress",
    "yantrik-mcp",
    "yantrik-browser-ports",
];

struct Source {
    krate: String,
    module: String,
    code: String,
}

struct Helper {
    krate: String,
    module: String,
    body: String,
}

#[derive(Debug)]
struct Finding {
    tool: String,
    marker: String,
    via: String,
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn trailing_ident(s: &str) -> &str {
    let start = s.char_indices().rev().take_while(|(_, c)| is_ident(*c)).last().map_or(s.len(), |(i, _)| i);
    &s[start..]
}

/// The end of the `{ … }` that opens at `open`, counting braces.
fn matching_end(code: &str, open: usize) -> usize {
    let mut depth = 0usize;
    for (i, c) in code[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return open + i + 1;
                }
            }
            _ => {}
        }
    }
    code.len()
}

/// The `impl Tool for` blocks of a file, and the rest of it.
fn tool_blocks(code: &str) -> (Vec<&str>, String) {
    let (mut blocks, mut outside, mut at) = (Vec::new(), String::new(), 0);
    while let Some(found) = code[at..].find("impl Tool for ") {
        let start = at + found;
        let Some(open) = code[start..].find('{').map(|i| start + i) else { break };
        let end = matching_end(code, open);
        outside.push_str(&code[at..start]);
        blocks.push(&code[start..end]);
        at = end;
    }
    outside.push_str(&code[at..]);
    (blocks, outside)
}

/// Every `fn name … { body }` in `code` (nested ones too): the name and the braces.
fn functions(code: &str) -> Vec<(String, &str)> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(found) = code[from..].find("fn ") {
        let at = from + found;
        from = at + 3;
        if code[..at].chars().next_back().is_some_and(is_ident) {
            continue;
        }
        let name: String = code[at + 3..].chars().take_while(|c| is_ident(*c)).collect();
        let mut depth = 0i32;
        let mut open = None;
        for (i, c) in code[at..].char_indices() {
            match c {
                '(' | '[' => depth += 1,
                ')' | ']' => depth -= 1,
                ';' if depth <= 0 => break,
                '{' if depth <= 0 => {
                    open = Some(at + i);
                    break;
                }
                _ => {}
            }
        }
        if let (false, Some(open)) = (name.is_empty(), open) {
            out.push((name, &code[open..matching_end(code, open)]));
        }
    }
    out
}

/// The calls in `text`: the module, crate or type written before the name (none for a bare call or a
/// method), and the name.
fn calls(text: &str) -> Vec<(Option<String>, String)> {
    let mut out = Vec::new();
    for (at, _) in text.match_indices('(') {
        let before = &text[..at];
        let name = trailing_ident(before);
        if name.is_empty() || before[..before.len() - name.len()].ends_with("fn ") {
            continue;
        }
        let head = &before[..before.len() - name.len()];
        let qualifier = head.strip_suffix("::").map(|path| trailing_ident(path).to_string()).filter(|q| !q.is_empty());
        out.push((qualifier, name.to_string()));
    }
    out
}

/// `use a::b::{c, d};` and `use a::b::c;` give `c -> b`, `d -> b`: the module a bare name was imported from.
fn imports(code: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for (at, _) in code.match_indices("use ") {
        let line_start = code[..at].rfind('\n').map_or(0, |i| i + 1);
        let before = code[line_start..at].trim();
        if !(before.is_empty() || before.starts_with("pub")) {
            continue;
        }
        let path = code[at + 4..].split(';').next().unwrap_or("").trim();
        if path.len() > 400 {
            continue;
        }
        if let Some((prefix, items)) = path.split_once('{') {
            let module = trailing_ident(prefix.trim_end_matches("::"));
            for item in items.trim_end_matches('}').split(',') {
                let name = item.split(" as ").next().unwrap_or("").trim();
                if !name.is_empty() && !module.is_empty() && name.chars().all(is_ident) {
                    out.insert(name.to_string(), module.to_string());
                }
            }
        } else if let Some((module_path, name)) = path.rsplit_once("::") {
            let name = name.split(" as ").next().unwrap_or("").trim();
            out.insert(name.to_string(), trailing_ident(module_path).to_string());
        }
    }
    out
}

struct Index {
    helpers: HashMap<String, Vec<Helper>>,
    imports: HashMap<(String, String), HashMap<String, String>>,
}

impl Index {
    fn new(sources: &[Source]) -> Self {
        let mut index = Index { helpers: HashMap::new(), imports: HashMap::new() };
        for source in sources {
            let (_, outside) = tool_blocks(&source.code);
            for (name, body) in functions(&outside) {
                index.helpers.entry(name).or_default().push(Helper {
                    krate: source.krate.clone(),
                    module: source.module.clone(),
                    body: body.to_string(),
                });
            }
            index.imports.entry((source.krate.clone(), source.module.clone())).or_default().extend(imports(&source.code));
        }
        index
    }

    /// The helpers a call written in `krate`/`module` can be.
    fn resolve(&self, krate: &str, module: &str, qualifier: Option<&str>, name: &str) -> Vec<&Helper> {
        let imported = self.imports.get(&(krate.to_string(), module.to_string())).and_then(|m| m.get(name));
        self.helpers.get(name).into_iter().flatten().filter(|h| match qualifier {
            None | Some("self" | "Self") => (h.krate == krate && h.module == module) || imported.is_some_and(|m| *m == h.module),
            Some("crate" | "super") => h.krate == krate,
            Some(q) if q.starts_with("yantrik") => h.krate == q.replace('_', "-"),
            Some(q) => h.module == q,
        }).collect()
    }
}

/// What in `text` itself says it reaches the network.
fn marker_in(text: &str) -> Option<String> {
    if let Some(marker) = MARKERS.iter().find(|m| text.contains(**m)) {
        return Some((*marker).to_string());
    }
    const START: &str = "Command::new(\"";
    for (at, _) in text.match_indices(START) {
        let program = text[at + START.len()..].split('"').next().unwrap_or("");
        if NETWORK_PROGRAMS.contains(&program) {
            return Some(format!("Command::new(\"{program}\")"));
        }
        if program == "git" && GIT_NETWORK_WORDS.iter().any(|w| text.contains(&format!("\"{w}\""))) {
            return Some("git with a network subcommand".into());
        }
    }
    None
}

/// The marker `text` holds or a helper it calls (to `depth` levels) holds, and the helpers on the way.
fn walk(index: &Index, at: (&str, &str), text: &str, depth: usize, seen: &mut HashSet<usize>, via: &mut Vec<String>) -> Option<String> {
    if let Some(marker) = marker_in(text) {
        return Some(marker);
    }
    if depth == 0 {
        return None;
    }
    for (qualifier, name) in calls(text) {
        for helper in index.resolve(at.0, at.1, qualifier.as_deref(), &name) {
            if !seen.insert(helper as *const Helper as usize) {
                continue;
            }
            via.push(format!("{}::{name}", helper.module));
            if let Some(marker) = walk(index, (&helper.krate, &helper.module), &helper.body, depth - 1, seen, via) {
                return Some(marker);
            }
            via.pop();
        }
    }
    None
}

/// Every tool whose block, or a helper it calls, reaches the network by a marker.
fn offenders(sources: &[Source]) -> Vec<Finding> {
    let index = Index::new(sources);
    let mut found = Vec::new();
    for source in sources {
        for block in tool_blocks(&source.code).0 {
            let (mut seen, mut via) = (HashSet::new(), Vec::new());
            let Some(marker) = walk(&index, (&source.krate, &source.module), block, DEPTH, &mut seen, &mut via) else { continue };
            for tool in tool_names(block) {
                found.push(Finding { tool, marker: marker.clone(), via: via.join(" -> ") });
            }
        }
    }
    found
}

fn load_sources() -> Vec<Source> {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut sources = Vec::new();
    for krate in CRATES {
        let mut files = Vec::new();
        rust_files(&crates.join(krate).join("src"), &mut files);
        for file in files {
            let text = std::fs::read_to_string(&file).unwrap();
            let stem = file.file_stem().unwrap().to_string_lossy().to_string();
            let module = if stem == "mod" { file.parent().unwrap().file_name().unwrap().to_string_lossy().to_string() } else { stem };
            let code = text.split("#[cfg(test)]").next().unwrap().to_string();
            sources.push(Source { krate: (*krate).to_string(), module, code });
        }
    }
    sources
}

#[test]
fn every_tool_that_reaches_out_by_any_route_is_declared() {
    let registered = tools();
    let (mut checked, mut undeclared) = (0, Vec::new());
    for Finding { tool, marker, via } in offenders(&load_sources()) {
        let Some((_, category)) = registered.iter().find(|(n, _)| *n == tool) else { continue };
        checked += 1;
        let declared = reach(&tool, category, &serde_json::json!({})) != Reach::Local;
        if !declared && !LOCAL_DESPITE_MARKERS.iter().any(|(n, _)| *n == tool) {
            let route = if via.is_empty() { "its own code".to_string() } else { format!("a helper it calls ({via})") };
            undeclared.push(format!("`{tool}` reaches out through {route}, by `{marker}`"));
        }
    }
    assert!(
        undeclared.is_empty(),
        "declared Local, and not listed in LOCAL_DESPITE_MARKERS with a reason: declare what each reaches in          yantrik_companion_core::reach.
{}",
        undeclared.join("
")
    );
    assert!(checked > 25, "the scan found {checked} tools that reach out: it is not reading the sources");
}

#[test]
fn the_reviewed_local_tools_are_registered_and_local() {
    for (name, _) in LOCAL_DESPITE_MARKERS {
        let (_, category) = tools().into_iter().find(|(n, _)| n == name).unwrap_or_else(|| panic!("{name} is not a registered tool"));
        assert_eq!(reach(name, category, &serde_json::json!({})), Reach::Local, "{name}");
    }
}

/// A tool server's tools are added at run time, so the scan never sees them: they are held by category.
#[test]
fn tools_added_at_run_time_are_held_by_category_not_by_name() {
    for name in ["mcp__anything_at_all", "mcp__fetch_fetch"] {
        assert_ne!(reach(name, "mcp", &serde_json::json!({})), Reach::Local, "{name}");
    }
}

// ── The scan itself: each way a tool passed the round-4 test, as a small source ──

fn source(krate: &str, module: &str, code: &str) -> Source {
    Source { krate: krate.into(), module: module.into(), code: code.into() }
}

fn tool(name: &str, body: &str) -> String {
    format!("impl Tool for T {{ fn name(&self) -> &'static str {{ \"{name}\" }} fn execute(&self) -> String {{ {body} }} }}")
}

fn flagged(sources: &[Source]) -> Vec<String> {
    offenders(sources).into_iter().map(|f| f.tool).collect()
}

#[test]
fn a_subprocess_that_is_not_a_listed_word_is_caught() {
    for program in ["wget", "yt-dlp", "xdg-open", "flatpak", "pip3", "npm"] {
        let code = tool("t", &format!("std::process::Command::new(\"{program}\").arg(\"x\").output()"));
        assert_eq!(flagged(&[source("a", "m", &code)]), ["t"], "{program}");
    }
    let clone = tool("t", "Command::new(\"git\").args([\"clone\", url]).output()");
    assert_eq!(flagged(&[source("a", "m", &clone)]), ["t"], "git clone");
    let bwrap = tool("t", "Command::new(\"bwrap\").arg(\"--share-net\").output()");
    assert_eq!(flagged(&[source("a", "m", &bwrap)]), ["t"], "bwrap --share-net");
}

#[test]
fn local_subprocesses_are_not_flagged() {
    for body in [
        "Command::new(\"ls\").output()",
        "Command::new(\"git\").args([\"status\"]).output()",
        "std::fs::read_to_string(path)",
        "Command::new(program).output()",
    ] {
        assert!(flagged(&[source("a", "m", &tool("t", body))]).is_empty(), "{body}");
    }
}

#[test]
fn a_helper_in_the_same_file_outside_the_block_is_followed() {
    let code = format!("fn fetch_it() {{ Command::new(\"curl\").output(); }}\n{}", tool("t", "fetch_it()"));
    assert_eq!(flagged(&[source("a", "m", &code)]), ["t"]);
    // Two levels down.
    let code = format!("fn outer() {{ inner() }} fn inner() {{ Command::new(\"wget\").output(); }}\n{}", tool("t", "self.outer()"));
    assert_eq!(flagged(&[source("a", "m", &code)]), ["t"]);
}

#[test]
fn a_helper_in_another_module_is_followed_by_path_or_by_import() {
    // The canvas shape: `crate::pipe::post_json`, where the curl lives in `pipe.rs`.
    let pipe = source("tools", "pipe", "pub fn post_json(url: &str) -> String { Command::new(\"curl\").output() }");
    let by_path = source("tools", "canvas", &tool("t", "crate::pipe::post_json(url)"));
    assert_eq!(flagged(&[pipe, by_path]), ["t"]);
    let pipe = source("tools", "pipe", "pub fn send(url: &str) -> String { Command::new(\"curl\").output() }");
    let by_import = source("tools", "canvas", &format!("use crate::pipe::{{send, other}};\n{}", tool("t", "send(url)")));
    assert_eq!(flagged(&[pipe, by_import]), ["t"], "a bare call of an imported name");
}

#[test]
fn a_helper_in_another_crate_is_followed() {
    let chat = source("yantrik-chat", "telegram", "pub fn send_message(text: &str) { ureq::post(url).send_string(text); }");
    let tool_file = source("yantrik-companion", "notes", &tool("t", "yantrik_chat::telegram::send_message(text)"));
    assert_eq!(flagged(&[chat, tool_file]), ["t"]);
    let ml = source("yantrik-ml", "client", "pub fn complete(p: &str) -> String { reqwest::blocking::get(p) }");
    let tool_file = source("yantrik-companion", "notes", &tool("t", "yantrik_ml::complete(prompt)"));
    assert_eq!(flagged(&[ml, tool_file]), ["t"]);
}

#[test]
fn the_runners_bubblewrap_helper_is_caught() {
    let sandbox = source("tools", "coder_sandbox", "pub fn build_bwrap_args() { args.push(\"--share-net\".into()); }");
    let coder = source("tools", "coder", &tool("code_execute", "coder_sandbox::build_bwrap_args()"));
    assert_eq!(flagged(&[sandbox, coder]), ["code_execute"]);
}

#[test]
fn a_call_of_the_same_name_in_an_unrelated_module_is_not_followed() {
    let net = source("tools", "net", "pub fn read(url: &str) -> String { Command::new(\"curl\").output() }");
    let files = source("tools", "files", &tool("t", "files::read(path)"));
    assert!(flagged(&[net, files]).is_empty());
}

/// Where the person lives, which an unattended weather lookup may name, comes only from the location
/// they set in `user_preferences`, never from memory text (an unattended `remember` can write any).
#[test]
fn the_persons_location_is_never_read_from_memory_text() {
    let src = include_str!("../src/companion.rs");
    let body = src.split("fn load_user_location(").nth(1).expect("load_user_location").split("\n}\n").next().unwrap();
    assert!(body.contains("FROM user_preferences WHERE category = 'location'"), "{body}");
    assert!(!body.contains("memories"), "the location is not read from memories: {body}");
}
