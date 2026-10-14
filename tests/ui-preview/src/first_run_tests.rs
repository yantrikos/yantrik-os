//! The first run (design/experience-final-2026-10-10.md, "First run, minute by minute"), drawn
//! from the production FirstRunScreen and the production chat, and driven with real key events:
//!
//! - Welcome: focus starts on Make text bigger; Space makes the text bigger live (1.25, 1.5,
//!   then back to normal); Tab reaches Read this to me, whose Enter speaks the screen's words;
//!   Tab reaches Get started, whose Enter goes to "Who should do the thinking?".
//!   A first boot the installer did not set up asks the person's name: one field, Enter goes on.
//!   Escape is Back on every screen, never closing the first run.
//! - Who thinks: focus starts on the first card; Space chooses it; the key field takes focus and
//!   typing then Enter hands the shell the provider and the key; the account's models are listed
//!   with none chosen, and the person picks one by keyboard; a pick that did not take offers Try
//!   again; "Show every provider" opens the full list.
//! - What your minds share: the sentence names the provider and is pre-checked; Space unchecks
//!   it and the screen says what that means; choosing Keep personal things on this computer
//!   shows the sentence unchecked.
//! - Getting ready and Your minds: the three rows per mind, in every state.
//! - The installer already chose: Get started goes straight to the checks, and starts them.
//! - The chat's first success: one suggestion and Type · Talk; Talk reaches the microphone; a
//!   reply that was not an answer says so with Try again; only a real answer brings "You're
//!   ready." with Done.
//!
//! Stills are written beside `output` (first-run-1-welcome.png …) for the pull request.

use super::*;
use slint::{ModelRc, SharedString, VecModel};

fn provider(id: &str, name: &str, line: &str, needs: &str, group: &str, placeholder: &str, key_help: bool) -> FirstRunProvider {
    FirstRunProvider {
        id: id.into(),
        name: name.into(),
        line: line.into(),
        needs: needs.into(),
        group: group.into(),
        key_placeholder: placeholder.into(),
        browser_sign_in: false,
        key_help,
    }
}

/// The named cards as wire/onboarding.rs makes them, with a model on this computer.
fn featured() -> Vec<FirstRunProvider> {
    vec![
        provider("anthropic", "Claude", "online, careful with words", "needs an Anthropic account", "cloud", "sk-ant-…", true),
        provider("openai", "OpenAI", "online, a good all-rounder", "needs an OpenAI account", "cloud", "sk-…", true),
        provider("gemini", "Google Gemini", "online, reads long documents", "needs a Google account", "cloud", "AIza…", true),
        provider("deepseek", "DeepSeek", "online, strong at reasoning, low cost", "needs a DeepSeek account", "cloud", "sk-…", true),
        provider("openrouter", "OpenRouter", "online, one account for many models", "needs an OpenRouter account", "aggregator", "sk-or-…", true),
        provider("nvidia-nim", "NVIDIA NIM", "online, many open models", "needs an NVIDIA account", "cloud", "nvapi-…", true),
        provider("this-computer", "On this computer", "private, slower", "already here (2.7 GB)", "this-computer", "", false),
    ]
}

/// A stand-in for the catalogue's full list: its length and order are the production's concern
/// (wire/onboarding.rs tests that every provider is there); here it only has to draw.
fn every() -> Vec<FirstRunProvider> {
    let mut v = featured();
    v.pop();
    for (id, name) in [("groq", "Groq"), ("mistral", "Mistral"), ("xai", "xAI"), ("ollama", "Ollama"), ("llamacpp", "llama.cpp")] {
        let local = matches!(id, "ollama" | "llamacpp");
        v.push(provider(
            id,
            name,
            if local { "on a computer you run it on" } else { "online" },
            &if local { format!("needs {name} running on this computer or your network") } else { format!("needs a {name} account") },
            if local { "local" } else { "cloud" },
            if local { "" } else { "key" },
            !local,
        ));
    }
    v
}

