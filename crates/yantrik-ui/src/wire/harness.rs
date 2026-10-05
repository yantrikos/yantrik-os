//! Which mind is answering — the shell's side of it.
//!
//! Three jobs. Wrap the companion so it is one candidate among others rather than the only one.
//! Serve the `harness` socket so anything else can attach. Keep the Settings screen and the
//! status bar showing the truth about both.
//!
//! The [`Host`] lives for the life of the shell and is reachable from anywhere in it through
//! [`host`], because two things need it that cannot hand each other a reference: this wiring, and
//! the shell's own control surface in `crate::control`.

use std::sync::{Arc, OnceLock};

use slint::{ComponentHandle, Model, ModelRc, Timer, TimerMode, VecModel};
use yantrik_harness::{Answer, Capabilities, Chunk, Harness, Health, Host, Turn};

use crate::app_context::AppContext;
use crate::bridge::CompanionBridge;
use crate::harness_catalogue::{self, Manifest};
use crate::{App, HarnessData, HarnessRowData};

/// How often the list is refreshed.
///
/// Harnesses arrive and leave on their own, so a list that only updated when the screen opened
/// would show one that left ten minutes ago. Two seconds is below noticing and costs a lock and a
/// few string clones.
const REFRESH: std::time::Duration = std::time::Duration::from_secs(2);

/// The screen and the section the catalogue is drawn on — `settings`, and `Harnesses` inside it.
///
/// The catalogue costs a directory walk, a handful of `stat` calls and one `systemctl show`, all
/// of which are free once and wasteful as a habit: on a machine nobody is touching it would be a
/// process spawn every two seconds forever. So it is only gathered while somebody is looking at
/// it, while a job this shell started is still running, or once at the start so the first open is
/// not blank. Everything else on this page — the picker, the status bar — needs only the attach
/// registry, which is in memory.
const SETTINGS_SCREEN: i32 = 7;
const HARNESSES_SECTION: i32 = 8;

static HOST: OnceLock<Host> = OnceLock::new();

/// The shell's harness host. Available once [`wire`] has run.
pub fn host() -> Option<&'static Host> {
    HOST.get()
}

// ── The companion, as one harness among others ──────────────────────

/// The built-in mind.
///
/// It is not reached over the protocol — it lives in this process and always has — so it is
/// wrapped rather than ported. That is the whole reason [`Harness`] still exists as a trait: for
/// the one mind that is compiled in. Everything else attaches.
///
/// Named once here so the chat path can ask "is the builtin driving?" without a string literal
/// of its own drifting away from this one.
pub const BUILTIN_ID: &str = "companion";

struct Companion {
    bridge: Arc<CompanionBridge>,
}

impl Harness for Companion {
    fn id(&self) -> &str {
        BUILTIN_ID
    }

    fn name(&self) -> &str {
        "Yantrik Companion"
    }

    fn capabilities(&self) -> Capabilities {
        // The only mind here with the OS's tools and its memory, because it is the only one
        // inside the process that owns them.
        Capabilities { streaming: true, tools: true, memory: true }
    }

    fn health(&self) -> Health {
        Health::Ready
    }

    fn send(&self, turn: Turn) -> Answer {
        // Asked from a phone, the companion reads and changes nothing for it.
        let remote = turn.is_remote();
        let tokens = self.bridge.send_message_from(turn.text, remote);
        let (tx, rx) = std::sync::mpsc::channel();
        // A thread rather than draining here: send() must return at once so the panel can start
        // rendering, and the companion's channel produces for as long as the model is talking.
        std::thread::Builder::new()
            .name("harness-companion".into())
            .spawn(move || {
                for token in tokens {
                    if tx.send(Chunk::Text(token)).is_err() {
                        return; // the panel stopped listening
                    }
                }
            })
            .ok();
        rx
    }
}

// ── Wiring ──────────────────────────────────────────────────────────

/// The run store at `$XDG_DATA_HOME/yantrik/runs.db`, or `None` (logged) when it cannot be opened.
fn open_runs() -> Option<Arc<yantrik_harness::run_store::RunStore>> {
    let dir = crate::agents::dir().parent()?.to_path_buf();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!(dir = %dir.display(), error = %e, "no directory for the run store; runs are not kept");
        return None;
    }
    let path = dir.join("runs.db");
    match yantrik_harness::run_store::RunStore::open(&path) {
        Ok(store) => Some(Arc::new(store)),
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "run store did not open; runs are not kept");
            None
        }
    }
}

/// How a mind is named in a notification: the catalogue's display name when it has one, since
/// that is the desktop's own word for it, else what the harness called itself, made safe. The
/// harness chose that string, and it is drawn under the shell's label: control and bidi
/// characters are dropped (a U+202E would reverse the line) and it is cut to 40 characters.
fn notice_name(harness: &str, catalogue: Option<&str>) -> String {
    const MAX: usize = 40;
    let clean = |raw: &str| -> String {
        let kept: String = raw.chars().filter(|c| !c.is_control() && !crate::approvals::is_format_char(*c)).collect();
        let flat = kept.split_whitespace().collect::<Vec<_>>().join(" ");
        if flat.chars().count() > MAX {
            let mut cut: String = flat.chars().take(MAX).collect();
            cut.push('\u{2026}');
            cut
        } else {
            flat
        }
    };
    let from_catalogue = catalogue.map(clean).filter(|n| !n.is_empty());
    from_catalogue.unwrap_or_else(|| clean(harness)).trim().to_string()
}

