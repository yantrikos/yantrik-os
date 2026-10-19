//! Chat v2 (the conversation panel), drawn by the production Lens from fixture data: a conversation
//! with a pending approval, the approval once resolved, the empty state with its starters, the
//! transcript's follow-the-bottom rule with its "New reply" pill, the composer's keys, a reply's
//! timeline (running, waiting on a question card, answered), a queued message, and the panel's
//! drag-to-resize. Pictures are written beside `output`.
use super::*;
use slint::platform::{Key, WindowEvent};
use slint::{Model, ModelRc, SharedString, VecModel};

type Pixels = slint::SharedPixelBuffer<slint::Rgb8Pixel>;

const W: u32 = 1280;
const H: u32 = 800;
// The panel: 12px from the right edge and 12px under the 32px status bar, 440px wide by default.
const PANEL_X: f32 = 1280.0 - 440.0 - 12.0;

/// Frames until nothing moves: the panel's slide-in is 200ms, and the follower settles on the
/// frame after a change.
fn settle(w: &MinimalSoftwareWindow) -> Pixels {
    let mut p = Pixels::new(W, H);
    for _ in 0..6 {
        std::thread::sleep(std::time::Duration::from_millis(60));
        slint::platform::update_timers_and_animations();
        w.request_redraw();
        w.draw_if_needed(|r| { r.render(p.make_mut_slice(), W as usize); });
    }
    p
}

fn save(p: &Pixels, path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut e = png::Encoder::new(BufWriter::new(File::create(path)?), W, H);
    e.set_color(png::ColorType::Rgb);
    e.set_depth(png::BitDepth::Eight);
    e.write_header()?.write_image_data(p.as_bytes())?;
    Ok(())
}

fn named(output: &str, tag: &str) -> String {
    output.replace(".png", &format!("-{tag}.png"))
}

fn msg(role: &str, content: &str, run: &str) -> MessageData {
    MessageData {
        role: role.into(),
        content: content.into(),
        is_streaming: false,
        blocks: ModelRc::new(VecModel::from(Vec::<ContentBlock>::new())),
        run: run.into(),
        mind: Default::default(),
        model: Default::default(),
    }
}

fn lines(rows: &[&str]) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(rows.iter().map(|r| SharedString::from(*r)).collect::<Vec<_>>()))
}

/// One line of a reply's timeline, as `lens_timeline::blocks` makes it.
fn line(kind: &str, text: &str, status: &str) -> ContentBlock {
    ContentBlock { block_type: kind.into(), text: text.into(), call: ToolCallData { status: status.into(), ..Default::default() } }
}

/// The Dallas weekend turn on VM 541 as it went, step by step.
fn asking() -> MessageData {
    msg("user", "What will the weather be in Dallas this weekend?", "")
}

fn hermes_reply(content: &str, streaming: bool, blocks: Vec<ContentBlock>) -> MessageData {
    MessageData {
        is_streaming: streaming,
        blocks: ModelRc::new(VecModel::from(blocks)),
        mind: "Hermes Agent".into(),
        model: "deepseek-v4.1-flash".into(),
        ..msg("assistant", content, "")
    }
}

fn weekend_question() -> QuestionRequest {
    QuestionRequest {
        agent: "hermes:main".into(),
        mind: "Hermes Agent".into(),
        request: "c1".into(),
        prompt: "Which days is this weekend for you?".into(),
        options: lines(&["Sat Oct 17 + Sun Oct 18", "Just the next 2 days (Thu/Fri)"]),
        asked: "09:14".into(),
        removes: "".into(),
        recommended: 0,
        in_reply: true,
    }
}

