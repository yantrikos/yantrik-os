//! The Agents screen (34), and every agent popped out into a window of its own.
//!
//! Both draw from the one store in `crate::agents`, through the `AgentsState` global in
//! `agents.slint`. The screen's instance of that global lives on the shell's window; each popped-out
//! `AgentWindow` has its own, filled by the same code from the same store — so the two views of an
//! agent cannot disagree, and closing a window closes a view, never the agent.
//!
//! A timer redraws a quarter of a second at a time: the list and the counts while the screen is up
//! (the "running · 2m" moves on its own), and each open agent window. A session is rebuilt only when
//! the store has changed or the person opened or folded something — or, while an approval card is
//! up in it, every tick, so its countdown moves.
//!
//! The same tick watches for what the person should hear about an agent they are not looking at —
//! "pi finished: …", "deepseek needs you" — and says it through the notification service, as the
//! desktop (see [`Watch`]).

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle, Model, ModelRc, Timer, TimerMode, VecModel};

use crate::agents::model::{
    bytes, now, Agent, Approval, ApprovalOutcome, CallState, Card, Details, Item, Mark, OutputKind, Provenance, State, Tab, Turn,
};
use crate::agents::{self, feed, launch, AgentId, Store};
use crate::app_context::AppContext;
use crate::{
    AccentPreset, AgentDetailsData, AgentHeaderData, AgentItemData, AgentMindData, AgentRoleData, AgentRowData,
    AgentRunData, AgentTabData, AgentWindow, AgentsState, App, ApprovalRequest, OverviewEdge, OverviewNode, ThemeMode,
    ThemeOverrides, ToolCallData,
};

/// The screen id `app.slint` draws the Agents screen at.
pub const SCREEN: i32 = 34;

const TICK: Duration = Duration::from_millis(250);

/// How many of an agent's turns a session shows. The rest are in its file.
const SHOWN_TURNS: usize = 30;

/// How many lines of a running call's text output show under its line.
const LIVE_LINES: usize = 8;

/// How much of a call's output an opened card shows. All of it is one click further, in the Editor.
const OPEN_BYTES: usize = 64 * 1024;

/// How much of one block of the mind's text is drawn.
const TEXT_BYTES: usize = 32 * 1024;

/// What one surface shows: the screen, or one popped-out window.
struct Surface {
    agent: Option<AgentId>,
    /// Cards and thinking the person opened, by key.
    expanded: HashSet<String>,
    items: Rc<VecModel<AgentItemData>>,
    keys: Vec<String>,
    /// The store revision and local change drawn last.
    drawn: Option<(u64, u64)>,
    local: u64,
}

impl Surface {
    fn new() -> Self {
        Surface {
            agent: None,
            expanded: HashSet::new(),
            items: Rc::new(VecModel::default()),
            keys: Vec::new(),
            drawn: None,
            local: 0,
        }
    }

    fn toggle(&mut self, key: &str, open: bool) {
        if open {
            self.expanded.insert(key.to_string());
        } else {
            self.expanded.remove(key);
        }
        self.local += 1;
    }
}

/// An agent in its own window.
struct Popped {
    window: AgentWindow,
    surface: Surface,
    /// Set by the window's ×. The window is hidden then, and let go on the next tick — never from
    /// inside its own close handler.
    closed: Rc<Cell<bool>>,
    title: String,
}

struct Screen {
    tab: Tab,
    selected: Option<AgentId>,
    /// The rows as drawn, so they can be held still while the pointer is over them.
    order: Vec<AgentId>,
    main: Surface,
    windows: BTreeMap<AgentId, Popped>,
    /// When New agent last read the catalog, while it is open.
    roles_read: Option<std::time::Instant>,
    /// The Overview's map, as big as it last said it was: what it is laid out for (#226).
    overview_size: (f32, f32),
}

type Shared = Rc<RefCell<Screen>>;

pub fn wire(ui: &App, _ctx: &AppContext) {
    let state: Shared = Rc::new(RefCell::new(Screen {
        tab: Tab::Active,
        selected: None,
        order: Vec::new(),
        main: Surface::new(),
        windows: BTreeMap::new(),
        roles_read: None,
        overview_size: (0.0, 0.0),
    }));
    let g = ui.global::<AgentsState>();
    g.set_items(ModelRc::from(state.borrow().main.items.clone()));

    let weak = ui.as_weak();
    let on = |f: fn(&App, &Shared, String)| {
        let (weak, state) = (weak.clone(), state.clone());
        move |arg: slint::SharedString| {
            if let Some(ui) = weak.upgrade() {
                f(&ui, &state, arg.to_string());
            }
        }
    };

    g.on_select_tab(on(|ui, state, key| {
        {
            let mut st = state.borrow_mut();
            st.tab = Tab::from_key(&key);
            st.order.clear();
            st.selected = None;
        }
        ui.global::<AgentsState>().set_tab(key.into());
        refresh(ui, state, true);
    }));
    g.on_select(on(|ui, state, id| {
        state.borrow_mut().selected = Some(AgentId(id));
        refresh(ui, state, true);
    }));
    g.on_pop_out(on(|ui, state, id| pop_out(ui, state, AgentId(id))));
    g.on_stop(on(|ui, state, id| {
        notice(&ui.global::<AgentsState>(), launch::stop(&AgentId(id)));
        refresh(ui, state, true);
    }));
    g.on_close(on(|ui, state, id| close(ui, state, AgentId(id), false)));
    g.on_close_confirmed(on(|ui, state, id| close(ui, state, AgentId(id), true)));
    g.on_close_cancelled({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                ui.global::<AgentsState>().set_confirm_close("".into());
            }
        }
    });
    g.on_toggle({
        let (weak, state) = (weak.clone(), state.clone());
        move |key, open| {
            let Some(ui) = weak.upgrade() else { return };
            state.borrow_mut().main.toggle(&key, open);
            refresh(&ui, &state, false);
        }
    });
    g.on_open_all(on(|ui, state, key| {
        let agent = state.borrow().main.agent.clone();
        if let Some(agent) = agent {
            notice(&ui.global::<AgentsState>(), open_all(&agent, &key));
        }
    }));
    g.on_pick_mind(on(|ui, _state, mind| pick_mind(&ui.global::<AgentsState>(), &mind)));
    g.on_start({
        let (weak, state) = (weak.clone(), state.clone());
        move |mind, prompt| {
            let Some(ui) = weak.upgrade() else { return };
            let outcome = launch::start(&mind, &prompt);
            started(&ui, &state, outcome);
        }
    });
    // ── Agents catalog: New agent → from the catalog. A role, its purpose and where it would run
    // are shown; Start hands the task to it as the person (`control_agents::hand_off`), held to
    // the role's reach like any hand-off.
    g.on_pick_role(on(|ui, _state, role| pick_role(&ui.global::<AgentsState>(), &role)));
    g.on_overview_resized({
        let (weak, state) = (weak.clone(), state.clone());
        move |width, height| {
            let Some(ui) = weak.upgrade() else { return };
            state.borrow_mut().overview_size = (width, height);
            refresh(&ui, &state, false);
        }
    });
    g.on_start_role({
        let (weak, state) = (weak.clone(), state.clone());
        move |role, task| {
            let Some(ui) = weak.upgrade() else { return };
            let outcome = crate::control_agents::hand_off_from_screen(&role, &task);
            started(&ui, &state, outcome);
        }
    });
    g.on_send({
        let (weak, state) = (weak.clone(), state.clone());
        move |agent, text| {
            let Some(ui) = weak.upgrade() else { return };
            notice(&ui.global::<AgentsState>(), launch::send(&AgentId(agent.to_string()), &text));
            refresh(&ui, &state, true);
        }
    });
    g.on_dismiss_notice({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                ui.global::<AgentsState>().set_notice("".into());
            }
        }
    });

    // ── Agents glue ──
    //
    // Allow and Deny on a card in a pane. The pane's card is the Lens's own component bound to the
    // one request id, and a press here goes to the very callbacks the Lens's card calls — the only
    // place a grant is made (`control_approvals::wire`). Answering here answers it there, and the
    // other way round, because there is one request and one store.
    forward_approvals(&g, &weak);
    g.on_show_agent(on(|ui, state, id| show_agent(ui, state, AgentId(id))));
    // The Lens's "open in Agents": the active mind's own conversation, `<id>:main`.
    ui.on_lens_open_in_agents({
        let (weak, state) = (weak.clone(), state.clone());
        move || {
            let Some(ui) = weak.upgrade() else { return };
            let Some(host) = crate::wire::harness::host() else { return };
            let agent = feed::main_agent(&host.active_id());
            // Known, so it can be selected, even before the first question: it is a live
            // conversation while its mind is attached.
            agents::store().upsert_agent(feed::meta_for(&agent));
            ui.set_lens_open(false);
            show_agent(&ui, &state, agent);
        }
    });
    // The Lens's History: every conversation with any mind, on the All tab, where each opens with
    // its whole session and can be carried on (#246).
    ui.on_lens_open_history({
        let (weak, state) = (weak.clone(), state.clone());
        move || {
            let Some(ui) = weak.upgrade() else { return };
            {
                let mut st = state.borrow_mut();
                st.tab = Tab::All;
                st.order.clear();
            }
            let g = ui.global::<AgentsState>();
            g.set_tab(Tab::All.key().into());
            g.set_view("list".into());
            ui.set_lens_open(false);
            ui.set_current_screen(SCREEN);
            ui.invoke_navigate(SCREEN);
            refresh(&ui, &state, true);
        }
    });

    let watch = RefCell::new(Watch::default());
    let timer = Timer::default();
    {
        let (weak, state) = (weak.clone(), state.clone());
        timer.start(TimerMode::Repeated, TICK, move || {
            agents::store().save_if_due();
            let seen = Seen::now();
            sync_with_host(&seen);
            let Some(ui) = weak.upgrade() else { return };
            // The Lens offers "open in Agents" while its conversation is an attached mind's.
            let lens_agent = crate::wire::harness::host()
                .map(|h| h.active_id())
                .is_some_and(|active| seen.minds.iter().any(|(id, _, _)| *id == active));
            if ui.get_lens_can_open_in_agents() != lens_agent {
                ui.set_lens_can_open_in_agents(lens_agent);
            }
            if ui.get_current_screen() == SCREEN {
                refresh(&ui, &state, false);
            }
            refresh_windows(&ui, &state);
            tell_the_person(&ui, &state, &mut watch.borrow_mut());
        });
    }
    // The timer lives as long as the shell, the idiom every wire module uses.
    std::mem::forget(timer);
}

/// New agent's Start, either way: the new agent selected on Active, or why it did not start.
fn started(ui: &App, state: &Shared, outcome: Result<AgentId, String>) {
    let g = ui.global::<AgentsState>();
    match outcome {
        Ok(agent) => {
            g.set_new_open(false);
            g.set_new_error("".into());
            {
                let mut st = state.borrow_mut();
                if st.tab != Tab::All {
                    st.tab = Tab::Active;
                    g.set_tab(Tab::Active.key().into());
                }
                st.order.clear();
                st.selected = Some(agent);
            }
            refresh(ui, state, true);
        }
        Err(why) => g.set_new_error(why.into()),
    }
}

