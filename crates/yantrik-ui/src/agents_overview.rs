//! The Agents Overview's map (#226): every attached mind and every agent at work, drawn as a
//! radial tree around this machine.
//!
//! The Agents list answers "what is this agent doing". It does not answer "what is at work on
//! this machine, who started what, and what is waiting on me" without reading every row. The
//! map does: the machine in the middle, each mind on the first ring, each agent hung off the
//! mind it runs on — or off the agent that started it, so a formation reads as a tree — and the
//! one state that asks something of the person drawn in amber, with the path to it amber too.
//!
//! Pure: minds and agents in, positions out. Nothing here knows about Slint or the agents store,
//! so the layout is tested without a window, and tests/ui-preview includes this file to lay out
//! its fixtures with the same code the shell runs.

use std::collections::BTreeMap;
use std::f32::consts::{PI, TAU};

/// An attached mind, as the Agents screen knows it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Mind {
    pub id: String,
    pub name: String,
    /// What it says it runs on: "deepseek-v4.1-flash · Hermes 0.9".
    pub detail: String,
}

/// An agent: the fields of the Agents list's row that the map draws.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Agent {
    pub id: String,
    pub mind: String,
    pub title: String,
    /// thinking | running_tool | waiting_for_you | idle | done | failed | harness_gone
    pub state: String,
    pub label: String,
    pub since: String,
    /// The agent that started this one, if any.
    pub parent: String,
    /// The catalog role it was started as, if any.
    pub role: String,
    /// The recipe that handed it the work, if any.
    pub origin: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    fn bottom(&self) -> f32 {
        self.y + self.h
    }

    /// Whether the two share any area. A rectangle with none — the hub's absent label — overlaps
    /// nothing.
    pub fn overlaps(&self, o: &Rect) -> bool {
        self.w > 0.0 && self.h > 0.0 && o.w > 0.0 && o.h > 0.0
            && self.x < o.x + o.w && o.x < self.x + self.w && self.y < o.y + o.h && o.y < self.y + self.h
    }

    fn around(x: f32, y: f32, size: f32) -> Rect {
        Rect { x: x - size / 2.0, y: y - size / 2.0, w: size, h: size }
    }
}

/// One thing on the map.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Node {
    /// The agent's id; `mind:<id>` for a mind, `hub` for the machine, `more:<mind>` for the
    /// finished agents folded into one.
    pub id: String,
    /// hub | mind | agent | more
    pub kind: &'static str,
    /// The centre, in the map's own pixels.
    pub x: f32,
    pub y: f32,
    pub size: f32,
    /// An agent's state; `mind` or `harness_gone` for a mind; `hub`; `done` for a fold.
    pub state: String,
    pub title: String,
    pub sub: String,
    /// Where its label sits: beside the node on the side away from the middle, or above or below
    /// it for a node straight above or below the middle. The hub has none: the machine's name
    /// leads the summary instead of sitting across the spokes.
    pub label: Rect,
}

/// A line on the map: a link from a node to what hangs off it, or a label's leader.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Edge {
    /// SVG path data, in the map's own pixels.
    pub d: String,
    /// The state of the node it leads to.
    pub state: String,
    /// It leads to something waiting on the person, so the way there can be followed from the
    /// middle.
    pub hot: bool,
    pub leader: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Map {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    /// "vm-520 · 3 minds attached · 4 working · 2 waiting on you · 1 finished"
    pub summary: String,
}

pub const LABEL_W: f32 = 200.0;
pub const LABEL_H: f32 = 38.0;
const GAP: f32 = 6.0;
const MARGIN: f32 = 12.0;
const HUB: f32 = 20.0;
const MIND: f32 = 26.0;
const AGENT: f32 = 18.0;
/// Past this many agents the finished ones fold into one node per mind: a map of fifty labels
/// is a list drawn badly.
pub const MAX_AGENTS: usize = 36;

