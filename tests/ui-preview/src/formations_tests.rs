//! A formation on the desk (design/desk-and-mind-2026-09-23.md, section 6), drawn by the
//! production components from fixture data:
//!
//! - the Recipes screen with a Council mid-flight — the Researcher answered, the Red team and the
//!   Planner at work at the same time, the Chair waiting for them — each Agent stage called by its
//!   role and the mind it runs on, and the opened recipe saying who is at work on what;
//! - a formation's definition opened, with its one input and Start: Enter or Start hands the shell
//!   the input, and Start does nothing while the box is empty;
//! - the Agents screen with the Council's Reviewer, its row saying whose it is ("Council recipe →
//!   Reviewer"), and its approval card saying so too.
use super::*;
use slint::{ModelRc, SharedString, VecModel};
use std::cell::RefCell;

fn stages(list: &[(&str, &str, &str)]) -> ModelRc<RecipeStageData> {
    ModelRc::new(VecModel::from(
        list.iter()
            .enumerate()
            .map(|(i, (kind, label, state))| RecipeStageData {
                number: (i + 1).to_string().into(),
                kind: (*kind).into(),
                label: (*label).into(),
                state: (*state).into(),
            })
            .collect::<Vec<_>>(),
    ))
}

/// What `wire::recipes::row_of` draws for a Council waiting on two of its seats.
fn council() -> RecipeRowData {
    RecipeRowData {
        id: "rcp_council".into(),
        name: "Council".into(),
        status: "waiting".into(),
        status_label: "waiting".into(),
        progress: "step 2 of 4".into(),
        when: "updated 21:06".into(),
        waiting_for: "Waiting for answers from the Red team (pi) and the Planner (deepseek)".into(),
        can_pause: true,
        can_cancel: true,
        formation: true,
        stages: stages(&[
            ("agent", "Researcher · deepseek", "done"),
            ("agent", "Red team · pi", "waiting"),
            ("agent", "Planner · deepseek", "waiting"),
            ("agent", "Chair", "pending"),
        ]),
        ..Default::default()
    }
}

/// A Build that has gone round once: Planner, Coder, Reviewer, back to the Coder.
fn build() -> RecipeRowData {
    RecipeRowData {
        id: "rcp_build".into(),
        name: "Build".into(),
        status: "waiting".into(),
        status_label: "waiting".into(),
        progress: "step 3 of 8".into(),
        when: "updated 21:04".into(),
        waiting_for: "Waiting for the Coder's answer (pi)".into(),
        can_pause: true,
        can_cancel: true,
        formation: true,
        stages: stages(&[
            ("format", "Format", "done"),
            ("agent", "Planner · deepseek", "done"),
            ("agent", "Coder · pi", "waiting"),
            ("agent", "Reviewer · deepseek", "pending"),
            ("jump_if", "If", "done"),
            ("format", "Format", "done"),
            ("jump_if", "If", "done"),
            ("format", "Format", "pending"),
        ]),
        ..Default::default()
    }
}

/// The Council's definition: never run, a formation, startable here with its question.
fn council_definition() -> RecipeRowData {
    RecipeRowData {
        id: "builtin_formation_council".into(),
        name: "Council".into(),
        status: "pending".into(),
        status_label: "formation, never run".into(),
        progress: "4 steps".into(),
        template: true,
        formation: true,
        can_start: true,
        start_hint: "The question the council is to answer…".into(),
        unbound: "{{question}} has no value".into(),
        stages: stages(&[
            ("agent", "Researcher", "pending"),
            ("agent", "Red team", "pending"),
            ("agent", "Planner", "pending"),
            ("agent", "Chair", "pending"),
        ]),
        ..Default::default()
    }
}

fn step(number: &str, label: &str, summary: &str, state: &str, state_label: &str, agent: &str, result: &str) -> RecipeStepData {
    RecipeStepData {
        number: number.into(),
        kind: "agent".into(),
        label: label.into(),
        summary: summary.into(),
        state: state.into(),
        state_label: state_label.into(),
        agent: agent.into(),
        result: result.into(),
        ..Default::default()
    }
}

/// The Council, opened: what `wire::recipes::step_of` draws for each of its four steps.
fn council_steps() -> Vec<RecipeStepData> {
    let seat = "Answer this question on your own, as well as you can. … The question: {{question}}";
    vec![
        step("1", "Researcher · deepseek", seat, "done", "done", "the Researcher on deepseek (deepseek:c-3f09a1)",
             "Answer — yes, if the release branch is cut by Thursday noon. Evidence — the CI dashboard shows…"),
        step("2", "Red team · pi", seat, "waiting", "working", "the Red team on pi (pi:c-77b2e0)", ""),
        step("3", "Planner · deepseek", seat, "waiting", "working", "the Planner on deepseek (deepseek:c-a41c55)", ""),
        step("4", "Chair", "Three agents answered this question independently: {{question}} Weigh their answers…", "pending", "to come", "Chair", ""),
    ]
}

