//! The longest real approval card in the Lens (#218), drawn by the production components at
//! 1280×800 and answered with real pointer events. `shell.run_recipe` publishes the longest
//! description of any action on this desktop; its card — the paragraph, an agent row, two
//! arguments, the session row — used to come out taller than the Lens panel. The panel's layout
//! ran out of room at the bottom: the reply box went off the edge and Deny/Allow showed as a
//! 3px sliver nobody could press. The card now keeps the height its panel allows — identity
//! pinned on top, details scrolling in the middle, buttons pinned at the bottom — and the
//! description is clamped under "show more", with its first sentence as the card's own summary
//! line.
use super::*;
use slint::{ModelRc, SharedString, VecModel};

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
const RUN_RECIPE_SUMMARY: &str = "Start a recipe with its inputs: a built-in one by its name or id, or one a mind made.";

fn lines(rows: &[&str]) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(rows.iter().map(|r| SharedString::from(*r)).collect::<Vec<_>>()))
}

fn message(role: &str, content: &str) -> MessageData {
    MessageData {
        role: role.into(),
        content: content.into(),
        is_streaming: false,
        blocks: ModelRc::new(VecModel::from(Vec::<ContentBlock>::new())),
    }
}

/// The waiting card, as `control_approvals::row_for` hands it to the Lens.
fn card(summary: &str) -> ApprovalRequest {
    ApprovalRequest {
        id: "appr-31".into(),
        agent: "pi:c-7f3a91".into(),
        on_behalf: "".into(),
        requester: "pi 0.87".into(),
        verified: "pi --mode rpc (pid 4242) · the attached mind".into(),
        discrepancies: lines(&[]),
        app: "shell".into(),
        action: "run_recipe".into(),
        summary: summary.into(),
        purpose: RUN_RECIPE_PURPOSE.into(),
        grade: "sensitive".into(),
        args: lines(&[
            "recipe: builtin_formation_council",
            "inputs: {\"question\": \"attack the plan to ship 0.4 on Friday\"}",
        ]),
        // Both arguments name themselves; no handle on this card needs the app's words (#54).
        target: "".into(),
        warning: "".into(),
        can_session: true,
        decision: "".into(),
        record: "".into(),
        age_text: "94s left".into(),
    }
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

/// Scan one button's column from the bottom up — the way a person looks for it — and answer
/// with the first y whose click fires. Returns None when nothing in the column answers, which
/// is exactly the #218 failure: the button the person needed was not inside the panel.
fn scan(w: &MinimalSoftwareWindow, x: f32, top: f32, bottom: f32, mut hit: impl FnMut() -> bool) -> Option<f32> {
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

    // The summary line is drawn, up in the card's pinned head: emptying it changes the picture.
    let led = render(w, width, height);
    ui.set_approvals(ModelRc::new(VecModel::from(vec![card("")])));
    render(w, width, height);
    let bare = render(w, width, height);
    let differ = led.as_slice().iter().zip(bare.as_slice()).filter(|(a, b)| a != b).count();
    assert!(differ >= 300, "the card leads with the description's first sentence: only {differ} pixels change when it is emptied");
    ui.set_approvals(ModelRc::new(VecModel::from(vec![card(RUN_RECIPE_SUMMARY)])));
    render(w, width, height);
    render(w, width, height);

    // Deny and Allow each answer a person's click somewhere inside the panel. The session row
    // sits under the buttons and answers first — the scan keeps going until the button itself
    // does. Deny is the left half of the card's content, Allow the right.
    let (deny_x, allow_x) = (panel_left + 100.0, panel_left + 280.0);
    let before = ui.get_denied();
    let deny_y = scan(w, deny_x, panel_top, panel_bottom - 4.0, || ui.get_denied() > before)
        .expect("Deny answers a click inside the panel — the longest card is answerable (#218)");
    let before = ui.get_allowed();
    let allow_y = scan(w, allow_x, panel_top, panel_bottom - 4.0, || ui.get_allowed() > before)
        .expect("Allow answers a click inside the panel (#218)");
    assert!(ui.get_sessioned() >= 1, "the session row under the buttons answers too");
    assert!((allow_y - deny_y).abs() <= 4.0, "Deny and Allow are one row: {deny_y} against {allow_y}");

    // The band the button actually answers on, to the pixel: walk out from the point that
    // answered until clicks stop landing on it. The #218 card left a 3px sliver of this row
    // inside the panel — a sliver is not a button, and a whole one is 32px tall.
    let mut top = deny_y;
    while top > panel_top {
        let before = ui.get_denied();
        click(w, deny_x, top - 1.0);
        if ui.get_denied() == before {
            break;
        }
        top -= 1.0;
    }
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

    println!(
        "PASS: the longest card fits the Lens at 1280×800 — Deny answers at {deny_y} and Allow at \
         {allow_y}, one row, the whole {}px button inside the panel above the reply box, the \
         session row reachable, and the card leads with the description's first sentence \
         ({differ} pixels drawn)",
        bottom - top
    );
    Ok(())
}