/// Lay out the map for a canvas `width` × `height` pixels.
///
/// The same input always gives the same map, and a change of one agent's state moves nothing:
/// every order here is by id, so a node never jumps under the pointer because something else
/// finished.
pub fn layout(host: &str, minds: &[Mind], agents: &[Agent], width: f32, height: f32) -> Map {
    let summary = match host {
        "" => summary(minds, agents),
        host => format!("{host} · {}", summary(minds, agents)),
    };
    if width < 2.0 * LABEL_W || height < 4.0 * LABEL_H {
        return Map { summary, ..Map::default() };
    }
    let items = tree(host, minds, agents);
    let (cx, cy) = (width / 2.0, height / 2.0);
    // An ellipse, not a circle: the map is wider than it is tall, and the outermost labels need
    // their own width beside the ring.
    let rx = (cx - LABEL_W - GAP - MARGIN - AGENT / 2.0).max(40.0);
    let ry = (cy - LABEL_H / 2.0 - MARGIN).max(40.0);
    let deepest = items.iter().map(|i| i.depth).max().unwrap_or(0);
    let reach = |depth: usize| -> f32 {
        match (depth, deepest) {
            (0, _) => 0.0,
            (_, 1) => 0.55,
            (d, max) => 0.34 + 0.66 * (d - 1) as f32 / (max - 1) as f32,
        }
    };
    let at = |depth: usize, angle: f32| -> (f32, f32) {
        let r = reach(depth);
        (cx + rx * r * angle.cos(), cy + ry * r * angle.sin())
    };

    let mut nodes: Vec<Node> = items
        .iter()
        .map(|i| {
            let (x, y) = at(i.depth, i.angle);
            let size = match i.kind {
                "hub" => HUB,
                "mind" => MIND,
                _ => AGENT,
            };
            Node {
                id: i.id.clone(),
                kind: i.kind,
                x,
                y,
                size,
                state: i.state.clone(),
                title: i.title.clone(),
                sub: i.sub.clone(),
                label: Rect::default(),
            }
        })
        .collect();
    place_labels(&mut nodes, cx, width, height);

    let mut edges = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let Some(p) = item.parent else { continue };
        let (px, py) = (nodes[p].x, nodes[p].y);
        let (x, y) = (nodes[i].x, nodes[i].y);
        // Out from the middle along the parent's ring, then out to the child: a straight line
        // from the hub, a curve everywhere else, so siblings fan out instead of crossing.
        let d = if items[p].depth == 0 {
            format!("M {px:.1} {py:.1} L {x:.1} {y:.1}")
        } else {
            let (qx, qy) = at(items[p].depth, item.angle);
            format!("M {px:.1} {py:.1} Q {qx:.1} {qy:.1} {x:.1} {y:.1}")
        };
        edges.push(Edge { d, state: item.state.clone(), hot: item.hot, leader: false });
    }
    for n in nodes.iter().filter(|n| n.label.w > 0.0) {
        let l = n.label;
        let ((x1, y1), (x2, y2)) = if l.x > n.x {
            ((n.x + n.size / 2.0, n.y), (l.x, l.y + l.h / 2.0))
        } else if l.x + l.w < n.x {
            ((n.x - n.size / 2.0, n.y), (l.x + l.w, l.y + l.h / 2.0))
        } else if l.y > n.y {
            ((n.x, n.y + n.size / 2.0), (n.x, l.y))
        } else {
            ((n.x, n.y - n.size / 2.0), (n.x, l.y + l.h))
        };
        edges.push(Edge {
            d: format!("M {x1:.1} {y1:.1} L {x2:.1} {y2:.1}"),
            state: n.state.clone(),
            hot: false,
            leader: true,
        });
    }
    Map { nodes, edges, summary }
}

/// "3 minds attached · 4 working · 2 waiting on you · 1 finished · 1 failed". Parts that are
/// zero are left out, except the minds: "0 minds attached" is the one a person needs to see.
pub fn summary(minds: &[Mind], agents: &[Agent]) -> String {
    let count = |states: &[&str]| agents.iter().filter(|a| states.contains(&a.state.as_str())).count();
    let plural = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    let mut parts = vec![plural(minds.len(), "mind attached", "minds attached")];
    for (n, text) in [
        (count(&["thinking", "running_tool"]), "working"),
        (count(&["waiting_for_you"]), "waiting on you"),
        (count(&["done"]), "finished"),
        (count(&["failed"]), "failed"),
    ] {
        if n > 0 {
            parts.push(format!("{n} {text}"));
        }
    }
    parts.join(" · ")
}

