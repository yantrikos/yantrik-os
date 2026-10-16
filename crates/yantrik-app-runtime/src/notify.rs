//! Tell the person something, in one line, from anywhere.
//!
//! ## Why this exists
//!
//! Until now an app had no way to raise a notification at all. A download finished, an event was
//! about to start, an update was waiting — and none of it left the app's own window. The shell
//! had a private toast queue, but the shell is not a library and an app is a separate process;
//! the notifications service existed and **nothing in the tree ever called it**.
//!
//! ```rust,ignore
//! use yantrik_app_runtime::notify::{self, Notification};
//!
//! notify::send(
//!     Notification::new("Downloads", "debian-13.iso finished")
//!         .body("Saved to ~/Downloads")
//!         .action("open_folder", "Open folder"),
//! );
//! ```
//!
//! ## What it promises
//!
//! **It never blocks the caller.** Every call hands the work to a short-lived thread and returns.
//! An app's UI thread has a three-second budget for a whole control-surface round trip; a
//! notification must not spend any of it. There is a test that calls `send` with no service
//! running and asserts the call returned in milliseconds.
//!
//! **It starts the service.** The notifications service is autostarted by the shell, but a
//! machine where it died, or an app launched before the shell finished, would otherwise silently
//! drop the message. `service::ensure` is the same on-demand start the calendar and email use.
//!
//! **It is bounded, and it says so once.** If the service cannot be reached the message is lost —
//! there is nowhere else to put it — and that is logged the first time and not again, because a
//! failing service plus a chatty retry is how a log stops being readable. At most
//! [`MAX_IN_FLIGHT`] sends are in the air at once; past that a send is dropped rather than
//! spawning threads without limit, since anything on this machine can call this.
//!
//! **It does not carry its own store.** Everything goes to the one service, so a notification
//! raised by an app is the same object the shell draws, the notification centre lists, and a
//! mind reads in `describe`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Once;
use std::time::Duration;

use yantrik_ipc_contracts::notifications::{
    AddRequest, NotificationAction, Source, Urgency, ADD,
};
use yantrik_ipc_transport::SyncRpcClient;

pub use yantrik_ipc_contracts::notifications::Urgency as Level;

/// The service id, as the socket is named.
const SERVICE: &str = "notifications";

/// How long one send may take on its own thread, after the service is known to be up. The store
/// is a mutex and a file write; anything past this is a service in trouble.
const CALL_TIMEOUT: Duration = Duration::from_secs(2);

/// How many sends may be in the air at once.
///
/// A ceiling rather than a queue: a caller in a loop should lose notifications, not accumulate
/// threads. Three at once is already more than a person can read.
pub const MAX_IN_FLIGHT: usize = 8;

static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
static UNREACHABLE_SAID: Once = Once::new();
static FLOODED_SAID: Once = Once::new();

/// One thing to tell the person.
#[derive(Debug, Clone)]
pub struct Notification {
    app: String,
    title: String,
    body: String,
    urgency: Urgency,
    actions: Vec<NotificationAction>,
    replaces_id: Option<String>,
}

impl Notification {
    /// Who is speaking, as the person would recognise it, and the one line being said.
    pub fn new(app: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            app: app.into(),
            title: title.into(),
            body: String::new(),
            urgency: Urgency::Normal,
            actions: Vec::new(),
            replaces_id: None,
        }
    }

    /// A sentence or two of detail under the title.
    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.body = body.into();
        self
    }

    /// How loud. `critical` stays on screen until it is dismissed and survives Do Not Disturb;
    /// use it for something the person loses by not seeing.
    pub fn urgency(mut self, urgency: Urgency) -> Self {
        self.urgency = urgency;
        self
    }

    /// A button, naming an action on **this app's own control surface**.
    ///
    /// When it is pressed the shell calls that action on the app, starting the app if it is not
    /// running — so `.action("open_folder", "Open folder")` works for an app that publishes
    /// `open_folder` and takes no arguments, and does nothing at all for one that does not.
    /// Only add a button for an action the surface really has; a button that does nothing is
    /// the dead control this repo forbids.
    pub fn action(mut self, id: impl Into<String>, label: impl Into<String>) -> Self {
        self.actions.push(NotificationAction::new(id, label));
        self
    }

    /// The same, for an action that needs arguments — Download Manager's "Open folder" is
    /// `open_folder` with `{"id": 4}`, the same call its own button makes.
    pub fn action_with(
        mut self,
        id: impl Into<String>,
        label: impl Into<String>,
        args: serde_json::Value,
    ) -> Self {
        self.actions
            .push(NotificationAction::new(id, label).with_args(args));
        self
    }

    /// Update an earlier notification instead of adding another, by its id.
    ///
    /// For a sender that reports the same thing twice — "downloading…" then "finished" — so the
    /// notification centre holds one line about it rather than a history of one file.
    pub fn replaces(mut self, id: impl Into<String>) -> Self {
        self.replaces_id = Some(id.into());
        self
    }

    fn into_request(self) -> AddRequest {
        AddRequest {
            app: self.app,
            title: self.title,
            body: self.body,
            urgency: self.urgency,
            actions: self.actions,
            source: Source::Yantrik,
            replaces_id: self.replaces_id,
            // A mind's notice comes only through the harness host, never through an app.
            notice: None,
        }
    }
}

