//! The longest real approval card in the Lens (#218), drawn by the production components at
//! 1280×800 and answered with real pointer events. `shell.run_recipe` publishes the longest
//! description of any action on this desktop; its card — the paragraph, an agent row, two
//! arguments, the session row — used to come out taller than the Lens panel. The panel's layout
//! ran out of room at the bottom: the reply box went off the edge and Deny/Allow showed as a
//! 3px sliver nobody could press. The card now keeps the height its panel allows. Since the
//! design sign-off of 4 October (item 6) the part that yields is the provenance at the top — who
//! is asking, the action's name, Details and source — while the consequence rows, the warning and
//! the buttons stay whole under it; `run` checks each of those is drawn above the buttons in the
//! Lens, and checks the rest at the card's natural height.
use super::*;
use super::approval_fit_tests::read_to_end;
use slint::{Model, ModelRc, SharedString, VecModel};

/// What `shell` publishes for `run_recipe` (crates/yantrik-ui/src/control_recipes.rs) — the
/// longest description any app publishes, and the card this defect was hit with.
const RUN_RECIPE_PURPOSE: &str = "Start a recipe with its inputs: a built-in one by its name or id, or one a mind made. A \
    formation — Council, Red team, Build, Writers' room; `describe shell` → `recipes` → \
    `formations` lists each with its `inputs` — hands work to roles from the agent \
    catalog: each works in its own pane on the Agents screen, its row saying which recipe it \
    works for, and the recipe's stages light on the Recipes screen as they answer; its result \
    comes as the recipe's completion. Answers with the run's id; `describe shell` → `recipes` \
    shows how it goes, and `cancel_recipe` stops it and lets its agents go. An agent another \
    agent or a recipe started cannot start a formation.";

/// Its first sentence: the one person-facing line the card leads with, exactly as `summary_of`
/// in crates/yantrik-ui/src/approvals.rs picks it out of the paragraph above.
pub(crate) const RUN_RECIPE_SUMMARY: &str = "Start a recipe with its inputs: a built-in one by its name or id, or one a mind made.";

pub(crate) fn lines(rows: &[&str]) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(rows.iter().map(|r| SharedString::from(*r)).collect::<Vec<_>>()))
}

pub(crate) fn message(role: &str, content: &str) -> MessageData {
    MessageData {
        role: role.into(),
        content: content.into(),
        is_streaming: false,
        blocks: ModelRc::new(VecModel::from(Vec::<ContentBlock>::new())),
        run: "".into(),
    }
}

/// The waiting card, as `control_approvals::row_for` hands it to the Lens.
pub(crate) fn card(summary: &str) -> ApprovalRequest {
    ApprovalRequest {
        id: "appr-31".into(),
        agent: "pi:c-7f3a91".into(),
        on_behalf: "".into(),
        requester: "pi 0.87".into(),
        verified: "pi --mode rpc (pid 4242) · the attached mind".into(),
        identity: "Caller process confirmed: node \u{b7} PID 4242 \u{b7} the attached mind pi".into(),
        identity_tag: "".into(),
        claim: "Claimed name: “pi 0.87”".into(),
        confirm_label: "Allow once".into(),
        destructive: false,
        what: "Runs: recipe: builtin_formation_council; inputs: {\"question\": \"attack the plan to ship 0.4 on Friday\"}".into(),
        exactly: "recipe: builtin_formation_council; inputs: {\"question\": \"attack the plan to ship 0.4 on Friday\"}".into(),
        undo: "".into(),
        discrepancies: lines(&[]),
        app: "shell".into(),
        action: "run_recipe".into(),
        summary: summary.into(),
        purpose: RUN_RECIPE_PURPOSE.into(),
        caller_says: "".into(),
        grade: "sensitive".into(),
        args: lines(&[
            "recipe: builtin_formation_council",
            "inputs: {\"question\": \"attack the plan to ship 0.4 on Friday\"}",
        ]),
        // Both arguments name themselves; no handle on this card needs the app's words (#54).
        target: "".into(),
        // The shell explains nothing per call, so the card is exactly what it was (#137).
        explained: "".into(),
        warning: "".into(),
        can_session: true,
        decision: "".into(),
        record: "".into(),
        age_text: "Expires in 2 min, then declined".into(),
        decided_at: "".into(),
        session: false,
    }
}