struct Item {
    id: String,
    kind: &'static str,
    parent: Option<usize>,
    depth: usize,
    state: String,
    title: String,
    sub: String,
    angle: f32,
    hot: bool,
}

/// The machine, its minds and their agents as one tree, with each node's angle worked out: every
/// node gets a wedge of the circle sized by how many leaves hang under it, and sits in the middle
/// of its wedge.
fn tree(host: &str, minds: &[Mind], agents: &[Agent]) -> Vec<Item> {
    let item = |id: String, kind: &'static str, state: &str, title: String, sub: String| Item {
        id,
        kind,
        parent: None,
        depth: 0,
        state: state.to_string(),
        title,
        sub,
        angle: 0.0,
        hot: false,
    };
    let mut items = vec![item(
        "hub".into(),
        "hub",
        "hub",
        "This machine".into(),
        if host.is_empty() { String::new() } else { host.to_string() },
    )];

    // Every mind, attached or not: an agent can outlive the mind it ran on (harness_gone), and
    // it still belongs under that mind's name.
    let mut all_minds: BTreeMap<String, (String, String, bool)> = minds
        .iter()
        .map(|m| (m.id.clone(), (m.name.clone(), m.detail.clone(), true)))
        .collect();
    for a in agents {
        all_minds.entry(a.mind.clone()).or_insert_with(|| (a.mind.clone(), "not attached".into(), false));
    }

    let (kept, folded) = fold(agents);
    let mut mind_at = BTreeMap::new();
    for (id, (name, detail, attached)) in &all_minds {
        let under = kept.iter().filter(|a| &a.mind == id).count() + folded.get(id).copied().unwrap_or(0);
        let sub = match (detail.is_empty(), under) {
            (true, 0) => "no agents".to_string(),
            (true, n) => plural_agents(n),
            (false, 0) => detail.clone(),
            (false, n) => format!("{detail} · {}", plural_agents(n)),
        };
        let state = if *attached { "mind" } else { "harness_gone" };
        let mut m = item(format!("mind:{id}"), "mind", state, name.clone(), sub);
        m.parent = Some(0);
        m.depth = 1;
        mind_at.insert(id.clone(), items.len());
        items.push(m);
    }

    // Agents by id, each under its parent agent when that agent is on the map and the chain back
    // up from it does not come round to itself; under its mind otherwise.
    let by_id: BTreeMap<&str, &Agent> = kept.iter().map(|a| (a.id.as_str(), a)).collect();
    let mut agent_at: BTreeMap<String, usize> = BTreeMap::new();
    let mut pending: Vec<&Agent> = kept.iter().collect();
    // Parents before children: each pass places the agents whose parent is placed already.
    while !pending.is_empty() {
        let before = pending.len();
        pending.retain(|a| {
            let parent = match parent_on_map(a, &by_id) {
                None => mind_at.get(&a.mind).copied(),
                Some(p) => match agent_at.get(p) {
                    Some(&at) => Some(at),
                    None => return true,
                },
            };
            let Some(parent) = parent else { return true };
            let title = if a.role.is_empty() { a.title.clone() } else { format!("{} · {}", a.role, a.title) };
            let mut sub: Vec<&str> = [a.label.as_str(), a.since.as_str()].into_iter().filter(|s| !s.is_empty()).collect();
            // The recipe that started a formation names the root of it, not every role under it.
            if !a.origin.is_empty() && items[parent].kind == "mind" {
                sub.push(a.origin.as_str());
            }
            let mut it = item(a.id.clone(), "agent", &a.state, title, sub.join(" · "));
            it.parent = Some(parent);
            it.depth = items[parent].depth + 1;
            agent_at.insert(a.id.clone(), items.len());
            items.push(it);
            false
        });
        if pending.len() == before {
            break;
        }
    }
    for (mind, n) in &folded {
        if let Some(&parent) = mind_at.get(mind) {
            let mut it = item(format!("more:{mind}"), "more", "done", format!("+{n} done"), "finished, folded to keep the map readable".into());
            it.parent = Some(parent);
            it.depth = 2;
            items.push(it);
        }
    }

    // What waits on the person, and every link on the way to it.
    for i in (1..items.len()).rev() {
        if items[i].state == "waiting_for_you" {
            items[i].hot = true;
        }
        if items[i].hot {
            if let Some(p) = items[i].parent {
                if p != 0 {
                    items[p].hot = true;
                }
            }
        }
    }

    let children = |items: &[Item], of: usize| -> Vec<usize> {
        let mut c: Vec<usize> = (0..items.len()).filter(|&i| items[i].parent == Some(of)).collect();
        c.sort_by(|&a, &b| items[a].id.cmp(&items[b].id));
        c
    };
    let mut leaves = vec![0usize; items.len()];
    for i in (0..items.len()).rev() {
        leaves[i] = children(&items, i).iter().map(|&c| leaves[c]).sum::<usize>().max(1);
    }
    // The first mind starts at the upper left, the way a page is read.
    let mut wedges = vec![(-0.75 * PI, -0.75 * PI + TAU); items.len()];
    for i in 0..items.len() {
        let (a0, a1) = wedges[i];
        items[i].angle = (a0 + a1) / 2.0;
        let kids = children(&items, i);
        let total: usize = kids.iter().map(|&c| leaves[c]).sum();
        let mut start = a0;
        for c in kids {
            let span = (a1 - a0) * leaves[c] as f32 / total.max(1) as f32;
            wedges[c] = (start, start + span);
            start += span;
        }
    }
    items
}

