//! Just enough of a .slint reader for the source checks: elements as brace-delimited blocks, and
//! the values of a property on one. Not a parser; a check built on it says "as far as a static
//! read can tell", and the pixel scenes in tests/ui-preview are what see the rendered result.

/// `src` with comments and string contents blanked to spaces, so a brace or a colour named in a
/// comment or a label is not read as markup. Byte offsets are unchanged.
pub fn strip(src: &str) -> String {
    let b = src.as_bytes();
    let mut out = b.to_vec();
    let mut i = 0;
    let blank = |out: &mut Vec<u8>, from: usize, to: usize| {
        for c in &mut out[from..to] {
            if *c != b'\n' {
                *c = b' ';
            }
        }
    };
    while i < b.len() {
        if b[i..].starts_with(b"//") {
            let end = b[i..].iter().position(|&c| c == b'\n').map_or(b.len(), |p| i + p);
            blank(&mut out, i, end);
            i = end;
        } else if b[i..].starts_with(b"/*") {
            let end = src[i + 2..].find("*/").map_or(b.len(), |p| i + 2 + p + 2);
            blank(&mut out, i, end);
            i = end;
        } else if b[i] == b'"' {
            let mut j = i + 1;
            while j < b.len() && b[j] != b'"' {
                j += if b[j] == b'\\' { 2 } else { 1 };
            }
            let end = j.min(b.len());
            blank(&mut out, i + 1, end);
            i = end + 1;
        } else {
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// One `{ ... }`: the offsets of its braces and how deep it sits.
#[derive(Clone, Copy)]
pub struct Block {
    pub open: usize,
    pub close: usize,
    pub depth: usize,
}

/// Every block in stripped source, outermost first.
pub fn blocks(s: &str) -> Vec<Block> {
    let (mut stack, mut out) = (Vec::new(), Vec::new());
    for (i, c) in s.bytes().enumerate() {
        match c {
            b'{' => stack.push(i),
            b'}' => {
                if let Some(open) = stack.pop() {
                    out.push(Block { open, close: i, depth: stack.len() });
                }
            }
            _ => {}
        }
    }
    out.sort_by_key(|b| b.open);
    out
}

/// A block's own text: its children blanked, so a property read from it is the element's own and
/// not a descendant's.
pub fn own_text(s: &str, block: Block, all: &[Block]) -> String {
    let mut own = s.as_bytes()[block.open + 1..block.close].to_vec();
    for child in all.iter().filter(|c| c.depth == block.depth + 1 && c.open > block.open && c.close < block.close) {
        for c in &mut own[child.open - block.open - 1..=child.close - block.open - 1] {
            *c = b' ';
        }
    }
    String::from_utf8_lossy(&own).into_owned()
}

/// The values of `name:` in `text`, whitespace collapsed. `color` does not match `border-color`.
pub fn values(text: &str, name: &str) -> Vec<String> {
    let key = format!("{name}:");
    let mut out = Vec::new();
    for (at, _) in text.match_indices(&key) {
        let before = text[..at].chars().next_back();
        if before.is_some_and(|c| c == '-' || c.is_ascii_alphanumeric()) {
            continue;
        }
        let rest = &text[at + key.len()..];
        if let Some(end) = rest.find(';') {
            out.push(rest[..end].split_whitespace().collect::<Vec<_>>().join(" "));
        }
    }
    out
}

/// Whether `value` names `token` itself (`Theme.accent`, not `Theme.accent-dim`, and not a
/// translucent wash of it, which shows the surface beneath rather than the accent).
pub fn names(value: &str, token: &str) -> bool {
    value.match_indices(token).any(|(at, _)| {
        let rest = &value[at + token.len()..];
        let ends = !rest.starts_with(|c: char| c == '-' || c == '_' || c.is_ascii_alphanumeric());
        ends && !rest.starts_with(".transparentize") && !rest.starts_with(".with-alpha")
    })
}
