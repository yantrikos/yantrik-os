//! What the Mind may ask Network by name: the same six actions, with the same names,
//! descriptions, parameters and grades as before, each the same command a button runs.
//!
//! The id is `network`, which is what `crates/yantrik-ui/src/wire/dock.rs` routes `network` and
//! `network_manager` to. The socket is `app-network.sock`, beside the service's own
//! `network.sock`: the service stores nothing and answers about the machine, the app is the
//! window someone is looking at, and `yos` tries `app-network` first.
//!
//! # The grades, and why
//!
//! This machine is reached over the network. That fact decides three of these six.
//!
//! `wifi_disconnect` and `wifi_radio` are **dangerous**, not sensitive. The ladder's own rule is
//! "anything that destroys work or state a person cannot get back". What these destroy is not
//! data, it is the channel: a caller driving this machine from somewhere else and turning the
//! radio off has cut the wire it would have sent the undo down. Nothing in the surface can put
//! it back, and no amount of care at the call site makes it recoverable — it needs someone in
//! the room. `stop` on a container is `sensitive` because a start is one call away; there is no
//! such call here. `wifi_radio` carries the grade for its worst argument, because the ladder
//! grades actions and not argument values, and `off` is the worst argument.
//!
//! `wifi_connect` is **sensitive**. It hands this machine to whatever is answering to that SSID,
//! which is worth a deliberate decision; it does not take the machine off the network it is on
//! — a wired link is untouched, and a failed association leaves the previous one standing. The
//! password is not an argument and cannot become one (#178): a network this machine has not
//! saved opens the window's own prompt and waits for a person to type into it.
//!
//! `wifi_forget` is **sensitive**, and it is only honestly sensitive because the service refuses
//! to forget the network currently in use: `nmcli connection delete` on the active profile takes
//! the link down as a side effect, which would have made this a `dangerous` thing wearing a
//! `sensitive` grade. What it does destroy is a stored credential nobody may have written down
//! — state that outlives the turn (#48).
//!
//! `wifi_scan` and `refresh` are **standard**, the floor. A scan puts the radio off the air for
//! a few seconds and changes nothing.
//!
//! No password is echoed by any of this: not in a result, not in `describe`, not in the notice.
//! None is taken in either — the only road a secret travels is the window's prompt to the
//! service, and no action argument, card or audit line is on it.
//!
//! # Settling
//!
//! Every change now runs in a job, so each intent is answered when its outcome is known
//! (`Settle::Explicit`): `refresh` and the three changes when the service has answered and the
//! machine has been read again, as before; `wifi_scan` and `wifi_connect`, which declare
//! `defers`, at once, as before, with the result arriving in `describe` and the notice.
//!
//! Changes reach the service one at a time (`lane.rs`). While one is in flight, the three
//! changes an intent waits on are refused with `busy`, and nothing is changed. `wifi_connect`,
//! answered once accepted, waits its turn instead.

use std::time::Duration;

use serde_json::json;
use yantrik_app::control::{Action, Param};
use yantrik_app::intents::{Intent, Intents, Settle};
use yantrik_app::Refused;

use crate::connect;
use crate::model::{Command, Model};
use crate::NetworkManager;

/// How long `refresh` and the three changes may take to answer: 14 s.
///
/// The ceiling is the Mind's. Its `app_action` client gives up after `ACT_TIMEOUT`, 15 s
/// (`crates/yantrik-companion-tools/src/app_ui.rs`). An answer later than that reaches nobody,
/// and the Mind sees a transport timeout for a change that may well have gone through, and may
/// try a `dangerous` one again. Settling within 14 s means the app always answers first, with
/// the outcome or with its own "did not settle" refusal, and leaves a second for the hop back.
///
/// The floor is the work. Every call to network-service is bounded by the client's 2 s default
/// timeout. A refresh is six calls, so 12 s at worst. A change is one call and a refresh, so
/// 14 s at worst. Only a service failing every call at the last moment reaches that, and it
/// still gets an answer before the Mind stops listening.
const CHANGE_WITHIN: Duration = Duration::from_secs(14);
/// A refresh needs 12 s at worst. It is held to the same 14 s, under the Mind's 15 s, for the
/// same reason.
const REFRESH_WITHIN: Duration = CHANGE_WITHIN;
/// The deferred two answer once they have started (or been refused).
const START_WITHIN: Duration = Duration::from_secs(5);

/// An argument as the old handlers read it: the text, trimmed, or empty.
fn trimmed(args: &yantrik_app::Args<'_>, name: &str) -> String {
    args.raw()[name].as_str().unwrap_or_default().trim().to_string()
}