/// The agent `a` hangs off: its parent, when that agent is on the map and the chain back up from
/// it does not come round to `a` again. `None` means it hangs off its mind.
fn parent_on_map<'a>(a: &'a Agent, by_id: &BTreeMap<&str, &Agent>) -> Option<&'a str> {
    let mut seen = vec![a.id.as_str()];
    let mut next = a.parent.as_str();
    while let Some(p) = by_id.get(next) {
        if seen.contains(&next) {
            return None;
        }
        seen.push(next);
        next = p.parent.as_str();
    }
    by_id.contains_key(a.parent.as_str()).then_some(a.parent.as_str())
}

fn plural_agents(n: usize) -> String {
    if n == 1 { "1 agent".into() } else { format!("{n} agents") }
}

/// Past `MAX_AGENTS`, the finished agents fold into a count per mind, oldest-looking ids first
/// kept out — what is working, waiting or failed always stays on the map.
fn fold(agents: &[Agent]) -> (Vec<Agent>, BTreeMap<String, usize>) {
    let mut kept: Vec<Agent> = agents.to_vec();
    kept.sort_by(|a, b| a.id.cmp(&b.id));
    let mut folded = BTreeMap::new();
    if kept.len() <= MAX_AGENTS {
        return (kept, folded);
    }
    let mut over = kept.len() - MAX_AGENTS;
    kept.retain(|a| {
        if over > 0 && a.state == "done" {
            over -= 1;
            *folded.entry(a.mind.clone()).or_insert(0) += 1;
            false
        } else {
            true
        }
    });
    (kept, folded)
}

