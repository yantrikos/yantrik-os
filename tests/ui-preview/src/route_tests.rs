//! The Agents Overview as a route (Pranab, 27 September): the production AgentsScreen in
//! Overview, showing one run as a line of stations. Real pointer events: a station opens its step
//! in the session, a branch opens the agent it started. A live route moves, and a finished one
//! asks for no redraw (#68).
use super::*;
use slint::{Model, ModelRc, VecModel};
use std::cell::RefCell;

fn stop(key: &str, kind: &str, state: &str, title: &str, sub: &str, track_in: &str, track_out: &str) -> RouteStopData {
    RouteStopData {
        key: key.into(),
        kind: kind.into(),
        state: state.into(),
        title: title.into(),
        sub: sub.into(),
        track_in: track_in.into(),
        track_out: track_out.into(),
        train_in: -1.0,
        train_out: -1.0,
        train_span: 0.0,
    }
}

/// One train over the live half-rows, as the shell lays it (agents/route.rs `lay_track`).
fn with_train(mut stops: Vec<RouteStopData>) -> Vec<RouteStopData> {
    let halves = stops.iter().map(|s| (s.track_in == "live") as usize + (s.track_out == "live") as usize).sum::<usize>();
    let span = if halves == 0 { 0.0 } else { 1.0 / halves as f32 };
    let mut at = 0.0;
    for s in stops.iter_mut() {
        s.train_span = span;
        if s.track_in == "live" {
            s.train_in = at;
            at += span;
        }
        if s.track_out == "live" {
            s.train_out = at;
            at += span;
        }
    }
    stops
}

/// A chat's request to review the release notes, live: three calls passed, a question answered,
/// a Reviewer it started still at work on a branch, and a call running now, with the reply ahead.
fn live_run() -> Vec<RouteStopData> {
    with_train(vec![
        stop("t4", "start", "passed", "Review the release notes before tonight's nightly", "Asked · 2m ago", "", "done"),
        stop("t4.0", "call", "passed", "Read NOTES.md", "1s", "done", "done"),
        stop("t4.2", "call", "passed", "git log 0f3e733..2c687eb --oneline", "2s", "done", "done"),
        stop("t4.3", "question", "passed", "Include the Writers' room rename?", "You answered: Yes", "done", "live"),
        stop("hermes:c-review", "branch", "here", "Reviewer: Check the notes against the log", "thinking", "live", "live"),
        stop("t4.5", "call", "here", "grep -n \"#37[0-9]\" NOTES.md", "running · 12s", "live", "ahead"),
        stop("", "end", "ahead", "Reply", "", "ahead", ""),
    ])
}

/// The same run once it finished.
fn finished_run() -> Vec<RouteStopData> {
    vec![
        stop("t4", "start", "passed", "Review the release notes before tonight's nightly", "Asked · 9m ago", "", "done"),
        stop("t4.0", "call", "passed", "Read NOTES.md", "1s", "done", "done"),
        stop("t4.2", "call", "passed", "git log 0f3e733..2c687eb --oneline", "2s", "done", "done"),
        stop("t4.3", "question", "passed", "Include the Writers' room rename?", "You answered: Yes", "done", "done"),
        stop("hermes:c-review", "branch", "passed", "Reviewer: Check the notes against the log", "done", "done", "done"),
        stop("t4.5", "call", "failed", "grep -n \"#37[0-9]\" NOTES.md", "0s · exit 1", "done", "done"),
        stop("", "end", "passed", "Done", "after 3m 4s", "done", ""),
    ]
}

fn save(pixels: &slint::SharedPixelBuffer<slint::Rgb8Pixel>, path: &str, width: u32, height: u32) -> Result<(), Box<dyn std::error::Error>> {
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(path)?), width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(pixels.as_bytes())?;
    println!("Rendered {path} ({width}×{height})");
    Ok(())
}

/// How many times the window asks to be drawn over about a second.
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