/// "<Mind> answered after you left. Open the chat to read it." A fixed sentence: the mind's words
/// are not quoted, so a late answer cannot pose as a prompt from the desktop, and a private
/// answer is not kept in the notification history. The full answer stays in that conversation's
/// history, where the person can read it.
fn late_answer_notice(name: &str) -> yantrik_app_runtime::notify::Notification {
    use yantrik_app_runtime::notify::{Level, Notification};
    // The mind's name as the desktop repeats it: one plain line, cut (security review of #648, M1).
    let name = crate::notification_groups::mind_name(name);
    let name = if name.is_empty() { "A mind".to_string() } else { name };
    Notification::new("Yantrik", format!("{name} answered after you left. Open the chat to read it."))
        .urgency(Level::Normal)
}

/// The notification for a late answer, or `None` while the person is in Private mode: nothing the
/// desktop raises should say that a mind has been working while they asked for none of it. Nor
/// while a test run has approvals off (`never_ask`): a gate run is when late answers happen, and
/// it must never put anything in front of the person.
fn late_answer_notice_for(
    late: &yantrik_harness::LateAnswer,
    private: bool,
    test_run: bool,
    catalogue: Option<&str>,
) -> Option<yantrik_app_runtime::notify::Notification> {
    if test_run {
        tracing::debug!(agent = %late.harness, "turn notice held back: a test run has approvals off");
        return None;
    }
    if private {
        return None;
    }
    Some(late_answer_notice(&notice_name(&late.harness, catalogue)))
}

pub fn wire(ui: &App, ctx: &AppContext) {
    // The memory grants read once as the shell starts, so their baseline is set now rather than
    // at the first question from the memory server, which may be hours away: until then a grant
    // written behind the shell's back would have been taken as the person's (#448 review).
    let _ = crate::memory_grants::load();
    let host = Host::new(vec![Arc::new(Companion { bridge: ctx.bridge.clone() })]);
    // Every turn a mind takes is kept as a run beside the agents' sessions (#25); without the
    // file the host still works, it just keeps no runs.
    let host = match open_runs() {
        Some(store) => host.with_runs(store),
        None => host,
    };
    // A mind the person has granted some use of their memory carries its credential with every
    // turn (#447); one with none carries nothing. Judged as `memory_validate` judges it: the
    // person's grants, with the first-party defaults only for the account that attached as the
    // mind account. A grants file that cannot be trusted hands nobody anything. The host asks
    // this outside its own lock, when a harness takes a turn.
    let host = host.with_memory(
        |harness, uid| {
            let store = crate::memory_grants::load();
            crate::memory_grants::carries_memory(store.as_ref(), harness, uid, yantrik_ipc_transport::mind_door::is_mind)
        },
        yantrik_ipc_transport::reach::token_digest,
    )
    // Where the harness presents it: the person's Mind serves their memory on a socket of its
    // own (#447), dialled only when it is there. Loopback TCP is a fallback per harness that
    // is off until one needs it, so it is never offered here.
    .with_memory_url(|| {
        use std::os::unix::fs::MetadataExt;
        let person = unsafe { libc::geteuid() };
        let socket = format!("/run/yantrik-mind/{person}/memory.sock");
        // Only a socket the mind account owns: one the person's own processes could have put
        // there would collect every credential a harness presents to it.
        let owner = std::fs::symlink_metadata(&socket).ok().map(|m| m.uid());
        owner.is_some_and(yantrik_ipc_transport::mind_door::is_mind).then(|| format!("unix:{socket}"))
    });
    // An answer that finishes after the person left its chat is told as a notification: the chat
    // is gone, and the text must not vanish without a word.
    let host = host.with_late_answer(|late| {
        let catalogue = crate::harness_catalogue::read_manifests(&crate::harness_catalogue::roots())
            .remove(&late.harness)
            .map(|m| m.name);
        let test_run = crate::never_ask::refusal().is_some();
        if let Some(notice) = late_answer_notice_for(&late, crate::private_mode::is_on(), test_run, catalogue.as_deref()) {
            yantrik_app_runtime::notify::send(notice);
        }
    });
    let _ = HOST.set(host.clone());

    // The agent terminal's side of agents (design/agents-workspace-2026-09-23.md, decision 3):
    // `agent_run` and the rest believe a token only as this host issued it and only from under
    // the harness it was issued to, and a command that ends after its call returned is noted
    // into its agent's next turn.
    crate::control_agent_terminal::serve_host(&host);

    serve_socket(host.clone(), ui.as_weak());

    // Choosing a mind, from Settings or from anywhere else that offers it.
    {
        let weak = ui.as_weak();
        let host = host.clone();
        ui.on_use_harness(move |id| {
            let Some(ui) = weak.upgrade() else { return };
            match choose(&host, &id) {
                Ok(()) => {
                    tracing::info!(harness = %id, "Now answering");
                    // Remembered, because choosing a mind is a decision about the machine and
                    // not about this run of the shell. It was not: the id lived in the host and
                    // the host lives with the process, so every update, crash or reboot handed
                    // the conversation back to the built-in without saying so — and the picker
                    // still showed the right name until you looked.
                    crate::wire::settings::set_preferred_mind(&id);
                    ui.set_harness_error("".into());
                }
                // Shown rather than logged: the person just clicked something and is owed an
                // answer about whether it worked.
                Err(e) => ui.set_harness_error(e.into()),
            }
            publish(&ui, &host);
        });
    }

    // Installing a mind, and starting one whose unit is merely stopped. Both change the machine,
    // so both are jobs: the row says what is happening and streams what the command says, rather
    // than freezing the settings screen for the half minute an `npm install -g` takes.
    {
        let weak = ui.as_weak();
        let host = host.clone();
        ui.on_install_harness(move |id| {
            let Some(ui) = weak.upgrade() else { return };
            let outcome = manifest(&id).and_then(|m| crate::harness_install::install(&m));
            report(&ui, &id, outcome);
            // Straight away rather than on the next tick: two seconds between pressing a button
            // and the row changing is two seconds in which it looks like nothing happened, and
            // that is exactly how a button gets pressed twice.
            publish(&ui, &host);
        });
    }
    {
        let weak = ui.as_weak();
        let host = host.clone();
        ui.on_start_harness(move |id| {
            let Some(ui) = weak.upgrade() else { return };
            let outcome = manifest(&id).and_then(|m| crate::harness_install::start(&m));
            report(&ui, &id, outcome);
            publish(&ui, &host);
        });
    }
    // The harness's own setup, in a terminal the person types into. Not a job: the window is the
    // progress, and when it closes the row reads the machine again as it always does.
    {
        let weak = ui.as_weak();
        ui.on_configure_harness(move |id| {
            let Some(ui) = weak.upgrade() else { return };
            if let Err(e) = configure(&id) {
                ui.set_harness_error(e.into());
            }
        });
    }

    crate::wire::harness_provider::wire(ui);

    publish(ui, &host);

    let timer = Timer::default();
    {
        let weak = ui.as_weak();
        let host = host.clone();
        timer.start(TimerMode::Repeated, REFRESH, move || {
            let Some(ui) = weak.upgrade() else { return };
            restore_choice(&host);
            publish(&ui, &host);
        });
    }
    // Keep timer alive
    std::mem::forget(timer);
}

