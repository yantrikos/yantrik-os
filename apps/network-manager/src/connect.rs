//! The windowless half of joining a Wi-Fi network.
//!
//! `wifi_connect` used to declare the network's password as an ordinary action argument
//! (#178). An action's arguments are what the approval card draws, what
//! `record_unasked_action` writes into `mind-audit.jsonl`, what a grant is bound to and what
//! the answer echoes — so a declared `password` parameter handed the secret to every
//! recording surface in the system before anything connected, and `yos check network`
//! rightly refused to pass. The action now takes the network name and nothing else. When the
//! network is not saved on this machine, the action raises the window's own password prompt
//! and the person types into it; what they type goes from the prompt to network-service and
//! never passes through an action argument, a card, a log line or a grant.
//!
//! Nothing in this file imports Slint or opens a socket, so `tests/network-core` can exercise
//! all of it — the same arrangement as `apps/email/src/state.rs`.

use yantrik_ipc_contracts::control_surface::{Action, Param};
use yantrik_ipc_contracts::network::{KnownNetwork, WifiConnectParams, WifiState};

/// What `wifi_connect` does with one network name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    /// NetworkManager already holds this network's credential, so the machine joins on its
    /// own — exactly what happens when the person clicks a saved row in the list.
    Join,
    /// Nothing is saved here, so the secret can only come from the person: the action raises
    /// the window's password prompt and the connect waits on what is typed into it.
    AskPerson,
}

/// Which of the two the action takes. This is the rule the person's own click follows in
/// `network_manager.slint`: a saved network joins outright, any other raises the password
/// dialog. The two paths stay one behaviour, so the mind cannot reach a network the person
/// could not, or skip a prompt the person would have seen.
pub fn plan(ssid: &str, known: &[KnownNetwork]) -> Plan {
    let ssid = ssid.trim();
    if known.iter().any(|k| k.ssid == ssid) {
        Plan::Join
    } else {
        Plan::AskPerson
    }
}

/// The action as published. The password is not a parameter here and cannot become one
/// without tripping both `yos check network` and the test beside this file: names like
/// `password` are refused wherever an action argument appears.
///
/// The description says what the mind has to tell the person, because a deferred action that
/// is waiting on a human should not read as one that failed.
pub fn wifi_connect_action() -> Action {
    Action::new(
        "wifi_connect",
        "Join a Wi-Fi network by name. If this machine has not saved the network, its \
         password is typed by the person: the action opens the Network window's own password \
         prompt and the connect waits on it. The password is never an argument of this \
         action, never in a card, a log or an answer.",
    )
    .defers()
    .risk("sensitive")
    .arg(Param::text("ssid").describe("The network name to join"))
}

/// What one connect attempt carries to the backend. Both the person's dialog and the action
/// build their request here, so "the box was empty" means the same thing on both paths: no
/// secret, not an empty one. The contract carries `Option` because an empty string used to
/// mean both "open network" and "the person cleared the field".
pub fn join_request(ssid: &str, typed: &str) -> WifiConnectParams {
    WifiConnectParams {
        ssid: ssid.trim().to_string(),
        password: (!typed.is_empty()).then(|| typed.to_string()),
    }
}

/// Where a connect request goes. The app implements this with the socket call to
/// network-service; `tests/network-core` implements it with a recorder.
pub trait Backend {
    fn wifi_connect(&self, request: &WifiConnectParams) -> Result<WifiState, String>;
}

/// The prompt's result, on its way to the backend. The typed secret travels inside the
/// request and inside nothing else: what comes back is the joined state or the service's
/// reason for refusing, and the service builds both without ever naming the secret.
pub fn submit<B: Backend>(backend: &B, ssid: &str, typed: &str) -> Result<WifiState, String> {
    backend.wifi_connect(&join_request(ssid, typed))
}

/// The answer while the machine joins on its own. The SSID, and not one word about a
/// secret: the result of this action must be readable in a log.
pub fn joining_answer(ssid: &str) -> serde_json::Value {
    serde_json::json!({
        "connecting_to": ssid,
        "settles": "wifi.connected_ssid and notice in describe",
    })
}

/// The answer when nothing is saved and only the person can supply the secret. It names the
/// wait and where the typing happens, so the mind can tell the person instead of polling a
/// connect that has not started.
pub fn waiting_answer(ssid: &str) -> serde_json::Value {
    serde_json::json!({
        "waiting_on_person": ssid,
        "prompt": "the Network window is asking the person for this network's password; \
                   what is typed there goes straight to the network service",
        "settles": "wifi.connected_ssid and notice in describe",
    })
}
