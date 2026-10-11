//! The network, operable as data: what the bar's mark and popover show, and what they do.
//!
//! What a pointer can do, a mind can ask for. `describe shell` publishes the one reading the bar
//! draws (`network`, and `wifi_networks` where there is a Wi-Fi device), and three actions do what
//! the popover's controls do:
//!
//!   * `set_wifi enabled=true|false` is the Wi-Fi tile: the radio. It answers with the radio as
//!     NetworkManager reports it afterwards, not with the request.
//!   * `disconnect_network` is the connected row's Disconnect: leave this network. It does not
//!     switch the radio off. It answers with the device's state afterwards.
//!     Both are `dangerous`, as the same verbs already are in the minds' tools (`wifi_disconnect`,
//!     `wifi_radio`) and in the Network Manager app: the same verb is not cheaper because it came
//!     in through the shell. What they destroy is the channel a remote mind reaches the machine
//!     by, and no reading of "another link is up" says that the mind's own path is that link
//!     (`grade_for`). Because no state can lower the grade, nothing that goes stale can make a
//!     call cheaper; the grade is still re-checked when the call runs (`grade_still_holds`), so
//!     a rule that ever does depend on state cannot be decided from a reading that has moved.
//!     A mind cannot take down a wired link at all: nothing in the popover brings it back.
//!   * `connect_wifi ssid=...` is a row. `sensitive`, because joining a network moves every
//!     mind's traffic onto it. It switches between networks the machine already has saved, and
//!     nothing else. A network it has no profile for, secured OR open, is not joined: the row is
//!     MARKED ("Yantrik Mind asked to join X") and the person decides, by clicking it. It does not
//!     open a password field, focus anything, replace a field the person has open or bring the
//!     shell forward: a field that appeared under someone's hands would take their next
//!     keystrokes, and a password typed for something else would go to whatever access point the
//!     mind named. An open network gets the same treatment because in Auto mode a `sensitive` act
//!     runs without a card, and an open attacker's network joined that way would be saved and
//!     rejoined by itself.
//!
//! D-Bus calls never run on the thread that draws: `set_wifi` and `disconnect_network` hand their
//! work to `answer_later`, and every connection has a call timeout (`network.rs`).
//!
//! # No password crosses this surface
//!
//! There is no `password`, `psk` or `passphrase` argument on any action here, on purpose, and no
//! action is named like one. Anything that can open the shell's socket can call an action, and a
//! Wi-Fi password handed to a mind's tool call lands in a transcript, a context window and whatever
//! the answering provider keeps. The only way a password reaches NetworkManager is typed into the
//! popover's own field by a person (`wire::network`). `no_published_action_can_carry_a_passphrase`
//! in `control_approvals` checks this mechanically over every `control*.rs`, and
//! `SECRET_PARAM_PERMITTED` has no entry for this file. `describe` carries network names and
//! signal, never anything the person typed: the field's text is not a property anyone can read.
//!
//! The older `wifi` field and the `online`, `type` and `connection` keys of `network` are still
//! published for one release, because the minds' tools and older `yos-mcp` read them.

use slint::ComponentHandle;
use yantrik_app_runtime::control::{answer_later, published_grade, regrade, Action, App as ControlSurface, Param};
use yantrik_os::network_model::{plan, Plan, Refusal};
use yantrik_os::{ConnectRequest, NetworkSnapshot};

use crate::App;

/// `answer_later` has no slot to leave work in when a handler is called outside a dispatch (an
/// in-process or test call). The work is NOT run inline then: it makes D-Bus calls, and on this
/// thread they would be made on the one that draws.
const NOT_A_DISPATCH: &str = "this action only runs as a dispatched call from the control surface";

/// What `connect_wifi` will do, decided from the reading and nothing else. Pure, so the refusal to
/// connect to a secured unknown network without a person is a test and not a promise.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ConnectPlan {
    /// A saved profile exists: switching to it involves no password and creates nothing.
    Join,
    /// No saved profile, secured or open: only a person may choose to join it.
    AskPerson,
}

