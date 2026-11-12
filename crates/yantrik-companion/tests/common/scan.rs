//! What the two source scans share: reading every crate's sources, finding the tool blocks and the
//! helpers they call, and following those calls. `reach_sources.rs` looks for network markers with it,
//! `reach_spawns.rs` for processes started by a computed name.
#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::path::Path;

use super::{rust_files, tool_names};

/// Programs that fetch, or install, or open a URL, when a tool starts one with `Command::new("…")`.
pub const NETWORK_PROGRAMS: &[&str] = &[
    "curl",
    "wget",
    "ssh",
    "scp",
    "rsync",
    "nc",
    "ncat",
    "socat",
    "telnet",
    "ftp",
    "pip",
    "pip3",
    "npm",
    "gem",
    "yt-dlp",
    "flatpak",
    "xdg-open",
    "claude",
    "apt",
    "apt-get",
    "nmap",
    "ping",
    "traceroute",
    "dig",
    "nslookup",
    "chromium",
];

pub const DEPTH: usize = 4;

/// The crates whose functions a tool can call into (directories under `crates/`).
pub const CRATES: &[&str] = &[
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

pub struct Source {
    pub krate: String,
    pub module: String,
    pub code: String,
}

pub struct Helper {
    pub krate: String,
    pub module: String,
    pub body: String,
}

#[derive(Debug)]
pub struct Finding {
    pub tool: String,
    pub marker: String,
    pub via: String,
}

pub fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

pub fn trailing_ident(s: &str) -> &str {
    let start = s
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_ident(*c))
        .last()
        .map_or(s.len(), |(i, _)| i);
    &s[start..]
}

