//! The Agents screen and an agent's own window, drawn by the production components from fixture
//! data, with real pointer events: the rows do not move under the pointer because the screen tells
//! the shell when the pointer is over the list, a row and a tab select, and a card opens.
use super::*;
use slint::{ModelRc, VecModel};
use std::cell::RefCell;

fn runs(lines: &[(&str, (u8, u8, u8), bool)]) -> ModelRc<AgentRunData> {
    let bg = slint::Color::from_rgb_u8(16, 23, 30);
    ModelRc::new(VecModel::from(
        lines
            .iter()
            .enumerate()
            .map(|(row, (text, fg, bold))| AgentRunData {
                text: (*text).into(),
                row: row as i32,
                col: 0,
                columns: text.chars().count() as i32,
                fg: slint::Color::from_rgb_u8(fg.0, fg.1, fg.2),
                bg,
                bold: *bold,
            })
            .collect::<Vec<_>>(),
    ))
}

fn call(name: &str, target: &str, summary: &str, arguments: &str, status: &str, output: &str) -> ToolCallData {
    ToolCallData {
        name: name.into(),
        target: target.into(),
        summary: summary.into(),
        arguments: arguments.into(),
        status: status.into(),
        output: output.into(),
    }
}

/// What the shell would put in the global for pi, two minutes into tidying a photos folder.
pub(crate) fn fill(g: &AgentsState, popped: bool) {
    const PLAIN: (u8, u8, u8) = (222, 230, 239);
    const GREEN: (u8, u8, u8) = (130, 207, 156);
    let tab = |id: &str, label: &str, count: i32| AgentTabData { id: id.into(), label: label.into(), count };
    g.set_tabs(ModelRc::new(VecModel::from(vec![
        tab("active", "Active", 2),
        tab("needs_you", "Needs you", 1),
        tab("complete", "Complete", 4),
        tab("all", "All", 6),
    ])));
    let row = |id: &str, mind: &str, title: &str, state: &str, label: &str, since: &str| AgentRowData {
        id: id.into(),
        mind: mind.into(),
        title: title.into(),
        state: state.into(),
        label: label.into(),
        since: since.into(),
        parent: "".into(),
        role: "".into(),
        origin: "".into(),
    };
    g.set_rows(ModelRc::new(VecModel::from(vec![
        row("deepseek:main", "DeepSeek", "release notes for 0.4", "waiting_for_you", "waiting for you", "40s"),
        row("pi:main", "pi", "tidy the photos folder, dupes into Trash", "running_tool", "running a tool", "2m"),
    ])));
    g.set_selected("pi:main".into());
    g.set_has_agent(true);
    g.set_popped(popped);
    g.set_header(AgentHeaderData {
        id: "pi:main".into(),
        mind: "pi".into(),
        title: "tidy the photos folder, dupes into Trash".into(),
        state: "running_tool".into(),
        label: "running a tool".into(),
        since: "2m".into(),
        status: "".into(),
        note: "pi holds one conversation at a time — the same one the Lens talks to.".into(),
        can_send: false,
        send_hint: "pi is working — wait, or Stop it".into(),
        can_stop: true,
    });
    let item = |kind: &str, key: &str, text: &str| AgentItemData {
        kind: kind.into(),
        key: key.into(),
        text: text.into(),
        ..Default::default()
    };
    g.set_items(ModelRc::new(VecModel::from(vec![
        item("prompt", "t1", "tidy the photos folder, dupes into Trash"),
        item("text", "t1.0", "I'll find duplicates by hash first, then move the copies — not the originals."),
        AgentItemData {
            expanded: false,
            ..item("thinking", "t1.1", "Hashing is safer than names: a copy can be renamed. fdupes -r lists groups; keep the oldest of each.")
        },
        AgentItemData {
            call: call("agent_run", "", r#"agent_run command="fdupes -r ~/Pictures""#, "{\n  \"command\": \"fdupes -r ~/Pictures\"\n}", "done", " "),
            badge: "verified · exit 0".into(),
            output_kind: "terminal".into(),
            more: "214 lines in all".into(),
            can_open_all: true,
            ..item("card", "t1.2", "")
        },
        item("text", "t1.3", "38 duplicates in 17 groups. Asking before anything moves."),
        AgentItemData {
            call: call("os_act", "files.move", r#"os_act files.move from="~/Pictures/copy of a.jpg" to="~/.local/share/Trash""#, "{}", "", " "),
            badge: "reported".into(),
            ..item("card", "t1.4", "")
        },
        AgentItemData {
            call: call("agent_run", "", r#"agent_run command="du -sh ~/Pictures""#, "", "running", ""),
            badge: "verified".into(),
            live: true,
            output_kind: "terminal".into(),
            runs: runs(&[("4.1G\t/home/pranab/Pictures", PLAIN, false)]),
            rows: 1,
            ..item("card", "t1.5", "")
        },
        AgentItemData {
            call: call("agent_run", "", r#"agent_run command="ls ~/Pictures/raw""#, "", "failed", "ls: cannot access '/home/pranab/Pictures/raw': No such file or directory"),
            badge: "verified · exit 2".into(),
            explain: "Run by the shell itself; the exit code is the process's own.".into(),
            expanded: true,
            output_kind: "text".into(),
            ..item("card", "t1.6", "")
        },
        item("note", "t1.7", "Asked you: files.move 38 files → Trash"),
    ])));
    // The finished command, opened: its last screen, drawn from cells.
    let model = g.get_items();
    let mut done = slint::Model::row_data(&model, 3).unwrap();
    done.expanded = true;
    done.call.output = "".into();
    done.runs = runs(&[
        ("/home/pranab/Pictures/2024/beach.jpg", GREEN, true),
        ("/home/pranab/Pictures/copy of beach.jpg", PLAIN, false),
        ("", PLAIN, false),
        ("/home/pranab/Pictures/a.jpg", GREEN, true),
        ("/home/pranab/Pictures/old/a (1).jpg", PLAIN, false),
    ]);
    done.rows = 5;
    slint::Model::set_row_data(&model, 3, done);
    g.set_details(AgentDetailsData {
        mind: "pi".into(),
        model: "qwen3.8-27b".into(),
        since: "21:04".into(),
        turns: "1".into(),
        calls: "4 (1 failed)".into(),
        commands: "3 (1 failed)".into(),
        command_lines: "  exit 0  fdupes -r ~/Pictures\n running  du -sh ~/Pictures\n  exit 2  ls ~/Pictures/raw".into(),
        files: "none named".into(),
        file_lines: "".into(),
        approvals: "1 asked · 0 answered".into(),
        tokens: "41k in · 1.2k out".into(),
        cost: "".into(),
        refused: "".into(),
        refused_lines: "".into(),
        basis: "Commands, files and approvals count only what the shell itself ran or asked. Calls include what the harness reported.".into(),
        role: "".into(),
        reach: "".into(),
        reach_patterns: "".into(),
    });
}

/// The first live catalog run (#190): the Red team finished, the Active tab empty beside six
/// complete, and its answer — headings, bold labels, italics, backticks, a list, a fence — as the
/// shell hands it to the pane: one item per block of the Lens's parser (`crate::markdown`), a
/// paragraph's and a list's styles made by `StyledText::from_markdown`, as `markdown::styled` does.
/// The last paragraph is still arriving, with its `**` not yet closed.
fn red_team(g: &AgentsState) {
    let tab = |id: &str, label: &str, count: i32| AgentTabData { id: id.into(), label: label.into(), count };
    g.set_tabs(ModelRc::new(VecModel::from(vec![
        tab("active", "Active", 0),
        tab("needs_you", "Needs you", 0),
        tab("complete", "Complete", 6),
        tab("all", "All", 6),
    ])));
    g.set_tab("active".into());
    g.set_rows(ModelRc::new(VecModel::from(Vec::<AgentRowData>::new())));
    g.set_empty_note("Nothing running. 6 complete.".into());
    g.set_selected("".into());
    g.set_has_agent(true);
    g.set_header(AgentHeaderData {
        id: "deepseek:c-4e1f07".into(),
        mind: "deepseek".into(),
        title: "attack the plan to ship 0.4 on Friday".into(),
        state: "done".into(),
        label: "done".into(),
        since: "21:14".into(),
        status: "".into(),
        note: "".into(),
        can_send: true,
        send_hint: "".into(),
        can_stop: false,
    });
    let mut details = g.get_details();
    details.mind = "deepseek".into();
    details.role = "Red team".into();
    details.reach = "nothing on this desktop, and it may ask for safe acts".into();
    details.reach_patterns = "nothing on this desktop beyond asking the person and reading its own session · at most safe".into();
    details.calls = "3".into();
    g.set_details(details);
    let block = |key: &str, block: &str, text: &str, markdown: &str| AgentItemData {
        kind: "text".into(),
        key: key.into(),
        block: block.into(),
        text: text.into(),
        styled: match block {
            "text" | "bullet" => slint::StyledText::from_markdown(markdown).expect("markdown StyledText takes"),
            _ => slint::StyledText::from_plain_text(text),
        },
        ..Default::default()
    };
    g.set_items(ModelRc::new(VecModel::from(vec![
        AgentItemData { kind: "prompt".into(), key: "t1".into(), text: "attack the plan to ship 0.4 on Friday".into(), ..Default::default() },
        block("t1.0.0", "heading", "Strongest point", "Strongest point"),
        block(
            "t1.0.1",
            "text",
            "How: the notes ship before the migration is tested, and release-check --tier rc is the only gate between them.",
            "**How:** the notes ship *before* the migration is tested, and `release-check --tier rc` is the only gate between them.",
        ),
        block(
            "t1.0.2",
            "bullet",
            "\u{2022} Risk: a failed migration leaves ~/.local/share/yantrik half-written, and the next boot reads a store that is neither the old one nor the new one\n\u{2022} Mitigation: run it on a copy first\n1. snapshot the folder\n2. migrate the copy",
            "\u{2022} **Risk:** a failed migration leaves `~/.local/share/yantrik` half-written, and the next boot reads a store that is *neither* the old one nor the new one\n\u{2022} *Mitigation:* run it on a copy first\n1. snapshot the folder\n2. migrate the copy",
        ),
        block("t1.0.3", "heading", "What I would check", "What I would check"),
        block(
            "t1.0.4",
            "code",
            "cp -a ~/.local/share/yantrik /tmp/y-copy\nrelease-check --tier rc --interactive --json /tmp/rc.json --only browser --only \"no window\" --skip blender",
            "cp -a ~/.local/share/yantrik /tmp/y-copy\nrelease-check --tier rc --interactive --json /tmp/rc.json --only browser --only \"no window\" --skip blender",
        ),
        block("t1.0.5", "text", "Verdict: it is **still arriving", "Verdict: it is **still arriving"),
    ])));
}

fn save(pixels: &slint::SharedPixelBuffer<slint::Rgb8Pixel>, path: &str, width: u32, height: u32) -> Result<(), Box<dyn std::error::Error>> {
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(path)?), width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(pixels.as_bytes())?;
    println!("Rendered {path} ({width}×{height})");
    Ok(())
}

fn hover(window: &MinimalSoftwareWindow, x: f32, y: f32) {
    window.dispatch_event(slint::platform::WindowEvent::PointerMoved { position: slint::LogicalPosition::new(x, y) });
}

pub fn run(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (width, height) = (1280u32, 800u32);
    let ui = AgentsProbe::new()?;
    let g = ui.global::<AgentsState>();
    fill(&g, false);
    let log: Rc<RefCell<Vec<String>>> = Rc::default();
    {
        let l = log.clone();
        g.on_select(move |id| l.borrow_mut().push(format!("select:{id}")));
        let l = log.clone();
        g.on_select_tab(move |id| l.borrow_mut().push(format!("tab:{id}")));
        let l = log.clone();
        g.on_toggle(move |key, open| l.borrow_mut().push(format!("toggle:{key}:{open}")));
        let l = log.clone();
        g.on_pop_out(move |id| l.borrow_mut().push(format!("pop-out:{id}")));
        let l = log.clone();
        g.on_stop(move |id| l.borrow_mut().push(format!("stop:{id}")));
    }
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(width, height));
    let draw = || {
        slint::platform::update_timers_and_animations();
        let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width, height);
        w.request_redraw();
        w.draw_if_needed(|r| { r.render(pixels.make_mut_slice(), width as usize); });
        pixels
    };
    let first = draw();
    save(&first, output, width, height)?;

    // The pointer over the list tells the shell; leaving tells it again.
    hover(w, 150., 200.);
    draw();
    draw();
    assert!(g.get_list_hovered(), "the pointer over the list is reported, so rows hold still under it");
    hover(w, 640., 30.);
    draw();
    draw();
    assert!(!g.get_list_hovered(), "and its leaving is reported, so a row that needs you can rise");

    // A row selects its agent; a double click pops it out.
    click(w, 150., 101.);
    assert!(log.borrow().contains(&"select:deepseek:main".to_string()), "{:?}", log.borrow());
    // A tab filters.
    let tabs_y = 33.;
    for x in (20..420).step_by(8) {
        click(w, x as f32, tabs_y);
    }
    assert!(log.borrow().iter().any(|e| e == "tab:complete"), "{:?}", log.borrow());
    // Stop reaches the agent shown.
    for y in (height as i32 - 60..height as i32 - 16).step_by(4) {
        click(w, 1060., y as f32);
    }
    assert!(log.borrow().iter().any(|e| e == "stop:pi:main"), "{:?}", log.borrow());

    // A card opens from its line, and says which card to the shell.
    click(w, 600., 476.);
    assert!(log.borrow().iter().any(|e| e == "toggle:t1.4:true"), "{:?}", log.borrow());

    ui.set_light(true);
    hover(w, 640., 790.);
    draw();
    // Past the buttons' colour animation, so the picture is the light theme and not a blend.
    std::thread::sleep(std::time::Duration::from_millis(300));
    save(&draw(), &output.replace(".png", "-light.png"), width, height)?;

    // An approval in the pane: the shell's own card (the Lens's component), naming the agent, with
    // Deny and Allow each answering the one request id it carries.
    ui.set_light(false);
    {
        let l = log.clone();
        g.on_approval_allow(move |id| l.borrow_mut().push(format!("allow:{id}")));
        let l = log.clone();
        g.on_approval_deny(move |id| l.borrow_mut().push(format!("deny:{id}")));
        let l = log.clone();
        g.on_approval_allow_session(move |id| l.borrow_mut().push(format!("allow-session:{id}")));
    }
    let item = |kind: &str, key: &str, text: &str| AgentItemData {
        kind: kind.into(),
        key: key.into(),
        text: text.into(),
        ..Default::default()
    };
    let lines = |rows: &[&str]| ModelRc::new(VecModel::from(rows.iter().map(|r| slint::SharedString::from(*r)).collect::<Vec<_>>()));
    let approval = ApprovalRequest {
        id: "appr-7".into(),
        agent: "pi:main".into(),
        on_behalf: "".into(),
        requester: "pi 0.87".into(),
        verified: "pi --mode rpc (pid 4242) · the attached mind".into(),
        discrepancies: lines(&[]),
        app: "files".into(),
        action: "move".into(),
        summary: "Move files or folders to another place, or into the recoverable Trash.".into(),
        purpose: "Move files or folders to another place, or into the recoverable Trash.".into(),
        grade: "sensitive".into(),
        args: lines(&["from: ~/Pictures/copy of a.jpg", "to: ~/.local/share/Trash"]),
        // Files names no handle — both arguments are paths a person can read (#54).
        target: "".into(),
        explained: "".into(),
        warning: "".into(),
        can_session: false,
        decision: "".into(),
        record: "".into(),
        age_text: "94s left".into(),
    };
    g.set_items(ModelRc::new(VecModel::from(vec![
        item("prompt", "t2", "move the duplicates into Trash"),
        item("text", "t2.0", "38 duplicates in 17 groups. Asking before anything moves."),
        AgentItemData { approval, ..item("approval", "t2.1", "files.move") },
    ])));
    draw();
    draw();
    save(&draw(), &output.replace(".png", "-approval.png"), width, height)?;
    // Allow is the right-hand button of the pair; scan the session's right half from the bottom up
    // until one of the two answers, and it must be Allow, for appr-7.
    let answered = |log: &Rc<RefCell<Vec<String>>>| log.borrow().iter().any(|e| e.starts_with("allow") || e.starts_with("deny"));
    for y in (90..(height as i32 - 70)).rev().step_by(4) {
        if answered(&log) {
            break;
        }
        click(w, 840., y as f32);
    }
    let answers: Vec<String> = log.borrow().iter().filter(|e| e.starts_with("allow") || e.starts_with("deny")).cloned().collect();
    assert_eq!(answers, vec!["allow:appr-7".to_string()], "the pane's Allow answers the one request id");
    for y in (90..(height as i32 - 70)).rev().step_by(4) {
        if log.borrow().iter().any(|e| e.starts_with("deny")) {
            break;
        }
        click(w, 460., y as f32);
    }
    assert!(log.borrow().iter().any(|e| e == "deny:appr-7"), "and its Deny the same id: {:?}", log.borrow());
    // Answered, it is the line it left, with no buttons: nothing to press any more.
    let answered_card = ApprovalRequest {
        id: "appr-7".into(),
        agent: "pi:main".into(),
        app: "files".into(),
        action: "move".into(),
        decision: "allowed".into(),
        record: "Allowed once: files.move — 21:05".into(),
        ..Default::default()
    };
    g.set_items(ModelRc::new(VecModel::from(vec![
        item("prompt", "t2", "move the duplicates into Trash"),
        AgentItemData { approval: answered_card, ..item("approval", "t2.1", "files.move") },
    ])));
    let before = log.borrow().len();
    draw();
    for y in (90..(height as i32 - 70)).step_by(6) {
        click(w, 840., y as f32);
        click(w, 460., y as f32);
    }
    let pressed: Vec<String> = log.borrow()[before..].iter().filter(|e| e.starts_with("allow") || e.starts_with("deny")).cloned().collect();
    assert!(pressed.is_empty(), "an answered card has no buttons: {pressed:?}");
    save(&draw(), &output.replace(".png", "-approval-answered.png"), width, height)?;

    // #190: a finished Red team, still open in the middle column, with the Active tab empty — the
    // list says what is true of the other tabs — and its answer drawn from its markdown.
    red_team(&g);
    draw();
    draw();
    let rich = draw();
    save(&rich, &output.replace(".png", "-markdown.png"), width, height)?;
    // The styles are drawn, not only parsed: the same paragraph as plain text is another picture.
    let styled_at = |items: &ModelRc<AgentItemData>| {
        (0..slint::Model::row_count(items)).find(|&i| slint::Model::row_data(items, i).is_some_and(|it| it.key == "t1.0.1")).unwrap()
    };
    let items = g.get_items();
    let row = styled_at(&items);
    let mut paragraph = slint::Model::row_data(&items, row).unwrap();
    let styled = paragraph.styled.clone();
    paragraph.styled = slint::StyledText::from_plain_text(&paragraph.text);
    slint::Model::set_row_data(&items, row, paragraph.clone());
    draw();
    let flat = draw();
    assert_ne!(rich.as_bytes(), flat.as_bytes(), "bold, italic and code are drawn: the paragraph differs from its plain text");
    paragraph.styled = styled;
    slint::Model::set_row_data(&items, row, paragraph);
    ui.set_light(true);
    draw();
    std::thread::sleep(std::time::Duration::from_millis(300));
    save(&draw(), &output.replace(".png", "-markdown-light.png"), width, height)?;
    ui.set_light(false);
    ui.hide()?;

    // One agent in its own window: the same components, its own global.
    let window = AgentWindow::new()?;
    window.set_agent_title("Agent · pi · tidy the photos folder, dupes into Trash".into());
    fill(&window.global::<AgentsState>(), true);
    window.show()?;
    let (ww, wh) = (1000u32, 680u32);
    w.set_size(slint::PhysicalSize::new(ww, wh));
    slint::platform::update_timers_and_animations();
    let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(ww, wh);
    w.request_redraw();
    w.draw_if_needed(|r| { r.render(pixels.make_mut_slice(), ww as usize); });
    save(&pixels, &output.replace(".png", "-window.png"), ww, wh)?;
    println!("PASS: Agents list hover reported and cleared, row select, tab filter, Stop, a card opened from its line; an approval card in the pane answered Allow and Deny for its one request id, and drew no buttons once answered; screen and window rendered");
    Ok(())
}

/// The shipped catalog as the shell lists it in New agent: name, purpose, reach, and the mind each
/// would run on with deepseek and pi attached (the Researcher and the Writer prefer openclaw first).
fn roles() -> ModelRc<AgentRoleData> {
    let role = |id: &str, name: &str, purpose: &str, reach: &str, runs_on: &str| AgentRoleData {
        id: id.into(),
        name: name.into(),
        purpose: purpose.into(),
        reach: reach.into(),
        runs_on: runs_on.into(),
        available: !runs_on.is_empty(),
    };
    ModelRc::new(VecModel::from(vec![
        role("researcher", "Researcher", "Finds out what is true and says how it knows, with sources.", "shell.open_app · at most standard", "deepseek"),
        role("planner", "Planner", "Turns a goal into steps someone can start on today; reads only.", "calendar and notes · at most safe", "deepseek"),
        role("coder", "Coder", "Makes a code change and proves it with the build and the tests.", "shell.agent_* and editor · at most sensitive", "pi"),
        role("reviewer", "Reviewer", "Reviews a change for bugs and risks; reads only.", "editor, documents and notes · at most safe", "deepseek"),
        role("red-team", "Red team", "Attacks a proposal to find how it breaks; touches nothing.", "nothing on this desktop beyond asking the person and reading its own session · at most safe", "deepseek"),
        role("writer", "Writer", "Writes a piece for its reader: a note, an email, release notes, a page.", "notes, documents and editor · at most standard", ""),
        role("chair", "Chair", "Weighs several answers to one question and gives a verdict.", "nothing on this desktop beyond asking the person and reading its own session · at most safe", "deepseek"),
        role("scribe", "Scribe", "Summarises a session, a document or a discussion for someone who was not there.", "notes · at most standard", "pi"),
    ]))
}

/// Agents catalog: New agent → from the catalog, drawn by the production screen. A role's row
/// names its role and its details say its reach; the dialog lists each role with its purpose and
/// where it would run; a real press on a role picks it, on the mode chips switches between a mind
/// and the catalog, and Start hands the picked role its task.
pub fn run_catalog(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (width, height) = (1280u32, 800u32);
    let ui = AgentsProbe::new()?;
    let g = ui.global::<AgentsState>();
    fill(&g, false);
    // A Reviewer handed a change to look at, selected: its row names the role, its details the reach.
    let row = AgentRowData {
        id: "deepseek:c-1a2b3c".into(),
        mind: "deepseek".into(),
        title: "review the change in ~/src/app before I ship it".into(),
        state: "thinking".into(),
        label: "thinking".into(),
        since: "12s".into(),
        parent: "".into(),
        role: "Reviewer".into(),
        origin: "".into(),
    };
    let mut rows: Vec<AgentRowData> = slint::Model::iter(&g.get_rows()).collect();
    rows.insert(0, row);
    g.set_rows(ModelRc::new(VecModel::from(rows)));
    g.set_selected("deepseek:c-1a2b3c".into());
    let mut header = g.get_header();
    header.id = "deepseek:c-1a2b3c".into();
    header.mind = "deepseek".into();
    header.title = "review the change in ~/src/app before I ship it".into();
    header.note = "".into();
    g.set_header(header);
    let mut details = g.get_details();
    details.mind = "deepseek".into();
    details.role = "Reviewer".into();
    details.reach = "the Editor, Documents and Notes, and it may ask for safe acts".into();
    details.reach_patterns = "editor, documents and notes · at most safe".into();
    g.set_details(details);
    g.set_roles(roles());

    let log: Rc<RefCell<Vec<String>>> = Rc::default();
    {
        let (l, weak) = (log.clone(), ui.as_weak());
        g.on_pick_role(move |id| {
            l.borrow_mut().push(format!("pick-role:{id}"));
            // What the shell does: the role is picked, the note says where it runs and what it
            // may reach in words, and the patterns behind the words go with it (#212).
            if let Some(ui) = weak.upgrade() {
                let g = ui.global::<AgentsState>();
                g.set_new_role(id.clone());
                g.set_new_note(format!("Runs on deepseek. May reach: the Editor, Documents and Notes, and it may ask for safe acts. Up to 4 turns and 15 minutes. ({id})").into());
                g.set_new_note_reach("editor, documents and notes · at most safe".into());
            }
        });
        let l = log.clone();
        g.on_start_role(move |role, task| l.borrow_mut().push(format!("start-role:{role}:{task}")));
        let l = log.clone();
        g.on_start(move |mind, task| l.borrow_mut().push(format!("start:{mind}:{task}")));
        let l = log.clone();
        g.on_pick_mind(move |mind| l.borrow_mut().push(format!("pick-mind:{mind}")));
    }
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
    save(&draw(), &output.replace(".png", "-role.png"), width, height)?;

    // New agent, from the catalog.
    g.set_new_open(true);
    g.set_new_from_catalog(true);
    draw();
    std::thread::sleep(std::time::Duration::from_millis(300));
    save(&draw(), output, width, height)?;

    // A press on a role picks it — the list's rows are the dialog's full width.
    for y in (120..680).step_by(6) {
        if log.borrow().iter().any(|e| e.starts_with("pick-role:")) {
            break;
        }
        click(w, 640., y as f32);
    }
    let picked: Vec<String> = log.borrow().iter().filter(|e| e.starts_with("pick-role:")).cloned().collect();
    assert_eq!(picked.len(), 1, "one press, one role: {:?}", log.borrow());
    assert!(g.get_new_open(), "picking a role keeps the dialog open");
    draw();
    std::thread::sleep(std::time::Duration::from_millis(300));
    save(&draw(), &output.replace(".png", "-picked.png"), width, height)?;

    // Start hands the picked role its task: the rightmost button along the dialog's bottom.
    for y in (300..760).rev().step_by(4) {
        if log.borrow().iter().any(|e| e.starts_with("start")) || !g.get_new_open() {
            break;
        }
        for x in (820..900).rev().step_by(8) {
            click(w, x as f32, y as f32);
            if log.borrow().iter().any(|e| e.starts_with("start")) || !g.get_new_open() {
                break;
            }
        }
    }
    let role = picked[0].trim_start_matches("pick-role:");
    assert!(
        log.borrow().iter().any(|e| e == &format!("start-role:{role}:")),
        "Start hands the picked role its task, and nothing else starts: {:?}",
        log.borrow()
    );
    assert!(!log.borrow().iter().any(|e| e.starts_with("start:")), "not a mind: {:?}", log.borrow());

    // The chips switch between a mind and the catalog.
    for y in (100..400).step_by(4) {
        if !g.get_new_from_catalog() {
            break;
        }
        for x in (380..520).step_by(10) {
            click(w, x as f32, y as f32);
            if !g.get_new_from_catalog() {
                break;
            }
        }
    }
    assert!(!g.get_new_from_catalog(), "the \"A mind\" chip leaves the catalog");
    assert!(g.get_new_open());
    draw();
    save(&draw(), &output.replace(".png", "-mind.png"), width, height)?;
    ui.set_light(true);
    g.set_new_from_catalog(true);
    draw();
    std::thread::sleep(std::time::Duration::from_millis(300));
    save(&draw(), &output.replace(".png", "-light.png"), width, height)?;
    println!("PASS: a role's row names its role and its details its reach; New agent lists the catalog's roles with their purposes; a press picks a role, Start hands it the task, and the chips switch between a mind and the catalog");
    Ok(())
}

/// The Lens in a conversation with an attached mind: its header offers "open in Agents", a press
/// reaches the shell, and a Lens talking to no agent (the built-in) offers nothing.
pub fn run_lens(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (width, height) = (1280u32, 800u32);
    let ui = LensAgentsProbe::new()?;
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
    // Past the panel's slide-in.
    std::thread::sleep(std::time::Duration::from_millis(300));
    save(&draw(), output, width, height)?;
    // The header runs along the top of the panel, right of the mind's name and left of ×.
    for x in (900..1240).step_by(6) {
        for y in (34..76).step_by(6) {
            if ui.get_opened() == 0 {
                click(w, x as f32, y as f32);
            }
        }
    }
    assert_eq!(ui.get_opened(), 1, "the header's \"open in Agents\" reaches the shell");
    let closed_before = ui.get_closed();
    // With nothing to open, there is no button: the same sweep opens nothing (× may still close).
    ui.set_can_open(false);
    draw();
    for x in (900..1200).step_by(6) {
        for y in (34..76).step_by(6) {
            click(w, x as f32, y as f32);
        }
    }
    assert_eq!(ui.get_opened(), 1, "no button when the conversation is no agent's");
    let _ = closed_before;
    save(&draw(), &output.replace(".png", "-builtin.png"), width, height)?;
    println!("PASS: the Lens header offers open in Agents for an agent's conversation, the press reaches the shell, and it is not offered otherwise");
    Ok(())
}
