//! The Agents screen (34), and every agent popped out into a window of its own.
//!
//! The screen is a workroom (see `agents_workroom` for what it says and `agents.slint` for how it
//! is drawn): decisions waiting, desks at work, the minds, and one run's detail when it is opened.
//! A popped-out window is still one agent's whole session, as it was.
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
    bytes, now, Agent, Approval, ApprovalOutcome, CallState, Card, Details, Item, Mark, OutputKind, Provenance, State, Turn,
};
use crate::agents::{self, feed, launch, AgentId, Store};
use crate::app_context::AppContext;
use super::agents_workroom as workroom;
use crate::{
    AccentPreset, AgentDetailsData, AgentHeaderData, AgentItemData, AgentMindData, AgentRoleData,
    AgentRunData, AgentWindow, AgentsState, App, ApprovalRequest, ChangeRowData, ChildRunData, DecisionData,
    DeskCardData, ResultData, RunFactData, ThemeMode, ThemeOverrides, ToolCallData, WorkCardData, WorkNavData,
};
use super::lens_work;

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
    /// The one run shown, when the row picked is a run (`agent#n`); None: the whole session.
    run: Option<u64>,
    /// The run drawn last, so a change of run redraws.
    drawn_run: Option<u64>,
    /// Cards and thinking the person opened, by key.
    expanded: HashSet<String>,
    items: Rc<VecModel<AgentItemData>>,
    keys: Vec<String>,
    /// The store revision and local change drawn last.
    drawn: Option<(u64, u64)>,
    local: u64,
    /// The screen's detail reads as a timeline: runs of calls are folded into one line each, and
    /// the run's ledger and facts are published. A popped-out window shows every card, as before.
    grouped: bool,
}

