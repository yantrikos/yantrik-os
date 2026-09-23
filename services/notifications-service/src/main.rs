//! Notifications service — the one owner of notifications on this machine.
//!
//! ## What this was
//!
//! A `Mutex<Vec<Notification>>` with five methods, autostarted by the shell, and **not one
//! caller anywhere in the tree**. The store was in memory, so anything it had ever held was gone
//! at the next boot — which never mattered, because nothing ever put anything in it.
//!
//! Meanwhile three other things were doing this job. `mako` held
//! `org.freedesktop.Notifications` and drew popups in its own style that the shell could not
//! see. The shell *also* implemented that interface, in `yantrik-os::dbus_notif`, and raced mako
//! for the name. And the shell had a private toast queue and a private JSON file in
//! `~/.yantrik/`, fed by screenshots, focus mode and the companion bridge, that no service and no
//! mind could read. Four notification systems; none of them knew about the others; apps had no
//! way to raise one at all.
//!
//! ## What it is now
//!
//! Everything that wants to tell the person something calls this service:
//!
//! ```text
//!   notify-send / Chromium / any app  ──org.freedesktop.Notifications──┐
//!   yos notify                        ──notifications.add─────────────┤
//!   download-manager, calendar        ──yantrik_app_runtime::notify───┤──► store (one file)
//!   the shell (updates, the mind)     ──notifications.add─────────────┘        │
//!                                                                              │
//!   the shell's toasts + screen 9     ◄──notifications.since(revision)─────────┘
//! ```
//!
//! The store is [`store::Store`]: one bounded file that survives a restart, with a revision so
//! the shell can poll cheaply. The freedesktop door is [`freedesktop`], in this process because
//! the store is in this process.
//!
//! Do Not Disturb is *not* here. It decides whether a toast pops, which is a question about the
//! screen; everything is stored and counted either way.
//!
//! ## Who sent it
//!
//! Found on 22 September 2026 (#114). A mind posted, as `app: "Yantrik"`, that Studio had made
//! three pictures and one had taken 41 seconds; Studio had made one, in 0.05 s. The record in
//! the store had the name, the false sentence and nothing about who had called. The kernel had
//! stamped the caller's pid on the socket the whole time (`SO_PEERCRED`) and this handler was
//! the one place that threw it away — `notify` filled in `Yantrik` for anything that gave no
//! name, and took any name that was given at its word.
//!
//! The approval card already answers this properly, with two lines it refuses to merge: what
//! the caller called itself, and the program `/proc` says opened the socket. Every notification
//! that comes through the socket carries the same two now — see [`attribute`] — filled in at the
//! moment the call arrives, which is the one moment the peer is certainly still there, and the
//! shell draws them in the card's words. Nothing in the request can set any of it.

mod freedesktop;
mod store;

use std::sync::Arc;

use yantrik_ipc_contracts::notifications::*;
use yantrik_ipc_transport::peer_identity::{self, Program};
use yantrik_ipc_transport::PeerCred;
#[cfg(test)]
use yantrik_service_sdk::gate::{self, Authority};
use yantrik_service_sdk::prelude::*;
use yantrik_service_sdk::{caller, Action, Param, Surface, View};

/// The id this surface publishes, and the app a grant for one of its actions is bound to.
const APP: &str = "notifications";

fn main() {
    // Before `run_service`, because the bus name is claimed on a thread of its own and the
    // outcome has to be in the log before the first `describe` asks about it. `init_tracing` is
    // idempotent and public for exactly this.
    yantrik_service_sdk::init_tracing("notifications");

    let store = Arc::new(store::Store::open(store::default_path()));
    if let Some(notice) = store.load_notice() {
        tracing::warn!("{notice}");
    }
    let (showing, held) = store.held();
    tracing::info!(
        path = %store.path().display(),
        showing,
        held,
        revision = store.revision(),
        "notification store opened"
    );

    let link = Arc::new(freedesktop::Link::new());

    // A plain std thread, not a tokio task: `zbus::blocking` drives its own async-io reactor and
    // blocking inside a tokio worker panics. `run_service` builds the tokio runtime *after*
    // this, so this thread is never one of its workers.
    {
        let store = store.clone();
        let link = link.clone();
        std::thread::Builder::new()
            .name("yos-freedesktop-notifications".into())
            .spawn(move || freedesktop::serve(store, link))
            .expect("failed to spawn the freedesktop notification thread");
    }

    ServiceBuilder::new("notifications")
        .handler(NotificationsHandler::new(store, link))
        .run();
}

struct NotificationsHandler {
    store: Arc<store::Store>,
    link: Arc<freedesktop::Link>,
    /// `app.describe` and `app.act`, dispatched as an app window's are.
    surface: Surface,
}

impl NotificationsHandler {
    fn new(store: Arc<store::Store>, link: Arc<freedesktop::Link>) -> NotificationsHandler {
        let surface = notifications_surface(store.clone(), link.clone());
        NotificationsHandler { store, link, surface }
    }
}

impl ServiceHandler for NotificationsHandler {
    fn service_id(&self) -> &str {
        "notifications"
    }

