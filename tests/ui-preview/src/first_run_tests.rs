//! The first run (design/experience-final-2026-10-10.md, "First run, minute by minute"), drawn
//! from the production FirstRunScreen and the production chat, and driven with real key events:
//!
//! - Welcome: on the wallpaper, centred, with what Yantrik is for; focus starts on Make text bigger; Space makes the text bigger live (1.25, 1.5,
//!   then back to normal); Tab reaches Read this to me, whose Enter speaks the screen's words;
//!   Tab reaches Get started, whose Enter goes to "Who should do the thinking?".
//!   A first boot the installer did not set up asks the person's name: one field, Enter goes on.
//!   Escape is Back on every screen, never closing the first run.
//! - Who thinks: what works without an account leads (found on the network, this computer, the
//!   free one), one card Recommended with its reason, never a model on another computer, whose
//!   card says the conversations go there; the network is looked at only on the
//!   person's say-so; focus starts on the first card; Space chooses it; the key field takes focus and
//!   typing then Enter hands the shell the provider and the key; the account's models are listed
//!   with none chosen, and the person picks one by keyboard; a pick that did not take offers Try
//!   again; "Show every provider" opens the full list; Advanced needs only the address, and
//!   Enter on it lists its models.
//! - What your minds share: the sentence names the provider and is pre-checked; Space unchecks
//!   it and the screen says what that means; choosing Keep personal things on this computer
//!   shows the sentence unchecked.
//! - Getting ready and Your minds: the companion's three rows, every one a check that runs, in
//!   every state; Hermes's own install is said and read out.
//! - The installer already chose: Get started goes straight to the checks, and starts them.
//! - The chat's first success: the cursor in the composer as it opens; three first tasks, each
//!   sent with one press; Type · Talk, and Talk reaches the microphone; a reply that was not an
//!   answer says so with Try again; only a real answer brings "You're ready." with Done.
//! - The desktop under the open chat: the home card ends before the chat begins.
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
        recommended: "".into(),
        caution: "".into(),
    }
}

/// The cards as wire/onboarding.rs makes them for a person with no account: a model found on
/// another computer first (never Recommended, and saying where conversations go), this
/// computer's, the free one (Recommended), then the accounts.
fn featured() -> Vec<FirstRunProvider> {
    vec![
        FirstRunProvider {
            caution: "Your conversations go to that computer. Choose it only if it's yours.".into(),
            ..provider(
                "nearby:http://192.168.1.5:8080/v1",
                "On 192.168.1.5, another computer on this network",
                "qwen3.8-flash on 192.168.1.5:8080",
                "already running, needs no account",
                "network",
                "",
                false,
            )
        },
        provider("this-computer", "On this computer", "private, slower", "already here (2.7 GB)", "this-computer", "", false),
        FirstRunProvider {
            recommended: "Recommended: free to start, and it does not train on what you send".into(),
            ..provider("groq", "Groq", "free, fast open models", "needs a free Groq account; 1,000 requests and 200K tokens a day per model", "free", "gsk_…", true)
        },
        provider("anthropic", "Claude", "online, careful with words", "needs an Anthropic account", "cloud", "sk-ant-…", true),
        provider("openai", "OpenAI", "online, a good all-rounder", "needs an OpenAI account", "cloud", "sk-…", true),
        provider("gemini", "Google Gemini", "online, reads long documents", "needs a Google account", "cloud", "AIza…", true),
        provider("deepseek", "DeepSeek", "online, strong at reasoning, low cost", "needs a DeepSeek account", "cloud", "sk-…", true),
        provider("openrouter", "OpenRouter", "online, one account for many models", "needs an OpenRouter account", "aggregator", "sk-or-…", true),
        provider("nvidia-nim", "NVIDIA NIM", "online, many open models", "needs an NVIDIA account", "cloud", "nvapi-…", true),
    ]
}

/// The same screen before anything was found: the free one leads, marked Recommended.
fn featured_none_found() -> Vec<FirstRunProvider> {
    featured().into_iter().filter(|c| c.group != "network" && c.group != "this-computer").collect()
}

