//! The chat's words are selectable and copyable (#1113), through one component for every surface.
//!
//! Slint's `Text` cannot be selected, so a reply could not be copied. A read-only `TextInput` can,
//! and `SelectableText` is that. These checks read the markup, as far as a static read can tell:
//! the Lens, an agent's pane and Mind View, and the kit's own bubble all draw their words with it,
//! and none keeps a code block of its own. The pixel scenes in tests/ui-preview see the look.

use crate::slint_source::strip;
use std::path::{Path, PathBuf};

fn kit(file: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("slint").join(file);
    strip(&std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
}

fn shell(file: &str) -> String {
    let path: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("../yantrik-ui-slint/ui").join(file);
    strip(&std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
}

/// Every file that draws a conversation: the Lens's message, the agent pane that Mind View shares,
/// the kit's bubble, and the tool card all of them show.
fn surfaces() -> Vec<(&'static str, String)> {
    vec![
        ("the Lens (chat_message.slint)", shell("components/chat_message.slint")),
        ("an agent's pane and Mind View (agents.slint)", shell("agents.slint")),
        ("the kit bubble (message_bubble.slint)", kit("message_bubble.slint")),
        ("the tool card (tool_call.slint)", kit("tool_call.slint")),
    ]
}

#[test]
fn the_selectable_text_is_a_read_only_wrapping_text_input() {
    let src = kit("selectable_text.slint");
    assert!(src.contains("inherits TextInput"), "selection needs a TextInput");
    for line in ["read-only: true;", "single-line: false;", "wrap: word-wrap;"] {
        assert!(src.contains(line), "SelectableText must keep `{line}`");
    }
}

#[test]
fn every_chat_surface_draws_its_words_with_the_selectable_text() {
    for (name, src) in surfaces() {
        assert!(src.contains("SelectableText"), "{name} does not use SelectableText");
    }
}

#[test]
fn the_reply_text_of_each_surface_is_drawn_by_the_selectable_text() {
    let values = [
        ("components/chat_message.slint", "text: root.data.content;"),
        ("components/chat_message.slint", "text: block.text;"),
        ("message_bubble.slint", "text: data.content;"),
        ("message_bubble.slint", "text: block.text;"),
        ("tool_call.slint", "text: root.call.arguments;"),
        ("tool_call.slint", "text: root.call.output;"),
    ];
    for (file, value) in values {
        let src = if file.starts_with("components/") || file == "agents.slint" { shell(file) } else { kit(file) };
        assert!(src.contains(value), "{file} no longer has `{value}`");
        for (at, _) in src.match_indices(value) {
            let opener = src[..at].rfind('{').map(|o| src[..o].trim_end()).unwrap_or("");
            assert!(
                opener.ends_with("SelectableText") || opener.ends_with("CodeBlock"),
                "{file}: `{value}` is drawn by `{}`, which cannot be selected",
                opener.rsplit(|c: char| c.is_whitespace()).next().unwrap_or("")
            );
        }
    }
}

#[test]
fn no_surface_keeps_a_code_block_of_its_own() {
    for (name, src) in surfaces() {
        assert!(!src.contains("code := Text"), "{name} draws its own code block; use CodeBlock");
    }
    for file in ["components/chat_message.slint", "agents.slint"] {
        assert!(shell(file).contains("CodeBlock {"), "{file} must draw code with CodeBlock");
    }
    assert!(kit("message_bubble.slint").contains("CodeBlock {"));
}

#[test]
fn the_copy_button_copies_exactly_the_blocks_text_and_nothing_else_touches_the_clipboard() {
    let src = kit("code_block.slint");
    let body = &src[src.find("function copy-all()").expect("the Copy handler")..];
    let body = &body[..body.find('}').expect("end of handler")];
    assert!(body.contains("code.select-all();") && body.contains("code.copy();"));
    assert!(src.contains("code := SelectableText") && src.contains("text: root.text;"));
    assert_eq!(src.matches(".copy()").count(), 1, "Copy is the only clipboard write");
    for (name, src) in surfaces().into_iter().chain([("the code block", src.clone())]) {
        assert!(!src.contains(".paste(") && !src.contains(".cut("), "{name} reads the clipboard");
    }
    for (name, src) in surfaces() {
        assert!(!src.contains(".copy()"), "{name} writes the clipboard itself; only the Copy button may");
    }
}