pub fn table(table: &mut Intents<NetworkManager>) {
    table.add(
        Intent::new(
            Action::new("refresh", "Re-read interfaces, Wi-Fi, resolvers and the firewall"),
            |_| Ok(Command::Refresh),
        )
        .settles(Settle::Explicit { within: REFRESH_WITHIN })
        .reply(|m: &Model| {
            json!({
                "connected": m.reading.status.connected(),
                "type": m.reading.status.kind,
                "wifi_adapter_present": m.reading.wifi.adapter_present,
                "firewall_state": m.reading.firewall.state.as_str(),
                "service_answered": m.reading.service_ok,
            })
        }),
    );
    table.add(
        // Defers: a rescan sweeps 2.4 and 5 GHz and the service allows it ten seconds. The list
        // arrives in `describe` under `wifi.in_range`.
        Intent::new(
            Action::new("wifi_scan", "Look for Wi-Fi networks in range").defers(),
            |_| Ok(Command::ScanAsked),
        )
        .settles(Settle::Explicit { within: START_WITHIN })
        .reply(|_: &Model| {
            json!({
                "scanning": true,
                "settles": "wifi.in_range and notice in describe",
            })
        }),
    );
    table.add(
        // Sensitive, and deferred: an association waits on a WPA handshake and on DHCP. The
        // published parameter list — and the reason there is no `password` in it — lives in
        // `connect::wifi_connect_action`.
        Intent::new(connect::wifi_connect_action(), |a| {
            let ssid = trimmed(a, "ssid");
            if ssid.is_empty() {
                return Err(Refused::new("missing_ssid", "a network name is needed to connect"));
            }
            Ok(Command::Join(ssid))
        })
        .settles(Settle::Explicit { within: START_WITHIN })
        .reply(|m: &Model| m.answers.connect.clone()),
    );
    table.add(
        // Dangerous: on a machine reached over the network this takes away the channel the undo
        // would travel on.
        Intent::new(
            Action::new(
                "wifi_disconnect",
                "Leave the Wi-Fi network this machine is on. On a machine reached over Wi-Fi \
                 this ends that connection and cannot be undone remotely.",
            )
            .risk("dangerous"),
            |_| Ok(Command::Disconnect),
        )
        .settles(Settle::Explicit { within: CHANGE_WITHIN })
        .reply(|m: &Model| {
            let state = m.answers.disconnect.clone().unwrap_or_default();
            json!({
                "disconnected": true,
                "connected_ssid": state.connected_ssid,
                "radio": state.radio.as_str(),
            })
        }),
    );
    table.add(
        // Sensitive: a stored credential, gone. The service refuses the active network, so this
        // cannot take the link down as a side effect.
        Intent::new(
            Action::new(
                "wifi_forget",
                "Delete a saved Wi-Fi network, and the password saved with it, from this machine. \
                 It cannot be undone: joining it again needs the password again.",
            )
            .risk("sensitive")
            .arg(Param::text("ssid").describe("The saved network to delete")),
            |a| {
                let ssid = trimmed(a, "ssid");
                if ssid.is_empty() {
                    return Err(Refused::new("missing_ssid", "a network name is needed to forget one"));
                }
                Ok(Command::Forget(ssid))
            },
        )
        .settles(Settle::Explicit { within: CHANGE_WITHIN })
        .reply(|m: &Model| match &m.answers.forget {
            Some(result) => json!({
                "forgotten": result.forgotten,
                "known": result.known.iter().map(|k| k.ssid.clone()).collect::<Vec<_>>(),
            }),
            None => json!({}),
        }),
    );
    table.add(
        // Dangerous, for the same reason as disconnect and one worse: a machine whose radio is
        // off cannot be told to turn it back on over the radio.
        Intent::new(
            Action::new(
                "wifi_radio",
                "Turn the Wi-Fi radio on or off. Turning it off on a machine reached over Wi-Fi \
                 ends that connection and cannot be undone remotely.",
            )
            .risk("dangerous")
            .arg(Param::text("state").describe("\"on\" or \"off\"")),
            |a| match trimmed(a, "state").to_lowercase().as_str() {
                "on" => Ok(Command::SetRadio(true)),
                "off" => Ok(Command::SetRadio(false)),
                // Named rather than guessed at. A `dangerous` action must not decide for itself
                // what an unrecognised word meant.
                other => Err(Refused::new(
                    "bad_state",
                    format!("state must be \"on\" or \"off\"; got \"{other}\""),
                )),
            },
        )
        .settles(Settle::Explicit { within: CHANGE_WITHIN })
        .reply(|m: &Model| {
            let state = m.answers.radio.clone().unwrap_or_default();
            json!({
                "radio": state.radio.as_str(),
                "connected_ssid": state.connected_ssid,
            })
        }),
    );
}
