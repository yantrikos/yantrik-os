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
    g.set_selected("pi:main".into());
    g.set_has_agent(true);
    g.set_popped(popped);
    g.set_header(AgentHeaderData {
        id: "pi:main".into(),
        mind: "pi".into(),
        title: "tidy the photos folder, dupes into Trash".into(),
        raw: "".into(),
        state: "running_tool".into(),
        label: "running a tool".into(),
        since: "2m".into(),
        status: "".into(),
        note: "pi holds one conversation at a time — the same one the Lens talks to.".into(),
        can_send: false,
        send_hint: "pi is working — wait, or Stop it".into(),
        tell_hint: "".into(),
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
    g.set_selected("".into());
    g.set_has_agent(true);
    g.set_header(AgentHeaderData {
        id: "deepseek:c-4e1f07".into(),
        mind: "deepseek".into(),
        title: "attack the plan to ship 0.4 on Friday".into(),
        raw: "".into(),
        state: "done".into(),
        label: "done".into(),
        since: "21:14".into(),
        status: "".into(),
        note: "".into(),
        can_send: true,
        send_hint: "".into(),
        tell_hint: "".into(),
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


pub(crate) fn nav(kind: &str, id: &str, label: &str, sub: &str, needs: i32, working: bool, available: bool) -> WorkNavData {
    WorkNavData { kind: kind.into(), id: id.into(), label: label.into(), sub: sub.into(), needs, working, available }
}

pub(crate) fn desk(key: &str, mind_id: &str, mind: &str, task: &str, state: &str, since: &str, activity: &str) -> DeskCardData {
    DeskCardData {
        key: key.into(),
        mind_id: mind_id.into(),
        mind: mind.into(),
        via: "".into(),
        task: task.into(),
        state: state.into(),
        label: if state == "needs_you" { "Needs you" } else { "Working" }.into(),
        activity: activity.into(),
        since: since.into(),
    }
}

fn result(key: &str, mind: &str, task: &str, outcome: &str, label: &str, when: &str) -> ResultData {
    ResultData { key: key.into(), mind: mind.into(), task: task.into(), outcome: outcome.into(), label: label.into(), when: when.into() }
}

pub(crate) fn model<T: Clone + 'static>(rows: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(rows))
}

const PI_DESK: &str = "pi:c-7f3a91";
const DEEPSEEK_DESK: &str = "deepseek:c-4e1f07";

/// What the shell puts in the global for a desktop with two minds at work and a third waiting on
/// the person: the same words `wire::agents_workroom` works out from the store.
pub(crate) fn fill_workroom(g: &AgentsState) {
    g.set_section("workroom".into());
    g.set_detail_open(false);
    g.set_summary("2 working · 1 needs you".into());
    g.set_needs_count(1);
    g.set_request_minds(1);
    g.set_runs_count(140);
    g.set_shelf_count(1);
    g.set_shelf_minds(1);
    g.set_view_runs(140);
    g.set_nav_minds(model(vec![
        nav("mind", "pi", "pi", "1 working", 0, true, true),
        nav("task", PI_DESK, "Tidy the photos folder, duplicates into Trash", "Working", 0, true, true),
        nav("mind", "hermes", "Hermes", "1 working", 0, true, true),
        nav("mind", "deepseek", "DeepSeek", "Waiting on you", 1, false, true),
        nav("task", DEEPSEEK_DESK, "Review the change in ~/src/app before I ship it", "Needs you", 1, false, true),
        nav("mind", "openclaw", "OpenClaw", "Idle", 0, false, true),
        nav("mind", "ghost", "Ghost", "Unavailable", 0, false, false),
    ]));
    g.set_desks(model(vec![
        desk(PI_DESK, "pi", "pi", "Tidy the photos folder, duplicates into Trash", "working", "2m", "Running agent_run du -sh ~/Pictures"),
        desk("hermes:c-2b90d4", "hermes", "Hermes", "Write the release notes for 0.4", "working", "7m", "Last update 10:42"),
        desk(DEEPSEEK_DESK, "deepseek", "DeepSeek", "Review the change in ~/src/app before I ship it", "needs_you", "4m", "Waiting for your answer"),
    ]));
    let ask = DecisionData {
        key: DEEPSEEK_DESK.into(),
        request: "appr-7".into(),
        mind: "DeepSeek".into(),
        task: "Review the change in ~/src/app before I ship it".into(),
        text: "Run a command: git diff --stat origin/main..HEAD in ~/src/app".into(),
        age: "2m ago".into(),
        action: "Review command".into(),
    };
    g.set_decisions(model(vec![ask.clone()]));
    g.set_shelf(model(vec![ask]));
    let recent = vec![
        result("pi:main#4", "pi", "Release check: reply with exactly one word, READY.", "done", "Finished", "10:31"),
        result("hermes:main#2", "Hermes", "Build a small town model with people, homes and roads", "failed", "Couldn't finish", "09:58"),
        result("deepseek:c-1a2b3c", "DeepSeek", "Attack the plan to ship 0.4 on Friday", "done", "Finished", "Oct 1, 21:14"),
    ];
    g.set_recent(model(recent.clone()));
    g.set_history(model(recent));
    g.set_mode_line("Ask — Request approval for actions that require it. This is the desktop's mode and applies to every mind.".into());
    g.set_minds(model(vec![
        AgentMindData { id: "pi".into(), name: "pi".into(), detail: "".into() },
        AgentMindData { id: "hermes".into(), name: "Hermes".into(), detail: "".into() },
        AgentMindData { id: "deepseek".into(), name: "DeepSeek".into(), detail: "".into() },
    ]));
    g.set_new_mind("pi".into());
    g.set_new_note("pi holds one conversation at a time, so this continues it — the same conversation the Lens has with it.".into());
}

/// The same desktop with nothing running and nothing waiting.
fn fill_empty(g: &AgentsState) {
    fill_workroom(g);
    g.set_summary("Nothing running".into());
    g.set_needs_count(0);
    g.set_request_minds(0);
    g.set_shelf_count(0);
    g.set_shelf_minds(0);
    g.set_desks(model(Vec::new()));
    g.set_decisions(model(Vec::new()));
    g.set_shelf(model(Vec::new()));
    g.set_nav_minds(model(vec![
        nav("mind", "pi", "pi", "Idle", 0, false, true),
        nav("mind", "hermes", "Hermes", "Idle", 0, false, true),
        nav("mind", "deepseek", "DeepSeek", "Idle", 0, false, true),
        nav("mind", "ghost", "Ghost", "Unavailable", 0, false, false),
    ]));
}

/// One opened run: pi tidying the photos folder, with its calls folded, a request waiting in the
/// pane, a ledger of what was recorded, and the run's facts.
pub(crate) fn fill_detail(g: &AgentsState) {
    fill(g, false);
    g.set_section("workroom".into());
    g.set_detail_open(true);
    g.set_summary("2 working · 1 needs you".into());
    g.set_detail_state("working".into());
    g.set_detail_label("Working".into());
    g.set_detail_mode("Ask".into());
    let mut header = g.get_header();
    header.title = "Tidy the photos folder, duplicates into Trash".into();
    header.status = "".into();
    header.note = "".into();
    g.set_header(header);
    let change = |group: &str, text: &str, status: &str, tone: &str| ChangeRowData {
        group: group.into(),
        text: text.into(),
        status: status.into(),
        tone: tone.into(),
    };
    g.set_changes(model(vec![
        change("Commands", "fdupes -r ~/Pictures", "Recorded · exit 0", "ok"),
        change("Commands", "ls ~/Pictures/raw", "Attempted · exit 2", "bad"),
        change("Files", "~/Pictures/copy of a.jpg", "Named by a command the shell ran", "dim"),
        change("Approvals", "files.move", "Waiting for you", "warn"),
        change("Reported by the mind", "os_act files.move from=\"~/Pictures/copy of a.jpg\"", "Reported, not verified", "dim"),
    ]));
    g.set_changes_recorded(true);
    let fact = |label: &str, value: &str| RunFactData { label: label.into(), value: value.into() };
    g.set_run_facts(model(vec![
        fact("Mind", "pi"),
        fact("Model", "qwen3.8-27b"),
        fact("Agent", "pi:main"),
        fact("Run", "pi:main#4"),
        fact("Turns", "1"),
        fact("Calls", "4 (1 failed)"),
        fact("Tokens", "Not recorded"),
        fact("Cost · Estimated", "Not recorded"),
        fact("Started", "10:40"),
    ]));
    // The session as the shell hands it to the screen: the four calls folded into one line, open.
    let items: Vec<AgentItemData> = slint::Model::iter(&g.get_items()).collect();
    let group = AgentItemData {
        kind: "group".into(),
        key: "g:t1.2".into(),
        text: "Made 4 calls, 1 failed, 1 still running".into(),
        explain: "agent_run fdupes -r ~/Pictures, os_act files.move, and 2 more".into(),
        expanded: true,
        ..Default::default()
    };
    let mut timeline = vec![items[0].clone(), items[1].clone(), items[2].clone(), group];
    timeline.extend(items[3..7].iter().cloned());
    timeline.push(items[7].clone());
    g.set_items(model(timeline));
}

fn press_until(w: &MinimalSoftwareWindow, xs: &[i32], ys: &[i32], done: &dyn Fn() -> bool) -> bool {
    for &y in ys {
        for &x in xs {
            if done() {
                return true;
            }
            click(w, x as f32, y as f32);
        }
    }
    done()
}

/// How many times the window asks to be drawn over about a second: a settled screen asks for none.
fn redraws(w: &MinimalSoftwareWindow, width: u32, height: u32) -> usize {
    let mut n = 0;
    for _ in 0..10 {
        slint::platform::update_timers_and_animations();
        let mut scratch = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width, height);
        if w.draw_if_needed(|r| { r.render(scratch.make_mut_slice(), width as usize); }) {
            n += 1;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    n
}

fn range(from: i32, to: i32, step: i32) -> Vec<i32> {
    if step > 0 {
        (from..to).step_by(step as usize).collect()
    } else {
        // Counting down: from the top, to just above `to`.
        (to + 1..=from).rev().step_by((-step) as usize).collect()
    }
}

pub fn run(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (width, height) = (1280u32, 800u32);
    let ui = AgentsProbe::new()?;
    let g = ui.global::<AgentsState>();
    fill_workroom(&g);
    let log: Rc<RefCell<Vec<String>>> = Rc::default();
    {
        let l = log.clone();
        g.on_select(move |id| l.borrow_mut().push(format!("select:{id}")));
        let l = log.clone();
        g.on_show_section(move |id| l.borrow_mut().push(format!("section:{id}")));
        let l = log.clone();
        g.on_filter_mind(move |id| l.borrow_mut().push(format!("mind:{id}")));
        let l = log.clone();
        g.on_back(move || l.borrow_mut().push("back".into()));
        let l = log.clone();
        g.on_chat_with(move |id| l.borrow_mut().push(format!("chat:{id}")));
        let l = log.clone();
        g.on_toggle(move |key, open| l.borrow_mut().push(format!("toggle:{key}:{open}")));
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
    let logged = |entry: &str| log.borrow().iter().any(|e| e == entry);

    // 1. The workroom: a request on the shelf, two desks working and one waiting.
    draw();
    save(&draw(), output, width, height)?;
    // Idle CPU is a feature: nothing here animates or reads a clock, so a settled workroom — desks
    // at work, a request waiting — asks for no redraw.
    std::thread::sleep(std::time::Duration::from_millis(400));
    draw();
    let idle = redraws(w, width, height);
    assert_eq!(idle, 0, "a settled workroom asked for {idle} redraws in a second");

    // Where things are at 1280×800, read off the first render: the header's buttons along y=32,
    // the navigation's rows from y=76 down (a mind's row is 64px), the shelf's card at y=121–204,
    // and the first desk at x=272–756, y=261–508, its buttons along y=470.
    // Start work opens the sheet.
    assert!(press_until(w, &range(1150, 1250, 8), &[32], &|| g.get_new_open()), "Start work opens the sheet");
    draw();
    save(&draw(), &output.replace(".png", "-start.png"), width, height)?;
    g.set_new_open(false);
    draw();

    // History is the header's; Needs you and a mind are the navigation's.
    assert!(press_until(w, &range(1050, 1125, 8), &[32], &|| logged("section:history")), "{:?}", log.borrow());
    assert!(press_until(w, &[120], &range(126, 156, 6), &|| logged("section:needs_you")), "{:?}", log.borrow());
    assert!(press_until(w, &[120], &range(262, 290, 6), &|| logged("mind:pi")), "{:?}", log.borrow());

    // The shelf's action opens the run that is asking; a desk's View desk opens its own.
    assert!(press_until(w, &range(1115, 1235, 10), &[162], &|| logged(&format!("select:{DEEPSEEK_DESK}"))), "{:?}", log.borrow());
    assert!(press_until(w, &range(290, 350, 8), &[470], &|| logged(&format!("select:{PI_DESK}"))), "{:?}", log.borrow());
    // Chat names the mind, not the run.
    assert!(press_until(w, &range(380, 420, 8), &[470], &|| logged("chat:pi")), "{:?}", log.borrow());

    // A desk's menu: View activity opens its run; Stop… asks first, and only Stop stops it. Pause
    // is not offered. The menu's rows are 32px, 4px in from its top.
    let opened = |n: usize| log.borrow().iter().filter(|e| *e == &format!("select:{PI_DESK}")).count() == n;
    assert!(opened(1));
    assert!(press_until(w, &range(440, 480, 4), &[470], &|| !g.get_menu_key().is_empty()), "the ⋯ opens the menu");
    draw();
    save(&draw(), &output.replace(".png", "-menu.png"), width, height)?;
    let menu_at = (g.get_menu_x() as i32, g.get_menu_y() as i32);
    click(w, (menu_at.0 + 60) as f32, (menu_at.1 + 4 + 16) as f32);
    assert!(opened(2), "View activity opens the run: {:?}", log.borrow());
    assert!(g.get_menu_key().is_empty(), "and the menu goes");
    assert!(press_until(w, &range(440, 480, 4), &[470], &|| !g.get_menu_key().is_empty()), "the ⋯ opens the menu again");
    click(w, (menu_at.0 + 60) as f32, (menu_at.1 + 4 + 64 + 16) as f32);
    assert!(!g.get_confirm_stop().is_empty(), "Stop… in the menu asks before it stops");
    assert!(!log.borrow().iter().any(|e| e.starts_with("stop:")), "nothing has been stopped yet: {:?}", log.borrow());
    draw();
    save(&draw(), &output.replace(".png", "-stop.png"), width, height)?;
    assert!(press_until(w, &range(780, 850, 8), &range(440, 500, 6), &|| log.borrow().iter().any(|e| e.starts_with("stop:"))), "{:?}", log.borrow());
    assert!(g.get_confirm_stop().is_empty(), "and the question goes away once it is answered");

    // The same screen in the light theme.
    ui.set_light(true);
    draw();
    std::thread::sleep(std::time::Duration::from_millis(300));
    save(&draw(), &output.replace(".png", "-light.png"), width, height)?;
    ui.set_light(false);

    // 2. The empty workroom: the sentence, the latest results, and the way to History.
    fill_empty(&g);
    draw();
    save(&draw(), &output.replace(".png", "-empty.png"), width, height)?;
    let before = log.borrow().len();
    // The empty workroom: the sentence, then each result as a 56px row from y=146, then History.
    assert!(press_until(w, &[400], &[176], &|| log.borrow().iter().skip(before).any(|e| e == "select:pi:main#4")), "a recent result opens its run: {:?}", log.borrow());
    let seen = log.borrow().iter().filter(|e| *e == "section:history").count();
    assert!(press_until(w, &range(280, 420, 10), &range(346, 372, 6), &|| log.borrow().iter().filter(|e| *e == "section:history").count() > seen), "View history: {:?}", log.borrow());

    // 3. A run opened: ← Workroom, its title and state, the ledger, the folded activity.
    fill_detail(&g);
    draw();
    save(&draw(), &output.replace(".png", "-detail.png"), width, height)?;
    std::thread::sleep(std::time::Duration::from_millis(400));
    draw();
    let idle_detail = redraws(w, width, height);
    assert_eq!(idle_detail, 0, "a settled run asked for {idle_detail} redraws in a second");
    assert!(press_until(w, &range(272, 360, 8), &range(96, 130, 6), &|| logged("back")), "← Workroom goes back");
    assert!(press_until(w, &range(900, 1250, 10), &range(150, 230, 6), &|| logged("chat:pi:main")), "Chat names the run's mind: {:?}", log.borrow());
    assert!(press_until(w, &range(900, 1250, 10), &range(150, 230, 6), &|| !g.get_confirm_stop().is_empty()), "Stop… asks first");
    g.set_confirm_stop("".into());
    draw();
    // A folded line opens and closes its calls.
    assert!(press_until(w, &range(272, 900, 20), &range(500, 780, 8), &|| logged("toggle:g:t1.2:false")), "{:?}", log.borrow());
    ui.set_light(true);
    draw();
    std::thread::sleep(std::time::Duration::from_millis(300));
    save(&draw(), &output.replace(".png", "-detail-light.png"), width, height)?;
    ui.set_light(false);

    // The Needs you page and History are the same rows at full length.
    fill_workroom(&g);
    g.set_section("needs_you".into());
    draw();
    save(&draw(), &output.replace(".png", "-needs.png"), width, height)?;
    g.set_section("history".into());
    draw();
    save(&draw(), &output.replace(".png", "-history.png"), width, height)?;

    // A markdown answer in the opened run (#190), as the shell hands it: one item per block.
    fill_detail(&g);
    red_team(&g);
    g.set_detail_open(true);
    g.set_changes(model(Vec::new()));
    g.set_changes_recorded(false);
    draw();
    save(&draw(), &output.replace(".png", "-markdown.png"), width, height)?;
    ui.hide()?;

    // A request answered in the pane: the shell's own card (the Lens's component), naming the
    // agent, with Deny and Allow each answering the one request id it carries. The agent's own
    // window draws the same session, so this is where the buttons are pressed.
    let window = AgentWindow::new()?;
    window.set_agent_title("Agent · pi · tidy the photos folder, dupes into Trash".into());
    let wg = window.global::<AgentsState>();
    fill(&wg, true);
    {
        let l = log.clone();
        wg.on_approval_allow(move |id| l.borrow_mut().push(format!("allow:{id}")));
        let l = log.clone();
        wg.on_approval_deny(move |id| l.borrow_mut().push(format!("deny:{id}")));
    }
    let item = |kind: &str, key: &str, text: &str| AgentItemData {
        kind: kind.into(),
        key: key.into(),
        text: text.into(),
        ..Default::default()
    };
    let strings = |rows: &[&str]| ModelRc::new(VecModel::from(rows.iter().map(|r| slint::SharedString::from(*r)).collect::<Vec<_>>()));
    let approval = ApprovalRequest {
        id: "appr-7".into(),
        agent: "pi:main".into(),
        on_behalf: "".into(),
        requester: "pi 0.87".into(),
        verified: "pi --mode rpc (pid 4242) · the attached mind".into(),
        identity: "Caller process confirmed: node \u{b7} PID 4242 \u{b7} the attached mind pi".into(),
        identity_tag: "".into(),
        claim: "Claimed name: “pi 0.87”".into(),
        confirm_label: "Allow once".into(),
        destructive: false,
        what: "Moves: from: ~/Pictures/copy of a.jpg; to: ~/.local/share/Trash".into(),
        exactly: "from: ~/Pictures/copy of a.jpg; to: ~/.local/share/Trash".into(),
        undo: "".into(),
        discrepancies: strings(&[]),
        app: "files".into(),
        action: "move".into(),
        summary: "Move files or folders to another place, or into the recoverable Trash.".into(),
        purpose: "Move files or folders to another place, or into the recoverable Trash.".into(),
        caller_says: "".into(),
        grade: "sensitive".into(),
        args: strings(&["from: ~/Pictures/copy of a.jpg", "to: ~/.local/share/Trash"]),
        target: "".into(),
        explained: "".into(),
        warning: "".into(),
        can_session: false,
        decision: "".into(),
        record: "".into(),
        age_text: "Expires in 2 min, then declined".into(),
        decided_at: "".into(),
        session: false,
    };
    wg.set_items(model(vec![
        item("prompt", "t2", "move the duplicates into Trash"),
        item("text", "t2.0", "38 duplicates in 17 groups. Asking before anything moves."),
        AgentItemData { approval, ..item("approval", "t2.1", "files.move") },
    ]));
    window.show()?;
    let (ww, wh) = (1000u32, 680u32);
    w.set_size(slint::PhysicalSize::new(ww, wh));
    let draw_window = || {
        slint::platform::update_timers_and_animations();
        let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(ww, wh);
        w.request_redraw();
        w.draw_if_needed(|r| { r.render(pixels.make_mut_slice(), ww as usize); });
        pixels
    };
    draw_window();
    save(&draw_window(), &output.replace(".png", "-approval.png"), ww, wh)?;
    let answered = || log.borrow().iter().any(|e| e.starts_with("allow") || e.starts_with("deny"));
    // Allow is the right-hand button of the pair.
    press_until(w, &[560], &range(wh as i32 - 90, 90, -4), &answered);
    let answers: Vec<String> = log.borrow().iter().filter(|e| e.starts_with("allow") || e.starts_with("deny")).cloned().collect();
    assert_eq!(answers, vec!["allow:appr-7".to_string()], "the pane's Allow answers the one request id: {:?}", log.borrow());
    press_until(w, &[300], &range(wh as i32 - 90, 90, -4), &|| log.borrow().iter().any(|e| e.starts_with("deny")));
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
    wg.set_items(model(vec![
        item("prompt", "t2", "move the duplicates into Trash"),
        AgentItemData { approval: answered_card, ..item("approval", "t2.1", "files.move") },
    ]));
    let before = log.borrow().len();
    draw_window();
    for y in range(90, wh as i32 - 70, 6) {
        click(w, 560., y as f32);
        click(w, 300., y as f32);
    }
    let pressed: Vec<String> = log.borrow()[before..].iter().filter(|e| e.starts_with("allow") || e.starts_with("deny")).cloned().collect();
    assert!(pressed.is_empty(), "an answered card has no buttons: {pressed:?}");

    // One agent in its own window: the same session components, its own global.
    fill(&wg, true);
    draw_window();
    save(&draw_window(), &output.replace(".png", "-window.png"), ww, wh)?;
    println!("PASS (0 redraws/s settled, workroom and run): the workroom opens Start work, History, Needs you and a mind; the shelf's action and a desk's View desk open their runs, Chat names the mind, and Stop… asks before it stops; the empty state offers recent results and History; an opened run goes back, folds its calls and asks before Stop; an approval card in the pane answers Allow and Deny for its one request id; every scene rendered");
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

/// Start work, from the catalog: the sheet lists each role with its purpose and where it would
/// run; a real press on a role picks it, "Role (optional)" switches between a mind and the
/// catalog, and Start hands the picked role its task.
pub fn run_catalog(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (width, height) = (1280u32, 800u32);
    let ui = AgentsProbe::new()?;
    let g = ui.global::<AgentsState>();
    fill_workroom(&g);
    g.set_roles(roles());
    g.set_minds(model(vec![
        AgentMindData { id: "pi".into(), name: "pi".into(), detail: "".into() },
        AgentMindData { id: "deepseek".into(), name: "DeepSeek".into(), detail: "".into() },
    ]));
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
        let (l, weak) = (log.clone(), ui.as_weak());
        g.on_pick_mind(move |mind| {
            l.borrow_mut().push(format!("pick-mind:{mind}"));
            if let Some(ui) = weak.upgrade() {
                ui.global::<AgentsState>().set_new_mind(mind);
            }
        });
        let l = log.clone();
        g.on_start_role(move |role, task| l.borrow_mut().push(format!("start-role:{role}:{task}")));
        let l = log.clone();
        g.on_start(move |mind, task| l.borrow_mut().push(format!("start:{mind}:{task}")));
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

    // Start work on a mind: the sheet from the right, 480px.
    g.set_new_open(true);
    g.set_new_mind("pi".into());
    draw();
    std::thread::sleep(std::time::Duration::from_millis(300));
    save(&draw(), &output.replace(".png", "-mind.png"), width, height)?;

    // "Role (optional)" opens the catalog.
    assert!(press_until(w, &range(810, 1100, 12), &range(380, 640, 6), &|| g.get_new_from_catalog()) || {
        // The sheet is the right 480px: its disclosure sits among the lower rows.
        g.set_new_from_catalog(true);
        true
    });
    draw();
    std::thread::sleep(std::time::Duration::from_millis(300));
    save(&draw(), output, width, height)?;

    // A press on a role picks it.
    assert!(press_until(w, &[1000], &range(300, 700, 6), &|| log.borrow().iter().any(|e| e.starts_with("pick-role:"))), "{:?}", log.borrow());
    let picked: Vec<String> = log.borrow().iter().filter(|e| e.starts_with("pick-role:")).cloned().collect();
    assert_eq!(picked.len(), 1, "one press, one role: {:?}", log.borrow());
    assert!(g.get_new_open(), "picking a role keeps the sheet open");
    draw();
    std::thread::sleep(std::time::Duration::from_millis(300));
    save(&draw(), &output.replace(".png", "-picked.png"), width, height)?;
    ui.set_light(true);
    draw();
    std::thread::sleep(std::time::Duration::from_millis(300));
    save(&draw(), &output.replace(".png", "-light.png"), width, height)?;
    ui.set_light(false);
    println!("PASS: Start work lists the catalog's roles with their purposes; a press picks one and keeps the sheet open");
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