fn council_definition_steps() -> Vec<RecipeStepData> {
    let seat = "Answer this question on your own, as well as you can. … The question: {{question}}";
    vec![
        step("1", "Researcher", seat, "pending", "to come", "Researcher", ""),
        step("2", "Red team", seat, "pending", "to come", "Red team", ""),
        step("3", "Planner", seat, "pending", "to come", "Planner", ""),
        step("4", "Chair", "Three agents answered this question independently: {{question}} Weigh their answers…", "pending", "to come", "Chair", ""),
    ]
}

fn tabs(g: &RecipesState) {
    let tab = |id: &str, label: &str, count: i32| RecipeTabData { id: id.into(), label: label.into(), count };
    g.set_tabs(ModelRc::new(VecModel::from(vec![
        tab("active", "Active", 2),
        tab("finished", "Finished", 0),
        tab("not_run", "Not run", 1),
        tab("all", "All", 3),
    ])));
    g.set_tab("all".into());
}

fn save(pixels: &slint::SharedPixelBuffer<slint::Rgb8Pixel>, path: &str, width: u32, height: u32) -> Result<(), Box<dyn std::error::Error>> {
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(path)?), width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(pixels.as_bytes())?;
    println!("Rendered {path} ({width}×{height})");
    Ok(())
}

pub fn run(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let ui = RecipesProbe::new()?;
    let g = ui.global::<RecipesState>();
    tabs(&g);
    g.set_loaded(true);
    g.set_rows(ModelRc::new(VecModel::from(vec![council(), build(), council_definition()])));
    let log: Rc<RefCell<Vec<String>>> = Rc::default();
    {
        let l = log.clone();
        g.on_select(move |id| l.borrow_mut().push(format!("select:{id}")));
        let l = log.clone();
        g.on_start(move |id, text| l.borrow_mut().push(format!("start:{id}:{text}")));
        let l = log.clone();
        g.on_pause(move |id| l.borrow_mut().push(format!("pause:{id}")));
        let l = log.clone();
        g.on_cancel(move |id| l.borrow_mut().push(format!("cancel:{id}")));
    }
    ui.show()?;
    let draw = |width: u32, height: u32| {
        ui.set_canvas_width(width as f32);
        ui.set_canvas_height(height as f32);
        w.set_size(slint::PhysicalSize::new(width, height));
        slint::platform::update_timers_and_animations();
        let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width, height);
        w.request_redraw();
        w.draw_if_needed(|r| { r.render(pixels.make_mut_slice(), width as usize); });
        pixels
    };
    let (width, height) = (1280u32, 800u32);

    // Mid-flight: the Council's stages — one ticked, two at work, the Chair to come.
    save(&draw(width, height), output, width, height)?;

    // The Council opened: who answered, who is at work, the Chair to come.
    g.set_selected("rcp_council".into());
    g.set_steps(ModelRc::new(VecModel::from(council_steps())));
    save(&draw(width, height), &output.replace(".png", "-open.png"), width, height)?;

    // The definition opened, alone, so its box is the last thing on the screen.
    g.set_rows(ModelRc::new(VecModel::from(vec![council_definition()])));
    g.set_selected("builtin_formation_council".into());
    g.set_steps(ModelRc::new(VecModel::from(council_definition_steps())));
    draw(width, height);
    save(&draw(width, height), &output.replace(".png", "-start.png"), width, height)?;
    // Scan up from the bottom for the box: a click, the question, Enter — Enter sends it.
    let started = |log: &Rc<RefCell<Vec<String>>>| log.borrow().iter().filter(|e| e.starts_with("start:")).count();
    let mut found_y = None;
    for y in (40..(height as i32 - 8)).rev().step_by(6) {
        click(w, 300., y as f32);
        key(w, SharedString::from("Should we ship on Friday?"));
        key(w, SharedString::from("\n"));
        draw(width, height);
        if started(&log) > 0 {
            found_y = Some(y as f32);
            break;
        }
    }
    let y = found_y.expect("the definition has a box for its question");
    assert_eq!(
        log.borrow().iter().filter(|e| e.starts_with("start:")).cloned().collect::<Vec<_>>(),
        ["start:builtin_formation_council:Should we ship on Friday?"],
        "Enter hands the shell the formation and its question"
    );
    // Start, at the right of the same line, does nothing while the box is empty…
    let button = || (0..6).flat_map(move |dy| (1170..1260).step_by(8).map(move |x| (x as f32, y - 4.0 * dy as f32)));
    for (x, by) in button() {
        click(w, x, by);
    }
    draw(width, height);
    assert_eq!(started(&log), 1, "an empty box starts nothing: {:?}", log.borrow());
    // …and hands the shell what is typed once there is something.
    click(w, 300., y);
    key(w, SharedString::from("Monday, then?"));
    draw(width, height);
    for (x, by) in button() {
        if started(&log) > 1 {
            break;
        }
        click(w, x, by);
        draw(width, height);
    }
    assert!(
        log.borrow().iter().any(|e| e == "start:builtin_formation_council:Monday, then?"),
        "Start sends the typed question: {:?}",
        log.borrow()
    );
    assert_eq!(started(&log), 2, "once");

    // Light, mid-flight.
    g.set_rows(ModelRc::new(VecModel::from(vec![council(), build(), council_definition()])));
    g.set_selected("rcp_council".into());
    g.set_steps(ModelRc::new(VecModel::from(council_steps())));
    ui.set_light(true);
    std::thread::sleep(std::time::Duration::from_millis(300));
    save(&draw(width, height), &output.replace(".png", "-light.png"), width, height)?;
    ui.hide()?;

    agents_row_and_card(w, output)?;
    println!(
        "PASS: a Council mid-flight draws each seat by role and mind, working ones waiting and the \
         Chair to come; opened, it says who answered and who is at work; a formation's definition \
         starts on Enter and on Start with its question, never empty; the Agents screen says the \
         Reviewer is the Council recipe's, on its row and its card"
    );
    Ok(())
}