/// The catalog's roles as New agent lists them: each one's purpose and reach, and the mind it
/// would run on now — or that none of its minds is attached.
fn roles_now() -> Vec<AgentRoleData> {
    let catalog = crate::agents::catalog::Catalog::load();
    let attached = crate::wire::harness::host().map(crate::agents::catalog::minds_now).unwrap_or_default();
    catalog
        .roles
        .iter()
        .map(|r| {
            let runs_on = r.pick_mind(&attached).ok().unwrap_or_default();
            AgentRoleData {
                id: r.id.as_str().into(),
                name: r.name.as_str().into(),
                purpose: r.purpose.as_str().into(),
                reach: r.reach.text().into(),
                available: !runs_on.is_empty(),
                runs_on: runs_on.into(),
            }
        })
        .collect()
}

fn pick_role(g: &AgentsState, role: &str) {
    g.set_new_role(role.into());
    g.set_new_error("".into());
    let catalog = crate::agents::catalog::Catalog::load();
    let Some(r) = catalog.find(role) else {
        g.set_new_note("".into());
        g.set_new_note_reach("".into());
        return;
    };
    let attached = crate::wire::harness::host().map(crate::agents::catalog::minds_now).unwrap_or_default();
    let (note, patterns) = match r.pick_mind(&attached) {
        // The reach in words a person reads; the patterns it was made from stay one click away
        // in the dialog (#212), because the doors enforce the patterns, not the sentence.
        Ok(mind) => (
            format!(
                "Runs on {mind}. May reach: {}. Hands back: {} Up to {} turns and {} minutes.",
                r.reach.words(),
                r.returns,
                r.budget.turns,
                r.budget.minutes
            ),
            r.reach.text(),
        ),
        Err(why) => (why, String::new()),
    };
    g.set_new_note(note.into());
    g.set_new_note_reach(patterns.into());
}

/// Say how a press went, when it did not go as asked.
fn notice(g: &AgentsState, outcome: Result<(), String>) {
    match outcome {
        Ok(()) => g.set_notice("".into()),
        Err(why) => g.set_notice(why.into()),
    }
}

/// What the harness host says right now: the minds attached, and the agents live on them.
struct Seen {
    /// Attached minds that can hold agents — every one but the built-in, which answers in the
    /// Lens: (id, name, what it says it runs on).
    minds: Vec<(String, String, String)>,
    agents: Vec<yantrik_harness::AgentEntry>,
    /// The approval requests waiting on the person, from the shell's own store: what a pane's
    /// approval card is drawn from. Read here, before the agents store is locked — nothing holds
    /// both locks at once.
    approvals: Vec<crate::approvals::Card>,
}

impl Seen {
    fn now() -> Seen {
        let approvals = crate::approvals::pending();
        let Some(host) = crate::wire::harness::host() else {
            return Seen { minds: Vec::new(), agents: Vec::new(), approvals };
        };
        Seen {
            minds: host
                .list()
                .into_iter()
                .filter(|e| !e.builtin)
                .map(|e| (e.id, e.name, e.detail.unwrap_or_default()))
                .collect(),
            agents: host.agents(),
            approvals,
        }
    }

    fn attached(&self) -> Vec<String> {
        self.minds.iter().map(|(id, _, _)| id.clone()).collect()
    }

    fn live(&self) -> Vec<AgentId> {
        self.agents.iter().map(|a| a.id.clone()).collect()
    }
}

/// Keep the list honest about the host: a conversation the host is running shows up here even
/// when this screen did not start it, and an agent caught working when its harness went is marked
/// so. Nothing else is taken from the host — the session itself comes in through `feed`.
fn sync_with_host(seen: &Seen) {
    if crate::wire::harness::host().is_none() {
        return;
    }
    let attached = seen.attached();
    let (unknown, gone) = agents::store().read(|s| {
        let unknown: Vec<agents::AgentMeta> = seen
            .agents
            .iter()
            .filter(|e| s.agent(&e.id).is_none())
            .map(|e| {
                let mut meta = agents::AgentMeta::new(e.id.clone(), e.harness_name.clone());
                meta.conversations = e.conversations;
                meta.started = e.started.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                meta
            })
            .collect();
        let gone: Vec<AgentId> = s
            .agents()
            .iter()
            .filter(|a| a.state.working() && a.meta.id.harness() != crate::wire::harness::BUILTIN_ID)
            .filter(|a| !attached.iter().any(|h| h == a.meta.id.harness()))
            .map(|a| a.meta.id.clone())
            .collect();
        (unknown, gone)
    });
    for meta in unknown {
        agents::store().upsert_agent(meta);
    }
    for agent in gone {
        agents::store().set_state(&agent, State::HarnessGone);
        // "When a harness dies": its pending approvals are withdrawn — a card for a mind that is
        // gone is refused, never granted. The approval store's tick redraws the Lens and the pane.
        crate::approvals::withdraw_for_agent(&agent.0);
    }
}

fn pick_mind(g: &AgentsState, mind: &str) {
    g.set_new_mind(mind.into());
    g.set_new_error("".into());
    // A mind has no reach of its own: whatever a role's patterns line said goes away with it.
    g.set_new_note_reach("".into());
    let seen = Seen::now();
    let name = seen.minds.iter().find(|(id, _, _)| id == mind).map(|(_, name, _)| name.clone()).unwrap_or_else(|| mind.to_string());
    let theirs: Vec<&yantrik_harness::AgentEntry> = seen.agents.iter().filter(|a| a.harness == mind).collect();
    let note = if theirs.iter().any(|a| a.conversations) {
        format!("{name} holds a conversation per agent: this one starts fresh, apart from the Lens.")
    } else if !theirs.is_empty() {
        format!(
            "{name} holds one conversation at a time, so this continues it — the same \
             conversation the Lens has with it."
        )
    } else {
        String::new()
    };
    g.set_new_note(note.into());
}

/// Redraw the screen: tabs, list, and the selected agent's session and details.
fn refresh(ui: &App, state: &Shared, force: bool) {
    let g = ui.global::<AgentsState>();
    let seen = Seen::now();
    let hovering = g.get_list_hovered();
    let mut st = state.borrow_mut();
    let st = &mut *st;
    agents::store().read(|s| {
        let tabs: Vec<AgentTabData> = Tab::EVERY
            .iter()
            .zip(s.counts())
            .map(|(tab, count)| AgentTabData { id: tab.key().into(), label: tab.label().into(), count: count as i32 })
            .collect();
        if let Some(model) = crate::models::changed(g.get_tabs(), tabs) {
            g.set_tabs(model);
        }

        let empty = empty_note(st.tab, s.counts());
        if g.get_empty_note() != empty.as_str() {
            g.set_empty_note(empty.into());
        }

        let hold = (hovering && !st.order.is_empty()).then_some(st.order.as_slice());
        let order = s.list(st.tab, hold);
        let rows: Vec<AgentRowData> = order.iter().filter_map(|id| s.agent(id)).map(row_of).collect();
        st.order = order;
        if let Some(model) = crate::models::changed(g.get_rows(), rows) {
            g.set_rows(model);
        }

        if st.selected.as_ref().is_none_or(|id| s.agent(id).is_none()) {
            st.selected = st.order.first().cloned();
        }
        let selected = st.selected.clone().map(|id| id.0).unwrap_or_default();
        if g.get_selected() != selected.as_str() {
            g.set_selected(selected.into());
        }
        let selected = st.selected.clone();
        draw(&g, &mut st.main, s, selected.as_ref(), &seen, force);

        if g.get_view() == "overview" {
            overview(&g, s, &seen, st.overview_size);
        }
    });

    if g.get_new_open() {
        let rows: Vec<AgentMindData> = seen
            .minds
            .iter()
            .map(|(id, name, detail)| AgentMindData { id: id.into(), name: name.into(), detail: detail.into() })
            .collect();
        if let Some(model) = crate::models::changed(g.get_minds(), rows) {
            g.set_minds(model);
        }
        // Start where the Lens is, when the Lens is talking to a mind that can hold an agent.
        if g.get_new_mind().is_empty() {
            if let Some(active) = crate::wire::harness::host().map(|h| h.active_id()) {
                if seen.minds.iter().any(|(id, _, _)| *id == active) {
                    pick_mind(&g, &active);
                }
            }
        }
        // The catalog's roles, read again every couple of seconds while the dialog is open: the
        // person's own files and the minds attached can change under it.
        if st.roles_read.is_none_or(|at| at.elapsed() >= ROLES_EVERY) {
            st.roles_read = Some(std::time::Instant::now());
            if let Some(model) = crate::models::changed(g.get_roles(), roles_now()) {
                g.set_roles(model);
            }
        }
    } else {
        st.roles_read = None;
    }
}

/// The Overview's map (#226): every agent whatever the tab, under the minds attached now, laid out
/// for the size the map last reported. Set only where it changed, so a map in which nothing moves
/// asks for no redraw.
fn overview(g: &AgentsState, s: &Store, seen: &Seen, (width, height): (f32, f32)) {
    use crate::agents_overview as map;
    let minds: Vec<map::Mind> = seen
        .minds
        .iter()
        .map(|(id, name, detail)| map::Mind { id: id.clone(), name: name.clone(), detail: detail.clone() })
        .collect();
    let agents: Vec<map::Agent> = s
        .list(Tab::All, None)
        .iter()
        .filter_map(|id| s.agent(id))
        .map(|a| {
            let row = row_of(a);
            map::Agent {
                id: row.id.into(),
                mind: row.mind.into(),
                title: row.title.into(),
                state: row.state.into(),
                label: row.label.into(),
                since: row.since.into(),
                parent: row.parent.into(),
                role: row.role.into(),
                origin: row.origin.into(),
            }
        })
        .collect();
    let laid = map::layout(host_name(), &minds, &agents, width, height);
    let nodes: Vec<OverviewNode> = laid
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
    let edges: Vec<OverviewEdge> = laid
        .edges
        .iter()
        .map(|e| OverviewEdge { d: e.d.as_str().into(), state: e.state.as_str().into(), hot: e.hot, leader: e.leader })
        .collect();
    if let Some(model) = crate::models::changed(g.get_overview_nodes(), nodes) {
        g.set_overview_nodes(model);
    }
    if let Some(model) = crate::models::changed(g.get_overview_edges(), edges) {
        g.set_overview_edges(model);
    }
    if g.get_overview_summary() != laid.summary.as_str() {
        g.set_overview_summary(laid.summary.into());
    }
}

/// This machine's name, for the middle of the map. Read once: it does not change under a running
/// shell, and the map is laid out again every time an agent does.
fn host_name() -> &'static str {
    static HOST: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    HOST.get_or_init(|| {
        std::fs::read_to_string("/proc/sys/kernel/hostname").map(|h| h.trim().to_string()).unwrap_or_default()
    })
}

/// How often New agent reads the catalog again while it is open.
const ROLES_EVERY: Duration = Duration::from_secs(2);

/// Redraw every popped-out window, and let go of the ones whose × was pressed.
fn refresh_windows(ui: &App, state: &Shared) {
    let mut st = state.borrow_mut();
    st.windows.retain(|_, p| !p.closed.get());
    if st.windows.is_empty() {
        return;
    }
    let seen = Seen::now();
    let windows = &mut st.windows;
    agents::store().read(|s| {
        for (id, popped) in windows.iter_mut() {
            sync_theme(ui, &popped.window);
            draw(&popped.window.global::<AgentsState>(), &mut popped.surface, s, Some(id), &seen, false);
        }
    });
}

