//! The Recipes screen, drawn by the production component from fixture data — a recipe waiting on
//! the person's answer, one running, one waiting on a timer, one paused, one failed on an unbound
//! `{{topic}}`, one done after taking a jump, and a built-in never run — with real pointer and key
//! events: a choice answers, a typed answer answers, a row opens, a tab filters, Pause reaches the
//! recipe shown, and Cancel asks before it cancels.
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

fn strings(list: &[&str]) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(list.iter().map(|s| SharedString::from(*s)).collect::<Vec<_>>()))
}

struct Row<'a> {
    id: &'a str,
    name: &'a str,
    status: &'a str,
    label: &'a str,
    progress: &'a str,
    when: &'a str,
    stages: &'a [(&'a str, &'a str, &'a str)],
}

fn row(r: Row) -> RecipeRowData {
    RecipeRowData {
        id: r.id.into(),
        name: r.name.into(),
        status: r.status.into(),
        status_label: r.label.into(),
        progress: r.progress.into(),
        when: r.when.into(),
        stages: stages(r.stages),
        can_pause: matches!(r.status, "running" | "waiting"),
        can_resume: r.status == "paused",
        can_cancel: matches!(r.status, "running" | "waiting" | "paused"),
        ..Default::default()
    }
}

/// What the shell would put in the global: the recipes as `wire::recipes::row_of` draws them.
pub fn rows() -> Vec<RecipeRowData> {
    vec![
        RecipeRowData {
            question: "Move the 38 duplicates to which folder?".into(),
            choices: strings(&["Archive", "Trash", "Leave them"]),
            can_answer: true,
            waiting_for: "Waiting for your answer".into(),
            ..row(Row {
                id: "rcp_tidy",
                name: "Tidy downloads",
                status: "waiting",
                label: "waiting for you",
                progress: "step 3 of 5",
                when: "updated 21:06",
                stages: &[
                    ("tool", "list_dir", "done"),
                    ("filter", "Filter", "done"),
                    ("ask_user", "Ask you", "waiting"),
                    ("tool", "move_files", "pending"),
                    ("notify", "Notify", "pending"),
                ],
            })
        },
        row(Row {
            id: "rcp_digest",
            name: "Morning email digest",
            status: "running",
            label: "running",
            progress: "step 3 of 5",
            when: "updated 21:07",
            stages: &[
                ("tool", "email_list", "done"),
                ("filter", "Filter", "done"),
                ("think", "Think", "current"),
                ("render", "Render", "pending"),
                ("notify", "Notify", "pending"),
            ],
        }),
        RecipeRowData {
            waiting_for: "Waiting for 15m to pass, 1h at most".into(),
            ..row(Row {
                id: "rcp_wind",
                name: "Evening wind-down",
                status: "waiting",
                label: "waiting",
                progress: "step 2 of 4",
                when: "updated 21:02",
                stages: &[
                    ("notify", "Notify", "done"),
                    ("wait_for", "Wait", "waiting"),
                    ("tool", "set_dnd", "pending"),
                    ("notify", "Notify", "pending"),
                ],
            })
        },
        RecipeRowData {
            waiting_for: "Paused before step 3.".into(),
            ..row(Row {
                id: "rcp_backup",
                name: "Weekly backup",
                status: "paused",
                label: "paused",
                progress: "step 3 of 4",
                when: "updated 20:41",
                stages: &[
                    ("tool", "disk_usage", "done"),
                    ("jump_if", "If", "done"),
                    ("tool", "run_command", "paused"),
                    ("notify", "Notify", "pending"),
                ],
            })
        },
        RecipeRowData {
            error: "Step 1 failed: web_search: no results for \"{{topic}}\"".into(),
            unbound: "{{topic}} has no value".into(),
            ..row(Row {
                id: "rcp_research",
                name: "Research a topic",
                status: "failed",
                label: "failed",
                progress: "stopped at step 1 of 4",
                when: "updated 20:15",
                stages: &[
                    ("tool", "web_search", "failed"),
                    ("think_cited", "Cite", "not_taken"),
                    ("validate", "Validate", "not_taken"),
                    ("render", "Render", "not_taken"),
                ],
            })
        },
        row(Row {
            id: "rcp_triage",
            name: "Inbox triage",
            status: "done",
            label: "done",
            progress: "5 steps",
            when: "updated 19:30",
            stages: &[
                ("jump_if", "If", "done"),
                ("tool", "summarise", "not_taken"),
                ("notify", "Notify", "not_taken"),
                ("branch", "Branch", "done"),
                ("notify", "Notify", "done"),
            ],
        }),
        RecipeRowData {
            template: true,
            unbound: "{{city}} has no value".into(),
            ..row(Row {
                id: "builtin_morning_briefing",
                name: "Morning briefing",
                status: "pending",
                label: "built-in, never run",
                progress: "6 steps",
                when: "",
                stages: &[
                    ("tool", "get_weather", "pending"),
                    ("tool", "calendar_today", "pending"),
                    ("tool", "email_unread", "pending"),
                    ("think", "Think", "pending"),
                    ("render", "Render", "pending"),
                    ("notify", "Notify", "pending"),
                ],
            })
        },
    ]
}