/// A card with room in the details for the #137 block: a short purpose, so the sentence and
/// its footnote sit above the scroll fold and can be measured whole. A card that speaks about
/// one call offers no standing yes (#137), so no session row is drawn under its buttons
/// either. The long-purpose fixture above is the sizing case (#218); this one is the block's
/// shape.
fn roomy_card(sentence: &str) -> ApprovalRequest {
    ApprovalRequest {
        purpose: "End a running process by PID.".into(),
        explained: sentence.into(),
        can_session: false,
        ..card(RUN_RECIPE_SUMMARY)
    }
}

pub(crate) fn render(w: &MinimalSoftwareWindow, width: u32, height: u32) -> slint::SharedPixelBuffer<slint::Rgb8Pixel> {
    slint::platform::update_timers_and_animations();
    let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width, height);
    w.request_redraw();
    w.draw_if_needed(|r| { r.render(pixels.make_mut_slice(), width as usize); });
    pixels
}

/// Render until two frames in a row are identical — the card has come to rest. The budget
/// is a wall, not a sleep: a scene that never settles fails instead of passing by accident.
pub(crate) fn settle(w: &MinimalSoftwareWindow, width: u32, height: u32) -> slint::SharedPixelBuffer<slint::Rgb8Pixel> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut prev = render(w, width, height);
    loop {
        std::thread::sleep(std::time::Duration::from_millis(16));
        let next = render(w, width, height);
        if next.as_slice() == prev.as_slice() {
            return next;
        }
        prev = next;
        assert!(std::time::Instant::now() < deadline, "the details section never came to rest");
    }
}

pub(crate) fn save(pixels: &slint::SharedPixelBuffer<slint::Rgb8Pixel>, path: &str, width: u32, height: u32) -> Result<(), Box<dyn std::error::Error>> {
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(path)?), width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(pixels.as_bytes())?;
    println!("Rendered {path} ({width}×{height})");
    Ok(())
}

/// Scan one button's column from the bottom up — the way a person looks for it — and answer
/// with the first y whose click fires. Returns None when nothing in the column answers, which
/// is exactly the #218 failure: the button the person needed was not inside the panel.
pub(crate) fn scan(w: &MinimalSoftwareWindow, x: f32, top: f32, bottom: f32, mut hit: impl FnMut() -> bool) -> Option<f32> {
    let mut y = bottom;
    while y >= top {
        click(w, x, y);
        if hit() {
            return Some(y);
        }
        y -= 4.0;
    }
    None
}

/// The rows whose pixels differ between two frames inside a box, as (first row, last row, how
/// many pixels). `None` when nothing differs.
pub(crate) fn diff_box(a: &[slint::Rgb8Pixel], b: &[slint::Rgb8Pixel], width: u32, xs: (u32, u32), ys: (u32, u32)) -> Option<(u32, u32, u32)> {
    let (mut top, mut bottom, mut n) = (u32::MAX, 0u32, 0u32);
    for y in ys.0..ys.1 {
        for x in xs.0..xs.1 {
            let at = (y * width + x) as usize;
            if a[at] != b[at] {
                top = top.min(y);
                bottom = y;
                n += 1;
            }
        }
    }
    (n > 0).then_some((top, bottom, n))
}

/// The top edge of the band a button answers on, walked up one pixel at a time from a point
/// that answered until a click there stops landing on it.
pub(crate) fn button_top(w: &MinimalSoftwareWindow, x: f32, from: f32, floor: f32, mut count: impl FnMut() -> i32) -> f32 {
    let mut top = from;
    while top > floor {
        let before = count();
        click(w, x, top - 1.0);
        if count() == before {
            break;
        }
        top -= 1.0;
    }
    top
}

