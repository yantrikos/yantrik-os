//! What the first run draws about the default mind, before the install it names can start.

/// The words are drawn on the model step whatever the connect status (not only after `connected`,
/// which is when the install has already begun), and on the welcome step when the installer
/// carried the pick, where no model step is drawn.
#[test]
fn the_first_run_draws_the_sentence_before_the_pick_and_on_the_installer_carried_path() {
    let slint = include_str!("../../yantrik-ui-slint/ui/onboarding_first_run.slint");
    let step = |from: &str, to: &str| {
        let a = slint.find(from).unwrap_or_else(|| panic!("{from}"));
        &slint[a..a + slint[a..].find(to).unwrap_or(slint.len() - a)]
    };
    let model_step = step("if root.step == 2 : VerticalLayout", "if root.step == 3 : VerticalLayout");
    let words = model_step.find("if root.default-mind-words != \"\" : Text").expect("the model step draws the words");
    let connect = model_step.find("root.connect-status").expect("the connect row");
    assert!(words < connect, "drawn above the controls that pick and connect, whatever their state");
    assert!(!model_step.contains("connect-status == \"connected\" && root.default-mind-words"), "not only once connected");
    let welcome = step("if root.step == 1 : VerticalLayout", "if root.step == 2 : VerticalLayout");
    assert!(welcome.contains("if root.installer-chose && root.default-mind-words != \"\" : Text"), "the carried path shows it before the first boot's install");
    let timer = include_str!("wire/harness.rs");
    let (set, noted) = (timer.find("ui.set_fr_default_mind_words(words.clone())").unwrap(), timer.find("default_mind::published(&words)").unwrap());
    assert!(set < noted, "noted as shown only after it was put on the screen");
}

