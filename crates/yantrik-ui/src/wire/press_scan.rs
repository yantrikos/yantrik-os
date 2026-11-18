//! A backstop for the rule that only the meeting brief can make the shell press "open this
//! message" in Email. The compiler holds it (`BriefMessage` has private fields and a private
//! constructor, and `app_act::press_message` is the one sender of `app.press`); these tests read
//! the source so that a change which goes around the types is seen.
//!
//! The shell runs as one process, so what is scanned is its own `src` and every workspace crate it
//! links, directly or through another: any of them sends as the shell.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Where an inline `#[cfg(test)] mod name {` block is in `text`: the byte range from the attribute
/// to the matching close brace. A `mod name;` declaration is not one, and nothing after it is cut.
fn test_blocks(text: &str) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(at) = text[from..].find("#[cfg(test)]").map(|i| i + from) {
        let mut rest = text[at + "#[cfg(test)]".len()..].trim_start();
        // Other attributes on the same item.
        while rest.starts_with("#[") {
            let Some(end) = rest.find(']') else { break };
            rest = rest[end + 1..].trim_start();
        }
        let header_end = rest.find(['{', ';']).unwrap_or(rest.len());
        let is_inline_mod = rest[header_end..].starts_with('{') && is_mod_header(rest[..header_end].trim());
        from = at + "#[cfg(test)]".len();
        if is_inline_mod {
            let open = text.len() - rest.len() + header_end;
            let close = matching_brace(text, open);
            found.push((at, close));
            from = close;
        }
    }
    found
}

/// Whether `header` (what is before the `{` of an item) is `mod name`, public or not.
fn is_mod_header(header: &str) -> bool {
    let mut h = header;
    if let Some(public) = h.strip_prefix("pub") {
        h = public.trim_start();
        if h.starts_with('(') {
            h = h.find(')').map_or("", |close| h[close + 1..].trim_start());
        }
    }
    h.strip_prefix("mod ").map(str::trim).is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
}

/// The index just past the `}` that closes the `{` at `open`: braces in strings, characters and
/// comments are not braces.
fn matching_brace(text: &str, open: usize) -> usize {
    let b = text.as_bytes();
    let (mut depth, mut i) = (0usize, open);
    while i < b.len() {
        match b[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'/') => i = text[i..].find('\n').map_or(b.len(), |n| i + n),
            b'/' if b.get(i + 1) == Some(&b'*') => i = skip_block_comment(b, i),
            b'"' => i = skip_string(b, i),
            b'r' if raw_string_hashes(b, i).is_some() => i = skip_raw_string(b, i),
            b'\'' => i = skip_char(b, i),
            _ => {}
        }
        i += 1;
    }
    b.len()
}

/// The index of the `/` that ends the (nested) block comment that starts at `i`.
fn skip_block_comment(b: &[u8], mut i: usize) -> usize {
    let mut depth = 0;
    while i + 1 < b.len() {
        match (b[i], b[i + 1]) {
            (b'/', b'*') => {
                depth += 1;
                i += 2;
            }
            (b'*', b'/') => {
                depth -= 1;
                i += 2;
                if depth == 0 {
                    return i - 1;
                }
            }
            _ => i += 1,
        }
    }
    b.len()
}

/// The index of the closing quote of the string that starts at `i`.
fn skip_string(b: &[u8], mut i: usize) -> usize {
    i += 1;
    while i < b.len() && b[i] != b'"' {
        i += if b[i] == b'\\' { 2 } else { 1 };
    }
    i
}

/// How many `#` a raw string that starts at `i` (`r"`, `r#"`) has, if one does, and `r` is not the
/// end of a longer word.
fn raw_string_hashes(b: &[u8], i: usize) -> Option<usize> {
    if i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_') {
        return None;
    }
    let hashes = b[i + 1..].iter().take_while(|&&c| c == b'#').count();
    (b.get(i + 1 + hashes) == Some(&b'"')).then_some(hashes)
}

fn skip_raw_string(b: &[u8], i: usize) -> usize {
    let hashes = raw_string_hashes(b, i).unwrap_or(0);
    let mut end = vec![b'"'];
    end.extend(std::iter::repeat(b'#').take(hashes));
    let body = i + 2 + hashes;
    b[body..].windows(end.len()).position(|w| w == end.as_slice()).map_or(b.len(), |n| body + n + end.len() - 1)
}

/// The index of the closing quote of a character literal that starts at `i`; a lifetime (`'a`) is
/// not one, and ends at its own `'`.
fn skip_char(b: &[u8], i: usize) -> usize {
    match (b.get(i + 1), b.get(i + 2)) {
        (Some(b'\\'), _) => b[i + 2..].iter().position(|&c| c == b'\'').map_or(b.len(), |n| i + 2 + n),
        (Some(_), Some(b'\'')) => i + 2,
        _ => i,
    }
}