fn approval(decision: &str, record: &str, decided_at: &str) -> ApprovalRequest {
    ApprovalRequest {
        id: "appr-31".into(),
        agent: "pi:main".into(),
        requester: "pi 0.87".into(),
        verified: "pi --mode rpc (pid 4242) \u{b7} the attached mind".into(),
        discrepancies: lines(&[]),
        app: "files".into(),
        action: "move".into(),
        summary: "Move files or folders to another place.".into(),
        purpose: "Move files or folders to another place.".into(),
        grade: "sensitive".into(),
        args: lines(&["from: ~/Pictures/2024", "to: ~/Pictures/By date"]),
        can_session: true,
        decision: decision.into(),
        record: record.into(),
        identity: "Caller process confirmed: node \u{b7} PID 4242 \u{b7} the attached mind pi".into(),
        identity_tag: "".into(),
        claim: "Claimed name: \u{201c}pi 0.87\u{201d}".into(),
        confirm_label: "Allow once".into(),
        what: "Moves: from: ~/Pictures/2024; to: ~/Pictures/By date".into(),
        exactly: "from: ~/Pictures/2024; to: ~/Pictures/By date".into(),
        age_text: if decision.is_empty() { "Expires in 2 min, then declined".into() } else { "".into() },
        decided_at: decided_at.into(),
        ..Default::default()
    }
}

fn conversation() -> Vec<MessageData> {
    vec![
        msg("user", "Tidy the photos folder and tell me what you moved.", ""),
        msg(
            "assistant",
            "I\u{2019}ll group the photos by the date they were taken. I\u{2019}ll ask before anything is moved.",
            "pi:main#3",
        ),
    ]
}

fn key(w: &MinimalSoftwareWindow, text: impl Into<SharedString>) {
    let text = text.into();
    w.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    w.dispatch_event(WindowEvent::KeyReleased { text });
}

fn shift_enter(w: &MinimalSoftwareWindow) {
    w.dispatch_event(WindowEvent::KeyPressed { text: Key::Shift.into() });
    key(w, Key::Return);
    w.dispatch_event(WindowEvent::KeyReleased { text: Key::Shift.into() });
}

/// Empty the focused field from the keyboard: Ctrl+A, then Backspace. Whatever an earlier step put
/// there (step 3's starter fills the composer), wherever the field happens to be drawn.
fn clear_field(w: &MinimalSoftwareWindow) {
    w.dispatch_event(WindowEvent::KeyPressed { text: Key::Control.into() });
    key(w, "a");
    w.dispatch_event(WindowEvent::KeyReleased { text: Key::Control.into() });
    key(w, Key::Backspace);
}

fn wheel(w: &MinimalSoftwareWindow, delta_y: f32) {
    w.dispatch_event(WindowEvent::PointerScrolled {
        position: slint::LogicalPosition::new(PANEL_X + 220.0, 300.0),
        delta_x: 0.0,
        delta_y,
    });
}

/// A press-move-release along a horizontal line, the way a person drags an edge.
fn drag(w: &MinimalSoftwareWindow, from: f32, to: f32, y: f32) {
    use slint::platform::PointerEventButton;
    let at = |x: f32| slint::LogicalPosition::new(x, y);
    w.dispatch_event(WindowEvent::PointerMoved { position: at(from) });
    w.dispatch_event(WindowEvent::PointerPressed { position: at(from), button: PointerEventButton::Left });
    let steps = 8;
    for i in 1..=steps {
        w.dispatch_event(WindowEvent::PointerMoved { position: at(from + (to - from) * i as f32 / steps as f32) });
        settle_one(w);
    }
    w.dispatch_event(WindowEvent::PointerReleased { position: at(to), button: PointerEventButton::Left });
}

fn settle_one(w: &MinimalSoftwareWindow) {
    let mut p = Pixels::new(W, H);
    slint::platform::update_timers_and_animations();
    w.request_redraw();
    w.draw_if_needed(|r| { r.render(p.make_mut_slice(), W as usize); });
}

fn differ(a: &Pixels, b: &Pixels) -> usize {
    a.as_slice().iter().zip(b.as_slice()).filter(|(x, y)| x != y).count()
}

