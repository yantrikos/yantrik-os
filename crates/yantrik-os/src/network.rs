//! Network monitor — reads NetworkManager over D-Bus and tells the shell when it changes.
//!
//! One reader, and it listens: NetworkManager's `PropertiesChanged`, `StateChanged` and
//! `AccessPointAdded`/`Removed` signals wake it, it re-reads, and it speaks only when the picture
//! differs from the last one. The old monitor asked every 15 seconds, so a cable pulled or a
//! network joined showed on the bar up to 15 seconds late, and it knew nothing about signal,
//! other networks or whether the internet was reachable.
//!
//! The reading is a [`NetworkSnapshot`] (see `network_model.rs`), handed to whoever called
//! [`subscribe`] and kept for [`latest`]. The bar, Quick Settings, the System screen and
//! `describe shell` all read that one snapshot. The older [`SystemEvent::NetworkChanged`] is still
//! sent when the link or the network name changes, for the features and the memory that consume it.
//!
//! This file also carries out the few things a person asks of the network from the bar, because
//! they have to reach NetworkManager the same way the reading does: [`request_scan`],
//! [`set_wifi_enabled`], [`disconnect`] and [`connect`]. A Wi-Fi password goes from the popover's
//! field into the `AddAndActivateConnection` call and nowhere else. It is never on a command
//! line (`nmcli ... password X` shows in every process's `/proc/<pid>/cmdline`), never in a log
//! line, and the buffers that held it are overwritten afterwards.
//!
//! On systems without NetworkManager the reading comes from `/sys/class/net`, which can say
//! whether a wired or a wireless interface is up and nothing more.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crossbeam_channel::Sender;
use zbus::blocking::Connection;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

use crate::events::SystemEvent;
use crate::network_model::{
    plan, AccessPoint, ConnectRequest, Connectivity, NetKind, NetState, NetworkSnapshot, Plan,
};

const NM: &str = "org.freedesktop.NetworkManager";
const NM_PATH: &str = "/org/freedesktop/NetworkManager";
const PROPS: &str = "org.freedesktop.DBus.Properties";
const DEVICE: &str = "org.freedesktop.NetworkManager.Device";
const WIRELESS: &str = "org.freedesktop.NetworkManager.Device.Wireless";
const AP: &str = "org.freedesktop.NetworkManager.AccessPoint";
const ACTIVE: &str = "org.freedesktop.NetworkManager.Connection.Active";
const SETTINGS_PATH: &str = "/org/freedesktop/NetworkManager/Settings";
const SETTINGS: &str = "org.freedesktop.NetworkManager.Settings";
const SETTINGS_CONN: &str = "org.freedesktop.NetworkManager.Settings.Connection";

/// The longest any one D-Bus call is waited for. zbus has no default timeout, and a wedged
/// NetworkManager would otherwise hold whichever thread asked, which on a handler is the thread
/// that draws the shell.
const CALL_TIMEOUT: Duration = Duration::from_secs(2);

/// The longest a burst of signals may keep postponing a re-read. A scan makes NetworkManager chatter
/// for seconds; without a cap the picture would not be re-read until it stopped.
const SETTLE_CAP: Duration = Duration::from_secs(2);

/// How long a burst of signals is allowed to settle before the picture is re-read. A scan makes
/// NetworkManager announce every access point's strength in turn.
const SETTLE: Duration = Duration::from_millis(400);

/// How long one attempt to join a network is waited for before it is called failed.
const JOIN_TIMEOUT: Duration = Duration::from_secs(45);

type Props = HashMap<String, OwnedValue>;
type Sink = Box<dyn Fn(NetworkSnapshot) + Send>;

static LATEST: Mutex<Option<NetworkSnapshot>> = Mutex::new(None);
static SINK: Mutex<Option<Sink>> = Mutex::new(None);

/// The one connection every action below shares. Each `new_connection()` is a socket and an
/// executor thread, and a mind that loops `connect_wifi` could otherwise pile them up against the
/// bus daemon's per-user connection limit and starve the shell's own D-Bus use.
static BUS: Mutex<Option<Connection>> = Mutex::new(None);

/// Held across the two quick mutating calls (`set_wifi_enabled`, `disconnect`), so two callers
/// cannot interleave a radio change with a disconnect decided from a reading the other has changed.
static MUTATING: Mutex<()> = Mutex::new(());

/// Whether the last attempt to read NetworkManager failed: the picture held by [`latest`] is then
/// older than it looks, and `describe shell` says so instead of presenting it as current.
static STALE: AtomicBool = AtomicBool::new(false);

/// Whether the signal listener is subscribed right now. While it is not, the monitor polls.
static LISTENING: AtomicBool = AtomicBool::new(false);

/// How soon a mind may start another join. A join runs up to [`JOIN_TIMEOUT`], and two mind
/// joins in quick succession fight over the device (`ActivateConnection` flip-flops).
const MIND_JOIN_SPACING: Duration = Duration::from_secs(5);

/// How often the picture is re-read while the signal listener is down.
const POLL_WITHOUT_SIGNALS: Duration = Duration::from_secs(15);

/// Longest wait before the signal listener tries to subscribe again.
const RESUBSCRIBE_CAP: Duration = Duration::from_secs(30);

/// Whether the held picture could not be refreshed the last time it was tried.
pub fn is_stale() -> bool {
    STALE.load(Ordering::Relaxed)
}

/// One attempt to join at a time, and mind-started ones spaced out. Pure so the rule is a test.
#[derive(Debug, Default)]
pub(crate) struct JoinGate {
    busy: bool,
    last_mind: Option<Instant>,
}

impl JoinGate {
    pub(crate) fn enter(&mut self, now: Instant, by_person: bool) -> Result<(), String> {
        if self.busy {
            return Err("a join is already in progress; wait for it to finish".to_string());
        }
        if !by_person {
            if let Some(last) = self.last_mind {
                if now.saturating_duration_since(last) < MIND_JOIN_SPACING {
                    return Err("a network was joined a moment ago; wait a few seconds before asking again".to_string());
                }
            }
            self.last_mind = Some(now);
        }
        self.busy = true;
        Ok(())
    }

    pub(crate) fn leave(&mut self) {
        self.busy = false;
    }
}

static JOIN_GATE: Mutex<JoinGate> = Mutex::new(JoinGate { busy: false, last_mind: None });

