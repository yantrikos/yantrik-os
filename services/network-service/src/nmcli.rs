//! `nmcli`, and what its answers mean.
//!
//! Split out of `main.rs` so the part that decides what a command's output *means* can be tested
//! on a machine with no NetworkManager on it, and on a machine with no Wi-Fi adapter in it — see
//! `tests/network-core`. Nothing below the [`run`] function starts a process; the parsers and the
//! classifier are pure functions over captured text.
//!
//! # Why nmcli
//!
//! `deploy/yantrik-os/build-debian-iso.sh` installs `network-manager` and enables the
//! `NetworkManager` unit; `deploy/yantrik-os/cloud-init/user-data.yaml` installs the same package
//! with a note saying it was added because nmcli is called from nine places in this codebase and
//! was absent from every machine. So NetworkManager is what this OS ships and nmcli is its
//! command line. `wpasupplicant` is installed too, but as NetworkManager's WPA backend rather
//! than as something to drive directly; there is no `iwd` and no `iwctl` anywhere in the build.
//!
//! # The terse format
//!
//! Everything here reads `nmcli -t`, which prints one record per line with `:` between fields.
//! A `:` inside a value is escaped `\:` and a backslash is escaped `\\`, and an SSID may contain
//! either — `Cafe: Free` and `C:\\Wifi` are both legal network names. Splitting on a bare `:`
//! silently truncates those, which is how an SSID becomes unjoinable without anything looking
//! wrong. [`split_terse`] is the one place that is handled.

use std::process::Command;

use yantrik_ipc_contracts::network::{KnownNetwork, ScannedNetwork};

// ══════════════════════════════════════════════════════════════════════
// Running it
// ══════════════════════════════════════════════════════════════════════

/// The result of running nmcli once, kept as data rather than collapsed at the call site.
///
/// "nmcli is not on this machine", "nmcli is here and NetworkManager refused" and "nmcli is here
/// and did it" are three different things to tell a person, and the code this replaces told them
/// all the same thing, which was nothing: three of the five Wi-Fi calls went into a `let _ =`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exit {
    /// There is no `nmcli` on this machine.
    Missing,
    /// It is here and would not start — a mode bit, a broken PATH entry.
    Unstartable(String),
    /// It ran to completion. `code` is `None` when a signal ended it.
    Ran {
        code: Option<i32>,
        stdout: String,
        stderr: String,
    },
}

/// How long nmcli may wait for an operation that waits.
///
/// The app's control surface gives an action three seconds on the UI thread before telling the
/// caller "the app did not answer", so anything that can be reached from a synchronous action has
/// to finish inside that. `nmcli -w 2` makes the timeout nmcli's, which means it comes back as a
/// readable error — "Error: Timeout expired (2 seconds)" — rather than as a caller being told the
/// window is stuck. Scanning and connecting cannot be done in two seconds and are not tried:
/// those two actions declare `defers` and run off the UI thread with the longer bounds below.
pub const QUICK_WAIT_SECS: u32 = 2;

/// The bound on a rescan. An adapter sweeping the 2.4 and 5 GHz bands takes several seconds.
pub const SCAN_WAIT_SECS: u32 = 10;

/// The bound on an association: DHCP and a WPA handshake on a slow AP.
pub const CONNECT_WAIT_SECS: u32 = 25;

/// The bound on re-activating a connection to apply changed resolvers.
///
/// `nmcli connection up` on a profile that is already up takes the link down and brings it back,
/// which means the address is leased again. Shorter than a Wi-Fi association because there is no
/// handshake in it, longer than [`QUICK_WAIT_SECS`] because there is a DHCP round trip.
pub const APPLY_WAIT_SECS: u32 = 20;

