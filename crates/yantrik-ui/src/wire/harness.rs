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
        let tokens = self.bridge.send_message(turn.text);
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

pub fn wire(ui: &App, ctx: &AppContext) {
    let host = Host::new(vec![Arc::new(Companion { bridge: ctx.bridge.clone() })]);
    let _ = HOST.set(host.clone());

    // The agent terminal's side of agents (design/agents-workspace-2026-09-23.md, decision 3):
    // `agent_run` and the rest believe a token only as this host issued it and only from under
    // the harness it was issued to, and a command that ends after its call returned is noted
    // into its agent's next turn.
    crate::control_agent_terminal::serve_host(&host);

    serve_socket(host.clone());

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
    let rows: Vec<HarnessRowData> = harness_catalogue::rows(&machine, entries)
        .into_iter()
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
            docs: row.docs.into(),
        })
        .collect();

    if let Some(model) = crate::models::changed(ui.get_harness_rows(), rows) {
        ui.set_harness_rows(model);
    }
    // Drives the one animation on the page, and only while something is really running.
    ui.set_harness_busy(busy);
}

/// Serve the `harness` socket for the life of the shell.
fn serve_socket(host: Host) {
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
            runtime.block_on(async {
                let address =
                    yantrik_ipc_transport::server::RpcServer::default_address("harness");
                tracing::info!(address = %address, "Harness socket listening (attach to answer)");
                let server = yantrik_ipc_transport::server::RpcServer::new(&address);
                if let Err(e) = server.serve(Arc::new(HarnessService { host })).await {
                    // Not fatal: a shell whose harness socket died still has its companion, and
                    // taking the desktop down over it would be the worse outcome.
                    tracing::warn!(error = %e, "Harness socket stopped; only built-in minds remain");
                }
            });
        })
        .ok();
}

struct HarnessService {
    host: Host,
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
        self.host.handle_from(method, &params, pid).map_err(|message| {
            yantrik_ipc_contracts::email::ServiceError { code: -32000, message }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness_catalogue::{Manifest, Machine, State, Unit};
    use yantrik_harness::protocol;

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