impl Surface {
    fn new() -> Self {
        Surface {
            agent: None,
            run: None,
            drawn_run: None,
            expanded: HashSet::new(),
            items: Rc::new(VecModel::default()),
            keys: Vec::new(),
            drawn: None,
            local: 0,
            grouped: false,
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
    /// The run opened, when the screen shows one.
    selected: Option<AgentId>,
    /// The run picked, when the selected row is one run of a chat (`agent#n`).
    run: Option<u64>,
    main: Surface,
    windows: BTreeMap<AgentId, Popped>,
    /// When New agent last read the catalog, while it is open.
    roles_read: Option<std::time::Instant>,
    /// The mind the workroom is narrowed to, by harness id.
    mind: Option<String>,
}

type Shared = Rc<RefCell<Screen>>;

pub fn wire(ui: &App, _ctx: &AppContext) {
    let state: Shared = Rc::new(RefCell::new(Screen {
        selected: None,
        run: None,
        main: Surface { grouped: true, ..Surface::new() },
        windows: BTreeMap::new(),
        roles_read: None,
        mind: None,
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

    g.on_select(on(|ui, state, id| {
        let key = agents::RowKey::parse(&id);
        let mut st = state.borrow_mut();
        st.selected = Some(key.agent);
        st.run = key.run;
        drop(st);
        // Choosing a run, from a desk, a request, History or the navigation, opens it.
        ui.global::<AgentsState>().set_detail_open(true);
        refresh(ui, state, true);
    }));
    // ── The workroom: its pages, a mind's narrowing, back from a run, and Chat ──
    g.on_back({
        let (weak, state) = (weak.clone(), state.clone());
        move || {
            if let Some(ui) = weak.upgrade() {
                leave_run(&ui, &state);
            }
        }
    });
    g.on_show_section(on(|ui, state, section| {
        let g = ui.global::<AgentsState>();
        state.borrow_mut().mind = None;
        g.set_mind_filter("".into());
        g.set_mind_filter_name("".into());
        g.set_section(section.into());
        leave_run(ui, state);
    }));
    g.on_filter_mind(on(|ui, state, mind| {
        let g = ui.global::<AgentsState>();
        state.borrow_mut().mind = Some(mind.clone());
        g.set_mind_filter(mind.into());
        g.set_section("workroom".into());
        leave_run(ui, state);
    }));
    g.on_chat_with(on(|ui, _state, id| notice(&ui.global::<AgentsState>(), chat_with(ui, &id))));
    g.on_pop_out(on(|ui, state, id| pop_out(ui, state, agent_of_row(&id))));
    g.on_stop(on(|ui, state, id| {
        notice(&ui.global::<AgentsState>(), launch::stop(&agent_of_row(&id)));
        refresh(ui, state, true);
    }));
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
            notice(&ui.global::<AgentsState>(), launch::tell(&AgentId(agent.to_string()), &text));
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
            let g = ui.global::<AgentsState>();
            g.set_section("history".into());
            g.set_mind_filter("".into());
            g.set_mind_filter_name("".into());
            g.set_detail_open(false);
            state.borrow_mut().mind = None;
            ui.set_lens_open(false);
            ui.set_current_screen(SCREEN);
            ui.invoke_navigate(SCREEN);
            refresh(&ui, &state, true);
        }
    });

    let watch = RefCell::new(Watch::default());
    // The Lens's pumps for answers picked back up after a restart (#246), kept alive here.
    let resumed_streams = crate::streaming::Streams::new();
    let timer = Timer::default();
    {
        let (weak, state) = (weak.clone(), state.clone());
        timer.start(TimerMode::Repeated, TICK, move || {
            agents::store().save_if_due();
            let seen = Seen::now();
            sync_with_host(&seen);
            // A terminal job sitting at its prompt waits on the person exactly like an approval
            // card does (#182): its row goes to WaitingForYou, and the Active list sorts it top.
            let waiting_jobs = waiting_input_jobs();
            let waiting: Vec<AgentId> = waiting_jobs.iter().map(|(agent, _)| agent.clone()).collect();
            agents::store().jobs_waiting(&waiting);
            let Some(ui) = weak.upgrade() else { return };
            pick_up_resumed(&ui, &resumed_streams);
            // The Lens offers "open in Agents" while its conversation is an attached mind's.
            let lens_agent = crate::wire::harness::host()
                .map(|h| h.active_id())
                .is_some_and(|active| seen.minds.iter().any(|(id, _, _)| *id == active));
            if ui.get_lens_can_open_in_agents() != lens_agent {
                ui.set_lens_can_open_in_agents(lens_agent);
            }
            publish_lens_questions(&ui);
            if ui.get_current_screen() == SCREEN {
                refresh(&ui, &state, false);
            }
            refresh_windows(&ui, &state);
            tell_the_person(&ui, &state, &mut watch.borrow_mut(), &waiting_jobs);
        });
    }
    // The timer lives as long as the shell, the idiom every wire module uses.
    std::mem::forget(timer);
}

/// Back to the page a run was opened from: nothing selected, and nothing half-asked.
fn leave_run(ui: &App, state: &Shared) {
    {
        let mut st = state.borrow_mut();
        st.selected = None;
        st.run = None;
    }
    let g = ui.global::<AgentsState>();
    g.set_detail_open(false);
    g.set_confirm_stop("".into());
    g.set_menu_key("".into());
    refresh(ui, state, true);
}

/// Chat: the person's conversation with a mind, in the Lens. `id` is a harness id (a desk's mind)
/// or an agent's id (`pi:c-7f3a91`, a run's): either way it is the mind that is chosen, and the
/// Lens opens on it. The choice is the Lens's own (`harness::choose`), so a mind that cannot
/// answer says why instead of the Lens opening on another.
fn chat_with(ui: &App, id: &str) -> Result<(), String> {
    let harness = id.split_once(':').map_or(id, |(harness, _)| harness);
    let host = crate::wire::harness::host().ok_or_else(|| "the harness host is not running yet".to_string())?;
    crate::wire::harness::choose(host, harness)?;
    // Remembered, as every choice of a mind is (`use_harness`): the host's choice is restored from
    // the saved one, and one that was never saved is handed back to the old mind a moment later.
    crate::wire::settings::set_preferred_mind(harness);
    // The Lens is drawn by the desktop and nowhere else, so Chat goes there, as `open_lens` does.
    if ui.get_current_screen() != 1 {
        ui.set_current_screen(1);
        ui.invoke_navigate(1);
    }
    ui.set_lens_open(true);
    ui.invoke_open_lens();
    Ok(())
}

/// Turns a re-attaching harness picked back up after the shell restarted (#246): back in their
/// agents, and, for the Lens's own conversation, back in the Lens, where the person was
/// waiting for the answer.
fn pick_up_resumed(ui: &App, streams: &crate::streaming::Streams) {
    let Some(host) = crate::wire::harness::host() else { return };
    for r in host.take_resumed() {
        let lens = feed::main_agent(&host.active_id()) == r.agent;
        tracing::info!(agent = %r.agent, lens, "A harness picked a turn back up after the restart");
        let mind = feed::meta_for(&r.agent).mind;
        if let Some(answer) = feed::resumed(r.agent, &r.prompt, r.answer, lens) {
            crate::wire::chat::resume_in_lens(&ui.as_weak(), &mind, &r.prompt, answer, streams);
        }
    }
}

/// Start work, either way: the new agent opened, or why it did not start.
fn started(ui: &App, state: &Shared, outcome: Result<AgentId, String>) {
    let g = ui.global::<AgentsState>();
    match outcome {
        Ok(agent) => {
            g.set_new_open(false);
            g.set_new_error("".into());
            {
                let mut st = state.borrow_mut();
                st.selected = Some(agent);
                st.run = None;
            }
            // The desk it was given is the page to watch.
            g.set_detail_open(true);
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

/// Redraw the screen: the workroom, and the opened run's session and details.
fn refresh(ui: &App, state: &Shared, force: bool) {
    let g = ui.global::<AgentsState>();
    let seen = Seen::now();
    // Read before the store's lock is taken: the terminal has its own.
    let jobs = waiting_input_jobs();
    let mut st = state.borrow_mut();
    let st = &mut *st;
    agents::store().read(|s| {
        // A run opened that the store no longer has (closed from another door) goes back to the
        // page it came from. Nothing is selected on its own: the workroom opens on the workroom.
        if st.selected.as_ref().is_some_and(|id| s.agent(id).is_none()) || (st.selected.is_none() && g.get_detail_open()) {
            st.selected = None;
            st.run = None;
            g.set_detail_open(false);
        }
        let selected = st
            .selected
            .clone()
            .map(|agent| agents::RowKey { agent, run: st.run }.id())
            .unwrap_or_default();
        if g.get_selected() != selected.as_str() {
            g.set_selected(selected.into());
        }
        let selected = st.selected.clone().filter(|_| g.get_detail_open());
        st.main.run = st.run;
        draw(&g, &mut st.main, s, selected.as_ref(), &seen, force);

        publish_workroom(&g, &room_now(s, &seen, &jobs), st.mind.as_deref());
        publish_lens(&g, s);
    });
    let mode_line = workroom::mode_line(crate::mind_mode::current());
    if g.get_mode_line() != mode_line.as_str() {
        g.set_mode_line(mode_line.into());
    }

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

/// What the Lens's conversation shows of the runs it started (Chat v2): a card per run, the run the
/// strip describes, and the counts. The runs are the active mind's own conversation, which is the
/// one the Lens is. Set only where it changed, so a conversation in which nothing moves asks for no
/// redraw.
fn publish_lens(g: &AgentsState, s: &Store) {
    let host = crate::wire::harness::host();
    let active = host.map(|h| h.active_id()).unwrap_or_else(|| crate::wire::harness::BUILTIN_ID.to_string());
    let works = s
        .agent(&feed::main_agent(&active))
        .map(|a| lens_work::works_of(a, now()))
        .unwrap_or_default();
    let cards: Vec<WorkCardData> = works.iter().map(work_card).collect();
    if let Some(model) = crate::models::changed(g.get_lens_cards(), cards) {
        g.set_lens_cards(model);
    }
    let strip = lens_work::strip_of(&works).map(work_card).unwrap_or_default();
    if g.get_lens_strip() != strip {
        g.set_lens_strip(strip);
    }
    let live = works.iter().filter(|w| w.live).count() as i32;
    let waiting = works.iter().filter(|w| w.state == "needs-you").count() as i32;
    if g.get_lens_live() != live {
        g.set_lens_live(live);
    }
    if g.get_lens_waiting() != waiting {
        g.set_lens_waiting(waiting);
    }
    let identity = if active == crate::wire::harness::BUILTIN_ID { "Built-in mind" } else { "Attached mind" };
    if g.get_lens_identity() != identity {
        g.set_lens_identity(identity.into());
    }
}

fn work_card(w: &lens_work::Work) -> WorkCardData {
    WorkCardData {
        run: w.run.as_str().into(),
        title: w.title.as_str().into(),
        mind: w.mind.as_str().into(),
        state: w.state.into(),
        label: w.label.into(),
        activity: w.activity.as_str().into(),
        can_view_desk: w.can_view_desk,
        can_review: w.can_review,
    }
}

/// The workroom as it stands: what the screen draws and `describe shell` says, from the one
/// reading of the stores.
fn room_now(s: &Store, seen: &Seen, jobs: &[(AgentId, String)]) -> workroom::Workroom {
    let mut minds: Vec<(String, String)> = seen.minds.iter().map(|(id, name, _)| (id.clone(), name.clone())).collect();
    minds.sort();
    let mut attached = seen.attached();
    attached.push(crate::wire::harness::BUILTIN_ID.to_string());
    workroom::compose(s, &minds, &attached, &seen.approvals, jobs, now())
}

/// `describe shell` under `workroom`: which page the screen is on and what it says — the counts,
/// each mind's state, and, for the person's own reader, the desks and the requests waiting. A mind
/// reading it is told the counts and the states, never what the person asked of the others or
/// what is being asked of them (the boundary `agents::PERSONS_FIELDS` holds for `agents`).
pub fn workroom_for_describe(ui: &App, agent_reading: bool) -> serde_json::Value {
    // Read before the store's lock is taken: the approvals and the terminal have their own.
    let seen = Seen::now();
    let jobs = waiting_input_jobs();
    let room = agents::store().read(|s| room_now(s, &seen, &jobs));
    let g = ui.global::<AgentsState>();
    let opened = g.get_detail_open().then(|| g.get_selected().to_string()).filter(|key| !key.is_empty());
    let narrowed = Some(g.get_mind_filter().to_string()).filter(|mind| !mind.is_empty());
    workroom::for_describe(&room, &g.get_section(), opened.as_deref(), narrowed.as_deref(), agent_reading)
}

/// The minds the workroom can be narrowed to, as `(id, name)`: the navigation's own list.
pub fn workroom_minds_now() -> Vec<(String, String)> {
    let seen = Seen::now();
    let jobs = waiting_input_jobs();
    let room = agents::store().read(|s| room_now(s, &seen, &jobs));
    room.nav.into_iter().filter(|n| n.kind == "mind").map(|n| (n.id, n.label)).collect()
}

/// The workroom, into the global: the navigation, the desks, the shelf and the pages behind it.
/// Every list is set only where it changed, so a workroom in which nothing moves asks for no
/// redraw. The header's counts and the navigation are the whole desktop's; the pages are the
/// narrowed mind's when one is chosen.
fn publish_workroom(g: &AgentsState, all: &workroom::Workroom, mind: Option<&str>) {
    let view = mind.map_or_else(|| all.clone(), |m| all.narrowed(m));
    let set = |current: slint::SharedString, text: String, put: &dyn Fn(slint::SharedString)| {
        if current != text.as_str() {
            put(text.into());
        }
    };
    set(g.get_summary(), all.summary(), &|t| g.set_summary(t));
    let name = mind
        .and_then(|m| all.nav.iter().find(|n| n.kind == "mind" && n.id == m))
        .map(|n| n.label.clone())
        .unwrap_or_default();
    set(g.get_mind_filter_name(), name, &|t| g.set_mind_filter_name(t));
    for (current, now, put) in [
        (g.get_needs_count(), all.requests as i32, &(|n| g.set_needs_count(n)) as &dyn Fn(i32)),
        (g.get_request_minds(), all.request_minds as i32, &|n| g.set_request_minds(n)),
        (g.get_runs_count(), all.runs as i32, &|n| g.set_runs_count(n)),
        (g.get_shelf_count(), view.requests as i32, &|n| g.set_shelf_count(n)),
        (g.get_shelf_minds(), view.request_minds as i32, &|n| g.set_shelf_minds(n)),
        (g.get_view_runs(), view.runs as i32, &|n| g.set_view_runs(n)),
    ] {
        if current != now {
            put(now);
        }
    }

    let nav: Vec<WorkNavData> = all
        .nav
        .iter()
        .map(|n| WorkNavData {
            kind: n.kind.into(),
            id: n.id.as_str().into(),
            label: n.label.as_str().into(),
            sub: n.sub.as_str().into(),
            needs: n.needs as i32,
            working: n.working,
            available: n.available,
        })
        .collect();
    if let Some(model) = crate::models::changed(g.get_nav_minds(), nav) {
        g.set_nav_minds(model);
    }
    let desks: Vec<DeskCardData> = view
        .desks
        .iter()
        .map(|d| DeskCardData {
            key: d.key.as_str().into(),
            mind_id: d.mind_id.as_str().into(),
            mind: d.mind.as_str().into(),
            via: d.via.as_str().into(),
            task: d.task.as_str().into(),
            state: d.state.into(),
            label: d.label.as_str().into(),
            activity: d.activity.as_str().into(),
            since: d.since.as_str().into(),
        })
        .collect();
    if let Some(model) = crate::models::changed(g.get_desks(), desks) {
        g.set_desks(model);
    }
    let decision = |d: &workroom::Decision| DecisionData {
        key: d.key.as_str().into(),
        request: d.request.as_str().into(),
        mind: d.mind.as_str().into(),
        task: d.task.as_str().into(),
        text: d.text.as_str().into(),
        age: workroom::ago(d.age_secs).into(),
        action: d.action.as_str().into(),
    };
    // The Needs you page lists the whole desktop's; the shelf is the narrowed mind's, three of them.
    let every: Vec<DecisionData> = all.decisions.iter().map(decision).collect();
    if let Some(model) = crate::models::changed(g.get_decisions(), every) {
        g.set_decisions(model);
    }
    let shelf: Vec<DecisionData> = view.decisions.iter().take(workroom::SHELF_SHOWN).map(decision).collect();
    if let Some(model) = crate::models::changed(g.get_shelf(), shelf) {
        g.set_shelf(model);
    }
    let result = |r: &workroom::Finished| ResultData {
        key: r.key.as_str().into(),
        mind: r.mind.as_str().into(),
        task: r.task.as_str().into(),
        outcome: r.outcome.into(),
        label: r.label.into(),
        when: r.when.as_str().into(),
    };
    let recent: Vec<ResultData> = view.history.iter().take(workroom::RECENT_SHOWN).map(result).collect();
    if let Some(model) = crate::models::changed(g.get_recent(), recent) {
        g.set_recent(model);
    }
    let history: Vec<ResultData> = view.history.iter().map(result).collect();
    if let Some(model) = crate::models::changed(g.get_history(), history) {
        g.set_history(model);
    }
}

/// The opened run: its state in the screen's words, the changes recorded and the facts behind it.
/// Worked out when the store changes, with the session, not every tick.
fn publish_detail(g: &AgentsState, s: &Store, a: &Agent, run: Option<u64>) {
    let (state, label) = workroom::state_of(a, run);
    if g.get_detail_state() != state {
        g.set_detail_state(state.into());
    }
    if g.get_detail_label() != label {
        g.set_detail_label(label.into());
    }
    let turns: Vec<&Turn> = match run {
        Some(n) => a.turns.iter().filter(|t| t.n == n).collect(),
        None => a.turns.iter().collect(),
    };
    let changes = workroom::changes_of(&turns);
    if g.get_changes_recorded() != workroom::recorded(&changes) {
        g.set_changes_recorded(workroom::recorded(&changes));
    }
    let rows: Vec<ChangeRowData> = changes
        .iter()
        .map(|c| ChangeRowData { group: c.group.into(), text: c.text.as_str().into(), status: c.status.as_str().into(), tone: c.tone.into() })
        .collect();
    if let Some(model) = crate::models::changed(g.get_changes(), rows) {
        g.set_changes(model);
    }
    let facts: Vec<RunFactData> =
        workroom::run_facts(a, run).into_iter().map(|(label, value)| RunFactData { label: label.into(), value: value.into() }).collect();
    if let Some(model) = crate::models::changed(g.get_run_facts(), facts) {
        g.set_run_facts(model);
    }
    let children: Vec<ChildRunData> = workroom::child_runs(s, a, run)
        .into_iter()
        .map(|c| ChildRunData { key: c.key.into(), title: c.title.into(), label: c.label.into(), state: c.state.into() })
        .collect();
    if let Some(model) = crate::models::changed(g.get_child_runs(), children) {
        g.set_child_runs(model);
    }
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
    let mut header = header_of(a, seen);
    if let Some(t) = surface.run.and_then(|n| a.turns.iter().find(|t| t.n == n)) {
        // One run of a chat: named by what it was asked.
        header.title = one_line(&t.prompt, TITLE_CHARS).into();
    }
    if g.get_header() != header {
        g.set_header(header);
    }
    let details = details_of(a, s.details(&a.meta.id).unwrap_or_default());
    if g.get_details() != details {
        g.set_details(details);
    }

    if surface.grouped {
        // The mode is the desktop's and moves with the person's choice, not with the store.
        let mode = workroom::mode_label(crate::mind_mode::current());
        if g.get_detail_mode() != mode {
            g.set_detail_mode(mode.into());
        }
    }
    let fresh = surface.agent.as_ref() != Some(&a.meta.id) || surface.drawn_run != surface.run;
    if fresh {
        surface.agent = Some(a.meta.id.clone());
        surface.drawn_run = surface.run;
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
    let items = items_of(a, &surface.expanded, &seen.approvals, surface.run);
    let items = if surface.grouped {
        publish_detail(g, s, a, surface.run);
        workroom::group_calls(items, &surface.expanded)
    } else {
        items
    };
    publish_items(g, surface, items, fresh);
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

/// The agent a run's key is about: a run's key is `agent#turn`.
fn agent_of_row(id: &str) -> AgentId {
    AgentId(id.split_once('#').map_or(id, |(agent, _)| agent).to_string())
}

/// What an agent was last asked, which is what its title is about now.
///
/// A title used to carry the conversation's first prompt for ever. The Lens's conversation with a
/// mind is one long-lived agent, so every request a person made there sat under whatever they
/// asked first, hours before: on VM 520, "Release check: reply with exactly one word, READY."
/// over a town model, a daily briefing and a game (#234, #246).
/// The longest a row's or a header's title is kept: they are one line and elide at the edge, so
/// this only bounds the work of a prompt pasted in whole.
const TITLE_CHARS: usize = 200;

pub(super) fn latest_request<'a>(turns: &'a [Turn], first: &'a str) -> &'a str {
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

pub(super) fn duration(secs: u64) -> String {
    match secs {
        0..=4 => "just now".into(),
        5..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m", secs / 60),
        _ => format!("{}h {}m", secs / 3600, (secs % 3600) / 60),
    }
}

/// Local time today, the date before today.
pub(super) fn clock(unix: u64) -> String {
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
    // A stuck task can be told something (#234): the box opens, and says what sending does.
    let stuck = crate::agents::progress::of(a, now()).and_then(|p| p.stuck);
    let tell_hint = stuck
        .as_deref()
        .filter(|_| reachable && !gone)
        .map(|why| format!("Stuck: {why}. Tell {} what to do differently (this interrupts the stuck step)…", a.meta.mind))
        .unwrap_or_default();
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
        title: one_line(latest_request(&a.turns, &a.meta.title), TITLE_CHARS).into(),
        state: a.state.key().into(),
        label: a.state.label().into(),
        since: since(a).into(),
        status: a.status.as_str().into(),
        note: note.into(),
        can_send: reachable && !gone && (!turn_open || !tell_hint.is_empty()),
        send_hint: send_hint.into(),
        tell_hint: tell_hint.into(),
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

pub(super) fn one_line(text: &str, max: usize) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    format!("{}…", flat.chars().take(max - 1).collect::<String>())
}

/// A session as the screen draws it, newest last. `pending` is the shell's approval store's
/// waiting requests: an approval item is drawn from there, never from the session.
fn items_of(a: &Agent, expanded: &HashSet<String>, pending: &[crate::approvals::Card], run: Option<u64>) -> Vec<AgentItemData> {
    let mut out = Vec::new();
    // One run of a chat: that turn alone. The rest of the conversation is the chat's, not this run's.
    if let Some(n) = run {
        if let Some(i) = a.turns.iter().position(|t| t.n == n) {
            return items_of_turns(a, &a.turns[i..=i], expanded, pending, out);
        }
    }
    let from = a.turns.len().saturating_sub(SHOWN_TURNS);
    if from > 0 {
        out.push(AgentItemData {
            kind: "note".into(),
            key: "earlier".into(),
            text: format!("{from} earlier turns are kept in its saved session, not shown here.").into(),
            ..Default::default()
        });
    }
    items_of_turns(a, &a.turns[from..], expanded, pending, out)
}

/// The items of `turns`, appended to `out`.
fn items_of_turns(
    a: &Agent,
    turns: &[Turn],
    expanded: &HashSet<String>,
    pending: &[crate::approvals::Card],
    mut out: Vec<AgentItemData>,
) -> Vec<AgentItemData> {
    for turn in turns {
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
                Item::Question(q) => out.push(question_of(q, key)),
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

const QUESTION_OPTIONS: usize = 6;
const QUESTION_OPTION_CHARS: usize = 40;
const QUESTION_CHARS: usize = 2000;

/// The questions the Lens's mind is waiting on, for the Lens to draw where approvals sit. Only
/// the active mind's own conversation: another agent's question is answered in Agents.
fn publish_lens_questions(ui: &App) {
    let Some(host) = super::harness::host() else { return };
    let agent = feed::main_agent(&host.active_id());
    let questions: Vec<crate::QuestionRequest> = agents::store().read(|s| {
        let Some(a) = s.agent(&agent) else { return Vec::new() };
        a.turns
            .iter()
            .flat_map(|t| t.items.iter())
            .filter_map(|i| match i {
                Item::Question(q) if q.waiting() => Some(lens_question(&a.meta.id.0, &a.meta.mind, q)),
                _ => None,
            })
            .collect()
    });
    if let Some(model) = crate::models::changed(ui.get_lens_questions(), questions) {
        ui.set_lens_questions(model);
    }
}

/// Every question an agent is waiting on a person to answer, for `describe shell`: beside
/// `pending_approvals`, so a second mind or a test can tell "waiting for someone to answer" from
/// "hung". The same limits as the cards; answering stays with the card (`answer_question` is the
/// person's), so this only says what is being asked.
pub fn questions_for_describe() -> serde_json::Value {
    let waiting: Vec<serde_json::Value> = agents::store().read(|s| {
        s.agents()
            .iter()
            .flat_map(|a| {
                a.turns.iter().flat_map(|t| t.items.iter()).filter_map(move |i| match i {
                    Item::Question(q) if q.waiting() => Some(serde_json::json!({
                        "agent": a.meta.id.0,
                        "mind": a.meta.mind,
                        "request": q.request,
                        "prompt": clip(&q.prompt, QUESTION_CHARS),
                        "options": q.options.iter().take(QUESTION_OPTIONS)
                            .map(|o| clip(o, QUESTION_OPTION_CHARS)).collect::<Vec<_>>(),
                    })),
                    _ => None,
                })
            })
            .collect()
    });
    serde_json::Value::Array(waiting)
}

/// One waiting question as the Lens's card draws it, held to the same limits as in Agents.
fn lens_question(agent: &str, mind: &str, q: &crate::agents::model::Question) -> crate::QuestionRequest {
    crate::QuestionRequest {
        agent: agent.into(),
        mind: mind.into(),
        request: q.request.as_str().into(),
        prompt: clip(&q.prompt, QUESTION_CHARS).into(),
        options: slint::ModelRc::new(slint::VecModel::from(
            q.options
                .iter()
                .take(QUESTION_OPTIONS)
                .map(|o| slint::SharedString::from(clip(o, QUESTION_OPTION_CHARS)))
                .collect::<Vec<_>>(),
        )),
    }
}

/// `s` drawn in at most `n` characters.
fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { s.chars().take(n - 1).chain(['…']).collect() }
}

/// A question the agent asked, as its card draws it: waiting with its answers, or the line it
/// leaves once answered or closed.
fn question_of(q: &crate::agents::model::Question, key: String) -> AgentItemData {
    AgentItemData {
        kind: "question".into(),
        key: key.into(),
        text: clip(&q.prompt, QUESTION_CHARS).into(),
        request: q.request.as_str().into(),
        // A clipped label is answered with the agent's full option (`answer_question`); the
        // label is only how it is drawn.
        options: slint::ModelRc::new(slint::VecModel::from(
            q.options
                .iter()
                .take(QUESTION_OPTIONS)
                .map(|o| slint::SharedString::from(clip(o, QUESTION_OPTION_CHARS)))
                .collect::<Vec<_>>(),
        )),
        answer: q.answer.as_str().into(),
        explain: q.closed.as_str().into(),
        ..Default::default()
    }
}

/// The person answered the agent's question through its card. The host takes it first — it
/// alone knows the run and refuses a second answer — and only then does the card settle; if the
/// host could not deliver it, the card closes with why instead of claiming an answer that never
/// arrived.
pub fn answer_question(agent: &str, request: &str, answer: &str) {
    let answer = answer.trim();
    if answer.is_empty() {
        return;
    }
    let id = AgentId(agent.to_string());
    // A button stands for the agent's own option, whatever its clipped label says.
    let full = agents::store().read(|s| {
        s.agent(&id)?.turns.iter().flat_map(|t| t.items.iter()).find_map(|i| match i {
            Item::Question(q) if q.request == request => {
                q.options.iter().find(|o| clip(o, QUESTION_OPTION_CHARS) == answer).cloned()
            }
            _ => None,
        })
    });
    let answer = full.as_deref().unwrap_or(answer);
    let Some(host) = super::harness::host() else { return };
    match host.answer_for(&id, request, &serde_json::Value::String(answer.to_string())) {
        Ok(()) => {
            agents::store().question_answered(&id, request, answer);
        }
        Err(why) => agents::store().question_closed(&id, request, &why),
    }
}

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
        request: Default::default(),
        options: Default::default(),
        answer: Default::default(),
        block: Default::default(),
        styled: Default::default(),
    }
}

// ── Acts ──────────────────────────────────────────────────────────

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
                notice(&window.global::<AgentsState>(), launch::tell(&AgentId(agent.to_string()), &text));
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
    // Only the person's click on a question card comes here; no control-surface action and no
    // harness call answers a run's question (#25).
    g.on_answer_question(|agent, request, answer| answer_question(&agent, &request, &answer));
}

/// Put one agent on the Agents screen: selected, under a tab that lists it, the screen shown.
/// The Lens's "open in Agents", a notification's Open, and `show_agent` all come here.
fn show_agent(ui: &App, state: &Shared, agent: AgentId) {
    // `show_agent` may name one run of a chat (`agent#n`): the chat's link to it opens that run.
    let key = agents::RowKey::parse(&agent.0);
    let known = agents::store().read(|s| s.agent(&key.agent).is_some());
    let g = ui.global::<AgentsState>();
    if !known {
        notice(&g, Err(format!("`{agent}` is no longer in the list.")));
        return;
    }
    {
        let mut st = state.borrow_mut();
        st.selected = Some(key.agent);
        st.run = key.run;
    }
    g.set_detail_open(true);
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
    /// Its turn is going round, or has gone quiet (#234): the shell's own reading, once a turn.
    Stuck { agent: AgentId, mind: String, title: String, why: crate::agents::progress::Stuck },
}

impl Notice {
    fn agent(&self) -> &AgentId {
        match self {
            Notice::Finished { agent, .. } | Notice::NeedsYou { agent, .. } | Notice::Stuck { agent, .. } => agent,
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
            Notice::Stuck { mind, title, why, .. } => (
                format!("{mind} looks stuck: \u{201c}{}\u{201d}", one_line(title, 60)),
                format!("{} Open it to see where, give it a hint, or stop it.", why.plain()),
            ),
        };
        let n = Notification::new("Yantrik", title)
            .body(body)
            // News, not a question with a deadline: Do Not Disturb holds it, like any other.
            .urgency(Level::Normal)
            // The shell presses `show_agent` on its own surface for this, as Download Manager's
            // "Open folder" is pressed on its.
            .action_with("show_agent", self.open_label(), serde_json::json!({ "agent": self.agent().0 }));
        // A stuck task can also be stopped from where the person is (#234). The press is the
        // person's, through the shell's own `stop_agent`, graded as it always is.
        if matches!(self, Notice::Stuck { .. }) {
            n.action_with("stop_agent", "Stop", serde_json::json!({ "agent": self.agent().0 }))
        } else {
            n
        }
    }

    /// "Tell it" for a stuck task, whose pane opens ready for a hint; "Open" for the rest.
    fn open_label(&self) -> &'static str {
        if matches!(self, Notice::Stuck { .. }) { "Tell it" } else { "Open" }
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
    /// The turn each agent was last said to be stuck in: once a turn, not once a tick.
    stuck: HashMap<AgentId, u64>,
}

impl Watch {
    /// `waiting_jobs` is the agent terminal's commands sitting at a prompt: `(agent, job)`.
    fn changes(&mut self, s: &Store, waiting_jobs: &[(AgentId, String)], at: u64) -> Vec<Notice> {
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
                            // The turn that ended, not the agent's title. A mind that holds one
                            // conversation across every chat keeps the title of its first prompt
                            // for good, and "Yantrik Mind finished: “Release check: reply with
                            // exactly one word…”" was the toast for a Blender scene (VM 520).
                            title: turn.prompt.clone(),
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
            if let (Some(turn), Some(p)) = (a.open_turn(), crate::agents::progress::of(a, at)) {
                if let Some(why) = p.stuck_kind {
                    if self.stuck.get(id) != Some(&turn.n) {
                        self.stuck.insert(id.clone(), turn.n);
                        out.push(Notice::Stuck {
                            agent: id.clone(),
                            mind: a.meta.mind.clone(),
                            title: turn.prompt.clone(),
                            why,
                        });
                    }
                }
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

/// The agent terminal's jobs sitting at a prompt: `(agent, job)`.
fn waiting_input_jobs() -> Vec<(AgentId, String)> {
    crate::control_agent_terminal::running_jobs()
        .into_iter()
        .filter(|(_, job)| job["waiting_for_input"] == true)
        .map(|(agent, job)| (agent, job["job"].as_str().unwrap_or_default().to_string()))
        .collect()
}

/// Send what the person should hear, except about what they are already looking at: the agent
/// selected on the Agents screen, one in a window of its own, or — with the Lens open — the
/// Lens's own mind, whose answer and cards are in front of them there. A "needs you" is also held
/// while the Lens is open at all: its approval card is in the Lens, and a toast would land on it.
fn tell_the_person(ui: &App, state: &Shared, watch: &mut Watch, waiting_jobs: &[(AgentId, String)]) {
    let now = crate::agents::model::now();
    let notices = agents::store().read(|s| watch.changes(s, waiting_jobs, now));
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

    /// `describe shell` says what an agent is waiting on the person to answer, and stops saying it
    /// once it is answered: a test or a second mind can tell "asked, waiting" from "hung".
    #[test]
    fn a_waiting_question_is_in_describe_until_it_is_answered() {
        let agent = AgentId::new("pi", "c-describe-question");
        agents::store().open_turn(&agent, "tidy Downloads");
        let ask = yantrik_harness::event::Event::Request {
            request_id: "q-describe-1".into(),
            prompt: "Delete the 3 old installers?".into(),
            options: vec!["Yes".into(), "No".into()],
        };
        agents::store().event(&agent, &ask, agents::model::Provenance::Reported);
        let mine = |v: serde_json::Value| -> Vec<serde_json::Value> {
            v.as_array().unwrap().iter().filter(|q| q["agent"] == agent.0.as_str()).cloned().collect()
        };
        let waiting = mine(questions_for_describe());
        assert_eq!(waiting.len(), 1, "{waiting:?}");
        assert_eq!(waiting[0]["request"], "q-describe-1");
        assert_eq!(waiting[0]["prompt"], "Delete the 3 old installers?");
        assert_eq!(waiting[0]["options"], serde_json::json!(["Yes", "No"]));

        agents::store().question_answered(&agent, "q-describe-1", "No");
        assert!(mine(questions_for_describe()).is_empty(), "an answered question is not waiting");
    }

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
        // Listed for the person whole, and for an agent without the person's fields.
        assert!(
            control.contains("\"agents\",") && control.contains("crate::agents::for_describe()")
                && control.contains("crate::agents::for_describe_by_an_agent()"),
            "describe shell lists the agents"
        );
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
            said: String::new(),
            caller_says: String::new(),
            can_session: true,
            status: crate::approvals::Status::Pending,
            record: String::new(),
            decided_at: String::new(),
            session: false,
            age_secs: 4,
        }
    }

    fn approvals_drawn(store: &Store, agent: &AgentId, pending: &[crate::approvals::Card]) -> Vec<AgentItemData> {
        items_of(store.agent(agent).unwrap(), &HashSet::new(), pending, None).into_iter().filter(|i| i.kind == "approval").collect()
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
        let items = items_of(s.agent(&pi).unwrap(), &HashSet::new(), &[pending_card("appr-7", "pi:c-7f3a91")], None);
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
        // The header button (Chat v2: ChatHeader) is offered only for an agent's conversation, and
        // the Lens forwards its press.
        let header = read("../yantrik-ui-slint/ui/components/chat_chrome.slint");
        assert!(header.contains("if root.can-open-in-agents : ChatIconButton {\n                    label: \"Open in Agents\";"));
        assert!(header.contains("clicked => { root.open-in-agents(); }"));
        let lens = read("../yantrik-ui-slint/ui/components/intent_lens.slint");
        assert!(lens.contains("open-in-agents => { root.open-in-agents(); }"));
        let desktop = read("../yantrik-ui-slint/ui/desktop.slint");
        assert!(desktop.contains("open-in-agents => { root.lens-open-in-agents(); }"));
        assert!(desktop.contains("can-open-in-agents: root.lens-can-open-in-agents;"));
        let app = read("../yantrik-ui-slint/ui/app.slint");
        assert!(app.contains("lens-open-in-agents => { root.lens-open-in-agents(); }"));
        let this = read("src/wire/agents.rs");
        assert!(this.contains("ui.on_lens_open_in_agents(") && this.contains("feed::main_agent(&host.active_id())"));
    }

    /// The session row says what its rule actually covers (#182). The rule is an (app, action)
    /// pair in the mode file; every door reads that file for every mind, and none asks which
    /// agent the rule was minted for — so "for this session" on its own reads as "for this
    /// agent's session", and it is not. Until the rule itself is scoped to the asking agent,
    /// the card must say plainly that the person is arming the whole desktop.
    #[test]
    fn the_session_row_says_its_rule_covers_every_mind() {
        let lens = read("../yantrik-ui-slint/ui/components/intent_lens.slint");
        let row = lens.split("if root.data.can-session :").nth(1).expect("the session row is drawn");
        assert!(
            row.lines().take(45).any(|l| l.contains("covers every mind and caller, until restart or the mode is lowered")),
            "the session row says whose sessions the rule covers, and until when, inside the row that mints it"
        );
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
        assert!(watch.changes(&s, &[], 0).is_empty(), "a session loaded from disk is not news");

        s.open_turn(&pi, "tidy the photos folder");
        assert!(watch.changes(&s, &[], 0).is_empty());
        s.approval_asked(&pi, "appr-3", "files.move");
        let told = watch.changes(&s, &[], 0);
        assert!(matches!(&told[..], [Notice::NeedsYou { what, .. }] if what.contains("files.move")), "{told:?}");
        assert!(watch.changes(&s, &[], 0).is_empty(), "said once");
        let waiting = [(pi.clone(), "job-9".to_string())];
        let told = watch.changes(&s, &waiting, 0);
        assert!(matches!(&told[..], [Notice::NeedsYou { what, .. }] if what.contains("waiting for input")), "{told:?}");

        s.approval_answered(&pi, "appr-3", true);
        s.close_turn(&pi, true);
        let told = watch.changes(&s, &waiting, 0);
        assert_eq!(told, vec![Notice::Finished { agent: pi.clone(), mind: "pi".into(), title: "tidy the photos folder".into(), ok: true }]);

        // A later turn in the same conversation is told by its own prompt, not the first one's.
        s.open_turn(&pi, "now rename them by date");
        s.close_turn(&pi, true);
        let told = watch.changes(&s, &waiting, 0);
        assert!(
            matches!(&told[..], [Notice::Finished { title, .. }] if title == "now rename them by date"),
            "{told:?}"
        );

        // A turn the person stopped needs no telling.
        s.open_turn(&pi, "and the videos");
        s.note(&pi, "Stop asked.");
        s.close_turn(&pi, false);
        assert!(watch.changes(&s, &waiting, 0).is_empty());
    }

    /// A task that goes quiet is said once a turn, in the desktop's words: not every tick, and
    /// not quoting anything the mind wrote (#234, #139).
    #[test]
    fn a_stuck_turn_is_said_once_in_the_desktops_words() {
        use crate::agents::progress::{Stuck, STUCK_QUIET_SECS};
        let mut s = Store::new();
        let hermes = AgentId("hermes:main".into());
        let mut watch = Watch::default();
        assert!(watch.changes(&s, &[], 0).is_empty());

        s.open_turn(&hermes, "build a small game");
        let later = crate::agents::model::now() + STUCK_QUIET_SECS + 30;
        let told = watch.changes(&s, &[], later);
        let [Notice::Stuck { why: Stuck::Quiet { .. }, title, .. }] = &told[..] else {
            panic!("one stuck notice: {told:?}")
        };
        assert_eq!(title, "build a small game");
        let words = told[0].notification();
        assert!(format!("{words:?}").contains("Nothing has been heard from it for 2 min"), "{words:?}");
        // What the person can do about it from the notification: tell it something (its pane opens
        // ready for a hint), or stop it (#234).
        let actions = format!("{words:?}");
        assert!(actions.contains("show_agent") && actions.contains("Tell it"), "{actions}");
        assert!(actions.contains("stop_agent") && actions.contains("Stop"), "{actions}");
        let finished = Notice::Finished { agent: hermes.clone(), mind: "hermes".into(), title: "x".into(), ok: true };
        let plain = format!("{:?}", finished.notification());
        assert!(plain.contains("Open") && !plain.contains("stop_agent"), "only a stuck task offers Stop: {plain}");
        assert!(watch.changes(&s, &[], later + 60).is_empty(), "once a turn, not once a tick");

        // The next task is news of its own.
        s.close_turn(&hermes, false);
        watch.changes(&s, &[], later);
        s.open_turn(&hermes, "then the town model");
        assert!(matches!(&watch.changes(&s, &[], later + 300)[..], [Notice::Stuck { .. }]));
    }

    /// Only the person interrupts a task (#234): `launch::tell` is called from the pane's own send,
    /// and nothing a mind can call (the control surface) reaches it or `Host::interrupt`. A mind's
    /// `send_to_agent` still waits for the turn in flight.
    #[test]
    fn only_the_person_can_interrupt_a_task() {
        for file in ["src/control_agents.rs", "src/control.rs", "src/control_agent_terminal.rs"] {
            let text = read(file);
            let code = text.split("#[cfg(test)]").next().unwrap();
            assert!(!code.contains("launch::tell(") && !code.contains(".interrupt("), "{file} can interrupt a task");
        }
        let wiring = read("src/wire/agents.rs");
        let wiring = wiring.split("#[cfg(test)]").next().unwrap();
        assert_eq!(wiring.matches("launch::tell(").count(), 2, "the pane's send and the pop-out's");
        let send = read("src/control_agents.rs");
        assert!(send.contains("launch::send_on(host, target, text)"), "send_to_agent keeps the ordinary send");
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

    /// #368: a role agent's first prompt is its whole brief, many lines long. A desk's task and the
    /// pane's header are one line each, and an embedded newline breaks a Text however it elides, so
    /// the brief drew over what was below it.
    #[test]
    fn a_multi_line_request_is_one_line_in_its_desk_and_its_header() {
        let mut s = Store::new();
        let id = AgentId("deepseek:c-brief1".into());
        s.open_turn(&id, "You are the Researcher on this desktop.\n\nFind out what is true\nand say how you know.");
        let a = s.agent(&id).unwrap();
        let seen = Seen { minds: Vec::new(), agents: Vec::new(), approvals: Vec::new() };
        let header = header_of(a, &seen);
        assert_eq!(header.title.as_str(), "You are the Researcher on this desktop. Find out what is true and say how you know.");
        let desk = |s: &Store| {
            let room = workroom::compose(s, &[], &[], &[], &[], now());
            room.desks.into_iter().next().expect("the open turn is a desk").task
        };
        assert!(!desk(&s).contains('\n'));
        s.open_turn(&id, &format!("{}\nend", "x".repeat(500)));
        let long = header_of(s.agent(&id).unwrap(), &seen);
        assert!(!long.title.contains('\n') && long.title.chars().count() <= TITLE_CHARS);
        assert!(!desk(&s).contains('\n'));
    }

    /// Agents catalog: a role's agent is named by its role on its desk, and its details say what
    /// the role may touch; an agent started on a mind alone says neither.
    #[test]
    fn a_roles_desk_names_the_role_and_its_details_say_its_reach() {
        let mut s = Store::new();
        let reviewer = AgentId("deepseek:c-role01".into());
        let mut meta = agents::AgentMeta::new(reviewer.clone(), "deepseek");
        meta.role = crate::agents::catalog::Catalog::from_layers(&crate::agents::catalog::SHIPPED, &[])
            .find("reviewer")
            .map(|r| r.meta());
        s.upsert_agent(meta);
        s.upsert_agent(agents::AgentMeta::new(AgentId("pi:c-plain1".into()), "pi"));
        let a = s.agent(&reviewer).unwrap();
        assert_eq!((workroom::via_of(a).as_str(), a.meta.mind.as_str()), ("Reviewer", "deepseek"));
        let details = details_of(a, s.details(&reviewer).unwrap_or_default());
        // #212: the reach reads as a sentence a person reads; the patterns the doors enforce are
        // one click away, never the first thing shown.
        assert_eq!(
            (details.role.as_str(), details.reach.as_str()),
            ("Reviewer", "the Editor, Documents and Notes, and it may ask for safe acts")
        );
        assert_eq!(details.reach_patterns.as_str(), "editor, documents and notes · at most safe");
        let plain = s.agent(&AgentId("pi:c-plain1".into())).unwrap();
        assert_eq!((workroom::via_of(plain).as_str(), details_of(plain, Details::default()).reach.as_str()), ("", ""));

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

        // Start work offers the catalog and the screen hands the picked role its task; a run's
        // details say the role and its reach, with the patterns behind a click.
        let slint = read("../yantrik-ui-slint/ui/agents.slint");
        for drawn in [
            "AgentsState.start-role(AgentsState.new-role",
            "AgentsState.pick-role(id)",
            "label: \"Role\"",
            "AgentsState.details.reach",
            "AgentsState.details.reach-patterns",
        ] {
            assert!(slint.contains(drawn), "{drawn:?} is not in agents.slint");
        }
        let sheet = read("../yantrik-ui-slint/ui/agents_start.slint");
        assert!(sheet.contains("Role (optional)") && sheet.contains("root.pick-role(r.id)"), "the sheet offers the catalog");
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
        let items = items_of(s.agent(&red).unwrap(), &HashSet::new(), &[], None);
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
        let first = items_of(s.agent(&red).unwrap(), &HashSet::new(), &[], None);
        let open = first.last().unwrap();
        assert_eq!((open.block.as_str(), open.text.as_str()), ("text", "It is **very"));
        assert_eq!(open.styled, slint::StyledText::from_plain_text("It is **very"), "an open marker is its characters");
        s.text(&red, " weak** here.\n\n- and a list");
        let next = items_of(s.agent(&red).unwrap(), &HashSet::new(), &[], None);
        let keys = |items: &[AgentItemData]| items.iter().map(|i| i.key.to_string()).collect::<Vec<_>>();
        assert!(keys(&next).starts_with(&keys(&first)), "{:?} then {:?}", keys(&first), keys(&next));
        let closed = &next[first.len() - 1];
        assert_eq!(closed.text, "It is very weak here.");
        assert!(format!("{:?}", closed.styled).contains("Strong"), "{:?}", closed.styled);
        assert_eq!(next.last().unwrap().block, "bullet");
    }

    /// A run's key is `agent#turn`, and what acts on a run acts on its agent.
    #[test]
    fn a_runs_key_names_its_agent() {
        let hermes = AgentId("hermes:main".into());
        assert_eq!(agent_of_row("hermes:main#4"), hermes, "Stop on a run acts on its agent");
        assert_eq!(agent_of_row("hermes:main"), hermes, "an agent's key is the agent");
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
        Turn { n, prompt: prompt.into(), started: n, ended: Some(n + 1), ok: Some(true), lost: false, origin: Default::default(), items: vec![], events: false, trail_seq: 0 }
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

        // The fold is the sheet's: the cue is drawn while the window is not at the bottom, from
        // the list's own height against the window's — never a hard-coded count of roles.
        let slint = read("../yantrik-ui-slint/ui/agents_start.slint");
        for drawn in [
            "more roles below",
            "roles-box.more-below",
            "-roles-flick.viewport-y < roles-col.preferred-height - roles-box.shown-h - Theme.sp-half",
        ] {
            assert!(slint.contains(drawn), "{drawn:?} is not in agents_start.slint");
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
    /// sensitive"). The sheet's note now says it in words, and the patterns — what the doors
    /// actually enforce — are one click away under them, never gone.
    #[test]
    fn the_dialogs_reach_reads_as_words_with_the_patterns_one_click_away() {
        let src = include_str!("agents.rs");
        let src = src.split("#[cfg(test)]").next().unwrap();
        assert!(src.contains("r.reach.words()"), "the note is the reach in words");
        assert!(src.contains("g.set_new_note_reach(patterns.into());"), "and the patterns go to the dialog with it");

        let slint = read("../yantrik-ui-slint/ui/agents.slint");
        let sheet = read("../yantrik-ui-slint/ui/agents_start.slint");
        for drawn in ["AgentsState.new-note-reach", "AgentsState.details.reach-patterns"] {
            assert!(slint.contains(drawn), "{drawn:?} is not in agents.slint");
        }
        assert!(sheet.contains("the exact patterns"), "the sheet offers the patterns behind the words");
        // Both ways of switching between a mind and the catalog clear the patterns with the note,
        // so a role's reach cannot linger under a mind that has none.
        assert!(
            slint.matches("AgentsState.new-note-reach = \"\";").count() >= 2,
            "switching between a mind and the catalog clears the patterns with the note"
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
        let items = items_of(s.agent(id).unwrap(), &HashSet::new(), &[], None);
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

    #[test]
    fn the_lens_draws_its_minds_questions_held_to_the_same_limits_and_answers_them_the_one_way() {
        let q = crate::agents::model::Question {
            request: "r1".into(),
            prompt: "?".repeat(5_000),
            options: (0..20).map(|i| format!("{i}{}", "y".repeat(100))).collect(),
            answer: String::new(),
            closed: String::new(),
            asked: 0,
        };
        let card = lens_question("hermes:main", "Hermes", &q);
        assert_eq!((card.agent.as_str(), card.mind.as_str(), card.request.as_str()), ("hermes:main", "Hermes", "r1"));
        assert_eq!(card.options.row_count(), QUESTION_OPTIONS);
        assert_eq!(card.prompt.chars().count(), QUESTION_CHARS);

        // The Lens's card answers through the Agents view's own callback, the one path to
        // `answer_for`; it has no answering of its own.
        let app = read("../yantrik-ui-slint/ui/app.slint");
        assert!(app.contains("question-answer(agent, request, answer) => { AgentsState.answer-question(agent, request, answer); }"));
        let desktop = read("../yantrik-ui-slint/ui/desktop.slint");
        assert!(desktop.contains("questions: root.questions;"), "the desktop screen hands the Lens its questions");
        let lens = read("../yantrik-ui-slint/ui/components/intent_lens.slint");
        assert!(lens.contains("for q in root.questions : QuestionCard"), "the Lens draws them");
    }

    #[test]
    fn a_question_card_offers_its_answers_while_it_waits_and_settles_after() {
        let mut q = crate::agents::model::Question {
            request: "r1".into(),
            prompt: "Delete 3 installers?".into(),
            options: vec!["Yes".into(), "No".into()],
            answer: String::new(),
            closed: String::new(),
            asked: 0,
        };
        let card = question_of(&q, "t1.0".into());
        assert_eq!((card.kind.as_str(), card.request.as_str(), card.text.as_str()), ("question", "r1", "Delete 3 installers?"));
        assert_eq!(card.options.row_count(), 2);
        q.answer = "Yes".into();
        assert_eq!(question_of(&q, "t1.0".into()).answer.as_str(), "Yes");

        // The agent picks the answers; it cannot push the pane apart with them.
        q.options = (0..50).map(|i| format!("option {i} {}", "x".repeat(200))).collect();
        q.prompt = "?".repeat(10_000);
        let card = question_of(&q, "t1.0".into());
        assert_eq!(card.options.row_count(), QUESTION_OPTIONS);
        assert_eq!(card.options.row_data(0).unwrap().chars().count(), QUESTION_OPTION_CHARS);
        assert_eq!(card.text.chars().count(), QUESTION_CHARS);
    }

    /// Only the person answers a run's question: the card's callback is the one caller of
    /// `answer_for`. No control-surface action (which a mind can call) and nothing in the
    /// harness protocol reaches it, so an agent cannot answer its own question.
    #[test]
    fn nothing_a_mind_can_call_answers_a_question() {
        for file in ["src/control_agents.rs", "src/control.rs", "src/control_agent_terminal.rs", "src/control_approvals.rs"] {
            let text = read(file);
            let code = text.split("#[cfg(test)]").next().unwrap();
            assert!(!code.contains(".answer_for(") && !code.contains("answer_question("), "{file} can answer a run's question");
        }
        let wiring = read("src/wire/agents.rs");
        let wiring = wiring.split("#[cfg(test)]").next().unwrap();
        assert_eq!(wiring.matches("answer_for(").count(), 1, "one caller: the card's answer");
        assert!(wiring.contains("g.on_answer_question("));
    }
}
