//! The apps' `.slint` screens publish their interactive rows and buttons over the accessibility
//! bus (`a11y.describe`). A `TouchArea` with no `accessible-role` publishes nothing a screen
//! reader can name, so a button and a label look the same. These tests keep the app screens
//! honest: every clickable `TouchArea` must carry a role (on itself or its parent), and every
//! role that is not `none` must carry a label.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn slint_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            slint_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "slint") {
            out.push(path);
        }
    }
}

/// One `{ ... }` block in a .slint file: the element or callback it opens, its byte span, and
/// the block that encloses it (its parent element).
struct Block {
    name: String,
    open: usize,
    close: usize,
    parent: Option<usize>,
}

/// The word immediately before a `{`, which names the element (`TouchArea`, `Rectangle`) or the
/// callback (`clicked`) that the block opens.
fn name_before(src: &str, open: usize) -> String {
    let before = &src[..open];
    let trimmed = before.trim_end();
    let word = trimmed.rsplit(|c: char| !c.is_alphanumeric() && c != '-' && c != '_').next().unwrap_or("");
    word.to_string()
}

/// Every `{ ... }` block, in opening order, with its parent. Braces inside string literals are
/// rare in these screens and none of the scanned files put a `{` in a string, so a plain scan is
/// enough.
fn blocks(src: &str) -> Vec<Block> {
    let bytes = src.as_bytes();
    let mut out: Vec<Block> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'{' {
            let parent = stack.last().copied();
            stack.push(out.len());
            out.push(Block { name: name_before(src, i), open: i, close: 0, parent });
        } else if b == b'}' {
            if let Some(idx) = stack.pop() {
                out[idx].close = i;
            }
        }
    }
    out
}

/// Whether the text of a block (its body, between the braces) declares the given attribute.
fn has_attr(src: &str, block: &Block, attr: &str) -> bool {
    let body = &src[block.open + 1..block.close];
    body.contains(attr)
}

/// The role a block declares, if any: the value after `accessible-role:`.
fn role_of(src: &str, block: &Block) -> Option<String> {
    let body = &src[block.open + 1..block.close];
    let pos = body.find("accessible-role")?;
    let after = &body[pos + "accessible-role".len()..];
    let after = after.trim_start().strip_prefix(':')?.trim_start();
    let value = after.split(|c: char| c == ';' || c == '\n').next().unwrap_or("").trim();
    if value.is_empty() { None } else { Some(value.to_string()) }
}

/// Whether a block (or, walking up, one of its parents) declares `accessible-role`.
fn has_role_anywhere(src: &str, blocks: &[Block], mut idx: usize) -> bool {
    loop {
        if has_attr(src, &blocks[idx], "accessible-role") {
            return true;
        }
        match blocks[idx].parent {
            Some(p) => idx = p,
            None => return false,
        }
    }
}

/// Every `TouchArea` that has a `clicked =>` handler must carry an `accessible-role` on itself or
/// a parent, and every role that is not `none` must carry an `accessible-label`.
#[test]
fn every_clickable_touch_area_in_an_app_is_named() {
    let root = repo_root();
    let mut files = Vec::new();
    slint_files(&root.join("apps"), &mut files);
    assert!(files.len() >= 4, "found only {} app .slint files; the scan is looking in the wrong place", files.len());

    let mut unnamed = Vec::new();
    let mut unlabelled = Vec::new();
    for file in &files {
        let rel = file.to_string_lossy().replace('\\', "/");
        let src = std::fs::read_to_string(file).unwrap_or_default();
        let blocks = blocks(&src);
        for (i, block) in blocks.iter().enumerate() {
            if block.name != "TouchArea" {
                continue;
            }
            let body = &src[block.open + 1..block.close];
            if !body.contains("clicked") {
                continue;
            }
            if !has_role_anywhere(&src, &blocks, i) {
                unnamed.push(format!("{rel}:{}: TouchArea with `clicked` has no accessible-role", line_of(&src, block.open)));
            }
            // A role on a parent still needs a label on the element that carries the role.
            let role_idx = role_owner(&src, &blocks, i);
            let role = role_of(&src, &blocks[role_idx]);
            if let Some(role) = role {
                if role != "none" && !has_attr(&src, &blocks[role_idx], "accessible-label") {
                    unlabelled.push(format!("{rel}:{}: accessible-role {role} has no accessible-label", line_of(&src, block.open)));
                }
            }
        }
    }
    assert!(unnamed.is_empty(), "these clickable TouchAreas publish no role:\n{}", unnamed.join("\n"));
    assert!(unlabelled.is_empty(), "these roles publish no label:\n{}", unlabelled.join("\n"));
}

/// The block that actually declares the role for `idx`: itself if it has one, else the nearest
/// parent that does.
fn role_owner(src: &str, blocks: &[Block], mut idx: usize) -> usize {
    loop {
        if has_attr(src, &blocks[idx], "accessible-role") {
            return idx;
        }
        match blocks[idx].parent {
            Some(p) => idx = p,
            None => return idx,
        }
    }
}

fn line_of(src: &str, byte: usize) -> usize {
    src[..byte].bytes().filter(|&b| b == b'\n').count() + 1
}