    /// Nothing reaches this: the transport calls [`ServiceHandler::handle_from`] with the
    /// peer's credentials for every request off the socket. It stays because the trait requires
    /// it, and it answers as a request with no credentials — "could not be identified" — rather
    /// than as anything more flattering.
    fn handle(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, ServiceError> {
        self.dispatch(method, params, None)
    }

    fn handle_from(
        &self,
        method: &str,
        params: serde_json::Value,
        peer: Option<PeerCred>,
    ) -> Result<serde_json::Value, ServiceError> {
        self.dispatch(method, params, peer)
    }
}

impl NotificationsHandler {
    fn dispatch(
        &self,
        method: &str,
        params: serde_json::Value,
        peer: Option<PeerCred>,
    ) -> Result<serde_json::Value, ServiceError> {
        // The agent-facing surface: what the machine is trying to tell the person, right now, in
        // one line and a small list — without opening the notification centre — and the four
        // things a mind may do about it. The ceiling and the mode are read per call, as an app
        // window's dispatch reads them. The desktop's own senders do not come this way: they call
        // `notifications.add` below.
        if let Some(answer) = self.surface.answer(method, &params, peer) {
            return answer;
        }
        match method {
            LIST => Ok(serde_json::to_value(self.store.list()).unwrap_or_default()),

            ADD => {
                let request = parse_add(&params)?;
                let who = who_is_calling(peer);
                let (app, sender) = attribute(&request.app, &who);
                let stored = self.store.add_from(AddRequest { app, ..request }, Some(sender));
                tracing::info!(
                    id = %stored.id,
                    app = %stored.app,
                    urgency = stored.urgency.as_str(),
                    sender = %who.line(),
                    "notification stored"
                );
                Ok(serde_json::to_value(stored).unwrap_or_default())
            }

            // The shell's poll. Cheap on purpose: it runs about once a second for as long as the
            // desktop is up, and the answer is empty almost every time.
            SINCE => {
                let revision = params["revision"].as_u64().unwrap_or(0);
                Ok(serde_json::to_value(self.store.since(revision)).unwrap_or_default())
            }

            DISMISS => {
                let id = required_str(&params, "id")?;
                let Some(n) = self.store.get(&id) else {
                    return Err(bad_request(format!("no notification with id `{id}`")));
                };
                if !self.store.dismiss(&id) {
                    // It exists and was already dismissed. Not an error — the caller wanted it
                    // gone and it is gone — but say which, so a caller is never told it changed
                    // something it did not.
                    return Ok(serde_json::json!({ "dismissed": id, "already": true }));
                }
                self.link.closed(&n, freedesktop::CloseReason::DismissedByUser);
                Ok(serde_json::json!({ "dismissed": id, "already": false }))
            }

            DISMISS_ALL => {
                let showing = self.store.showing();
                let count = self.store.dismiss_all();
                for n in &showing {
                    self.link.closed(n, freedesktop::CloseReason::DismissedByUser);
                }
                Ok(serde_json::json!({ "dismissed": count }))
            }

            MARK_READ => {
                let id = optional_str(&params, "id")?;
                let count = self.store.mark_read(id.as_deref());
                Ok(serde_json::json!({ "marked_read": count }))
            }

            ACTION => {
                let id = required_str(&params, "id")?;
                let action_id = required_str(&params, "action_id")?;
                let invoked = self
                    .store
                    .invoke(&id, &action_id)
                    .map_err(bad_request)?;
                // The sender hears about it. Without this, an action button on a notification
                // from any program but ours is a button that does nothing.
                self.link.action_invoked(&invoked, &action_id);
                tracing::info!(id = %id, action = %action_id, app = %invoked.app, "action invoked");
                Ok(serde_json::json!({
                    "id": invoked.id,
                    "action_id": action_id,
                    "app": invoked.app,
                    "source": invoked.source.as_str(),
                    "told_the_sender": invoked.source == Source::Freedesktop,
                }))
            }

            other => Err(ServiceError {
                code: -32601,
                message: format!("Unknown method: {other}"),
            }),
        }
    }