/// Fill one surface with one agent: its header and details every time (they carry the clock), its
/// session when something in it changed.
fn draw(g: &AgentsState, surface: &mut Surface, s: &Store, agent: Option<&AgentId>, seen: &Seen, force: bool) {
    let Some(a) = agent.and_then(|id| s.agent(id)) else {
        if g.get_has_agent() {
            g.set_has_agent(false);
            g.set_header(AgentHeaderData::default());
            g.set_details(AgentDetailsData::default());
        }
        if surface.agent.take().is_some() || surface.items.row_count() > 0 {
            publish_items(g, surface, Vec::new(), true);
        }
        return;
    };
    if !g.get_has_agent() {
        g.set_has_agent(true);
    }
    let header = header_of(a, seen);
    if g.get_header() != header {
        g.set_header(header);
    }
    let details = details_of(a, s.details(&a.meta.id).unwrap_or_default());
    if g.get_details() != details {
        g.set_details(details);
    }

    let fresh = surface.agent.as_ref() != Some(&a.meta.id);
    if fresh {
        surface.agent = Some(a.meta.id.clone());
        surface.expanded.clear();
        surface.drawn = None;
    }
    let stamp = (s.revision(), surface.local);
    // An approval card waiting in the pane counts down, which the store does not change for.
    let counting = !a.pending_approvals.is_empty();
    if !force && !fresh && !counting && surface.drawn == Some(stamp) {
        return;
    }
    surface.drawn = Some(stamp);
    publish_items(g, surface, items_of(a, &surface.expanded, &seen.approvals), fresh);
}

/// Put a session's items in the model: in place when only the end changed, so the view keeps its
/// scroll and each card keeps its state; a new model when the agent or the shape changed.
fn publish_items(g: &AgentsState, surface: &mut Surface, items: Vec<AgentItemData>, fresh: bool) {
    let keys: Vec<String> = items.iter().map(|i| i.key.to_string()).collect();
    let extends = !fresh && keys.len() >= surface.keys.len() && surface.keys.iter().zip(&keys).all(|(a, b)| a == b);
    if extends {
        let model = &surface.items;
        for (i, item) in items.into_iter().enumerate() {
            if i < model.row_count() {
                if model.row_data(i).as_ref() != Some(&item) {
                    model.set_row_data(i, item);
                }
            } else {
                model.push(item);
            }
        }
    } else {
        surface.items = Rc::new(VecModel::from(items));
        g.set_items(ModelRc::from(surface.items.clone()));
    }
    surface.keys = keys;
}

// ── From the store to what the screen draws ────────────────────────

fn row_of(a: &Agent) -> AgentRowData {
    AgentRowData {
        id: a.meta.id.0.as_str().into(),
        mind: a.meta.mind.as_str().into(),
        title: latest_request(&a.turns, &a.meta.title).into(),
        state: a.state.key().into(),
        label: a.state.label().into(),
        since: since(a).into(),
        parent: a.meta.parent.as_ref().map(|p| p.0.clone()).unwrap_or_default().into(),
        role: a.meta.role.as_ref().map(|r| r.name.clone()).unwrap_or_default().into(),
        origin: a.meta.recipe.as_ref().map(|r| r.label()).unwrap_or_default().into(),
    }
}

/// What an agent was last asked, which is what its row is about now.
///
/// The row used to carry the conversation's first prompt for ever. The Lens's conversation with a
/// mind is one long-lived agent, so every request a person made there sat under whatever they
/// asked first, hours before: on VM 520, "Release check: reply with exactly one word, READY."
/// over a town model, a daily briefing and a game (#234, #246).
fn latest_request<'a>(turns: &'a [Turn], first: &'a str) -> &'a str {
    turns.iter().rev().map(|t| t.prompt.trim()).find(|p| !p.is_empty()).unwrap_or(first)
}

/// "2m" while it works, "21:02" once it has stopped.
fn since(a: &Agent) -> String {
    if a.state.live() {
        duration(now().saturating_sub(a.since))
    } else {
        clock(a.since)
    }
}

fn duration(secs: u64) -> String {
    match secs {
        0..=4 => "just now".into(),
        5..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m", secs / 60),
        _ => format!("{}h {}m", secs / 3600, (secs % 3600) / 60),
    }
}

/// Local time today, the date before today.
fn clock(unix: u64) -> String {
    use chrono::TimeZone;
    let Some(at) = chrono::Local.timestamp_opt(unix as i64, 0).single() else { return String::new() };
    if at.date_naive() == chrono::Local::now().date_naive() {
        at.format("%H:%M").to_string()
    } else {
        at.format("%b %-d, %H:%M").to_string()
    }
}

fn header_of(a: &Agent, seen: &Seen) -> AgentHeaderData {
    let harness = a.meta.id.harness();
    let attached = seen.minds.iter().any(|(id, _, _)| id == harness);
    let builtin = harness == crate::wire::harness::BUILTIN_ID;
    let reachable = launch::reachable(&a.meta.id, &seen.live(), &seen.attached());
    let turn_open = a.open_turn().is_some();
    let gone = a.state == State::HarnessGone;
    let send_hint = if gone {
        "its harness is gone".to_string()
    } else if !attached {
        format!("{} is not attached right now", a.meta.mind)
    } else if !reachable {
        "this conversation has ended — start a new agent to go on".to_string()
    } else if turn_open {
        format!("{} is working — wait, or Stop it", a.meta.mind)
    } else {
        String::new()
    };
    let note = if a.meta.conversations || builtin {
        String::new()
    } else {
        format!("{} holds one conversation at a time — the same one the Lens talks to.", a.meta.mind)
    };
    AgentHeaderData {
        id: a.meta.id.0.as_str().into(),
        mind: a.meta.mind.as_str().into(),
        title: latest_request(&a.turns, &a.meta.title).into(),
        state: a.state.key().into(),
        label: a.state.label().into(),
        since: since(a).into(),
        status: a.status.as_str().into(),
        note: note.into(),
        can_send: reachable && !turn_open && !gone,
        send_hint: send_hint.into(),
        can_stop: attached && !builtin && a.busy(),
    }
}

fn details_of(a: &Agent, d: Details) -> AgentDetailsData {
    let model = if !a.meta.model.is_empty() { a.meta.model.clone() } else { d.usage.model.clone() };
    let calls = if d.failed_calls > 0 { format!("{} ({} failed)", d.calls, d.failed_calls) } else { d.calls.to_string() };
    let failed_commands = d.commands.iter().filter(|(_, _, s)| *s == CallState::Failed).count();
    let commands = match (d.commands.len(), failed_commands) {
        (0, _) => "none run by the shell".to_string(),
        (n, 0) => n.to_string(),
        (n, f) => format!("{n} ({f} failed)"),
    };
    let command_lines: Vec<String> = d
        .commands
        .iter()
        .rev()
        .take(6)
        .rev()
        .map(|(line, exit, state)| {
            let how = match (state, exit) {
                (CallState::Running, _) => "running".to_string(),
                (_, Some(code)) => format!("exit {code}"),
                (s, None) => s.key().to_string(),
            };
            format!("{how:>8}  {}", one_line(line, 60))
        })
        .collect();
    let mut file_lines: Vec<String> = d.files.iter().take(6).cloned().collect();
    if d.files.len() > 6 {
        file_lines.push(format!("and {} more", d.files.len() - 6));
    }
    let tokens = if d.usage.reported {
        format!("{} in · {} out", thousands(d.usage.input_tokens), thousands(d.usage.output_tokens))
    } else {
        "not reported".to_string()
    };
    let cost = if d.usage.cost_usd > 0.0 { format!("${:.2}", d.usage.cost_usd) } else { String::new() };
    // The catalog role it was started as, and what that role may touch — held on every door.
    // The reach reads as words a person reads (#212), with the patterns the doors enforce one
    // click away; a session saved before the words were kept has only the patterns, and then
    // there is nothing further to open.
    let (reach, reach_patterns) = match a.meta.role.as_ref() {
        Some(r) if !r.reach_words.is_empty() => (r.reach_words.clone(), r.reach.clone()),
        Some(r) => (r.reach.clone(), String::new()),
        None => (String::new(), String::new()),
    };
    AgentDetailsData {
        mind: a.meta.mind.as_str().into(),
        model: model.into(),
        since: clock(a.meta.started).into(),
        turns: d.turns.to_string().into(),
        calls: calls.into(),
        commands: commands.into(),
        command_lines: command_lines.join("\n").into(),
        files: if d.files.is_empty() { "none named".into() } else { d.files.len().to_string().into() },
        file_lines: file_lines.join("\n").into(),
        approvals: format!("{} asked · {} answered", d.approvals_asked, d.approvals_answered).into(),
        tokens: tokens.into(),
        cost: cost.into(),
        refused: if d.refused > 0 { format!("{} events", d.refused).into() } else { "".into() },
        // What was refused, and why (#212): the store's newest lines, oldest first.
        refused_lines: d.refusals.join("\n").into(),
        role: a.meta.role.as_ref().map(|r| r.name.clone()).unwrap_or_default().into(),
        reach: reach.into(),
        reach_patterns: reach_patterns.into(),
        basis: "Commands, files and approvals count only what the shell itself ran or asked. Calls \
                include what the harness reported."
            .into(),
    }
}

fn thousands(n: u64) -> String {
    if n >= 10_000 {
        format!("{}k", n / 1000)
    } else if n >= 1000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        n.to_string()
    }
}

fn one_line(text: &str, max: usize) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    format!("{}…", flat.chars().take(max - 1).collect::<String>())
}

/// A session as the screen draws it, newest last. `pending` is the shell's approval store's
/// waiting requests: an approval item is drawn from there, never from the session.
fn items_of(a: &Agent, expanded: &HashSet<String>, pending: &[crate::approvals::Card]) -> Vec<AgentItemData> {
    let mut out = Vec::new();
    let from = a.turns.len().saturating_sub(SHOWN_TURNS);
    if from > 0 {
        out.push(AgentItemData {
            kind: "note".into(),
            key: "earlier".into(),
            text: format!("{from} earlier turns are kept in its saved session, not shown here.").into(),
            ..Default::default()
        });
    }
    for turn in &a.turns[from..] {
        if !turn.prompt.is_empty() {
            out.push(AgentItemData {
                kind: "prompt".into(),
                key: format!("t{}", turn.n).into(),
                text: turn.prompt.as_str().into(),
                // The first prompt says who sent it (#194) — the row's own attribution. The
                // store keeps no sender per turn, so later prompts stay "you", as before.
                sent_by: if turn.n == 1 { first_sender(&a.meta) } else { String::new() }.into(),
                ..Default::default()
            });
        }
        for (j, item) in turn.items.iter().enumerate() {
            let key = format!("t{}.{}", turn.n, j);
            let open = expanded.contains(&key);
            match item {
                Item::Text(text) => {
                    let text = text.last(TEXT_BYTES);
                    let text = text.trim_matches('\n');
                    if text.trim().is_empty() {
                        continue;
                    }
                    out.extend(prose_of(&key, text));
                }
                Item::Thinking(text) => {
                    let text = text.last(TEXT_BYTES);
                    let shown = if open { text.trim().to_string() } else { one_line(&text, 160) };
                    out.push(AgentItemData {
                        kind: "thinking".into(),
                        key: key.into(),
                        text: shown.into(),
                        expanded: open,
                        ..Default::default()
                    });
                }
                Item::Note(note) => {
                    out.push(AgentItemData { kind: "note".into(), key: key.into(), text: note.as_str().into(), ..Default::default() })
                }
                Item::Card(card) => out.push(card_of(card, key, open)),
                Item::Approval(approval) => out.push(approval_of(a, approval, key, pending)),
            }
        }
    }
    out
}

