//! Yantrik Network Manager, on YantrikApp (design/app-framework-2026-10-21.md, step 10: behaviour
//! unchanged).
//!
//! What this app is for: to say truthfully how this machine is connected, and to change it when
//! asked. Everything else it used to draw — Bluetooth, VPN, diagnostics, firewall profiles — has
//! come off the screen, because behind all eighteen of those controls was a `tracing::info!` and
//! nothing else.
//!
//! # What was wrong, before
//!
//! **The app called five methods the service did not have** (`network.wifi_toggle`, `…_scan`,
//! `…_connect`, `…_disconnect`, `…_forget`), and three of the five went into a `let _ =`, so
//! every press returned "Unknown method" into a discarded `Result` and the app looked like it had
//! worked. **Two readings were fabricated**: `wifi-enabled` and `firewall-enabled` were Slint
//! defaults Rust never wrote, and a security audit of this OS recorded "Firewall: Off" from one.
//! **There was no control surface and nowhere to report a failure.** design/network-2026-09-20.md
//! is the account.
//!
//! # How it is put together now
//!
//! One typed contract (`yantrik_ipc_contracts::network`) that both this app and
//! `services/network-service` build and parse. Every change is one command, raised by the button
//! and by the intent of the same name (`update.rs`), so a person and a mind cannot be told
//! different things about it. Every read and every change runs in a job, never on the UI thread
//! (`backend.rs`, `reading.rs`), changes one at a time (`lane.rs`), and a reading taken before
//! the one on screen is dropped. Every
//! failure lands in the `notice` strip and in `describe.notice`. The agent rail is the
//! framework's; `main.rs` is three lines.

use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::Duration;

use yantrik_app::app::{App, Boot, Changed, Startup, WindowId};
use yantrik_app::control::View;
use yantrik_app::cx::Cx;
use yantrik_app::effects::Effects;
use yantrik_app::intents::Intents;
use yantrik_app::rail::{Rail, RailCommand};
use yantrik_app::timers::TimerId;
use yantrik_app::Refused;

mod backend;
/// The windowless half of connecting: the plan, the published action, the request the typed
/// secret travels in. Pure, so `secret_tests.rs` exercises it without a window.
mod connect;
mod describe;
mod intents;
/// One change to the machine at a time: the one in flight, and the ones waiting behind it.
mod lane;
mod model;
/// Which section the window opens on: the interface the default route leaves by.
mod opening;
mod rail;
mod reading;
mod update;
mod view;
#[cfg(test)]
mod intent_tests;
#[cfg(test)]
mod lane_tests;
#[cfg(test)]
mod secret_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod rest_tests;

pub use model::{Command, Model};
pub use reading::Reading;

slint::include_modules!();
yantrik_app::window!(NetworkManagerApp);
yantrik_app::rail_window!(NetworkManagerApp);

/// The machine is read again this often. A cable pulled out while the window is open should
/// show, and three seconds is the same cadence the shell polls the rest of the system at.
pub const REFRESH: TimerId = TimerId("network.refresh");
pub const REFRESH_EVERY: Duration = Duration::from_secs(3);

pub struct NetworkManager;

impl App for NetworkManager {
    type Ui = NetworkManagerApp;
    type Model = Model;
    type Command = Command;
    type Settings = ();
    const ID: &'static str = "network";

    fn model(boot: &Boot<()>) -> Result<Model, Startup> {
        // Nothing has been asked yet, so nothing is known. The first refresh decides, and until
        // it has landed the window says "unknown" rather than "off".
        Ok(Model {
            reading: Reading::default(),
            read_once: false,
            shown: 0,
            sequence: Arc::new(AtomicU64::new(0)),
            refreshing: 0,
            opened: false,
            section: opening::WIFI,
            notice: String::new(),
            scanning: false,
            in_range: Vec::new(),
            prompt_ssid: String::new(),
            prompt_status: String::new(),
            ai: Default::default(),
            answers: Default::default(),
            rail: Rail::new(boot.mind.clone()),
            mind: boot.mind.clone(),
            net: backend::for_app(&boot.paths),
            lane: Default::default(),
        })
    }

    fn wire(cx: &Cx<Self>) {
        let ui = cx.ui();
        // `enabled` is the state being asked for, never a flip: the switch has already flipped.
        ui.on_wifi_radio_set(cx.cmd1(Command::SetRadio));
        ui.on_wifi_scan(cx.cmd(|| Command::Scan));
        // An empty box is not a password; `join_request` decides that, and it is the same
        // function the intent's path goes through.
        ui.on_wifi_connect(cx.cmd2(|ssid: slint::SharedString, password: slint::SharedString| {
            Command::Connect(connect::join_request(&ssid, &password))
        }));
        ui.on_wifi_disconnect(cx.cmd(|| Command::Disconnect));
        ui.on_wifi_forget(cx.cmd1(|ssid: slint::SharedString| Command::Forget(ssid.to_string())));
        ui.on_ai_explain_pressed(cx.cmd(|| Command::Explain));
        ui.on_ai_dismiss(cx.cmd(|| Command::DismissAi));
        // A rail row goes to the section it is about (`context-opens: true` on the screen).
        ui.on_agent_context_activated(cx.cmd1(|id: slint::SharedString| Command::OpenSection(id.to_string())));
        drop(ui);
        // The suggestion, and the card's Close (an answer has nothing to apply, so both of its
        // buttons dismiss it).
        yantrik_app::rail::wire(cx);
    }

    fn start(_: &Model, fx: &mut Effects<Self>) {
        fx.every(REFRESH, REFRESH_EVERY, Command::Tick);
        fx.then(Command::Refresh);
    }

    fn update(m: &mut Model, command: Command, fx: &mut Effects<Self>) -> Result<(), Refused> {
        update::update(m, command, fx)
    }

    fn render(ui: &NetworkManagerApp, _: WindowId, m: &Model, changed: &Changed) {
        view::render(ui, m, changed);
    }

    fn describe(m: &Model) -> View {
        describe::view(m)
    }

    fn intents(table: &mut Intents<Self>) {
        intents::table(table);
    }

    fn rail(command: RailCommand) -> Option<Command> {
        Some(Command::Rail(command))
    }
}