/// Each label beside its node, on the side away from the middle — or under or over it, for a node
/// straight below or above the middle, where a label beside it would lie across the spokes — then
/// moved the least distance up or down that clears every label already placed and every node.
/// When that spot has no room on the map (a small screen, a node near the edge), the label tries
/// the other spots around its node in turn: a label pushed off the map, or onto a node, hides the
/// very thing it names.
fn place_labels(nodes: &mut [Node], cx: f32, width: f32, height: f32) {
    let cy = height / 2.0;
    let bodies: Vec<Rect> = nodes.iter().map(|n| Rect::around(n.x, n.y, n.size + 4.0)).collect();
    // The middle out: the hub, then the minds, then agents from the inner rings outward, so the
    // labels nearest the middle keep their places and the outer ones make room.
    let mut order: Vec<usize> = (0..nodes.len()).collect();
    order.sort_by(|&a, &b| {
        let da = (nodes[a].x - cx).abs();
        let db = (nodes[b].x - cx).abs();
        rank(nodes[a].kind).cmp(&rank(nodes[b].kind)).then(da.total_cmp(&db)).then(nodes[a].id.cmp(&nodes[b].id))
    });
    let mut placed: Vec<Rect> = Vec::new();
    for i in order {
        let n = &nodes[i];
        if n.kind == "hub" {
            continue;
        }
        let (dx, dy) = (n.x - cx, n.y - cy);
        let steep = dx.abs() < 0.35 * dy.abs();
        let spot = |r: Rect| Rect { x: r.x.clamp(MARGIN, width - MARGIN - LABEL_W), ..r };
        let right = spot(Rect { x: n.x + n.size / 2.0 + GAP, y: n.y - LABEL_H / 2.0, w: LABEL_W, h: LABEL_H });
        let left = spot(Rect { x: n.x - n.size / 2.0 - GAP - LABEL_W, y: n.y - LABEL_H / 2.0, w: LABEL_W, h: LABEL_H });
        let below = spot(Rect { x: n.x - LABEL_W / 2.0, y: n.y + n.size / 2.0 + GAP, w: LABEL_W, h: LABEL_H });
        let above = spot(Rect { x: n.x - LABEL_W / 2.0, y: n.y - n.size / 2.0 - GAP - LABEL_H, w: LABEL_W, h: LABEL_H });
        let (outward, inward) = if n.x >= cx { (right, left) } else { (left, right) };
        let spots = match (steep, dy > 0.0) {
            (true, true) => [below, outward, inward, above],
            (true, false) => [above, outward, inward, below],
            (false, true) => [outward, below, above, inward],
            (false, false) => [outward, above, below, inward],
        };
        // Its own node too: a label slid up or down over the node it names hides it.
        let obstacles: Vec<Rect> = placed.iter().chain(bodies.iter()).copied().collect();
        let on_map = |r: &Rect| r.y >= MARGIN && r.bottom() <= height - MARGIN;
        let clear = |r: &Rect| on_map(r) && !obstacles.iter().any(|o| o.overlaps(r));
        // The nearest clear place up or down from each spot, the first spot that has one.
        let label = spots
            .iter()
            .find_map(|&want| {
                let down = slide(want, &obstacles, 1.0);
                let up = slide(want, &obstacles, -1.0);
                [down, up]
                    .into_iter()
                    .filter(|r| clear(r))
                    .min_by(|a, b| (a.y - want.y).abs().total_cmp(&(b.y - want.y).abs()))
            })
            .unwrap_or_else(|| Rect { y: spots[0].y.clamp(MARGIN, height - MARGIN - LABEL_H), ..spots[0] });
        placed.push(label);
        nodes[i].label = label;
    }
}

fn rank(kind: &str) -> u8 {
    match kind {
        "hub" => 0,
        "mind" => 1,
        _ => 2,
    }
}

