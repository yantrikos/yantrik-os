//! The Agents Overview (#226): the production AgentsScreen in Overview, filled with fixture data
//! and laid out by the shell's own layout — crates/yantrik-ui/src/agents_overview.rs, included
//! below — for the size the map reports, as the shell does. Real pointer events: an agent pressed
//! on the map opens in the list; the map comes back laid out when Overview is chosen again; what
//! waits on the person is drawn amber; and a map in which nothing changes asks for no redraw.
use super::*;
use slint::{Model, ModelRc, VecModel};
use std::cell::RefCell;

#[path = "../../../crates/yantrik-ui/src/agents_overview.rs"]
#[allow(dead_code)]
mod layout;

fn mind(id: &str, name: &str, detail: &str) -> layout::Mind {
    layout::Mind { id: id.into(), name: name.into(), detail: detail.into() }
}

#[allow(clippy::too_many_arguments)]
fn agent(id: &str, mind: &str, state: &str, label: &str, since: &str, parent: &str, role: &str, title: &str) -> layout::Agent {
    layout::Agent {
        id: id.into(),
        mind: mind.into(),
        title: title.into(),
        state: state.into(),
        label: label.into(),
        since: since.into(),
        parent: parent.into(),
        role: role.into(),
        origin: String::new(),
    }
}

/// Three minds; a Council a recipe started on Hermes, its Chair with three roles under it, the
/// Planner waiting on the person; pi waiting on a card of its own; a finished Hermes agent; a
/// failed OpenClaw one; and a pi agent whose parent has gone.
fn fixture() -> (Vec<layout::Mind>, Vec<layout::Agent>) {
    let minds = vec![
        mind("hermes", "Hermes", "deepseek-v4.1-flash"),
        mind("openclaw", "OpenClaw", "ollama-cloud/kimi-k3"),
        mind("pi", "pi", "qwen3.8-max"),
    ];
    let mut chair = agent("hermes:council", "hermes", "thinking", "thinking", "3m", "", "Chair", "Publish tonight's nightly?");
    chair.origin = "Council recipe".into();
    let agents = vec![
        chair,
        agent("hermes:council-1", "hermes", "running_tool", "running a tool", "2m", "hermes:council", "Researcher", "What changed since the last nightly"),
        agent("hermes:council-2", "hermes", "thinking", "thinking", "2m", "hermes:council", "Red team", "Attack the publish plan"),
        agent("hermes:council-3", "hermes", "waiting_for_you", "waiting for you", "1m", "hermes:council", "Planner", "Order the release steps"),
        agent("hermes:photos", "hermes", "done", "done", "21:02", "", "", "Tidy the photos folder"),
        agent("pi:main", "pi", "waiting_for_you", "waiting for you", "4m", "", "", "Delete the old calendar events"),
        agent("pi:inbox", "pi", "running_tool", "running a tool", "1m", "pi:gone", "", "Summarise the inbox"),
        agent("openclaw:web", "openclaw", "failed", "failed", "20:51", "", "", "Read the top three stories"),
    ];
    (minds, agents)
}