/// The Settings list, for `describe shell`.
///
/// Read on demand rather than from what the screen last published: a `describe` is asked a
/// question and can afford one `systemctl show`, and answering from a cache that only refreshes
/// while a person has the page open would answer "not installed" about a harness installed ten
/// minutes ago.
pub fn catalogue_for_describe() -> serde_json::Value {
    let entries = match host() {
        Some(host) => host.list(),
        None => Vec::new(),
    };
    let machine = harness_catalogue::machine(crate::harness_install::views());
    serde_json::Value::Array(
        harness_catalogue::rows(&machine, &entries)
            .iter()
            .map(|row| {
                serde_json::json!({
                    "id": row.id,
                    "name": row.name,
                    // The key rather than the label: one is matched on by a program and the
                    // other is read by a person, and conflating them is how "needs setup"
                    // becomes a string comparison against a UI string.
                    "state": row.state.key(),
                    "detail": row.detail,
                    // What to do next, in the same words the row shows. Never a credential:
                    // this names a file at most.
                    "need": row.need,
                    "can_answer": row.state.can_answer(),
                    "can_install": row.can_install,
                    "can_start": row.can_start,
                    // The harness's own setup (its model, its sign-in), offered by
                    // `configure_harness`. Empty when there is none, or it is not installed yet.
                    "configure": row.configure,
                    // Which saved provider it was given (`assign_provider`), or its own settings.
                    "provider": row.provider_line,
                    "can_assign_provider": row.can_assign_provider,
                    "can_revert_provider": row.can_revert_provider,
                    "builtin": row.builtin,
                    "docs": row.docs,
                })
            })
            .collect(),
    )
}

/// The manifest for an id, or a sentence saying there is none.
fn manifest(id: &str) -> Result<Manifest, String> {
    harness_catalogue::read_manifests(&harness_catalogue::roots())
        .remove(id)
        .ok_or_else(|| format!("nothing on this machine describes a harness called `{id}`"))
}

/// The same two jobs the buttons start, for the shell's control surface.
///
/// Parity, the same way `use_harness` has it: anything a person can do on the Harnesses screen
/// an agent can ask for, and the grading on the action is what decides whether the person is
/// asked first.
pub fn install(id: &str) -> Result<String, String> {
    crate::harness_install::install(&manifest(id)?)
}

pub fn start(id: &str) -> Result<String, String> {
    crate::harness_install::start(&manifest(id)?)
}

pub fn configure(id: &str) -> Result<String, String> {
    crate::harness_install::configure(&manifest(id)?)
}

/// Choose which mind answers, refusing one its row says cannot take a turn.
///
/// A mind whose process is gone leaves the registry at once (#67, [`yantrik_harness::Host`]), so
/// choosing it is refused by `set_active`'s own sentence naming what is attached — the Settings
/// row, drawn from the same registry, shows it unattached at the same moment. For a mind the
/// registry does list, the row is what the person is looking at, so the choice asks it first:
/// whatever it says, the page and the action cannot disagree.
pub fn choose(host: &Host, id: &str) -> Result<(), String> {
    let entries = host.list();
    let machine = harness_catalogue::machine(crate::harness_install::views());
    if let Some(refusal) = row_refusal(&machine, &entries, id) {
        return Err(refusal);
    }
    host.set_active(id)
}