/// Proof that this caller holds the single join slot. Taken BEFORE a worker thread is spawned, so a
/// mind that loops `connect_wifi` cannot start threads faster than joins finish; released when the
/// join ends, however it ends.
#[must_use = "the join slot is released when this is dropped"]
pub struct JoinSlot(());

impl Drop for JoinSlot {
    fn drop(&mut self) {
        if let Ok(mut gate) = JOIN_GATE.lock() {
            gate.leave();
        }
    }
}

/// Claim the join slot, or say why not.
pub fn begin_join(by_person: bool) -> Result<JoinSlot, String> {
    let mut gate = JOIN_GATE.lock().map_err(|_| "the network service is busy".to_string())?;
    gate.enter(Instant::now(), by_person)?;
    Ok(JoinSlot(()))
}

/// The last reading, if there has been one.
pub fn latest() -> Option<NetworkSnapshot> {
    LATEST.lock().ok().and_then(|g| g.clone())
}

/// Be told whenever the picture changes. One listener: the shell's. It is called at once with the
/// last reading when there is one, so starting late loses nothing. It runs on the monitor's
/// thread, so it must hand the data to the thread that draws and return.
pub fn subscribe(sink: impl Fn(NetworkSnapshot) + Send + 'static) {
    if let Some(now) = latest() {
        sink(now);
    }
    if let Ok(mut slot) = SINK.lock() {
        *slot = Some(Box::new(sink));
    }
}

/// Record a reading and pass it on, unless it is the one already held.
pub(crate) fn publish(snapshot: NetworkSnapshot) -> bool {
    {
        let Ok(mut held) = LATEST.lock() else { return false };
        if held.as_ref() == Some(&snapshot) {
            return false;
        }
        *held = Some(snapshot.clone());
    }
    if let Ok(slot) = SINK.lock() {
        if let Some(sink) = slot.as_ref() {
            sink(snapshot);
        }
    }
    true
}

/// Main loop for the network monitor thread.
pub fn run_network_monitor(tx: Sender<SystemEvent>) {
    let connection = match new_connection() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "No system D-Bus — network monitor disabled");
            return;
        }
    };

    let has_nm = zbus::blocking::fdo::DBusProxy::new(&connection)
        .ok()
        .and_then(|p| p.list_names().ok())
        .map(|names| names.iter().any(|n| n.as_str() == NM))
        .unwrap_or(false);

    if !has_nm {
        tracing::info!("NetworkManager not available — using /sys/class/net fallback");
        run_fallback_monitor(tx);
        return;
    }

    tracing::info!("Network monitor started (NetworkManager signals)");

    // One thread does nothing but listen and say "something changed". The other re-reads. They
    // are separate so that a read, which takes D-Bus round trips of its own, never makes the
    // listener miss the signals that arrive meanwhile.
    // One slot is enough: a wake already waiting says everything a second one would.
    let (wake_tx, wake_rx) = crossbeam_channel::bounded::<()>(1);
    let listener_conn = connection.clone();
    // The monitor keeps a sender of its own so the channel never reads as "disconnected" while the
    // listener is down: that would turn the poll below into a busy loop.
    let _keepalive = wake_tx.clone();
    let spawned = std::thread::Builder::new()
        .name("yos-network-signals".into())
        .spawn(move || listen_for_signals(&listener_conn, &wake_tx));
    if spawned.is_err() {
        // No listener at all: the monitor still runs, on its poll, and says so.
        tracing::warn!("Could not start the network signal listener; polling instead");
    }

    let mut last_link: Option<(bool, Option<String>)> = None;
    loop {
        match read_snapshot(&connection) {
            Some(snapshot) => {
                STALE.store(false, Ordering::Relaxed);
                announce_link_change(&tx, &snapshot, &mut last_link);
                publish(snapshot);
            }
            None => {
                // The held picture is older than it looks. Say so; never present it as current.
                if !STALE.swap(true, Ordering::Relaxed) {
                    tracing::warn!("NetworkManager did not answer; the network picture is stale");
                }
            }
        }
        // Block until NetworkManager says something, then let the burst settle. With no listener
        // (it is down, resubscribing) or a failed read, ask again on a timer instead of waiting
        // for a signal that cannot come: the picture must not freeze at the last reading.
        if LISTENING.load(Ordering::Relaxed) && !is_stale() {
            let _ = wake_rx.recv();
        } else {
            let _ = wake_rx.recv_timeout(POLL_WITHOUT_SIGNALS);
        }
        let deadline = Instant::now() + SETTLE_CAP;
        while Instant::now() < deadline && wake_rx.recv_timeout(SETTLE).is_ok() {}
    }
}

/// Subscribe to NetworkManager's signals and wake the reader on each; when the subscription ends or
/// cannot be made, say so, wake the reader once (it re-reads, then polls while this is down) and
/// try again with a growing pause. Returns only when the reader is gone.
fn listen_for_signals(conn: &Connection, wake_tx: &crossbeam_channel::Sender<()>) {
    let mut pause = Duration::from_secs(1);
    loop {
        let subscribed = match zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(NM)
            .and_then(|b| b.path_namespace(NM_PATH))
            .map(|b| b.build())
        {
            Ok(rule) => zbus::blocking::MessageIterator::for_match_rule(rule, conn, None)
                .map_err(|e| format!("could not subscribe to NetworkManager signals: {e}")),
            Err(e) => Err(format!("could not build the NetworkManager match rule: {e}")),
        };
        match subscribed {
            Ok(iter) => {
                LISTENING.store(true, Ordering::Relaxed);
                pause = Duration::from_secs(1);
                for message in iter {
                    if message.is_err() {
                        break;
                    }
                    // Full means a wake is already waiting; disconnected means the reader is gone.
                    if let Err(crossbeam_channel::TrySendError::Disconnected(_)) = wake_tx.try_send(()) {
                        return;
                    }
                }
                tracing::warn!("NetworkManager signal stream ended; resubscribing");
            }
            Err(why) => tracing::warn!(%why, "Network signal listener is down; retrying"),
        }
        LISTENING.store(false, Ordering::Relaxed);
        // Wake the reader so it re-reads now and falls back to its poll, not at the next signal.
        if let Err(crossbeam_channel::TrySendError::Disconnected(_)) = wake_tx.try_send(()) {
            return;
        }
        std::thread::sleep(pause);
        pause = (pause * 2).min(RESUBSCRIBE_CAP);
    }
}