/// `text` without its inline test modules.
fn without_test_modules(text: &str) -> String {
    let mut out = String::new();
    let mut at = 0;
    for (start, end) in test_blocks(text) {
        out.push_str(&text[at..start]);
        at = end;
    }
    out.push_str(&text[at..]);
    out
}

/// The module names `text` declares as test-only files: `#[cfg(test)] mod name;`.
fn test_declarations(text: &str) -> Vec<String> {
    text.match_indices("#[cfg(test)]")
        .filter_map(|(at, tag)| {
            let rest = text[at + tag.len()..].trim_start();
            let line = rest.lines().next()?.trim();
            let name = line.strip_prefix("pub ").unwrap_or(line).strip_prefix("mod ")?.strip_suffix(';')?;
            Some(name.trim().to_string())
        })
        .collect()
}

/// Every `.rs` file under `dir`, as (path from `root`, its production code): test modules cut,
/// files that are only tests left out.
fn production_code(dir: &Path, root: &Path) -> Vec<(String, String)> {
    let mut files = Vec::new();
    let mut tests: BTreeSet<PathBuf> = BTreeSet::new();
    collect(dir, &mut files, &mut tests);
    files
        .into_iter()
        .filter(|(path, _)| !tests.iter().any(|t| path.starts_with(t)) && !path.ends_with("tests.rs") && !path.components().any(|c| c.as_os_str() == "tests"))
        .filter_map(|(path, text)| path.strip_prefix(root).ok().map(|rel| (rel.display().to_string(), without_test_modules(&text))))
        .collect()
}

fn collect(dir: &Path, files: &mut Vec<(PathBuf, String)>, tests: &mut BTreeSet<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, files, tests);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            // `name.rs` and `name/` beside this file, or under it when this is `mod.rs`, `main.rs`
            // or `lib.rs`.
            let own = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let home = if matches!(own.as_str(), "mod" | "main" | "lib") { dir.to_path_buf() } else { dir.join(&own) };
            for name in test_declarations(&text) {
                tests.insert(home.join(&name));
                tests.insert(home.join(format!("{name}.rs")));
            }
            files.push((path, text));
        }
    }
}

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap_or_else(|e| panic!("the repository root: {e}"))
}

/// The `src` directories of `yantrik-ui` and of every workspace crate it depends on, directly or
/// through another, read from `cargo metadata --no-deps`: a dependency with a `path` is a
/// workspace crate. This is the simpler form of the closure: crates from a registry are not
/// scanned, and nor are dev- or build-dependencies.
fn linked_sources() -> Vec<PathBuf> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let out = std::process::Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap_or_else(|e| panic!("cargo metadata did not run: {e}"));
    assert!(out.status.success(), "cargo metadata failed: {}", String::from_utf8_lossy(&out.stderr));
    let meta: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("cargo metadata said: {e}"));
    let packages = meta["packages"].as_array().cloned().unwrap_or_default();
    let by_name = |name: &str| packages.iter().find(|p| p["name"] == name).cloned();
    let (mut seen, mut todo) = (BTreeSet::new(), vec!["yantrik-ui".to_string()]);
    while let Some(name) = todo.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        let Some(package) = by_name(&name) else { continue };
        for dep in package["dependencies"].as_array().into_iter().flatten() {
            if dep["path"].is_string() && dep["kind"].is_null() {
                todo.extend(dep["name"].as_str().map(str::to_string));
            }
        }
    }
    seen.iter()
        .filter_map(|n| by_name(n))
        .filter_map(|p| p["manifest_path"].as_str().and_then(|m| Path::new(m).parent().map(|d| d.join("src"))))
        .collect()
}

/// The production code of the shell and of everything it links, by file.
fn linked_code() -> Vec<(String, String)> {
    let root = repository();
    linked_sources().iter().flat_map(|src| production_code(src, &root)).collect()
}

/// The files with a line that satisfies `hit`, sorted.
fn files_with_a_line(code: &[(String, String)], hit: impl Fn(&str) -> bool) -> Vec<String> {
    let mut found: Vec<String> = code.iter().filter(|(_, text)| text.lines().any(&hit)).map(|(path, _)| path.clone()).collect();
    found.sort();
    found
}

/// The method's name, put together here so that this file does not spell it in one piece.
const METHOD: &str = concat!("app", ".", "press");

const APP_ACT: &str = "crates/yantrik-ui/src/wire/app_act.rs";
const MEETING_BRIEF: &str = "crates/yantrik-ui/src/wire/meeting_brief.rs";
/// The app side that answers a press: it dispatches `app.press`, it never sends it.
const CONTROL: &str = "frameworks/yantrik-app/src/control.rs";

