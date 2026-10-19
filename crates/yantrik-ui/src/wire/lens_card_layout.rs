//! Source checks for how the Lens draws an approval card (review of #580). The card is Slint, and
//! the rendered checks are in `tests/ui-preview/src/chat_tests.rs`; these are the ones that need
//! no render and run with every `cargo test`: each is named for the mistake it prevents.

const LENS: &str = include_str!("../../../yantrik-ui-slint/ui/components/intent_lens.slint");

/// The text of the `ApprovalCard` instance in the chat panel, from `for item[idx] in
/// root.approvals` to its `view-action` hookup.
fn card_instance() -> &'static str {
    let from = LENS.find("for item[idx] in root.approvals : ApprovalCard").expect("the chat panel draws the card");
    let rest = &LENS[from..];
    &rest[..rest.find("view-action =>").expect("and wires its View action")]
}

/// The line naming a waiting question appears above the reply box when a run asks (the work strip
/// did, when a run went live), which is when requests arrive, and the reply box grows as a person
/// types. A limit that read either moved the card's bottom edge, with its buttons, between a press
/// and a release; the question line has the strip's old room, and the limit reserves it.
#[test]
fn the_cards_height_does_not_read_the_question_line_or_the_reply_box_as_they_are_now() {
    let card = card_instance();
    let limit = &card[card.find("height-limit:").expect("the card is given a limit")..];
    assert!(!limit.contains("composer.bar-height -") && !limit.contains("composer.bar-height)"), "the reply box's current height: {limit}");
    assert!(!limit.contains("composer-asks"), "whether the question line is drawn now: {limit}");
    assert!(limit.contains("composer.bar-height-max") && limit.contains("Theme.chat-strip-height"), "the worst case of both: {limit}");
}

/// Everything above the card must be a constant of the panel: the header. The transcript, the
/// questions, the question line and the reply box all come after it.
#[test]
fn the_card_sits_directly_under_the_header_above_everything_that_changes() {
    let card = LENS.find("for item[idx] in root.approvals : ApprovalCard").unwrap();
    for later in [
        "ChatTranscript {",
        "for q in root.questions : VerticalLayout",
        "if root.composer-asks != \"\" : Rectangle",
        "composer := ChatComposer",
    ] {
        let at = LENS.find(later).unwrap_or_else(|| panic!("the panel has {later}"));
        assert!(card < at, "{later} is drawn after the card, so it cannot move it");
    }
}

/// The line above the reply box fits in the room the limit reserves for it.
#[test]
fn the_question_line_is_no_taller_than_the_room_kept_for_it() {
    let line = &LENS[LENS.find("if root.composer-asks != \"\" : Rectangle").unwrap()..];
    assert!(line[..200].contains("height: 32px;"), "{}", &line[..200]);
    let tokens = include_str!("../../../yantrik-design-tokens/slint/theme.slint");
    assert!(tokens.contains("chat-strip-height: 48px;"), "32px fits in the 48px kept");
}

#[test]
fn the_composers_tallest_height_is_what_its_own_arithmetic_allows() {
    let composer = include_str!("../../../yantrik-ui-slint/ui/components/chat_composer.slint");
    assert!(composer.contains("min(200px, input.preferred-height + 14px) + 72px"));
    assert!(composer.contains("bar-height-max: 272px"), "200 + 72");
}

/// Sensitive and standard used to share the amber edge.
#[test]
fn the_edge_and_heading_tell_the_grades_apart() {
    assert!(LENS.contains("root.data.grade == \"sensitive\" ? Theme.chat-needs-you : Theme.chat-border"), "standard is neutral, sensitive amber");
    assert!(LENS.contains("border-color: root.grade-edge;"));
    assert!(LENS.contains("\"Sensitive\"") && LENS.contains("\"Dangerous\""), "the heading names the grade");
}

/// Approval security properties that must hold after a restyle: no key reaches either button and
/// nothing is focused when the card appears.
#[test]
fn the_card_still_takes_no_key_and_no_focus() {
    let from = LENS.find("export component ApprovalCard").unwrap();
    let end = LENS[from..].find("\nexport component ").map_or(LENS.len(), |n| from + n);
    let card = &LENS[from..end];
    for banned in ["FocusScope", "key-pressed", "forward-focus", "focus()", "init =>"] {
        assert!(!card.contains(banned), "ApprovalCard must not contain `{banned}`");
    }
}

#[test]
fn the_resolved_line_names_the_action_and_how_long_the_grant_lasts() {
    assert!(LENS.contains("root.data.app + \".\" + root.data.action"));
    assert!(LENS.contains("(this session)") && LENS.contains("(once)"));
}