/// `NetworkChanged` for the consumers that predate the snapshot, only when the link or the name
/// changes. Signal strength is not part of it, so a wandering needle does not wake memory.
fn announce_link_change(
    tx: &Sender<SystemEvent>,
    snapshot: &NetworkSnapshot,
    last: &mut Option<(bool, Option<String>)>,
) {
    let now = (snapshot.online(), snapshot.ssid.clone());
    if last.as_ref() == Some(&now) {
        return;
    }
    let _ = tx.send(SystemEvent::NetworkChanged {
        connected: now.0,
        ssid: now.1.clone(),
        signal: snapshot.strength,
    });
    *last = Some(now);
}

/// Fallback monitor for systems without NetworkManager. Checks /sys/class/net for carrier state.
fn run_fallback_monitor(tx: Sender<SystemEvent>) {
    let mut last_link: Option<(bool, Option<String>)> = None;
    loop {
        let snapshot = sysfs_snapshot();
        announce_link_change(&tx, &snapshot, &mut last_link);
        publish(snapshot);
        std::thread::sleep(Duration::from_secs(15));
    }
}

/// What `/sys/class/net` can say: whether a wired or wireless interface is up. No name, no signal,
/// no address — those are NetworkManager's, and a reading that invented them would be a lie.
fn sysfs_snapshot() -> NetworkSnapshot {
    let mut snapshot = NetworkSnapshot::default();
    let Ok(entries) = std::fs::read_dir("/sys/class/net") else { return snapshot };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name == "lo" || ["docker", "veth", "br-", "virbr"].iter().any(|p| name.starts_with(p)) {
            continue;
        }
        let wireless = entry.path().join("wireless").exists();
        if wireless {
            snapshot.wifi_present = true;
        }
        let up = std::fs::read_to_string(entry.path().join("operstate"))
            .map(|s| s.trim() == "up")
            .unwrap_or(false);
        if wireless {
            snapshot.radio_on |= up;
        }
        if up && snapshot.state != NetState::Connected {
            snapshot.state = NetState::Connected;
            snapshot.kind = if wireless { NetKind::Wifi } else { NetKind::Wired };
        }
    }
    snapshot
}

/// A fixed wired reading for the mock observer, so a development machine without NetworkManager
/// still draws the indicator.
pub(crate) fn mock_snapshot() -> NetworkSnapshot {
    NetworkSnapshot {
        kind: NetKind::Wired,
        state: NetState::Connected,
        ip: Some("10.0.2.15".into()),
        connectivity: Connectivity::Full,
        ..NetworkSnapshot::default()
    }
}

// ── reading NetworkManager ─────────────────────────────────────────────────────────────────

fn get_all(conn: &Connection, path: &str, iface: &str) -> Option<Props> {
    let msg = conn.call_method(Some(NM), path, Some(PROPS), "GetAll", &(iface,)).ok()?;
    msg.body().deserialize::<Props>().ok()
}

fn p_u32(p: &Props, key: &str) -> Option<u32> {
    p.get(key).and_then(|v| u32::try_from(v).ok())
}

fn p_u8(p: &Props, key: &str) -> Option<u8> {
    p.get(key).and_then(|v| u8::try_from(v).ok())
}

fn p_bool(p: &Props, key: &str) -> Option<bool> {
    p.get(key).and_then(|v| bool::try_from(v).ok())
}

fn p_string(p: &Props, key: &str) -> Option<String> {
    p.get(key).and_then(|v| <&str>::try_from(v).ok()).map(str::to_string)
}

fn p_path(p: &Props, key: &str) -> Option<String> {
    let v = p.get(key)?.try_clone().ok()?;
    OwnedObjectPath::try_from(v).ok().map(|p| p.as_str().to_string())
}

fn p_paths(p: &Props, key: &str) -> Vec<String> {
    p.get(key)
        .and_then(|v| v.try_clone().ok())
        .and_then(|v| Vec::<OwnedObjectPath>::try_from(v).ok())
        .map(|v| v.into_iter().map(|p| p.as_str().to_string()).collect())
        .unwrap_or_default()
}

fn p_bytes(p: &Props, key: &str) -> Option<Vec<u8>> {
    p.get(key).and_then(|v| v.try_clone().ok()).and_then(|v| Vec::<u8>::try_from(v).ok())
}

fn is_real(path: &str) -> bool {
    !path.is_empty() && path != "/"
}

struct ActiveConn {
    kind: String,
    state: u32,
    id: String,
    ip4: Option<String>,
    specific: Option<String>,
    path: String,
    default: bool,
}

fn is_vpn(kind: &str) -> bool {
    matches!(kind, "vpn" | "wireguard")
}

/// The first IPv4 address of a configuration object, without its prefix.
fn first_ipv4(conn: &Connection, config_path: &str) -> Option<String> {
    let props = get_all(conn, config_path, "org.freedesktop.NetworkManager.IP4Config")?;
    let rows = props
        .get("AddressData")
        .and_then(|v| v.try_clone().ok())
        .and_then(|v| Vec::<HashMap<String, OwnedValue>>::try_from(v).ok())?;
    rows.first().and_then(|row| row.get("address")).and_then(|v| <&str>::try_from(v).ok()).map(str::to_string)
}

/// Names of the networks this machine has a saved Wi-Fi profile for.
fn saved_ssids(conn: &Connection) -> Vec<Vec<u8>> {
    let Ok(msg) = conn.call_method(Some(NM), SETTINGS_PATH, Some(SETTINGS), "ListConnections", &()) else {
        return Vec::new();
    };
    let Ok(paths) = msg.body().deserialize::<Vec<OwnedObjectPath>>() else { return Vec::new() };
    let mut out = Vec::new();
    for path in paths {
        let Ok(reply) = conn.call_method(Some(NM), path.as_str(), Some(SETTINGS_CONN), "GetSettings", &()) else {
            continue;
        };
        let Ok(settings) = reply.body().deserialize::<HashMap<String, HashMap<String, OwnedValue>>>() else {
            continue;
        };
        if let Some(wifi) = settings.get("802-11-wireless") {
            if let Some(ssid) = wifi.get("ssid").and_then(|v| v.try_clone().ok()).and_then(|v| Vec::<u8>::try_from(v).ok()) {
                out.push(ssid);
            }
        }
    }
    out
}