/// Whose voice a pane's first prompt speaks with — the same attribution its row shows: the
/// recipe that sent it ("Council recipe"), else the agent that started this one (its id). "" for
/// one the person typed, which the pane draws as "you".
fn first_sender(meta: &agents::AgentMeta) -> String {
    if let Some(recipe) = &meta.recipe {
        return recipe.label();
    }
    meta.parent.as_ref().map(|p| p.0.clone()).unwrap_or_default()
}

/// One block of the mind's text as the pane draws it, one item per block: a paragraph or a list
/// with its bold, italic, inline code and links (`StyledText`), a heading, or a code block. The
/// blocks are the Lens's own reading of the text (`crate::markdown`), so the two views of one
/// answer cannot disagree about where a list or a fence begins.
///
/// Read again from the whole text on every redraw, so an answer still arriving is drawn as far as
/// it has got — a fence still open is a code block, a `**` still open is two asterisks — and the
/// blocks before the last keep their keys, so the view extends in place.
fn prose_of(key: &str, text: &str) -> Vec<AgentItemData> {
    crate::markdown::parse_blocks(text)
        .iter()
        .enumerate()
        .map(|(n, block)| {
            let kind = match block.block_type {
                kind @ ("heading" | "bullet" | "code") => kind,
                // A trail line the feed did not take out is still words, not a card: a card is
                // only ever made from the store's own `Card`.
                _ => "text",
            };
            AgentItemData {
                kind: "text".into(),
                key: format!("{key}.{n}").into(),
                block: kind.into(),
                text: block.text.as_str().into(),
                styled: crate::markdown::styled(block),
                ..Default::default()
            }
        })
        .collect()
}

/// What the list says when the tab it is on has no one in it: what is true of the others. "No
/// agents yet" only when there are none at all — an empty Active tab beside six finished agents
/// said there were none, with one of them open in the middle column (#190).
fn empty_note(tab: Tab, counts: [usize; 4]) -> String {
    let [active, _, complete, all] = counts;
    if all == 0 {
        return "No agents yet. New agent starts one; a question in the Lens to an attached mind \
                shows up here too."
            .to_string();
    }
    // Every agent is either active (working, waiting, or idle between prompts) or complete.
    let (going, done) = (format!("{active} active"), format!("{complete} complete"));
    match tab {
        Tab::Active => format!("Nothing running. {done}."),
        Tab::NeedsYou if active > 0 => format!("Nothing is waiting on you. {going}."),
        Tab::NeedsYou => format!("Nothing is waiting on you. {done}."),
        Tab::Complete => format!("Nothing has finished yet. {going}."),
        // All holds every agent, so it is empty only when there are none, above.
        Tab::All => format!("{going}, {done}."),
    }
}

/// An approval, as the pane draws it: the shell's own card — the Lens's component, filled from the
/// shell's approval store under the one request id — while the request waits; the line it left
/// once it is over.
///
/// The buttons are drawn only when all three hold: the session item is one the shell made
/// (`approval_asked`, never an event or the agent's text), the shell's store says the request is
/// still waiting, and that request was asked for THIS agent — its token's, not its words'. Anything
/// else draws a line with no buttons.
fn approval_of(a: &Agent, approval: &Approval, key: String, pending: &[crate::approvals::Card]) -> AgentItemData {
    let live = (approval.outcome == ApprovalOutcome::Pending)
        .then(|| {
            pending.iter().find(|c| {
                c.id == approval.request
                    && c.status == crate::approvals::Status::Pending
                    && c.verified.agent == a.meta.id.0
            })
        })
        .flatten();
    let card = match live {
        Some(card) => crate::ApprovalRequest { on_behalf: a.meta.on_behalf().into(), ..crate::control_approvals::row_for(card.clone()) },
        None => {
            let (app, action) = approval.what.split_once('.').unwrap_or((approval.what.as_str(), ""));
            let (decision, record) = match approval.outcome {
                // Answered or taken back a moment ago, and not yet settled here: the approval
                // store redraws on its own tick and this follows.
                ApprovalOutcome::Pending => ("asked", format!("Asked you: {}", approval.what)),
                outcome => (outcome.key(), approval.record.clone()),
            };
            ApprovalRequest {
                id: approval.request.as_str().into(),
                agent: a.meta.id.0.as_str().into(),
                app: app.into(),
                action: action.into(),
                decision: decision.into(),
                record: record.into(),
                ..Default::default()
            }
        }
    };
    AgentItemData {
        kind: "approval".into(),
        key: key.into(),
        text: approval.what.as_str().into(),
        approval: card,
        ..Default::default()
    }
}

/// One call, as the card draws it.
fn card_of(c: &Card, key: String, open: bool) -> AgentItemData {
    let call = c.as_call();
    let has_output = !c.output.bytes.is_empty();
    let live = c.running() && has_output;
    let lines = c.output.lines();
    let total = c.output.bytes.total();

    let mut badge = vec![c.provenance.key().to_string()];
    if let Some(code) = c.exit_code {
        badge.push(format!("exit {code}"));
    }

    let mut explain: Vec<String> = Vec::new();
    if !c.summary.is_empty() {
        explain.push(c.summary.clone());
    }
    match (c.state, c.mark) {
        (_, Some(Mark::EndWithoutStart)) => {
            explain.push("The harness said this call ended, and never said it started.".into())
        }
        (_, Some(Mark::OutputWithoutStart)) => {
            explain.push("Output arrived for a call the harness never said it started.".into())
        }
        (CallState::Untold, None) => {
            explain.push("Read from the mind's own text. It did not say how the call went.".into())
        }
        (CallState::Interrupted, None) => {
            explain.push("Still open when its turn ended; it never said how it went.".into())
        }
        _ => {}
    }
    if c.provenance == Provenance::Verified && c.is_command() && c.ended.is_some() {
        explain.push("Run by the shell itself; the exit code is the process's own.".into());
    }
    if let Some(mark) = c.mark {
        badge.push(mark.label().into());
    }

    // What ToolCallCard shows when opened is `call.output`; a single space while folded only tells
    // it there is something to open.
    let (output, card_output) = match c.output.kind {
        OutputKind::Text if open => (String::new(), c.output.plain(OPEN_BYTES)),
        OutputKind::Text if live => (c.output.tail_lines(LIVE_LINES), " ".to_string()),
        OutputKind::Text | OutputKind::Terminal if has_output && !open => (String::new(), " ".to_string()),
        _ => (String::new(), String::new()),
    };
    let more = match c.output.kind {
        OutputKind::Text if open && total > OPEN_BYTES as u64 => format!("the last {} of {}", bytes(OPEN_BYTES as u64), bytes(total)),
        OutputKind::Text if live && lines > LIVE_LINES as u64 => format!("the last {LIVE_LINES} of {lines} lines"),
        OutputKind::Terminal if (open || live) && lines > 0 => {
            format!("{lines} line{} in all", if lines == 1 { "" } else { "s" })
        }
        _ => String::new(),
    };
    let (runs, rows) = if c.output.kind == OutputKind::Terminal && (open || live) {
        let runs: Vec<AgentRunData> = c
            .output
            .runs()
            .into_iter()
            .map(|r| AgentRunData {
                text: r.text.into(),
                row: r.row as i32,
                col: r.col as i32,
                columns: r.width as i32,
                fg: slint::Color::from_rgb_u8(r.fg.0, r.fg.1, r.fg.2),
                bg: slint::Color::from_rgb_u8(r.bg.0, r.bg.1, r.bg.2),
                bold: r.bold,
            })
            .collect();
        let rows = runs.iter().map(|r| r.row + 1).max().unwrap_or(1);
        (ModelRc::new(VecModel::from(runs)), rows)
    } else {
        // The empty model compares equal to itself, so a card with nothing to draw is not
        // re-sent to the view on every redraw.
        (ModelRc::default(), 0)
    };

    AgentItemData {
        kind: "card".into(),
        key: key.into(),
        text: Default::default(),
        sent_by: Default::default(),
        call: ToolCallData {
            name: call.name.as_str().into(),
            target: call.target.as_str().into(),
            summary: call.summary().into(),
            arguments: call.detail().into(),
            status: c.state.status().into(),
            output: card_output.into(),
        },
        badge: badge.join(" · ").into(),
        explain: explain.join("\n").into(),
        expanded: open,
        live,
        output_kind: match c.output.kind {
            OutputKind::None => "",
            OutputKind::Text => "text",
            OutputKind::Terminal => "terminal",
        }
        .into(),
        output: output.into(),
        more: more.into(),
        can_open_all: has_output && (c.output.kind == OutputKind::Terminal || total > OPEN_BYTES as u64),
        runs,
        rows,
        approval: Default::default(),
        block: Default::default(),
        styled: Default::default(),
    }
}

// ── Acts ──────────────────────────────────────────────────────────

/// Close an agent: asked first when it is still working; then stopped, its window closed, and
/// taken off the list with its saved session.
fn close(ui: &App, state: &Shared, agent: AgentId, confirmed: bool) {
    let g = ui.global::<AgentsState>();
    let busy = agents::store().read(|s| s.agent(&agent).is_some_and(Agent::busy));
    if busy && !confirmed {
        g.set_confirm_close(agent.0.as_str().into());
        return;
    }
    g.set_confirm_close("".into());
    if busy {
        if let Err(why) = launch::stop(&agent) {
            tracing::info!(agent = %agent, %why, "closing an agent that could not be stopped");
        }
    }
    agents::store().remove_agent(&agent);
    {
        let mut st = state.borrow_mut();
        if let Some(popped) = st.windows.remove(&agent) {
            let _ = popped.window.hide();
        }
        if st.selected.as_ref() == Some(&agent) {
            st.selected = None;
        }
    }
    refresh(ui, state, true);
}

/// The title an agent's window carries. labwc and the taskbar know a window by it.
fn window_title(mind: &str, title: &str) -> String {
    one_line(&format!("{}{mind} · {title}", agents::WINDOW_TITLE_PREFIX), 90)
}

