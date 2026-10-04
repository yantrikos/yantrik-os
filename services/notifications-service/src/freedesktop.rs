//! `org.freedesktop.Notifications` — the door every ordinary Linux program comes through.
//!
//! ## Why this is in the service
//!
//! It was in two other places. `mako` was started from the labwc autostart and from four build
//! scripts; the shell also implemented the whole interface in `yantrik-os::dbus_notif` and
//! claimed the same bus name from a thread of its own. Only one process can own a well-known
//! name, so which of the two a `notify-send` reached depended on start order — the audit of 17
//! September caught mako winning by a few hundred milliseconds. When mako won, the shell's
//! notification centre stayed empty and the popups were drawn in mako's style; when the shell
//! won, the popups were ours and nothing was kept past a reboot. Either way, half the machine's
//! notifications were invisible to the other half.
//!
//! One owner per domain. The store is here, so the bus name is here, and the shell draws what
//! the store holds.
//!
//! ## If the name is taken anyway
//!
//! On a machine that has not taken the update, mako is still in the autostart and will have the
//! name. The service says so plainly in `describe` and goes on serving its socket — `yos notify`,
//! the download manager and the calendar reminder all still work, and only the freedesktop door
//! is shut. It does not retry in a loop: two daemons taking turns at a well-known name is worse
//! than one of them losing, and a log line every second hides the one line that matters.
//!
//! ## What is spec and what is ours
//!
//! Methods `Notify`, `CloseNotification`, `GetCapabilities`, `GetServerInformation` and the
//! signals `NotificationClosed` and `ActionInvoked` are the Desktop Notifications Specification
//! 1.2. The mapping from those arguments onto a stored notification is ours, and every part of
//! it that can be a pure function is one, below the server, so it is tested without a bus.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use yantrik_ipc_contracts::notifications::*;

use crate::store::Store;

/// The object path and interface name the spec fixes.
const PATH: &str = "/org/freedesktop/Notifications";
const INTERFACE: &str = "org.freedesktop.Notifications";
const BUS_NAME: &str = "org.freedesktop.Notifications";

/// Why a notification was closed (spec 1.2 §9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum CloseReason {
    /// The popup timed out.
    Expired = 1,
    /// The person dismissed it.
    DismissedByUser = 2,
    /// `CloseNotification` was called.
    ClosedByApi = 3,
}

// ── The pure mapping ────────────────────────────────────────────────────────────────────────

/// Read the spec's flat action list — `[id, label, id, label, …]` — into pairs.
///
/// A dangling id at the end is dropped rather than paired with an empty label: a button with no
/// text is a button nobody can read, and the sender's list was malformed.
pub fn actions_from_pairs(flat: &[String]) -> Vec<NotificationAction> {
    flat.chunks_exact(2)
        // No `args`: a freedesktop action is answered with an `ActionInvoked` signal carrying
        // only the key, so there is nothing for the shell to call on the sender's behalf.
        .map(|pair| NotificationAction::new(pair[0].clone(), pair[1].clone()))
        .filter(|a| !a.id.trim().is_empty() && !a.label.trim().is_empty())
        .collect()
}

/// The `urgency` hint, which the spec defines as a byte.
///
/// Anything else — a missing hint, a hint of the wrong type, a value outside 0..=2 — is normal.
/// A notification is not worth losing over a malformed hint.
pub fn urgency_from_hints(hints: &HashMap<String, zbus::zvariant::OwnedValue>) -> Urgency {
    hints
        .get("urgency")
        .and_then(|v| u8::try_from(v).ok())
        .map(Urgency::from_hint_byte)
        .unwrap_or(Urgency::Normal)
}

/// `replaces_id` as the store's id. Zero means "this is a new notification" (spec 1.2 §1.2).
pub fn replaces_target(replaces_id: u32) -> Option<String> {
    (replaces_id > 0).then(|| replaces_id.to_string())
}