    /// `app.act` under a pinned authority, as the socket's dispatch runs it with
    /// `Authority::now()`. The tests' door.
    #[cfg(test)]
    fn act(
        &self,
        params: &serde_json::Value,
        peer: Option<PeerCred>,
        authority: Authority,
    ) -> Result<serde_json::Value, ServiceError> {
        self.surface.act(params, peer, authority)
    }
}

/// Everything the machine is currently trying to say, newest first, with the counts a caller
/// reading one line needs — and, plainly, whether the freedesktop door is open.
fn describe_view(store: &store::Store, link: &freedesktop::Link) -> View {
    let showing = store.list();
    let unread = store.unread();
    let critical = showing
        .iter()
        .filter(|n| n.urgency == Urgency::Critical && !n.read)
        .count();
    let (_showing, held) = store.held();

    let summary = if showing.is_empty() {
        "Notifications — nothing pending".to_string()
    } else if critical > 0 {
        format!(
            "Notifications — {} showing, {unread} unread, {critical} critical",
            showing.len()
        )
    } else {
        format!("Notifications — {} showing, {unread} unread", showing.len())
    };

    let items: Vec<serde_json::Value> = showing
        .iter()
        .take(20)
        .map(|n| {
            serde_json::json!({
                "id": n.id,
                "app": n.app,
                "title": n.title,
                "body": n.body,
                "urgency": n.urgency.as_str(),
                "source": n.source.as_str(),
                // Who this machine says sent it, beside `app`, which is who they said. A
                // caller reading this list can compare the two, which is the whole point.
                "sender": n.sender,
                "read": n.read,
                "at": n.created_at,
                "actions": n.actions.iter().map(|a| a.id.clone()).collect::<Vec<_>>(),
            })
        })
        .collect();

    let mut view = View::new(summary)
        .with("count", showing.len() as i64)
        .with("unread", unread as i64)
        .with("critical", critical as i64)
        .with("held", held as i64)
        .with("revision", store.revision() as i64)
        .with("store", store.path().display().to_string())
        // Not a boolean: when this door is shut the caller needs to know who shut it, and
        // `false` would send them looking through logs for the name.
        .with("freedesktop", link.status())
        .with("notifications", serde_json::Value::Array(items));

    // Failure said twice: the person sees an empty notification centre, and a caller reading
    // this sees why it is empty.
    if let Some(notice) = store.load_notice() {
        view = view.with("notice", notice.to_string());
    }
    view
}

/// The notifications surface, answering on the service's own socket.
///
/// Every action first meets the rule an app window's dispatch enforces — the machine's ceiling,
/// then any grant, then the person's mode — on the grade this surface publishes for it, and the
/// argument checks. This service used to dispatch straight away, whatever the ceiling said (#153).
/// Everything here is `standard`, which the mode runs unasked in every mode (`SOCKET_FLOOR`), so
/// what the gate changes in practice is the ceiling: a machine set to `safe` refuses `notify` on
/// this door as it refuses every app's `standard` actions.
fn notifications_surface(store: Arc<store::Store>, link: Arc<freedesktop::Link>) -> Surface {
    let mut surface = Surface::new(APP).socket_name("notifications").describe({
        let (store, link) = (store.clone(), link.clone());
        move || describe_view(&store, &link)
    });
    for spec in notification_actions() {
        let (store, link) = (store.clone(), link.clone());
        surface = match spec.name.as_str() {
            "notify" => surface.action(spec, move |args| notify(&store, args)),
            "dismiss" => surface.action(spec, move |args| dismiss(&store, &link, args)),
            "dismiss_all" => surface.action(spec, move |_| dismiss_all(&store, &link)),
            "mark_read" => surface.action(spec, move |args| mark_read(&store, args)),
            // Published and graded, but no handler here: a mistake in this file.
            // `every_published_action_has_a_handler` keeps it from shipping.
            _ => surface,
        };
    }
    surface
}

/// The one a mind reaches for when it says "I'll tell you when it's done" — and then has to
/// actually tell them.
///
/// Who sent it is what the kernel says opened the socket, read now — the dispatch hands the
/// caller to this handler (`caller()`) for exactly this call — and never what the call says.
fn notify(store: &store::Store, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let title = required_str(args, "title").map_err(|e| e.message)?;
    let who = who_is_calling(caller().map(PeerCred::from));
    let (app, sender) = attribute(args["app"].as_str().unwrap_or_default(), &who);
    let stored = store.add_from(
        AddRequest {
            app,
            title,
            body: args["body"].as_str().unwrap_or_default().to_string(),
            urgency: Urgency::parse(args["urgency"].as_str().unwrap_or("normal")),
            source: Source::Yantrik,
            ..Default::default()
        },
        Some(sender),
    );
    tracing::info!(
        id = %stored.id,
        app = %stored.app,
        sender = %who.line(),
        "notification stored by `notify`"
    );
    // The caller is told what it was filed under and what was recorded about it, so a mind that
    // said `Yantrik` learns on the spot that the row will not.
    Ok(serde_json::json!({ "id": stored.id, "app": stored.app, "sender": stored.sender }))
}

fn dismiss(
    store: &store::Store,
    link: &freedesktop::Link,
    args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let id = required_str(args, "id").map_err(|e| e.message)?;
    let Some(n) = store.get(&id) else {
        return Err(format!("no notification with id `{id}`"));
    };
    let changed = store.dismiss(&id);
    if changed {
        link.closed(&n, freedesktop::CloseReason::DismissedByUser);
    }
    Ok(serde_json::json!({ "dismissed": id, "already": !changed }))
}

fn dismiss_all(store: &store::Store, link: &freedesktop::Link) -> Result<serde_json::Value, String> {
    let showing = store.showing();
    let cleared = store.dismiss_all();
    for n in &showing {
        link.closed(n, freedesktop::CloseReason::DismissedByUser);
    }
    Ok(serde_json::json!({ "dismissed": cleared }))
}

fn mark_read(store: &store::Store, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let id = optional_str(args, "id").map_err(|e| e.message)?;
    let count = store.mark_read(id.as_deref());
    Ok(serde_json::json!({ "marked_read": count }))
}

/// The grade this surface publishes for `action`, from the same table `describe` hands out, so
/// the grade a caller is shown and the grade that is enforced cannot come apart.
#[cfg(test)]
fn published_grade(action: &str) -> Option<&'static str> {
    notification_actions().into_iter().find(|a| a.name == action).map(|a| a.permission)
}

/// What the notifications service can be asked to do.
fn notification_actions() -> Vec<Action> {
    vec![
        // `standard`, not `safe`: it puts something on the person's screen. It is not
        // `sensitive` either — a notification changes nothing and reaches nowhere outside this
        // machine, and grading it higher would put a confirmation in front of the one thing a
        // mind needs in order to keep a promise it made out loud.
        Action::new(
            "notify",
            "Tell the person something. Shows as a toast and is kept in the notification \
             centre. Use it to finish a promise — \"I'll tell you when the build is done\".",
        )
        .risk("standard")
        .arg(Param::text("title").describe("One line, the thing being said"))
        .arg(
            Param::text("body")
                .optional()
                .describe("A sentence or two of detail"),
        )
        .arg(
            Param::text("urgency")
                .optional()
                .describe("low, normal (default) or critical. critical stays until dismissed"),
        )
        .arg(
            Param::text("app")
                .optional()
                .describe(
                    "Who is speaking, as the person would recognise it. Kept as your claim \
                     beside the program this machine verified sent it; leave it out to be \
                     filed under that program's own name. `Yantrik` is the desktop's and is \
                     not granted to anything else",
                ),
        ),
        Action::new("dismiss", "Dismiss one notification by id")
            .risk("standard")
            .arg(Param::text("id").describe("The notification id, as shown in the list")),
        Action::new("dismiss_all", "Dismiss every notification now showing").risk("standard"),
        Action::new(
            "mark_read",
            "Clear the unread badge — for one notification with `id`, or all of them without it",
        )
        .risk("standard")
        .arg(Param::text("id").optional().describe("One notification, or omit for all")),
    ]
}

// ── Who sent it ─────────────────────────────────────────────────────────────────────────────

/// The desktop's own name. Granted as `app` only to the desktop itself; anything else that
/// claims it is filed under its own program, and the claim is kept beside it.
const OS_NAME: &str = "Yantrik";

/// The desktop itself: the shell, and this service. The only callers `Yantrik` belongs to.
const DESKTOP_BINARIES: &[&str] = &["yantrik-ui", "notifications-service"];

/// The store's last resort for a row with no name, as it always was.
const NAMELESS: &str = "unknown";

/// Is the program on the socket the desktop itself?
///
/// Judged by the DIRECT peer, not by the first recognisable ancestor. The shell sends its own
/// notifications from its own process, so its peer *is* `yantrik-ui`. A mind driven through a
/// bridge the shell spawned has `yantrik-ui` above it but `yos` on the socket, and it is the
/// mind, not the shell — the card's walk names the shell for it, which is right for a card and
/// wrong for handing out the shell's name.
fn is_the_desktop(who: &Program) -> bool {
    who.direct
        .as_ref()
        .is_some_and(|f| DESKTOP_BINARIES.contains(&peer_identity::basename(&f.exe)))
}