/// The end of the `{ … }` that opens at `open`, counting braces.
pub fn matching_end(code: &str, open: usize) -> usize {
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
pub fn tool_blocks(code: &str) -> (Vec<&str>, String) {
    let (mut blocks, mut outside, mut at) = (Vec::new(), String::new(), 0);
    while let Some(found) = code[at..].find("impl Tool for ") {
        let start = at + found;
        let Some(open) = code[start..].find('{').map(|i| start + i) else {
            break;
        };
        let end = matching_end(code, open);
        outside.push_str(&code[at..start]);
        blocks.push(&code[start..end]);
        at = end;
    }
    outside.push_str(&code[at..]);
    (blocks, outside)
}

/// Every `fn name … { body }` in `code` (nested ones too): the name and the braces.
pub fn functions(code: &str) -> Vec<(String, &str)> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(found) = code[from..].find("fn ") {
        let at = from + found;
        from = at + 3;
        if code[..at].chars().next_back().is_some_and(is_ident) {
            continue;
        }
        let name: String = code[at + 3..]
            .chars()
            .take_while(|c| is_ident(*c))
            .collect();
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
pub fn calls(text: &str) -> Vec<(Option<String>, String)> {
    let mut out = Vec::new();
    for (at, _) in text.match_indices('(') {
        let before = &text[..at];
        let name = trailing_ident(before);
        if name.is_empty() || before[..before.len() - name.len()].ends_with("fn ") {
            continue;
        }
        let head = &before[..before.len() - name.len()];
        let qualifier = head
            .strip_suffix("::")
            .map(|path| trailing_ident(path).to_string())
            .filter(|q| !q.is_empty());
        out.push((qualifier, name.to_string()));
    }
    out
}

/// `use a::b::{c, d};` and `use a::b::c;` give `c -> b`, `d -> b`: the module a bare name was imported from.
pub fn imports(code: &str) -> HashMap<String, String> {
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

pub struct Index {
    pub helpers: HashMap<String, Vec<Helper>>,
    pub imports: HashMap<(String, String), HashMap<String, String>>,
}

impl Index {
    pub fn new(sources: &[Source]) -> Self {
        let mut index = Index {
            helpers: HashMap::new(),
            imports: HashMap::new(),
        };
        for source in sources {
            let (_, outside) = tool_blocks(&source.code);
            for (name, body) in functions(&outside) {
                index.helpers.entry(name).or_default().push(Helper {
                    krate: source.krate.clone(),
                    module: source.module.clone(),
                    body: body.to_string(),
                });
            }
            index
                .imports
                .entry((source.krate.clone(), source.module.clone()))
                .or_default()
                .extend(imports(&source.code));
        }
        index
    }

    /// The helpers a call written in `krate`/`module` can be.
    pub fn resolve(
        &self,
        krate: &str,
        module: &str,
        qualifier: Option<&str>,
        name: &str,
    ) -> Vec<&Helper> {
        let imported = self
            .imports
            .get(&(krate.to_string(), module.to_string()))
            .and_then(|m| m.get(name));
        self.helpers
            .get(name)
            .into_iter()
            .flatten()
            .filter(|h| match qualifier {
                None | Some("self" | "Self") => {
                    (h.krate == krate && h.module == module)
                        || imported.is_some_and(|m| *m == h.module)
                }
                Some("crate" | "super") => h.krate == krate,
                Some(q) if q.starts_with("yantrik") => h.krate == q.replace('_', "-"),
                Some(q) => h.module == q,
            })
            .collect()
    }
}

pub fn walk_with(
    detect: fn(&str) -> Option<String>,
    index: &Index,
    at: (&str, &str),
    text: &str,
    depth: usize,
    seen: &mut HashSet<usize>,
    via: &mut Vec<String>,
) -> Option<String> {
    if let Some(marker) = detect(text) {
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
            if let Some(marker) = walk_with(
                detect,
                index,
                (&helper.krate, &helper.module),
                &helper.body,
                depth - 1,
                seen,
                via,
            ) {
                return Some(marker);
            }
            via.pop();
        }
    }
    None
}

/// `code` without its `#[cfg(test)]` items: the attribute and the item after it (a `mod { … }` block, a
/// `fn`, a `use …;`), by brace matching. What follows an item is kept, so a tool that comes after a
/// `#[cfg(test)] mod fake {}` is still read.
pub fn without_test_items(code: &str) -> String {
    const ATTR: &str = "#[cfg(test)]";
    let mut out = String::new();
    let mut rest = code;
    while let Some(at) = rest.find(ATTR) {
        out.push_str(&rest[..at]);
        let after = &rest[at + ATTR.len()..];
        // The item ends at the first `;` before any `{` (a `use`, a `const`), else at its matching `}`.
        let end = match (after.find('{'), after.find(';')) {
            (Some(open), Some(semi)) if semi < open => semi + 1,
            (Some(open), _) => matching_end(after, open),
            (None, Some(semi)) => semi + 1,
            (None, None) => after.len(),
        };
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

/// Every source file of the scanned crates, test items removed.
pub fn load_sources() -> Vec<Source> {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut sources = Vec::new();
    for krate in CRATES {
        let mut files = Vec::new();
        rust_files(&crates.join(krate).join("src"), &mut files);
        for file in files {
            let text = std::fs::read_to_string(&file).unwrap();
            let stem = file.file_stem().unwrap().to_string_lossy().to_string();
            let module = if stem == "mod" {
                file.parent()
                    .unwrap()
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .to_string()
            } else {
                stem
            };
            sources.push(Source {
                krate: (*krate).to_string(),
                module,
                code: without_test_items(&text),
            });
        }
    }
    sources
}

/// Every source file's text, test items and all, with its path (for the whole-text guards).
pub fn raw_sources() -> Vec<(String, String)> {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut out = Vec::new();
    for krate in CRATES {
        let mut files = Vec::new();
        rust_files(&crates.join(krate).join("src"), &mut files);
        for file in files {
            out.push((
                file.display().to_string(),
                std::fs::read_to_string(&file).unwrap(),
            ));
        }
    }
    out
}

pub fn source(krate: &str, module: &str, code: &str) -> Source {
    Source {
        krate: krate.into(),
        module: module.into(),
        code: code.into(),
    }
}

pub fn tool(name: &str, body: &str) -> String {
    format!("impl Tool for T {{ fn name(&self) -> &'static str {{ \"{name}\" }} fn execute(&self) -> String {{ {body} }} }}")
}
