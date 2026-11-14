//! What `run_command` shows of a listing: `find`, `ls` and `du` print where things are, so their
//! output is filtered, not only the place they were pointed at (#1247).
//!
//! The paths a command was given are checked before it runs ([`crate::safe_command::check`]); a
//! folder it walks into on its own is not. `find ~ -name '*.csv'` with a `tokens.json` in a backup
//! folder would still print that name, and the name says where a secret is kept. Every entry that
//! [`crate::safe_command::protected`] refuses is dropped here, and a count says that some were
//! left out, never which.

use std::path::Path;

/// `stdout` of `argv` (already checked and expanded) without the entries the AI never reads. Other
/// programs print no path they were not given and come back whole.
pub fn without_protected(argv: &[String], stdout: &str) -> String {
    let (kept, hidden) = match argv[0].as_str() {
        "find" => keep_lines(stdout, |line| Some(line)),
        "du" => keep_lines(stdout, |line| line.split_once('\t').map(|(_, path)| path)),
        "ls" => return ls(argv, stdout),
        _ => return stdout.to_string(),
    };
    with_count(kept, hidden)
}

/// The lines whose path (`path_of` finds it; none means the line holds no path) is not protected.
fn keep_lines<'a>(stdout: &'a str, path_of: impl Fn(&'a str) -> Option<&'a str>) -> (Vec<&'a str>, usize) {
    let mut kept = Vec::new();
    let mut hidden = 0;
    for line in stdout.lines() {
        if path_of(line).is_some_and(crate::safe_command::protected) {
            hidden += 1;
        } else {
            kept.push(line);
        }
    }
    (kept, hidden)
}

/// `ls`: a folder's name in a `dir:` header (`-R`, or several folders given), then its entries,
/// each as a bare name or the tail of a long line. A hidden folder's whole section goes with it.
fn ls(argv: &[String], stdout: &str) -> String {
    let asked: Vec<&String> = argv[1..].iter().filter(|a| !a.starts_with('-')).collect();
    let has_headers = asked.len() > 1 || argv.iter().any(|a| is_recursive(a));
    let mut dir = asked.first().map_or_else(|| ".".to_string(), |d| d.to_string());
    let mut in_hidden_dir = false;
    let (mut kept, mut hidden) = (Vec::new(), 0);
    for line in stdout.lines() {
        if let Some(header) = line.strip_suffix(':').filter(|_| has_headers) {
            dir = header.to_string();
            in_hidden_dir = crate::safe_command::protected(&dir);
        }
        if in_hidden_dir || names_a_protected_entry(line, &dir) {
            hidden += usize::from(!line.is_empty());
        } else {
            kept.push(line);
        }
    }
    with_count(kept, hidden)
}

/// `-R`, alone or in a cluster of letters (`-laR`), or spelled out.
fn is_recursive(arg: &str) -> bool {
    arg == "--recursive" || (arg.starts_with('-') && !arg.starts_with("--") && arg.contains('R'))
}

/// Whether the entry a line of `ls` prints (the whole line, or its tail after the date of a long
/// listing, and the target of a link) is protected inside `dir`.
fn names_a_protected_entry(line: &str, dir: &str) -> bool {
    let words: Vec<&str> = line.split_whitespace().collect();
    (0..words.len()).any(|from| {
        let tail = words[from..].join(" ");
        tail.split(" -> ").any(|name| crate::safe_command::protected(&Path::new(dir).join(name).to_string_lossy()))
    })
}

fn with_count(kept: Vec<&str>, hidden: usize) -> String {
    let mut out = kept.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    if hidden > 0 {
        out.push_str(&format!("({hidden} entries are not shown: each is, or is inside, a place or file the AI never reads)\n"));
    }
    out
}
