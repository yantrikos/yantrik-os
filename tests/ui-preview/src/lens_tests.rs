//! A mind's finished turn in the Lens, drawn by the production MessageBubble from fixture data:
//! its tool calls one under another, the answer under them, and a call that opens when "show all"
//! is pressed (#211). Before, every block of a turn measured zero high — the calls were drawn on
//! top of each other and the answer never appeared — while the Agents pane showed the same turn.
use super::*;
use slint::{ModelRc, VecModel};

fn tool(summary: &str, arguments: &str) -> ContentBlock {
    ContentBlock {
        block_type: "tool".into(),
        text: summary.into(),
        call: ToolCallData {
            name: "os_act".into(),
            target: "".into(),
            summary: summary.into(),
            arguments: arguments.into(),
            status: "".into(),
            output: "".into(),
        },
    }
}

fn block(kind: &str, text: &str) -> ContentBlock {
    ContentBlock { block_type: kind.into(), text: text.into(), call: ToolCallData::default() }
}

fn turn(blocks: Vec<ContentBlock>) -> MessageData {
    MessageData {
        role: "assistant".into(),
        content: "".into(),
        is_streaming: false,
        blocks: ModelRc::new(VecModel::from(blocks)),
    }
}

/// The calls of the turn the live tour could not read: a note saved, the calendar read, a page.
fn calls() -> Vec<ContentBlock> {
    vec![
        tool(
            "os_act notes.create title=\"Groceries\"",
            "{\"app\": \"notes\", \"action\": \"create\", \"args\": {\"title\": \"Groceries\", \"body\": \"milk, eggs\"}}",
        ),
        tool("os_describe calendar", "{\"app\": \"calendar\"}"),
        tool(
            "os_act browser.read url=\"https://news.ycombinator.com\"",
            "{\"app\": \"browser\", \"action\": \"read\", \"args\": {\"url\": \"https://news.ycombinator.com\"}}",
        ),
    ]
}

/// A paragraph long enough to wrap onto several lines in the Lens, and the same news in one line.
const LONG: &str = "The top story on Hacker News this morning is a new release of the Rust compiler, \
    which makes incremental builds of large workspaces noticeably faster; the second is a long \
    essay on keeping small teams fast, and the third is a show-and-tell of a desktop written in Rust.";
const SHORT: &str = "The top story is a Rust release.";

/// Its answer, one block of each kind the Lens draws as text.
fn answer(say: bool, paragraph: &str) -> Vec<ContentBlock> {
    let t = |s: &str| if say { s.to_string() } else { String::new() };
    vec![
        block("heading", &t("Done")),
        block("bullet", &t("\u{2022} Saved the note \"Groceries\".\n\u{2022} Nothing is on your calendar today.")),
        block("text", &t(paragraph)),
        block("code", &t("git log --oneline -3")),
    ]
}

fn render(w: &MinimalSoftwareWindow, width: u32, height: u32) -> slint::SharedPixelBuffer<slint::Rgb8Pixel> {
    slint::platform::update_timers_and_animations();
    let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width, height);
    w.request_redraw();
    w.draw_if_needed(|r| { r.render(pixels.make_mut_slice(), width as usize); });
    pixels
}

fn save(pixels: &slint::SharedPixelBuffer<slint::Rgb8Pixel>, path: &str, width: u32, height: u32) -> Result<(), Box<dyn std::error::Error>> {
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(path)?), width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(pixels.as_bytes())?;
    println!("Rendered {path} ({width}×{height})");
    Ok(())
}

/// The bubble on its own: every block takes its own height, the answer's words are drawn, and a
/// call opens in place.
pub fn run(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (width, height) = (640u32, 800u32);
    let ui = MessageBubbleProbe::new()?;
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(width, height));
    let laid_out = |blocks: Vec<ContentBlock>| {
        ui.set_data(turn(blocks));
        render(w, width, height);
        ui.get_bubble_height()
    };

    // The bubble's own padding, top and bottom, and the layout's spacing between blocks.
    let (padding, spacing) = (8.0, 4.0);
    let one = laid_out(calls()[..1].to_vec());
    let card = one - padding;
    assert!(card >= 20.0, "a call's card is at least a line high, not {card}px (the bubble was {one}px)");
    let three = laid_out(calls());
    assert!(
        three - one >= 2.0 * (card + spacing) - 1.0,
        "three calls stand one under another: {three}px for three against {one}px for one"
    );
    let words = |say, paragraph| {
        let mut blocks = calls();
        blocks.extend(answer(say, paragraph));
        blocks
    };
    let short = laid_out(words(true, SHORT));
    assert!(
        short - three >= 60.0,
        "the answer (a heading, two bullets, a sentence, a command) takes room under the calls: {short}px against {three}px"
    );
    // A paragraph that wraps keeps every line it wraps to. Measured at its unwrapped width, it
    // was given one line and the rest were cut off (#197 found the same in the Agents pane).
    let answered = laid_out(words(true, LONG));
    assert!(
        answered - short >= 30.0,
        "a paragraph wrapped onto three lines takes three lines: {answered}px against {short}px for one"
    );

    // Its words are on screen: the same turn with the answer's text emptied draws differently.
    let drawn = render(w, width, height);
    ui.set_data(turn(words(false, LONG)));
    let blank = render(w, width, height);
    let differ = drawn
        .as_slice()
        .iter()
        .zip(blank.as_slice())
        .filter(|(a, b)| a != b)
        .count();
    assert!(differ >= 300, "the answer's words are drawn: only {differ} pixels change when they are emptied");

    ui.set_data(turn(words(true, LONG)));
    save(&render(w, width, height), output, width, height)?;

    // "show all" on the first call opens it where it stands, and the turn grows to hold it.
    let before = ui.get_bubble_height();
    crate::click(w, 300.0, 4.0 + card / 2.0);
    render(w, width, height);
    let after = ui.get_bubble_height();
    assert!(after > before + 10.0, "\"show all\" opens the call's arguments: {after}px against {before}px");
    save(&render(w, width, height), &output.replace(".png", "-open.png"), width, height)?;

    println!(
        "PASS: a finished turn stands one call under another ({one}px, {three}px), its answer is drawn under them with every wrapped line ({short}px, {answered}px, {differ} pixels), and show all opens a call ({before}px to {after}px)"
    );
    Ok(())
}

/// The whole Lens with the person's question and the turn, for the picture a person sees.
pub fn run_lens(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (width, height) = (1280u32, 800u32);
    let ui = LensAnswerProbe::new()?;
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(width, height));
    let mut blocks = calls();
    blocks.extend(answer(true, LONG));
    let question = MessageData {
        role: "user".into(),
        content: "Save a note called Groceries, tell me what is on my calendar, and the top story on Hacker News".into(),
        is_streaming: false,
        blocks: ModelRc::new(VecModel::from(Vec::<ContentBlock>::new())),
    };
    ui.set_messages(ModelRc::new(VecModel::from(vec![question, turn(blocks)])));
    render(w, width, height);
    // Past the panel's slide-in.
    std::thread::sleep(std::time::Duration::from_millis(300));
    save(&render(w, width, height), output, width, height)?;
    Ok(())
}