pub fn run(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (width, height) = (1280u32, 800u32);
    // The Lens panel's box at 1280×800 (theme.slint): right-docked, between the two bars, and
    // clipped — a button under its bottom edge is a button that cannot be pressed.
    let (panel_left, panel_top, panel_bottom) = (900.0f32, 32.0f32, 760.0f32);
    // The reply box at the panel's foot is at least this tall (the chat bar's own
    // `max(48px, …)`); a button reaching under its top edge is a button the reply box covers.
    let reply_top = panel_bottom - 48.0;

    let ui = ApprovalLensProbe::new()?;
    ui.set_messages(ModelRc::new(VecModel::from(vec![
        message("user", "Start the Council on the plan to ship 0.4 on Friday."),
        message("assistant", "Asking the shell to start the Council — it needs your approval first."),
    ])));
    ui.set_approvals(ModelRc::new(VecModel::from(vec![card(RUN_RECIPE_SUMMARY)])));
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(width, height));
    render(w, width, height);
    // Past the panel's slide-in.
    std::thread::sleep(std::time::Duration::from_millis(300));
    save(&render(w, width, height), output, width, height)?;

    // Deny and Allow each answer a person's click somewhere inside the panel. The session row
    // sits under the buttons and answers first — the scan keeps going until the button itself
    // does. Deny is the left half of the card's content, Allow the right.
    let (deny_x, allow_x) = (panel_left + 100.0, panel_left + 280.0);
    let read_at = (panel_left + 200.0, 170.0);
    let empty = || ModelRc::new(VecModel::from(Vec::<ApprovalRequest>::new()));
    let before = ui.get_denied();
    let deny_y = scan(w, deny_x, panel_top, panel_bottom - 4.0, || ui.get_denied() > before)
        .expect("Deny answers a click inside the panel — the longest card is answerable (#218)");
    // Fourth review of #639: the card is never taller than the Lens allows, and when even its
    // pinned lines do not fit beside its buttons the section above them scrolls and Allow — and
    // the session row, a standing yes — wait until it has been read to its end. A press before
    // then grants nothing.
    let top = button_top(w, deny_x, deny_y, panel_top, || ui.get_denied());
    let before = ui.get_allowed();
    click(w, allow_x, top + 14.0);
    let waited = ui.get_allowed() == before;
    if waited {
        assert_eq!(ui.get_sessioned(), 0, "the session row waits with Allow");
        read_to_end(w, read_at.0, read_at.1, width, height);
    }
    let before = ui.get_allowed();
    let allow_y = scan(w, allow_x, panel_top, panel_bottom - 4.0, || ui.get_allowed() > before)
        .expect("Allow answers a click inside the panel (#218)");
    println!("the longest card: Allow {}", if waited { "waited until the card was read to its end" } else { "was live at once" });
    // On main before the sign-off this failed: the pinned identity rows alone outgrew the Lens's
    // 240px, the buttons and the session row were pushed out of the card, and a transcript
    // bubble sat on the session row.
    assert!(ui.get_sessioned() >= 1, "the session row under the buttons answers too");
    assert!((allow_y - deny_y).abs() <= 4.0, "Deny and Allow are one row: {deny_y} against {allow_y}");

    // The band the button actually answers on, to the pixel. The #218 card left a 3px sliver of
    // this row inside the panel — a sliver is not a button, and a whole one is 32px tall.
    let mut bottom = deny_y;
    while bottom < panel_bottom {
        let before = ui.get_denied();
        click(w, deny_x, bottom + 1.0);
        if ui.get_denied() == before {
            break;
        }
        bottom += 1.0;
    }
    assert!(bottom - top >= 28.0, "the whole button answers, not a sliver: {}px of it does", bottom - top);
    assert!(top >= panel_top, "the button starts inside the Lens, at {top}");
    assert!(bottom <= reply_top, "the button ends above the reply box, inside the Lens: its lowest answer is {bottom}, the reply box starts at {reply_top}");
    assert!(allow_y >= panel_top && allow_y <= reply_top, "Allow is inside the Lens too, at {allow_y}");

    // ── Sign-off item 6, and review of #639 S1: what the card shows at rest ──
    //
    // The Lens gives a card 240px at 1280×800. Each line is checked to be drawn whole, inside the
    // panel and above the top of the button band, by changing its words for others of the same
    // length and finding where the frame changed: a clipped line changes a sliver, a line out of
    // view changes nothing. Each card starts fresh (the list is emptied first), at its top.
    let xs = (panel_left as u32 + 16, 1264);
    let ys = (panel_top as u32, reply_top as u32);
    let check = |a: &[slint::Rgb8Pixel], b: &[slint::Rgb8Pixel], btn: f32, what: &str| -> (u32, u32) {
        let (t, bt, n) = diff_box(a, b, width, xs, ys).unwrap_or_else(|| panic!("{what} is not drawn anywhere in the Lens"));
        println!("{what}: drawn on rows {t}..{bt} ({n} pixels), the buttons start at {btn}");
        assert!(bt - t >= 7, "{what} is a whole line of type, not a clipped sliver: rows {t}..{bt}");
        assert!((bt as f32) < btn - 2.0, "{what} ends above the buttons: row {bt}, buttons at {btn}");
        assert!(t as f32 > panel_top, "{what} is inside the panel");
        (t, bt)
    };
    // Shown fresh, its buttons found: (the frame, the top of the button band).
    let fresh = |base: &ApprovalRequest, what: &str| -> (slint::SharedPixelBuffer<slint::Rgb8Pixel>, f32) {
        ui.set_approvals(empty());
        settle(w, width, height);
        ui.set_approvals(ModelRc::new(VecModel::from(vec![base.clone()])));
        let a = settle(w, width, height);
        let before = ui.get_denied();
        let deny = scan(w, deny_x, panel_top, panel_bottom - 4.0, || ui.get_denied() > before)
            .unwrap_or_else(|| panic!("{what}: Decline answers inside the panel"));
        (a, button_top(w, deny_x, deny, panel_top, || ui.get_denied()))
    };
    let row_drawn_above = |base: ApprovalRequest, change: &dyn Fn(&mut ApprovalRequest), what: &str| -> (u32, u32) {
        let (a, btn) = fresh(&base, what);
        let mut changed = base;
        change(&mut changed);
        ui.set_approvals(ModelRc::new(VecModel::from(vec![changed])));
        let b = settle(w, width, height);
        check(a.as_slice(), b.as_slice(), btn, what)
    };
    // The same, for a card that has to scroll: Allow is pressed first and must grant nothing; the
    // card is read to its end; the line is then found drawn; and Allow then answers. The change
    // is written into the model on screen, row by row, the way the shell's once-a-second
    // republish writes it (`models::update` in control_approvals::publish) — so the card is not
    // rebuilt, and what was read stays read.
    let drawn_once_read = |base: ApprovalRequest, change: &dyn Fn(&mut ApprovalRequest), what: &str| -> (u32, u32) {
        let (_, btn) = fresh(&base, what);
        let before = ui.get_allowed();
        click(w, allow_x, btn + 14.0);
        assert_eq!(ui.get_allowed(), before, "{what}: the card has to scroll, and Allow grants nothing before its end");
        read_to_end(w, read_at.0, read_at.1, width, height);
        let a = settle(w, width, height);
        let mut changed = base;
        change(&mut changed);
        ui.get_approvals().set_row_data(0, changed);
        let b = settle(w, width, height);
        let found = check(a.as_slice(), b.as_slice(), btn, what);
        let before = ui.get_allowed();
        click(w, allow_x, btn + 14.0);
        assert!(ui.get_allowed() > before, "{what}: once read to its end, Allow answers");
        found
    };
    let deleting = super::review_stills::delete_card();
    let what = row_drawn_above(deleting.clone(), &|c: &mut ApprovalRequest| c.what = "Removes: id: sweep-demo-not-real".into(), "the \"Deletes:\" line");
    let undo = row_drawn_above(deleting.clone(), &|c: &mut ApprovalRequest| c.undo = "Undo: not possible, the app said so".into(), "the undo line");
    assert!(undo.0 > what.1, "the undo line is under the line that says what changes");
    let dangerous = super::review_stills::dangerous_card();
    row_drawn_above(
        dangerous.clone(),
        &|c: &mut ApprovalRequest| c.warning = "This is graded dangerous \u{2014} it can destroy data or state.".into(),
        "the warning",
    );

    // Eight arguments at their longest, the session row under the buttons, a discrepancy and the
    // app's word about the call: more than the Lens's 240px holds beside the buttons. At rest the
    // card shows who is asking and what the action is, from its top; the arguments are reached by
    // scrolling, and Allow waits until they have all been in view.
    // The last one ends "wv", so its tail can be changed to "vw" — the same glyphs, the same
    // width — without moving a line break, and the change is found where the tail is drawn.
    let long: Vec<String> = (0..8)
        .map(|i| if i == 7 { format!("argument{i}: {}wv", "v".repeat(58)) } else { format!("argument{i}: {}", "v".repeat(60)) })
        .collect();
    let crowded = ApprovalRequest {
        id: "appr-crowded".into(),
        args: lines(&long.iter().map(String::as_str).collect::<Vec<_>>()),
        what: format!("Runs: {}", long.join("; ")).into(),
        exactly: long.join("; ").into(),
        discrepancies: lines(&["The caller called this `standard`; the app publishes `sensitive`."]),
        explained: "After this the recipe's agents start at once.".into(),
        identity: "Caller process confirmed: sshd-session \u{b7} PID 2290461 \u{b7} from a terminal".into(),
        ..card(RUN_RECIPE_SUMMARY)
    };
    let who = row_drawn_above(crowded.clone(), &|c: &mut ApprovalRequest| c.identity = "Caller process confirmed: sshd-session \u{b7} PID 2290467 \u{b7} from a terminal".into(), "the identity line, eight long arguments");
    let claim = row_drawn_above(crowded.clone(), &|c: &mut ApprovalRequest| c.claim = "Claimed name: \u{201c}pi 0.88\u{201d}".into(), "the claim, eight long arguments");
    let doubt = row_drawn_above(
        crowded.clone(),
        &|c: &mut ApprovalRequest| c.discrepancies = lines(&["The caller called this `standard`; the app publishes `dangerous`."]),
        "the discrepancy, eight long arguments",
    );
    let said = row_drawn_above(
        crowded.clone(),
        // Its first word: the line is cut at the card's edge, so its end is not on screen.
        &|c: &mut ApprovalRequest| c.summary = "Begin a recipe with its inputs: a built-in one by its name or id, or one a mind made.".into(),
        "the action's sentence, eight long arguments",
    );
    assert!(who.1 < claim.0 && claim.1 < doubt.0 && doubt.1 < said.0, "fact, claim, discrepancy, then the sentence: {who:?} {claim:?} {doubt:?} {said:?}");
    // Every argument the grant binds is in view before Allow can be pressed — the last one's last
    // character included.
    let mut last = long.clone();
    last[7] = format!("argument7: {}vw", "v".repeat(58));
    drawn_once_read(
        crowded.clone(),
        &|c: &mut ApprovalRequest| {
            c.exactly = last.join("; ").into();
            c.what = format!("Runs: {}", last.join("; ")).into();
        },
        "the last argument's tail, eight long arguments",
    );
    // The command a mind could hide behind a cut line: its dangerous end is drawn before Allow
    // can be pressed. (The card `row_for` makes of a real agent_run call is checked in the
    // shell's own tests: a_long_agent_run_command_is_shown_whole_or_not_asked.)
    let rm = ApprovalRequest {
        id: "appr-rm".into(),
        app: "shell".into(),
        action: "agent_run".into(),
        summary: "Run one command line in a fresh terminal of your own.".into(),
        args: lines(&["command: echo cleaning the temporary build files now, one moment please && rm -rf ~/x"]),
        what: "Runs: command: echo cleaning the temporary build files now, one moment please && rm -rf ~/x".into(),
        exactly: "command: echo cleaning the temporary build files now, one moment please && rm -rf ~/x".into(),
        warning: "What it runs can do anything you can. Allowing it for the session lets any mind or caller on this desktop run any command through it, until the shell restarts or the mode is lowered.".into(),
        discrepancies: lines(&["The caller called this `standard`; the app publishes `sensitive`."]),
        ..card(RUN_RECIPE_SUMMARY)
    };
    let rm_changed = |c: &mut ApprovalRequest| {
        c.what = "Runs: command: echo cleaning the temporary build files now, one moment please && rm -rf ~/y".into();
        c.exactly = "command: echo cleaning the temporary build files now, one moment please && rm -rf ~/y".into();
    };
    drawn_once_read(rm.clone(), &rm_changed, "\"rm -rf ~/x\" at the end of the command");
    ui.set_approvals(empty());
    settle(w, width, height);
    ui.set_approvals(ModelRc::new(VecModel::from(vec![rm])));
    save(&settle(w, width, height), &output.replace(".png", "-rm.png"), width, height)?;
    ui.set_approvals(empty());
    settle(w, width, height);
    ui.set_approvals(ModelRc::new(VecModel::from(vec![crowded])));
    save(&settle(w, width, height), &output.replace(".png", "-eight-args.png"), width, height)?;
    read_to_end(w, read_at.0, read_at.1, width, height);
    save(&settle(w, width, height), &output.replace(".png", "-eight-args-read.png"), width, height)?;
    let before = ui.get_allowed();
    scan(w, allow_x, panel_top, panel_bottom - 4.0, || ui.get_allowed() > before).expect("Allow answers with eight long arguments, once read");
    let before = ui.get_sessioned();
    scan(w, panel_left + 200.0, panel_top, panel_bottom - 4.0, || ui.get_sessioned() > before).expect("and the session row too");

    // And the red button answers with the action's own words on it.
    ui.set_approvals(ModelRc::new(VecModel::from(vec![deleting])));
    settle(w, width, height);
    let before = ui.get_allowed();
    scan(w, allow_x, panel_top, panel_bottom - 4.0, || ui.get_allowed() > before).expect("\"Delete event\" answers a click inside the panel");
    save(&settle(w, width, height), &output.replace(".png", "-delete.png"), width, height)?;

    // ── The card at its natural height: what the Lens's cap scrolls is all there ──
    let natural = ApprovalCardProbe::new()?;
    natural.set_data(card(RUN_RECIPE_SUMMARY));
    natural.show()?;
    let size = |p: &ApprovalCardProbe| (440u32, p.get_card_h().ceil() as u32);
    let (nw, nh) = size(&natural);
    w.set_size(slint::PhysicalSize::new(nw, nh));
    let led = settle(w, nw, nh);
    // The 18px line is the description's first sentence: emptying it changes the picture.
    natural.set_data(card(""));
    let (bw, bh) = size(&natural);
    w.set_size(slint::PhysicalSize::new(bw, bh));
    let bare = settle(w, bw, bh);
    let differ = led.as_slice().iter().zip(bare.as_slice()).filter(|(a, b)| a != b).count();
    assert!(differ >= 300, "the card leads with the description's first sentence: only {differ} pixels change when it is emptied");

    // #137 and review of #639 S2: the app's sentence about one call is on the card's face, on a
    // sensitive card as on a dangerous one, and it wraps — elided, it cut off exactly the clause
    // that was its point.
    let sentence = "After this, prompts go to images.example and may cost money, and every \
        picture this app draws from now on is drawn there rather than on this machine.";
    // Measured after a frame is drawn: the card's height is its laid-out height.
    w.set_size(slint::PhysicalSize::new(440, 900));
    let measure = |data: ApprovalRequest| {
        natural.set_data(data);
        settle(w, 440, 900);
        natural.get_card_h()
    };
    let without = measure(roomy_card(""));
    let with = measure(roomy_card(sentence));
    println!("#137 line: the sensitive card is {without}px without the sentence and {with}px with it");
    assert!(with - without >= 45.0, "the per-call sentence is on the face of a sensitive card and wraps: {}px", with - without);

    // Sign-off, 5 October: Decline is the outline kind, transparent at rest, and showed the card's
    // ground — in light the bar's grey, so it read as a filled grey key beside the outline it is
    // in dark. The card has a ground of its own (`chat-card`): white in light, the bar's in dark.
    for (dark, want) in [(false, (0xff, 0xff, 0xff)), (true, (0x10, 0x14, 0x17))] {
        natural.global::<ThemeMode>().set_dark(dark);
        natural.set_data(card(RUN_RECIPE_SUMMARY));
        let (gw, gh) = size(&natural);
        w.set_size(slint::PhysicalSize::new(gw, gh));
        let p = settle(w, gw, gh);
        // Inside the card's left padding, halfway down: the ground and nothing drawn on it.
        let c = p.as_slice()[(gh / 2 * gw + 14) as usize];
        assert_eq!((c.r, c.g, c.b), want, "the approval card's ground (dark: {dark})");
    }
    natural.global::<ThemeMode>().set_dark(true);

    println!(
        "PASS: the longest card fits the Lens at 1280×800 — Deny answers at {deny_y} and Allow at \
         {allow_y}, one row, the whole {}px button inside the panel above the reply box, the \
         session row reachable; what changes, the undo line and the warning are drawn whole above \
         the buttons; with eight 60-character arguments the identity line, the claim, the \
         discrepancy, the sentence, the pinned line and the buttons are all drawn; the card leads \
         with the description's first sentence ({differ} pixels drawn); the per-call sentence is \
         on the face of a sensitive card and wraps",
        bottom - top,
    );
    Ok(())
}