/// A line that spells the method `app.press`, or that could be building it from pieces:
/// `"app"` and `"press"` (or `".press"`) as quoted pieces. One line only: a construction split over
/// lines is for the compiler to hold.
fn names_the_press(line: &str) -> bool {
    // A comment says the name, as this file's own do; it sends nothing.
    !line.trim_start().starts_with("//")
        && (line.contains(METHOD) || (line.contains("\"app") && line.contains("press\"")))
}

#[test]
fn the_press_is_spelled_and_sent_in_the_shell_only_by_app_act() {
    let code = linked_code();
    assert!(code.iter().any(|(path, _)| path == APP_ACT), "the scan reads the shell's own code");
    assert!(code.iter().any(|(path, _)| path.starts_with("crates/yantrik-companion/")), "and the crates it links");
    assert_eq!(
        files_with_a_line(&code, names_the_press),
        [APP_ACT, CONTROL],
        "a new sender of app.press can open any message by folder and id: it belongs in wire/app_act.rs and nowhere else"
    );
    // The quoted literal anywhere in the repository, tests and scripts included.
    assert_eq!(files_quoting(METHOD), [APP_ACT, CONTROL]);
}

/// Every file of the repository (Rust or Python, not `target`, not `.git`) that holds `word` in
/// quotes, as paths from the repository root.
fn files_quoting(word: &str) -> Vec<String> {
    fn walk(dir: &Path, root: &Path, needle: &str, found: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if path.is_dir() {
                if !matches!(name.as_str(), "target" | ".git" | "node_modules") {
                    walk(&path, root, needle, found);
                }
            } else if (name.ends_with(".rs") || name.ends_with(".py")) && std::fs::read_to_string(&path).is_ok_and(|t| t.contains(needle)) {
                found.extend(path.strip_prefix(root).ok().map(|r| r.display().to_string()));
            }
        }
    }
    let root = repository();
    let mut found = Vec::new();
    walk(&root, &root, &format!("\"{word}\""), &mut found);
    found.sort();
    found
}

#[test]
fn only_the_brief_can_make_a_message_the_shell_presses() {
    let shell = production_code(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &repository());
    assert_eq!(
        files_with_a_line(&shell, |l| l.contains("BriefMessage::new(") || l.contains("BriefMessage {")),
        [MEETING_BRIEF],
        "a `BriefMessage` is made from the brief's own mail only, never from a poster's reference"
    );
    assert_eq!(
        files_with_a_line(&shell, |l| l.contains("\"open_message_id\"")),
        [APP_ACT],
        "the press of a message is the one function in app_act.rs"
    );
}

mod scanner {
    use super::*;

    #[test]
    fn a_test_module_declared_before_the_code_cuts_nothing() {
        let text = "#[cfg(test)]\nmod rc_keys;\n\nfn main() { press(); }\n";
        assert_eq!(without_test_modules(text), text);
        assert_eq!(test_declarations(text), ["rc_keys"]);
    }

    #[test]
    fn an_inline_test_module_is_cut_by_its_braces_and_the_code_after_it_is_kept() {
        let text = "fn a() {}\n#[cfg(test)]\nmod tests {\n    fn t() { let s = \"}\"; let c = '}'; /* } */ }\n    // }\n    const R: &str = r#\"}\"#;\n}\nfn after() { app.press }\n";
        let kept = without_test_modules(text);
        assert!(kept.contains("fn a()") && kept.contains("fn after() { app.press }"), "{kept}");
        assert!(!kept.contains("fn t()"), "{kept}");
    }

    #[test]
    fn a_cfg_test_on_one_item_and_a_lifetime_do_not_cut_anything() {
        let text = "#[cfg(test)]\nfn helper<'a>(x: &'a str) { let _ = '{'; }\nfn real() {}\n";
        assert_eq!(without_test_modules(text), text);
    }

    #[test]
    fn a_pub_crate_test_module_and_stacked_attributes_are_cut() {
        let text = "#[cfg(test)]\n#[allow(dead_code)]\npub(crate) mod t {\n  fn x() {}\n}\nfn kept() {}\n";
        assert_eq!(without_test_modules(text).trim(), "fn kept() {}");
    }

    #[test]
    fn pieces_of_the_method_name_are_seen() {
        for line in [
            "let m = \"app.press\";",
            "let m = concat!(\"app\", \".press\");",
            "let m = format!(\"app.{}\", \"press\");",
            "let m = [\"app\", \"press\"].join(\".\");",
        ] {
            assert!(names_the_press(line), "{line}");
        }
        assert!(!names_the_press("let m = \"app.act\";"));
        assert!(!names_the_press("/// Only `app.press` runs it."), "a comment names it and sends nothing");
    }
}