fn step(number: &str, kind: &str, label: &str, summary: &str, state: &str, state_label: &str) -> RecipeStepData {
    RecipeStepData {
        number: number.into(),
        kind: kind.into(),
        label: label.into(),
        summary: summary.into(),
        state: state.into(),
        state_label: state_label.into(),
        ..Default::default()
    }
}

/// The running digest, opened.
fn digest_steps() -> Vec<RecipeStepData> {
    vec![
        RecipeStepData {
            result: "12 messages: 3 from people, 9 newsletters and receipts".into(),
            detail: "tool: email_list\narguments: {\"folder\":\"INBOX\",\"since\":\"yesterday\"}\nkeeps it as: inbox\non error: retry, 2 times at most".into(),
            ..step("1", "tool", "email_list", "email_list folder=\"INBOX\" since=\"yesterday\"", "done", "done")
        },
        RecipeStepData {
            result: "[{\"from\":\"Asha\",\"subject\":\"Friday?\"},{\"from\":\"Ravi\",\"subject\":\"the draft\"},…]".into(),
            ..step("2", "filter", "Filter", "inbox where from_person = true", "done", "done")
        },
        step("3", "think", "Think", "Summarise {{people}} in three lines, most urgent first", "current", "running now"),
        step("4", "render", "Render", "a summary of summary", "pending", "to come"),
        step("5", "notify", "Notify", "Your morning: {{shown}}", "pending", "to come"),
    ]
}

/// The failed research, opened: the unbound placeholder, its error, what did not run.
fn research_steps() -> Vec<RecipeStepData> {
    vec![
        RecipeStepData {
            result: "web_search: no results for \"{{topic}}\"".into(),
            unbound: "{{topic}} had no value when it ran".into(),
            detail: "tool: web_search\narguments: {\"query\":\"{{topic}} latest\"}\nkeeps it as: hits\non error: stop the recipe".into(),
            ..step("1", "tool", "web_search", "web_search query=\"{{topic}} latest\"", "failed", "failed")
        },
        step("2", "think_cited", "Cite", "What changed recently? — from hits", "not_taken", "not taken"),
        step("3", "validate", "Validate", "keep the cited claims in cited", "not_taken", "not taken"),
        step("4", "render", "Render", "a summary of checked", "not_taken", "not taken"),
    ]
}