/// What a catalogue row says against choosing a mind the registry still lists, if anything.
///
/// Only the registry's own candidates are asked: for an id it does not hold, `set_active`
/// already answers with the list it does. A listed row always says its mind can answer — the
/// attachment wins in the catalogue, and the registry drops a session the moment the kernel
/// says its process is gone — so this is a guarantee rather than a second opinion: were a row
/// ever to say a listed mind cannot answer, the refusal stays the row's own `need` sentence.
fn row_refusal(
    machine: &harness_catalogue::Machine,
    entries: &[yantrik_harness::Entry],
    id: &str,
) -> Option<String> {
    if !entries.iter().any(|e| e.id == id) {
        return None;
    }
    harness_catalogue::rows(machine, entries)
        .into_iter()
        .find(|r| r.id == id)
        .filter(|r| !r.state.can_answer())
        .map(|r| r.need)
        .filter(|need| !need.is_empty())
}

/// Say what happened where the person is looking.
///
/// A refusal goes on the page rather than into the log for the same reason the picker's does:
/// somebody just pressed a button and is owed an answer about whether it worked. The command that
/// was started is logged, never shown — it is long, and the row is already streaming its output.
fn report(ui: &App, id: &str, outcome: Result<String, String>) {
    match outcome {
        Ok(command) => {
            tracing::info!(harness = %id, command = %command, "started a harness job");
            ui.set_harness_error("".into());
        }
        Err(e) => ui.set_harness_error(format!("{id}: {e}").into()),
    }
}

/// Give the conversation back to the mind the person chose, once it is there to take it.
///
/// Not at startup: at startup the only mind on this machine is the built-in one, because a
/// harness exists by attaching and nothing has attached yet. A remembered choice therefore
/// cannot be honoured when it is read — only when the thing it names turns up, which may be
/// seconds after boot or minutes, and which is exactly what this timer is already watching for.
///
/// Silent when there is nothing to do, and it does not fight the person: choosing any mind
/// saves that choice, so switching back to the built-in makes the built-in the preference.
fn restore_choice(host: &Host) {
    let want = crate::wire::settings::preferred_mind();
    if want.is_empty() || host.active_id() == want {
        return;
    }
    if !host.list().iter().any(|e| e.id == want) {
        return;
    }
    match host.set_active(&want) {
        Ok(()) => tracing::info!(harness = %want, "Answering again with the chosen mind"),
        Err(e) => tracing::warn!(harness = %want, error = %e, "Could not restore the chosen mind"),
    }
}

/// Put the current list of minds in front of the person.
fn publish(ui: &App, host: &Host) {
    let entries = host.list();
    let active = host.active_id();

    let rows: Vec<HarnessData> = entries
        .iter()
        .map(|e| HarnessData {
            id: e.id.clone().into(),
            name: e.name.clone().into(),
            detail: e.detail.clone().unwrap_or_default().into(),
            builtin: e.builtin,
            active: e.active,
            tools: e.capabilities.tools,
            memory: e.capabilities.memory,
            status: if e.active {
                "answering".into()
            } else if e.builtin {
                "built in".into()
            } else {
                "attached".into()
            },
        })
        .collect();

    if let Some(model) = crate::models::changed(ui.get_harnesses(), rows) {
        ui.set_harnesses(model);
    }
    ui.set_harness_count(entries.len() as i32);

    // The status bar shows the NAME, not the id: it is read at a glance by a person, and `mind`
    // beside the clock says less than "Yantrik Mind".
    let name = entries
        .iter()
        .find(|e| e.active)
        .map(|e| e.name.clone())
        .unwrap_or_else(|| "no mind".to_string());
    ui.set_active_harness_name(name.into());
    ui.set_active_harness_id(active.into());

    // What the answering mind says it is running on — its model, its memory, wherever it lives.
    // Only an attached one has this; the built-in's model is the shell's own configuration and
    // the rail keeps showing that when there is nothing better. The two are not interchangeable:
    // with a harness driving, the rail read "qwen3.5:9b" from a settings file while every answer
    // came from a different model on a different machine.
    let driving = entries.iter().find(|e| e.active && !e.builtin);
    let detail = driving.and_then(|e| e.detail.clone()).unwrap_or_default();
    ui.set_active_harness_detail(detail.into());
    // Separate from the detail above, because a harness may attach without saying what it runs
    // on. "Something else is answering" is true either way, and it is what the status bar needs
    // in order to stop advertising the shell's own provider as the thing doing the work.
    ui.set_harness_driving(driving.is_some());

    super::runs_on_card::publish(ui);
    publish_catalogue(ui, &entries);
}