/// A stand-in for the catalogue's full list: its length and order are the production's concern
/// (wire/onboarding.rs tests that every provider is there); here it only has to draw.
fn every() -> Vec<FirstRunProvider> {
    let mut v: Vec<FirstRunProvider> = featured().into_iter().filter(|c| c.group != "network" && c.group != "this-computer").collect();
    for (id, name) in [("mistral", "Mistral"), ("xai", "xAI"), ("ollama", "Ollama"), ("llamacpp", "llama.cpp")] {
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

/// What the real checks report (wire/onboarding_proofs.rs): the built-in companion's three rows,
/// all of them checks that ran. The default mind, Hermes, says how its own install is going.
fn minds_done() -> Vec<FirstRunMind> {
    vec![
        mind("Yantrik Companion", "Ready", true, vec![
            check("answer", "Can it answer?", "passed", "Passed", "Yantrik Companion answered in 840 ms: “OK”"),
            check("remember", "Can it remember?", "passed", "Passed", "wrote a note to its memory, recalled it by its words (1 found), deleted it"),
            check("desktop", "Can it reach the desktop?", "passed", "Passed", "described the desktop in 12 ms: 31 actions it can take"),
        ]),
    ]
}

/// Where each first task's row is drawn in the chat at 1280×800 (the first wraps), and Talk
/// (read from 6-first-success.png).
const FIRST_TASKS_AT: [(f32, f32); 3] = [(1000.0, 316.0), (1000.0, 378.0), (1000.0, 430.0)];
const TALK_AT: (f32, f32) = (956.0, 479.0);

const HERMES_SETTING_UP: &str = "Setting up Hermes, your mind, about 4 minutes. It needs the internet.";

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
    ui.set_featured(ModelRc::new(VecModel::from(featured_none_found())));
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

    // 1. Welcome: on the wallpaper, centred, what Yantrik is for and what the next minutes do.
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
    assert!(!ui.get_spoken().contains("Your files are safe"), "nothing that makes no sense on a new machine");
    assert!(ui.get_spoken().contains("you decide what they may do"), "{}", ui.get_spoken());
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

    // 2. Who should do the thinking? Nothing found yet: the free one leads, Recommended with its
    // reason, and the look on the network waits for the person's say-so. Focus is on the first card.
    render(w, width, height, &still("2-who-thinks"))?;
    ui.invoke_read();
    assert!(ui.get_spoken().contains("Recommended: free to start"), "the recommendation is read out: {}", ui.get_spoken());
    space();
    assert_eq!(ui.get_chosen_id(), "groq", "Space chooses the focused card, the free one");
    assert_eq!(ui.get_connect_mode(), "key");
    type_text(w, "gsk_test");
    render(w, width, height, &still("2-connect"))?;
    enter();
    assert_eq!(ui.get_submitted(), "groq:gsk_test", "the key field had focus and Enter connects");
    // The account lists its models and the person chooses one: nothing is chosen for them.
    let model = |id: &str, name: &str| FirstRunModel { id: id.into(), name: name.into() };
    ui.set_models(ModelRc::new(VecModel::from(vec![
        model("openai/gpt-oss-120b", "openai/gpt-oss-120b"),
        model("qwen/qwen3.8-27b", "qwen/qwen3.8-27b"),
        model("openai/gpt-oss-20b", "openai/gpt-oss-20b"),
    ])));
    ui.set_connect_status("choose-model".into());
    ui.set_connect_words("It lists 3 models. Choose one.".into());
    render(w, width, height, &still("2-choose-model"))?;
    assert_eq!(ui.get_chosen_model(), "", "no model is pre-selected");
    tab();
    space();
    assert_eq!(ui.get_picked(), "qwen/qwen3.8-27b", "the person picks, by keyboard");
    ui.set_connect_status("pick-failed".into());
    ui.set_connect_words("Needs attention: qwen/qwen3.8-27b answered, but your minds could not be set to use it (not one of the listed models)".into());
    render(w, width, height, &still("2-pick-failed"))?;
    enter();
    assert_eq!(ui.get_retries(), 1, "Try again has focus, and retries the pick");
    ui.set_connect_status("connected".into());
    ui.set_connect_words("Connected to Groq. Your minds will use qwen/qwen3.8-27b.".into());
    ui.set_default_mind_words(HERMES_SETTING_UP.into());
    render(w, width, height, &still("2-connected"))?;
    ui.set_light(true);
    render(w, width, height, &still("2-connected-light"))?;
    ui.set_light(false);
    // Choose another: the list is back. The person asks to look on the network; what was found
    // leads, saying the conversations would go to that computer, the free one keeps Recommended,
    // and the look's words say what it found.
    ui.set_connect_mode("".into());
    ui.set_connect_status("".into());
    ui.set_connect_words("".into());
    ui.set_chosen_id("".into());
    ui.set_nearby_words("Asks the other computers on your network, on Ollama's and llama.cpp's usual ports, for a few seconds.".into());
    ui.set_nearby_status("looking".into());
    ui.set_nearby_words("Looking on your network (started 2:14)".into());
    render(w, width, height, &still("2-looking"))?;
    ui.set_featured(ModelRc::new(VecModel::from(featured())));
    ui.set_nearby_status("found".into());
    ui.set_nearby_words("Found one model server on your network. It is first in the list.".into());
    render(w, width, height, &still("2-found-on-network"))?;
    ui.set_nearby_status("".into());
    ui.set_nearby_words("".into());
    ui.set_show_every(true);
    render(w, width, height, &still("2-every-provider"))?;
    ui.set_show_every(false);
    // Advanced: the endpoint is enough. Enter on it lists its models; an id can still be typed.
    // Choosing a card clears the last one's models, as choose_provider does.
    ui.set_models(ModelRc::new(VecModel::from(Vec::<FirstRunModel>::new())));
    ui.set_connect_mode("advanced".into());
    ui.set_chosen_id("custom".into());
    ui.set_chosen_name("your endpoint".into());
    render(w, width, height, &still("2-advanced"))?;
    type_text(w, "http://192.168.1.5:8080/v1");
    enter();
    assert_eq!(ui.get_custom_submitted(), "http://192.168.1.5:8080/v1||", "Enter on the address asks for its models, no id needed");
    ui.set_models(ModelRc::new(VecModel::from(vec![model("qwen3.8-flash-next-unsloth-ud-iq4_xs", "qwen3.8-flash-next-unsloth-ud-iq4_xs")])));
    ui.set_connect_status("choose-model".into());
    ui.set_connect_words("It lists one model. Choose it to go on.".into());
    render(w, width, height, &still("2-advanced-models"))?;
    ui.set_models(ModelRc::new(VecModel::from(Vec::<FirstRunModel>::new())));
    ui.set_connect_status("connected".into());
    ui.set_connect_mode("key".into());

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

    // 4. Getting your minds ready: every row a check that runs, and Hermes's own install said.
    ui.set_step(4);
    ui.set_minds(ModelRc::new(VecModel::from(minds_checking())));
    render(w, width, height, &still("4-checking"))?;
    ui.set_minds(ModelRc::new(VecModel::from(minds_done())));
    ui.set_checks_done(true);
    render(w, width, height, &still("4-done"))?;
    ui.invoke_read();
    assert!(ui.get_spoken().contains("Setting up Hermes"), "Hermes setting itself up is read out too: {}", ui.get_spoken());
    ui.set_scale(1.5);
    render(w, width, height, &still("4-done-bigger"))?;
    ui.set_scale(1.0);
    // Every Tab stop on the checks shows its ring, and Tab and Shift+Tab go round the screen's own
    // buttons and nothing else (14 Oct: a stop with no ring, where Enter did nothing). The ring
    // is drawn by the button that has the focus; "" here is the screen itself, which draws none.
    assert_eq!(ui.get_focused_label(), "Continue", "Continue has the focus once the checks are done");
    let round = ["Make text bigger", "Read this to me", "Details", "Details", "Details", "Check again", "Continue"];
    for (direction, order) in [("Tab", round.to_vec()), ("Shift+Tab", round.iter().rev().skip(1).chain(["Continue"].iter()).copied().collect())] {
        let mut seen = Vec::new();
        for _ in 0..round.len() {
            if direction == "Tab" { tab() } else { key(w, slint::platform::Key::Backtab.into()) }
            seen.push(ui.get_focused_label().to_string());
        }
        assert_eq!(seen, order, "{direction} goes round the checks screen's buttons, each with its ring");
    }
    assert_eq!(ui.get_step(), 4, "going round pressed nothing");
    enter();
    assert_eq!(ui.get_step(), 5, "Enter on the focused Continue continues");
    // Reached by Tab, not only by being given the focus.
    ui.set_step(4);
    tab();
    key(w, slint::platform::Key::Backtab.into());
    assert_eq!(ui.get_focused_label(), "Continue");
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
    // Opened as the first run opens it: chat mode, then open. The composer takes the cursor a
    // tick later, so typing works without a click (13 Oct: it took no keys until clicked).
    let chat = FirstRunChatProbe::new()?;
    chat.set_open(false);
    chat.show()?;
    chat.set_open(true);
    slint::platform::update_timers_and_animations();
    std::thread::sleep(std::time::Duration::from_millis(80));
    render(w, width, height, &still("6-first-success"))?;
    type_text(w, "hello");
    enter();
    assert_eq!(chat.get_sent(), "hello", "the cursor is in the composer when the chat opens");
    // Each first task is one press: it is sent as it is.
    for ((x, y), task) in FIRST_TASKS_AT.into_iter().zip([
        "Make a note called Groceries with milk, eggs and bread",
        "What's on my calendar this week?",
        "Open Files and show me what's in Documents",
    ]) {
        click(w, x, y);
        assert_eq!(chat.get_sent(), task);
    }
    click(w, TALK_AT.0, TALK_AT.1);
    assert_eq!(chat.get_talks(), 1, "Talk reaches the microphone");
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
    chat.hide()?;

    // The desktop under the open chat: the home card ends before the chat begins.
    let desk = DesktopChatProbe::new()?;
    desk.show()?;
    render(w, width, height, &still("8-desktop-chat-open"))?;
    assert!(desk.get_home_right() <= desk.get_chat_left(), "the home card ({}) runs under the chat ({})", desk.get_home_right(), desk.get_chat_left());
    desk.set_chat_open(false);
    render(w, width, height, &still("8-desktop-chat-closed"))?;
    assert_eq!(desk.get_home_right(), (width as f32 + 760.0) / 2.0, "closed, the home card is centred as before");
    desk.hide()?;

    println!("PASS first run: welcome, who thinks, sharing, checks, minds and the chat's first success, by keyboard");
    Ok(())
}