/// Run nmcli once and keep everything it said.
///
/// `wait_secs` becomes nmcli's own `-w`, so a command that hangs is ended by nmcli with a message
/// rather than by us with a dropped process.
pub fn run(wait_secs: u32, argv: &[&str]) -> Exit {
    let wait = wait_secs.to_string();
    let mut args: Vec<&str> = vec!["-w", wait.as_str()];
    args.extend_from_slice(argv);
    match Command::new("nmcli").args(&args).output() {
        Ok(out) => Exit::Ran {
            code: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Exit::Missing,
        Err(e) => Exit::Unstartable(e.to_string()),
    }
}

/// Run nmcli with the password on stdin rather than on the command line.
///
/// `nmcli device wifi connect <ssid> password <pw>` puts the passphrase in `argv`, where every
/// other user on the machine reads it out of `/proc/<pid>/cmdline` for as long as the process
/// lives — and a connect runs for up to twenty-five seconds. `--ask` makes nmcli prompt for the
/// secret on its own terminal instead, and with stdin a pipe it reads the line from there. The
/// passphrase is then only ever in this process's memory, in the pipe, and in NetworkManager's,
/// which is where it has to end up anyway.
///
/// What this does not fix is stated in `design/network-2026-09-20.md`: NetworkManager writes the
/// PSK into `/etc/NetworkManager/system-connections/<name>.nmconnection`, root-readable, because
/// that is what "save this network" means.
pub fn run_with_secret(wait_secs: u32, argv: &[&str], secret: &str) -> Exit {
    use std::io::Write as _;

    let wait = wait_secs.to_string();
    let mut args: Vec<&str> = vec!["-w", wait.as_str(), "--ask"];
    args.extend_from_slice(argv);

    let spawned = Command::new("nmcli")
        .args(&args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();

    let mut child = match spawned {
        Ok(child) => child,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Exit::Missing,
        Err(e) => return Exit::Unstartable(e.to_string()),
    };

    if let Some(stdin) = child.stdin.as_mut() {
        // A write that fails is not fatal here: nmcli may already have decided it does not need a
        // secret (a saved network, an open network), closed the pipe and gone on. The exit status
        // below is what says whether it worked.
        let _ = stdin.write_all(secret.as_bytes());
        let _ = stdin.write_all(b"\n");
        let _ = stdin.flush();
    }
    // Dropped so nmcli sees EOF if it asks for a second secret rather than waiting for one.
    drop(child.stdin.take());

    match child.wait_with_output() {
        Ok(out) => Exit::Ran {
            code: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        },
        Err(e) => Exit::Unstartable(e.to_string()),
    }
}

/// The two ways of running nmcli above, as something a test can stand in for.
///
/// The functions that change a profile (see `autoconnect.rs`) take one of these rather than call
/// [`run`] themselves, so a test can record every argv they would have run and answer each one,
/// on a machine with no NetworkManager — which is the only way to prove that a password never
/// reaches a command line: look at every command line.
pub trait Runner {
    fn run(&self, wait_secs: u32, argv: &[&str]) -> Exit;
    fn run_with_secret(&self, wait_secs: u32, argv: &[&str], secret: &str) -> Exit;
}

/// The real nmcli.
pub struct System;

impl Runner for System {
    fn run(&self, wait_secs: u32, argv: &[&str]) -> Exit {
        run(wait_secs, argv)
    }

    fn run_with_secret(&self, wait_secs: u32, argv: &[&str], secret: &str) -> Exit {
        run_with_secret(wait_secs, argv, secret)
    }
}

// ══════════════════════════════════════════════════════════════════════
// What went wrong
// ══════════════════════════════════════════════════════════════════════

/// The distinguishable ways asking this machine about Wi-Fi can fail.
///
/// Every one of these used to arrive as the same nothing. They are separate because the right
/// thing to do about each is different: a missing adapter is not something a person can press a
/// button about, a wrong password is, and a polkit refusal is a different conversation again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trouble {
    /// `nmcli` is not installed.
    NmcliMissing,
    /// `nmcli` is installed but would not start.
    NmcliUnstartable(String),
    /// The daemon is not running, so nothing can be read or changed.
    NetworkManagerDown,
    /// This machine has no Wi-Fi adapter. Not "the radio is off".
    NoWifiAdapter,
    /// polkit or NetworkManager's own permissions turned the change away.
    PermissionDenied(String),
    /// The AP rejected the secret, or no secret was given for a network that needs one.
    WrongPassword(String),
    /// No access point with that name was in range.
    SsidNotFound(String),
    /// nmcli's own `-w` ran out.
    TimedOut(String),
    /// It failed and said something else. Its own first line, never reworded.
    Said(String),
}

impl Trouble {
    /// The one sentence a person on screen and a mind reading `describe` are both owed.
    pub fn message(&self) -> String {
        match self {
            Trouble::NmcliMissing => {
                "nmcli is not installed on this machine, so Wi-Fi cannot be read or changed"
                    .to_string()
            }
            Trouble::NmcliUnstartable(why) => format!("nmcli could not be started: {why}"),
            Trouble::NetworkManagerDown => {
                "NetworkManager is not running, so Wi-Fi cannot be read or changed".to_string()
            }
            Trouble::NoWifiAdapter => "this machine has no Wi-Fi adapter".to_string(),
            Trouble::PermissionDenied(said) => {
                format!("this session is not allowed to change networking: {said}")
            }
            Trouble::WrongPassword(said) => format!("the network refused the password: {said}"),
            Trouble::SsidNotFound(said) => format!("no network with that name was found: {said}"),
            Trouble::TimedOut(said) => format!("nmcli gave up waiting: {said}"),
            Trouble::Said(said) => said.clone(),
        }
    }

    /// A word a caller can branch on without reading English.
    pub fn kind(&self) -> &'static str {
        match self {
            Trouble::NmcliMissing => "nmcli_missing",
            Trouble::NmcliUnstartable(_) => "nmcli_unstartable",
            Trouble::NetworkManagerDown => "networkmanager_down",
            Trouble::NoWifiAdapter => "no_wifi_adapter",
            Trouble::PermissionDenied(_) => "permission_denied",
            Trouble::WrongPassword(_) => "wrong_password",
            Trouble::SsidNotFound(_) => "ssid_not_found",
            Trouble::TimedOut(_) => "timed_out",
            Trouble::Said(_) => "failed",
        }
    }
}