/// Put the Settings list in front of the person: every mind this machine could have.
///
/// Deliberately not the same list as above. That one is the picker and holds only what can be
/// handed a turn; this one is what a person opens *because* a mind is missing, and its whole
/// point is the rows that are not attached.
fn publish_catalogue(ui: &App, entries: &[yantrik_harness::Entry]) {
    let busy = crate::harness_install::busy();
    let looking = ui.get_current_screen() == SETTINGS_SCREEN
        && ui.get_settings_category() == HARNESSES_SECTION;
    // The first pass always runs, so the page is populated before anyone can navigate to it.
    let first = ui.get_harness_rows().row_count() == 0;
    if !looking && !busy && !first {
        return;
    }

    // A job's outcome stops being news once the harness it was for is answering questions. The
    // row is about the present, and "install finished" on a mind that is now attached is the
    // page still talking about five minutes ago.
    let attached: Vec<String> = entries.iter().map(|e| e.id.clone()).collect();
    crate::harness_install::clear_settled(&attached);

    let machine = harness_catalogue::machine(crate::harness_install::views());
    let runs_on = super::runs_on_card::current();
    let rows: Vec<HarnessRowData> = harness_catalogue::rows(&machine, entries)
        .into_iter()
        .map(|mut row| {
            // What it runs on, from the one resolver, in the map's own words. A provider handed
            // to it from Settings keeps saying so; otherwise its own report, or nothing when it
            // is not attached and so has said nothing.
            if !row.provider_line.starts_with("Provider: ") || row.provider_line == "Provider: its own settings" {
                let line = super::runs_on_card::row_line(&runs_on, &row.id);
                if !line.is_empty() {
                    row.provider_line = line;
                }
            }
            row
        })
        .map(|row| HarnessRowData {
            id: row.id.into(),
            name: row.name.into(),
            detail: row.detail.into(),
            state: row.state.label().into(),
            need: row.need.into(),
            log: row.log.into(),
            builtin: row.builtin,
            active: row.active,
            attached: row.attached,
            busy: row.busy,
            tools: row.tools,
            memory: row.memory,
            can_install: row.can_install,
            can_start: row.can_start,
            configure_label: row.configure.into(),
            provider_line: row.provider_line.into(),
            can_assign_provider: row.can_assign_provider,
            can_revert_provider: row.can_revert_provider,
            docs: row.docs.into(),
        })
        .collect();

    if let Some(model) = crate::models::changed(ui.get_harness_rows(), rows) {
        ui.set_harness_rows(model);
    }
    // Drives the one animation on the page, and only while something is really running.
    ui.set_harness_busy(busy);
}

/// How often a shell whose harness socket another process holds looks again (#367).
const HELD_RETRY: std::time::Duration = std::time::Duration::from_secs(5);

/// Serve the `harness` socket for the life of the shell.
///
/// While another running process holds the socket, the shell never takes it from it (that one may
/// be serving a session of its own), but it no longer gives up either (#367): on VM 520 a stale
/// shell kept the socket for two days, every newer shell logged one refusal and ran with no minds,
/// and the person saw a working desktop that no mind could reach. Now the shell says so on the
/// status bar, naming the process, and looks again every few seconds, taking the socket the moment
/// it is free.
fn serve_socket(host: Host, ui: slint::Weak<App>) {
    std::thread::Builder::new()
        .name("harness-socket".into())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(runtime) => runtime,
                Err(e) => {
                    tracing::warn!(error = %e, "No runtime; nothing can attach as a mind");
                    return;
                }
            };
            let address = yantrik_ipc_transport::server::RpcServer::default_address("harness");
            let mut said: Option<String> = None;
            loop {
                let notice = held_notice(std::path::Path::new(&address));
                if notice != said {
                    match &notice {
                        Some(n) => tracing::warn!(address = %address, "{n}; looking again every {}s", HELD_RETRY.as_secs()),
                        None if said.is_some() => tracing::info!(address = %address, "Harness socket is free again; taking it"),
                        None => {}
                    }
                    let shown = notice.clone().unwrap_or_default();
                    let _ = ui.upgrade_in_event_loop(move |ui| ui.set_minds_notice(shown.into()));
                    said = notice.clone();
                }
                if notice.is_some() {
                    std::thread::sleep(HELD_RETRY);
                    continue;
                }
                tracing::info!(address = %address, "Harness socket listening (attach to answer)");
                let server = yantrik_ipc_transport::server::RpcServer::new(&address);
                let service = HarnessService::new(host.clone());
                let served = runtime.block_on(server.serve(Arc::new(service)));
                match served {
                    // Taken between the look and the bind: say so and look again.
                    Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => continue,
                    Err(e) => {
                        // Not fatal: a shell whose harness socket died still has its companion,
                        // and taking the desktop down over it would be the worse outcome.
                        tracing::warn!(error = %e, "Harness socket stopped; only built-in minds remain");
                        return;
                    }
                    Ok(()) => return,
                }
            }
        })
        .ok();
}

/// What the status bar says while another process holds the harness socket at `path`, naming it
/// so the person (or whoever looks) knows what to stop; `None` when the socket is free to take.
fn held_notice(path: &std::path::Path) -> Option<String> {
    use yantrik_ipc_transport::owner::{self, Holder};
    if owner::who_holds(path, owner::CLAIM_PING) == Holder::Nobody {
        return None;
    }
    let pid = std::os::unix::net::UnixStream::connect(path).ok().and_then(|s| owner::peer_of(&s)).map(|p| p.pid);
    Some(match pid {
        Some(pid) if pid as u32 != std::process::id() => {
            let what = owner::exe_of(pid)
                .map(|exe| exe.rsplit('/').next().unwrap_or(&exe).to_string())
                .unwrap_or_else(|| "another process".to_string());
            format!("Minds can't reach this desktop: {what} (pid {pid}) holds their socket")
        }
        _ => "Minds can't reach this desktop: another process holds their socket".to_string(),
    })
}

