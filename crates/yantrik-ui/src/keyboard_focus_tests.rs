//! Source checks that the first screens a person meets can be used with the keyboard alone
//! (#1165). The behaviour itself is driven by tests/ui-preview (installer_keys_tests.rs,
//! first_run_tests.rs, chat_parts_tests.rs for the Lens buttons); these keep the pieces it relies
//! on from being edited away.

const KIT: &str = include_str!("../../yantrik-ui-slint/ui/components/onboard_kit.slint");
const FIRST_RUN: &str = include_str!("../../yantrik-ui-slint/ui/onboarding_first_run.slint");
const INSTALLER: &str = include_str!("../../yantrik-ui-slint/ui/installer.slint");
const CHAT_PARTS: &str = include_str!("../../yantrik-ui-slint/ui/components/chat_parts.slint");

/// The comment-free lines of a Slint file.
fn code(src: &str) -> String {
    src.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n")
}

#[test]
fn the_installer_buttons_show_an_accent_ring_when_focused() {
    let kit = code(KIT);
    assert!(kit.contains("if button.focused : Rectangle"), "PillButton draws a ring from its button's focus");
    assert!(kit.contains("border-color: Theme.accent;"));
}

#[test]
fn the_first_run_buttons_show_an_accent_ring_when_focused() {
    let src = code(FIRST_RUN);
    let button = &src[src.find("component FrButton").unwrap()..src.find("component FrCheckbox").unwrap()];
    assert!(button.contains("if fs.has-focus : Rectangle") && button.contains("border-color: Theme.accent;"));
    assert!(button.contains("fs := FocusScope"), "and takes a Tab stop");
}

#[test]
fn the_lens_buttons_take_tab_and_enter_and_show_the_ring() {
    let src = code(CHAT_PARTS);
    assert_eq!(src.matches("FocusScope {").count(), 2, "ChatPill and ChatIconButton each take focus");
    assert_eq!(src.matches("event.text == Key.Return || event.text == \" \"").count(), 2, "Enter and Space press them");
    assert!(src.contains("pill-key.has-focus ? Theme.accent") && src.contains("icon-key.has-focus"));
}

#[test]
fn tab_never_chooses_the_way_to_install_and_the_chips_are_pressed_by_enter_or_space() {
    let src = code(INSTALLER);
    let chip = &src[src.find("component ModeChip").unwrap()..src.find("component SegmentBlock").unwrap()];
    assert!(chip.contains("chip-key := KeyReach"), "a chip is a focus stop");
    assert!(chip.contains("activate => { root.clicked(); }"));
    assert!(!chip.contains("focus-changed") && !chip.contains("changed has-focus"), "focus alone chooses nothing");
}

#[test]
fn the_installed_screen_waits_while_there_is_something_to_remove() {
    let src = code(INSTALLER);
    assert!(src.contains("root.restart-held || root.install-note != \"\" || root.media-removable"));
    assert!(src.contains("!root.restart-waits && root.restart-in > 0"), "no countdown while it waits");
}

#[test]
fn the_disk_screen_says_encryption_is_on_and_what_a_forgotten_password_costs() {
    let src = code(INSTALLER);
    assert!(src.contains("Encryption is on."));
    assert!(src.contains("a forgotten password means an unreadable disk"));
}

#[test]
fn macos_is_mentioned_beside_what_is_there_only_when_a_mac_disk_was_found() {
    let src = code(INSTALLER);
    let at = src.find("Nothing here can be installed into yet. In macOS").unwrap();
    assert!(src[at - 80..at].contains("root.macos-disks != \"\""), "behind the macOS-disk check");
}

#[test]
fn the_provider_step_offers_the_free_accounts_helper_first_and_says_the_desktop_works_without_a_model() {
    let src = code(FIRST_RUN);
    let step = &src[src.find("if root.step == 2 : VerticalLayout").unwrap()..];
    let helper = step.find("FreeAiCard { }").expect("the helper is on the step");
    for later in ["root.look-nearby()", "for p[i] in root.featured", "Advanced"] {
        assert!(helper < step.find(later).unwrap(), "the helper comes before {later}");
    }
    assert!(step.contains("Every option needs a free account (Groq takes about 2 minutes) or your own GPU."));
    assert!(step.contains("The desktop and its apps work without a model. You can connect a Mind later in Settings."));
    assert!(!step.contains("chosen-id = \"groq\""), "no provider is chosen for the person");
}
