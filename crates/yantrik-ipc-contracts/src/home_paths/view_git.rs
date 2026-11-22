//! The git settings a command's view carries (#1265, round 6).
//!
//! `~/.gitconfig` and `~/.config/git/config` hold a person's name and editor, and also their
//! tokens (a URL rewrite with a token in it, an `extraheader`, a `credential.helper`). So the view
//! never binds either file. It is given a **synthesized** config built from a list of the keys that
//! may be shown, read from both files: a key not on the list is absent, a section that is not on it
//! is absent (`credential.*`, `http.*`, `url.*`, `include*`, `core.sshCommand`).
//!
//! The synthesized config reaches git as `GIT_CONFIG_COUNT` / `GIT_CONFIG_KEY_n` /
//! `GIT_CONFIG_VALUE_n` (`bwrap --setenv`), git's own way to be given settings with no file, so
//! nothing of it is written anywhere a command could find it, and all three doors that start a
//! view (a job, the Terminal tab, a companion tool) get it from the same words.
//!
//! The words also **unset** the environment that would point git at other files or programs
//! (`GIT_*`, `XDG_CONFIG_HOME`) before the settings are set, so a variable the shell was started
//! with does not reach a command through the doors that pass the shell's environment on.
//! `/etc/gitconfig` is on the host and still applies, as does a repository's own `.git/config`.

use std::path::Path;

/// The most settings carried, and the longest value; more is dropped, not cut.
const MAX_KEYS: usize = 200;
const MAX_VALUE: usize = 1024;
const MAX_FILE: u64 = 1 << 20;

/// Whether a setting may be shown. A list of what may be shown: the sections and keys named here,
/// nothing else. `diff.*` is all of it but what runs a program. `alias.*` is not on it at all: an
/// alias can carry a global option (`-c key=value`) or a URL with a credential, every value shown is
/// also a word of the `bwrap` command line that any local account can read, and a build needs none.
fn may_be_shown(section: &str, sub: Option<&str>, key: &str) -> bool {
    match (section, sub) {
        ("user", None) => matches!(key, "name" | "email"),
        ("init", None) => key == "defaultbranch",
        ("core", None) => matches!(key, "autocrlf" | "eol" | "editor"),
        ("pull", None) => matches!(key, "rebase" | "ff"),
        ("merge", None) => key == "conflictstyle",
        ("color", _) => true,
        ("diff", None) => !matches!(key, "external"),
        ("diff", Some(_)) => !matches!(key, "textconv" | "command"),
        _ => false,
    }
}

/// A value as git reads it: quotes, the escapes `\n \t \b \" \\`, and a comment outside quotes.
/// `None` for one that is not well formed.
fn unquote(raw: &str) -> Option<String> {
    let mut out: Vec<(char, bool)> = Vec::new();
    let (mut quoted, mut chars) = (false, raw.chars());
    while let Some(c) = chars.next() {
        match c {
            '"' => quoted = !quoted,
            '\\' => out.push((
                match chars.next()? {
                    'n' => '\n',
                    't' => '\t',
                    'b' => '\u{8}',
                    e @ ('"' | '\\') => e,
                    _ => return None,
                },
                true,
            )),
            '#' | ';' if !quoted => break,
            c => out.push((c, quoted)),
        }
    }
    while out.last().is_some_and(|(c, q)| !q && c.is_whitespace()) {
        out.pop();
    }
    (!quoted).then(|| out.into_iter().map(|(c, _)| c).collect())
}

/// `[section]`, `[section "sub"]` or `[section.sub]`: the section lower case, and the subsection.
fn header(inner: &str) -> Option<(String, Option<String>)> {
    match inner.split_once('"') {
        Some((section, rest)) => {
            let sub = rest.trim_end().strip_suffix('"')?;
            Some((section.trim().to_lowercase(), Some(sub.replace("\\\"", "\"").replace("\\\\", "\\"))))
        }
        None => match inner.trim().split_once('.') {
            Some((section, sub)) => Some((section.to_lowercase(), Some(sub.to_string()))),
            None => Some((inner.trim().to_lowercase(), None)),
        },
    }
}