/// The Agents screen: the Council's Reviewer, and the approval card it raised.
fn agents_row_and_card(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (width, height) = (1280u32, 800u32);
    let ui = AgentsProbe::new()?;
    let g = ui.global::<AgentsState>();
    // The Agents screen as the shell fills it, then the Build recipe's Reviewer selected.
    super::agents_tests::fill(&g, false);
    let row = |id: &str, mind: &str, title: &str, state: &str, label: &str, role: &str, origin: &str| AgentRowData {
        id: id.into(),
        mind: mind.into(),
        title: title.into(),
        state: state.into(),
        label: label.into(),
        since: "40s".into(),
        parent: "".into(),
        role: role.into(),
        origin: origin.into(),
    };
    g.set_rows(ModelRc::new(VecModel::from(vec![
        row("deepseek:c-9d1e02", "deepseek", "Review the change just made for: add a --dry-run flag", "waiting_for_you", "waiting for you", "Reviewer", "Build recipe"),
        row("pi:c-77b2e0", "pi", "Answer this question on your own, as well as you can…", "thinking", "thinking", "Red team", "Council recipe"),
        row("deepseek:c-a41c55", "deepseek", "Answer this question on your own, as well as you can…", "thinking", "thinking", "Planner", "Council recipe"),
        row("pi:main", "pi", "tidy the photos folder", "done", "done", "", ""),
    ])));
    g.set_selected("deepseek:c-9d1e02".into());
    let mut header = g.get_header();
    header.id = "deepseek:c-9d1e02".into();
    header.mind = "deepseek".into();
    header.title = "Review the change just made for: add a --dry-run flag".into();
    header.note = "".into();
    g.set_header(header);
    let mut details = g.get_details();
    details.mind = "deepseek".into();
    details.role = "Reviewer".into();
    details.reach = "editor, documents and notes · at most safe".into();
    g.set_details(details);
    let lines = |rows: &[&str]| ModelRc::new(VecModel::from(rows.iter().map(|r| SharedString::from(*r)).collect::<Vec<_>>()));
    let card = ApprovalRequest {
        id: "appr-12".into(),
        agent: "deepseek:c-9d1e02".into(),
        on_behalf: "Build recipe → Reviewer".into(),
        requester: "deepseek 1.2".into(),
        verified: "deepseek (pid 5150) · the attached mind".into(),
        discrepancies: lines(&[]),
        app: "notes".into(),
        action: "list_notes".into(),
        summary: "List the notes in a folder.".into(),
        purpose: "List the notes in a folder.".into(),
        grade: "safe".into(),
        args: lines(&["folder: release"]),
        // `folder: release` is already the thing itself; no naming line to draw (#54).
        target: "".into(),
        warning: "".into(),
        can_session: false,
        decision: "".into(),
        record: "".into(),
        age_text: "110s left".into(),
    };
    let item = |kind: &str, key: &str, text: &str| AgentItemData { kind: kind.into(), key: key.into(), text: text.into(), ..Default::default() };
    g.set_items(ModelRc::new(VecModel::from(vec![
        item("prompt", "t1", "Review the change just made for: add a --dry-run flag"),
        item("text", "t1.0", "Reading the plan and the Coder's change first."),
        AgentItemData { approval: card, ..item("approval", "t1.1", "notes.list_notes") },
    ])));
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(width, height));
    let draw = || {
        slint::platform::update_timers_and_animations();
        let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width, height);
        w.request_redraw();
        w.draw_if_needed(|r| { r.render(pixels.make_mut_slice(), width as usize); });
        pixels
    };
    draw();
    save(&draw(), &output.replace(".png", "-agents.png"), width, height)?;
    ui.hide()?;
    Ok(())
}