/// NetworkManager's whole picture, in one go.
fn read_snapshot(conn: &Connection) -> Option<NetworkSnapshot> {
    let nm = get_all(conn, NM_PATH, NM)?;
    let connectivity = Connectivity::from_nm(p_u32(&nm, "Connectivity").unwrap_or(0));
    let radio_on = p_bool(&nm, "WirelessEnabled").unwrap_or(false);
    let primary = p_path(&nm, "PrimaryConnection").filter(|p| is_real(p));

    let mut actives: Vec<ActiveConn> = Vec::new();
    for path in p_paths(&nm, "ActiveConnections") {
        let Some(a) = get_all(conn, &path, ACTIVE) else { continue };
        let ip4 = p_path(&a, "Ip4Config").filter(|p| is_real(p)).and_then(|p| first_ipv4(conn, &p));
        actives.push(ActiveConn {
            kind: p_string(&a, "Type").unwrap_or_default(),
            state: p_u32(&a, "State").unwrap_or(0),
            id: p_string(&a, "Id").unwrap_or_default(),
            ip4,
            specific: p_path(&a, "SpecificObject").filter(|p| is_real(p)),
            default: p_bool(&a, "Default").unwrap_or(false),
            path,
        });
    }

    // The Wi-Fi device, if the machine has one. Present is a fact about the hardware; the radio
    // may be off.
    let mut wifi_device: Option<String> = None;
    for path in p_paths(&nm, "Devices") {
        if let Some(d) = get_all(conn, &path, DEVICE) {
            if p_u32(&d, "DeviceType") == Some(2) {
                wifi_device = Some(path);
                break;
            }
        }
    }

    let vpn = actives.iter().any(|a| is_vpn(&a.kind) && a.state == 2);
    let carries = |a: &&ActiveConn| matches!(a.kind.as_str(), "802-11-wireless" | "802-3-ethernet");
    // The carrier: the primary connection when it is up, otherwise the default one, otherwise any
    // that is up; failing all of those, one that is still coming up.
    let carrier = actives
        .iter()
        .filter(carries)
        .find(|a| a.state == 2 && primary.as_deref() == Some(a.path.as_str()))
        .or_else(|| actives.iter().filter(carries).find(|a| a.state == 2 && a.default))
        .or_else(|| actives.iter().filter(carries).find(|a| a.state == 2))
        .or_else(|| actives.iter().filter(carries).find(|a| a.state == 1));

    let mut snapshot = NetworkSnapshot {
        connectivity,
        vpn,
        wifi_present: wifi_device.is_some(),
        radio_on: wifi_device.is_some() && radio_on,
        ..NetworkSnapshot::default()
    };

    snapshot.links = actives.iter().filter(carries).filter(|a| a.state == 2).count().min(255) as u8;

    if let Some(c) = carrier {
        snapshot.state = if c.state == 2 { NetState::Connected } else { NetState::Connecting };
        snapshot.kind = if c.kind == "802-11-wireless" { NetKind::Wifi } else { NetKind::Wired };
        snapshot.ip = c.ip4.clone();
        if snapshot.kind == NetKind::Wifi {
            // The profile's name is usually the network's, but only the access point knows it.
            let ap_props = c.specific.as_deref().and_then(|p| get_all(conn, p, AP));
            snapshot.ssid = ap_props
                .as_ref()
                .and_then(|p| p_bytes(p, "Ssid"))
                .map(|b| String::from_utf8_lossy(&b).to_string())
                .filter(|s| !s.is_empty())
                .or_else(|| Some(c.id.clone()));
            snapshot.strength = ap_props.as_ref().and_then(|p| p_u8(p, "Strength"));
        }
    }

    if let (Some(device), true) = (wifi_device.as_deref(), snapshot.radio_on) {
        snapshot.access_points = read_access_points(conn, device, &snapshot);
    }
    Some(snapshot)
}

/// The visible networks, with what is saved and what is joined marked on them.
fn read_access_points(conn: &Connection, device: &str, now: &NetworkSnapshot) -> Vec<AccessPoint> {
    let Some(wireless) = get_all(conn, device, WIRELESS) else { return Vec::new() };
    let saved = saved_ssids(conn);
    let mut found = Vec::new();
    for path in p_paths(&wireless, "AccessPoints") {
        let Some(p) = get_all(conn, &path, AP) else { continue };
        let ssid_bytes = p_bytes(&p, "Ssid").unwrap_or_default();
        let ssid = String::from_utf8_lossy(&ssid_bytes).to_string();
        let (flags, wpa, rsn) = (
            p_u32(&p, "Flags").unwrap_or(0),
            p_u32(&p, "WpaFlags").unwrap_or(0),
            p_u32(&p, "RsnFlags").unwrap_or(0),
        );
        let (_, enterprise) = security_of(flags, wpa, rsn);
        found.push(AccessPoint {
            connected: now.kind == NetKind::Wifi && now.ssid.as_deref() == Some(ssid.as_str()),
            known: saved.iter().any(|s| *s == ssid_bytes),
            secured: flags & 1 != 0 || wpa != 0 || rsn != 0,
            enterprise,
            strength: p_u8(&p, "Strength").unwrap_or(0).min(100),
            ssid,
        });
    }
    NetworkSnapshot::arrange(found)
}

/// `(sae_only, enterprise)` from an access point's flags. Key management bits: 0x100 PSK, 0x200
/// 802.1X, 0x400 SAE.
fn security_of(_flags: u32, wpa: u32, rsn: u32) -> (bool, bool) {
    let enterprise = (wpa | rsn) & 0x200 != 0 && (wpa | rsn) & 0x100 == 0 && (wpa | rsn) & 0x400 == 0;
    let sae_only = rsn & 0x400 != 0 && rsn & 0x100 == 0;
    (sae_only, enterprise)
}

// ── doing what the bar asks ─────────────────────────────────────────────────────────────────

/// A system-bus connection whose calls give up after [`CALL_TIMEOUT`].
fn new_connection() -> zbus::Result<Connection> {
    zbus::blocking::connection::Builder::system()?.method_timeout(CALL_TIMEOUT).build()
}

/// The shared connection, made on first use. A `Connection` is a handle: cloning it opens nothing.
fn system_bus() -> Result<Connection, String> {
    let mut slot = BUS.lock().map_err(|_| "the network service is busy".to_string())?;
    if let Some(conn) = slot.as_ref() {
        return Ok(conn.clone());
    }
    let conn = new_connection().map_err(|_| "the network service (NetworkManager) is not reachable".to_string())?;
    *slot = Some(conn.clone());
    Ok(conn)
}