/// What this machine can establish about whoever is on the socket, read now.
///
/// Now and not later: the peer is waiting for this call's answer, so it is alive, and for a
/// mind's request it is `yos`, which exits the moment the answer arrives.
fn who_is_calling(peer: Option<PeerCred>) -> Program {
    peer_identity::resolve(peer.map(|p| p.pid))
}

/// The record of who sent a notification, and the `app` it is filed under.
///
/// `app_given` is the caller's word, verbatim or empty. It is kept as the claim whatever it
/// says; what it decides is the name on the row:
///
/// * a name the caller gave is the name on the row — except the desktop's own, which only the
///   desktop gets. A mind that says `Yantrik` is filed under its own program, and the row says
///   what it claimed beside what was verified;
/// * no name is the verified program's own name (`hermes_cli.main`, `yantrik-terminal`),
///   `Yantrik` for the desktop itself, and `unknown` when nothing could be established. Never
///   `Yantrik` by default, which is what this said for every nameless call from anything.
///
/// A caller nothing could be established about — no credentials, an unreadable `/proc` — is
/// not the desktop. That is the direction to fail in: the name is worth taking only if the
/// machine can say who did not get it.
fn attribute(app_given: &str, who: &Program) -> (String, Sender) {
    let claimed = app_given.trim();
    let claimed = (!claimed.is_empty()).then(|| claimed.to_string());
    let desktop = is_the_desktop(who);
    let app = match claimed.as_deref() {
        Some(name) if name.eq_ignore_ascii_case(OS_NAME) && !desktop => who.name(),
        Some(name) => name.to_string(),
        None if desktop => OS_NAME.to_string(),
        None => who.name(),
    };
    let app = if app.is_empty() { NAMELESS.to_string() } else { app };
    let sender = Sender {
        claimed,
        verified: who.line(),
        pid: who.pid(),
        exe: who.exe(),
    };
    (app, sender)
}

// ── Parsing ─────────────────────────────────────────────────────────────────────────────────

fn bad_request(message: String) -> ServiceError {
    ServiceError {
        code: -32602,
        message,
    }
}

/// One text argument that may have arrived as a number.
///
/// Every id this store hands out is a counter rendered as a string — "67", "75" — and a caller
/// that puts one back through anything which reads JSON gets a number instead. `yos act
/// notifications dismiss id=67` did exactly that, and this service read a non-string as no
/// string at all: a mind following the published describe to the letter was told "missing `id`"
/// for an id it had just read off the list, and had no way to learn better from the words.
///
/// `yos` now keeps a value bound for a `string` parameter as text, so that route is fixed at
/// source. This is the other half: a number is what the caller meant either way, whatever
/// plumbing it came through, so it is read as the id it spells. Anything else — an object, an
/// array, a flag — is not an id in any spelling, and is refused by name below.
fn as_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn kind_of(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "a boolean",
        serde_json::Value::Number(_) => "a number",
        serde_json::Value::String(_) => "a string",
        serde_json::Value::Array(_) => "an array",
        serde_json::Value::Object(_) => "an object",
    }
}

fn required_str(params: &serde_json::Value, key: &str) -> Result<String, ServiceError> {
    let value = params.get(key).unwrap_or(&serde_json::Value::Null);
    if value.is_null() {
        return Err(bad_request(format!("missing `{key}`")));
    }
    match as_text(value).filter(|s| !s.trim().is_empty()) {
        Some(text) => Ok(text),
        // "missing" was what this said for a value that was plainly there, which sent every
        // reader looking for an argument it had already sent. Say what arrived and what was
        // wanted instead.
        None if value.is_string() => Err(bad_request(format!("missing `{key}`"))),
        None => Err(bad_request(format!(
            "`{key}` must be a string, and {} arrived",
            kind_of(value)
        ))),
    }
}

/// The same reading for an argument that may simply be absent.
///
/// The distinction matters more here than anywhere: `mark_read` with no id marks EVERY
/// notification read, so a number that fell through `as_str` did not fail — it quietly cleared
/// the whole centre for a caller that had asked about one line of it.
///
/// Only an absent argument means "all of them". An id that is present and unreadable is an
/// error, and an empty one stays an id that matches nothing, because widening either of those
/// into the whole list is the same accident by another route.
fn optional_str(params: &serde_json::Value, key: &str) -> Result<Option<String>, ServiceError> {
    match params.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => match as_text(value) {
            Some(text) => Ok(Some(text)),
            None => Err(bad_request(format!(
                "`{key}` must be a string naming one notification, or be left out to mean all \
                 of them, and {} arrived",
                kind_of(value)
            ))),
        },
    }
}