/// Open an agent in a window of its own — or, when it already has one, bring that one forward.
fn pop_out(ui: &App, state: &Shared, agent: AgentId) {
    let known = agents::store().read(|s| s.agent(&agent).map(|a| (a.meta.mind.clone(), a.meta.title.clone())));
    let Some((mind, title)) = known else {
        notice(&ui.global::<AgentsState>(), Err("That agent is no longer in the list.".into()));
        return;
    };

    // One window per agent: a second Pop out raises the first.
    {
        let st = state.borrow();
        if let Some(popped) = st.windows.get(&agent) {
            popped.closed.set(false);
            let _ = popped.window.show();
            popped.window.window().set_minimized(false);
            let title = popped.title.clone();
            // wlrctl is a process; the UI thread does not wait on it.
            std::thread::spawn(move || {
                crate::windows::present(&title);
            });
            return;
        }
    }

    let window = match AgentWindow::new() {
        Ok(window) => window,
        Err(e) => {
            notice(&ui.global::<AgentsState>(), Err(format!("Could not open a window for this agent: {e}")));
            return;
        }
    };
    // A window of its own, not a second desktop. The shell runs under SLINT_FULLSCREEN=1 so that
    // its own window covers the display, and every window this process creates reads the same
    // variable: the pop-out came up fullscreen, with no title bar and nothing to close, move or
    // resize it by (#231). Said here, for this window, rather than by clearing the variable,
    // which the shell's own window still needs.
    window.window().set_fullscreen(false);
    let title = window_title(&mind, &title);
    window.set_agent_title(title.as_str().into());
    sync_theme(ui, &window);
    let surface = Surface::new();
    {
        let g = window.global::<AgentsState>();
        g.set_popped(true);
        g.set_items(ModelRc::from(surface.items.clone()));
    }
    wire_window(&window, state, &agent, &ui.as_weak());
    let closed = Rc::new(Cell::new(false));
    {
        let closed = closed.clone();
        window.window().on_close_requested(move || {
            closed.set(true);
            slint::CloseRequestResponse::HideWindow
        });
    }
    let seen = Seen::now();
    let mut st = state.borrow_mut();
    let popped = st.windows.entry(agent.clone()).or_insert(Popped { window, surface, closed, title });
    agents::store().read(|s| draw(&popped.window.global::<AgentsState>(), &mut popped.surface, s, Some(&agent), &seen, true));
    if let Err(e) = popped.window.show() {
        st.windows.remove(&agent);
        drop(st);
        notice(&ui.global::<AgentsState>(), Err(format!("Could not show a window for this agent: {e}")));
    }
}

/// The acts a popped-out window has: fold and open, Stop, say more, open all output, and Allow and
/// Deny on an approval card — which go to the shell's own, like the screen's.
fn wire_window(window: &AgentWindow, state: &Shared, agent: &AgentId, shell: &slint::Weak<App>) {
    let g = window.global::<AgentsState>();
    forward_approvals(&g, shell);
    let weak = window.as_weak();
    g.on_toggle({
        let (state, agent) = (state.clone(), agent.clone());
        move |key, open| {
            let mut st = state.borrow_mut();
            let Some(popped) = st.windows.get_mut(&agent) else { return };
            popped.surface.toggle(&key, open);
            let seen = Seen::now();
            agents::store().read(|s| draw(&popped.window.global::<AgentsState>(), &mut popped.surface, s, Some(&agent), &seen, false));
        }
    });
    g.on_stop({
        let weak = weak.clone();
        move |agent| {
            if let Some(window) = weak.upgrade() {
                notice(&window.global::<AgentsState>(), launch::stop(&AgentId(agent.to_string())));
            }
        }
    });
    g.on_send({
        let weak = weak.clone();
        move |agent, text| {
            if let Some(window) = weak.upgrade() {
                notice(&window.global::<AgentsState>(), launch::send(&AgentId(agent.to_string()), &text));
            }
        }
    });
    g.on_open_all({
        let (weak, agent) = (weak.clone(), agent.clone());
        move |key| {
            if let Some(window) = weak.upgrade() {
                notice(&window.global::<AgentsState>(), open_all(&agent, &key));
            }
        }
    });
    g.on_dismiss_notice({
        let weak = weak.clone();
        move || {
            if let Some(window) = weak.upgrade() {
                window.global::<AgentsState>().set_notice("".into());
            }
        }
    });
}

/// Allow, Allow for this session and Deny on a pane's approval card, handed to the shell's own
/// callbacks — the ones the Lens's card and the overlay call, and the only place a grant is made
/// (`control_approvals::wire`). Nothing here grants or denies: it presses the same button.
fn forward_approvals(g: &AgentsState, shell: &slint::Weak<App>) {
    g.on_approval_allow({
        let shell = shell.clone();
        move |id| {
            if let Some(ui) = shell.upgrade() {
                ui.invoke_approval_allow(id);
            }
        }
    });
    g.on_approval_allow_session({
        let shell = shell.clone();
        move |id| {
            if let Some(ui) = shell.upgrade() {
                ui.invoke_approval_allow_session(id);
            }
        }
    });
    g.on_approval_deny({
        let shell = shell.clone();
        move |id| {
            if let Some(ui) = shell.upgrade() {
                ui.invoke_approval_deny(id);
            }
        }
    });
}

/// Put one agent on the Agents screen: selected, under a tab that lists it, the screen shown.
/// The Lens's "open in Agents", a notification's Open, and `show_agent` all come here.
fn show_agent(ui: &App, state: &Shared, agent: AgentId) {
    let known = agents::store().read(|s| s.agent(&agent).map(|a| a.state));
    let g = ui.global::<AgentsState>();
    let Some(agent_state) = known else {
        notice(&g, Err(format!("`{agent}` is no longer in the list.")));
        return;
    };
    {
        let mut st = state.borrow_mut();
        if !st.tab.holds(agent_state) {
            st.tab = Tab::All;
            g.set_tab(Tab::All.key().into());
        }
        st.order.clear();
        st.selected = Some(agent);
    }
    ui.set_current_screen(SCREEN);
    ui.invoke_navigate(SCREEN);
    refresh(ui, state, true);
}

// ── Telling the person ────────────────────────────────────────────

/// Something the person should hear about an agent they are not looking at.
#[derive(Clone, Debug, PartialEq)]
enum Notice {
    /// Its turn ended — finished, or not.
    Finished { agent: AgentId, mind: String, title: String, ok: bool },
    /// A card of its waits on the person: an approval, or a command at a prompt.
    NeedsYou { agent: AgentId, mind: String, what: String },
}

impl Notice {
    fn agent(&self) -> &AgentId {
        match self {
            Notice::Finished { agent, .. } | Notice::NeedsYou { agent, .. } => agent,
        }
    }

    /// Said as the desktop, in the desktop's words. The title quotes the task, and nothing the
    /// agent wrote goes in: a notification from `Yantrik` must not carry a mind's sentences as
    /// though the desktop had said them (#139).
    fn notification(&self) -> yantrik_app_runtime::notify::Notification {
        use yantrik_app_runtime::notify::{Level, Notification};
        let (title, body) = match self {
            Notice::Finished { mind, title, ok: true, .. } => (
                format!("{mind} finished: \u{201c}{}\u{201d}", one_line(title, 60)),
                "Its turn is done. Open it to read what it said and what it ran.".to_string(),
            ),
            Notice::Finished { mind, title, ok: false, .. } => (
                format!("{mind} could not finish: \u{201c}{}\u{201d}", one_line(title, 60)),
                "Its turn ended without finishing. Open it to see where it stopped.".to_string(),
            ),
            Notice::NeedsYou { mind, what, .. } => (format!("{mind} needs you"), what.clone()),
        };
        Notification::new("Yantrik", title)
            .body(body)
            // News, not a question with a deadline: Do Not Disturb holds it, like any other.
            .urgency(Level::Normal)
            // The shell presses `show_agent` on its own surface for this, as Download Manager's
            // "Open folder" is pressed on its.
            .action_with("show_agent", "Open", serde_json::json!({ "agent": self.agent().0 }))
    }
}

/// Watches the store, tick by tick, for turns that ended and cards that began waiting.
///
/// The first look only learns what is already there — a session loaded from disk is history, not
/// news. After that an agent's newest turn ending, or a new request or waiting command of its,
/// is a [`Notice`] once.
#[derive(Default)]
struct Watch {
    primed: bool,
    /// Each agent's newest ended turn, as `(turn, ended at)`.
    ended: HashMap<AgentId, (u64, u64)>,
    /// What each agent was already known to be waiting on: request ids and job ids.
    waiting: HashMap<AgentId, BTreeSet<String>>,
}

impl Watch {
    /// `waiting_jobs` is the agent terminal's commands sitting at a prompt: `(agent, job)`.
    fn changes(&mut self, s: &Store, waiting_jobs: &[(AgentId, String)]) -> Vec<Notice> {
        let mut out = Vec::new();
        for a in s.agents() {
            let id = &a.meta.id;
            if let Some(turn) = a.turns.last() {
                if let Some(at) = turn.ended {
                    let before = self.ended.insert(id.clone(), (turn.n, at));
                    // A turn with no prompt is the shell's own account of something outside any
                    // turn, and a turn the person stopped needs no telling.
                    if self.primed && before != Some((turn.n, at)) && !turn.prompt.is_empty() && !stopped(turn) {
                        out.push(Notice::Finished {
                            agent: id.clone(),
                            mind: a.meta.mind.clone(),
                            title: a.meta.title.clone(),
                            ok: turn.ok != Some(false),
                        });
                    }
                }
            }
            let now: BTreeSet<String> = a
                .pending_approvals
                .iter()
                .cloned()
                .chain(waiting_jobs.iter().filter(|(agent, _)| agent == id).map(|(_, job)| job.clone()))
                .collect();
            let before = self.waiting.insert(id.clone(), now.clone()).unwrap_or_default();
            let fresh: Vec<&String> = now.difference(&before).collect();
            if self.primed && !fresh.is_empty() {
                let asked = fresh.iter().find_map(|request| {
                    a.turns.iter().rev().flat_map(|t| t.items.iter()).find_map(|item| match item {
                        Item::Approval(ap) if &&ap.request == request => Some(ap.what.clone()),
                        _ => None,
                    })
                });
                let what = match asked {
                    Some(what) => format!("It is asking to be allowed {what}. Allow or Deny on its card."),
                    None => "One of its commands is waiting for input; answer in its card.".to_string(),
                };
                out.push(Notice::NeedsYou { agent: id.clone(), mind: a.meta.mind.clone(), what });
            }
        }
        self.primed = true;
        out
    }
}

/// Whether a turn ended because the person stopped it.
fn stopped(turn: &crate::agents::model::Turn) -> bool {
    turn.items.iter().any(|item| {
        matches!(item, Item::Note(note) if note.starts_with("Stop asked") || note.contains(yantrik_harness::host::STOPPED))
    })
}

/// Send what the person should hear, except about what they are already looking at: the agent
/// selected on the Agents screen, one in a window of its own, or — with the Lens open — the
/// Lens's own mind, whose answer and cards are in front of them there. A "needs you" is also held
/// while the Lens is open at all: its approval card is in the Lens, and a toast would land on it.
fn tell_the_person(ui: &App, state: &Shared, watch: &mut Watch) {
    let waiting_jobs: Vec<(AgentId, String)> = crate::control_agent_terminal::running_jobs()
        .into_iter()
        .filter(|(_, job)| job["waiting_for_input"] == true)
        .map(|(agent, job)| (agent, job["job"].as_str().unwrap_or_default().to_string()))
        .collect();
    let notices = agents::store().read(|s| watch.changes(s, &waiting_jobs));
    if notices.is_empty() {
        return;
    }
    let lens_open = ui.get_lens_open();
    let lens_agent = crate::wire::harness::host().map(|h| feed::main_agent(&h.active_id()));
    let st = state.borrow();
    for notice in notices {
        let agent = notice.agent();
        let on_screen = ui.get_current_screen() == SCREEN && st.selected.as_ref() == Some(agent);
        let in_window = st.windows.get(agent).is_some_and(|p| !p.closed.get());
        let in_lens = lens_open && (lens_agent.as_ref() == Some(agent) || matches!(notice, Notice::NeedsYou { .. }));
        if on_screen || in_window || in_lens {
            continue;
        }
        yantrik_app_runtime::notify::send(notice.notification());
    }
}