/// What the shell's `overview` does with a layout: the same fields, into the screen's global.
fn publish(g: &AgentsState, map: &layout::Map) {
    let nodes: Vec<OverviewNode> = map
        .nodes
        .iter()
        .map(|n| OverviewNode {
            id: n.id.as_str().into(),
            kind: n.kind.into(),
            x: n.x,
            y: n.y,
            size: n.size,
            state: n.state.as_str().into(),
            title: n.title.as_str().into(),
            sub: n.sub.as_str().into(),
            label_x: n.label.x,
            label_y: n.label.y,
            label_w: n.label.w,
            label_h: n.label.h,
        })
        .collect();
    let edges: Vec<OverviewEdge> = map
        .edges
        .iter()
        .map(|e| OverviewEdge { d: e.d.as_str().into(), state: e.state.as_str().into(), hot: e.hot, leader: e.leader })
        .collect();
    g.set_overview_nodes(ModelRc::new(VecModel::from(nodes)));
    g.set_overview_edges(ModelRc::new(VecModel::from(edges)));
    g.set_overview_summary(map.summary.as_str().into());
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
    let ui = AgentsProbe::new()?;
    let g = ui.global::<AgentsState>();
    let (minds, agents) = fixture();

    g.set_tabs(ModelRc::new(VecModel::from(vec![
        AgentTabData { id: "active".into(), label: "Active".into(), count: 4 },
        AgentTabData { id: "needs_you".into(), label: "Needs you".into(), count: 2 },
        AgentTabData { id: "complete".into(), label: "Complete".into(), count: 2 },
        AgentTabData { id: "all".into(), label: "All".into(), count: 8 },
    ])));
    g.set_rows(ModelRc::new(VecModel::from(
        agents
            .iter()
            .map(|a| AgentRowData {
                id: a.id.as_str().into(),
                mind: a.mind.as_str().into(),
                title: a.title.as_str().into(),
                state: a.state.as_str().into(),
                label: a.label.as_str().into(),
                since: a.since.as_str().into(),
                parent: a.parent.as_str().into(),
                role: a.role.as_str().into(),
                origin: a.origin.as_str().into(),
            })
            .collect::<Vec<_>>(),
    )));

    // What the shell answers the map with. Recorded, so the test knows the size it was laid out
    // for and how often it was asked.
    let asked: Rc<RefCell<Vec<(f32, f32)>>> = Rc::default();
    let pressed: Rc<RefCell<Vec<String>>> = Rc::default();
    {
        let (weak, asked, minds, agents) = (ui.as_weak(), asked.clone(), minds.clone(), agents.clone());
        g.on_overview_resized(move |width, height| {
            asked.borrow_mut().push((width, height));
            let ui = weak.unwrap();
            publish(&ui.global::<AgentsState>(), &layout::layout("vm-520", &minds, &agents, width, height));
        });
    }
    {
        let (weak, pressed) = (ui.as_weak(), pressed.clone());
        g.on_select_tab(move |tab| {
            pressed.borrow_mut().push(format!("tab:{tab}"));
            weak.unwrap().global::<AgentsState>().set_tab(tab);
        });
    }
    {
        let (weak, pressed) = (ui.as_weak(), pressed.clone());
        g.on_select(move |id| {
            pressed.borrow_mut().push(format!("select:{id}"));
            weak.unwrap().global::<AgentsState>().set_selected(id);
        });
    }
    {
        let pressed = pressed.clone();
        g.on_pop_out(move |id| pressed.borrow_mut().push(format!("pop:{id}")));
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
    std::thread::sleep(std::time::Duration::from_millis(350));
    let pixels = draw();
    save(&pixels, output, width, height)?;

    let (map_w, map_h) = *asked.borrow().last().expect("the map reported its size");
    assert!(map_w > 900.0 && map_h > 500.0, "the map has most of the screen: {map_w}×{map_h}");
    assert_eq!(g.get_overview_nodes().row_count(), 1 + minds.len() + agents.len(), "the machine, every mind and every agent");
    assert_eq!(
        g.get_overview_summary(),
        "vm-520 · 3 minds attached · 4 working · 2 waiting on you · 1 finished · 1 failed"
    );

    // Where the map sits on screen. The screen and the overview pad it the same on every side,
    // so its inset is what the width leaves over, split in two, and the same at the bottom.
    let inset = (width as f32 - map_w) / 2.0;
    let (ox, oy) = (inset, height as f32 - inset - map_h);
    let map = layout::layout("vm-520", &minds, &agents, map_w, map_h);
    let node = |id: &str| map.nodes.iter().find(|n| n.id == id).unwrap_or_else(|| panic!("no node {id}")).clone();
    let px = |x: f32, y: f32| {
        let p = pixels.as_slice()[(y as usize) * width as usize + x as usize];
        (p.r as i32, p.g as i32, p.b as i32)
    };
    // A node's ring, sampled just inside its right edge.
    let ring = |id: &str| {
        let n = node(id);
        px(ox + n.x + n.size / 2.0 - 1.5, oy + n.y)
    };
    // Amber is warm with more green than blue (#d4a574); the failed state's red has as much blue
    // as green, and the success green has more green than red.
    let amber = |(r, g, b): (i32, i32, i32)| r > 150 && r > g && g > b + 25;
    for id in ["pi:main", "hermes:council-3"] {
        assert!(amber(ring(id)), "{id} waits on the person and is drawn amber: {:?}", ring(id));
    }
    for id in ["hermes:council-1", "openclaw:web", "hermes:photos"] {
        assert!(!amber(ring(id)), "{id} does not wait on anyone and is not amber: {:?}", ring(id));
    }

    // Pressing an agent on the map opens it in the list with its session, whatever tab it is on.
    let red_team = node("hermes:council-2");
    click(w, ox + red_team.x, oy + red_team.y);
    draw();
    assert_eq!(
        *pressed.borrow(),
        vec!["tab:all".to_string(), "select:hermes:council-2".to_string()],
        "the Red team's node opens it under All"
    );
    assert_eq!(g.get_view(), "list", "and the screen shows the list again");

    // Chosen again, the map asks to be laid out again, and is.
    let before = asked.borrow().len();
    g.set_view("overview".into());
    draw();
    assert!(asked.borrow().len() > before, "the map reported its size when it came back");
    assert_eq!(g.get_overview_nodes().row_count(), 1 + minds.len() + agents.len());

    // A map in which nothing changes costs nothing to leave on screen: no timer, no redraw (#68).
    std::thread::sleep(std::time::Duration::from_millis(350));
    draw();
    let mut redraws = 0;
    for _ in 0..10 {
        slint::platform::update_timers_and_animations();
        let mut scratch = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width, height);
        if w.draw_if_needed(|r| { r.render(scratch.make_mut_slice(), width as usize); }) {
            redraws += 1;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert_eq!(redraws, 0, "an idle map asked for {redraws} redraws in a second");

    // The same map in the light theme, and at the smaller screen, for the eye.
    ui.set_light(true);
    save(&draw(), &output.replace(".png", "-light.png"), width, height)?;
    ui.set_light(false);
    let (cw, ch) = (800u32, 600u32);
    ui.set_canvas_width(cw as f32);
    ui.set_canvas_height(ch as f32);
    w.set_size(slint::PhysicalSize::new(cw, ch));
    let mut small = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(cw, ch);
    for _ in 0..3 {
        slint::platform::update_timers_and_animations();
        w.request_redraw();
        w.draw_if_needed(|r| { r.render(small.make_mut_slice(), cw as usize); });
    }
    let (sw, sh) = *asked.borrow().last().unwrap();
    assert!(sw < 800.0 && sh < 600.0, "the map was laid out again for the smaller screen: {sw}×{sh}");
    save(&small, &output.replace(".png", "-compact.png"), cw, ch)?;

    println!(
        "PASS: the Overview lays out the machine, 3 minds and 8 agents for {map_w}×{map_h}; waiting agents are amber and no other is; pressing an agent opens it in the list under All; the map is laid out again when chosen again and at 800×600; an idle map asks for no redraw"
    );
    Ok(())
}