/// Read an `notifications.add` payload.
///
/// `body` is optional and `app` may be left out, because the shortest useful call is a title —
/// and the old handler made both `title` and `body` required, so the one-line send every caller
/// actually wants was a -32602. Urgency is parsed leniently for the same reason: a typo in one
/// field is not worth losing the message over.
///
/// An absent `app` comes out empty, not as a name. Which name goes on the row is decided by
/// [`attribute`], from what the kernel says about the caller; this parser has no such fact.
fn parse_add(params: &serde_json::Value) -> Result<AddRequest, ServiceError> {
    let title = required_str(params, "title")?;
    let actions = params["actions"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|a| {
                    let id = a["id"].as_str()?.to_string();
                    let label = a["label"].as_str().unwrap_or(&id).to_string();
                    Some(NotificationAction {
                        id,
                        label,
                        args: a.get("args").filter(|v| v.is_object()).cloned(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(AddRequest {
        // `app_id` as well as `app`: the old method took `app_id`, and a caller written against
        // it should not silently start reporting itself as nameless.
        app: params["app"]
            .as_str()
            .or_else(|| params["app_id"].as_str())
            .unwrap_or_default()
            .to_string(),
        title,
        body: params["body"].as_str().unwrap_or_default().to_string(),
        urgency: Urgency::parse(params["urgency"].as_str().unwrap_or("normal")),
        actions,
        source: match params["source"].as_str() {
            Some("freedesktop") => Source::Freedesktop,
            _ => Source::Yantrik,
        },
        replaces_id: optional_str(params, "replaces_id")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_title_is_the_whole_of_a_minimum_send() {
        // The old handler required `body` too, so the one-line call every caller wants was a
        // parameter error. Nothing called it, which is how that survived.
        let req = parse_add(&serde_json::json!({ "title": "Build finished" })).unwrap();
        assert_eq!(req.title, "Build finished");
        assert_eq!(req.body, "");
        // No name is no name. What goes on the row is `attribute`'s decision, from the caller's
        // credentials, which a parser does not have.
        assert_eq!(req.app, "");
        assert_eq!(req.urgency, Urgency::Normal);
        assert_eq!(req.source, Source::Yantrik);
    }

    #[test]
    fn a_send_with_no_title_is_refused_and_says_which_field() {
        let err = parse_add(&serde_json::json!({ "body": "no title here" })).unwrap_err();
        assert_eq!(err.code, -32602);
        assert!(err.message.contains("title"), "{}", err.message);
        // Whitespace is not a title either.
        assert!(parse_add(&serde_json::json!({ "title": "   " })).is_err());
    }

    #[test]
    fn the_old_app_id_spelling_still_names_the_sender() {
        let req = parse_add(&serde_json::json!({ "title": "x", "app_id": "downloads" })).unwrap();
        assert_eq!(req.app, "downloads");
    }

    #[test]
    fn actions_without_a_label_fall_back_to_their_id() {
        let req = parse_add(&serde_json::json!({
            "title": "x",
            "actions": [{ "id": "open_folder", "label": "Open folder" }, { "id": "retry" }],
        }))
        .unwrap();
        assert_eq!(req.actions.len(), 2);
        assert_eq!(req.actions[1].label, "retry");
    }

    #[test]
    fn an_id_that_arrived_as_a_number_is_the_id_it_spells() {
        // Found on 22 September 2026: `yos act notifications dismiss id=67` was refused with
        // "missing `id`" while `"id": "67"` over the raw socket dismissed it, because the CLI
        // ran every value through a JSON parser and this read a non-string as nothing at all.
        // The CLI keeps text as text now; a number still arrives from anything else that
        // re-reads JSON on the way, and it is not ambiguous — every id here is decimal.
        assert_eq!(
            required_str(&serde_json::json!({ "id": 67 }), "id").unwrap(),
            "67"
        );
        assert_eq!(
            optional_str(&serde_json::json!({ "id": 67 }), "id").unwrap(),
            Some("67".to_string())
        );
    }

    #[test]
    fn an_argument_that_is_not_text_at_all_is_told_what_was_wanted() {
        // The old message for every one of these was "missing `id`", which sent the reader
        // looking for an argument it had plainly sent.
        let err = required_str(&serde_json::json!({ "id": { "n": 1 } }), "id").unwrap_err();
        assert_eq!(err.code, -32602);
        assert!(err.message.contains("must be a string"), "{}", err.message);
        assert!(err.message.contains("an object"), "{}", err.message);
        // And an argument that really is absent still says so.
        assert!(required_str(&serde_json::json!({}), "id")
            .unwrap_err()
            .message
            .contains("missing"));
    }

    #[test]
    fn mark_read_with_an_unreadable_id_refuses_rather_than_clearing_everything() {
        // `mark_read` with no id marks EVERY notification read, so an id this could not read
        // used to mean the whole centre was cleared for a caller asking about one line of it.
        assert!(optional_str(&serde_json::json!({ "id": ["67"] }), "id").is_err());
        // No id at all is still the whole-list case, which is what the action publishes.
        assert_eq!(optional_str(&serde_json::json!({}), "id").unwrap(), None);
        assert_eq!(
            optional_str(&serde_json::json!({ "id": serde_json::Value::Null }), "id").unwrap(),
            None
        );
        // An empty id names nothing and marks nothing, which is what it did before. Reading it
        // as "no id given" would clear the whole centre on a caller's blank field.
        assert_eq!(
            optional_str(&serde_json::json!({ "id": "" }), "id").unwrap(),
            Some(String::new())
        );
    }

    // ── Who sent it ──

    fn facts(pid: i32, exe: &str, cmdline: &str) -> peer_identity::ProcessFacts {
        peer_identity::ProcessFacts {
            pid,
            exe: exe.into(),
            short_cmdline: cmdline.into(),
            started: pid as u64 * 100,
        }
    }

    /// Hermes through the bridge, as `ps` showed it on the VM on 22 September.
    fn hermes() -> Program {
        peer_identity::choose(vec![
            facts(7311, "/usr/bin/python3.11", "python3 yos act notifications notify title=x"),
            facts(958, "/usr/bin/python3.11", "python3 yos-mcp"),
            facts(
                689,
                "/home/yantrik/.hermes/hermes-agent/venv/bin/python",
                "python -m hermes_cli.main gateway run --replace",
            ),
            facts(1, "/usr/lib/systemd/systemd", "systemd --user"),
        ])
    }

    /// The shell, sending one of its own from its own process.
    fn shell() -> Program {
        peer_identity::choose(vec![
            facts(7456, "/opt/yantrik/bin/yantrik-ui", "yantrik-ui config.yaml"),
            facts(1, "/usr/lib/systemd/systemd", "systemd --user"),
        ])
    }

    /// Somebody typing `yos notify` into the Terminal app.
    fn terminal() -> Program {
        peer_identity::choose(vec![
            facts(9001, "/usr/bin/python3.11", "python3 yos notify done"),
            facts(8800, "/usr/bin/bash", "bash"),
            facts(812, "/opt/yantrik/bin/yantrik-terminal", "yantrik-terminal"),
            facts(7456, "/opt/yantrik/bin/yantrik-ui", "yantrik-ui config.yaml"),
        ])
    }

    #[test]
    fn an_app_given_by_a_caller_that_is_not_the_desktop_is_recorded_as_a_claim() {
        // Notification 134: the name was taken at its word and nothing else was kept. Now the
        // name is the claim, and what the kernel established sits beside it.
        let (app, sender) = attribute("Studio", &hermes());
        assert_eq!(app, "Studio", "a name that is not the desktop's is the caller's to use");
        assert_eq!(sender.claimed.as_deref(), Some("Studio"));
        assert!(sender.verified.contains("hermes_cli.main"), "{}", sender.verified);
        assert!(sender.verified.contains("pid 689"), "{}", sender.verified);
        assert_eq!(sender.pid, 689);
        assert!(sender.exe.ends_with("venv/bin/python"), "{}", sender.exe);
    }

    #[test]
    fn no_app_given_files_it_under_the_program_that_called() {
        // This used to say `Yantrik` for every nameless call, from anything on the machine.
        let (app, sender) = attribute("", &hermes());
        assert_eq!(app, "hermes_cli.main");
        assert_eq!(sender.claimed, None, "the machine chose the name; nobody claimed it");
        assert!(sender.verified.contains("pid 689"), "{}", sender.verified);

        let (app, sender) = attribute("   ", &terminal());
        assert_eq!(app, "yantrik-terminal");
        assert!(
            sender.verified.starts_with("a program started from a terminal: "),
            "{}",
            sender.verified
        );

        // Nothing established: the store's old last resort, and the card's words for it.
        let (app, sender) = attribute("", &Program::unknown());
        assert_eq!(app, NAMELESS);
        assert_eq!(sender.verified, peer_identity::UNIDENTIFIED);
        assert_eq!(sender.pid, 0);
        assert_eq!(sender.exe, "");
    }

    #[test]
    fn the_desktops_own_name_is_kept_for_the_desktop_alone() {
        // The shell's own sends — an update waiting, a mind asking — come from its own process.
        let (app, sender) = attribute("Yantrik", &shell());
        assert_eq!(app, "Yantrik");
        assert_eq!(sender.claimed.as_deref(), Some("Yantrik"));
        assert!(sender.verified.contains("yantrik-ui"), "{}", sender.verified);
        assert_eq!(attribute("", &shell()).0, "Yantrik", "the desktop's default is its own name");
        let this_service = peer_identity::choose(vec![facts(
            7469,
            "/opt/yantrik/bin/notifications-service",
            "notifications-service",
        )]);
        assert_eq!(attribute("", &this_service).0, "Yantrik");

        // A mind that says `Yantrik` is filed under itself, and the claim is kept beside it —
        // the row will say what it claimed and what was verified, and the two will differ.
        let (app, sender) = attribute("Yantrik", &hermes());
        assert_eq!(app, "hermes_cli.main");
        assert_eq!(sender.claimed.as_deref(), Some("Yantrik"));
        assert_eq!(attribute("yantrik", &hermes()).0, "hermes_cli.main", "case is not a loophole");
        assert_eq!(attribute(" Yantrik ", &hermes()).0, "hermes_cli.main", "nor is whitespace");

        // A bridge the shell itself spawned still has `yos` on the socket: it is a mind, and
        // the card's walk naming the shell above it does not make it the shell.
        let via_bridge = peer_identity::choose(vec![
            facts(9101, "/usr/bin/python3.11", "python3 yos act notifications notify title=x"),
            facts(790, "/usr/bin/python3.11", "python3 yos-mcp"),
            facts(7456, "/opt/yantrik/bin/yantrik-ui", "yantrik-ui config.yaml"),
        ]);
        assert!(!is_the_desktop(&via_bridge));
        assert_ne!(attribute("Yantrik", &via_bridge).0, "Yantrik");

        // A caller nothing could be established about does not get it either.
        let (app, sender) = attribute("Yantrik", &Program::unknown());
        assert_eq!(app, NAMELESS);
        assert_eq!(sender.claimed.as_deref(), Some("Yantrik"));

        // Only the bare name is the desktop's. "Yantrik Companion" is a name like any other,
        // and the row beside it says who really sent it.
        assert_eq!(attribute("Yantrik Companion", &hermes()).0, "Yantrik Companion");
    }

    #[test]
    fn the_surface_publishes_notify_at_standard() {
        // The point of the whole `notify` action is that a mind can use it without a
        // confirmation dialog standing between a promise and keeping it. If somebody grades it
        // up later, this says what was lost.
        let notify = notification_actions()
            .into_iter()
            .find(|a| a.name == "notify")
            .expect("notify is published");
        assert_eq!(notify.permission, "standard");
        assert_eq!(notify.schema()["permission"], "standard");
        // And the caller is told the message is on screen by the time the call returns, not
        // queued somewhere it might still be dropped.
        assert_eq!(notify.schema()["settles"], "on return");
    }

    // ── The rule every `app.act` meets (#153) ──

    /// A handler over a store of its own, with the freedesktop door never opened.
    fn handler() -> NotificationsHandler {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "yantrik-notifications-153-{}-{}.json",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_file(&path);
        NotificationsHandler::new(Arc::new(store::Store::open(path)), Arc::new(freedesktop::Link::new()))
    }

    fn at(ceiling: &str, mode: &str) -> Authority {
        Authority { ceiling: ceiling.into(), mode: gate::Mode::named(mode), granted: false }
    }

    fn notify(grant: Option<&str>) -> serde_json::Value {
        let mut params = serde_json::json!({ "action": "notify", "args": { "title": "Build finished" } });
        if let Some(grant) = grant {
            params["grant"] = grant.into();
        }
        params
    }

    /// Grants the stand-in shell spent. `ok-*` holds, anything else is refused in its words.
    static SPENT: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    /// What each grant was spent against, as the shell would have been handed it.
    static SPENT_AGAINST: std::sync::Mutex<Vec<(String, String)>> = std::sync::Mutex::new(Vec::new());

    fn spend_through_a_stand_in_shell() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            // The shell's store of agents, as the shell keeps it: the one token these tests carry
            // is a live agent's with no role, so the gate alone decides for it. Any other token
            // is no live agent's, and is refused.
            {
                use yantrik_service_sdk::reach::{keep_reach_with, token_digest, Standing};
                keep_reach_with(|digest| {
                    if digest == token_digest("tok-7f3a") {
                        Standing::Plain
                    } else {
                        Standing::Unknown
                    }
                });
            }
            gate::spend_grants_with(|id, _app, _action, args| {
                if !id.starts_with("ok-") {
                    return Err(format!("no approval request `{id}`."));
                }
                SPENT.lock().unwrap_or_else(|e| e.into_inner()).push(id.to_string());
                SPENT_AGAINST.lock().unwrap_or_else(|e| e.into_inner()).push((id.to_string(), args.to_string()));
                Ok(())
            });
        });
    }

    /// `notify` is how a mind keeps a promise it made out loud, and the desktop's own processes
    /// cross these sockets with `standard` calls the dispatch cannot yet tell from a mind's (#43).
    /// So `standard` needs no grant in any mode — plan included — here as on every app's door.
    #[test]
    fn notify_needs_no_grant_in_any_mode() {
        for mode in ["plan", "ask", "auto", "bypass"] {
            let h = handler();
            let answer = h
                .act(&notify(None), None, at("sensitive", mode))
                .unwrap_or_else(|e| panic!("{mode}: {}", e.message));
            assert_eq!(answer["accepted"], serde_json::json!(true), "{mode}");
            assert_eq!(h.store.list().len(), 1, "{mode}: the notification was stored");
        }
    }

    /// The ceiling binds this door as it binds every app's: a machine set to `safe` refuses
    /// `notify` on the grade alone, grant or none, before anything is stored — and a grant it
    /// refused was never offered to the shell (#154).
    #[test]
    fn a_ceiling_of_safe_refuses_notify_whatever_the_grant() {
        spend_through_a_stand_in_shell();
        for grant in [None, Some("ok-153-notify")] {
            let h = handler();
            let err = h.act(&notify(grant), None, at("safe", "bypass")).unwrap_err();
            assert!(
                err.message.starts_with("CEILING: notifications.notify is graded `standard`"),
                "grant={grant:?}: {}",
                err.message
            );
            assert_eq!(err.code, -32602);
            assert!(h.store.list().is_empty(), "grant={grant:?}: nothing is stored");
        }
        let spent = SPENT.lock().unwrap_or_else(|e| e.into_inner());
        assert!(!spent.iter().any(|id| id == "ok-153-notify"), "spent above the ceiling: {spent:?}");
    }

    /// An agent token travels beside `args`, never among them: one a caller put among them is
    /// taken out before the grant is spent, so the shell is handed the arguments alone.
    #[test]
    fn an_agent_token_among_the_arguments_is_not_what_a_grant_is_bound_to() {
        spend_through_a_stand_in_shell();
        let h = handler();
        let params = serde_json::json!({
            "action": "notify",
            "args": { "title": "Build finished", "agent_token": "smuggled" },
            "agent_token": "tok-7f3a",
            "grant": "ok-153-token",
        });
        h.act(&params, None, at("sensitive", "ask")).unwrap_or_else(|e| panic!("{}", e.message));
        let against = SPENT_AGAINST.lock().unwrap_or_else(|e| e.into_inner());
        let (_, args) = against.iter().find(|(id, _)| id == "ok-153-token").expect("the grant was spent");
        assert_eq!(args, r#"{"title":"Build finished"}"#);
        assert_eq!(h.store.list()[0].title, "Build finished");
    }

    /// As on a window: a grant that rides on a call is spent past the ceiling, whether or not the
    /// mode would have asked, and one the shell refuses ends the call in the shell's words.
    #[test]
    fn a_grant_that_does_not_hold_ends_the_call() {
        spend_through_a_stand_in_shell();
        let h = handler();
        let err = h.act(&notify(Some("made-up")), None, at("sensitive", "ask")).unwrap_err();
        assert!(
            err.message.starts_with("GRANT: `made-up` does not authorise notifications.notify"),
            "{}",
            err.message
        );
        assert!(h.store.list().is_empty());
    }

    /// `describe` takes no authority, and the grades it publishes are the grades `act` enforces.
    #[test]
    fn describe_needs_nothing_and_publishes_the_grades_act_enforces() {
        let described = handler().dispatch("app.describe", serde_json::json!({}), None).expect("describe");
        let actions = described["actions"].as_array().expect("actions");
        assert_eq!(actions.len(), notification_actions().len());
        for a in actions {
            let name = a["name"].as_str().unwrap();
            assert_eq!(a["permission"].as_str(), published_grade(name), "{name}");
        }
    }

    /// Every action `describe` offers reaches a handler past the gate, and an action it does not
    /// offer is answered as that before any grant is looked at.
    #[test]
    fn every_published_action_has_a_handler() {
        for spec in notification_actions() {
            let reply = handler().act(
                &serde_json::json!({ "action": spec.name, "args": {} }),
                None,
                at("dangerous", "bypass"),
            );
            if let Err(e) = reply {
                assert!(!e.message.starts_with("unknown action"), "{}: {}", spec.name, e.message);
            }
        }
        let err = handler()
            .act(
                &serde_json::json!({ "action": "clear_history", "args": {}, "grant": "made-up" }),
                None,
                at("dangerous", "ask"),
            )
            .unwrap_err();
        assert_eq!(err.code, -32602);
        assert_eq!(
            err.message,
            "unknown action `clear_history`; this app offers: notify, dismiss, dismiss_all, mark_read"
        );
    }

    /// Arguments are checked as on every app's door: an undeclared one is named, and one of the
    /// wrong type is refused by the dispatch with what arrived — where before this surface moved
    /// onto the shared dispatch, `notify body=7` stored an empty body in silence.
    #[test]
    fn the_arguments_are_checked_as_an_apps_are() {
        let h = handler();
        let err = h
            .act(&serde_json::json!({ "action": "notify", "args": { "title": "x", "colour": "red" } }), None, at("sensitive", "ask"))
            .unwrap_err();
        assert_eq!((err.code, err.message.as_str()), (-32602, "`notify` has no argument `colour`; it takes: title, body, urgency, app"));
        let err = h
            .act(&serde_json::json!({ "action": "notify", "args": { "title": "x", "body": true } }), None, at("sensitive", "ask"))
            .unwrap_err();
        assert_eq!(err.message, "`notify` argument `body` must be a string, and a boolean arrived");
        let err = h
            .act(&serde_json::json!({ "action": "notify", "args": {} }), None, at("sensitive", "ask"))
            .unwrap_err();
        assert_eq!(err.message, "`notify` needs argument `title`");
        assert!(h.store.list().is_empty(), "nothing was stored by a refused call");

        // The handler's own sentences still stand behind the dispatch's.
        let err = h
            .act(&serde_json::json!({ "action": "dismiss", "args": { "id": "404" } }), None, at("sensitive", "ask"))
            .unwrap_err();
        assert_eq!(err.message, "no notification with id `404`");
    }

    /// Each act has its own name on the service's own socket, and answers with the view after it.
    #[test]
    fn every_act_gets_its_own_action_id_and_the_view_after_it() {
        let h = handler();
        let first = h.act(&notify(None), None, at("sensitive", "ask")).unwrap();
        let second = h.act(&notify(None), None, at("sensitive", "ask")).unwrap();
        assert_ne!(first["action_id"], second["action_id"]);
        assert!(first["action_id"].as_str().unwrap().starts_with("notifications#"), "{first}");
        assert_eq!(second["state"]["count"], 2, "{second}");
        assert_eq!(second["summary"], "Notifications — 2 showing, 2 unread");
    }

    /// The surface declares nothing the dispatch cannot check.
    #[test]
    fn the_surface_is_declared_soundly() {
        assert!(handler().surface.registry().problems().is_empty());
    }

    /// End to end over a real socket: the service's own handler, bound the way `run_service`
    /// binds it, answering `app.act notify` through the shared dispatch — and the kernel's account
    /// of the caller reaching `notify` across it, which is what files the row under the program
    /// that really sent it (#114). The same call made with no socket has no caller to attribute.
    #[cfg(unix)]
    #[test]
    fn notify_over_a_real_socket_is_attributed_to_the_program_that_opened_it() {
        use std::io::{BufRead, BufReader, Write};
        use std::os::unix::net::UnixStream;

        // The ceiling and the mode are read from the files, as on a real call: an empty home
        // is the shipped defaults, `sensitive` and `ask`, under which `notify` runs unasked.
        let root = std::env::temp_dir().join(format!("notifications-socket-{}", std::process::id()));
        std::fs::create_dir_all(root.join("home")).unwrap();
        std::env::set_var("HOME", root.join("home"));
        let address = root.join("notifications-test.sock").display().to_string();

        let h = handler();
        let store = h.store.clone();
        {
            let address = address.clone();
            std::thread::spawn(move || {
                let runtime = tokio::runtime::Runtime::new().expect("a runtime");
                let _ = runtime.block_on(RpcServer::new(&address).serve(Arc::new(h)));
            });
        }
        let call = |request: serde_json::Value| -> serde_json::Value {
            for _ in 0..500 {
                if let Ok(mut socket) = UnixStream::connect(&address) {
                    socket.write_all(format!("{request}\n").as_bytes()).unwrap();
                    let mut line = String::new();
                    BufReader::new(socket).read_line(&mut line).unwrap();
                    return serde_json::from_str(&line).expect(&line);
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            panic!("nothing ever bound {address}");
        };

        let reply = call(serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "app.act",
            "params": { "action": "notify", "args": { "title": "Build finished", "app": "Yantrik" } },
        }));
        let answer = &reply["result"];
        assert_eq!(answer["accepted"], true, "{reply}");
        assert_eq!(answer["settled"], true);
        assert!(answer["action_id"].as_str().unwrap().starts_with("notifications#"), "{reply}");
        let sender = &answer["result"]["sender"];
        assert_ne!(sender["verified"], peer_identity::UNIDENTIFIED, "the caller did not cross the dispatch: {reply}");
        assert!(sender["pid"].as_i64().unwrap_or(0) > 0, "{reply}");
        // A test binary is not the desktop, so the desktop's own name is not granted to it.
        assert_ne!(answer["result"]["app"], "Yantrik", "{reply}");
        assert_eq!(store.list()[0].title, "Build finished");

        // The refusals are the dispatch's, with the codes an app's are given.
        let reply = call(serde_json::json!({
            "jsonrpc": "2.0", "id": 2, "method": "app.act",
            "params": { "action": "clear_history", "args": {} },
        }));
        assert_eq!(reply["error"]["code"], -32602, "{reply}");
        let reply = call(serde_json::json!({
            "jsonrpc": "2.0", "id": 3, "method": "app.act",
            "params": { "action": "dismiss", "args": { "id": 67 } },
        }));
        // An id sent as the number it spells is the id: the handler reads "67" and says there is
        // no such notification, in its own words — not a refusal about JSON.
        assert_eq!(reply["error"]["message"], "no notification with id `67`", "{reply}");
        let reply = call(serde_json::json!({
            "jsonrpc": "2.0", "id": 5, "method": "app.act",
            "params": { "action": "dismiss", "args": { "id": 6.7 } },
        }));
        assert_eq!(reply["error"]["message"], "`dismiss` argument `id` must be a string, and a number arrived", "{reply}");

        // And `describe` over the same socket reports what was stored.
        let reply = call(serde_json::json!({ "jsonrpc": "2.0", "id": 4, "method": "app.describe", "params": {} }));
        assert_eq!(reply["result"]["app"], "notifications", "{reply}");
        assert_eq!(reply["result"]["state"]["count"], 1, "{reply}");

        // Made with no socket, the same call has nobody to attribute it to.
        let direct = handler().act(&notify(None), None, at("sensitive", "ask")).unwrap();
        assert_eq!(direct["result"]["sender"]["verified"], peer_identity::UNIDENTIFIED);
    }
}