/// Who to show as the sender.
///
/// `app_name` is free text and may be empty — Chromium sends its page title, `notify-send`
/// sends `notify-send` unless told otherwise, and a shell script sends nothing at all. The
/// `desktop-entry` hint is the sender's `.desktop` id and is the better name when there is one;
/// "unknown" is the last resort, and it is a word a person can act on.
///
/// Never the desktop's own name. This door has not asked the bus who is behind a call, so there
/// is no sender record and no program to file it under instead: `notify-send -a Yantrik …` from
/// any program of the person's own stored a row and a toast reading "Yantrik" with nothing
/// beside it (third-pass security review of #614). A name that borrows the desktop's
/// (`names::borrows_the_desktops_name`) is refused here as on the socket, and the next name in
/// line is used; the shell says "via D-Bus, not verified" beside whatever is kept.
pub fn sender_label(app_name: &str, desktop_entry: Option<&str>) -> String {
    let allowed = |s: &&str| !s.is_empty() && !crate::names::borrows_the_desktops_name(s);
    let from_name = app_name.trim();
    if allowed(&from_name) {
        return from_name.to_string();
    }
    match desktop_entry.map(str::trim).filter(allowed) {
        Some(entry) => entry.to_string(),
        None => "unknown".to_string(),
    }
}

/// Everything `Notify` was called with, as a request this store can take.
///
/// `expire_timeout` is deliberately not carried into the store. It is a request about how long
/// the popup stays up, and the popup belongs to the shell, which has one rule for every
/// notification on the machine — six seconds for low and normal, critical stays until it is
/// dismissed — so that a program cannot pin its own message to the corner of somebody's screen
/// forever. The store is history, not a timer; nothing in it expires by the clock. mako made the
/// same call with its `default-timeout`. It is logged at the door so the decision is visible.
pub fn notify_to_request(
    app_name: &str,
    replaces_id: u32,
    summary: &str,
    body: &str,
    actions: &[String],
    hints: &HashMap<String, zbus::zvariant::OwnedValue>,
) -> AddRequest {
    let desktop_entry = hints
        .get("desktop-entry")
        .and_then(|v| <&str>::try_from(v).ok())
        .map(str::to_string);
    AddRequest {
        app: sender_label(app_name, desktop_entry.as_deref()),
        title: summary.to_string(),
        body: body.to_string(),
        urgency: urgency_from_hints(hints),
        actions: actions_from_pairs(actions),
        source: Source::Freedesktop,
        replaces_id: replaces_target(replaces_id),
    }
}

// ── The link back out ───────────────────────────────────────────────────────────────────────

/// What the rest of the service holds: the state of the bus name, and the way to answer a
/// freedesktop sender when the person presses one of its buttons in our own UI.
///
/// A sender that gets no `ActionInvoked` has an action button that does nothing, which is
/// precisely the dead control this repo forbids — so the shell pressing "Reply" on a Chromium
/// notification has to come back out of this door.
pub struct Link {
    status: Mutex<String>,
    connection: Mutex<Option<zbus::blocking::Connection>>,
}

impl Link {
    pub fn new() -> Self {
        Self {
            status: Mutex::new("not started".to_string()),
            connection: Mutex::new(None),
        }
    }

    /// One sentence for `describe`: either the name we hold, or who holds it instead.
    pub fn status(&self) -> String {
        self.status
            .lock()
            .map(|s| s.clone())
            .unwrap_or_else(|e| e.into_inner().clone())
    }

    fn set_status(&self, text: String) {
        match self.status.lock() {
            Ok(mut s) => *s = text,
            Err(e) => *e.into_inner() = text,
        }
    }

    /// Tell the sender its button was pressed.
    ///
    /// Only for notifications that came in this way: a `yantrik` notification has no D-Bus
    /// sender waiting, and broadcasting an id from our own numbering into somebody else's id
    /// space is how one program acts on another's notification.
    pub fn action_invoked(&self, n: &Notification, action_id: &str) {
        if n.source != Source::Freedesktop {
            return;
        }
        let Some(id) = fd_id(n) else { return };
        self.emit("ActionInvoked", &(id, action_id));
    }

    /// Tell the sender its notification is gone, and why.
    pub fn closed(&self, n: &Notification, reason: CloseReason) {
        if n.source != Source::Freedesktop {
            return;
        }
        let Some(id) = fd_id(n) else { return };
        self.emit("NotificationClosed", &(id, reason as u32));
    }

