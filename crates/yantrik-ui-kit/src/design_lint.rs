//! The design-system lint: the kit and the tokens are the only way a screen is drawn.
//!
//! Yantrik OS has one theme (yantrik-design-tokens) and one component kit (this crate), and an
//! audit on 21 Oct 2026 found screens around them anyway: 124 private components, 11 of them
//! buttons, 54 raw colours and 72 pixel font sizes (Pranab: "we don't have common libraries to
//! make sure we have consistency?"). This holds every `.slint` file in the shell, the apps, Alt+Tab
//! and the kit itself to four rules:
//!
//! - `hex`: no colour literal. A colour is a theme token (`Theme.…`).
//! - `px-font`: no font size in pixels. A size is a token (`Theme.fs-…`), so it follows the type
//!   scale and the person's text size.
//! - `private-control`: no screen draws its own button, chip, pill, badge or card. The kit has
//!   those (`YButton`, …), and a screen that needs a new one adds it to the kit.
//! - `std-widgets`: no screen imports Slint's stock widgets, which bring their own look.
//!
//! What was there on the day this landed is listed in `design-lint-baseline.txt`, per file and
//! rule. The list may only shrink: a file over its count fails, and so does one under it, so the
//! list is kept true and every fix is kept. To rewrite the list after a fix (never to raise it):
//! `DESIGN_LINT_WRITE=1 cargo test -p yantrik-ui-kit design_lint`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::slint_source::strip;

const BASELINE: &str = "crates/yantrik-ui-kit/design-lint-baseline.txt";
/// Where `.slint` that draws the OS lives. The tokens are the one place colours and sizes are
/// written as values, so they are not scanned.
const ROOTS: &[&str] = &["crates/yantrik-ui-slint/ui", "crates/yantrik-ui-kit/slint", "crates/yantrik-switcher/ui", "apps"];
/// The kit itself may define controls: that is what it is for.
const KIT: &str = "crates/yantrik-ui-kit/slint/";
const CONTROL_SUFFIXES: &[&str] = &["Button", "Btn", "Chip", "Pill", "Badge", "Card"];

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn slint_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        // The entry's own type, not its target's: a symlinked folder is not walked, so the walk
        // stays in the repository and cannot loop.
        if e.file_type().is_ok_and(|t| t.is_dir()) {
            if name != "target" && name != "node_modules" && !name.starts_with('.') {
                slint_files(&p, out);
            }
        } else if name.ends_with(".slint") {
            out.push(p);
        }
    }
}

/// Colour literals: `#` and 3, 4, 6 or 8 hex digits, not part of a longer word.
fn hex_colours(code: &str) -> usize {
    let b = code.as_bytes();
    let mut n = 0;
    for (i, &c) in b.iter().enumerate() {
        if c != b'#' {
            continue;
        }
        let digits = b[i + 1..].iter().take_while(|d| d.is_ascii_hexdigit()).count();
        let next = b.get(i + 1 + digits).copied().unwrap_or(b' ');
        if [3, 4, 6, 8].contains(&digits) && !(next.is_ascii_alphanumeric() || next == b'_' || next == b'-') {
            n += 1;
        }
    }
    n
}

/// `font-size: 13px` and the like: a literal pixel size, wherever the expression starts with one.
fn px_fonts(code: &str) -> usize {
    code.match_indices("font-size:")
        .filter(|(at, m)| {
            let rest = code[at + m.len()..].trim_start();
            let digits = rest.chars().take_while(|c| c.is_ascii_digit() || *c == '.').count();
            digits > 0 && rest[digits..].starts_with("px")
        })
        .count()
}

/// `component FooButton inherits …` outside the kit.
fn private_controls(code: &str) -> usize {
    code.lines()
        .filter_map(|l| {
            let l = l.trim_start();
            let l = l.strip_prefix("export ").unwrap_or(l);
            let rest = l.strip_prefix("component ")?;
            let name = rest.split_whitespace().next()?;
            rest.contains(" inherits ").then_some(name)
        })
        .filter(|name| CONTROL_SUFFIXES.iter().any(|s| name.ends_with(s)))
        .count()
}

/// An import from Slint's stock widget set. Read from the raw source: `strip` blanks the string.
fn std_widgets(raw: &str) -> usize {
    raw.lines().filter(|l| l.trim_start().starts_with("import ") && l.contains("\"std-widgets.slint\"")).count()
}