/// Stdout on success; on failure, what actually went wrong, classified.
///
/// nmcli writes a usable sentence for every one of these and we hand that sentence back rather
/// than replace it with our own guess about what it meant.
pub fn outcome(exit: &Exit) -> Result<String, Trouble> {
    match exit {
        Exit::Missing => Err(Trouble::NmcliMissing),
        Exit::Unstartable(why) => Err(Trouble::NmcliUnstartable(why.clone())),
        Exit::Ran {
            code: Some(0),
            stdout,
            ..
        } => Ok(stdout.clone()),
        Exit::Ran {
            code,
            stdout,
            stderr,
        } => {
            let said = first_line(stderr)
                .or_else(|| first_line(stdout))
                .unwrap_or_else(|| match code {
                    Some(c) => format!("nmcli exited with status {c}"),
                    None => "nmcli was killed".to_string(),
                });
            Err(classify(&said))
        }
    }
}

/// Which [`Trouble`] one of nmcli's own error lines is.
///
/// Matched on the stable part of each message rather than the whole sentence: nmcli's wording has
/// changed across releases and the phrases below have not. Anything unrecognised is passed
/// through verbatim as [`Trouble::Said`], because nmcli's sentence is still better than ours.
pub fn classify(said: &str) -> Trouble {
    let low = said.to_lowercase();
    if low.contains("networkmanager is not running")
        || low.contains("could not connect: no such file or directory")
        || low.contains("nm is not running")
    {
        return Trouble::NetworkManagerDown;
    }
    if low.contains("no wi-fi device found")
        || low.contains("no wifi device found")
        || low.contains("wi-fi scan is not supported")
    {
        return Trouble::NoWifiAdapter;
    }
    if low.contains("not authorized")
        || low.contains("access denied")
        || low.contains("permission denied")
        || low.contains("insufficient privileges")
    {
        return Trouble::PermissionDenied(said.to_string());
    }
    if low.contains("secrets were required")
        || low.contains("no secrets provided")
        || low.contains("802-11-wireless-security")
        || low.contains("passwords or encryption keys are required")
    {
        return Trouble::WrongPassword(said.to_string());
    }
    if low.contains("no network with ssid")
        || low.contains("no network with name")
        || low.contains("unknown connection")
    {
        return Trouble::SsidNotFound(said.to_string());
    }
    if low.contains("timeout expired") || low.contains("timed out") {
        return Trouble::TimedOut(said.to_string());
    }
    Trouble::Said(said.to_string())
}