pub(crate) fn plan_connect(snapshot: Option<&NetworkSnapshot>, ssid: &str) -> Result<ConnectPlan, String> {
    let snapshot = snapshot.ok_or("the network state is not known yet")?;
    if !snapshot.wifi_present {
        return Err("this machine has no Wi-Fi device".to_string());
    }
    if !snapshot.radio_on {
        return Err("Wi-Fi is switched off; use set_wifi enabled=true first".to_string());
    }
    // The same decision the join itself makes, asked with no secret: it can only say "saved or
    // open, go", "needs a secret", or why not.
    let probe = ConnectRequest { ssid: ssid.to_string(), secret: None, by_person: false };
    match plan(snapshot, &probe, false) {
        Ok(Plan::UseSaved) => Ok(ConnectPlan::Join),
        // An open network nobody has saved is the easiest one to impersonate, and the profile a
        // join creates is rejoined by itself afterwards: a person's choice, not a caller's.
        Ok(Plan::JoinOpen) | Ok(Plan::JoinSecured { .. }) => Ok(ConnectPlan::AskPerson),
        Err(Refusal::NeedsSecret) => Ok(ConnectPlan::AskPerson),
        Err(other) => Err(other.say(ssid)),
    }
}

/// The grade an action carries right now.
///
/// Disconnecting, or switching the Wi-Fi radio off, is `dangerous` here as it is in the companion's
/// tools and in the Network Manager app, whichever links are up: "another link remains" does not
/// mean the caller's own path remains (a mind reaching the box over Wi-Fi with a cable also in is
/// cut off all the same), and a link NetworkManager counts as up may carry nothing. The reading is
/// kept as an argument so a future rule that depends on state has one place to go, but it can only
/// RAISE a grade: nothing here ever publishes less than `dangerous` for these two.
pub(crate) fn grade_for(action: &str, _snapshot: Option<&NetworkSnapshot>) -> &'static str {
    match action {
        "disconnect_network" | "set_wifi" => "dangerous",
        _ => "standard",
    }
}

fn rank(grade: &str) -> usize {
    yantrik_ipc_transport::gate::grade(grade).unwrap_or(usize::MAX)
}

/// Whether the grade the gate asked about is still the grade this action would be asked for now.
///
/// `published` is what the gate read at dispatch; `fresh` is a reading taken as late as the caller
/// can. If the grade has gone UP in between, the approval (or the lack of one) was for something
/// cheaper than what is about to happen, and the call is refused: a retry is asked at the new
/// grade. A reading that is known to be stale is no reading.
pub(crate) fn grade_still_holds(
    action: &str,
    published: &str,
    fresh: Option<&NetworkSnapshot>,
    stale: bool,
) -> Result<(), String> {
    if stale {
        return Err("the network state could not be read just now, so nothing was changed; try again".to_string());
    }
    if rank(grade_for(action, fresh)) > rank(published) {
        return Err(format!(
            "the network changed since `{action}` was graded `{published}`; it is graded `{}` now, so \
             nothing was changed. Ask again",
            grade_for(action, fresh)
        ));
    }
    Ok(())
}

/// Publish those grades. Called on every new reading and on every `describe`, both on the thread
/// that owns the surface (the only one `regrade` answers on); until the first call the actions are
/// declared `dangerous`.
pub(crate) fn sync_grades(snapshot: Option<&NetworkSnapshot>) {
    for action in ["disconnect_network", "set_wifi"] {
        // `Err` is "no surface installed on this thread yet", and the declared grade stands.
        let _ = regrade(action, grade_for(action, snapshot));
    }
}

/// The reading as `describe shell` publishes it under `network`.
///
/// `online`, `type` and `connection` are the keys this object had before it grew the rest; they
/// stay for one release.
pub fn network_for_describe(snapshot: Option<&NetworkSnapshot>, ui: &App) -> serde_json::Value {
    // A describe is what a caller reads before it acts: have the published grades match it.
    sync_grades(snapshot);
    let Some(s) = snapshot else {
        // No reading yet: say what the properties say and no more.
        return serde_json::json!({
            "kind": serde_json::Value::Null,
            "state": serde_json::Value::Null,
            "online": ui.get_network_online(),
            "type": ui.get_network_medium().to_string(),
            "connection": ui.get_network_detail().to_string(),
        });
    };
    let readout = crate::wire::network::readout(s);
    serde_json::json!({
        "kind": s.kind.as_str(),
        "state": s.state.as_str(),
        "ssid": s.ssid,
        "strength": s.strength,
        "ip": s.ip,
        "vpn": s.vpn,
        "connectivity": s.connectivity.as_str(),
        "wifi_device": s.wifi_present,
        "wifi_radio": if s.wifi_present { serde_json::Value::from(s.radio_on) } else { serde_json::Value::Null },
        "popover_open": ui.get_network_open(),
        // True when the last attempt to read NetworkManager failed: everything above is then the
        // last picture that was read, and a caller should not treat it as current.
        "stale": yantrik_os::network::is_stale(),
        // The three keys before this object grew.
        "online": readout.online,
        "type": readout.medium,
        "connection": readout.detail,
    })
}