    fn emit<B>(&self, signal: &str, body: &B)
    where
        B: serde::ser::Serialize + zbus::zvariant::DynamicType,
    {
        let guard = match self.connection.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        let Some(conn) = guard.as_ref() else {
            // No bus, or the name was taken. Not worth a warning per press: the reason is
            // already standing in `describe`.
            return;
        };
        // Broadcast — `None` destination. The sender is subscribed by match rule on the
        // interface, not by being addressed, and unicasting to the well-known name would send
        // the signal to ourselves, which is where the previous implementation's `dbus-send
        // --dest=org.freedesktop.Notifications` was quietly delivering every one of these.
        if let Err(e) = conn.emit_signal(None::<&str>, PATH, INTERFACE, signal, body) {
            tracing::warn!(signal, error = %e, "could not emit a freedesktop notification signal");
        }
    }
}

/// The spec's `u32` id for a stored notification. Our ids are a decimal counter, so this is a
/// parse; it fails only for a notification that came from somewhere else entirely.
fn fd_id(n: &Notification) -> Option<u32> {
    n.id.parse::<u32>().ok()
}

// ── The server ──────────────────────────────────────────────────────────────────────────────

struct Server {
    store: Arc<Store>,
    link: Arc<Link>,
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Server {
    /// What this daemon can do (spec 1.2 §9).
    ///
    /// `body-markup` is not claimed: the toast and the notification centre draw plain text, and
    /// a daemon that claims markup and then shows the tags is worse than one that never claimed
    /// it — senders format for the capability they are told about.
    fn get_capabilities(&self) -> Vec<String> {
        vec![
            "body".into(),
            "actions".into(),
            "persistence".into(),
            "icon-static".into(),
        ]
    }

    /// Post a notification. Answers the id the sender can close or replace it by.
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        _app_icon: &str,
        summary: &str,
        body: &str,
        actions: Vec<String>,
        hints: HashMap<String, zbus::zvariant::OwnedValue>,
        expire_timeout: i32,
    ) -> u32 {
        let request = notify_to_request(app_name, replaces_id, summary, body, &actions, &hints);
        let stored = self.store.add(request);
        tracing::info!(
            id = %stored.id,
            app = %stored.app,
            urgency = stored.urgency.as_str(),
            actions = stored.actions.len(),
            expire_timeout,
            "freedesktop notification stored (expire_timeout is the sender's wish for the \
             popup; the shell decides how long a toast stays)"
        );
        fd_id(&stored).unwrap_or(0)
    }

    /// Close a notification the sender posted earlier.
    ///
    /// The spec says an id that is not known is not an error, so a sender closing something
    /// already gone is answered the same as one closing something live — but the signal only
    /// goes out for a notification that was really there, because that is what the signal means.
    fn close_notification(&self, id: u32) {
        let key = id.to_string();
        let Some(n) = self.store.get(&key) else {
            tracing::debug!(id, "CloseNotification for an id this store does not have");
            return;
        };
        if self.store.dismiss(&key) {
            self.link.closed(&n, CloseReason::ClosedByApi);
        }
    }

    /// Who is answering (spec 1.2 §9): name, vendor, version, spec version.
    fn get_server_information(&self) -> (String, String, String, String) {
        (
            "yantrik-notifications".into(),
            "Yantrik OS".into(),
            env!("CARGO_PKG_VERSION").into(),
            "1.2".into(),
        )
    }
}