/// NetworkManager's picture read right now, on the shared connection: what a decision about to be
/// acted on is made from, instead of [`latest`], which can be seconds behind.
pub fn read_fresh() -> Option<NetworkSnapshot> {
    read_snapshot(&system_bus().ok()?)
}

/// Ask the Wi-Fi device to look for networks. Called when the popover opens and not otherwise,
/// because a scan takes the radio off the air for a moment. NetworkManager refuses a scan that
/// follows another too closely; that is not an error worth a message, the list is still fresh.
pub fn request_scan() -> Result<(), String> {
    let conn = system_bus()?;
    let nm = get_all(&conn, NM_PATH, NM).ok_or("NetworkManager did not answer")?;
    let device = wifi_device_path(&conn, &nm).ok_or("this machine has no Wi-Fi device")?;
    let options: HashMap<String, Value<'_>> = HashMap::new();
    conn.call_method(Some(NM), device.as_str(), Some(WIRELESS), "RequestScan", &(options,))
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn wifi_device_path(conn: &Connection, nm: &Props) -> Option<String> {
    p_paths(nm, "Devices")
        .into_iter()
        .find(|path| get_all(conn, path, DEVICE).and_then(|d| p_u32(&d, "DeviceType")) == Some(2))
}

/// Switch the Wi-Fi radio on or off, and answer with what NetworkManager says afterwards.
pub fn set_wifi_enabled(on: bool) -> Result<bool, String> {
    let _one_at_a_time = MUTATING.lock().unwrap_or_else(|e| e.into_inner());
    let conn = system_bus()?;
    let nm = get_all(&conn, NM_PATH, NM).ok_or("NetworkManager did not answer")?;
    if wifi_device_path(&conn, &nm).is_none() {
        return Err("this machine has no Wi-Fi device".to_string());
    }
    conn.call_method(Some(NM), NM_PATH, Some(PROPS), "Set", &(NM, "WirelessEnabled", Value::from(on)))
        .map_err(|e| format!("NetworkManager refused: {e}"))?;
    // Read back: the property, not the request.
    let after = get_all(&conn, NM_PATH, NM).ok_or("NetworkManager did not answer")?;
    Ok(p_bool(&after, "WirelessEnabled").unwrap_or(false))
}

/// One wired or Wi-Fi connection that is up, as `disconnect` sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Link {
    pub(crate) wired: bool,
    pub(crate) primary: bool,
    pub(crate) device: Option<String>,
}

/// Which device `disconnect` takes down. The primary connection's if it qualifies, else the first.
///
/// `allow_wired` is false for a mind's call: `Device.Disconnect` also blocks that device's
/// autoconnect until something asks for it again. The popover can rejoin a Wi-Fi network but has no
/// control that brings a wired link back, so a mind that unplugged the cable in software would
/// leave the machine off the network until a reboot or a command line. A wired link is skipped
/// for it, and when it is the only one, refused with the reason.
pub(crate) fn choose_disconnect_target(links: &[Link], allow_wired: bool) -> Result<String, String> {
    let usable: Vec<&Link> = links.iter().filter(|l| allow_wired || !l.wired).collect();
    if usable.is_empty() {
        return Err(if links.iter().any(|l| l.wired) {
            "the connection is wired, and a wired link cannot be brought back from the popover once it is \
             taken down: leave it to the person at the machine"
                .to_string()
        } else {
            "nothing is connected".to_string()
        });
    }
    let mut target: Option<&Option<String>> = None;
    for link in usable {
        if link.primary || target.is_none() {
            target = Some(&link.device);
        }
    }
    target.cloned().flatten().ok_or_else(|| "nothing is connected".to_string())
}

/// Disconnect the device carrying the connection. The radio stays as it was: this is "leave this
/// network", and the Wi-Fi tile that used to do it ran `nmcli radio wifi off` under a caption that
/// said "disconnect". The device will not rejoin by itself until a person asks it to.
///
/// `allow_wired`: see [`choose_disconnect_target`]. The target is chosen from a reading taken
/// inside this call, not from whatever the caller last saw.
///
/// Answers with the device's state afterwards, as NetworkManager words it.
pub fn disconnect(allow_wired: bool) -> Result<String, String> {
    let _one_at_a_time = MUTATING.lock().unwrap_or_else(|e| e.into_inner());
    let conn = system_bus()?;
    let nm = get_all(&conn, NM_PATH, NM).ok_or("NetworkManager did not answer")?;
    let primary = p_path(&nm, "PrimaryConnection").filter(|p| is_real(p));
    let mut links: Vec<Link> = Vec::new();
    for path in p_paths(&nm, "ActiveConnections") {
        let Some(a) = get_all(&conn, &path, ACTIVE) else { continue };
        let kind = p_string(&a, "Type").unwrap_or_default();
        if !matches!(kind.as_str(), "802-11-wireless" | "802-3-ethernet") {
            continue;
        }
        links.push(Link {
            wired: kind == "802-3-ethernet",
            primary: primary.as_deref() == Some(path.as_str()),
            device: p_paths(&a, "Devices").into_iter().next(),
        });
    }
    let device = choose_disconnect_target(&links, allow_wired)?;
    conn.call_method(Some(NM), device.as_str(), Some(DEVICE), "Disconnect", &())
        .map_err(|e| format!("NetworkManager refused: {e}"))?;
    let after = get_all(&conn, &device, DEVICE).ok_or("NetworkManager did not answer")?;
    Ok(device_state_word(p_u32(&after, "State").unwrap_or(0)).to_string())
}

fn device_state_word(code: u32) -> &'static str {
    match code {
        100 => "connected",
        110 => "disconnecting",
        40..=90 => "connecting",
        120 => "failed",
        _ => "disconnected",
    }
}

/// Whether a join of this kind may go ahead for this caller: a mind's may only use a saved profile.
pub(crate) fn permitted(by_person: bool, decided: &Plan) -> Result<(), String> {
    if by_person || matches!(decided, Plan::UseSaved) {
        Ok(())
    } else {
        Err("that network is not saved on this machine; only a person can choose to join it".to_string())
    }
}

/// What one look at a joining connection's state means for the wait.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum JoinStep {
    Connected,
    KeepWaiting,
    Failed,
}