pub fn run(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let ui = ChatProbe::new()?;
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(W, H));
    // The composer row as VM 541 showed it (14 Oct): Hermes on a Custom model, connected.
    let picker = ui.global::<PickerState>();
    picker.set_mind_label("Hermes Agent".into());
    picker.set_model_label("qwen3.5:27b \u{b7} Custom".into());
    picker.set_efforts(ModelRc::new(VecModel::from(vec![SharedString::from("easy"), "medium".into(), "hard".into()])));
    picker.set_status_words("connected \u{b7} qwen3.5:27b".into());
    picker.set_status_state("connected".into());

    // ── 1. A conversation with a pending approval ──
    ui.set_messages(ModelRc::new(VecModel::from(conversation())));
    ui.set_live_runs(1);
    ui.set_waiting_runs(1);
    ui.set_approvals(ModelRc::new(VecModel::from(vec![approval("", "", "")])));
    let waiting = settle(w);
    save(&waiting, &named(output, "approval"))?;
    // No default key answers a request: Enter, with the cursor wherever it is, grants and refuses
    // nothing. (The buttons are pointer-only, which is the whole mechanism.)
    key(w, Key::Return);
    settle(w);
    assert_eq!((ui.get_allowed(), ui.get_denied()), (0, 0), "Enter must not answer an approval");
    assert_eq!(ui.get_sends(), 0, "Enter in an empty composer sends nothing");

    // ── 1b. The card does not move under the pointer (review of #580) ──
    //
    // The line naming a waiting question comes and goes above the reply box while a request
    // waits, and the box grows as a person types. Neither may move the card or resize it, so the
    // rows the card is drawn in are the same pixels in every state.
    let card_rows = |p: &Pixels| -> Vec<slint::Rgb8Pixel> {
        let from = (112 * W + PANEL_X as u32) as usize;
        (0..230usize)
            .flat_map(|row| p.as_slice()[from + row * W as usize..from + row * W as usize + 440].to_vec())
            .collect()
    };
    let with_strip = card_rows(&waiting);
    ui.set_composer_asks("Hermes Agent asks: Which days is this weekend for you?".into());
    ui.set_live_runs(0);
    ui.set_waiting_runs(0);
    assert!(
        card_rows(&settle(w)) == with_strip,
        "the card is drawn in the same place with and without the question line"
    );
    ui.set_composer_asks("".into());
    key(w, "a");
    for _ in 0..6 {
        shift_enter(w);
    }
    assert!(
        card_rows(&settle(w)) == with_strip,
        "the card is drawn in the same place while the reply box grows"
    );
    for _ in 0..8 {
        key(w, Key::Backspace);
    }
    settle(w);
    ui.set_live_runs(1);
    ui.set_waiting_runs(1);

    // ── 1c. Sensitive and standard requests do not look alike ──
    let mut standard = approval("", "", "");
    standard.grade = "standard".into();
    ui.set_approvals(ModelRc::new(VecModel::from(vec![standard])));
    let standard_card = settle(w);
    save(&standard_card, &named(output, "approval-standard"))?;
    let mut dangerous = approval("", "", "");
    dangerous.grade = "dangerous".into();
    ui.set_approvals(ModelRc::new(VecModel::from(vec![dangerous])));
    let dangerous_card = settle(w);
    assert!(differ(&waiting, &standard_card) > 300, "a standard request is not drawn like a sensitive one");
    assert!(differ(&waiting, &dangerous_card) > 300, "nor is a dangerous one");
    assert!(differ(&standard_card, &dangerous_card) > 300, "and the three are three");
    ui.set_approvals(ModelRc::new(VecModel::from(vec![approval("", "", "")])));
    settle(w);

    // ── 2. The same request, once resolved: one collapsed line ──
    ui.set_approvals(ModelRc::new(VecModel::from(vec![approval(
        "allowed",
        "Allowed once: files.move \u{2014} 10:42",
        "10:42",
    )])));
    ui.set_live_runs(0);
    ui.set_waiting_runs(0);
    let resolved = settle(w);
    save(&resolved, &named(output, "approved"))?;
    assert!(differ(&waiting, &resolved) > 2000, "a resolved approval draws differently from a waiting one");
    // "View action" opens the run of the agent that asked. Found by clicking where a person would.
    let mut opened = false;
    'scan: for y in (100..700).step_by(4) {
        for x in (PANEL_X as i32 + 150..PANEL_X as i32 + 420).step_by(16) {
            ui.set_opened_run("".into());
            crate::click(w, x as f32, y as f32);
            if ui.get_opened_run() == "pi:main" {
                opened = true;
                break 'scan;
            }
        }
    }
    assert!(opened, "\"View action\" on a resolved approval opens the agent that asked");
    // The line names the action and how long the grant lasts: a session rule reads differently
    // from a one-off (review of #580).
    let once = settle(w);
    let mut session = approval("allowed", "Allowed for this session: files.move \u{2014} 10:42", "10:42");
    session.session = true;
    ui.set_approvals(ModelRc::new(VecModel::from(vec![session])));
    let for_session = settle(w);
    save(&for_session, &named(output, "approved-session"))?;
    assert!(differ(&once, &for_session) > 50, "\"(this session)\" is drawn where \"(once)\" was");
    let card = include_str!("../../../crates/yantrik-ui-slint/ui/components/intent_lens.slint");
    assert!(card.contains("(this session)") && card.contains("(once)") && card.contains("root.data.app + \".\" + root.data.action"));
    assert_eq!((ui.get_allowed(), ui.get_denied()), (0, 0), "and pressing it answers nothing");

    // ── 3. The empty state, and a starter that fills the composer without sending ──
    ui.set_messages(ModelRc::new(VecModel::from(Vec::<MessageData>::new())));
    ui.set_approvals(ModelRc::new(VecModel::from(Vec::<ApprovalRequest>::new())));
    let empty = settle(w);
    save(&empty, &named(output, "empty"))?;
    let mut filled = false;
    'starters: for y in (380..560).step_by(6) {
        crate::click(w, PANEL_X + 120.0, y as f32);
        let after = settle(w);
        // The composer is the bottom 112px of the panel: words in the box change it.
        if differ(&empty, &after) > 150 {
            save(&after, &named(output, "starter"))?;
            filled = true;
            break 'starters;
        }
    }
    assert!(filled, "a starter puts words in the composer");
    assert_eq!(ui.get_sends(), 0, "and never sends them by itself");
    // The person thinks better of it: select all and delete, and the box is as it was. Without
    // this the starter's words stayed in the box for the rest of the scene, and the "hello" typed
    // in section 5 went out as "Help me plan hello". The app was right (the scene had chosen
    // "Plan something"); the scene never put the box back.
    w.dispatch_event(WindowEvent::KeyPressed { text: Key::Control.into() });
    key(w, "a");
    w.dispatch_event(WindowEvent::KeyReleased { text: Key::Control.into() });
    key(w, Key::Backspace);
    // The box only: the starter keeps its hover fill under the pointer that pressed it.
    let composer_rows = |p: &Pixels| -> Vec<slint::Rgb8Pixel> {
        (H as usize - 172..H as usize - 60)
            .flat_map(|row| p.as_slice()[row * W as usize + PANEL_X as usize..row * W as usize + PANEL_X as usize + 440].to_vec())
            .collect()
    };
    let (before, after) = (composer_rows(&empty), composer_rows(&settle(w)));
    let changed = before.iter().zip(&after).filter(|(a, b)| a != b).count();
    assert!(changed < 150, "select all and delete empties the box a starter filled ({changed} pixels differ)");

    // ── 4. Following the bottom, and the New reply pill ──
    let model = Rc::new(VecModel::from(
        (0..30)
            .map(|i| {
                if i % 2 == 0 {
                    msg("user", &format!("Question number {i}, with enough words to be a real message."), "")
                } else {
                    msg("assistant", &format!("Answer number {i}. It takes a couple of lines so that thirty of them are taller than the panel."), "")
                }
            })
            .collect::<Vec<_>>(),
    ));
    ui.set_messages(ModelRc::from(model.clone()));
    settle(w);
    let at_bottom = ui.get_scroll_y();
    assert!(at_bottom < -100.0, "thirty messages are taller than the panel ({at_bottom})");
    assert!(ui.get_following(), "opened at the bottom, the transcript follows");

    model.push(msg("assistant", "A new reply while you are at the bottom.", ""));
    save(&settle(w), &named(output, "follows"))?;
    assert!(ui.get_scroll_y() < at_bottom - 20.0, "a new message at the bottom is followed: {} -> {}", at_bottom, ui.get_scroll_y());
    assert!(ui.get_following() && !ui.get_missed());

    // The last message grows as it streams: still followed.
    let before = ui.get_scroll_y();
    let last = model.row_count() - 1;
    let mut m = model.row_data(last).unwrap();
    m.content = "A new reply while you are at the bottom. It keeps arriving, word after word, until it takes four or five lines of the panel to say it all, as a streaming answer does.".into();
    model.set_row_data(last, m);
    settle(w);
    assert!(ui.get_scroll_y() < before - 10.0 && ui.get_following(), "a reply that grows is followed: {} -> {}", before, ui.get_scroll_y());

    // Scrolled up to read: a new reply leaves them where they are and says so.
    wheel(w, 20000.0);
    settle(w);
    assert!(!ui.get_following() && ui.get_scroll_y() > -2.0, "scrolled to the top: following {}, y {}", ui.get_following(), ui.get_scroll_y());
    model.push(msg("assistant", "A reply that arrives while you are reading further up.", ""));
    let reading = settle(w);
    save(&reading, &named(output, "new-reply"))?;
    assert!(ui.get_scroll_y() > -2.0, "a reply does not pull the reader away: y {}", ui.get_scroll_y());
    assert!(ui.get_missed() && !ui.get_following(), "and the pill says something is new");
    // The pill sits at the bottom of the transcript, centred; found by clicking up the middle.
    let mut returned = false;
    for y in (400..690).rev().step_by(3) {
        crate::click(w, PANEL_X + 220.0, y as f32);
        settle(w);
        if ui.get_following() {
            returned = true;
            break;
        }
    }
    assert!(returned && !ui.get_missed(), "\"New reply\" returns to the newest line and follows again");
    let newest = ui.get_scroll_y();
    assert!(newest < -100.0, "and it is at the bottom again ({newest})");

    // ── 5. The composer: Enter sends, Shift+Enter is a new line, and sending follows ──
    wheel(w, 20000.0);
    settle(w);
    assert!(!ui.get_following());
    // The words, not the picker row under them (the mind chip is at 690 once it has its label).
    crate::click(w, PANEL_X + 150.0, 645.0);
    // Step 3's starter left its words in the box (a starter fills and never sends); a person
    // clears them before writing their own.
    clear_field(w);
    settle(w);
    for c in "hello".chars() {
        key(w, c.to_string());
    }
    key(w, Key::Return);
    settle(w);
    // Exactly what was typed: no starter's words in front of it.
    assert_eq!((ui.get_sends(), ui.get_sent().as_str()), (1, "hello"), "Enter sends the message");
    assert!(ui.get_following(), "and sending a message goes to the bottom, where the reply will be");
    for c in "a".chars() {
        key(w, c.to_string());
    }
    shift_enter(w);
    for c in "b".chars() {
        key(w, c.to_string());
    }
    settle(w);
    assert_eq!(ui.get_sends(), 1, "Shift+Enter breaks the line and sends nothing");
    key(w, Key::Return);
    settle(w);
    assert_eq!((ui.get_sends(), ui.get_sent().as_str()), (2, "a\nb"), "the two lines go together");

    // ── 6. One reply, one timeline: running, a question card in it, answered, and a queued message ──
    timeline(w, &ui, output, &reading)?;
    ui.set_panel_width(380.0);
    timeline(w, &ui, &named(output, "narrow"), &reading)?;
    ui.set_panel_width(440.0);

    // ── 7. Resizable from 380 to 560, 440 by default ──
    ui.set_messages(ModelRc::new(VecModel::from(conversation())));
    settle(w);
    assert!((ui.get_panel_width() - 440.0).abs() < 0.5, "440px by default, not {}", ui.get_panel_width());
    drag(w, PANEL_X + 3.0, PANEL_X - 57.0, 300.0);
    settle(w);
    assert!((ui.get_panel_width() - 500.0).abs() < 2.0, "dragged 60px wider: {}", ui.get_panel_width());
    let wide_x = 1280.0 - ui.get_panel_width() - 12.0;
    drag(w, wide_x + 3.0, wide_x - 400.0, 300.0);
    settle(w);
    assert!((ui.get_panel_width() - 560.0).abs() < 0.5, "no wider than 560: {}", ui.get_panel_width());
    let wider_x = 1280.0 - 560.0 - 12.0;
    drag(w, wider_x + 3.0, wider_x + 600.0, 300.0);
    settle(w);
    assert!((ui.get_panel_width() - 380.0).abs() < 0.5, "no narrower than 380: {}", ui.get_panel_width());
    save(&settle(w), &named(output, "narrow"))?;

    // ── 8. Idle: settled, nothing asks for another frame ──
    settle(w);
    slint::platform::update_timers_and_animations();
    let mut p = Pixels::new(W, H);
    let redrawn = w.draw_if_needed(|r| { r.render(p.make_mut_slice(), W as usize); });
    assert!(!redrawn, "a settled conversation draws nothing: no looping animation, no timer");

    println!("PASS: Chat v2 \u{2014} approval, resolved line, empty state with starters, follow-the-bottom with the New reply pill, Enter and Shift+Enter, a reply's timeline (running, question card, answered), a queued message, resize 380\u{2013}560, idle");
    Ok(())
}