/// Send it. Returns at once; the work happens on a thread of its own.
pub fn send(notification: Notification) {
    // Claim a slot before spawning, and give it back in the thread. Claim-then-check rather than
    // check-then-claim: two threads reading `< MAX` at the same moment would both pass.
    if IN_FLIGHT.fetch_add(1, Ordering::SeqCst) >= MAX_IN_FLIGHT {
        IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
        FLOODED_SAID.call_once(|| {
            tracing::warn!(
                max = MAX_IN_FLIGHT,
                "dropping a notification: too many sends already in flight. Something is \
                 notifying in a loop."
            );
        });
        return;
    }

    let request = notification.into_request();
    let spawned = std::thread::Builder::new()
        .name("yantrik-notify".into())
        .spawn(move || {
            deliver(request);
            IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
        });

    if spawned.is_err() {
        IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
        tracing::warn!("could not spawn a thread to send a notification");
    }
}

/// The blocking half. Only ever called on the thread `send` made.
fn deliver(request: AddRequest) {
    let app = request.app.clone();
    let title = request.title.clone();

    if let Err(e) = crate::service::ensure(SERVICE) {
        report_unreachable(&app, &title, &e);
        return;
    }

    let params = match serde_json::to_value(&request) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "a notification could not be serialised");
            return;
        }
    };

    match SyncRpcClient::for_service(SERVICE)
        .with_timeout(CALL_TIMEOUT)
        .call(ADD, params)
    {
        Ok(stored) => {
            tracing::debug!(
                id = %stored["id"].as_str().unwrap_or("?"),
                app = %app,
                "notification sent"
            );
        }
        Err(e) => report_unreachable(&app, &title, &e.message),
    }
}

/// Said once. A notifications service that is down will be down for every send after this one,
/// and a line per attempt buries the first one, which is the only one that says when it started.
fn report_unreachable(app: &str, title: &str, why: &str) {
    UNREACHABLE_SAID.call_once(|| {
        tracing::warn!(
            app,
            title,
            error = why,
            "the notifications service could not be reached — this notification is lost, and \
             further ones will be dropped without another line until it comes back"
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn a_send_returns_immediately_with_no_service_running() {
        // The whole point. This runs in a test process with no notifications socket and no
        // shell to ask for one, so the delivery thread will spend a couple of seconds failing —
        // and the caller, which in a real app is the UI thread, must not wait for any of it.
        let _env = crate::env_lock();
        std::env::set_var("XDG_RUNTIME_DIR", std::env::temp_dir());
        let started = Instant::now();
        for i in 0..3 {
            send(Notification::new("test", format!("nobody is listening {i}")));
        }
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_millis(250),
            "send blocked the caller for {elapsed:?}"
        );
    }

    #[test]
    fn the_number_of_sends_in_flight_is_bounded() {
        // Anything on this machine can call `send`. A caller in a loop has to lose
        // notifications rather than spawn threads until the process dies.
        let _env = crate::env_lock();
        std::env::set_var("XDG_RUNTIME_DIR", std::env::temp_dir());
        for i in 0..(MAX_IN_FLIGHT * 20) {
            send(Notification::new("flood", format!("{i}")));
        }
        assert!(
            IN_FLIGHT.load(Ordering::SeqCst) <= MAX_IN_FLIGHT,
            "in-flight sends went past the ceiling"
        );
    }

    #[test]
    fn the_builder_fills_in_what_a_one_line_send_leaves_out() {
        let request = Notification::new("Downloads", "debian.iso finished")
            .body("Saved to ~/Downloads")
            .urgency(Urgency::Low)
            .action("open_folder", "Open folder")
            .into_request();
        assert_eq!(request.app, "Downloads");
        assert_eq!(request.urgency, Urgency::Low);
        assert_eq!(request.actions.len(), 1);
        assert_eq!(request.actions[0].label, "Open folder");
        assert_eq!(request.replaces_id, None);
        // Everything from here is ours, never freedesktop — that source is set by the door a
        // notification came through, and this one came through the socket.
        assert_eq!(request.source, Source::Yantrik);

        let minimal = Notification::new("Calendar", "Standup in 10 minutes").into_request();
        assert_eq!(minimal.body, "");
        assert_eq!(minimal.urgency, Urgency::Normal);
        assert!(minimal.actions.is_empty());
    }
}