fn tabs(g: &RecipesState, active: &str) {
    let tab = |id: &str, label: &str, count: i32| RecipeTabData { id: id.into(), label: label.into(), count };
    g.set_tabs(ModelRc::new(VecModel::from(vec![
        tab("active", "Active", 4),
        tab("finished", "Finished", 2),
        tab("not_run", "Not run", 1),
        tab("all", "All", 7),
    ])));
    g.set_tab(active.into());
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
    tabs(&g, "all");
    g.set_loaded(true);
    g.set_rows(ModelRc::new(VecModel::from(rows())));
    let log: Rc<RefCell<Vec<String>>> = Rc::default();
    {
        let l = log.clone();
        g.on_select(move |id| l.borrow_mut().push(format!("select:{id}")));
        let l = log.clone();
        g.on_select_tab(move |id| l.borrow_mut().push(format!("tab:{id}")));
        let l = log.clone();
        g.on_answer(move |id, text| l.borrow_mut().push(format!("answer:{id}:{text}")));
        let l = log.clone();
        g.on_pause(move |id| l.borrow_mut().push(format!("pause:{id}")));
        let l = log.clone();
        g.on_resume(move |id| l.borrow_mut().push(format!("resume:{id}")));
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
    let logged = |event: &str| log.borrow().iter().any(|e| e == event);
    // Everything below the screen's AppHeader (Theme.h-app-header), which Recipes wears since
    // the VM 520 sweep of 4 October: every press lands that much lower than it did before.
    const HEADER: f32 = 48.;

    // Every recipe, in the desk's order, nothing opened.
    let (width, height) = (1280u32, 1000u32);
    save(&draw(width, height), output, width, height)?;

    // The question, answered from its choices…
    click(w, 69., 217. + HEADER);
    assert!(logged("answer:rcp_tidy:Archive"), "a choice answers: {:?}", log.borrow());
    // …and in the person's own words: Answer does nothing while the box is empty, then sends
    // what was typed; Enter sends too.
    click(w, 1195., 253. + HEADER);
    assert!(!log.borrow().iter().any(|e| e == "answer:rcp_tidy:"), "an empty answer is not sent");
    click(w, 590., 255. + HEADER);
    key(w, "Keep the newest".into());
    draw(width, height);
    click(w, 1195., 253. + HEADER);
    assert!(logged("answer:rcp_tidy:Keep the newest"), "Answer sends the typed answer: {:?}", log.borrow());
    click(w, 590., 255. + HEADER);
    key(w, "Leave them here".into());
    key(w, "\n".into());
    assert!(logged("answer:rcp_tidy:Leave them here"), "Enter sends it: {:?}", log.borrow());
    // A row opens from its name; a tab filters.
    click(w, 113., 327. + HEADER);
    assert!(logged("select:rcp_digest"), "{:?}", log.borrow());
    click(w, 125., 32. + HEADER);
    assert!(logged("tab:finished"), "{:?}", log.borrow());

    // The running digest, opened below its row, with Pause and Cancel.
    g.set_selected("rcp_digest".into());
    g.set_steps(ModelRc::new(VecModel::from(digest_steps())));
    save(&draw(1280, 1000), &output.replace(".png", "-open.png"), 1280, 1000)?;
    click(w, 1123., 718. + HEADER);
    assert!(logged("pause:rcp_digest"), "Pause reaches the recipe shown: {:?}", log.borrow());
    // Cancel asks first: one press cancels nothing, the second — on "Cancel recipe" — does.
    click(w, 1208., 718. + HEADER);
    draw(1280, 1000);
    assert!(!logged("cancel:rcp_digest"), "Cancel asks before it cancels");
    save(&draw(1280, 1000), &output.replace(".png", "-confirm.png"), 1280, 1000)?;
    click(w, 1215., 718. + HEADER);
    assert!(logged("cancel:rcp_digest"), "and cancels once asked: {:?}", log.borrow());

    // Finished: the failed research opened, with its unbound {{topic}}.
    let finished: Vec<RecipeRowData> = rows().into_iter().filter(|r| r.status == "failed" || r.status == "done").collect();
    g.set_rows(ModelRc::new(VecModel::from(finished)));
    tabs(&g, "finished");
    g.set_selected("rcp_research".into());
    g.set_steps(ModelRc::new(VecModel::from(research_steps())));
    save(&draw(1280, 800), &output.replace(".png", "-failed.png"), 1280, 800)?;

    // And the overview in the light theme.
    g.set_rows(ModelRc::new(VecModel::from(rows())));
    tabs(&g, "all");
    g.set_selected("".into());
    ui.set_light(true);
    std::thread::sleep(std::time::Duration::from_millis(300));
    save(&draw(1280, 1000), &output.replace(".png", "-light.png"), 1280, 1000)?;
    ui.hide()?;
    println!(
        "PASS: a choice answers, a typed answer sends on Answer and on Enter (never empty), a row \
         opens, a tab filters, Pause reaches the recipe shown, Cancel asks first; overview, opened, \
         failed and light rendered"
    );
    Ok(())
}