/// The real approval card, drawn by IntentLens (not a hand-built pair), answered by keys: Tab,
/// Enter, Return and Space, however many times and in whatever order focus lands, press neither
/// Deny, Allow once nor the session row; a click on each still does (#583). Deleting
/// `pointer-only: true` from either card button makes Tab-Enter reach it and fail the first
/// assertion. The accessibility default action is not reachable from here; it is covered by the
/// source scan `consent_buttons_are_pointer_only` in the kit crate.
pub fn run_pointer_only(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    use slint::platform::Key;
    let (width, height) = (1280u32, 800u32);
    let (panel_left, panel_top, panel_bottom) = (900.0f32, 32.0f32, 760.0f32);
    let ui = ApprovalLensProbe::new()?;
    ui.set_messages(ModelRc::new(VecModel::from(vec![message("user", "Start the Council.")])));
    ui.set_approvals(ModelRc::new(VecModel::from(vec![card(RUN_RECIPE_SUMMARY)])));
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(width, height));
    render(w, width, height);
    std::thread::sleep(std::time::Duration::from_millis(300));
    save(&settle(w, width, height), output, width, height)?;

    // Shift+Tab and Tab both ways round the whole focus chain, with every confirming key at each
    // stop: if either button, or the row under them, could take focus it would be pressed here.
    let keys_round = || {
        for _ in 0..12 {
            for k in [slint::SharedString::from(Key::Tab), "\n".into(), "\r".into(), " ".into()] {
                key(w, k);
            }
            key(w, slint::SharedString::from(Key::Backtab));
            key(w, "\n".into());
            key(w, " ".into());
            render(w, width, height);
        }
    };
    keys_round();
    assert_eq!(
        (ui.get_allowed(), ui.get_denied()),
        (0, 0),
        "Tab, Enter, Return and Space press neither Allow once nor Deny on the real approval card"
    );
    // The same on a destructive card: the red "Delete event" button is the same pointer-only
    // button with other words and another fill (sign-off item 1).
    ui.set_approvals(ModelRc::new(VecModel::from(vec![super::review_stills::delete_card()])));
    settle(w, width, height);
    keys_round();
    assert_eq!((ui.get_allowed(), ui.get_denied()), (0, 0), "no key presses the red \"Delete event\" button or Decline");

    let (deny_x, allow_x) = (panel_left + 100.0, panel_left + 280.0);
    let before = ui.get_denied();
    scan(w, deny_x, panel_top, panel_bottom - 4.0, || ui.get_denied() > before).expect("a click on Decline answers");
    let before = ui.get_allowed();
    scan(w, allow_x, panel_top, panel_bottom - 4.0, || ui.get_allowed() > before).expect("a click on Delete event answers");
    println!("PASS approval card: keys press nothing (Tab/Backtab/Enter/Return/Space ×12, on an Allow-once card and on a red Delete-event card), a click answers Decline and the action");
    Ok(())
}