/// The visible networks, where there is a Wi-Fi device to see them with; `null` where there is not,
/// like `battery` on a machine without one. Names, signal and what is saved: nothing typed.
pub fn wifi_networks_for_describe(snapshot: Option<&NetworkSnapshot>) -> serde_json::Value {
    match snapshot {
        Some(s) if s.wifi_present => serde_json::Value::Array(
            s.access_points
                .iter()
                .map(|p| {
                    serde_json::json!({
                        "ssid": p.ssid,
                        "strength": p.strength,
                        "secured": p.secured,
                        "known": p.known,
                        "connected": p.connected,
                    })
                })
                .collect(),
        ),
        _ => serde_json::Value::Null,
    }
}

/// Mark one row as asked for, and nothing else.
///
/// The row says "Yantrik Mind asked to join X" and the bar's mark gets its attention dot; the person
/// decides by clicking it, and a password, if one is needed, is typed into a field the PERSON
/// opened. This does not open the popover, replace a password row the person already has open,
/// focus a field or raise the shell: any of those would put an access point of a mind's choosing
/// under whatever the person is typing at that moment.
fn mark_for_person(ui: &App, ssid: &str) -> Result<serde_json::Value, String> {
    // Who MADE this call, from the caller's identity, and not whichever mind the person has selected:
    // a call from another process would otherwise be put in the selected mind's mouth.
    let who = match crate::mind_view::requester_now() {
        crate::mind_view::Requester::Mind(name) if !name.trim().is_empty() => name,
        _ => "A mind".to_string(),
    };
    let g = ui.global::<crate::NetworkState>();
    g.set_requested_ssid(ssid.into());
    g.set_requested_by(who.as_str().into());
    Ok(serde_json::json!({
        "joined": false,
        "needs_person": true,
        "ssid": ssid,
        "marked": true,
        "popover_open": ui.get_network_open(),
        "note": format!(
            "{ssid} is not a network this machine has saved, so nothing was sent and nothing was \
             joined. Its row is marked \"{who} asked to join\" for the person to click when they \
             choose; a password, if it needs one, is typed by them at the machine. No password \
             can be passed through this surface."
        ),
    }))
}