fn check(id: &str, label: &str, state: &str, words: &str, details: &str) -> FirstRunCheck {
    FirstRunCheck { id: id.into(), label: label.into(), state: state.into(), words: words.into(), details: details.into() }
}

fn mind(name: &str, words: &str, ok: bool, checks: Vec<FirstRunCheck>) -> FirstRunMind {
    FirstRunMind { id: name.to_lowercase().into(), name: name.into(), words: words.into(), ok, checks: ModelRc::new(VecModel::from(checks)) }
}

fn minds_checking() -> Vec<FirstRunMind> {
    vec![mind("Yantrik Companion", "Checking", false, vec![
        check("answer", "Can it answer?", "passed", "Passed", "https://api.anthropic.com/v1 · answered in 840 ms"),
        check("remember", "Can it remember?", "checking", "Checking (started 2:16)", ""),
        check("desktop", "Can it reach the desktop?", "not-checked", "Not checked yet", ""),
    ])]
}

fn minds_done() -> Vec<FirstRunMind> {
    vec![
        mind("Yantrik Companion", "Ready", true, vec![
            check("answer", "Can it answer?", "passed", "Passed", "https://api.anthropic.com/v1 · answered in 840 ms"),
            check("remember", "Can it remember?", "passed", "Passed", "wrote a note, recalled it, deleted it"),
            check("desktop", "Can it reach the desktop?", "passed", "Passed", "attached as companion"),
        ]),
        mind("Hermes", "Needs attention: the key was refused", false, vec![
            check("answer", "Can it answer?", "attention", "Needs attention: the key was refused", "401 from the provider"),
            check("remember", "Can it remember?", "passed", "Passed", ""),
            check("desktop", "Can it reach the desktop?", "passed", "Passed", ""),
        ]),
    ]
}

fn strings(v: &[&str]) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(v.iter().map(|s| SharedString::from(*s)).collect::<Vec<_>>()))
}

fn type_text(w: &MinimalSoftwareWindow, s: &str) {
    for c in s.chars() {
        key(w, c.to_string().into());
    }
}

fn render(w: &MinimalSoftwareWindow, width: u32, height: u32, path: &str) -> Result<(), Box<dyn std::error::Error>> {
    slint::platform::update_timers_and_animations();
    let mut p = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width, height);
    w.request_redraw();
    w.draw_if_needed(|r| {
        r.render(p.make_mut_slice(), width as usize);
    });
    let f = BufWriter::new(File::create(path)?);
    let mut e = png::Encoder::new(f, width, height);
    e.set_color(png::ColorType::Rgb);
    e.set_depth(png::BitDepth::Eight);
    e.write_header()?.write_image_data(p.as_bytes())?;
    println!("Rendered {path}");
    Ok(())
}

