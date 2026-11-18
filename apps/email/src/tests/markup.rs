//! What the markup must hold for the Rust side to be right.

/// The Enhance buttons used to hand the composer's body to the callback, and the handler —
/// which reads the body from the composer itself — put it into the prompt a second time, so
/// every rewrite request carried the email twice. The defect lives in the markup, so the markup
/// is what this pins: a button may name the tone, never the body.
#[test]
fn enhance_buttons_pass_a_tone_and_never_the_body() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../crates/yantrik-ui-slint/ui/email.slint");
    let src = std::fs::read_to_string(path).expect("the shared email markup");
    let calls: Vec<&str> = src.lines().filter(|l| l.contains("enhance-text(")).collect();
    assert!(calls.len() >= 4, "the four Enhance buttons should call enhance-text");
    for call in &calls {
        assert!(
            !call.contains("compose-body"),
            "an Enhance button hands the body to the handler, which sends it twice: {call}"
        );
    }
}

/// The reading pane's buttons send the pane's id, which is the open message's mark, and the
/// rows send their own: the two must stay that way for a press to act on what it shows.
#[test]
fn the_toolbar_sends_the_panes_id_and_a_row_sends_its_own() {
    let src = include_str!("../../../../crates/yantrik-ui-slint/ui/email.slint");
    for callback in ["archive-email", "delete-email", "mark-read", "mark-flagged"] {
        assert!(src.contains(&format!("root.{callback}(root.email-detail.id)")), "{callback} from the toolbar");
    }
    assert!(src.contains("root.email-selected(email.id)"));
    assert!(src.contains("root.mark-flagged(email.id)"));
    assert!(src.contains("root.toggle-thread-message(tmsg.id)"));
}