/// Add the three network actions to the shell's surface.
pub fn actions(surface: ControlSurface, ui: &App) -> ControlSurface {
    let radio_weak = ui.as_weak();
    let leave_weak = ui.as_weak();
    let join_weak = ui.as_weak();

    surface
        .action(
            // The Wi-Fi tile. `dangerous`, like `wifi_radio` in the minds' tools: switching it off
            // cuts the machine off the network, and every mind with it, while it is off.
            //
            // The answer is NetworkManager's `WirelessEnabled` read back after the call, not the
            // argument: a refused change (polkit, a hardware switch) comes back as the radio's
            // real state and the caller can see it did not take.
            Action::new(
                "set_wifi",
                "Switch the Wi-Fi radio on or off. This is the radio, not a disconnect: to leave \
                 one network and keep Wi-Fi on, use disconnect_network. Answers with the radio's \
                 state as NetworkManager reports it afterwards. Refused on a machine with no \
                 Wi-Fi device",
            )
            .risk("dangerous")
            .arg(Param::flag("enabled").describe("true for on, false for off")),
            move |args| {
                let _ui = radio_weak.upgrade().ok_or_else(|| "the shell is gone".to_string())?;
                let on = args["enabled"].as_bool().ok_or("`enabled` must be true or false")?;
                let latest = yantrik_os::network::latest();
                if !latest.as_ref().map_or(false, |s| s.wifi_present) {
                    return Err("this machine has no Wi-Fi device".to_string());
                }
                // The grade the gate asked about, held against the machine as it is now.
                let published = published_grade("set_wifi").ok_or("`set_wifi` has no published grade")?;
                grade_still_holds("set_wifi", published, latest.as_ref(), yantrik_os::network::is_stale())?;
                // The D-Bus call runs on the RPC side, not on the thread that draws.
                let work = move || {
                    // And once more, from a reading taken here, right before the act.
                    grade_still_holds("set_wifi", published, yantrik_os::network::read_fresh().as_ref(), false)?;
                    let now = yantrik_os::network::set_wifi_enabled(on)?;
                    Ok(serde_json::json!({
                        "wifi_radio": now,
                        "took_effect": now == on,
                    }))
                };
                answer_later(work).map(|()| serde_json::json!("answered by the work")).map_err(|_| NOT_A_DISPATCH.to_string())
            },
        )
        .action(
            // The connected row's Disconnect. `dangerous`, like `wifi_disconnect` (`grade_for`). It leaves the network
            // and leaves the radio alone, which the Quick Settings tile did the other way round.
            Action::new(
                "disconnect_network",
                "Disconnect from the Wi-Fi network the machine is on. A wired link is never taken \
                 down by this: it cannot be brought back from the popover. It does not \
                 switch the Wi-Fi radio off: use set_wifi for that. The machine will not rejoin \
                 by itself until it is asked to. Answers with the device's state afterwards",
            )
            .risk("dangerous"),
            move |_args| {
                let _ui = leave_weak.upgrade().ok_or_else(|| "the shell is gone".to_string())?;
                let latest = yantrik_os::network::latest();
                let published = published_grade("disconnect_network").ok_or("`disconnect_network` has no published grade")?;
                grade_still_holds("disconnect_network", published, latest.as_ref(), yantrik_os::network::is_stale())?;
                let work = move || {
                    grade_still_holds("disconnect_network", published, yantrik_os::network::read_fresh().as_ref(), false)?;
                    // `false`: a mind may not take down a wired link, which the popover cannot restore.
                    let device_state = yantrik_os::network::disconnect(false)?;
                    let after = yantrik_os::network::latest();
                    Ok(serde_json::json!({
                        "device_state": device_state,
                        "wifi_radio": after.as_ref().filter(|s| s.wifi_present).map(|s| s.radio_on),
                        "note": "NetworkManager has been asked to disconnect; `describe shell` network.state shows when it has finished",
                    }))
                };
                answer_later(work).map(|()| serde_json::json!("answered by the work")).map_err(|_| NOT_A_DISPATCH.to_string())
            },
        )
        .action(
            // A row of the popover. `sensitive`: joining a network moves the machine, and every
            // mind on it, onto that network.
            //
            // It switches only between saved networks, and for the rest it marks the row for the
            // person: see the module comment. There is deliberately no argument that could carry a
            // password.
            Action::new(
                "connect_wifi",
                "Switch to a visible Wi-Fi network this machine has already saved. For any other, \
                 secured or open, it does not connect: it marks that row in the network popover as \
                 asked for, and the person decides by clicking it (and types any password at the \
                 machine). It does not open a field, take the keyboard or bring the shell forward. \
                 Never takes a password",
            )
            .risk("sensitive")
            .arg(Param::text("ssid").describe("The network's name, as `describe shell` lists it under wifi_networks")),
            move |args| {
                let ui = join_weak.upgrade().ok_or_else(|| "the shell is gone".to_string())?;
                let ssid = args["ssid"].as_str().map(str::trim).filter(|s| !s.is_empty()).ok_or("`ssid` is required")?.to_string();
                match plan_connect(yantrik_os::network::latest().as_ref(), &ssid)? {
                    ConnectPlan::AskPerson => mark_for_person(&ui, &ssid),
                    ConnectPlan::Join => {
                        // Saved: no secret in the request, nothing is created, so there is nothing
                        // to protect and nothing to ask. The attempt runs off this thread and its
                        // outcome arrives as a change in the network state.
                        // One join at a time and spaced: a refusal comes back to the caller.
                        crate::wire::network::start_join(
                            ui.as_weak(),
                            ConnectRequest { ssid: ssid.clone(), secret: None, by_person: false },
                        )?;
                        Ok(serde_json::json!({
                            "joined": false,
                            "joining": ssid,
                            "network_state": yantrik_os::network::latest().map(|s| s.state.as_str()),
                            "note": "joining has started and takes a few seconds; read `describe shell` network.state and network.ssid to see whether it worked",
                        }))
                    }
                }
            },
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use yantrik_os::{AccessPoint, Connectivity, NetKind, NetState};

    fn ap(ssid: &str, secured: bool, known: bool, connected: bool, enterprise: bool) -> AccessPoint {
        AccessPoint { ssid: ssid.into(), strength: 60, secured, known, connected, enterprise }
    }

    /// Five networks, one joined, one secured and unknown: the laptop shape of the story.
    fn laptop() -> NetworkSnapshot {
        NetworkSnapshot {
            kind: NetKind::Wifi,
            state: NetState::Connected,
            ssid: Some("Home".into()),
            strength: Some(80),
            ip: Some("192.168.1.20".into()),
            connectivity: Connectivity::Full,
            wifi_present: true,
            radio_on: true,
            access_points: vec![
                ap("Home", true, true, true, false),
                ap("Neighbour", true, true, false, false),
                ap("Cafe", true, false, false, false),
                ap("Guest", false, false, false, false),
                ap("Corp", true, false, false, true),
            ],
            ..NetworkSnapshot::default()
        }
    }

    // ── connect_wifi: the refusal to connect without a person ──

    #[test]
    fn a_saved_network_is_switched_to_without_asking_anyone() {
        assert_eq!(plan_connect(Some(&laptop()), "Neighbour"), Ok(ConnectPlan::Join));
    }

    /// In Auto mode a `sensitive` act runs without a card. An open network nobody saved is the
    /// easiest to impersonate, and the profile a join makes is rejoined by itself, so a caller does
    /// not get to choose it: it is marked for the person like a secured unknown one.
    #[test]
    fn an_open_network_nobody_saved_is_left_to_the_person() {
        assert_eq!(plan_connect(Some(&laptop()), "Guest"), Ok(ConnectPlan::AskPerson));
    }

    /// The whole security point of the action: a secured network with no saved profile is not
    /// joined by a caller, because the only way to join it is a password and none may be passed.
    #[test]
    fn a_secured_unknown_network_is_not_joined_it_asks_the_person() {
        assert_eq!(plan_connect(Some(&laptop()), "Cafe"), Ok(ConnectPlan::AskPerson));
    }

    #[test]
    fn it_refuses_what_it_cannot_do_and_says_why() {
        let wired = NetworkSnapshot { kind: NetKind::Wired, state: NetState::Connected, ..NetworkSnapshot::default() };
        assert!(plan_connect(Some(&wired), "Home").unwrap_err().contains("no Wi-Fi device"));
        let off = NetworkSnapshot { radio_on: false, ..laptop() };
        assert!(plan_connect(Some(&off), "Home").unwrap_err().contains("set_wifi enabled=true"));
        assert!(plan_connect(Some(&laptop()), "Nowhere").unwrap_err().contains("not in range"));
        assert!(plan_connect(Some(&laptop()), "Corp").unwrap_err().contains("Network settings"));
        assert!(plan_connect(None, "Home").is_err());
    }

    // ── describe ──

    #[test]
    fn the_network_list_exists_only_where_there_is_a_wifi_device() {
        let rows = wifi_networks_for_describe(Some(&laptop()));
        assert_eq!(rows.as_array().unwrap().len(), 5);
        assert_eq!(rows[2]["ssid"], "Cafe");
        assert_eq!(rows[2]["secured"], true);
        assert_eq!(rows[2]["known"], false);
        let wired = NetworkSnapshot { kind: NetKind::Wired, state: NetState::Connected, ..NetworkSnapshot::default() };
        assert!(wifi_networks_for_describe(Some(&wired)).is_null(), "no Wi-Fi device, no Wi-Fi list");
        assert!(wifi_networks_for_describe(None).is_null());
    }

    #[test]
    fn describe_carries_names_and_signal_and_nothing_a_person_typed() {
        // The rows are built from the reading alone; no NetworkState property is read for them.
        let src = code();
        let rows = &src[src.find("pub fn wifi_networks_for_describe").unwrap()..src.find("/// Mark one row as asked for").unwrap()];
        assert!(!rows.contains("NetworkState"), "the list comes from the reading, not from the window's state:\n{rows}");
        assert!(!rows.contains("asking"), "the open password row is not part of the description");
        // And the object published under `network`, bar the field that says which row is open
        // (`popover_open` is a bool).
        let object = &src[src.find("pub fn network_for_describe").unwrap()..src.find("pub fn wifi_networks_for_describe").unwrap()];
        assert!(!object.contains("asking"), "describe must not say which row the password field is open at:\n{object}");
    }

    // ── the three actions ──

    /// The part of this file above its tests, which is what the shell runs.
    fn code() -> String {
        include_str!("control_network.rs").split("#[cfg(test)]").next().unwrap().to_string()
    }

    /// One action's declaration and handler, to the next one.
    fn declaration(name: &str) -> String {
        let src = code();
        // The `Action::new(` whose first argument is this name; the name also appears elsewhere.
        let at = src
            .match_indices("Action::new(")
            .map(|(i, _)| i)
            .find(|&i| src[i + "Action::new(".len()..].trim_start().starts_with(&format!("\"{name}\",")))
            .unwrap_or_else(|| panic!("`{name}` is no longer published"));
        let rest = &src[at..];
        let end = rest[1..].find("Action::new(").map(|i| i + 1).unwrap_or(rest.len());
        rest[..end].to_string()
    }

    /// Each answer is read back from the system, not the request echoed.
    #[test]
    fn the_answers_are_read_back_from_networkmanager_not_echoed() {
        let radio = declaration("set_wifi");
        assert!(radio.contains("let now = yantrik_os::network::set_wifi_enabled(on)?"), "the radio's answer is what NetworkManager says:\n{radio}");
        assert!(radio.contains("\"wifi_radio\": now"), "and it is what the answer carries:\n{radio}");
        let leave = declaration("disconnect_network");
        assert!(leave.contains("let device_state = yantrik_os::network::disconnect(false)?"), "disconnect answers with the device's state:\n{leave}");
        assert!(leave.contains("\"device_state\": device_state"));
    }

    /// Disconnect and the radio are different acts. The Quick Settings tile ran the radio under a
    /// caption that said disconnect; here neither action may do the other's work.
    #[test]
    fn disconnect_does_not_touch_the_radio_and_the_radio_does_not_disconnect() {
        let leave = declaration("disconnect_network");
        assert!(!leave.contains("set_wifi_enabled"), "disconnect_network must not switch the radio:\n{leave}");
        let radio = declaration("set_wifi");
        assert!(!radio.contains("network::disconnect"), "set_wifi must not disconnect:\n{radio}");
    }

    /// The ask-the-person path marks a row and does nothing else: it must not open a password
    /// field, replace the one the person has open, focus anything, open the popover or raise the
    /// shell, because any of those puts a mind's chosen access point under the person's next
    /// keystrokes. And it must not reach a join at all.
    #[test]
    fn marking_a_row_never_opens_replaces_or_focuses_a_password_field() {
        let src = code();
        let ask = &src[src.find("fn mark_for_person").unwrap()..src.find("/// Add the three network actions").unwrap()];
        for forbidden in [
            "set_asking_ssid", "asking", "raise_shell", "set_network_open", "set_quick_settings_open",
            "start_join", "connect(", "ConnectRequest", "WifiSecret", "AddAndActivate", "focus",
        ] {
            assert!(!ask.contains(forbidden), "the mark path must not reach `{forbidden}`:\n{ask}");
        }
        assert!(ask.contains("set_requested_ssid") && ask.contains("set_requested_by"), "it marks the row and says who asked");
        assert!(ask.contains("\"needs_person\": true"), "and says a person has to decide");
        // The answer reports the popover as it found it, and does not claim to have opened it.
        assert!(ask.contains("\"popover_open\": ui.get_network_open()"));
        // The handler routes both unknown cases there and only there.
        let join = declaration("connect_wifi");
        assert!(join.contains("ConnectPlan::AskPerson => mark_for_person(&ui, &ssid)"));
        let join_arm = &join[join.find("ConnectPlan::Join =>").unwrap()..];
        assert!(join_arm.contains("secret: None, by_person: false"), "the only join a caller can start carries no secret and is not a person's:\n{join_arm}");
        // And no other file sets the open row from a control path.
        for file in ["control_overlays.rs", "control.rs"] {
            let other = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(file)).unwrap();
            assert!(!other.contains("set_asking_ssid"), "{file} must not open a password row");
        }
    }

    // ── grades ──

    fn on_wifi_only() -> NetworkSnapshot {
        NetworkSnapshot { links: 1, ..laptop() }
    }

    /// H1: the repo already grades these verbs `dangerous` (the minds' `wifi_disconnect` and
    /// `wifi_radio`, the Network Manager app's own actions). The shell published them `standard`
    /// whenever two links were up, so a mind refused on the tool could call the shell's action
    /// cheaper. No reading may lower them: the grade is `dangerous` for every shape of machine.
    #[test]
    fn disconnect_and_radio_off_are_dangerous_for_every_reading() {
        let wired = NetworkSnapshot { kind: NetKind::Wired, state: NetState::Connected, links: 1, ..NetworkSnapshot::default() };
        let readings = [
            None,
            Some(on_wifi_only()),
            Some(NetworkSnapshot { links: 2, ..laptop() }), // wired + Wi-Fi: "another link remains"
            Some(NetworkSnapshot { links: 3, ..laptop() }),
            Some(wired),
            Some(NetworkSnapshot::default()),
        ];
        for reading in &readings {
            for action in ["disconnect_network", "set_wifi"] {
                assert_eq!(grade_for(action, reading.as_ref()), "dangerous", "{action} on {reading:?}");
            }
        }
        // Nothing else is graded here.
        assert_eq!(grade_for("connect_wifi", Some(&laptop())), "standard");
    }

    /// H1, against the other surfaces: the shell's grade is not below the one the minds' tools and
    /// the app declare for the same verb.
    #[test]
    fn the_shells_grade_matches_the_tools_for_the_same_verbs() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let tools = std::fs::read_to_string(root.join("yantrik-companion-tools/src/wifi.rs")).unwrap();
        for tool in ["wifi_disconnect", "wifi_radio"] {
            let at = tools.find(&format!("\"{tool}\"")).unwrap_or_else(|| panic!("`{tool}` is no longer a tool"));
            assert!(tools[at..].contains("PermissionLevel::Dangerous"), "{tool} is graded Dangerous elsewhere");
        }
        assert!(declaration("set_wifi").contains(".risk(\"dangerous\")"));
        assert!(declaration("disconnect_network").contains(".risk(\"dangerous\")"));
        assert!(declaration("connect_wifi").contains(".risk(\"sensitive\")"));
    }

    /// H2: the grade the gate read is re-checked against the machine when the call runs, and a call
    /// whose grade went up since is refused, not run at the old one.
    #[test]
    fn a_call_graded_at_one_level_is_refused_if_the_grade_has_risen() {
        let both = NetworkSnapshot { links: 2, ..laptop() };
        // Same grade: goes ahead.
        assert!(grade_still_holds("disconnect_network", "dangerous", Some(&both), false).is_ok());
        // The stale case of the review: it was read at `standard` while two links were up, and the
        // cable has gone since. It must not run at `standard`.
        let refused = grade_still_holds("disconnect_network", "standard", Some(&on_wifi_only()), false).unwrap_err();
        assert!(refused.contains("changed") && refused.contains("dangerous"), "{refused}");
        assert!(grade_still_holds("set_wifi", "sensitive", None, false).is_err(), "no reading is the worse case");
        // A reading known to be stale decides nothing.
        assert!(grade_still_holds("set_wifi", "dangerous", Some(&both), true).unwrap_err().contains("could not be read"));
    }

    /// H2: both handlers hold the grade at the handler and again inside the work, before the call.
    #[test]
    fn both_handlers_recheck_the_grade_before_acting() {
        for (name, call) in [("set_wifi", "network::set_wifi_enabled("), ("disconnect_network", "network::disconnect(")] {
            let d = declaration(name);
            let published = d.find("published_grade(").unwrap_or_else(|| panic!("{name} never reads the grade the gate asked about"));
            let first = d.find("grade_still_holds(").unwrap();
            let work = d.find("let work = move ||").unwrap();
            let last = d.rfind("grade_still_holds(").unwrap();
            let act = d.find(call).unwrap();
            assert!(published < first && first < work, "{name} checks in the handler, before it hands off");
            assert!(work < last && last < act, "{name} checks again inside the work, right before `{call}`");
            assert!(d.contains("read_fresh()"), "{name}'s second check reads the machine, not the snapshot");
        }
    }

    /// The grades are declared in the safe direction and kept in step with each reading and with
    /// every describe.
    #[test]
    fn the_grades_are_declared_and_kept_in_step() {
        assert!(declaration("connect_wifi").contains(".risk(\"sensitive\")"));
        assert!(code().contains("sync_grades(snapshot);"), "describe re-publishes the grades");
        let wire = include_str!("wire/network.rs").split("#[cfg(test)]").next().unwrap().to_string();
        assert!(wire.contains("crate::control_network::sync_grades("), "every new reading re-publishes them");
    }

    /// L4: outside a dispatch the work is refused, never run on the caller's thread.
    #[test]
    fn work_that_has_nowhere_to_go_is_refused_not_run_inline() {
        let src = code();
        assert!(!src.contains(".or_else(|work| work())"), "D-Bus work must not run inline on the UI thread");
        for name in ["set_wifi", "disconnect_network"] {
            assert!(declaration(name).contains("map_err(|_| NOT_A_DISPATCH.to_string())"), "{name} refuses instead");
        }
    }

    /// M3: a mind's `disconnect_network` never takes down a wired link; the popover's own does.
    #[test]
    fn the_control_surface_never_disconnects_a_wired_link() {
        assert!(declaration("disconnect_network").contains("network::disconnect(false)"));
        let wire = include_str!("wire/network.rs").split("#[cfg(test)]").next().unwrap().to_string();
        assert!(wire.contains("network::disconnect(true)"), "the person's Disconnect keeps working on any link");
    }

    /// M2: a mind's join goes through the one-at-a-time slot and a refusal reaches the caller.
    #[test]
    fn a_minds_join_takes_the_slot_and_hears_a_refusal() {
        let join = declaration("connect_wifi");
        assert!(join.contains("by_person: false },\n                        )?;"), "start_join's refusal is returned to the caller");
        let wire = include_str!("wire/network.rs").split("#[cfg(test)]").next().unwrap().to_string();
        let start = &wire[wire.find("pub(crate) fn start_join").unwrap()..];
        assert!(start.find("begin_join(").unwrap() < start.find("thread::spawn").unwrap(), "the slot is taken before a thread exists");
    }

    /// L3: the mark names whoever made the call, not whichever mind the person selected.
    #[test]
    fn the_mark_names_the_caller_not_the_selected_mind() {
        let src = code();
        let ask = &src[src.find("fn mark_for_person").unwrap()..src.find("/// Add the three network actions").unwrap()];
        assert!(ask.contains("requester_now()"));
        assert!(!ask.contains("get_active_harness_name"), "the selected mind is not who called");
    }

    /// M4: `describe` says when the picture could not be refreshed.
    #[test]
    fn describe_says_when_the_picture_is_stale() {
        let src = code();
        let object = &src[src.find("pub fn network_for_describe").unwrap()..src.find("pub fn wifi_networks_for_describe").unwrap()];
        assert!(object.contains("\"stale\": yantrik_os::network::is_stale()"));
    }

    /// D-Bus has no default timeout, and these handlers run on the thread that draws: the calls
    /// are made inside work handed to `answer_later`, never directly in the handler.
    #[test]
    fn no_handler_calls_dbus_on_the_ui_thread() {
        for name in ["set_wifi", "disconnect_network"] {
            let d = declaration(name);
            let work = d.find("let work = move ||").unwrap_or_else(|| panic!("{name} has no work closure"));
            let later = d.find("answer_later(work)").unwrap_or_else(|| panic!("{name} does not use answer_later"));
            for call in ["network::set_wifi_enabled(", "network::disconnect(", "network::read_fresh("] {
                if let Some(at) = d.find(call) {
                    assert!(at > work && at < later, "{name} calls {call} outside its work closure");
                }
            }
        }
        // connect_wifi and the helpers it uses never call them at all.
        let src = code();
        let rest = &src[src.find("fn mark_for_person").unwrap()..];
        let join = declaration("connect_wifi");
        for call in ["network::set_wifi_enabled(", "network::disconnect(", "network::request_scan(", "network::connect(", "network::read_fresh("] {
            assert!(!join.contains(call) && !rest[..rest.find("/// Add the three").unwrap()].contains(call), "`{call}` on the UI thread");
        }
    }

    /// Nothing on this surface can carry a password: not in a name, not in an argument.
    #[test]
    fn no_action_or_argument_here_is_named_like_a_secret() {
        let src = code();
        for (index, _) in src.match_indices("Action::new(") {
            let rest = &src[index..];
            let name = rest.split('"').nth(1).unwrap_or("");
            let lower = name.to_ascii_lowercase();
            for word in ["passphrase", "password", "passwd", "pin", "secret", "credential", "unlock", "psk", "key"] {
                assert!(!lower.contains(word), "action `{name}` reads as a way in for a secret");
            }
        }
        for (index, _) in src.match_indices("Param::") {
            let rest = &src[index..];
            let name = rest.split('"').nth(1).unwrap_or("");
            let lower = name.to_ascii_lowercase();
            for word in ["passphrase", "password", "passwd", "pin", "secret", "credential", "psk", "key"] {
                assert!(!lower.contains(word), "argument `{name}` reads as a way in for a secret");
            }
        }
        // The three arguments that exist.
        assert!(src.contains("Param::flag(\"enabled\")") && src.contains("Param::text(\"ssid\")"));
    }

    /// And the surface is on the shell: the describe keys and the actions are registered.
    #[test]
    fn the_shell_publishes_the_network_and_its_three_actions() {
        let control = include_str!("control.rs");
        let control: String = control.split("#[cfg(test)]").next().unwrap().split_whitespace().collect();
        assert!(control.contains(".with(\"network\",crate::control_network::network_for_describe("), "describe shell publishes `network`");
        assert!(control.contains(".with(\"wifi_networks\",crate::control_network::wifi_networks_for_describe("), "and `wifi_networks`");
        assert!(control.contains("crate::control_network::actions(surface,ui)"), "and the three actions are on the surface");
    }
}