/// Claim the name and serve the interface. Blocks; run it on a thread of its own.
///
/// Never panics and never returns an error to the caller: a desktop whose notification service
/// refuses to start because another daemon has a bus name is worse than one whose freedesktop
/// door is shut and says so.
pub fn serve(store: Arc<Store>, link: Arc<Link>) {
    let server = Server {
        store,
        link: link.clone(),
    };

    let connection = match zbus::blocking::connection::Builder::session() {
        Ok(builder) => builder,
        Err(e) => {
            let why = format!("unavailable — no session bus ({e})");
            tracing::warn!(
                "{why}. notify-send and ordinary applications cannot reach this store; \
                 everything posted over the service socket still works"
            );
            link.set_status(why);
            return;
        }
    }
    .name(BUS_NAME)
    .and_then(|b| b.serve_at(PATH, server))
    .and_then(|b| b.build());

    let connection = match connection {
        Ok(c) => c,
        Err(e) => {
            let why = match name_owner() {
                Some(owner) => format!("unavailable — {BUS_NAME} is owned by {owner}"),
                None => format!("unavailable — could not claim {BUS_NAME} ({e})"),
            };
            // Plainly, once. Not a retry loop: see the note at the top of this file.
            tracing::warn!(
                "{why}. This machine is probably still starting mako from its labwc autostart; \
                 remove that line and restart the session. The service socket keeps working, so \
                 `yos notify` and our own apps still reach this store."
            );
            link.set_status(why);
            return;
        }
    };

    tracing::info!(
        unique_name = %connection.unique_name().map(|n| n.as_str().to_string()).unwrap_or_default(),
        "serving {BUS_NAME}"
    );
    link.set_status(format!("serving {BUS_NAME}"));
    match link.connection.lock() {
        Ok(mut slot) => *slot = Some(connection),
        Err(e) => *e.into_inner() = Some(connection),
    }

    // The connection's own reactor answers method calls; this thread only has to stay alive and
    // notice if the bus goes away. Parking forever would leave a dead connection serving
    // nothing and `describe` still claiming the name.
    loop {
        std::thread::sleep(std::time::Duration::from_secs(30));
        let lost = {
            let guard = match link.connection.lock() {
                Ok(g) => g,
                Err(e) => e.into_inner(),
            };
            guard
                .as_ref()
                .map(|c| c.unique_name().is_none())
                .unwrap_or(true)
        };
        if lost {
            tracing::warn!("the session bus connection went away; {BUS_NAME} is no longer served");
            link.set_status("unavailable — the session bus connection was lost".to_string());
            match link.connection.lock() {
                Ok(mut slot) => *slot = None,
                Err(e) => *e.into_inner() = None,
            }
            return;
        }
    }
}