/// The first line that carries anything. nmcli's errors are one line; a usage banner is many, and
/// the first is still the one worth showing.
pub fn first_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_string)
}

// ══════════════════════════════════════════════════════════════════════
// Reading the terse format
// ══════════════════════════════════════════════════════════════════════

/// Split one `nmcli -t` record into its fields, undoing nmcli's escaping.
///
/// nmcli escapes a `:` inside a value as `\:` and a `\` as `\\`. An SSID is free-form and both
/// turn up in real ones — a café calling its network `Cafe: Free`, a Windows share name with a
/// path in it. `line.split(':')` cuts those in half, and a truncated SSID is one this machine
/// cannot join while the list on screen looks perfectly reasonable.
pub fn split_terse(line: &str) -> Vec<String> {
    let mut fields = vec![String::new()];
    let mut chars = line.trim_end_matches(['\r', '\n']).chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                // A known escape gives back the character it stands for.
                Some(escaped @ (':' | '\\')) => fields.last_mut().unwrap().push(escaped),
                // Anything else was not an escape; keep both characters as they were written.
                Some(other) => {
                    let last = fields.last_mut().unwrap();
                    last.push('\\');
                    last.push(other);
                }
                None => fields.last_mut().unwrap().push('\\'),
            },
            ':' => fields.push(String::new()),
            other => fields.last_mut().unwrap().push(other),
        }
    }
    fields
}

fn field(parts: &[String], index: usize) -> String {
    parts.get(index).cloned().unwrap_or_default().trim().to_string()
}

/// One row of `nmcli -t -f DEVICE,TYPE,STATE,CONNECTION device status`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Device {
    pub name: String,
    /// `wifi`, `ethernet`, `loopback`, `bridge`…
    pub kind: String,
    /// `connected`, `disconnected`, `unavailable`, `unmanaged`.
    pub state: String,
    pub connection: String,
}

pub const DEVICE_FIELDS: &str = "DEVICE,TYPE,STATE,CONNECTION";

pub fn parse_devices(text: &str) -> Vec<Device> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let parts = split_terse(line);
            Device {
                name: field(&parts, 0),
                kind: field(&parts, 1),
                state: field(&parts, 2),
                connection: field(&parts, 3),
            }
        })
        .collect()
}

/// The Wi-Fi device this machine has, if it has one.
///
/// `None` is the answer for the wired machine and it is the answer the whole app was getting
/// wrong: it is not the same as a radio that is off, and it must not be reported as one.
/// `unmanaged` counts as present — the hardware is in the machine, NetworkManager has simply been
/// told not to touch it, and saying "no adapter" there would be a second wrong answer.
pub fn wifi_device(devices: &[Device]) -> Option<&Device> {
    devices.iter().find(|d| d.kind.eq_ignore_ascii_case("wifi"))
}

/// The radio switch, from `nmcli -t radio wifi`.
///
/// nmcli answers `enabled` or `disabled` for the software switch whether or not there is any
/// hardware behind it, which is exactly why this is never read on its own: presence comes from
/// the device list and only then is this consulted.
pub fn parse_radio(text: &str) -> Option<bool> {
    match text.trim().to_lowercase().as_str() {
        "enabled" => Some(true),
        "disabled" => Some(false),
        _ => None,
    }
}

pub const SCAN_FIELDS: &str = "IN-USE,SSID,SIGNAL,SECURITY,RATE";

/// The access points from `nmcli -t -f IN-USE,SSID,SIGNAL,SECURITY,RATE device wifi list`.
///
/// `saved` is the SSIDs of this machine's stored connections, so the list can mark which rows
/// will be joined without asking for anything. Passed in rather than read here: this function
/// runs no processes.
pub fn parse_scan(text: &str, saved: &[String]) -> Vec<ScannedNetwork> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let parts = split_terse(line);
            let in_use = field(&parts, 0);
            let ssid = field(&parts, 1);
            // nmcli prints an empty SSID field for a hidden network. Reported as hidden rather
            // than dropped: a row that quietly disappears is how a list comes to say "there is
            // nothing here" when there is.
            let hidden = ssid.is_empty();
            let security = field(&parts, 3);
            ScannedNetwork {
                is_saved: !hidden && saved.iter().any(|s| s == &ssid),
                ssid,
                hidden,
                signal: field(&parts, 2).parse().unwrap_or(0),
                // nmcli prints `--` for an open network; keep its own word, do not invent "Open".
                security,
                rate: field(&parts, 4),
                is_connected: in_use == "*",
            }
        })
        .collect()
}