pub fn run(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let ui = AgentsProbe::new()?;
    let g = ui.global::<AgentsState>();
    g.set_tabs(ModelRc::new(VecModel::from(vec![
        AgentTabData { id: "active".into(), label: "Active".into(), count: 1 },
        AgentTabData { id: "needs_you".into(), label: "Needs you".into(), count: 0 },
        AgentTabData { id: "complete".into(), label: "Complete".into(), count: 3 },
        AgentTabData { id: "all".into(), label: "All".into(), count: 4 },
    ])));
    let row = |id: &str, title: &str, state: &str, label: &str, since: &str, origin: &str| AgentRowData {
        id: id.into(),
        mind: "hermes".into(),
        title: title.into(),
        state: state.into(),
        label: label.into(),
        since: since.into(),
        origin: origin.into(),
        ..Default::default()
    };
    g.set_rows(ModelRc::new(VecModel::from(vec![
        row("hermes:main#4", "Review the release notes before tonight's nightly", "running_tool", "running a tool", "2m", "Chat"),
        row("hermes:main#2", "Tidy the photos folder", "done", "done", "20:41", "Chat"),
        row("pi:c-web", "Read the top three stories", "failed", "failed", "20:12", ""),
    ])));
    g.set_selected("hermes:main#4".into());
    g.set_route_title("Review the release notes before tonight's nightly".into());
    g.set_route_summary("Running · 2m 4s · 4 passed".into());
    g.set_route_stops(ModelRc::new(VecModel::from(live_run())));
    g.set_route_live(true);

    let pressed: Rc<RefCell<Vec<String>>> = Rc::default();
    {
        let (weak, pressed) = (ui.as_weak(), pressed.clone());
        g.on_select_tab(move |tab| {
            pressed.borrow_mut().push(format!("tab:{tab}"));
            weak.unwrap().global::<AgentsState>().set_tab(tab);
        });
    }
    {
        let pressed = pressed.clone();
        g.on_select(move |id| pressed.borrow_mut().push(format!("select:{id}")));
    }
    {
        let pressed = pressed.clone();
        g.on_toggle(move |key, open| pressed.borrow_mut().push(format!("toggle:{key}:{open}")));
    }
    g.set_view("overview".into());

    let (width, height) = (1280u32, 800u32);
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(width, height));
    let draw = || {
        let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width, height);
        for _ in 0..3 {
            slint::platform::update_timers_and_animations();
            w.request_redraw();
            w.draw_if_needed(|r| { r.render(pixels.make_mut_slice(), width as usize); });
        }
        pixels
    };
    draw();
    std::thread::sleep(std::time::Duration::from_millis(400));
    save(&draw(), output, width, height)?;
    std::thread::sleep(std::time::Duration::from_millis(500));
    save(&draw(), &output.replace(".png", "-later.png"), width, height)?;

    // A live route moves: the train and the pulse ask for frames on their own.
    let live = redraws(w, width, height);
    assert!(live >= 5, "a live route asked for only {live} redraws in a second");

    // Where the stations are at 1280×800: the first one's middle 153px down, one every 56px, and
    // their words from 386px across (the list is 270px, then the route's padding and line).
    let station = |i: usize| (440.0, 153.0 + 56.0 * i as f32);

    // A step opens in the session, unfolded: the list view, that card toggled open.
    let (x, y) = station(1);
    click(w, x, y);
    draw();
    assert_eq!(g.get_view(), "list", "a station opens the session");
    assert_eq!(*pressed.borrow(), vec!["toggle:t4.0:true".to_string()], "and unfolds its step");

    // A branch is an agent of its own: its route, found under All.
    g.set_view("overview".into());
    draw();
    pressed.borrow_mut().clear();
    let (x, y) = station(4);
    click(w, x, y);
    draw();
    assert_eq!(*pressed.borrow(), vec!["tab:all".to_string(), "select:hermes:c-review".to_string()]);
    assert_eq!(g.get_view(), "overview", "and stays on the route");

    // The reply ahead is not a place yet: pressing it does nothing.
    pressed.borrow_mut().clear();
    let (x, y) = station(6);
    click(w, x, y);
    draw();
    assert!(pressed.borrow().is_empty(), "the reply ahead opened {:?}", pressed.borrow());

    // Finished, the same route stands still: nothing on it reads the clock (#68).
    g.set_route_live(false);
    g.set_route_summary("Done · 3m 4s · 4 calls".into());
    g.set_route_stops(ModelRc::new(VecModel::from(finished_run())));
    draw();
    std::thread::sleep(std::time::Duration::from_millis(400));
    save(&draw(), &output.replace(".png", "-finished.png"), width, height)?;
    let idle = redraws(w, width, height);
    assert_eq!(idle, 0, "a finished route asked for {idle} redraws in a second");

    ui.set_light(true);
    g.set_route_live(true);
    g.set_route_summary("Running · 2m 4s · 4 passed".into());
    g.set_route_stops(ModelRc::new(VecModel::from(live_run())));
    save(&draw(), &output.replace(".png", "-light.png"), width, height)?;
    ui.set_light(false);

    println!("PASS: the Overview draws one run as a route of {} stations; a station opens its step in the session, a branch opens its agent, the reply ahead opens nothing; a live route moves ({live} redraws/s) and a finished one asks for none", g.get_route_stops().row_count());
    Ok(())
}