/// `state` is the active connection's `State` (0 unknown, 1 activating, 2 activated, 3
/// deactivating, 4 deactivated), or `None` when NetworkManager did not answer inside the call
/// timeout. No answer is not a failure: the join may be succeeding, so it keeps waiting until the
/// cap, and only a state that says failed (or the cap) deletes the profile and gives up.
pub(crate) fn join_step(state: Option<u32>, elapsed: Duration) -> JoinStep {
    match state {
        Some(2) => JoinStep::Connected,
        Some(1) | Some(0) | None if elapsed < JOIN_TIMEOUT => JoinStep::KeepWaiting,
        _ => JoinStep::Failed,
    }
}

/// How a join ended, as far as the caller can know when it did not wait.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Joined {
    /// The network is up.
    Connected,
    /// NetworkManager accepted the request and is joining; watch the snapshot.
    Started,
}

/// Join a visible network.
///
/// With `wait`, blocks (so: call it from a worker, never the thread that draws) until the
/// connection is up or has failed. A failed attempt that created a profile deletes it again, or
/// NetworkManager would go on retrying a wrong password in the background and keep it saved.
///
/// The request is taken by value and dropped before this returns, which overwrites the secret.
/// Errors are words for a person and never contain it.
///
/// `_slot` is the single join slot ([`begin_join`]), held until this returns.
pub fn connect(request: ConnectRequest, wait: bool, _slot: JoinSlot) -> Result<Joined, String> {
    let ssid = request.ssid.clone();
    let conn = system_bus()?;
    let snapshot = read_snapshot(&conn).ok_or("NetworkManager did not answer")?;
    let nm = get_all(&conn, NM_PATH, NM).ok_or("NetworkManager did not answer")?;
    let device = wifi_device_path(&conn, &nm).ok_or("this machine has no Wi-Fi device")?;

    // The strongest access point carrying this name, for the object path and its security.
    let wireless = get_all(&conn, &device, WIRELESS).ok_or("NetworkManager did not answer")?;
    let mut best: Option<(u8, String, u32, u32)> = None;
    for path in p_paths(&wireless, "AccessPoints") {
        let Some(p) = get_all(&conn, &path, AP) else { continue };
        if p_bytes(&p, "Ssid").map(|b| String::from_utf8_lossy(&b).to_string()).as_deref() != Some(ssid.as_str()) {
            continue;
        }
        let strength = p_u8(&p, "Strength").unwrap_or(0);
        if best.as_ref().map_or(true, |b| strength > b.0) {
            best = Some((strength, path, p_u32(&p, "WpaFlags").unwrap_or(0), p_u32(&p, "RsnFlags").unwrap_or(0)));
        }
    }
    let (_, ap_path, wpa, rsn) = best.ok_or_else(|| format!("{ssid} is not in range"))?;
    let (sae_only, _) = security_of(0, wpa, rsn);

    let decided = plan(&snapshot, &request, sae_only).map_err(|r| r.say(&ssid))?;
    // The rule that a mind only switches to a network the machine already has saved belongs HERE,
    // on the reading this call just took, and not only in the caller's earlier look at a snapshot
    // that may be seconds old: a profile deleted in between must not turn a switch into a join.
    permitted(request.by_person, &decided)?;

    let (created, active) = match decided {
        Plan::UseSaved => {
            let profile = saved_profile_path(&conn, &ssid).ok_or_else(|| format!("no saved profile for {ssid}"))?;
            let reply = conn
                .call_method(
                    Some(NM),
                    NM_PATH,
                    Some(NM),
                    "ActivateConnection",
                    &(
                        ObjectPath::try_from(profile.as_str()).map_err(|e| e.to_string())?,
                        ObjectPath::try_from(device.as_str()).map_err(|e| e.to_string())?,
                        ObjectPath::try_from(ap_path.as_str()).map_err(|e| e.to_string())?,
                    ),
                )
                .map_err(|e| format!("could not start joining {ssid}: {e}"))?;
            let active: OwnedObjectPath = reply.body().deserialize().map_err(|e| e.to_string())?;
            (None, active)
        }
        Plan::JoinOpen | Plan::JoinSecured { .. } => {
            let settings = build_settings(&request, &decided);
            let reply = conn
                .call_method(
                    Some(NM),
                    NM_PATH,
                    Some(NM),
                    "AddAndActivateConnection",
                    &(
                        &settings,
                        ObjectPath::try_from(device.as_str()).map_err(|e| e.to_string())?,
                        ObjectPath::try_from(ap_path.as_str()).map_err(|e| e.to_string())?,
                    ),
                )
                // The error text is NetworkManager's own and does not echo the request.
                .map_err(|e| format!("could not start joining {ssid}: {e}"))?;
            drop(settings);
            let (profile, active): (OwnedObjectPath, OwnedObjectPath) =
                reply.body().deserialize().map_err(|e| e.to_string())?;
            (Some(profile), active)
        }
    };
    let had_secret = request.secret.is_some();
    // The secret has done its work. Overwrite it now, not when this function happens to return.
    drop(request);

    if !wait {
        return Ok(Joined::Started);
    }

    let started = Instant::now();
    loop {
        std::thread::sleep(Duration::from_millis(250));
        let state = get_all(&conn, active.as_str(), ACTIVE).and_then(|a| p_u32(&a, "State"));
        match join_step(state, started.elapsed()) {
            JoinStep::Connected => return Ok(Joined::Connected),
            JoinStep::KeepWaiting => continue,
            JoinStep::Failed => {
                // Deactivating, deactivated, gone: it failed, or timed out still activating.
                if let Some(profile) = created.as_ref() {
                    let _ = conn.call_method(Some(NM), profile.as_str(), Some(SETTINGS_CONN), "Delete", &());
                }
                let _ = conn.call_method(Some(NM), NM_PATH, Some(NM), "DeactivateConnection", &(active.as_ref(),));
                return Err(if had_secret {
                    format!("Could not join {ssid}. Check the password and try again.")
                } else {
                    format!("Could not join {ssid}.")
                });
            }
        }
    }
}

fn saved_profile_path(conn: &Connection, ssid: &str) -> Option<String> {
    let msg = conn.call_method(Some(NM), SETTINGS_PATH, Some(SETTINGS), "ListConnections", &()).ok()?;
    let paths = msg.body().deserialize::<Vec<OwnedObjectPath>>().ok()?;
    for path in paths {
        let reply = conn.call_method(Some(NM), path.as_str(), Some(SETTINGS_CONN), "GetSettings", &()).ok()?;
        let settings = reply.body().deserialize::<HashMap<String, HashMap<String, OwnedValue>>>().ok()?;
        let named = settings
            .get("802-11-wireless")
            .and_then(|w| w.get("ssid"))
            .and_then(|v| v.try_clone().ok())
            .and_then(|v| Vec::<u8>::try_from(v).ok());
        if named.as_deref() == Some(ssid.as_bytes()) {
            return Some(path.as_str().to_string());
        }
    }
    None
}