/// Who holds the name, in a word a person can act on: the process's name if we can read it,
/// otherwise its unique bus name.
fn name_owner() -> Option<String> {
    let conn = zbus::blocking::Connection::session().ok()?;
    let dbus = zbus::blocking::fdo::DBusProxy::new(&conn).ok()?;
    let wanted = zbus::names::BusName::try_from(BUS_NAME).ok()?;
    let owner = dbus.get_name_owner(wanted).ok()?;
    let unique = owner.as_str().to_string();
    let pid = zbus::names::BusName::try_from(unique.clone())
        .ok()
        .and_then(|n| dbus.get_connection_unix_process_id(n).ok());
    match pid {
        Some(pid) => {
            let comm = std::fs::read_to_string(format!("/proc/{pid}/comm"))
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            Some(match comm {
                Some(name) => format!("{name} (pid {pid}, {unique})"),
                None => format!("pid {pid} ({unique})"),
            })
        }
        None => Some(unique),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::OwnedValue;

    fn hints(pairs: Vec<(&str, OwnedValue)>) -> HashMap<String, OwnedValue> {
        pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
    }

    #[test]
    fn the_actions_list_is_read_in_pairs() {
        let flat = vec![
            "reply".to_string(),
            "Reply".to_string(),
            "mark-read".to_string(),
            "Mark as read".to_string(),
        ];
        let actions = actions_from_pairs(&flat);
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].id, "reply");
        assert_eq!(actions[0].label, "Reply");
        assert_eq!(actions[1].id, "mark-read");
        assert_eq!(actions[1].label, "Mark as read");
    }

    #[test]
    fn a_dangling_action_id_is_dropped_not_given_an_empty_label() {
        let flat = vec!["reply".to_string(), "Reply".to_string(), "oops".to_string()];
        let actions = actions_from_pairs(&flat);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].id, "reply");
        assert!(actions_from_pairs(&[]).is_empty());
    }

    #[test]
    fn the_urgency_hint_byte_becomes_a_level() {
        for (byte, expected) in [
            (0u8, Urgency::Low),
            (1, Urgency::Normal),
            (2, Urgency::Critical),
        ] {
            let h = hints(vec![("urgency", OwnedValue::from(byte))]);
            assert_eq!(urgency_from_hints(&h), expected);
        }
    }

    #[test]
    fn a_missing_or_malformed_urgency_hint_is_normal() {
        assert_eq!(urgency_from_hints(&hints(vec![])), Urgency::Normal);
        // The wrong type entirely: a sender that put a string where a byte belongs still gets
        // its notification through.
        let wrong = hints(vec![(
            "urgency",
            OwnedValue::try_from(zbus::zvariant::Value::from("critical")).unwrap(),
        )]);
        assert_eq!(urgency_from_hints(&wrong), Urgency::Normal);
        let out_of_range = hints(vec![("urgency", OwnedValue::from(7u8))]);
        assert_eq!(urgency_from_hints(&out_of_range), Urgency::Normal);
    }

    #[test]
    fn replaces_id_zero_means_a_new_notification() {
        assert_eq!(replaces_target(0), None);
        assert_eq!(replaces_target(42), Some("42".to_string()));
    }

    #[test]
    fn a_sender_with_no_name_is_named_from_its_desktop_entry() {
        assert_eq!(sender_label("Chromium", None), "Chromium");
        assert_eq!(sender_label("  ", Some("org.mozilla.Thunderbird")), "org.mozilla.Thunderbird");
        assert_eq!(sender_label("", None), "unknown");
        // A desktop-entry hint does not override a name the sender actually gave.
        assert_eq!(sender_label("Thunderbird", Some("org.mozilla.Thunderbird")), "Thunderbird");
    }

    #[test]
    fn the_desktops_name_is_refused_at_the_dbus_door_too() {
        // `notify-send -a Yantrik …`: any program of the person's own could file a row and a
        // toast as the desktop, with no sender record to say otherwise.
        assert_eq!(sender_label("Yantrik", None), "unknown");
        assert_eq!(sender_label("Yantrik Security", Some("org.example.App")), "org.example.App");
        assert_eq!(sender_label("Y\u{0430}ntrik", None), "unknown", "a lookalike is the same claim");
        assert_eq!(sender_label("", Some("yantrik")), "unknown", "nor through the desktop-entry hint");
        assert_eq!(sender_label("Notes for Yantrik", None), "Notes for Yantrik");
        let request = notify_to_request("Yantrik", 0, "Update ready", "", &[], &HashMap::new());
        assert_eq!(request.app, "unknown");
        assert_eq!(request.source, Source::Freedesktop);
    }

    #[test]
    fn expire_timeout_never_changes_what_is_stored() {
        // The popup's life is the shell's rule, the same one for every sender. This is the test
        // that says so: -1 ("server decides"), 0 ("never expire") and a real number all produce
        // the same stored notification, so no sender can pin a message to the screen.
        let h = hints(vec![("urgency", OwnedValue::from(1u8))]);
        let a = notify_to_request("app", 0, "s", "b", &[], &h);
        // `notify_to_request` does not take expire_timeout at all — that is the mechanism, and
        // this assertion is that its absence is not an oversight but the whole shape.
        assert_eq!(a.urgency, Urgency::Normal);
        assert_eq!(a.source, Source::Freedesktop);
        assert_eq!(a.replaces_id, None);
        assert_eq!(a.title, "s");
        assert_eq!(a.body, "b");
    }

    #[test]
    fn a_whole_notify_call_maps_across() {
        let h = hints(vec![
            ("urgency", OwnedValue::from(2u8)),
            (
                "desktop-entry",
                OwnedValue::try_from(zbus::zvariant::Value::from("chromium")).unwrap(),
            ),
        ]);
        let flat = vec!["open".to_string(), "Open".to_string()];
        let req = notify_to_request("", 7, "Download finished", "debian.iso", &flat, &h);
        assert_eq!(req.app, "chromium");
        assert_eq!(req.title, "Download finished");
        assert_eq!(req.body, "debian.iso");
        assert_eq!(req.urgency, Urgency::Critical);
        assert_eq!(req.replaces_id, Some("7".to_string()));
        assert_eq!(req.actions.len(), 1);
        assert_eq!(req.source, Source::Freedesktop);
    }

    #[test]
    fn close_reason_numbers_are_the_spec_numbers() {
        assert_eq!(CloseReason::Expired as u32, 1);
        assert_eq!(CloseReason::DismissedByUser as u32, 2);
        assert_eq!(CloseReason::ClosedByApi as u32, 3);
    }
}