pub fn run(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (width, height) = (1280u32, 800u32);
    let still = |name: &str| output.replace(".png", &format!("-{name}.png"));
    let tab = || key(w, "\t".into());
    let space = || key(w, " ".into());
    let enter = || key(w, "\n".into());

    let ui = FirstRunProbe::new()?;
    ui.set_ask_name(true);
    ui.set_featured(ModelRc::new(VecModel::from(featured())));
    ui.set_every_provider(ModelRc::new(VecModel::from(every())));
    ui.set_shared_items(strings(&["Your name", "Your town", "The app you're in", "What your minds remember about you"]));
    ui.set_private_lane_words("Uses the 2.7 GB model already on this computer. Answers about you may be slower.".into());
    // The shell's answer to choosing a card (wire/onboarding.rs, on_fr_choose_provider).
    let weak = ui.as_weak();
    ui.on_choose_provider(move |id| {
        let ui = weak.unwrap();
        let card = featured().into_iter().chain(every()).find(|c| c.id == id).unwrap();
        ui.set_chosen_id(card.id.clone());
        ui.set_chosen_name(card.name.clone());
        ui.set_chosen_key_placeholder(card.key_placeholder.clone());
        ui.set_chosen_key_help(card.key_help);
        ui.set_connect_mode(if id == "this-computer" { "this-computer".into() } else { "key".into() });
    });
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(width, height));

    // 1. Welcome.
    render(w, width, height, &still("1-welcome"))?;
    space();
    assert_eq!(ui.get_scale_now(), 1.25, "Make text bigger has focus first and applies live");
    render(w, width, height, &still("1-welcome-bigger"))?;
    space();
    assert_eq!(ui.get_scale_now(), 1.5);
    space();
    assert_eq!(ui.get_scale_now(), 1.0, "a third press goes back to normal");
    tab();
    enter();
    assert!(ui.get_spoken().starts_with("Welcome to Yantrik."), "Read this to me speaks the screen: {}", ui.get_spoken());
    assert!(ui.get_spoken().contains("What should your minds call you?"), "{}", ui.get_spoken());
    // A first boot the installer did not set up asks the name: one field, and empty skips it.
    tab();
    type_text(w, "Asha");
    render(w, width, height, &still("1-welcome-name"))?;
    enter();
    assert_eq!(ui.get_step(), 2, "Enter in the name field is Get started");
    assert_eq!(ui.get_named(), "Asha");
    key(w, slint::platform::Key::Escape.into());
    assert_eq!(ui.get_step(), 1, "Escape is Back, never closing the first run");
    tab();
    tab();
    tab();
    enter();
    assert_eq!(ui.get_step(), 2, "Get started, by keyboard");

    // 2. Who should do the thinking? Focus is on the first card.
    render(w, width, height, &still("2-who-thinks"))?;
    space();
    assert_eq!(ui.get_chosen_id(), "anthropic", "Space chooses the focused card");
    assert_eq!(ui.get_connect_mode(), "key");
    type_text(w, "sk-ant-test");
    render(w, width, height, &still("2-connect"))?;
    enter();
    assert_eq!(ui.get_submitted(), "anthropic:sk-ant-test", "the key field had focus and Enter connects");
    // The account lists its models and the person chooses one: nothing is chosen for them.
    let model = |id: &str, name: &str| FirstRunModel { id: id.into(), name: name.into() };
    ui.set_models(ModelRc::new(VecModel::from(vec![
        model("claude-opus-4-1", "Claude Opus 4.1"),
        model("claude-sonnet-4-5", "Claude Sonnet 4.5"),
        model("claude-haiku-4-5", "Claude Haiku 4.5"),
    ])));
    ui.set_connect_status("choose-model".into());
    ui.set_connect_words("It lists 3 models. Choose one.".into());
    render(w, width, height, &still("2-choose-model"))?;
    assert_eq!(ui.get_chosen_model(), "", "no model is pre-selected");
    tab();
    space();
    assert_eq!(ui.get_picked(), "claude-sonnet-4-5", "the person picks, by keyboard");
    ui.set_connect_status("pick-failed".into());
    ui.set_connect_words("Needs attention: claude-sonnet-4-5 answered, but your minds could not be set to use it (not one of the listed models)".into());
    render(w, width, height, &still("2-pick-failed"))?;
    enter();
    assert_eq!(ui.get_retries(), 1, "Try again has focus, and retries the pick");
    ui.set_connect_status("connected".into());
    ui.set_connect_words("Connected to Claude. Your minds will use claude-sonnet-4-5.".into());
    render(w, width, height, &still("2-connected"))?;
    ui.set_light(true);
    render(w, width, height, &still("2-connected-light"))?;
    ui.set_light(false);
    // Choose another: the list is back, and the full catalogue opens under it.
    ui.set_connect_mode("".into());
    ui.set_show_every(true);
    render(w, width, height, &still("2-every-provider"))?;
    ui.set_show_every(false);

    // 3. What your minds share.
    ui.set_step(3);
    ui.set_show_shared(true);
    render(w, width, height, &still("3-share"))?;
    assert!(ui.get_share(), "pre-checked");
    space();
    assert!(!ui.get_share(), "Space on the focused sentence unchecks it");
    render(w, width, height, &still("3-share-off"))?;
    space();
    assert!(ui.get_share());
    // Keep personal things on this computer: the sharing sentence shows unchecked.
    tab();
    tab();
    space();
    assert!(ui.get_private_lane(), "the private lane, by keyboard");
    assert!(!ui.get_share(), "the share sentence no longer looks checked");
    render(w, width, height, &still("3-private-lane"))?;
    key(w, slint::platform::Key::Escape.into());
    assert_eq!(ui.get_step(), 2, "Escape on step 3 goes Back");
    ui.set_private_lane(false);
    ui.set_share(true);

    // 4. Getting your minds ready.
    ui.set_step(4);
    ui.set_minds(ModelRc::new(VecModel::from(minds_checking())));
    render(w, width, height, &still("4-checking"))?;
    ui.set_minds(ModelRc::new(VecModel::from(minds_done())));
    ui.set_checks_done(true);
    render(w, width, height, &still("4-done"))?;
    ui.set_scale(1.5);
    render(w, width, height, &still("4-done-bigger"))?;
    ui.set_scale(1.0);
    enter();
    assert_eq!(ui.get_step(), 5, "Continue takes the focus once the checks are done, and Enter presses it");

    // 5. Your minds.
    ui.set_step(5);
    render(w, width, height, &still("5-minds"))?;
    enter();
    assert_eq!(ui.get_finishes(), 1, "Continue has focus on Your minds");
    ui.set_light(true);
    render(w, width, height, &still("5-minds-light"))?;
    ui.set_light(false);

    // The installer already chose: no second provider question.
    ui.hide()?;
    let ui2 = FirstRunProbe::new()?;
    ui2.set_installer_chose(true);
    ui2.show()?;
    render(w, width, height, &still("1-welcome-installer-chose"))?;
    tab();
    tab();
    enter();
    assert_eq!(ui2.get_step(), 4, "Get started goes to the checks when the installer chose");
    assert_eq!(ui2.get_check_starts(), 1, "and starts them");
    ui2.hide()?;

    // 6 and 7, in the chat beside the desktop.
    let chat = FirstRunChatProbe::new()?;
    chat.show()?;
    render(w, width, height, &still("6-first-success"))?;
    tab();
    tab();
    enter();
    assert_eq!(chat.get_talks(), 1, "Talk, by keyboard, reaches the microphone");
    let msg = |role: &str, content: &str| MessageData {
        role: role.into(),
        content: content.into(),
        is_streaming: false,
        blocks: ModelRc::new(VecModel::from(Vec::<ContentBlock>::new())),
        run: Default::default(),
        mind: Default::default(),
        model: Default::default(),
    };
    chat.set_messages(ModelRc::new(VecModel::from(vec![
        msg("user", "Make a note reminding me to call Sam"),
        msg("assistant", "Done. The note \"Call Sam\" is on your desktop."),
    ])));
    chat.set_generating(true);
    render(w, width, height, &still("6-answering"))?;
    chat.set_generating(false);
    // A reply that was not an answer keeps the person here, with the plain sentence and Try again.
    chat.set_outcome("failed".into());
    chat.set_failure("Yantrik Companion did not answer this time.".into());
    render(w, width, height, &still("6-not-answered"))?;
    enter();
    assert_eq!(chat.get_retries(), 1, "Try again takes the focus");
    chat.set_outcome("answered".into());
    chat.set_failure("".into());
    render(w, width, height, &still("7-ready"))?;

    println!("PASS first run: welcome, who thinks, sharing, checks, minds and the chat's first success, by keyboard");
    Ok(())
}
