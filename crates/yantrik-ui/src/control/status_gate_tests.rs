//! The tests of the status gate of the shell's control surface. Separate from control.rs, which is
//! over the file-size limit, so that file does not grow.

/// The mind's status line is the person's chat in miniature: `describe shell` withholds it
/// from an agent caller exactly as it withholds the conversation (#587 review).
#[test]
fn the_conversation_status_is_withheld_from_an_agent_caller() {
    let src = include_str!("../control.rs");
    let at = src.find(".with(\n                    \"conversation_status\"").expect("conversation_status field");
    let field = &src[at..at + 260];
    assert!(field.contains("if agent_reading { String::new() }"), "{field}");
    assert!(field.contains("status_for_describe"), "{field}");
}