/// Move `r` straight up or down until it touches nothing in `obstacles`.
fn slide(mut r: Rect, obstacles: &[Rect], direction: f32) -> Rect {
    for _ in 0..(obstacles.len() * 2 + 1) {
        let Some(hit) = obstacles.iter().find(|o| o.overlaps(&r)) else { return r };
        r.y = if direction > 0.0 { hit.bottom() + GAP } else { hit.y - GAP - r.h };
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mind(id: &str, name: &str, detail: &str) -> Mind {
        Mind { id: id.into(), name: name.into(), detail: detail.into() }
    }

    fn agent(id: &str, mind: &str, state: &str, parent: &str, role: &str, title: &str) -> Agent {
        Agent {
            id: id.into(),
            mind: mind.into(),
            title: title.into(),
            state: state.into(),
            label: state.replace('_', " "),
            since: "2m".into(),
            parent: parent.into(),
            role: role.into(),
            origin: String::new(),
        }
    }

    /// Three minds; a Council started by a recipe, its Chair with three roles under it; pi
    /// waiting on the person; a finished Hermes agent; a failed OpenClaw one; and one whose
    /// parent has gone.
    fn fixture() -> (Vec<Mind>, Vec<Agent>) {
        let minds = vec![
            mind("hermes", "Hermes", "deepseek-v4.1-flash"),
            mind("openclaw", "OpenClaw", "ollama-cloud/kimi-k3"),
            mind("pi", "pi", "qwen3.8-max"),
        ];
        let mut chair = agent("hermes:council", "hermes", "thinking", "", "Chair", "Publish tonight's nightly?");
        chair.origin = "Council recipe".into();
        let agents = vec![
            chair,
            agent("hermes:council-1", "hermes", "running_tool", "hermes:council", "Researcher", "What changed since the last nightly"),
            agent("hermes:council-2", "hermes", "thinking", "hermes:council", "Red team", "Attack the publish plan"),
            agent("hermes:council-3", "hermes", "waiting_for_you", "hermes:council", "Planner", "Order the release steps"),
            agent("hermes:photos", "hermes", "done", "", "", "Tidy the photos folder"),
            agent("pi:main", "pi", "waiting_for_you", "", "", "Delete the old calendar events"),
            agent("pi:orphan", "pi", "running_tool", "pi:gone", "", "Summarise the inbox"),
            agent("openclaw:web", "openclaw", "failed", "", "", "Read the top three stories"),
        ];
        (minds, agents)
    }

    fn node<'a>(map: &'a Map, id: &str) -> &'a Node {
        map.nodes.iter().find(|n| n.id == id).unwrap_or_else(|| panic!("no node {id}"))
    }

    #[test]
    fn no_label_covers_another_label_or_a_node_and_all_stay_on_the_map() {
        let (minds, agents) = fixture();
        // Every map size from the Agents screen at 800×600 (744×467 of map) to a large monitor,
        // not three picked ones: 760×480 passed while 744×467 put two labels over their nodes.
        let sizes = (0..=60).flat_map(|i| (0..=30).map(move |j| (700.0 + 20.0 * i as f32, 420.0 + 10.0 * j as f32)));
        for (w, h) in sizes.chain([(744.0, 467.0), (744.0, 475.0), (1224.0, 675.0)]) {
            let map = layout("vm-520", &minds, &agents, w, h);
            assert_eq!(map.nodes.len(), 1 + 3 + agents.len(), "hub, three minds and every agent at {w}×{h}");
            for (i, a) in map.nodes.iter().enumerate() {
                let l = a.label;
                assert!(l.x >= 0.0 && l.y >= 0.0 && l.x + l.w <= w && l.y + l.h <= h, "{} label off the map at {w}×{h}: {l:?}", a.id);
                for (j, b) in map.nodes.iter().enumerate() {
                    // Its own node included: at 744×467 a label slid up over the very node it
                    // named, and a check that skipped a label's own node passed it.
                    assert!(!l.overlaps(&Rect::around(b.x, b.y, b.size)), "{}'s label covers node {} at {w}×{h}", a.id, b.id);
                    if i != j {
                        assert!(!l.overlaps(&b.label), "{} and {} labels overlap at {w}×{h}", a.id, b.id);
                    }
                }
            }
        }
    }

    #[test]
    fn a_formation_hangs_off_its_chair_and_the_chair_off_its_mind() {
        let (minds, agents) = fixture();
        let map = layout("vm-520", &minds, &agents, 1240.0, 690.0);
        let (cx, cy) = (620.0, 345.0);
        let reach = |n: &Node| ((n.x - cx) / (620.0 - LABEL_W)).hypot((n.y - cy) / 345.0);
        let chair = node(&map, "hermes:council");
        let hermes = node(&map, "mind:hermes");
        for role in ["hermes:council-1", "hermes:council-2", "hermes:council-3"] {
            assert!(reach(node(&map, role)) > reach(chair), "{role} sits further out than its Chair");
        }
        assert!(reach(chair) > reach(hermes), "the Chair sits further out than Hermes");
        assert!(chair.sub.contains("Council recipe"), "the recipe names the formation's root: {}", chair.sub);
        assert!(!node(&map, "hermes:council-1").sub.contains("Council recipe"), "and not every role under it");
        assert_eq!(chair.title, "Chair · Publish tonight's nightly?");
    }

    #[test]
    fn an_agent_whose_parent_is_gone_hangs_off_its_mind() {
        let (minds, agents) = fixture();
        let map = layout("vm-520", &minds, &agents, 1240.0, 690.0);
        let orphan = node(&map, "pi:orphan");
        let pi = node(&map, "mind:pi");
        let link = format!("M {:.1} {:.1} Q", pi.x, pi.y);
        let to = format!("{:.1} {:.1}", orphan.x, orphan.y);
        assert!(
            map.edges.iter().any(|e| !e.leader && e.d.starts_with(&link) && e.d.ends_with(&to)),
            "pi:orphan is linked from pi"
        );
    }

    #[test]
    fn the_way_to_what_is_waiting_is_marked_from_the_middle_out() {
        let (minds, agents) = fixture();
        let map = layout("vm-520", &minds, &agents, 1240.0, 690.0);
        let ends = |id: &str| {
            let n = node(&map, id);
            let to = format!("{:.1} {:.1}", n.x, n.y);
            map.edges.iter().find(|e| !e.leader && e.d.ends_with(&to)).unwrap_or_else(|| panic!("no link to {id}")).clone()
        };
        assert!(ends("hermes:council-3").hot, "the link to the waiting Planner");
        assert!(ends("hermes:council").hot, "the link to the Chair above it");
        assert!(ends("mind:hermes").hot, "the link from the machine to Hermes");
        assert!(!ends("hermes:council-1").hot, "not the Researcher beside it");
        assert!(!ends("mind:openclaw").hot, "not a mind with nothing waiting");
    }

    #[test]
    fn nothing_moves_when_one_agent_changes_state_and_the_same_input_gives_the_same_map() {
        let (minds, mut agents) = fixture();
        let before = layout("vm-520", &minds, &agents, 1240.0, 690.0);
        assert_eq!(before, layout("vm-520", &minds, &agents, 1240.0, 690.0));
        agents[1].state = "done".into();
        let after = layout("vm-520", &minds, &agents, 1240.0, 690.0);
        for (a, b) in before.nodes.iter().zip(&after.nodes) {
            assert_eq!((a.id.as_str(), a.x, a.y, a.label), (b.id.as_str(), b.x, b.y, b.label));
        }
        // The order agents arrive in does not matter either.
        agents.reverse();
        let reversed = layout("vm-520", &minds, &agents, 1240.0, 690.0);
        assert_eq!(after.nodes, reversed.nodes);
    }

    #[test]
    fn a_parent_chain_that_comes_round_to_itself_does_not_hang_the_map() {
        let minds = vec![mind("pi", "pi", "")];
        let agents = vec![
            agent("pi:a", "pi", "thinking", "pi:b", "", "a"),
            agent("pi:b", "pi", "thinking", "pi:a", "", "b"),
        ];
        let map = layout("", &minds, &agents, 1240.0, 690.0);
        assert!(map.nodes.iter().any(|n| n.id == "pi:a") && map.nodes.iter().any(|n| n.id == "pi:b"));
    }

    #[test]
    fn a_crowded_machine_folds_what_is_finished_and_keeps_everything_else() {
        let minds = vec![mind("hermes", "Hermes", "")];
        let mut agents: Vec<Agent> =
            (0..45).map(|i| agent(&format!("hermes:{i:02}"), "hermes", "done", "", "", "old job")).collect();
        agents.push(agent("hermes:zz-live", "hermes", "running_tool", "", "", "live job"));
        agents.push(agent("hermes:zz-wait", "hermes", "waiting_for_you", "", "", "needs you"));
        let map = layout("", &minds, &agents, 1880.0, 1000.0);
        let shown = map.nodes.iter().filter(|n| n.kind == "agent").count();
        assert_eq!(shown, MAX_AGENTS);
        let more = node(&map, "more:hermes");
        assert_eq!(more.title, format!("+{} done", agents.len() - MAX_AGENTS));
        assert!(map.nodes.iter().any(|n| n.id == "hermes:zz-live") && map.nodes.iter().any(|n| n.id == "hermes:zz-wait"));
    }

    #[test]
    fn the_summary_counts_what_a_person_would_ask_about() {
        let (minds, agents) = fixture();
        assert_eq!(
            summary(&minds, &agents),
            "3 minds attached · 4 working · 2 waiting on you · 1 finished · 1 failed"
        );
        assert_eq!(summary(&[], &[]), "0 minds attached");
        assert_eq!(summary(&minds[..1], &agents[4..5]), "1 mind attached · 1 finished");
    }

    #[test]
    fn a_map_too_small_to_draw_draws_nothing_but_still_counts() {
        let (minds, agents) = fixture();
        let map = layout("", &minds, &agents, 300.0, 120.0);
        assert!(map.nodes.is_empty() && map.edges.is_empty());
        assert!(map.summary.starts_with("3 minds attached"));
    }
}