/// A window has its own copy of every global, so it takes the shell's theme — dark or light, the
/// accent, any community theme — from the shell, and keeps taking it.
fn sync_theme(ui: &App, window: &AgentWindow) {
    macro_rules! copy {
        ($global:ty, $get:ident, $set:ident) => {{
            let want = ui.global::<$global>().$get();
            if window.global::<$global>().$get() != want {
                window.global::<$global>().$set(want);
            }
        }};
    }
    copy!(ThemeMode, get_dark, set_dark);
    copy!(AccentPreset, get_index, set_index);
    copy!(ThemeOverrides, get_enabled, set_enabled);
    copy!(ThemeOverrides, get_bg_deep_override, set_bg_deep_override);
    copy!(ThemeOverrides, get_bg_surface_override, set_bg_surface_override);
    copy!(ThemeOverrides, get_bg_card_override, set_bg_card_override);
    copy!(ThemeOverrides, get_bg_elevated_override, set_bg_elevated_override);
    copy!(ThemeOverrides, get_amber_override, set_amber_override);
    copy!(ThemeOverrides, get_cyan_override, set_cyan_override);
    copy!(ThemeOverrides, get_text_primary_override, set_text_primary_override);
    copy!(ThemeOverrides, get_text_secondary_override, set_text_secondary_override);
    copy!(ThemeOverrides, get_text_dim_override, set_text_dim_override);
    copy!(ThemeOverrides, get_accent_override, set_accent_override);
}

/// All of a call's output, in the Editor: written to a file beside the sessions and opened there.
fn open_all(agent: &AgentId, key: &str) -> Result<(), String> {
    let Some((turn, index)) = parse_key(key) else { return Err("That is not a call.".into()) };
    let found = agents::store().read(|s| {
        let a = s.agent(agent)?;
        let turn = a.turns.iter().find(|t| t.n == turn)?;
        match turn.items.get(index)? {
            Item::Card(c) => Some((c.call.clone(), c.as_call().summary(), c.exit_code, c.output.all())),
            _ => None,
        }
    });
    let Some((call, summary, exit, text)) = found else {
        return Err("That call is no longer in the session.".into());
    };
    let dir = agents::dir().join("output");
    std::fs::create_dir_all(&dir).map_err(|e| format!("Could not make {}: {e}", dir.display()))?;
    let name = format!("{}-{}.txt", agent.0, call)
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '_' })
        .collect::<String>();
    let path = dir.join(name);
    let exit = exit.map(|c| format!(" · exit {c}")).unwrap_or_default();
    std::fs::write(&path, format!("# {summary}{exit}\n\n{text}"))
        .map_err(|e| format!("Could not write {}: {e}", path.display()))?;
    let path = path.to_string_lossy().into_owned();
    crate::wire::dock::spawn_app_with_args("editor", "yantrik-text-editor", &[&path]);
    Ok(())
}

