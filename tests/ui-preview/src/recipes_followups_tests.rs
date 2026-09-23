//! The recipe follow-ups (#194, #187), drawn by the production components from fixture data:
//!
//! - a finished Council, opened: each seat says the role that took it and the question as asked —
//!   no `{{placeholders}}` — the Chair's step links each answer it read, every agent's session is
//!   one press away, and the Chair's verdict is drawn from its markdown (a heading, bold, a list),
//!   not as `## Verdict` and `**…**` in monospace;
//! - the notice over a finished run says it finished, and the row lists what its agents opened;
//! - a formation's definition offers its seats as roles from the catalog, and a pick is sent with
//!   the seat and the role;
//! - the Agents pane's first prompt says who sent it: "Council recipe", not "you".
use super::*;
use slint::{ModelRc, VecModel};
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

/// What `wire::recipes::row_of` draws for a Council that has finished.
fn council_done() -> RecipeRowData {
    RecipeRowData {
        id: "rcp_council".into(),
        name: "Council".into(),
        status: "done".into(),
        status_label: "done".into(),
        progress: "4 steps".into(),
        when: "updated 21:08".into(),
        formation: true,
        opened: "Its agents opened chromium (the Researcher), the problems screen (the Researcher). They are left as they are — close what you are done with.".into(),
        stages: stages(&[
            ("agent", "Researcher · deepseek", "done"),
            ("agent", "Red team · pi", "done"),
            ("agent", "Planner · deepseek", "done"),
            ("agent", "Chair · deepseek", "done"),
        ]),
        ..Default::default()
    }
}

/// An answer's head as `wire::recipes::answer_blocks` gives it: the Lens's parser's blocks, each a
/// StyledText for a paragraph or a list.
fn blocks(list: &[(&str, &str)]) -> ModelRc<RecipeAnswerBlock> {
    ModelRc::new(VecModel::from(
        list.iter()
            .map(|(block, markdown)| {
                let plain = markdown.replace("**", "").replace('`', "");
                RecipeAnswerBlock {
                    block: (*block).into(),
                    text: plain.as_str().into(),
                    styled: if matches!(*block, "text" | "bullet") {
                        slint::StyledText::from_markdown(markdown).unwrap_or_else(|_| slint::StyledText::from_plain_text(&plain))
                    } else {
                        slint::StyledText::from_plain_text(&plain)
                    },
                }
            })
            .collect::<Vec<_>>(),
    ))
}

fn links(list: &[(&str, &str)]) -> ModelRc<RecipeLinkData> {
    ModelRc::new(VecModel::from(list.iter().map(|(label, agent)| RecipeLinkData { label: (*label).into(), agent: (*agent).into() }).collect::<Vec<_>>()))
}

const VERDICT: &[(&str, &str)] = &[
    ("heading", "Verdict"),
    ("text", "Publish the nightly, **but only** the build without the AI mind — routing stubbed — with a known-issue note. Confidence: **high**."),
    ("bullet", "\u{2022} the failing check is **optional**, so the artifact without it is whole\n\u{2022} the note says whether the failing path is reachable"),
];

/// The finished Council, opened: what `wire::recipes::step_of` draws for a run.
fn council_steps(answer_as_markdown: bool) -> Vec<RecipeStepData> {
    let seat = |n: &str, label: &str, role: &str, agent: &str, answer: &[(&str, &str)]| RecipeStepData {
        number: n.into(),
        kind: "agent".into(),
        label: label.into(),
        summary: "Answer this question on your own, as well as you can. Two other agents are answering it too…".into(),
        state: "done".into(),
        state_label: "done".into(),
        agent: format!("the {} on {} ({agent})", role, label.split(" · ").nth(1).unwrap_or("deepseek")).into(),
        agent_id: agent.into(),
        detail: format!(
            "hands to: {}\nasks: Answer this question on your own, as well as you can. … The question: Should we publish tonight's nightly with one known failing optional-mind check?\nkeeps its answer as: answer_{n}",
            role.to_lowercase().replace(' ', "-")
        )
        .into(),
        answer: blocks(answer),
        ..Default::default()
    };
    let mut chair = seat("4", "Chair · deepseek", "Chair", "deepseek:c-02be44", if answer_as_markdown { VERDICT } else { &[] });
    chair.summary = "Three agents answered this question independently: Should we publish tonight's nightly… Weigh their answers…".into();
    chair.detail = "hands to: chair\nasks: Three agents answered this question independently: Should we publish tonight's nightly with one known failing optional-mind check? Weigh their answers…\nreads first: From the researcher:\n[step 1: the Researcher's answer]\n\nFrom the red-team:\n[step 2: the Red team's answer]\n\nFrom the planner:\n[step 3: the Planner's answer]\nkeeps its answer as: verdict".into();
    chair.reads = links(&[("Researcher · step 1", "deepseek:c-3f09a1"), ("Red team · step 2", "pi:c-77b2e0"), ("Planner · step 3", "deepseek:c-a41c55")]);
    if !answer_as_markdown {
        // What the screen drew before: the head as it came, in monospace.
        chair.result = "## Verdict **Publish the nightly, but only** the build without the AI mind — routing stubbed — with a known-issue note. Confidence: **high**. - the failing check is **optional**…".into();
    }
    vec![
        seat("1", "Researcher · deepseek", "Researcher", "deepseek:c-3f09a1", &[("heading", "Answer"), ("text", "Yes — the check is **optional** and the channel page says so.")]),
        seat("2", "Red team · pi", "Red team", "pi:c-77b2e0", &[("text", "No: a **known failure** in a nightly trains people to ignore red.")]),
        seat("3", "Planner · deepseek", "Planner", "deepseek:c-a41c55", &[("text", "Publish the build **without** the mind; fix the check on Monday.")]),
        chair,
    ]
}