/// The settings of one config file's text that may be shown, as (`section[.sub].key`, value).
/// A line that continues on the next, or is not well formed, is dropped.
pub fn shown_settings(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut at: Option<(String, Option<String>)> = None;
    let mut lines = text.lines();
    while let Some(first) = lines.next() {
        let mut line = first.trim().to_string();
        let mut continued = false;
        while line.ends_with('\\') && !line.ends_with("\\\\") {
            continued = true;
            line = lines.next().map(|n| n.trim().to_string()).unwrap_or_default();
        }
        let mut body = line.as_str();
        if let Some(rest) = body.strip_prefix('[') {
            // The closing bracket outside quotes.
            let mut quoted = false;
            let end = rest.char_indices().find(|&(_, c)| match c {
                '"' => {
                    quoted = !quoted;
                    false
                }
                ']' => !quoted,
                _ => false,
            });
            let Some((end, _)) = end else { at = None; continue };
            at = header(&rest[..end]);
            body = rest[end + 1..].trim();
        }
        if continued || body.is_empty() || body.starts_with(['#', ';']) {
            continue;
        }
        let Some((section, sub)) = &at else { continue };
        let name_end = body.find(|c: char| c.is_whitespace() || c == '=').unwrap_or(body.len());
        let (key, rest) = (body[..name_end].to_lowercase(), body[name_end..].trim_start());
        let valid = key.starts_with(|c: char| c.is_ascii_alphabetic()) && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
        let value = match rest.strip_prefix('=') {
            Some(v) => unquote(v.trim()),
            None if rest.is_empty() || rest.starts_with(['#', ';']) => Some("true".to_string()),
            None => None,
        };
        let Some(value) = value.filter(|v| valid && v.len() <= MAX_VALUE) else { continue };
        if may_be_shown(section, sub.as_deref(), &key) {
            let full = match sub {
                Some(s) => format!("{section}.{s}.{key}"),
                None => format!("{section}.{key}"),
            };
            out.push((full, value));
        }
    }
    out
}

fn read_capped(path: &Path) -> String {
    use std::io::Read;
    let mut text = String::new();
    if let Ok(f) = std::fs::File::open(path) {
        let _ = f.take(MAX_FILE).read_to_string(&mut text);
    }
    text
}

/// Git's variables that a job's own environment may carry, unset by name as well as every `GIT_*`
/// the shell has.
const GIT_VARS: &[&str] = &[
    "GIT_CONFIG_GLOBAL", "GIT_CONFIG_SYSTEM", "GIT_CONFIG_NOSYSTEM", "GIT_CONFIG", "GIT_CONFIG_COUNT", "GIT_CONFIG_PARAMETERS",
    "GIT_DIR", "GIT_WORK_TREE", "GIT_EXEC_PATH", "GIT_SSH", "GIT_SSH_COMMAND", "GIT_ASKPASS", "GIT_PROXY_COMMAND", "GIT_EXTERNAL_DIFF",
    "GIT_PAGER", "GIT_EDITOR", "GIT_SEQUENCE_EDITOR", "GIT_TEMPLATE_DIR", "GIT_ALTERNATE_OBJECT_DIRECTORIES", "GIT_OBJECT_DIRECTORY",
    "GIT_INDEX_FILE", "GIT_COMMON_DIR", "GIT_NAMESPACE", "GIT_CURL_VERBOSE", "GIT_TRACE", "XDG_CONFIG_HOME",
];

/// The names to unset: the list, and every `GIT_*` and `GIT_CONFIG_*` in this process's environment.
/// Sorted and without repeats, so the words are the same twice.
fn scrubbed_names(env: impl Iterator<Item = String>) -> Vec<String> {
    let mut names: Vec<String> = GIT_VARS.iter().map(|s| s.to_string()).chain(env.filter(|k| k.starts_with("GIT_"))).collect();
    names.sort();
    names.dedup();
    names
}

/// The `bwrap` words that scrub git's environment, then give the view the person's shown git
/// settings: the account-wide file first, then `~/.gitconfig` (which wins, as it does for git).
pub fn git_words(home: &Path) -> Vec<String> {
    let mut a: Vec<String> = scrubbed_names(std::env::vars_os().filter_map(|(k, _)| k.into_string().ok()))
        .into_iter()
        .flat_map(|name| ["--unsetenv".to_string(), name])
        .collect();
    a.extend(setting_words(home));
    a
}

fn setting_words(home: &Path) -> Vec<String> {
    let mut all = shown_settings(&read_capped(&home.join(".config/git/config")));
    all.extend(shown_settings(&read_capped(&home.join(".gitconfig"))));
    all.truncate(MAX_KEYS);
    if all.is_empty() {
        return Vec::new();
    }
    let mut a = vec!["--setenv".to_string(), "GIT_CONFIG_COUNT".to_string(), all.len().to_string()];
    for (i, (key, value)) in all.into_iter().enumerate() {
        a.extend(["--setenv".to_string(), format!("GIT_CONFIG_KEY_{i}"), key, "--setenv".to_string(), format!("GIT_CONFIG_VALUE_{i}"), value]);
    }
    a
}