pub const CONNECTION_FIELDS: &str = "NAME,UUID,TYPE,ACTIVE";

/// The saved Wi-Fi connections from `nmcli -t -f NAME,UUID,TYPE,ACTIVE connection show`.
///
/// Rows of any other type are dropped, because the wired connection NetworkManager makes for
/// eth0 is saved too and "Known networks" listing "Wired connection 1" would be nonsense.
pub fn parse_known(text: &str) -> Vec<KnownNetwork> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|line| {
            let parts = split_terse(line);
            let kind = field(&parts, 2);
            if kind != "802-11-wireless" {
                return None;
            }
            Some(KnownNetwork {
                ssid: field(&parts, 0),
                uuid: field(&parts, 1),
                is_active: field(&parts, 3) == "yes",
            })
        })
        .collect()
}

pub const ACTIVE_FIELDS: &str = "NAME,UUID,TYPE,DEVICE";

/// One row of `nmcli -t -f NAME,UUID,TYPE,DEVICE connection show --active`.
///
/// The UUID is carried because it is the only stable handle on a profile. A connection's *name*
/// is free-form — NetworkManager names a Wi-Fi profile after the SSID, and an SSID may contain a
/// space, a colon or a leading dash — so `nmcli connection modify <name>` on a profile called
/// `-h` or `Cafe: Free` argues with nmcli's own argument parsing. `modify uuid <uuid>` cannot.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActiveConnection {
    pub name: String,
    pub uuid: String,
    /// `802-3-ethernet`, `802-11-wireless`, `loopback`, `wireguard`…
    pub kind: String,
    /// The interface it is up on. Empty for a profile with no device, which nmcli does list.
    pub device: String,
}

pub fn parse_active(text: &str) -> Vec<ActiveConnection> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let parts = split_terse(line);
            ActiveConnection {
                name: field(&parts, 0),
                uuid: field(&parts, 1),
                kind: field(&parts, 2),
                device: field(&parts, 3),
            }
        })
        .collect()
}

/// Which connection profile "set the DNS on this machine" means.
///
/// Resolvers live on a connection profile, and a machine may have several up at once — a wired
/// link, a Wi-Fi link, a VPN, and always loopback. Writing to the wrong one is a change that
/// reports success and resolves nothing, which is the failure mode of the tool this replaces one
/// layer down.
///
/// `with_gateway` is the devices NetworkManager gives an IPv4 gateway, read by the caller with
/// `device show`. That is the default route in NetworkManager's own words, and the profile
/// carrying it is the one whose resolvers the machine is actually using. When nothing has a
/// gateway — a machine with an address and no route — the first non-loopback active profile is
/// taken, because changing the resolvers of the only link there is is still the right guess, and
/// the caller is told which profile it was either way.
///
/// Loopback is never chosen. NetworkManager manages `lo` as a profile of its own on 1.42 and
/// later, it sorts first in nmcli's output, and setting resolvers on it changes nothing while
/// looking exactly like success.
pub fn resolver_connection<'a>(
    active: &'a [ActiveConnection],
    with_gateway: &[String],
) -> Option<&'a ActiveConnection> {
    let usable = || {
        active.iter().filter(|c| {
            !c.device.is_empty()
                && c.device != "lo"
                && !c.kind.eq_ignore_ascii_case("loopback")
                && !c.uuid.is_empty()
        })
    };
    usable()
        .find(|c| with_gateway.iter().any(|d| d == &c.device))
        .or_else(|| usable().next())
}

pub const DEVICE_DNS_FIELDS: &str = "IP4.DNS,IP6.DNS";