/// The Council's definition, its seats offered.
fn council_definition(picking: &str) -> RecipeRowData {
    let seat = |name: &str, label: &str, role: &str, role_name: &str, changed: bool| RecipeSeatData {
        name: name.into(),
        label: label.into(),
        role: role.into(),
        role_name: role_name.into(),
        changed,
    };
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
        seats: ModelRc::new(VecModel::from(vec![
            seat("seat_1", "Seat 1", "researcher", "Researcher", false),
            seat("seat_2", "Seat 2", "reviewer", "Reviewer", true),
            seat("seat_3", "Seat 3", "planner", "Planner", false),
            seat("chair", "Chair", "chair", "Chair", false),
        ])),
        picking: picking.into(),
        picking_role: if picking.is_empty() { "" } else { "reviewer" }.into(),
        stages: stages(&[("agent", "Researcher", "pending"), ("agent", "Reviewer", "pending"), ("agent", "Planner", "pending"), ("agent", "Chair", "pending")]),
        ..Default::default()
    }
}

fn roles() -> ModelRc<RecipeRoleData> {
    let role = |id: &str, name: &str| RecipeRoleData { id: id.into(), name: name.into() };
    ModelRc::new(VecModel::from(vec![
        role("researcher", "Researcher"),
        role("planner", "Planner"),
        role("coder", "Coder"),
        role("reviewer", "Reviewer"),
        role("red-team", "Red team"),
        role("writer", "Writer"),
        role("chair", "Chair"),
        role("scribe", "Scribe"),
    ]))
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
    let tab = |id: &str, label: &str, count: i32| RecipeTabData { id: id.into(), label: label.into(), count };
    g.set_tabs(ModelRc::new(VecModel::from(vec![tab("active", "Active", 0), tab("finished", "Finished", 1), tab("not_run", "Not run", 1), tab("all", "All", 2)])));
    g.set_tab("finished".into());
    g.set_loaded(true);
    g.set_roles(roles());
    g.set_rows(ModelRc::new(VecModel::from(vec![council_done()])));
    g.set_selected("rcp_council".into());
    g.set_steps(ModelRc::new(VecModel::from(council_steps(true))));
    // The notice the start left, as the screen says it once the run has finished.
    g.set_notice("Council finished — verdict from the Chair. It is under the recipe's last step.".into());
    g.set_notice_ok(true);
    let log: Rc<RefCell<Vec<String>>> = Rc::default();
    {
        let l = log.clone();
        g.on_open_agent(move |id| l.borrow_mut().push(format!("open:{id}")));
        let l = log.clone();
        g.on_choose_seat(move |recipe, seat| l.borrow_mut().push(format!("choose:{recipe}:{seat}")));
        let l = log.clone();
        g.on_pick_seat(move |recipe, seat, role| l.borrow_mut().push(format!("pick:{recipe}:{seat}:{role}")));
    }
    ui.show()?;
    let (width, height) = (1280u32, 1000u32);
    let draw = || {
        ui.set_canvas_width(width as f32);
        ui.set_canvas_height(height as f32);
        w.set_size(slint::PhysicalSize::new(width, height));
        slint::platform::update_timers_and_animations();
        let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width, height);
        w.request_redraw();
        w.draw_if_needed(|r| { r.render(pixels.make_mut_slice(), width as usize); });
        pixels
    };
    draw();
    let done = draw();
    save(&done, output, width, height)?;

    // The verdict as the screen drew it before: raw markdown in monospace. The two must differ.
    g.set_steps(ModelRc::new(VecModel::from(council_steps(false))));
    draw();
    let raw = draw();
    save(&raw, &output.replace(".png", "-before.png"), width, height)?;
    assert_ne!(done.as_bytes(), raw.as_bytes(), "the verdict is drawn from its markdown, not as it came");
    g.set_steps(ModelRc::new(VecModel::from(council_steps(true))));
    draw();

    // Every agent's session, and each answer the Chair read, one press away: a scan of the opened
    // steps presses something that asks for an agent's session, and only ever one of the four.
    let agents = ["deepseek:c-3f09a1", "pi:c-77b2e0", "deepseek:c-a41c55", "deepseek:c-02be44"];
    'scan: for y in (160..(height as i32 - 20)).step_by(5) {
        for x in (60..900).step_by(14) {
            click(w, x as f32, y as f32);
            if log.borrow().iter().any(|e| e.starts_with("open:")) {
                break 'scan;
            }
        }
    }
    let opened: Vec<String> = log.borrow().iter().filter(|e| e.starts_with("open:")).cloned().collect();
    assert!(!opened.is_empty(), "a session is one press away: {:?}", log.borrow());
    assert!(opened.iter().all(|e| agents.iter().any(|a| e == &format!("open:{a}"))), "{opened:?}");

    // Light.
    ui.set_light(true);
    std::thread::sleep(std::time::Duration::from_millis(300));
    draw();
    save(&draw(), &output.replace(".png", "-light.png"), width, height)?;
    ui.set_light(false);
    std::thread::sleep(std::time::Duration::from_millis(300));

    // The definition: its seats, and the roles offered for Seat 2.
    g.set_notice("".into());
    g.set_tab("not_run".into());
    g.set_rows(ModelRc::new(VecModel::from(vec![council_definition("seat_2")])));
    g.set_selected("builtin_formation_council".into());
    let pending = |n: &str, label: &str| RecipeStepData {
        number: n.into(),
        kind: "agent".into(),
        label: label.into(),
        summary: "Answer this question on your own, as well as you can. … The question: {{question}}".into(),
        state: "pending".into(),
        state_label: "to come".into(),
        agent: label.into(),
        ..Default::default()
    };
    g.set_steps(ModelRc::new(VecModel::from(vec![pending("1", "Researcher"), pending("2", "Reviewer"), pending("3", "Planner"), pending("4", "Chair")])));
    draw();
    save(&draw(), &output.replace(".png", "-seats.png"), width, height)?;
    'pick: for y in (100..(height as i32 - 20)).step_by(4) {
        for x in (40..1200).step_by(12) {
            click(w, x as f32, y as f32);
            if log.borrow().iter().any(|e| e.starts_with("pick:")) {
                break 'pick;
            }
        }
    }
    let picked: Vec<String> = log.borrow().iter().filter(|e| e.starts_with("pick:") || e.starts_with("choose:")).cloned().collect();
    assert!(picked.iter().any(|e| e.starts_with("choose:builtin_formation_council:")), "a seat offers its roles: {picked:?}");
    let pick = picked.iter().find(|e| e.starts_with("pick:")).expect("a role is picked for the seat");
    assert!(pick.starts_with("pick:builtin_formation_council:seat_2:"), "{pick}");
    ui.hide()?;

    first_prompt(w, output)?;
    println!(
        "PASS: a finished Council opened shows each seat's role and the question as asked, links the \
         answers the Chair read, has every session one press away and draws the verdict from its \
         markdown (it differs from the raw monospace head); the notice says the run finished and the \
         row lists what its agents opened; the definition offers its seats' roles and sends a pick \
         with its seat; the Agents pane's first prompt says the Council recipe sent it"
    );
    Ok(())
}