/// Every (rule, file) with a count above zero, files as repo-relative paths with `/`.
fn current() -> BTreeMap<(String, String), usize> {
    let root = repo();
    let mut files = Vec::new();
    for r in ROOTS {
        slint_files(&root.join(r), &mut files);
    }
    let mut out = BTreeMap::new();
    for f in files {
        let rel = f.strip_prefix(&root).unwrap_or(&f).to_string_lossy().replace('\\', "/");
        let raw = std::fs::read_to_string(&f).unwrap_or_default();
        let code = strip(&raw);
        let mut put = |rule: &str, n: usize| {
            if n > 0 {
                out.insert((rule.to_string(), rel.clone()), n);
            }
        };
        put("hex", hex_colours(&code));
        put("px-font", px_fonts(&code));
        if !rel.starts_with(KIT) {
            put("private-control", private_controls(&code));
            put("std-widgets", std_widgets(&raw));
        }
    }
    out
}

fn read_baseline() -> BTreeMap<(String, String), usize> {
    let text = std::fs::read_to_string(repo().join(BASELINE)).unwrap_or_default();
    text.lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let mut parts = l.split_whitespace();
            let (rule, count, path) = (parts.next()?, parts.next()?.parse().ok()?, parts.next()?);
            Some(((rule.to_string(), path.to_string()), count))
        })
        .collect()
}

fn write_baseline(now: &BTreeMap<(String, String), usize>) {
    let mut text = String::from(
        "# The design-system lint's list of what was there before it (src/design_lint.rs). It may only\n\
         # shrink: fix a file, then rewrite this with DESIGN_LINT_WRITE=1 cargo test -p yantrik-ui-kit design_lint.\n\
         # rule count path\n",
    );
    for ((rule, path), n) in now {
        text.push_str(&format!("{rule} {n} {path}\n"));
    }
    std::fs::write(repo().join(BASELINE), text).expect("write the baseline");
}

#[test]
fn every_screen_is_drawn_with_the_kit_and_the_tokens() {
    let now = current();
    if std::env::var("DESIGN_LINT_WRITE").as_deref() == Ok("1") {
        let before = read_baseline();
        let raised: Vec<String> = now
            .iter()
            .filter(|(k, n)| before.get(*k).copied().unwrap_or(0) < **n)
            .map(|((rule, path), n)| format!("{rule} {path}: {} -> {n}", before.get(&(rule.clone(), path.clone())).copied().unwrap_or(0)))
            .collect();
        // The first list is whatever is there; after that it may only come down.
        let first = !repo().join(BASELINE).exists();
        assert!(first || raised.is_empty(), "the list may only shrink; these would raise it:\n  {}", raised.join("\n  "));
        write_baseline(&now);
        return;
    }
    let base = read_baseline();
    let mut worse = Vec::new();
    let mut better = Vec::new();
    for (k, n) in &now {
        let allowed = base.get(k).copied().unwrap_or(0);
        if *n > allowed {
            worse.push(format!("{} in {}: {n} (allowed {allowed})", k.0, k.1));
        }
    }
    for (k, allowed) in &base {
        let n = now.get(k).copied().unwrap_or(0);
        if n < *allowed {
            better.push(format!("{} in {}: {n} (listed {allowed})", k.0, k.1));
        }
    }
    assert!(
        worse.is_empty(),
        "drawn around the kit and the tokens (use a Theme token, a kit component, or add one to the kit):\n  {}",
        worse.join("\n  ")
    );
    assert!(
        better.is_empty(),
        "fixed, so the list must come down to keep the fix (DESIGN_LINT_WRITE=1 cargo test -p yantrik-ui-kit design_lint):\n  {}",
        better.join("\n  ")
    );
}

#[test]
fn the_rules_see_what_they_are_meant_to() {
    assert_eq!(hex_colours("background: #0b121c; color: #fff; x: #12345678;"), 3);
    assert_eq!(hex_colours("a: #12345; b: #abcdefg; c: Theme.bg;"), 0, "not a colour");
    assert_eq!(px_fonts("font-size: 13px; font-size: Theme.fs-body; font-size: 11px * root.scale;"), 2);
    assert_eq!(px_fonts("font-size: Theme.fs-caption * 2;"), 0);
    assert_eq!(private_controls("component IconButton inherits TouchArea {\nexport component StatusChip inherits Rectangle {\ncomponent Row inherits Rectangle {"), 2);
    assert_eq!(std_widgets("import { ListView } from \"std-widgets.slint\";\nimport { Theme } from \"theme.slint\";"), 1);
    // Comments and strings are not markup.
    assert_eq!(hex_colours(&strip("// #ffffff\ntext: \"#ffffff\";")), 0);
}