/// The settings dictionary for a new profile. The secret is borrowed into it, not copied.
///
/// Separate from `connect` so a test can check what goes in the dictionary without a bus: the
/// key-management word, and that an open network has no security section at all.
pub(crate) fn build_settings<'a>(
    request: &'a ConnectRequest,
    decided: &Plan,
) -> HashMap<&'static str, HashMap<&'static str, Value<'a>>> {
    let mut connection: HashMap<&'static str, Value<'a>> = HashMap::new();
    connection.insert("type", Value::from("802-11-wireless"));
    connection.insert("id", Value::from(request.ssid.as_str()));
    // An open network is the easiest to impersonate. Unless a person pressed Join on it, the
    // profile made for it does not rejoin by itself.
    if matches!(decided, Plan::JoinOpen) && !request.by_person {
        connection.insert("autoconnect", Value::from(false));
    }
    let mut wireless: HashMap<&'static str, Value<'a>> = HashMap::new();
    wireless.insert("ssid", Value::from(request.ssid.as_bytes().to_vec()));
    wireless.insert("mode", Value::from("infrastructure"));

    let mut out = HashMap::new();
    out.insert("connection", connection);
    out.insert("802-11-wireless", wireless);
    if let (Plan::JoinSecured { key_mgmt }, Some(secret)) = (decided, request.secret.as_ref()) {
        let mut security: HashMap<&'static str, Value<'a>> = HashMap::new();
        security.insert("key-mgmt", Value::from(*key_mgmt));
        security.insert("psk", Value::from(secret.expose()));
        out.insert("802-11-wireless-security", security);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network_model::WifiSecret;

    #[test]
    fn an_open_network_gets_no_security_section_and_a_secured_one_gets_its_key_word() {
        let open = ConnectRequest { ssid: "Guest".into(), secret: None, by_person: false };
        let dict = build_settings(&open, &Plan::JoinOpen);
        assert!(!dict.contains_key("802-11-wireless-security"));

        let secured = ConnectRequest { ssid: "Cafe".into(), secret: Some(WifiSecret::new("long-enough-1".into())), by_person: true };
        let dict = build_settings(&secured, &Plan::JoinSecured { key_mgmt: "sae" });
        let security = dict.get("802-11-wireless-security").expect("a secured join carries a security section");
        assert!(security.contains_key("psk"));
        assert_eq!(security.get("key-mgmt").and_then(|v| v.downcast_ref::<&str>().ok()), Some("sae"));
    }

    /// A profile made for an open network by anything but a person's press does not rejoin by
    /// itself; one the person chose may.
    #[test]
    fn an_open_networks_profile_autoconnects_only_when_a_person_chose_it() {
        let flag = |by_person| {
            let r = ConnectRequest { ssid: "Guest".into(), secret: None, by_person };
            build_settings(&r, &Plan::JoinOpen)["connection"].get("autoconnect").and_then(|v| v.downcast_ref::<bool>().ok())
        };
        assert_eq!(flag(false), Some(false), "not chosen by a person: autoconnect off");
        assert_eq!(flag(true), None, "chosen by a person: NetworkManager's default (on)");
    }

    #[test]
    fn every_connection_here_has_a_call_timeout_and_a_bounded_wake() {
        let src = include_str!("network.rs");
        let code = src.split("#[cfg(test)]").next().unwrap();
        assert!(!code.contains("Connection::system()"), "connections come from new_connection(), which sets the timeout");
        assert!(code.contains("method_timeout(CALL_TIMEOUT)"));
        assert!(code.contains(".sender(NM)"), "the signal match names NetworkManager as its sender");
        assert!(code.contains("bounded::<()>(1)") && !code.contains("unbounded"));
        assert!(code.contains("SETTLE_CAP"), "the settle loop has a total cap");
    }

    #[test]
    fn a_saved_network_plan_puts_no_secret_in_any_dictionary() {
        let request = ConnectRequest { ssid: "Home".into(), secret: Some(WifiSecret::new("long-enough-1".into())), by_person: false };
        let dict = build_settings(&request, &Plan::UseSaved);
        assert!(!dict.contains_key("802-11-wireless-security"), "a saved profile carries its own key; none is sent");
    }

    #[test]
    fn flags_tell_wpa3_only_and_enterprise_networks_apart() {
        // WPA2-PSK: RSN with the PSK bit.
        assert_eq!(security_of(1, 0, 0x100), (false, false));
        // WPA3-SAE only.
        assert_eq!(security_of(1, 0, 0x400), (true, false));
        // WPA2/WPA3 transition offers PSK as well, so the PSK profile works.
        assert_eq!(security_of(1, 0, 0x500), (false, false));
        // 802.1X.
        assert_eq!(security_of(1, 0, 0x200), (false, true));
        // Open.
        assert_eq!(security_of(0, 0, 0), (false, false));
    }

    #[test]
    fn the_monitor_listens_for_signals_and_never_polls_networkmanager() {
        let src = include_str!("network.rs");
        let code = src.split("#[cfg(test)]").next().unwrap();
        let nm_loop = &code[code.find("pub fn run_network_monitor").unwrap()..code.find("fn announce_link_change").unwrap()];
        assert!(nm_loop.contains("for_match_rule"), "the NetworkManager path subscribes to signals");
        let monitor = &code[code.find("pub fn run_network_monitor").unwrap()..code.find("fn listen_for_signals").unwrap()];
        assert!(!monitor.contains("thread::sleep"), "the NetworkManager path has no sleep-and-ask loop");
    }

    /// The password never reaches a command line or a log: this file has no `Command`, and no
    /// tracing call names a request or a secret.
    #[test]
    fn no_process_is_spawned_and_nothing_logs_a_request() {
        let src = include_str!("network.rs");
        let code = src.split("#[cfg(test)]").next().unwrap();
        assert!(!code.contains("Command::new"), "NetworkManager is reached over D-Bus, not through nmcli");
        for line in code.lines().filter(|l| l.contains("tracing::")) {
            assert!(!line.contains("request") && !line.contains("secret"), "a log line names the request: {line}");
        }
    }

    // ── security review round 3 ──

    /// M1: a mind's join may only use a saved profile, decided inside `connect()` on its own fresh
    /// reading. The case that got through: a saved open profile deleted between the handler's look
    /// and the worker's, so the switch had become a create-and-join of an open network.
    #[test]
    fn a_minds_join_may_only_use_a_saved_profile_inside_connect_itself() {
        assert!(permitted(false, &Plan::UseSaved).is_ok());
        assert!(permitted(false, &Plan::JoinOpen).is_err(), "an open network nobody saved is a person's to join");
        assert!(permitted(false, &Plan::JoinSecured { key_mgmt: "wpa-psk" }).is_err());
        for plan in [Plan::UseSaved, Plan::JoinOpen, Plan::JoinSecured { key_mgmt: "wpa-psk" }] {
            assert!(permitted(true, &plan).is_ok(), "a person's press may join");
        }
        let code = include_str!("network.rs").split("#[cfg(test)]").next().unwrap();
        let connect = &code[code.find("pub fn connect(").unwrap()..code.find("fn saved_profile_path").unwrap()];
        let check = connect.find("permitted(request.by_person, &decided)?").expect("connect() applies the rule itself");
        assert!(check < connect.find("AddAndActivateConnection").unwrap(), "before any D-Bus write");
    }

    /// M2: one join at a time, a mind's joins spaced, and a person never held back by a mind's.
    #[test]
    fn only_one_join_runs_at_a_time_and_a_mind_cannot_loop_them() {
        let t0 = Instant::now();
        let mut gate = JoinGate::default();
        assert!(gate.enter(t0, false).is_ok());
        assert!(gate.enter(t0, true).unwrap_err().contains("already in progress"), "a second join waits, person or mind");
        assert!(gate.enter(t0, false).is_err());
        gate.leave();
        assert!(gate.enter(t0 + Duration::from_secs(1), false).unwrap_err().contains("a moment ago"), "a mind's next join is spaced");
        assert!(gate.enter(t0 + Duration::from_secs(1), true).is_ok(), "a person is never held back by a mind's spacing");
        gate.leave();
        assert!(gate.enter(t0 + MIND_JOIN_SPACING + Duration::from_secs(1), false).is_ok());
    }

    /// The only test that touches the static gate, so it cannot race another.
    #[test]
    fn the_slot_is_released_when_it_is_dropped() {
        let first = begin_join(true).expect("the slot is free");
        assert!(begin_join(true).is_err());
        drop(first);
        drop(begin_join(true).expect("free again after the join ended"));
    }

    /// M2: no per-call connection, and `connect` cannot be called without the slot.
    #[test]
    fn calls_share_one_connection_and_connect_requires_the_slot() {
        let code = include_str!("network.rs").split("#[cfg(test)]").next().unwrap();
        let makers = code.lines().filter(|l| !l.trim_start().starts_with("//") && l.contains("new_connection()")).count();
        assert_eq!(makers, 3, "the monitor's, the shared bus's, and the definition: no per-call connection");
        assert!(code.contains("pub fn connect(request: ConnectRequest, wait: bool, _slot: JoinSlot)"));
        assert!(code.contains("MUTATING.lock()"), "radio and disconnect do not interleave");
    }

    fn link(wired: bool, primary: bool, device: &str) -> Link {
        Link { wired, primary, device: Some(device.to_string()) }
    }

    /// M3: a mind cannot take down a wired link the popover has no control to bring back.
    #[test]
    fn a_mind_cannot_disconnect_a_wired_link() {
        let only_wired = [link(true, true, "/eth0")];
        assert!(choose_disconnect_target(&only_wired, false).unwrap_err().contains("wired"));
        assert_eq!(choose_disconnect_target(&only_wired, true), Ok("/eth0".to_string()), "the person at the machine may");
        // Wired is primary, Wi-Fi is up too: a mind's disconnect leaves the cable alone.
        let both = [link(true, true, "/eth0"), link(false, false, "/wlan0")];
        assert_eq!(choose_disconnect_target(&both, false), Ok("/wlan0".to_string()));
        assert_eq!(choose_disconnect_target(&both, true), Ok("/eth0".to_string()), "the primary, as before");
        assert!(choose_disconnect_target(&[], false).unwrap_err().contains("nothing is connected"));
    }

    /// M4: the listener restarts, the monitor polls while it is down and says when a read failed.
    #[test]
    fn a_dead_signal_listener_restarts_and_the_monitor_never_freezes_silently() {
        let code = include_str!("network.rs").split("#[cfg(test)]").next().unwrap();
        let listener = &code[code.find("fn listen_for_signals").unwrap()..code.find("/// `NetworkChanged` for the consumers").unwrap()];
        assert!(listener.contains("loop {") && listener.contains("RESUBSCRIBE_CAP"), "it resubscribes with a growing pause");
        assert!(listener.contains("tracing::warn!"), "and says it is down");
        assert!(listener.contains("LISTENING.store(false"), "and tells the reader it is down");
        let monitor = &code[code.find("pub fn run_network_monitor").unwrap()..code.find("fn listen_for_signals").unwrap()];
        assert!(monitor.contains("POLL_WITHOUT_SIGNALS"), "the reader polls while there is no listener");
        assert!(monitor.contains("STALE.swap(true") && monitor.contains("STALE.store(false"), "a failed read marks the picture stale until the next good one");
        assert!(!monitor[monitor.find("let mut last_link").unwrap()..].contains("return"), "a closed channel or a failed read does not end the monitor");
    }

    /// L5: a poll that got no answer is not a failed join.
    #[test]
    fn a_join_poll_with_no_answer_keeps_waiting_until_the_cap() {
        let soon = Duration::from_secs(3);
        assert_eq!(join_step(None, soon), JoinStep::KeepWaiting, "the 2 s call timeout is not a failure");
        assert_eq!(join_step(Some(1), soon), JoinStep::KeepWaiting);
        assert_eq!(join_step(Some(2), soon), JoinStep::Connected);
        assert_eq!(join_step(Some(4), soon), JoinStep::Failed, "deactivated is a failure");
        assert_eq!(join_step(Some(3), soon), JoinStep::Failed);
        assert_eq!(join_step(None, JOIN_TIMEOUT), JoinStep::Failed, "and the cap still ends the wait");
        assert_eq!(join_step(Some(1), JOIN_TIMEOUT), JoinStep::Failed);
    }
}