/// The reply's timeline in each of its states, at the panel's current width, with pictures named
/// `<output>-timeline-*.png`: the Dallas weekend turn running, waiting on its question (a card with
/// the recommended answer marked, in the reply, and the reply box answering it), answered and
/// finished (the card folded to one line), and a message queued while it ran.
fn timeline(w: &MinimalSoftwareWindow, ui: &ChatProbe, output: &str, before: &Pixels) -> Result<(), Box<dyn std::error::Error>> {
    let thought = line("thinking", "Thought for 4 s", "done");
    let searched = line("step", "Searched the web for \u{201c}dallas weather\u{201d} \u{2014} 3 results", "done");

    // Running: the steps so far, one live line, nothing in the reply's words yet.
    ui.set_messages(ModelRc::new(VecModel::from(vec![asking(), hermes_reply("", true, Vec::new())])));
    ui.set_live_steps_row(1);
    ui.set_live_steps(ModelRc::new(VecModel::from(vec![
        thought.clone(),
        searched.clone(),
        line("status", "Opening a web page\u{2026} \u{00b7} 24 s", "running"),
    ])));
    ui.set_composer_mode("queue".into());
    ui.set_composer_placeholder("Queue a message for Hermes Agent\u{2026}".into());
    ui.set_live_runs(1);
    let running = settle(w);
    save(&running, &named(output, "timeline-running"))?;
    assert!(differ(before, &running) > 2000, "a running reply draws its timeline");

    // Waiting on its question: the card, where it was asked, and the box answers it.
    let mut ask = line("question", "Which days is this weekend for you?", "waiting");
    ask.call.name = "c1".into();
    ui.set_questions(ModelRc::new(VecModel::from(vec![weekend_question()])));
    ui.set_live_steps(ModelRc::new(VecModel::from(vec![thought.clone(), searched.clone(), ask])));
    ui.set_composer_mode("answer".into());
    ui.set_composer_placeholder("Answer Hermes Agent\u{2019}s question\u{2026}".into());
    ui.set_composer_asks("Hermes Agent asks: Which days is this weekend for you?".into());
    let waiting = settle(w);
    save(&waiting, &named(output, "timeline-question"))?;
    assert!(differ(&running, &waiting) > 2000, "the card is drawn in the reply");
    // Pressed where a person would: the recommended answer is the first, filled button.
    ui.set_answered("".into());
    let panel_x = 1280.0 - ui.get_panel_width() - 12.0;
    'press: for y in (120..600).step_by(4) {
        for x in (panel_x as i32 + 60..panel_x as i32 + 300).step_by(20) {
            crate::click(w, x as f32, y as f32);
            if !ui.get_answered().is_empty() {
                break 'press;
            }
        }
    }
    assert_eq!(ui.get_answered().as_str(), "c1:Sat Oct 17 + Sun Oct 18:pressed", "the recommended answer, pressed");
    // Enter in the reply box goes to the shell, which answers the card with it (lens_queue).
    let sends = ui.get_sends();
    // The words, not the picker row under them: with the question line above it the box is the
    // same height, and its text starts at the top.
    crate::click(w, panel_x + 150.0, 645.0);
    clear_field(w);
    for c in "Saturday".chars() {
        key(w, c.to_string());
    }
    // With words in the box the mic gives its place to Answer, inside the box.
    save(&settle(w), &named(output, "composer-typed"))?;
    key(w, Key::Return);
    settle(w);
    assert_eq!((ui.get_sends(), ui.get_sent().as_str()), (sends + 1, "Saturday"), "Enter hands the words on");

    // Answered and going on: the card is one line in the timeline, and the live line is under
    // the words, where the person pressed (VM 541, 14 Oct: after a tap nothing moved there).
    let mut took = line("question", "Answered: Sat Oct 17 + Sun Oct 18", "answered");
    took.call.name = "c1".into();
    ui.set_questions(ModelRc::new(VecModel::from(Vec::<QuestionRequest>::new())));
    ui.set_messages(ModelRc::new(VecModel::from(vec![
        asking(),
        hermes_reply("Happy to look that up \u{2014} I just need to know which days first.", true, Vec::new()),
    ])));
    ui.set_live_steps(ModelRc::new(VecModel::from(vec![
        thought.clone(),
        searched.clone(),
        took,
        line("status", "Thinking\u{2026}", "running"),
    ])));
    ui.set_composer_mode("queue".into());
    ui.set_composer_placeholder("Queue a message for Hermes Agent\u{2026}".into());
    ui.set_composer_asks("".into());
    save(&settle(w), &named(output, "timeline-after-answer"))?;

    // Answered, and finished: the card folds to one line, and the reply's words sit under it all.
    let mut answered = line("question", "Answered: Sat Oct 17 + Sun Oct 18", "answered");
    answered.call.name = "c1".into();
    let done = vec![
        thought,
        searched,
        answered,
        line("step", "Opened a web page \u{2014} weather.example/dallas", "done"),
        line("text", "Dallas this weekend: Saturday sunny, 29\u{b0}C; Sunday a few clouds, 27\u{b0}C.", ""),
    ];
    ui.set_live_steps_row(-1);
    ui.set_live_steps(ModelRc::new(VecModel::from(Vec::<ContentBlock>::new())));
    ui.set_questions(ModelRc::new(VecModel::from(Vec::<QuestionRequest>::new())));
    ui.set_composer_mode("".into());
    ui.set_composer_placeholder("".into());
    ui.set_composer_asks("".into());
    ui.set_live_runs(0);
    ui.set_messages(ModelRc::new(VecModel::from(vec![
        asking(),
        hermes_reply("Dallas this weekend: Saturday sunny, 29\u{b0}C; Sunday a few clouds, 27\u{b0}C.", false, done.clone()),
    ])));
    let finished = settle(w);
    save(&finished, &named(output, "timeline-answered"))?;
    assert!(differ(&waiting, &finished) > 2000, "the card folded");

    // Queued while it runs: the bubble says when it goes, and its × takes it back.
    ui.set_messages(ModelRc::new(VecModel::from(vec![
        asking(),
        hermes_reply("", true, Vec::new()),
        msg("queued", "And Austin?", ""),
    ])));
    ui.set_live_steps_row(1);
    ui.set_live_steps(ModelRc::new(VecModel::from(vec![
        line("thinking", "Thought for 4 s", "done"),
        line("status", "Searching the web\u{2026} \u{00b7} 12 s", "running"),
    ])));
    ui.set_queued_mind("Hermes Agent".into());
    ui.set_composer_mode("queue".into());
    ui.set_composer_placeholder("Queue a message for Hermes Agent\u{2026}".into());
    ui.set_live_runs(1);
    let queued = settle(w);
    save(&queued, &named(output, "queued"))?;
    let cancels = ui.get_cancels();
    'cancel: for y in (100..680).step_by(3) {
        for x in (1280 - 60..1280 - 12).step_by(4) {
            crate::click(w, x as f32, y as f32);
            if ui.get_cancels() > cancels {
                break 'cancel;
            }
        }
    }
    assert_eq!(ui.get_cancels(), cancels + 1, "\u{00d7} takes the queued message back");

    ui.set_live_steps_row(-1);
    ui.set_live_steps(ModelRc::new(VecModel::from(Vec::<ContentBlock>::new())));
    ui.set_queued_mind("".into());
    ui.set_composer_mode("".into());
    ui.set_composer_placeholder("".into());
    ui.set_live_runs(0);

    // A problem with the model: its one press sits after the model chip, which stays, so a person
    // can choose another model (VM 541, 15 Oct: the LAN model was gone from its list).
    let picker = ui.global::<PickerState>();
    picker.set_status_words("qwen3.5:27b is not one of the models Custom lists".into());
    picker.set_status_state("unreachable".into());
    picker.set_fix_label("Try again".into());
    save(&settle(w), &named(output, "composer-fixing"))?;
    picker.set_status_words("connected \u{b7} qwen3.5:27b".into());
    picker.set_status_state("connected".into());
    picker.set_fix_label("".into());
    Ok(())
}