/// The resolvers NetworkManager says it applied to a device, from
/// `nmcli -t -f IP4.DNS,IP6.DNS device show <dev>`.
///
/// The keys are indexed — `IP4.DNS[1]`, `IP4.DNS[2]` — and nmcli prints the key with no value at
/// all when there are none, so an empty result is "NetworkManager applied no resolvers here" and
/// not a parse that went wrong. Read in nmcli's order and not sorted: the order is which resolver
/// is asked first, and re-ordering it would misreport which server answers.
///
/// Everything after the first field is joined back with `:` rather than taken as one field. An
/// IPv6 resolver is `2606:4700:4700::1111`, which is colons all the way down: nmcli's terse mode
/// escapes them as `\:` and [`split_terse`] puts the address back together, but a version that
/// does not escape them — or a value read from anywhere else — would otherwise be truncated to
/// `2606`. A truncated resolver is one this machine cannot ask, reported as the one it was told
/// to use. The key itself never contains a colon, so the first split is always the right one.
pub fn parse_device_dns(text: &str) -> Vec<String> {
    let mut servers = Vec::new();
    for line in text.lines() {
        let parts = split_terse(line);
        let Some(key) = parts.first() else { continue };
        if !key.starts_with("IP4.DNS") && !key.starts_with("IP6.DNS") {
            continue;
        }
        let value = parts[1..].join(":").trim().to_string();
        if value.is_empty() || value == "--" {
            continue;
        }
        servers.push(value);
    }
    servers
}

pub const DEVICE_SHOW_FIELDS: &str =
    "GENERAL.CONNECTION,GENERAL.STATE,IP4.ADDRESS,IP4.GATEWAY,GENERAL.HWADDR";

/// What `nmcli -t -f ... device show <dev>` says about one device.
///
/// The output is `KEY:VALUE` one per line rather than one record per line, and the address key is
/// `IP4.ADDRESS[1]` — an index, because a device may hold several. The first is taken.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceDetail {
    pub connection: Option<String>,
    pub address: Option<String>,
    pub prefix: Option<u8>,
    pub gateway: Option<String>,
}

pub fn parse_device_show(text: &str) -> DeviceDetail {
    let mut detail = DeviceDetail::default();
    for line in text.lines() {
        let parts = split_terse(line);
        let key = field(&parts, 0);
        let value = field(&parts, 1);
        if value.is_empty() || value == "--" {
            continue;
        }
        if key.starts_with("IP4.ADDRESS") && detail.address.is_none() {
            // `192.168.1.5/24`
            let (addr, prefix) = match value.split_once('/') {
                Some((a, p)) => (a.to_string(), p.parse::<u8>().ok()),
                None => (value.clone(), None),
            };
            detail.address = Some(addr);
            detail.prefix = prefix;
        } else if key.starts_with("IP4.GATEWAY") && detail.gateway.is_none() {
            detail.gateway = Some(value);
        } else if key == "GENERAL.CONNECTION" && value != "--" {
            detail.connection = Some(value);
        }
    }
    detail
}

/// A CIDR prefix as the dotted mask the window has a field for.
///
/// `None` above 32 rather than a wrapped shift: an out-of-range prefix is a parse that went
/// wrong, and `0.0.0.0` would be a plausible-looking wrong answer.
pub fn prefix_to_mask(prefix: u8) -> Option<String> {
    if prefix > 32 {
        return None;
    }
    let bits: u32 = if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix as u32)
    };
    let o = bits.to_be_bytes();
    Some(format!("{}.{}.{}.{}", o[0], o[1], o[2], o[3]))
}

/// The SSID this machine is joined to, from the device list and the scan.
///
/// `GENERAL.CONNECTION` is the *connection profile* name, which is the SSID for a profile
/// NetworkManager made itself and is not for one somebody renamed. The `IN-USE` marker in the
/// scan is the SSID, so that is preferred and the profile name is the fallback.
pub fn connected_ssid(scanned: &[ScannedNetwork], device: Option<&Device>) -> Option<String> {
    if let Some(row) = scanned.iter().find(|n| n.is_connected) {
        if !row.ssid.is_empty() {
            return Some(row.ssid.clone());
        }
    }
    let device = device?;
    if !device.state.eq_ignore_ascii_case("connected") {
        return None;
    }
    let name = device.connection.trim();
    if name.is_empty() || name == "--" {
        return None;
    }
    Some(name.to_string())
}