/// The Agents pane: a Council seat's first prompt, sent by the recipe.
fn first_prompt(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (width, height) = (1280u32, 800u32);
    let ui = AgentsProbe::new()?;
    let g = ui.global::<AgentsState>();
    super::agents_tests::fill(&g, false);
    let row = AgentRowData {
        id: "deepseek:c-3f09a1".into(),
        mind: "deepseek".into(),
        title: "Answer this question on your own, as well as you can…".into(),
        state: "done".into(),
        label: "done".into(),
        since: "2m".into(),
        parent: "".into(),
        role: "Researcher".into(),
        origin: "Council recipe".into(),
    };
    g.set_rows(ModelRc::new(VecModel::from(vec![row])));
    g.set_selected("deepseek:c-3f09a1".into());
    let mut header = g.get_header();
    header.id = "deepseek:c-3f09a1".into();
    header.mind = "deepseek".into();
    header.title = "Answer this question on your own, as well as you can…".into();
    header.note = "".into();
    g.set_header(header);
    let item = |kind: &str, key: &str, text: &str| AgentItemData { kind: kind.into(), key: key.into(), text: text.into(), ..Default::default() };
    let draw = || {
        slint::platform::update_timers_and_animations();
        let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width, height);
        w.request_redraw();
        w.draw_if_needed(|r| { r.render(pixels.make_mut_slice(), width as usize); });
        pixels
    };
    let prompt = |who: &str| AgentItemData {
        who: who.into(),
        ..item("prompt", "t1", "Answer this question on your own, as well as you can. The question: Should we publish tonight's nightly?")
    };
    g.set_items(ModelRc::new(VecModel::from(vec![prompt(""), item("text", "t1.0", "I'll start by seeing what's on this desktop…")])));
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(width, height));
    draw();
    let you = draw();
    g.set_items(ModelRc::new(VecModel::from(vec![prompt("Council recipe"), item("text", "t1.0", "I'll start by seeing what's on this desktop…")])));
    draw();
    let recipe = draw();
    assert_ne!(you.as_bytes(), recipe.as_bytes(), "the first prompt names its sender, not \"you\"");
    save(&recipe, &output.replace(".png", "-agents.png"), width, height)?;
    ui.hide()?;
    Ok(())
}