struct HarnessService {
    host: Host,
    /// Whether the mind door was served when this socket was bound, decided then and kept. Asked
    /// afresh on every attach it could be switched off by any of the person's processes, which
    /// own the door directory: a chmod, an attach as `mind` that replaces the real Mind, and a
    /// chmod back (#448 review).
    door_served: bool,
}

impl HarnessService {
    fn new(host: Host) -> HarnessService {
        HarnessService { host, door_served: yantrik_ipc_transport::mind_door::serving_dir().is_some() }
    }
}

impl yantrik_ipc_transport::server::ServiceHandler for HarnessService {
    fn service_id(&self) -> &str {
        "harness"
    }

    fn handle(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, yantrik_ipc_contracts::email::ServiceError> {
        self.handle_from(method, params, None)
    }

    /// Told who is on the other end, so the host can record which process attached. An agent's
    /// token is only believed from that process or one it started — the pid is the kernel's
    /// word (`SO_PEERCRED`, read at accept), never the harness's own.
    fn handle_from(
        &self,
        method: &str,
        params: serde_json::Value,
        peer: Option<yantrik_ipc_transport::PeerCred>,
    ) -> Result<serde_json::Value, yantrik_ipc_contracts::email::ServiceError> {
        // 0 is what the transport writes when the kernel gave no pid.
        let pid = peer.and_then(|p| u32::try_from(p.pid).ok()).filter(|pid| *pid > 0);
        let uid = peer.map(|p| p.uid);
        // Private mode: every harness is an agent, and no agent is served while it is on — at the
        // door (where the transport already refuses) and on the person's own socket, where a
        // harness running as the person attaches and polls.
        let refused = if crate::private_mode::is_on() {
            Some(yantrik_ipc_transport::privacy::REFUSAL.to_string())
        } else {
            first_party_claim_refused(
                method,
                &params,
                uid,
                self.door_served,
                yantrik_ipc_transport::mind_door::is_mind,
            )
        };
        let answer = match refused {
            Some(why) => Err(why),
            None => self.host.handle_from(method, &params, pid, uid),
        };
        answer.map_err(|message| yantrik_ipc_contracts::email::ServiceError { code: -32000, message })
    }
}

/// Why an attach under the first-party Yantrik Mind's id is refused, or `None` to let it through.
///
/// The id is only a name the harness gives itself, and it is the one the person's memory grants
/// treat as the person's own Mind (#447). Where the mind door is served, the real Mind runs as the
/// mind account and attaches with that uid, so a process that is not that account and calls
/// itself `mind` is an impostor. Refused, it cannot take the name at all, nor replace the real
/// Mind's session by re-attaching under it. A caller the kernel could not name (the TCP dev path)
/// cannot show it is the account either.
///
/// Where no door is served, as on an install the account migration has not reached, the Mind
/// runs as a unit of the person's own and attaches with the person's uid, so the name is let
/// through. It earns nothing by it: the first-party memory defaults still go only to the mind
/// account, which is decided from the uid kept at attach.
fn first_party_claim_refused(
    method: &str,
    params: &serde_json::Value,
    uid: Option<u32>,
    door_served: bool,
    is_mind: impl Fn(u32) -> bool,
) -> Option<String> {
    if method != yantrik_harness::protocol::ATTACH
        || params["id"].as_str() != Some(crate::memory_grants::FIRST_PARTY_MIND)
        || !door_served
        || uid.is_some_and(is_mind)
    {
        return None;
    }
    Some(format!(
        "`{}` is the id of the person's own Yantrik Mind, which attaches from its own account; attach under a different id",
        crate::memory_grants::FIRST_PARTY_MIND
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness_catalogue::{Manifest, Machine, State, Unit};
    use yantrik_harness::protocol;

    /// #367: a socket another live process answers on is reported, not taken, and a socket file
    /// nobody listens on any more is free.
    #[test]
    fn a_harness_socket_someone_else_answers_on_is_named_and_a_dead_one_is_free() {
        use std::io::{BufRead, BufReader, Write};
        let dir = std::env::temp_dir().join(format!("yantrik-367-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("harness.sock");
        let _ = std::fs::remove_file(&path);
        assert_eq!(held_notice(&path), None, "nothing there: free");

        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let serving = std::thread::spawn(move || {
            // Answer every line of the first few connections the way a running shell does.
            for stream in listener.incoming().take(3).flatten() {
                let mut out = stream.try_clone().unwrap();
                for line in BufReader::new(stream).lines().map_while(Result::ok) {
                    let _ = line;
                    let _ = out.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":\"harness\"}\n");
                }
            }
        });
        let said = held_notice(&path).expect("a live holder is reported");
        assert!(said.starts_with("Minds can't reach this desktop:"), "{said}");
        drop(serving);

        // A socket file left by a process that is gone is free to take.
        let _ = std::fs::remove_file(&path);
        drop(std::os::unix::net::UnixListener::bind(&path).unwrap());
        assert_eq!(held_notice(&path), None, "a socket nobody listens on is free");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The chip is wired: the shell sets it, the status bar draws it, and its click goes to Minds.
    #[test]
    fn the_minds_notice_reaches_the_status_bar_and_opens_minds() {
        let wiring = include_str!("harness.rs");
        assert!(wiring.contains("ui.set_minds_notice("));
        let app = include_str!("../../../yantrik-ui-slint/ui/app.slint");
        assert!(app.contains("minds-notice: root.minds-notice;"));
        let at = app.find("minds-notice-clicked =>").expect("the chip's click is handled");
        assert!(app[at..at + 200].contains("root.settings-category = 8;"), "it opens Minds");
        let bar = include_str!("../../../yantrik-ui-slint/ui/components/status_bar.slint");
        // The notice is its own chip, drawn only while there is something to say, and its click is the
        // bar's `minds-notice-clicked`.
        assert!(bar.contains("if root.minds-notice != \"\" : BarSlot"));
        assert!(bar.contains("clicked => { root.minds-notice-clicked(); }"));
    }

    /// #447: the first-party Mind's id is the mind account's alone wherever the door is served.
    #[test]
    fn only_the_mind_account_attaches_as_the_first_party_mind_where_the_door_is_served() {
        let mind_account = |uid: u32| uid == 990;
        let attach = |id: &str| serde_json::json!({ "id": id, "name": "Yantrik Mind" });
        let refused = |params: &serde_json::Value, uid: Option<u32>, door: bool| {
            first_party_claim_refused(protocol::ATTACH, params, uid, door, mind_account)
        };

        // The door is served: the person's own processes, and a caller nobody can name, are
        // refused the name; the mind account is not.
        let why = refused(&attach("mind"), Some(1000), true).expect("a person's process is refused");
        assert!(why.contains("`mind`") && why.contains("different id"), "{why}");
        assert!(refused(&attach("mind"), None, true).is_some(), "an unnamed caller cannot show it is the account");
        assert_eq!(refused(&attach("mind"), Some(990), true), None);

        // Any other id is anybody's, and any other method is not an attach.
        assert_eq!(refused(&attach("pi"), Some(1000), true), None);
        assert_eq!(refused(&attach("Mind"), Some(1000), true), None, "another name, and no defaults with it");
        assert_eq!(
            first_party_claim_refused(protocol::POLL, &attach("mind"), Some(1000), true, mind_account),
            None
        );

        // No door, as before the account migration: the Mind runs as the person and attaches so.
        assert_eq!(refused(&attach("mind"), Some(1000), false), None);
    }

    /// The socket hands the host the account the kernel named at accept, beside the pid, and the
    /// host keeps both for whoever later asks who holds a memory credential (#447).
    #[test]
    fn the_harness_socket_keeps_the_account_the_kernel_named_at_attach() {
        use yantrik_ipc_transport::server::ServiceHandler;
        let service = HarnessService { host: Host::new(vec![]), door_served: false };
        // A test process serves no door, so the first-party id goes through here, as it does on
        // an install the account migration has not reached.
        let params = serde_json::json!({ "id": "mind", "name": "Yantrik Mind", "conversations": true });
        let me = unsafe { libc::getuid() };
        let peer = yantrik_ipc_transport::PeerCred { pid: std::process::id() as i32, uid: me, gid: me };
        service.handle_from(protocol::ATTACH, params, Some(peer)).unwrap();
        let agent = service.host.start_agent("mind").unwrap();
        let credential =
            service.host.memory_credential(&agent, yantrik_ipc_transport::reach::token_digest).unwrap().unwrap();
        let held = service.host.memory_credential_holder(&credential).unwrap();
        assert_eq!((held.pid, held.uid), (Some(std::process::id()), Some(me)));
    }

    /// Where the door was served when the socket was bound, a harness of the person's cannot
    /// attach as the first-party Mind, and nothing it does to the door directory afterwards
    /// changes that: the answer was decided at bind and kept.
    #[test]
    fn the_door_decided_at_bind_keeps_the_first_party_name_for_the_mind_account() {
        use yantrik_ipc_transport::server::ServiceHandler;
        let service = HarnessService { host: Host::new(vec![]), door_served: true };
        let params = serde_json::json!({ "id": "mind", "name": "Yantrik Mind", "conversations": true });
        let me = unsafe { libc::getuid() };
        let peer = yantrik_ipc_transport::PeerCred { pid: std::process::id() as i32, uid: me, gid: me };
        let refused = service.handle_from(protocol::ATTACH, params, Some(peer)).unwrap_err();
        assert!(refused.message.contains("person's own Yantrik Mind"), "{}", refused.message);
        assert!(!refused.message.contains("  "), "one sentence, no stray spaces: {}", refused.message);
        assert!(service.host.list().iter().all(|e| e.id != "mind"), "nothing attached as mind");
    }

    /// pi's manifest, plus what — if anything — systemd says about its unit. The manifest
    /// needs nothing the machine cannot already answer for, so the unit is the one fact that
    /// could disagree with the registry — and the disagreement the row must not make (#67).
    fn pi_machine(unit: Option<Unit>) -> Machine {
        let mut machine = Machine::default();
        machine.manifests.insert(
            "pi".into(),
            Manifest {
                id: "pi".into(),
                name: "Pi".into(),
                unit: "yantrik-pi.service".into(),
                ..Default::default()
            },
        );
        if let Some(unit) = unit {
            machine.units.insert("yantrik-pi.service".into(), unit);
        }
        machine
    }

    /// A host with pi attached without peer credentials — the shape of the TCP dev path, where
    /// presence is left to the grace of missed polls. The registry has promised a mind; the
    /// question is what the row and the chooser do with that promise.
    fn host_with_pi() -> Host {
        let host = Host::new(vec![]);
        host.handle(protocol::ATTACH, &serde_json::json!({ "id": "pi", "name": "Pi" })).unwrap();
        host
    }

    #[test]
    fn a_hand_started_mind_is_chosen_even_while_its_unit_says_stopped() {
        // #67 in the other direction: the fix lives in the registry, which drops a session the
        // moment the kernel says its process is gone — not in a rule that prefers a stopped
        // unit over a mind the registry lists. A harness started from a terminal polls happily
        // while its unit file sits installed and inactive; refusing it here would take *Use
        // this* away from a mind that is answering.
        let host = host_with_pi();
        let stopped = Unit { loaded: true, enabled: true, ..Default::default() };
        let machine = pi_machine(Some(stopped));
        assert_eq!(row_refusal(&machine, &host.list(), "pi"), None);
        // The row the person sees agrees with the action: attached, and able to answer.
        let rows = harness_catalogue::rows(&machine, &host.list());
        let pi = rows.iter().find(|r| r.id == "pi").unwrap();
        assert_eq!(pi.state, State::Answering);
        assert!(pi.attached && pi.state.can_answer());
        assert_eq!(pi.need, "");
        // A mind whose process really died is refused before ever reaching the row: the
        // registry has dropped it, and `set_active` answers with its own sentence naming what
        // is attached (yantrik-harness's host tests pin that).
        choose(&host, "pi").unwrap();
        assert_eq!(host.active_id(), "pi");
    }

    #[test]
    fn the_refusal_defers_to_the_registry_where_the_row_cannot_contradict_it() {
        let host = host_with_pi();
        // systemd has no record at all — a container without a user manager, or a harness run
        // by hand while its unit file was never installed. The attachment is the fresher fact.
        assert_eq!(row_refusal(&pi_machine(None), &host.list(), "pi"), None);
        // A unit genuinely running: the promise stands and nothing is refused.
        let up = Unit { loaded: true, active: true, ..Default::default() };
        assert_eq!(row_refusal(&pi_machine(Some(up)), &host.list(), "pi"), None);
        // An id the registry does not hold gets `set_active`'s sentence — the list of what is
        // attached — not a row's.
        assert_eq!(row_refusal(&pi_machine(None), &host.list(), "hermes"), None);
    }
}

#[cfg(test)]
mod late_answer_tests {
    use super::*;

    fn late(harness: &str, text: &str) -> yantrik_harness::LateAnswer {
        yantrik_harness::LateAnswer { harness: harness.into(), turn_id: 7, conversation: "main".into(), text: text.into() }
    }

    #[test]
    fn a_late_answer_notice_is_a_fixed_sentence_that_never_quotes_the_answer() {
        let answer = "Approval needed: allow files_delete ~/Documents? hunter2";
        let said = format!("{:?}", late_answer_notice_for(&late("pi", answer), false, false, None).unwrap());
        assert!(said.contains("pi answered after you left. Open the chat to read it."), "{said}");
        for leak in ["Approval", "files_delete", "hunter2"] {
            assert!(!said.contains(leak), "{leak} leaked: {said}");
        }
    }

    #[test]
    fn no_late_answer_notice_is_raised_in_private_mode() {
        assert!(late_answer_notice_for(&late("pi", "hello"), true, false, None).is_none());
    }

    /// While a test run has approvals off, a late answer is not told; with it off, or once it has
    /// run out, it is told as before.
    #[test]
    fn no_late_answer_notice_is_raised_during_a_test_run() {
        use crate::never_ask::NeverAsk;
        use std::time::{Duration, Instant};
        let start = Instant::now();
        let mut switch = NeverAsk::default();
        assert!(late_answer_notice_for(&late("pi", "hello"), false, switch.active(start), None).is_some(), "off");
        switch.engage(start, 5);
        assert!(late_answer_notice_for(&late("pi", "hello"), false, switch.active(start), None).is_none(), "on");
        let later = start + Duration::from_secs(300);
        assert!(late_answer_notice_for(&late("pi", "hello"), false, switch.active(later), None).is_some(), "run out");
    }

    #[test]
    fn the_name_in_a_late_answer_notice_is_clipped_and_stripped() {
        let evil = format!("pi\u{202e}\u{7}\n{}", "x".repeat(100));
        let name = notice_name(&evil, None);
        assert!(!name.contains('\u{202e}') && !name.contains('\u{7}') && !name.contains('\n'), "{name:?}");
        assert!(name.chars().count() <= 41, "{name:?}");
    }

    #[test]
    fn the_catalogue_name_is_preferred_and_a_blank_one_falls_back() {
        assert_eq!(notice_name("hermes-x", Some("Hermes")), "Hermes");
        assert_eq!(notice_name("hermes-x", Some("  \u{202e} ")), "hermes-x");
        assert_eq!(notice_name("pi", None), "pi");
    }

}