/// `t12.3` → turn 12, item 3.
fn parse_key(key: &str) -> Option<(u64, usize)> {
    let (turn, index) = key.strip_prefix('t')?.split_once('.')?;
    Some((turn.parse().ok()?, index.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn read(relative: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
    }

    /// A screen is in five places, and a screen missing from any one of them is a screen some door
    /// cannot reach: `show_screen` refuses it, `open_app` does not know it, the launcher has no tile,
    /// the taskbar vanishes on it, or `describe` never says it exists. Problem reports shipped
    /// without the taskbar; this checks every place at once.
    #[test]
    fn the_agents_screen_is_registered_everywhere_a_screen_must_be() {
        // app.slint draws it at this id, as AgentsScreen, and keeps the taskbar on it.
        let app = read("../yantrik-ui-slint/ui/app.slint");
        let branch = format!("if current-screen == {SCREEN} : WindowFrame");
        let at = app.find(&branch).expect("app.slint draws the Agents screen at SCREEN");
        assert!(app[at..].lines().take(40).any(|l| l.trim_start().starts_with("AgentsScreen {")), "screen {SCREEN} draws AgentsScreen");
        let taskbar = app.lines().find(|l| l.contains(": Rectangle") && l.contains("current-screen == 1 ||")).expect("the taskbar's condition");
        assert!(taskbar.contains(&format!("current-screen == {SCREEN}")), "the taskbar shows on the Agents screen: {taskbar}");
        // Its window and its global are exported, or the shell cannot pop an agent out or fill one.
        assert!(app.contains("export { AgentWindow, AgentsState"), "AgentWindow and AgentsState are exported from app.slint");

        // `show_screen agents`, and describe names it.
        assert_eq!(crate::control::screen_name(SCREEN), "agents");
        let control = read("src/control.rs");
        let control = control.split("#[cfg(test)]").next().unwrap();
        assert!(control.contains(".with(\"agents\", crate::agents::for_describe())"), "describe shell lists the agents");
        assert!(control.contains("problems, agents"), "show_screen's description offers agents");

        // `open_app agents`, the listing a caller reads, and what it is for.
        use crate::wire::dock::{availability, route, Availability, Launch};
        assert_eq!(route("agents"), Some(Launch::Screen(SCREEN)));
        assert_eq!(availability("agents", &[]), Availability::Ready);
        let listed = crate::wire::dock::openable();
        let entry = listed.iter().find(|a| a["name"] == "agents").expect("open_app lists agents");
        assert!(entry["for"].as_str().is_some_and(|f| f.contains("agent")), "{entry}");

        // A launcher tile, with an icon of its own.
        assert!(crate::apps::builtin_apps().iter().any(|e| e.app_id == "agents" && e.name == "Agents"));
        let icons = read("../yantrik-ui-kit/slint/icon.slint");
        assert!(icons.contains("id == \"agents\""), "Icons.app knows agents");
    }

    #[test]
    fn a_popped_out_window_is_known_as_an_agent_by_its_title() {
        let title = window_title("pi", "tidy the files in the terminal folder");
        assert!(title.starts_with(agents::WINDOW_TITLE_PREFIX), "{title}");
        // The window list would otherwise take "files" or "terminal" from the task and mark
        // Files or Terminal as open.
        assert_eq!(crate::windows::derive_app_id(&title), "agents");
    }

    #[test]
    fn a_card_folded_is_one_line_and_open_is_everything() {
        use crate::agents::model::{Stream, Card};
        let mut card = Card::new("j1", "agent_run", "", serde_json::json!({"command": "fdupes -r ~/Pictures"}), Provenance::Verified, 0);
        for n in 0..20 {
            card.output.push(Stream::Stdout, format!("line {n}\n").as_bytes());
        }
        // Running: the line, and the live tail under it.
        let running = card_of(&card, "t1.0".into(), false);
        assert_eq!(running.call.status, "running");
        assert!(running.live);
        assert_eq!(running.output.lines().count(), LIVE_LINES);
        assert_eq!(running.more, format!("the last {LIVE_LINES} of 20 lines"));
        // Ended: one line with the exit code on it, nothing under it until opened.
        card.state = CallState::Ok;
        card.exit_code = Some(0);
        card.ended = Some(1);
        let folded = card_of(&card, "t1.0".into(), false);
        assert!(!folded.live);
        assert_eq!(folded.badge, "verified · exit 0");
        assert_eq!(folded.call.summary, r#"agent_run command="fdupes -r ~/Pictures""#);
        assert_eq!(folded.output, "");
        let open = card_of(&card, "t1.0".into(), true);
        assert!(open.call.output.contains("line 0\n") && open.call.output.contains("line 19"));
        assert!(open.call.arguments.contains("fdupes"), "the arguments in full");
    }

    fn pending_card(id: &str, agent: &str) -> crate::approvals::Card {
        let purpose = "Run one command line in a fresh terminal of your own.";
        crate::approvals::Card {
            id: id.into(),
            requester: "pi 0.87".into(),
            verified: crate::approvals::Verified {
                line: "pi --mode rpc (pid 4242)".into(),
                agent: agent.into(),
                ..Default::default()
            },
            app: "shell".into(),
            action: "agent_run".into(),
            grade: "sensitive".into(),
            purpose: purpose.into(),
            summary: crate::approvals::summary_of(purpose),
            args: vec!["command: rm -rf build".into()],
            target: String::new(),
            explained: String::new(),
            warning: String::new(),
            can_session: true,
            status: crate::approvals::Status::Pending,
            record: String::new(),
            age_secs: 4,
        }
    }

    fn approvals_drawn(store: &Store, agent: &AgentId, pending: &[crate::approvals::Card]) -> Vec<AgentItemData> {
        items_of(store.agent(agent).unwrap(), &HashSet::new(), pending).into_iter().filter(|i| i.kind == "approval").collect()
    }

    /// Design decision 4: the card in the pane is the shell's, with Allow and Deny bound to the one
    /// request id, and only while the shell's own store says that request is waiting for THIS agent.
    #[test]
    fn an_approval_is_the_shells_card_in_the_pane_of_the_agent_its_token_named() {
        let mut s = Store::new();
        let pi = AgentId("pi:c-7f3a91".into());
        s.open_turn(&pi, "clean the build");
        s.approval_asked(&pi, "appr-7", "shell.agent_run");

        // Waiting, for pi: the whole card, the Lens's own data, buttons live (no decision yet).
        let drawn = approvals_drawn(&s, &pi, &[pending_card("appr-7", "pi:c-7f3a91")]);
        assert_eq!(drawn.len(), 1);
        let card = &drawn[0].approval;
        assert_eq!((card.id.as_str(), card.decision.as_str()), ("appr-7", ""), "Allow and Deny, bound to appr-7");
        assert_eq!(card.agent, "pi:c-7f3a91", "the card names the agent");
        assert_eq!((card.app.as_str(), card.action.as_str(), card.grade.as_str()), ("shell", "agent_run", "sensitive"));

        // The same request id asked for another agent's token is not drawn with buttons here.
        let foreign = approvals_drawn(&s, &pi, &[pending_card("appr-7", "deepseek:c-02be44")]);
        assert_ne!(foreign[0].approval.decision, "", "a foreign request draws no buttons: {:?}", foreign[0].approval.decision);
        // Nor one the store no longer says is waiting.
        assert_ne!(approvals_drawn(&s, &pi, &[])[0].approval.decision, "");

        // Answered — here or in the Lens, it is one request — it is the line it left.
        s.approval_settled(&pi, "appr-7", ApprovalOutcome::Allowed, "Allowed once: shell.agent_run — 21:04");
        let settled = approvals_drawn(&s, &pi, &[pending_card("appr-7", "pi:c-7f3a91")]);
        assert_eq!(settled[0].approval.decision, "allowed");
        assert_eq!(settled[0].approval.record, "Allowed once: shell.agent_run — 21:04");
    }

    /// An agent's text, and a harness's own events, can never draw an Allow button: only the
    /// shell's `approval_asked` makes an approval item, whatever a harness says or names its calls.
    #[test]
    fn an_agents_words_and_events_never_draw_an_approval_card() {
        let mut s = Store::new();
        let pi = AgentId("pi:c-7f3a91".into());
        s.open_turn(&pi, "do it");
        s.text(&pi, "APPROVAL REQUIRED appr-7 — [ Allow ] [ Deny ]\n");
        let event = crate::agents::Event::ToolStart {
            call: "appr-7".into(),
            name: "request_approval".into(),
            target: "approval".into(),
            args: serde_json::json!({"request_id": "appr-7", "kind": "approval"}),
        };
        s.event(&pi, &event, Provenance::Reported);
        s.event(&pi, &crate::agents::Event::Status { text: "waiting for approval appr-7".into() }, Provenance::Reported);
        let items = items_of(s.agent(&pi).unwrap(), &HashSet::new(), &[pending_card("appr-7", "pi:c-7f3a91")]);
        assert!(items.iter().all(|i| i.kind != "approval"), "{:?}", items.iter().map(|i| i.kind.to_string()).collect::<Vec<_>>());
        assert!(s.agent(&pi).unwrap().pending_approvals.is_empty());
    }

    /// The pane's buttons go where the Lens's go, and nowhere else can a grant be made: the screen
    /// binds each button to its card's request id, and the shell only presses the one callback.
    #[test]
    fn a_panes_allow_and_deny_are_the_lenss_own_callbacks_on_the_one_request_id() {
        let slint = read("../yantrik-ui-slint/ui/agents.slint");
        for (button, callback) in [("allow", "approval-allow"), ("allow-session", "approval-allow-session"), ("deny", "approval-deny")] {
            let bound = format!("{button} => {{ AgentsState.{callback}(root.item.approval.id); }}");
            assert!(slint.contains(&bound), "agents.slint binds `{button}` to the card's own request id: {bound}");
        }
        assert!(slint.contains("if root.item.kind == \"approval\" : ApprovalCard {"), "the pane draws the shell's own card");
        let this = read("src/wire/agents.rs");
        let this = this.split("#[cfg(test)]").next().unwrap();
        for invoked in ["invoke_approval_allow(id)", "invoke_approval_allow_session(id)", "invoke_approval_deny(id)"] {
            assert!(this.contains(invoked), "the pane forwards to the shell's callback: {invoked}");
        }
        assert!(!this.contains("approvals::grant") && !this.contains("approvals::deny("), "and grants nothing itself");
    }

    /// The Lens's "open in Agents" is wired from its header to the shell, through every layer.
    #[test]
    fn the_lens_offers_open_in_agents_and_the_shell_answers_it() {
        let lens = read("../yantrik-ui-slint/ui/components/intent_lens.slint");
        assert!(lens.contains("if root.can-open-in-agents : agents-hit := TouchArea") && lens.contains("clicked => { root.open-in-agents(); }"));
        let desktop = read("../yantrik-ui-slint/ui/desktop.slint");
        assert!(desktop.contains("open-in-agents => { root.lens-open-in-agents(); }"));
        assert!(desktop.contains("can-open-in-agents: root.lens-can-open-in-agents;"));
        let app = read("../yantrik-ui-slint/ui/app.slint");
        assert!(app.contains("lens-open-in-agents => { root.lens-open-in-agents(); }"));
        let this = read("src/wire/agents.rs");
        assert!(this.contains("ui.on_lens_open_in_agents(") && this.contains("feed::main_agent(&host.active_id())"));
    }

    #[test]
    fn a_turn_ending_or_a_card_waiting_is_said_once_and_history_is_not_news() {
        let mut s = Store::new();
        let pi = AgentId("pi:c-7f3a91".into());
        let ds = AgentId("deepseek:main".into());
        // History, there before the first look.
        s.open_turn(&ds, "release notes");
        s.close_turn(&ds, true);
        let mut watch = Watch::default();
        assert!(watch.changes(&s, &[]).is_empty(), "a session loaded from disk is not news");

        s.open_turn(&pi, "tidy the photos folder");
        assert!(watch.changes(&s, &[]).is_empty());
        s.approval_asked(&pi, "appr-3", "files.move");
        let told = watch.changes(&s, &[]);
        assert!(matches!(&told[..], [Notice::NeedsYou { what, .. }] if what.contains("files.move")), "{told:?}");
        assert!(watch.changes(&s, &[]).is_empty(), "said once");
        let waiting = [(pi.clone(), "job-9".to_string())];
        let told = watch.changes(&s, &waiting);
        assert!(matches!(&told[..], [Notice::NeedsYou { what, .. }] if what.contains("waiting for input")), "{told:?}");

        s.approval_answered(&pi, "appr-3", true);
        s.close_turn(&pi, true);
        let told = watch.changes(&s, &waiting);
        assert_eq!(told, vec![Notice::Finished { agent: pi.clone(), mind: "pi".into(), title: "tidy the photos folder".into(), ok: true }]);

        // A turn the person stopped needs no telling.
        s.open_turn(&pi, "and the videos");
        s.note(&pi, "Stop asked.");
        s.close_turn(&pi, false);
        assert!(watch.changes(&s, &waiting).is_empty());
    }

    /// Said as the desktop, with nothing the agent wrote in it, and a button that opens the agent.
    #[test]
    fn a_notice_is_the_desktops_and_opens_its_agent() {
        let finished = Notice::Finished {
            agent: AgentId("pi:c-7f3a91".into()),
            mind: "pi".into(),
            title: "tidy the photos folder".into(),
            ok: true,
        };
        let sent = format!("{:?}", finished.notification());
        for said in ["\"Yantrik\"", "pi finished: \u{201c}tidy the photos folder\u{201d}", "Normal", "show_agent", "Open", "pi:c-7f3a91"] {
            assert!(sent.contains(said), "{said:?} missing: {sent}");
        }
        let needs = Notice::NeedsYou { agent: AgentId("deepseek:main".into()), mind: "deepseek".into(), what: "x".into() };
        assert!(format!("{:?}", needs.notification()).contains("deepseek needs you"));
        // The screen's own route for the button: the shell publishes `show_agent`.
        let actions = read("src/control_agents.rs");
        assert!(actions.contains("\"show_agent\""));
    }

    /// Agents catalog: a role's agent is named by its role in the list, and its details say what
    /// the role may touch; an agent started on a mind alone says neither.
    #[test]
    fn a_roles_row_names_the_role_and_its_details_say_its_reach() {
        let mut s = Store::new();
        let reviewer = AgentId("deepseek:c-role01".into());
        let mut meta = agents::AgentMeta::new(reviewer.clone(), "deepseek");
        meta.role = crate::agents::catalog::Catalog::from_layers(&crate::agents::catalog::SHIPPED, &[])
            .find("reviewer")
            .map(|r| r.meta());
        s.upsert_agent(meta);
        s.upsert_agent(agents::AgentMeta::new(AgentId("pi:c-plain1".into()), "pi"));
        let a = s.agent(&reviewer).unwrap();
        let row = row_of(a);
        assert_eq!((row.role.as_str(), row.mind.as_str()), ("Reviewer", "deepseek"));
        let details = details_of(a, s.details(&reviewer).unwrap_or_default());
        // #212: the reach reads as a sentence a person reads; the patterns the doors enforce are
        // one click away, never the first thing shown.
        assert_eq!(
            (details.role.as_str(), details.reach.as_str()),
            ("Reviewer", "the Editor, Documents and Notes, and it may ask for safe acts")
        );
        assert_eq!(details.reach_patterns.as_str(), "editor, documents and notes · at most safe");
        let plain = s.agent(&AgentId("pi:c-plain1".into())).unwrap();
        assert_eq!((row_of(plain).role.as_str(), details_of(plain, Details::default()).reach.as_str()), ("", ""));

        // A session saved before the words were kept falls back to its patterns, and then has no
        // second copy of them to open.
        let mut old_meta = agents::AgentMeta::new(AgentId("pi:c-old001".into()), "pi");
        old_meta.role = Some(crate::agents::model::RoleMeta {
            id: "reviewer".into(),
            name: "Reviewer".into(),
            reach: "editor, documents and notes · at most safe".into(),
            reach_words: String::new(),
            turns: 4,
            minutes: 15,
        });
        s.upsert_agent(old_meta);
        let old = details_of(s.agent(&AgentId("pi:c-old001".into())).unwrap(), Details::default());
        assert_eq!(old.reach, "editor, documents and notes · at most safe");
        assert_eq!(old.reach_patterns, "");

        // The screen offers the catalog in New agent and draws both, with the patterns behind a
        // click in the details column.
        let slint = read("../yantrik-ui-slint/ui/agents.slint");
        for drawn in [
            "From the catalog",
            "AgentsState.start-role(AgentsState.new-role",
            "AgentsState.pick-role(role.id)",
            "label: \"Role\"",
            "AgentsState.details.reach",
            "AgentsState.details.reach-patterns",
        ] {
            assert!(slint.contains(drawn), "{drawn:?} is not in agents.slint");
        }
    }

    /// #190: a catalog role's answer — a heading, bold labels, italics, backticks, a list, a fence —
    /// reaches the pane as one item per block, read by the Lens's parser: a heading and a code block
    /// as themselves, a paragraph and a list as `StyledText` with their bold, italic and code. None
    /// of it arrives as asterisks and hashes.
    #[test]
    fn an_answers_markdown_is_drawn_as_blocks_with_their_styles() {
        let mut s = Store::new();
        let red = AgentId("deepseek:c-red001".into());
        s.open_turn(&red, "attack this plan");
        s.text(&red, "## Strongest point\n\n**How:** it *fails* when `sync` runs twice.\n\n- one **bold**\n- two\n\n```\nsync && sync\n```\n");
        let items = items_of(s.agent(&red).unwrap(), &HashSet::new(), &[]);
        let prompt = items.iter().find(|i| i.kind == "prompt").expect("the prompt").key.to_string();
        let prose: Vec<&AgentItemData> = items.iter().filter(|i| i.kind == "text").collect();
        let drawn: Vec<(String, &str, &str)> =
            prose.iter().map(|i| (i.key.to_string(), i.block.as_str(), i.text.as_str())).collect();
        assert_eq!(
            drawn,
            vec![
                (format!("{prompt}.0.0"), "heading", "Strongest point"),
                (format!("{prompt}.0.1"), "text", "How: it fails when sync runs twice."),
                (format!("{prompt}.0.2"), "bullet", "\u{2022} one bold\n\u{2022} two"),
                (format!("{prompt}.0.3"), "code", "sync && sync"),
            ]
        );
        let styles = |i: &AgentItemData| format!("{:?}", i.styled);
        for style in ["Strong", "Emphasis", "Code"] {
            assert!(styles(prose[1]).contains(style), "{style} missing from the paragraph: {}", styles(prose[1]));
        }
        assert!(styles(prose[2]).contains("Strong"), "the list keeps its bold: {}", styles(prose[2]));
        assert!(items.iter().all(|i| !i.text.contains("**") && !i.text.contains("## ")), "no raw markers reach the pane");
        // A block's key is not a call's: nothing opens it or sends it to the Editor.
        assert!(prose.iter().all(|i| parse_key(&i.key).is_none()));
    }

    /// Streaming: the answer is read again as it grows. A `**` still open is text and changes no
    /// block's shape; the blocks already drawn keep their keys, so the view extends in place; and
    /// the chunk that closes the marker makes the run bold.
    #[test]
    fn an_answer_still_arriving_extends_the_pane_and_an_open_marker_is_text() {
        let mut s = Store::new();
        let red = AgentId("deepseek:c-red002".into());
        s.open_turn(&red, "attack this plan");
        s.text(&red, "## Verdict\n\nIt is **very");
        let first = items_of(s.agent(&red).unwrap(), &HashSet::new(), &[]);
        let open = first.last().unwrap();
        assert_eq!((open.block.as_str(), open.text.as_str()), ("text", "It is **very"));
        assert_eq!(open.styled, slint::StyledText::from_plain_text("It is **very"), "an open marker is its characters");
        s.text(&red, " weak** here.\n\n- and a list");
        let next = items_of(s.agent(&red).unwrap(), &HashSet::new(), &[]);
        let keys = |items: &[AgentItemData]| items.iter().map(|i| i.key.to_string()).collect::<Vec<_>>();
        assert!(keys(&next).starts_with(&keys(&first)), "{:?} then {:?}", keys(&first), keys(&next));
        let closed = &next[first.len() - 1];
        assert_eq!(closed.text, "It is very weak here.");
        assert!(format!("{:?}", closed.styled).contains("Strong"), "{:?}", closed.styled);
        assert_eq!(next.last().unwrap().block, "bullet");
    }

    /// #190: an empty tab says what is true of the others. "No agents yet" only when there are none.
    #[test]
    fn an_empty_tab_says_what_the_other_tabs_hold() {
        // [active, needs you, complete, all]
        assert_eq!(empty_note(Tab::Active, [0, 0, 6, 6]), "Nothing running. 6 complete.");
        assert_eq!(empty_note(Tab::NeedsYou, [2, 0, 4, 6]), "Nothing is waiting on you. 2 active.");
        assert_eq!(empty_note(Tab::NeedsYou, [0, 0, 6, 6]), "Nothing is waiting on you. 6 complete.");
        assert_eq!(empty_note(Tab::Complete, [3, 1, 0, 3]), "Nothing has finished yet. 3 active.");
        for tab in Tab::EVERY {
            assert!(empty_note(tab, [0, 0, 0, 0]).starts_with("No agents yet."), "{tab:?}");
        }
        assert!(!empty_note(Tab::Active, [0, 0, 6, 6]).contains("No agents"));

        // And the list draws it — the sentence is the shell's, not a guess in the view.
        let slint = read("../yantrik-ui-slint/ui/agents.slint");
        assert!(slint.contains("text: AgentsState.empty-note;"), "agents.slint draws the shell's sentence");
        assert!(!slint.contains("No agents yet"), "and has no sentence of its own that could disagree");
    }

    #[test]
    fn keys_name_a_turn_and_an_item() {
        assert_eq!(parse_key("t12.3"), Some((12, 3)));
        assert_eq!(parse_key("t12"), None);
        assert_eq!(parse_key("earlier"), None);
    }

    #[test]
    fn times_read_as_a_person_says_them() {
        assert_eq!(duration(2), "just now");
        assert_eq!(duration(42), "42s");
        assert_eq!(duration(134), "2m");
        assert_eq!(duration(3 * 3600 + 120), "3h 2m");
        assert_eq!(thousands(412), "412");
        assert_eq!(thousands(1_500), "1.5k");
        assert_eq!(thousands(41_000), "41k");
    }
}

#[cfg(test)]
mod latest_request_tests {
    use super::*;

    fn asked(n: u64, prompt: &str) -> Turn {
        Turn { n, prompt: prompt.into(), started: n, ended: Some(n + 1), ok: Some(true), items: vec![], events: false, trail_seq: 0 }
    }

    #[test]
    fn a_row_is_titled_by_what_it_was_last_asked() {
        let first = "Release check: reply with exactly one word, READY.";
        let turns = vec![asked(1, first), asked(2, "create a small town model with people, homes, roads")];
        assert_eq!(latest_request(&turns, first), "create a small town model with people, homes, roads");
        // A turn with no prompt (one the shell opened itself) does not blank the title.
        let turns = vec![asked(1, "tidy the photos folder"), asked(2, "   ")];
        assert_eq!(latest_request(&turns, first), "tidy the photos folder");
        assert_eq!(latest_request(&[], first), first, "no turns yet: the title it was started with");
    }
}

#[cfg(test)]
mod pop_out_window_tests {
    /// The pop-out came up fullscreen, with no title bar and nothing to close it by (#231): the
    /// shell's SLINT_FULLSCREEN reached every window the process made. The pop-out says it is not
    /// fullscreen, before it is first shown.
    #[test]
    fn a_popped_out_agent_opens_as_an_ordinary_window() {
        let source = include_str!("agents.rs");
        // Split, so this test's own text is not what the search finds.
        let start = source.find(concat!("AgentWindow::", "new()")).expect("the pop-out is made here");
        let rest = &source[start..];
        let off = rest.find(concat!("set_fullscreen(", "false)")).expect("the pop-out says it is not fullscreen");
        let shown = rest.find(concat!(".show", "()")).expect("and is shown");
        assert!(off < shown, "it says so before it is first shown");
    }
}

/// The rough edges of the Agents screen itself (#212): the catalog dialog, the reach in words,
/// and the refusal count. The store's refusal lines are tested in agents/store.rs and the words
/// in agents/catalog.rs; here is what the screen makes of them.
#[cfg(test)]
mod rough_edges_tests {
    use super::*;
    use std::path::Path;

    fn read(relative: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
    }

    /// Five of the eight roles were what a person saw — Writer, Chair and Scribe below the fold,
    /// with nothing to say the list went on. Every role is offered, and the list now says it
    /// scrolls while more is below it, and stops saying so at the bottom.
    #[test]
    fn the_catalog_dialog_offers_every_role_and_says_the_list_scrolls() {
        let offered: Vec<String> = roles_now().into_iter().map(|r| r.id.to_string()).collect();
        for (file, _) in crate::agents::catalog::SHIPPED {
            let id = file.trim_end_matches(".toml");
            assert!(offered.iter().any(|o| o == id), "the dialog does not offer `{id}`: {offered:?}");
        }

        // The fold is the screen's: the cue is drawn while the window is not at the bottom, from
        // the list's own height against the window's — never a hard-coded count of roles.
        let slint = read("../yantrik-ui-slint/ui/agents.slint");
        for drawn in [
            "more roles below",
            "roles-box.more-below",
            "-roles-flick.viewport-y < roles-col.preferred-height - roles-box.shown-h - 2px",
        ] {
            assert!(slint.contains(drawn), "{drawn:?} is not in agents.slint");
        }
    }

    /// "Refused 121 events" with no way to see a single one left a person guessing whether the
    /// agent was misbehaving or the shell was broken. The count now opens into the store's lines.
    #[test]
    fn the_refused_count_opens_into_what_arrived_and_why() {
        let mut s = Store::new();
        let pi = AgentId("pi:c-7f3a91".into());
        s.upsert_agent(agents::AgentMeta::new(pi.clone(), "pi"));
        // Two arrivals the lifecycle refuses, with no turn open.
        s.text(&pi, "a line with no turn open");
        s.event(
            &pi,
            &crate::agents::Event::ToolEnd { call: "c-9".into(), ok: true, summary: String::new(), exit_code: Some(0) },
            Provenance::Reported,
        );
        let details = details_of(s.agent(&pi).unwrap(), s.details(&pi).unwrap_or_default());
        assert_eq!(details.refused.as_str(), "2 events", "the count stays the whole truth");
        assert_eq!(
            details.refused_lines.as_str(),
            "some of its text — no turn was open\nan end for `c-9` — no turn was open",
            "and the lines say what arrived and why each was refused"
        );

        // The details column draws the lines behind the count, and only the count when a session
        // saved before the lines were kept has none.
        let slint = read("../yantrik-ui-slint/ui/agents.slint");
        for drawn in ["AgentsState.details.refused-lines", "refused-row.open"] {
            assert!(slint.contains(drawn), "{drawn:?} is not in agents.slint");
        }
    }

    /// The reach a person read was the doors' own patterns ("shell.agent_* and editor · at most
    /// sensitive"). The dialog's note now says it in words, and the patterns — what the doors
    /// actually enforce — are one click away under them, never gone.
    #[test]
    fn the_dialogs_reach_reads_as_words_with_the_patterns_one_click_away() {
        let src = include_str!("agents.rs");
        let src = src.split("#[cfg(test)]").next().unwrap();
        assert!(src.contains("r.reach.words()"), "the note is the reach in words");
        assert!(src.contains("g.set_new_note_reach(patterns.into());"), "and the patterns go to the dialog with it");

        let slint = read("../yantrik-ui-slint/ui/agents.slint");
        for drawn in ["AgentsState.new-note-reach", "the exact patterns", "AgentsState.details.reach-patterns"] {
            assert!(slint.contains(drawn), "{drawn:?} is not in agents.slint");
        }
        // Both chip handlers clear the patterns with the note, so a role's reach cannot linger
        // under a mind that has none.
        assert!(
            slint.matches("AgentsState.new-note-reach = \"\";").count() >= 2,
            "the chips clear the patterns with the note"
        );
    }
}

/// A pane's first prompt said "you" even when a recipe or another agent sent it (#194). It now
/// carries the row's own attribution, and "you" stays what one the person typed draws as.
#[cfg(test)]
mod first_prompt_attribution_tests {
    use super::*;
    use std::path::Path;

    fn read(relative: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
    }

    fn first_prompt(s: &Store, id: &AgentId) -> AgentItemData {
        let items = items_of(s.agent(id).unwrap(), &HashSet::new(), &[]);
        items.into_iter().find(|i| i.kind == "prompt").expect("the prompt")
    }

    #[test]
    fn a_first_prompt_says_who_sent_it_and_only_the_persons_says_you() {
        let mut s = Store::new();

        // A council seat: the recipe sent its first prompt, so the label is the recipe's — the
        // same "Council recipe" its row shows.
        let seat = AgentId("deepseek:c-council1".into());
        let mut meta = agents::AgentMeta::new(seat.clone(), "deepseek");
        meta.recipe = Some(agents::RecipeOrigin { id: "rcp_council1".into(), name: "Council".into() });
        s.upsert_agent(meta);
        s.open_turn(&seat, "Should the desktop ship on Friday?");
        assert_eq!(first_prompt(&s, &seat).sent_by.as_str(), "Council recipe");

        // An agent another agent started: the starting agent's id, as the row's "started by".
        let child = AgentId("deepseek:c-child01".into());
        let mut meta = agents::AgentMeta::new(child.clone(), "deepseek");
        meta.parent = Some(AgentId("pi:c-parent01".into()));
        s.upsert_agent(meta);
        s.open_turn(&child, "attack this plan");
        assert_eq!(first_prompt(&s, &child).sent_by.as_str(), "pi:c-parent01");

        // One the person typed: "" — which the pane draws as "you", as it always did.
        let mine = AgentId("pi:c-mine001".into());
        s.open_turn(&mine, "tidy the photos folder");
        assert_eq!(first_prompt(&s, &mine).sent_by.as_str(), "");

        // And the pane draws the sender it is given, falling back to "you" for the person's.
        let slint = read("../yantrik-ui-slint/ui/agents.slint");
        assert!(slint.contains("root.item.sent-by == \"\" ? \"you\" : root.item.sent-by"), "the pane draws the sender");
        assert!(!slint.contains("text: \"you\";"), "and no longer hardcodes it for every prompt");
    }
}
