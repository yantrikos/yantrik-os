//! Calendar service — event CRUD via filesystem-backed JSON storage.
//!
//! Events stored as individual `.json` files in `~/.local/share/yantrik/calendar/`.
//!
//! Methods:
//!   calendar.events        { start_date, end_date }        → Vec<CalendarEvent>
//!   calendar.get_event     { id }                          → CalendarEvent
//!   calendar.create_event  { title, start, end, ... }      → CalendarEvent
//!   calendar.update_event  { id, title?, start?, ... }     → CalendarEvent
//!   calendar.delete_event  { id }                          → ()
//!   calendar.upsert_remote { remote_id, title, start, ... } → CalendarEvent
//!   calendar.revision      { }                             → CalendarRevision
//!
//! Those parameter names are not written out here any more. They come from
//! `yantrik_ipc_contracts::calendar`, which the calendar app builds its requests from, because
//! this list and the app's calls used to disagree while each looked right in its own file.
//!
//! This service is the machine's one calendar. The built-in companion's calendar tools used to
//! keep their own events in SQLite, so an appointment the mind made was somewhere the Calendar
//! app would never show. They call these methods now, which is what the last three exist for:
//! a mind needs to read one event before changing it, and a sync needs a way in that does not
//! store the same Google event twice.
//!
//! `calendar.revision` is the consequence of there being one owner and several writers. An open
//! window cannot re-list a month every few seconds to find out whether somebody else wrote
//! something, and it cannot go on showing what it read when it last navigated either. So it asks
//! this instead: two numbers, a `stat` per file and no parse, and a listing only when they move.

mod store;

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use store::EventStore;
use yantrik_ipc_contracts::calendar::{
    method, CalendarEvent, CreateEventParams, DeleteEventParams, EventsParams, GetEventParams,
    UpdateEventParams, UpsertRemoteEventParams, DEFAULT_REMINDER_MINUTES, MAX_REMINDER_MINUTES,
};
#[cfg(test)]
use yantrik_service_sdk::gate::{self, Authority};
use yantrik_service_sdk::prelude::*;
use yantrik_service_sdk::{
    agent_token, caller, reach, Action, Param, PeerCred, Surface, View,
};
use yantrik_ipc_transport::peer_identity;

/// The id this surface publishes, and the app a grant for one of its actions is bound to.
const APP: &str = "calendar";

fn main() {
    std::fs::create_dir_all(calendar_dir()).ok();
    yantrik_service_sdk::init_tracing("calendar");

    // The reminder timer is not started here, and on purpose: this service is started on demand
    // (`autostart = false`) and stopped freely, so a timer in it only ran while something
    // happened to have opened the calendar. The timer lives in the notifications service, which
    // the shell autostarts, and reads the event files this service writes — see its `reminders`.

    ServiceBuilder::new("calendar")
        .handler(CalendarHandler::default())
        .run();
}

fn calendar_dir() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".local/share/yantrik/calendar")
    } else {
        PathBuf::from("/tmp/yantrik-calendar")
    }
}

struct CalendarHandler {
    /// Shared between the data methods, which read and write it, and the surface, which reports
    /// on it and moves it.
    store: Arc<EventStore>,
    /// `app.describe` and `app.act`, dispatched as an app window's are.
    surface: Surface,
}

impl Default for CalendarHandler {
    fn default() -> Self {
        let store = Arc::new(EventStore::new(calendar_dir()));
        CalendarHandler {
            store: store.clone(),
            surface: calendar_surface(store, Arc::new(BurstGate::new(ADD_BURST_LIMIT, ADD_BURST_WINDOW))),
        }
    }
}

/// Read the parameters of one method, naming the method when they do not fit.
fn params_for<T: serde::de::DeserializeOwned>(
    method_name: &str,
    params: serde_json::Value,
) -> Result<T, ServiceError> {
    serde_json::from_value(params).map_err(|e| ServiceError {
        code: -32602,
        message: format!("{method_name}: {e}"),
    })
}

impl ServiceHandler for CalendarHandler {
    fn service_id(&self) -> &str {
        "calendar"
    }

    fn handle(
        &self,
        method_name: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, ServiceError> {
        self.handle_from(method_name, params, None)
    }

    fn handle_from(
        &self,
        method_name: &str,
        params: serde_json::Value,
        peer: Option<PeerCred>,
    ) -> Result<serde_json::Value, ServiceError> {
        // The agent-facing surface: what the calendar holds, and the three moves on it that are
        // this service's to make. The ceiling and the mode are read per call from the files the
        // shell writes, as an app window's dispatch reads them.
        if let Some(answer) = self.surface.answer(method_name, &params, peer) {
            return answer;
        }
        // The raw methods are the desktop's own plumbing (#332): the Calendar window, the
        // companion and the shell's wiring call them by name, and until #43 decides what each
        // one is worth, nobody else gets them at all — `create_event` from here takes a
        // caller-written `creator`, which the graded door establishes by machine and never by
        // argument. The gate is the kernel's account of the calling process; the graded
        // actions on `app.act` above answer any caller, under the ceiling, the mode and the
        // grant, exactly as before. A method that is not one of this service's is left to the
        // unknown-method answer below, whoever asks: the protocol's `-32601` for a name nobody
        // serves is the checker's (`yos check`) and every caller's to get.
        if method_name.starts_with("calendar.") {
            if let Err(why) = own_process_only(peer) {
                return Err(ServiceError { code: -32001, message: why });
            }
        }
        match method_name {
            method::EVENTS => {
                let p: EventsParams = params_for(method_name, params)?;
                let events = self.store.list(&p)?;
                Ok(serde_json::to_value(events).unwrap())
            }
            method::GET_EVENT => {
                let p: GetEventParams = params_for(method_name, params)?;
                let event = self.store.get(&p.id).ok_or_else(|| ServiceError {
                    code: -32602,
                    message: format!("No event here with id {}", p.id),
                })?;
                Ok(serde_json::to_value(event).unwrap())
            }
            method::CREATE_EVENT => {
                let p: CreateEventParams = params_for(method_name, params)?;
                let event = self.store.create(&p)?;
                tracing::info!(id = %event.id, title = %event.title, "Created event");
                Ok(serde_json::to_value(event).unwrap())
            }
            method::UPDATE_EVENT => {
                let p: UpdateEventParams = params_for(method_name, params)?;
                let event = self.store.update(&p)?;
                tracing::info!(id = %event.id, "Updated event");
                Ok(serde_json::to_value(event).unwrap())
            }
            method::DELETE_EVENT => {
                let p: DeleteEventParams = params_for(method_name, params)?;
                self.store.delete(&p.id)?;
                tracing::info!(id = %p.id, "Deleted event");
                Ok(serde_json::json!(null))
            }
            method::UPSERT_REMOTE => {
                let p: UpsertRemoteEventParams = params_for(method_name, params)?;
                let known = self.store.find_by_remote_id(&p.remote_id).is_some();
                let event = self.store.upsert_remote(&p)?;
                tracing::info!(
                    id = %event.id,
                    remote_id = %p.remote_id,
                    known,
                    "Stored event from a remote calendar"
                );
                Ok(serde_json::to_value(event).unwrap())
            }
            method::REVISION => {
                // No parameters, and whatever arrived is ignored rather than refused: this is the
                // cheapest question on the socket and a caller that sends `{}`, `null` or nothing
                // at all should get the same answer. Nothing here writes, touches or creates, so
                // asking repeatedly cannot be what makes the answer change.
                Ok(serde_json::to_value(self.store.revision()).unwrap())
            }
            _ => Err(ServiceError {
                code: -1,
                message: format!("Unknown method: {method_name}"),
            }),
        }
    }
}

impl CalendarHandler {
    /// `app.act` under a pinned authority, as the socket's dispatch would run it. The tests' door.
    #[cfg(test)]
    fn act(
        &self,
        params: &serde_json::Value,
        authority: Authority,
    ) -> Result<serde_json::Value, ServiceError> {
        self.surface.act(params, None, authority)
    }

    /// A handler over a scratch store in its own directory. The tests' calendar.
    #[cfg(test)]
    fn over(dir: PathBuf) -> CalendarHandler {
        let store = Arc::new(EventStore::new(dir));
        CalendarHandler {
            store: store.clone(),
            surface: calendar_surface(
                store,
                Arc::new(BurstGate::new(ADD_BURST_LIMIT, ADD_BURST_WINDOW)),
            ),
        }
    }
}

// ══════════════════════════════════════════════════════════════════════
// Control surface (app.describe / app.act)
// ══════════════════════════════════════════════════════════════════════

/// The surface this socket answers `app.describe` and `app.act` with.
///
/// Three of the methods above — list, create, update — in the one store, under the rules a bare
/// method never had: arguments checked as declared, the grade enforced, the revision guarded.
/// The old comment on this socket called publishing actions "two ways into one store"; they are
/// two doors into one store, which is the arrangement weather and system-monitor already have —
/// the store owns the events, and a door that adds the protocol's checks adds no second copy.
///
/// What stays off it, on purpose: `calendar.delete_event` and the ownership rules beside it
/// (#201) belong to the Calendar app's window, where a delete that cannot be undone is graded
/// for a person to approve; `calendar.revision` and `calendar.upsert_remote` are plumbing an
/// open window and the syncer call directly — plumbing nobody should be asked to *act* on.
fn calendar_surface(store: Arc<EventStore>, burst: Arc<BurstGate>) -> Surface {
    let describing = store.clone();
    Surface::new(APP)
        .socket_name("calendar")
        .describe(move || describe_view(&describing))
        .action(list_events_action(), {
            let store = store.clone();
            move |args| list_events(&store, args)
        })
        .action(add_event_action(), {
            let store = store.clone();
            move |args| add_event(&store, &burst, args)
        })
        .action(update_event_action(), {
            let store = store.clone();
            move |args| update_event(&store, args)
        })
        .action(update_own_event_action(), move |args| update_own_event(&store, args))
}

/// What this surface can be asked to do, as `describe` publishes it.
#[cfg(test)]
fn calendar_actions() -> Vec<Action> {
    vec![
        list_events_action(),
        add_event_action(),
        update_event_action(),
        update_own_event_action(),
    ]
}

/// The grade this surface publishes for `action`, from the same table `describe` hands out, so
/// the grade a caller is shown and the grade that is enforced cannot come apart.
#[cfg(test)]
fn published_grade(action: &str) -> Option<&'static str> {
    calendar_actions().into_iter().find(|a| a.name == action).map(|a| a.permission)
}

/// The events overlapping a range — what the calendar holds.
///
/// `safe`: a read of files under the user's own home, the same listing the window draws. The
/// default is the seven days `describe` reports on, so a caller that says nothing sees what the
/// window opens on.
fn list_events_action() -> Action {
    Action::new("list_events", "List the events overlapping a date range, oldest first")
        .risk("safe")
        .arg(Param::text("start_date").describe("`YYYY-MM-DDTHH:MM:SS` or `YYYY-MM-DD`; defaults to now").optional())
        .arg(Param::text("end_date").describe("The other end of the range; defaults to seven days out").optional())
}

/// A new event, timed or all-day.
///
/// `standard` — `docs/sdk/grades.md`'s ruling on the Calendar app's own `add_event`: an event
/// can be deleted again, and nothing about it disturbs a person until a reminder fires. The
/// arguments are the store's own (`start`, `end`), not the window's date-and-time pair, because
/// this door reaches the same store the method above does.
fn add_event_action() -> Action {
    Action::new("add_event", "Put a new event on the calendar")
        .arg(Param::text("title").describe("What the event is called"))
        .arg(Param::text("start").describe("When it begins: `YYYY-MM-DDTHH:MM:SS` or `YYYY-MM-DD`"))
        .arg(Param::text("end").describe("When it ends; not before `start`"))
        .arg(Param::text("description").describe("Notes kept with the event").optional())
        .arg(Param::text("location").describe("Where it takes place").optional())
        .arg(Param::flag("all_day").describe("A day, not a time — it is not announced").optional())
        // The reminder is a fact about the event, stored with it and announced by the
        // notifications service whether or not anything ever opens the calendar again (#78).
        .arg(
            Param::integer("reminder_minutes")
                .describe("How many minutes before it starts to announce it; ten when not \
                           given. All-day events are not announced")
                .optional(),
        )
}

/// The arguments both update doors declare, written down once: the split of #332 changes who
/// may call, not what a call carries.
fn update_event_args(action: Action) -> Action {
    action
        .arg(Param::text("id").describe("The event's id, as `list_events` reports it"))
        .arg(Param::text("title").describe("A new title").optional())
        .arg(Param::text("start").describe("A new start, same formats as `add_event`").optional())
        .arg(Param::text("end").describe("A new end").optional())
        .arg(Param::text("description").describe("New notes kept with the event").optional())
        .arg(Param::text("location").describe("A new place").optional())
        .arg(Param::flag("all_day").describe("Whether the day is the appointment").optional())
        .arg(
            Param::integer("reminder_minutes")
                .describe("How many minutes before it starts to announce it; unchanged \
                           when not given")
                .optional(),
        )
}

/// Change any stored event; fields left out keep what they had.
///
/// `sensitive` (#332), by `docs/sdk/grades.md`'s rule to grade an action by the worst its
/// arguments allow: the `id` names any event in the store — the person's own, a Google-synced
/// one, another caller's — so a call here can rewrite an appointment that is not the caller's,
/// and the person sees a card. It cannot hand the event to a new owner — the stored creator
/// record stays what it was (#201) — and editing an event the caller created itself stays
/// `standard`, at `update_own_event` below: the #201 split the delete doors already have.
fn update_event_action() -> Action {
    update_event_args(
        Action::new("update_event", "Change any stored event; fields left out keep what they had")
            .risk("sensitive"),
    )
}

/// Change an event this caller created itself; fields left out keep what they had.
///
/// `standard`, like `add_event` and the Calendar window's `delete_own_event`: for the caller
/// that made the event this is the edit half of the arrangement #201 built for deletes — an
/// unattended harness can put an event on, move it and take it off again without a person
/// being asked. Every other event is refused here and stays with `update_event` above: same
/// grade, same card, exactly as it was.
fn update_own_event_action() -> Action {
    update_event_args(Action::new(
        "update_own_event",
        "Change an event this caller created itself; fields left out keep what they had. Any \
         other event is `update_event`, which asks a person first",
    ))
}

fn list_events(store: &EventStore, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let now = chrono::Local::now().naive_local();
    let params = EventsParams {
        start_date: text_arg(args, "start_date").unwrap_or_else(|| iso(&now)),
        end_date: text_arg(args, "end_date").unwrap_or_else(|| iso(&(now + chrono::Duration::days(7)))),
    };
    store
        .list(&params)
        .map(|events| serde_json::to_value(events).unwrap())
        .map_err(|e| e.message)
}

fn add_event(
    store: &EventStore,
    burst: &BurstGate,
    args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let is_all_day = args.get("all_day").and_then(|v| v.as_bool()).unwrap_or(false);
    let reminder_minutes = reminder_arg(args)?;
    // The Calendar window's own refusal, on this door too (#332): the notifications service
    // never announces an all-day event, so a reminder asked for beside `all_day` would be
    // stored and then never fire — a promise kept on file and broken in fact.
    if is_all_day && reminder_minutes.is_some() {
        return Err(
            "an all-day event is never announced; drop `reminder_minutes` or drop `all_day`"
                .to_string(),
        );
    }
    let params = CreateEventParams {
        // The dispatch has already refused anything missing or of another type; `check_arguments`
        // is why the reads below can unwrap_or_default without second-guessing the caller.
        title: text_arg(args, "title").unwrap_or_default(),
        start: text_arg(args, "start").unwrap_or_default(),
        end: text_arg(args, "end").unwrap_or_default(),
        description: text_arg(args, "description").unwrap_or_default(),
        location: text_arg(args, "location"),
        color: String::new(),
        is_all_day,
        attendees: Vec::new(),
        // Who is asking, as the machine establishes it — never from these arguments, which the
        // caller writes; see `requester` (#201).
        creator: requester(),
        reminder_minutes,
    };
    // Taken after the argument checks and right before the write: a refused call consumed
    // nothing, and every stored event announces itself to the person, so the cap counts
    // exactly the announcements this caller can make (#332).
    burst.hit(&burst_key())?;
    let event = store.create(&params).map_err(|e| e.message)?;
    tracing::info!(id = %event.id, title = %event.title, "Created event");
    serde_json::to_value(event).map_err(|e| e.to_string())
}

fn update_event(store: &EventStore, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let params = update_params(args)?;
    apply_update(store, params)
}

/// The `standard` update door: an event this very caller created, and nothing else (#332).
///
/// The record is read back from the event's own file, and the caller is what `requester`
/// establishes just now — the same two machine-established strings the #201 delete rule
/// compares, never anything the request claims. Every other event is refused in a sentence
/// that points at `update_event`, where the person sees the card.
fn update_own_event(
    store: &EventStore,
    args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let params = update_params(args)?;
    if let Some(stored) = store.get(&params.id) {
        let me = requester();
        if !created_by_the_caller(stored.creator.as_deref(), me.as_deref()) {
            return Err(not_the_makers(&stored, me));
        }
    }
    // No stored event to compare against — an id that is not here, or one that is a path —
    // goes to the store, whose refusal is the one sentence both update doors share.
    apply_update(store, params)
}

/// The update both doors declared, as the store's own parameters.
fn update_params(args: &serde_json::Value) -> Result<UpdateEventParams, String> {
    Ok(UpdateEventParams {
        id: text_arg(args, "id").unwrap_or_default(),
        title: text_arg(args, "title"),
        start: text_arg(args, "start"),
        end: text_arg(args, "end"),
        description: text_arg(args, "description"),
        location: text_arg(args, "location"),
        is_all_day: args.get("all_day").and_then(|v| v.as_bool()),
        attendees: None,
        remote_id: None,
        reminder_minutes: reminder_arg(args)?,
    })
}

fn apply_update(
    store: &EventStore,
    params: UpdateEventParams,
) -> Result<serde_json::Value, String> {
    let event = store.update(&params).map_err(|e| e.message)?;
    tracing::info!(id = %event.id, "Updated event");
    serde_json::to_value(event).map_err(|e| e.to_string())
}

/// The #201 rule, on this side of the wire: the caller that created an event may change it
/// without a person being asked. One comparison of two strings the machine established — the
/// record the store kept at creation and the identity `requester` verified just now. The
/// Calendar window's `ownership` module carries the same rule for its own door; it lives in a
/// binary crate, so the rule is written twice rather than moved into a library both binaries
/// would have to depend on (#320's `valid_id` is the precedent).
fn created_by_the_caller(creator: Option<&str>, caller: Option<&str>) -> bool {
    match (creator, caller) {
        (Some(creator), Some(caller)) => !creator.trim().is_empty() && creator == caller,
        _ => false,
    }
}

/// Why `update_own_event` refused, in the shape the window's `delete_own_event` refusal has:
/// both identities named as the machine established them, and the door that does change
/// anybody's event.
fn not_the_makers(stored: &CalendarEvent, me: Option<String>) -> String {
    let who = me.unwrap_or_else(|| "nobody this machine could identify".to_string());
    let why = match stored.creator.as_deref() {
        Some(made_by) => format!(
            "“{}” was created by {made_by} and this call is {who}: only the caller that \
             created an event may change it without a person being asked",
            stored.title
        ),
        None => format!(
            "“{}” has no creator on record — it is older than the record, a person made it \
             in the window, or a sync stored it — and this call is {who}: only the caller \
             that created an event may change it without a person being asked",
            stored.title
        ),
    };
    format!("{why}. Any event changes through `update_event`, which asks first")
}

/// A declared text argument, present or not. The dispatch guarantees the type.
fn text_arg(args: &serde_json::Value, name: &str) -> Option<String> {
    args.get(name).and_then(|v| v.as_str()).map(str::to_string)
}

/// The `reminder_minutes` argument as this door reads it — the same reading, and the same
/// sentences, as the Calendar window's (#78): absent is "did not say", and anything present
/// must be a number of minutes inside the contract's cap. Checked here rather than left to the
/// store, whose refusal names only the cap: the error a caller can act on names the whole
/// range it should have stayed inside.
fn reminder_arg(args: &serde_json::Value) -> Result<Option<u32>, String> {
    match args.get("reminder_minutes") {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(v) => {
            let m = v.as_i64().ok_or("`reminder_minutes` must be a number of minutes")?;
            if !(0..=i64::from(MAX_REMINDER_MINUTES)).contains(&m) {
                return Err(format!(
                    "`reminder_minutes` must be between 0 and {MAX_REMINDER_MINUTES}, and was {m}"
                ));
            }
            Ok(Some(m as u32))
        }
    }
}

fn iso(dt: &chrono::NaiveDateTime) -> String {
    dt.format("%Y-%m-%dT%H:%M:%S").to_string()
}

/// Who the machine establishes is asking, in the one spelling every surface agrees on (#201):
/// `agent <mind>:<conversation>` where the call carried an agent token whose reach the shell
/// knows, else the program the kernel's peer credentials lead to, else nothing.
///
/// The Calendar app's window records creators the same way, from the same two sources, so an
/// event made through either door carries the record `delete_own_event` reads back. A token the
/// reach file does not know is no agent — that falls through to the program rather than
/// refusing twice — and nothing is invented when the machine cannot tell: an event with no
/// creator is one nobody may delete unasked.
fn requester() -> Option<String> {
    if let Some(token) = agent_token() {
        if let Ok(Some(reach)) = reach::reach_of(&token) {
            return Some(format!("agent {}", reach.agent));
        }
    }
    let who = caller()?;
    let name = peer_identity::resolve(Some(who.pid)).name();
    if name.is_empty() { None } else { Some(name) }
}

// ── The burst cap on `add_event` (#332) ─────────────────────────────
//
// Every stored event announces itself to the person through the notifications service, so a
// loop of `add_event` is a loop of notifications: ten in a minute is far more than a person
// driving a calendar and far less than a runaway caller. The cap is per caller, not per
// socket, so one flooded agent does not lock out the window beside it.

/// How many `add_event` calls one caller may make inside [`ADD_BURST_WINDOW`].
const ADD_BURST_LIMIT: usize = 10;
/// The window the cap counts over.
const ADD_BURST_WINDOW: Duration = Duration::from_secs(60);

/// Recent `add_event` calls per caller, oldest first.
struct BurstGate {
    limit: usize,
    window: Duration,
    hits: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl BurstGate {
    fn new(limit: usize, window: Duration) -> Self {
        BurstGate { limit, window, hits: Mutex::new(HashMap::new()) }
    }

    /// Take an add slot for `who` at `now`: `Ok` while the caller is under the cap, `Err` with
    /// the wait when it is not. Checked and recorded in one lock, so a caller racing itself
    /// cannot take two slots at once.
    fn hit_at(&self, who: &str, now: Instant) -> Result<(), String> {
        let mut hits = self.hits.lock().unwrap_or_else(|e| e.into_inner());
        let queue = hits.entry(who.to_string()).or_default();
        while let Some(front) = queue.front() {
            if now.saturating_duration_since(*front) > self.window {
                queue.pop_front();
            } else {
                break;
            }
        }
        if queue.len() >= self.limit {
            let retry = queue
                .front()
                .map(|oldest| {
                    self.window.saturating_sub(now.saturating_duration_since(*oldest)).as_secs() + 1
                })
                .unwrap_or(1);
            return Err(format!(
                "this caller added {limit} events in the last {window} seconds and every \
                 stored event announces itself to the person: wait {retry} seconds and add \
                 this one again",
                limit = self.limit,
                window = self.window.as_secs(),
            ));
        }
        queue.push_back(now);
        Ok(())
    }

    fn hit(&self, who: &str) -> Result<(), String> {
        self.hit_at(who, Instant::now())
    }
}

/// Whose column of the burst cap this call writes to: the agent a token the reach file knows
/// belongs to, else the user the kernel's peer credentials report, else one shared column — an
/// in-process call has neither, and a caller the machine cannot place is exactly the one that
/// should share a cap rather than get a column of its own.
///
/// Never the token itself. The caller writes the token, so a column per token was a column per
/// call for anyone who wrote a new one each time — eleven adds under eleven made-up tokens went
/// straight past the cap. The same belief `requester()` uses decides the column here.
fn burst_key() -> String {
    if let Some(token) = agent_token() {
        if let Ok(Some(reach)) = reach::reach_of(&token) {
            return format!("agent {}", reach.agent);
        }
    }
    match caller() {
        Some(who) => format!("uid {}", who.uid),
        None => "unidentified".to_string(),
    }
}

// ── The raw methods answer the desktop's own programs (#332) ────────

/// Whether `/proc` says this executable is one of this OS's own binaries: `yantrik` itself —
/// the CLI whose `ask` and `serve` run the companion, calendar and network tools included — and
/// every `yantrik-*` program beside it. A binary replaced mid-run reads as
/// `/path/yantrik-x (deleted)`, which still passes — right for a service restarted while a
/// caller holds the socket. A path that is not absolute is not the kernel's answer and is
/// refused like anything else.
fn is_own_binary(exe: &str) -> bool {
    let Some(base) = exe.strip_prefix('/').and_then(|path| path.rsplit('/').next()) else {
        return false;
    };
    let base = base.strip_suffix(" (deleted)").unwrap_or(base);
    base == "yantrik" || base.starts_with("yantrik-")
}

/// The peer check on the raw methods, and the refusals in sentences.
///
/// Same-uid limits stand (#154): this stops accidents and casual impersonation — a script,
/// another user's process, a caller that never says who it is — not hostile code running as
/// the same user, which can be the shell's own child and wear its name.
fn own_process_only(peer: Option<PeerCred>) -> Result<(), String> {
    let Some(peer) = peer else {
        return Err(
            "the kernel would not say which process is calling, and the raw calendar.* \
             methods answer the desktop's own programs; the graded actions on app.act \
             answer any caller"
                .to_string(),
        );
    };
    let exe = std::fs::read_link(format!("/proc/{}/exe", peer.pid))
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    if is_own_binary(&exe) {
        return Ok(());
    }
    Err(if exe.is_empty() {
        format!(
            "the process calling the raw calendar.* methods (pid {}) could not be identified \
             from /proc, and those methods answer the desktop's own programs; the graded \
             actions on app.act answer any caller",
            peer.pid
        )
    } else {
        format!(
            "the raw calendar.* methods answer the desktop's own programs and the process \
             calling is {exe} (pid {}); the graded actions on app.act answer any caller",
            peer.pid
        )
    })
}

/// What this calendar holds, and how an event gets announced.
///
/// The reminder note is the point of this view: the timer does not run here — this service
/// is on demand, so a timer in it would only tick while something had opened the calendar.
/// It runs in the notifications service, which is always up, and a caller reading this is
/// told where reminders live and what one is worth rather than left to assume the wrong
/// thing.
fn describe_view(store: &EventStore) -> View {
    let revision = store.revision();
    let now = chrono::Local::now().naive_local();
    let upcoming = store
        .list(&EventsParams {
            start_date: iso(&now),
            end_date: iso(&(now + chrono::Duration::days(7))),
        })
        .unwrap_or_default();

    let next = upcoming.first();
    let summary = match next {
        Some(e) => format!(
            "Calendar — {} events stored, next: {} at {}",
            revision.events, e.title, e.start
        ),
        None => format!(
            "Calendar — {} events stored, nothing in the next seven days",
            revision.events
        ),
    };

    View::new(summary)
        .with("events", revision.events as i64)
        .with("store", store.dir().to_string_lossy().to_string())
        .with(
            "upcoming",
            serde_json::Value::Array(
                upcoming
                    .iter()
                    .take(10)
                    .map(|e| {
                        serde_json::json!({
                            "id": e.id,
                            "title": e.title,
                            "start": e.start,
                            "end": e.end,
                            "all_day": e.is_all_day,
                        })
                    })
                    .collect(),
            ),
        )
        .with(
            "reminders",
            serde_json::json!({
                "default_minutes": DEFAULT_REMINDER_MINUTES,
                "hosted_by": "notifications",
                "note": "Every timed event is announced through the notifications \
                         service, once, its own `reminder_minutes` before it starts — ten \
                         when the event does not say. The timer runs in the notifications \
                         service, which the shell keeps up from boot, so a reminder set for \
                         tomorrow fires whether or not anything ever opens the calendar. \
                         All-day events are not announced: there is no time of day to \
                         announce them at.",
            }),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    /// A handler over a calendar of its own: a fresh directory under /tmp, never `$HOME`, so
    /// the tests neither touch the developer's events nor depend on what is in them.
    fn scratch() -> (CalendarHandler, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "yos-calendar-surface-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        (CalendarHandler::over(dir.clone()), dir)
    }

    /// A machine at `ceiling`, in `mode`, with no grant spent — the authority pinned per case
    /// rather than inherited from whatever files the machine running the tests has.
    fn at(ceiling: &str, mode: &str) -> Authority {
        Authority { ceiling: ceiling.into(), mode: gate::Mode::named(mode), granted: false }
    }

    /// `app.act` for `action` with `args`, and a grant id beside it when the test says one was
    /// spent for the call.
    fn act_params(action: &str, args: serde_json::Value, grant: Option<&str>) -> serde_json::Value {
        let mut params = serde_json::json!({ "action": action, "args": args });
        if let Some(grant) = grant {
            params["grant"] = grant.into();
        }
        params
    }

    /// A point in the store's own date format, `days` from now. Days, not minutes: nothing in
    /// these tests may depend on how fast the machine is.
    fn days_from_now(days: i64) -> String {
        iso(&(chrono::Local::now().naive_local() + chrono::Duration::days(days)))
    }

    /// Grants the stand-in shell spent. `ok-*` holds, anything else is refused in its words.
    static SPENT: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    /// What each grant was spent against, as the shell would have been handed it.
    static SPENT_AGAINST: std::sync::Mutex<Vec<(String, String)>> = std::sync::Mutex::new(Vec::new());

    fn spend_through_a_stand_in_shell() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
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

    /// Add an event a few days out, the way an ask-mode caller would, and hand back the whole
    /// envelope: its `result` is the event as stored, with the id the store gave it.
    fn an_event(handler: &CalendarHandler, title: &str) -> serde_json::Value {
        handler
            .act(
                &act_params(
                    "add_event",
                    serde_json::json!({ "title": title, "start": days_from_now(2), "end": days_from_now(3) }),
                    None,
                ),
                at("sensitive", "ask"),
            )
            .expect("add_event is standard: it runs unasked in ask mode")
    }

    /// `add_event` and `update_own_event` are `standard`, `update_event` is `sensitive` — any
    /// event in the store, including the person's own, is what its `id` argument allows
    /// (#332) — and `list_events` is `safe`: the grades `docs/sdk/grades.md` rules, on the
    /// same store as the Calendar app's own actions. And `describe` shows exactly what `act`
    /// enforces.
    #[test]
    fn the_published_grades_are_what_describe_shows() {
        assert_eq!(published_grade("list_events"), Some("safe"));
        assert_eq!(published_grade("add_event"), Some("standard"));
        assert_eq!(published_grade("update_event"), Some("sensitive"));
        assert_eq!(published_grade("update_own_event"), Some("standard"));

        let (handler, dir) = scratch();
        let described = handler.handle("app.describe", serde_json::json!({})).expect("describe");
        let actions = described["actions"].as_array().expect("actions");
        assert_eq!(actions.len(), calendar_actions().len());
        for a in actions {
            let name = a["name"].as_str().unwrap();
            assert_eq!(a["permission"].as_str(), published_grade(name), "{name}");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The whole point of #179, end to end inside the store: an event added through `app.act`
    /// is on the calendar — listed by `list_events`, counted by the next `describe` — and the
    /// act answers with the state after it, so nobody has to ask twice. An in-process act has
    /// no caller to record, and inventing one is not on (#201).
    #[test]
    fn an_event_added_through_the_surface_is_on_the_calendar() {
        let (handler, dir) = scratch();
        let answer = an_event(&handler, "Standup");
        assert_eq!(answer["accepted"], true);
        assert_eq!(answer["settled"], true);
        assert_eq!(answer["result"]["title"], "Standup");
        assert!(
            answer["result"].get("creator").map(|c| c.is_null()).unwrap_or(true),
            "no caller, no creator record: {}",
            answer["result"]
        );
        assert!(answer["summary"].as_str().unwrap().contains("1 events stored"), "{answer}");

        let listed = handler
            .act(&act_params("list_events", serde_json::json!({}), None), at("sensitive", "ask"))
            .expect("a safe read needs nothing");
        assert_eq!(listed["result"].as_array().unwrap().len(), 1);
        assert_eq!(listed["result"][0]["title"], "Standup");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The reminder lead #78 added travels through this door as through the window's: a stated
    /// lead is stored on the event, an unstated one keeps the contract's default, and a lead
    /// outside the cap is refused at the door in a sentence that names the range — before
    /// anything is stored.
    #[test]
    fn a_reminder_lead_travels_with_the_event() {
        let (handler, dir) = scratch();
        let said = handler
            .act(
                &act_params(
                    "add_event",
                    serde_json::json!({
                        "title": "Standup",
                        "start": days_from_now(2),
                        "end": days_from_now(3),
                        "reminder_minutes": 30,
                    }),
                    None,
                ),
                at("sensitive", "ask"),
            )
            .expect("a stated lead is stored");
        assert_eq!(said["result"]["reminder_minutes"], 30);

        let unstated = an_event(&handler, "Unsaid");
        assert_eq!(
            unstated["result"]["reminder_minutes"],
            DEFAULT_REMINDER_MINUTES,
            "an unstated lead keeps the default the contract promises"
        );

        for bad in [-5, i64::from(MAX_REMINDER_MINUTES) + 1] {
            let err = handler
                .act(
                    &act_params(
                        "add_event",
                        serde_json::json!({
                            "title": "Nope",
                            "start": days_from_now(2),
                            "end": days_from_now(3),
                            "reminder_minutes": bad,
                        }),
                        None,
                    ),
                    at("sensitive", "ask"),
                )
                .unwrap_err();
            assert_eq!(
                err.message,
                format!("`reminder_minutes` must be between 0 and {MAX_REMINDER_MINUTES}, and was {bad}")
            );
        }
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2, "a refused lead stored nothing");

        // An update moves the lead, and one that does not say keeps what is stored. In auto
        // mode, which runs a sensitive action unasked — `update_event` reached every stored
        // event, so #332 graded it for a person to approve.
        let id = said["result"]["id"].clone();
        let moved = handler
            .act(
                &act_params("update_event", serde_json::json!({ "id": id, "reminder_minutes": 5 }), None),
                at("sensitive", "auto"),
            )
            .expect("auto mode runs the sensitive edit unasked");
        assert_eq!(moved["result"]["reminder_minutes"], 5);
        let kept = handler
            .act(
                &act_params("update_event", serde_json::json!({ "id": id, "title": "Renamed" }), None),
                at("sensitive", "auto"),
            )
            .expect("auto mode runs the sensitive edit unasked");
        assert_eq!(kept["result"]["reminder_minutes"], 5, "an unstated lead keeps what it had");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The default listing is the week `describe` reports on: an event two days out is in it
    /// whether or not the caller says anything, and a stated range that misses it comes back
    /// empty — the store's overlap rule, applied as declared.
    #[test]
    fn list_events_answers_a_range_and_otherwise_shows_the_week() {
        let (handler, dir) = scratch();
        an_event(&handler, "Standup");
        let week = handler
            .act(&act_params("list_events", serde_json::json!({}), None), at("safe", "plan"))
            .unwrap();
        assert_eq!(week["result"].as_array().unwrap().len(), 1);
        let far = handler
            .act(
                &act_params(
                    "list_events",
                    serde_json::json!({ "start_date": days_from_now(10), "end_date": days_from_now(20) }),
                    None,
                ),
                at("safe", "plan"),
            )
            .unwrap();
        assert!(far["result"].as_array().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// `standard` needs no grant in any mode, plan included. Each mode gets its own calendar,
    /// and the proof is the same in all four: the event lands.
    #[test]
    fn add_event_reaches_its_handler_in_every_mode_without_a_grant() {
        for mode in ["plan", "ask", "auto", "bypass"] {
            let (handler, dir) = scratch();
            let answer = handler
                .act(
                    &act_params(
                        "add_event",
                        serde_json::json!({ "title": "Standup", "start": days_from_now(2), "end": days_from_now(3) }),
                        None,
                    ),
                    at("sensitive", mode),
                )
                .unwrap_or_else(|e| panic!("{mode}: {}", e.message));
            assert_eq!(answer["accepted"], true, "{mode}");
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    /// The ceiling binds this door as it binds every app's: a machine set to `safe` refuses
    /// `add_event` on the grade alone, grant or none — before anything is written, and a grant
    /// it refused was never offered to the shell (#154).
    #[test]
    fn a_ceiling_of_safe_refuses_add_event_whatever_the_grant() {
        spend_through_a_stand_in_shell();
        for grant in [None, Some("ok-179-calendar")] {
            let (handler, dir) = scratch();
            let err = handler
                .act(
                    &act_params(
                        "add_event",
                        serde_json::json!({ "title": "Standup", "start": days_from_now(2), "end": days_from_now(3) }),
                        grant,
                    ),
                    at("safe", "bypass"),
                )
                .unwrap_err();
            assert!(
                err.message.starts_with("CEILING: calendar.add_event is graded `standard`"),
                "grant={grant:?}: {}",
                err.message
            );
            assert_eq!(err.code, -32602);
            assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "grant={grant:?}: an event was stored anyway");
            let _ = std::fs::remove_dir_all(dir);
        }
        let spent = SPENT.lock().unwrap_or_else(|e| e.into_inner());
        assert!(!spent.iter().any(|id| id == "ok-179-calendar"), "spent above the ceiling: {spent:?}");
    }

    /// As on a window: a grant that rides on a call is spent past the ceiling, and one the
    /// shell refuses ends the call in the shell's words — before anything is stored.
    #[test]
    fn a_grant_that_does_not_hold_ends_the_call() {
        spend_through_a_stand_in_shell();
        let (handler, dir) = scratch();
        let err = handler
            .act(
                &act_params(
                    "add_event",
                    serde_json::json!({ "title": "Standup", "start": days_from_now(2), "end": days_from_now(3) }),
                    Some("made-up"),
                ),
                at("sensitive", "ask"),
            )
            .unwrap_err();
        assert!(
            err.message.starts_with("GRANT: `made-up` does not authorise calendar.add_event"),
            "{}",
            err.message
        );
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// An agent token travels beside `args`, never among them: one a caller put among them is
    /// taken out before the grant is spent, so the shell is handed the arguments alone. The
    /// stale revision is here to stop the call at the guard, after the spend, without touching
    /// the store.
    #[test]
    fn an_agent_token_among_the_arguments_is_not_what_a_grant_is_bound_to() {
        spend_through_a_stand_in_shell();
        let (handler, dir) = scratch();
        let params = serde_json::json!({
            "action": "list_events",
            "args": { "agent_token": "smuggled" },
            "agent_token": "tok-7f3a",
            "grant": "ok-179-token",
            "expect_revision": "0000000000000000",
        });
        let err = handler.act(&params, at("sensitive", "ask")).unwrap_err();
        assert!(err.message.starts_with("STALE: this app is at revision "), "{}", err.message);
        let against = SPENT_AGAINST.lock().unwrap_or_else(|e| e.into_inner());
        let (_, args) = against.iter().find(|(id, _)| id == "ok-179-token").expect("the grant was spent");
        assert_eq!(args, "{}");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// `update_event` edits the stored event and nothing else: fields left out keep what they
    /// had, and the creator record survives (#201) — an update cannot hand somebody else's
    /// event to a new owner, which is what a later delete-unasked would key off. An id that is
    /// not here is refused in the store's words. In auto mode throughout: the action is
    /// `sensitive` since #332, and auto is the mode that runs a sensitive action unasked.
    #[test]
    fn update_event_edits_a_stored_event_and_keeps_its_owner() {
        let (handler, dir) = scratch();
        let added = an_event(&handler, "Standup");
        let (id, started) = (added["result"]["id"].clone(), added["result"]["start"].clone());
        let updated = handler
            .act(
                &act_params("update_event", serde_json::json!({ "id": id, "title": "Renamed" }), None),
                at("sensitive", "auto"),
            )
            .expect("auto mode runs the sensitive edit unasked");
        assert_eq!(updated["result"]["title"], "Renamed");
        assert_eq!(updated["result"]["start"], started, "a field left out kept what it had");

        // An event stored with an owner keeps it through a surface edit.
        handler
            .store
            .create(&CreateEventParams {
                title: "Somebody else's".into(),
                start: days_from_now(4),
                end: days_from_now(5),
                description: String::new(),
                location: None,
                color: String::new(),
                is_all_day: false,
                attendees: Vec::new(),
                creator: Some("agent pi:owned".into()),
                reminder_minutes: None,
            })
            .expect("seeded");
        let listed = handler
            .act(&act_params("list_events", serde_json::json!({}), None), at("sensitive", "ask"))
            .unwrap();
        let owned = listed["result"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["creator"] == serde_json::json!("agent pi:owned"))
            .expect("the seeded creator record");
        handler
            .act(
                &act_params("update_event", serde_json::json!({ "id": owned["id"], "location": "Room 2" }), None),
                at("sensitive", "auto"),
            )
            .expect("auto mode runs the sensitive edit unasked");
        let listed = handler
            .act(&act_params("list_events", serde_json::json!({}), None), at("sensitive", "ask"))
            .unwrap();
        assert_eq!(
            listed["result"].as_array().unwrap().iter().find(|e| e["id"] == owned["id"]).unwrap()["creator"],
            "agent pi:owned",
            "an update re-assigned the event"
        );

        let err = handler
            .act(
                &act_params("update_event", serde_json::json!({ "id": "no-such-id" }), None),
                at("sensitive", "auto"),
            )
            .unwrap_err();
        assert_eq!(err.code, -32602);
        assert_eq!(err.message, "No event here with id no-such-id");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// An id is a name inside the store's own folder, never a path. `update_event` reads with
    /// the argument's id and writes to a path built from the id stored inside the file it read,
    /// so a crafted id reached any `.json` the person owns — the bug notes-service had and
    /// fixed (#161, #320), back on the headless door #179 opened. Both the argument's id and
    /// the id read back from a file are refused before the filesystem is touched, and an id
    /// the service itself handed out still round-trips.
    #[test]
    fn a_crafted_id_cannot_reach_past_the_calendar_dir() {
        let (handler, dir) = scratch();
        // A sentinel outside the store's folder, planted as an event whose own id points back
        // out: pre-fix, an update read it and then wrote over it through that stored id. Named
        // after this test's own scratch dir, which no other test uses.
        let outside =
            dir.with_file_name(format!("{}-outside.json", dir.file_name().unwrap().to_string_lossy()));
        let traversal = format!("../{}", outside.file_stem().unwrap().to_string_lossy());
        let planted = serde_json::json!({
            "id": traversal,
            "title": "Sentinel",
            "description": "",
            "start": days_from_now(2),
            "end": days_from_now(3),
            "is_all_day": false,
            "location": null,
            "attendees": [],
            "recurrence": null,
            "calendar_id": "default",
            "remote_id": null,
        });
        std::fs::write(&outside, serde_json::to_string_pretty(&planted).unwrap()).unwrap();
        let before = std::fs::read_to_string(&outside).unwrap();

        for id in [traversal.as_str(), "/tmp/x"] {
            let err = handler
                .act(
                    &act_params("update_event", serde_json::json!({ "id": id, "title": "Taken" }), None),
                    at("sensitive", "auto"),
                )
                .unwrap_err();
            assert_eq!(err.code, -32602);
            assert_eq!(
                err.message,
                format!("`{id}` is not an event id; ids are the names list_events reports, never paths"),
                "a path is refused as the path it is, not reported missing"
            );
        }
        assert_eq!(std::fs::read_to_string(&outside).unwrap(), before, "the file outside survived");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "a refused update stored nothing");

        // The doors the surface does not publish hold the same rule at the store itself: the
        // raw method's read answers "not here" without opening anything, and its delete
        // refuses rather than removing what is outside.
        assert!(handler.store.get(&traversal).is_none());
        let err = handler.store.delete(&traversal).unwrap_err();
        assert_eq!(err.code, -32602);
        assert_eq!(std::fs::read_to_string(&outside).unwrap(), before, "a refused delete removed nothing");

        // And an id the service handed out still round-trips add → update → list.
        let added = an_event(&handler, "Real");
        let id = added["result"]["id"].clone();
        handler
            .act(
                &act_params("update_event", serde_json::json!({ "id": id, "title": "Renamed" }), None),
                at("sensitive", "auto"),
            )
            .expect("auto mode runs the sensitive edit unasked");
        let listed = handler
            .act(&act_params("list_events", serde_json::json!({}), None), at("sensitive", "ask"))
            .expect("a safe read");
        let events = listed["result"].as_array().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["title"], "Renamed");

        let _ = std::fs::remove_file(&outside);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Every action `describe` offers reaches a handler past the gate, and an action it does
    /// not offer — including the delete this surface deliberately does not publish — is
    /// answered as that before any grant is looked at: -32602, in the dispatch's words.
    #[test]
    fn every_published_action_has_a_handler() {
        let (handler, dir) = scratch();
        for spec in calendar_actions() {
            let outcome = handler.act(
                &act_params(&spec.name, serde_json::json!({}), None),
                at("dangerous", "bypass"),
            );
            if let Err(err) = outcome {
                // `list_events` answers; the two that need arguments refuse for them. Neither
                // answer may be the gate's or the dispatch's "no such action".
                for prefix in ["CEILING:", "GRANT:", "STALE:", "unknown action"] {
                    assert!(!err.message.starts_with(prefix), "{}: {}", spec.name, err.message);
                }
            }
        }
        let err = handler
            .act(
                &act_params("delete_event", serde_json::json!({ "id": "x" }), Some("made-up")),
                at("dangerous", "ask"),
            )
            .unwrap_err();
        assert_eq!(err.code, -32602);
        assert_eq!(
            err.message,
            "unknown action `delete_event`; this app offers: list_events, add_event, update_event, update_own_event"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Arguments are checked on this door as on every app's — and a caller cannot declare its
    /// own creator: `creator` is not an argument `add_event` takes, so the #201 record is only
    /// ever what the machine establishes.
    #[test]
    fn the_arguments_are_checked_as_an_apps_are() {
        let (handler, dir) = scratch();
        // A number converts to a string without loss — `title: 5` becomes "5", by the SDK's own
        // rule for models. A boolean cannot be a title: the type refusal names it.
        let err = handler
            .act(
                &act_params("add_event", serde_json::json!({ "title": true, "start": days_from_now(2), "end": days_from_now(3) }), None),
                at("sensitive", "ask"),
            )
            .unwrap_err();
        assert_eq!(err.code, -32602);
        assert_eq!(err.message, "`add_event` argument `title` must be a string, and a boolean arrived");

        // A complete, otherwise-valid add that also tries to name its own creator: refused for
        // the undeclared name — the record is only ever what the machine establishes (#201).
        let err = handler
            .act(
                &act_params(
                    "add_event",
                    serde_json::json!({
                        "title": "x",
                        "start": days_from_now(2),
                        "end": days_from_now(3),
                        "creator": "me",
                    }),
                    None,
                ),
                at("sensitive", "ask"),
            )
            .unwrap_err();
        assert_eq!(
            err.message,
            "`add_event` has no argument `creator`; it takes: title, start, end, description, \
             location, all_day, reminder_minutes"
        );

        let err = handler
            .act(&act_params("add_event", serde_json::json!({}), None), at("sensitive", "ask"))
            .unwrap_err();
        assert_eq!(err.message, "`add_event` needs argument `title`");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Stale is refused before the store is written — and the store keeps nothing: an act
    /// decided on a state this calendar has left does not happen.
    #[test]
    fn an_act_decided_on_an_old_revision_stores_nothing() {
        let (handler, dir) = scratch();
        let params = serde_json::json!({
            "action": "add_event",
            "args": { "title": "Standup", "start": days_from_now(2), "end": days_from_now(3) },
            "expect_revision": "0000000000000000",
        });
        let err = handler.act(&params, at("sensitive", "ask")).unwrap_err();
        assert!(err.message.starts_with("STALE: this app is at revision "), "{}", err.message);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The surface declares nothing the dispatch cannot check.
    #[test]
    fn the_surface_is_declared_soundly() {
        let (handler, dir) = scratch();
        assert!(handler.surface.registry().problems().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// `app.act` for `action` carrying an agent token beside the args, as a mind's call does.
    fn act_as(action: &str, args: serde_json::Value, token: &str) -> serde_json::Value {
        serde_json::json!({ "action": action, "args": args, "agent_token": token })
    }

    /// Two agents the shell is standing in for, each with a token in the reach file: one to
    /// create events and one to try at them. The reader is installed once for the process, as
    /// the shell's own is; a token it does not know — every other test's — reads as no reach,
    /// exactly as the missing file did before.
    fn stand_in_reach_file() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            reach::read_reach_with(|token| match token {
                "tok-owner" => Some(reach::Reach {
                    agent: "pi:c-owner".into(),
                    role: "coder".into(),
                    name: "Coder".into(),
                    surfaces: vec!["calendar".into()],
                    ceiling: "sensitive".into(),
                }),
                "tok-stranger" => Some(reach::Reach {
                    agent: "deepseek:c-stranger".into(),
                    role: "reviewer".into(),
                    name: "Reviewer".into(),
                    surfaces: vec!["calendar".into()],
                    ceiling: "sensitive".into(),
                }),
                _ => None,
            });
        });
    }

    /// The ownership rule #332 put on both update doors, end to end over the socket's dispatch,
    /// and the `requester()` case item 5 of the issue asked to have pinned: a token the reach
    /// file believes. An event added under a token records the agent the token belongs to —
    /// never anything from the arguments — the agent that made it may move it at `standard`,
    /// and any other caller gets the refusal naming both identities and pointing at
    /// `update_event`, where a person sees the card. That card is the last assertion: the
    /// stranger's `update_event` in ask mode stops at the gate's `GRANT:` refusal.
    #[test]
    fn only_the_maker_moves_an_event_unasked_and_the_reach_file_is_what_names_the_maker() {
        stand_in_reach_file();
        let (handler, dir) = scratch();

        // A token in the reach file is believed: the event records the agent it belongs to.
        let added = handler
            .act(
                &act_as(
                    "add_event",
                    serde_json::json!({ "title": "Owner's", "start": days_from_now(2), "end": days_from_now(3) }),
                    "tok-owner",
                ),
                at("sensitive", "ask"),
            )
            .expect("add_event is standard");
        let id = added["result"]["id"].clone();
        assert_eq!(added["result"]["creator"], "agent pi:c-owner", "the creator is the token's agent");

        // The maker moves its own event, unasked, at `standard`.
        let moved = handler
            .act(
                &act_as("update_own_event", serde_json::json!({ "id": id, "title": "Moved" }), "tok-owner"),
                at("sensitive", "ask"),
            )
            .expect("the maker's own event needs no card");
        assert_eq!(moved["result"]["title"], "Moved");
        assert_eq!(moved["result"]["creator"], "agent pi:c-owner", "an update re-assigned the event");

        // Another agent — identified by its own token, not by anything it says — is refused in
        // a sentence naming both identities and the door that does ask.
        let err = handler
            .act(
                &act_as("update_own_event", serde_json::json!({ "id": id, "title": "Taken" }), "tok-stranger"),
                at("sensitive", "ask"),
            )
            .unwrap_err();
        assert_eq!(err.code, -32602);
        assert!(err.message.contains("agent pi:c-owner"), "{}", err.message);
        assert!(err.message.contains("agent deepseek:c-stranger"), "{}", err.message);
        assert!(err.message.ends_with("Any event changes through `update_event`, which asks first"), "{}", err.message);
        let listed = handler
            .act(&act_params("list_events", serde_json::json!({}), None), at("safe", "plan"))
            .unwrap();
        assert_eq!(listed["result"].as_array().unwrap()[0]["title"], "Moved", "a refused update changed nothing");

        // And that door asks: `update_event` is `sensitive`, so the stranger's call stops at
        // the gate with the card's sentence, in a mode that raises cards.
        let err = handler
            .act(
                &act_as("update_event", serde_json::json!({ "id": id, "title": "Taken" }), "tok-stranger"),
                at("sensitive", "ask"),
            )
            .unwrap_err();
        assert!(
            err.message.starts_with("GRANT: calendar.update_event is graded `sensitive` and this machine is in ask mode"),
            "{}",
            err.message
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// An event with no creator on record — older than the record, made by a person in the
    /// window, or stored by a sync — is nobody's to move unasked: `update_own_event` refuses it
    /// and says why, and the refusal still points at the door a person decides on.
    #[test]
    fn an_event_with_no_creator_on_record_is_nobodys_to_move_unasked() {
        let (handler, dir) = scratch();
        let added = an_event(&handler, "Orphan");
        let id = added["result"]["id"].clone();
        assert!(added["result"]["creator"].is_null(), "an in-process act has no caller to record");
        let err = handler
            .act(
                &act_params("update_own_event", serde_json::json!({ "id": id, "title": "Taken" }), None),
                at("sensitive", "ask"),
            )
            .unwrap_err();
        assert!(err.message.contains("has no creator on record"), "{}", err.message);
        assert!(err.message.ends_with("Any event changes through `update_event`, which asks first"), "{}", err.message);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// An id that is not here is refused in the store's words on the own-event door too, and a
    /// crafted id never reaches the ownership comparison: both fall through to the one refusal
    /// the store already pins (#320).
    #[test]
    fn update_own_event_refuses_a_missing_or_crafted_id_as_the_store_does() {
        let (handler, dir) = scratch();
        for id in ["no-such-id", "../outside"] {
            let err = handler
                .act(
                    &act_params("update_own_event", serde_json::json!({ "id": id, "title": "Taken" }), None),
                    at("sensitive", "ask"),
                )
                .unwrap_err();
            assert_eq!(err.code, -32602, "{id}");
            assert!(
                err.message == format!("No event here with id {id}")
                    || err.message == format!("`{id}` is not an event id; ids are the names list_events reports, never paths"),
                "{id}: {}",
                err.message
            );
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A reminder is a promise the notifications service keeps for timed events only, so this
    /// door refuses the two ways it could be made silently (#332): `all_day` beside
    /// `reminder_minutes` on `add_event` — the window's own refusal, in the window's words — and
    /// an `update_event` that would turn a reminding timed event into an all-day one. The
    /// store's sentence names the stored lead; the event is left exactly as it was.
    #[test]
    fn no_all_day_change_may_silence_a_reminder() {
        let (handler, dir) = scratch();
        let err = handler
            .act(
                &act_params(
                    "add_event",
                    serde_json::json!({
                        "title": "Fair",
                        "start": days_from_now(2),
                        "end": days_from_now(3),
                        "all_day": true,
                        "reminder_minutes": 30,
                    }),
                    None,
                ),
                at("sensitive", "ask"),
            )
            .unwrap_err();
        assert_eq!(err.message, "an all-day event is never announced; drop `reminder_minutes` or drop `all_day`");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "a refused add stored nothing");

        let added = handler
            .act(
                &act_params(
                    "add_event",
                    serde_json::json!({
                        "title": "Flight",
                        "start": days_from_now(2),
                        "end": days_from_now(3),
                        "reminder_minutes": 45,
                    }),
                    None,
                ),
                at("sensitive", "ask"),
            )
            .expect("a timed event with a lead is what the store keeps");
        let id = added["result"]["id"].clone();

        let err = handler
            .act(
                &act_params("update_event", serde_json::json!({ "id": id, "all_day": true }), None),
                at("sensitive", "auto"),
            )
            .unwrap_err();
        assert!(err.message.contains("never announced"), "{}", err.message);
        assert!(err.message.contains("45 minutes"), "the sentence names the stored lead: {}", err.message);
        let listed = handler
            .act(&act_params("list_events", serde_json::json!({}), None), at("safe", "plan"))
            .unwrap();
        let stored = &listed["result"].as_array().unwrap()[0];
        assert_eq!(stored["is_all_day"], false, "a refused update changed nothing");
        assert_eq!(stored["reminder_minutes"], 45);

        // The other directions stay open: `all_day: false` on a timed event is the no-op it
        // says, and an all-day event may become timed — it gains a reminder, loses none.
        handler
            .act(
                &act_params("update_event", serde_json::json!({ "id": id, "all_day": false }), None),
                at("sensitive", "auto"),
            )
            .expect("saying what is already so changes nothing");
        let all_day = handler
            .act(
                &act_params(
                    "add_event",
                    serde_json::json!({ "title": "Fair", "start": days_from_now(4), "end": days_from_now(4), "all_day": true }),
                    None,
                ),
                at("sensitive", "ask"),
            )
            .expect("an all-day event with no reminder asked for is fine");
        let day_id = all_day["result"]["id"].clone();
        let timed = handler
            .act(
                &act_params("update_event", serde_json::json!({ "id": day_id, "all_day": false }), None),
                at("sensitive", "auto"),
            )
            .expect("all-day to timed gains a reminder");
        assert_eq!(timed["result"]["is_all_day"], false);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The burst cap item 4 of #332 asked for, on the door itself: ten adds in a minute go
    /// through — far more than a person driving a calendar — and the eleventh is refused in a
    /// sentence that says why and how long to wait, having stored nothing.
    #[test]
    fn a_flood_of_adds_is_refused_before_it_floods_the_person() {
        let (handler, dir) = scratch();
        for i in 0..ADD_BURST_LIMIT {
            handler
                .act(
                    &act_params(
                        "add_event",
                        serde_json::json!({ "title": format!("Event {i}"), "start": days_from_now(2), "end": days_from_now(3) }),
                        None,
                    ),
                    at("sensitive", "ask"),
                )
                .unwrap_or_else(|e| panic!("{i}: {}", e.message));
        }
        let err = handler
            .act(
                &act_params(
                    "add_event",
                    serde_json::json!({ "title": "One too many", "start": days_from_now(2), "end": days_from_now(3) }),
                    None,
                ),
                at("sensitive", "ask"),
            )
            .unwrap_err();
        assert!(
            err.message.contains(&format!("added {ADD_BURST_LIMIT} events in the last {} seconds", ADD_BURST_WINDOW.as_secs())),
            "{}",
            err.message
        );
        assert!(err.message.contains("announces itself"), "{}", err.message);
        assert!(err.message.contains("seconds and add this one again"), "{}", err.message);

        // A token the reach file does not know is not a column of its own: the caller wrote it,
        // and a new one per call used to be a fresh cap per call.
        let err = handler
            .act(
                &act_as(
                    "add_event",
                    serde_json::json!({ "title": "Under a new name", "start": days_from_now(2), "end": days_from_now(3) }),
                    "made-up-token-11",
                ),
                at("sensitive", "ask"),
            )
            .unwrap_err();
        assert!(err.message.contains("seconds and add this one again"), "{}", err.message);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), ADD_BURST_LIMIT, "a refused add stored nothing");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The cap's counting, on constructed instants: it refuses at the limit, counts one caller
    /// apart from another, and lets a caller back in once its oldest add has left the window.
    #[test]
    fn the_burst_cap_counts_a_window_per_caller() {
        let burst = BurstGate::new(2, Duration::from_secs(60));
        let t0 = Instant::now();
        assert!(burst.hit_at("a", t0).is_ok());
        assert!(burst.hit_at("a", t0 + Duration::from_secs(1)).is_ok());
        let err = burst.hit_at("a", t0 + Duration::from_secs(2)).unwrap_err();
        assert!(err.contains("wait 59 seconds"), "{err}");
        assert!(burst.hit_at("b", t0 + Duration::from_secs(2)).is_ok(), "one caller's flood is another's business");
        // Sixty-one seconds on, the first add has left the window and the cap lets one in —
        // and only one: at that same instant the add from one second in is inside the window
        // still, so the very next call is refused again.
        assert!(burst.hit_at("a", t0 + Duration::from_secs(61)).is_ok(), "the window slides");
        assert!(burst.hit_at("a", t0 + Duration::from_secs(61)).is_err());
    }

    /// The raw `calendar.*` methods are the desktop's own plumbing (#332): a caller the kernel
    /// cannot place, or a process that is not a Yantrik binary, is refused before any method is
    /// looked at — `create_event` from here takes a caller-written `creator`, which the graded
    /// door establishes by machine. The graded doors are untouched by the check: `app.describe`
    /// still answers any caller at all.
    #[test]
    fn the_raw_methods_answer_only_the_desktops_own_programs() {
        let (handler, dir) = scratch();
        // No peer credentials at all: refused, and the sentence says where the open door is.
        let err = handler.handle(method::EVENTS, serde_json::json!({})).unwrap_err();
        assert_eq!(err.code, -32001);
        assert!(err.message.contains("would not say which process is calling"), "{}", err.message);
        assert!(err.message.contains("app.act"), "{}", err.message);

        // A real process that is not a Yantrik binary — this test runner itself.
        let pid = std::process::id();
        let peer = PeerCred { pid: pid as i32, uid: 0, gid: 0 };
        let err = handler
            .handle_from(method::CREATE_EVENT, serde_json::json!({ "title": "x" }), Some(peer))
            .unwrap_err();
        assert_eq!(err.code, -32001);
        assert!(err.message.contains("the process calling is"), "{}", err.message);
        assert!(err.message.contains(&format!("(pid {pid})")), "{}", err.message);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "a refused create stored nothing");

        // The gate itself never sees the check: `app.describe` answers with no peer.
        let described = handler.handle("app.describe", serde_json::json!({})).expect("describe answers any caller");
        assert_eq!(described["app"], "calendar");

        // And a name no service serves keeps the protocol's own answer for a caller the raw
        // methods refuse: `yos check` probes with one and expects -32601, which the socket
        // layer maps from this -1.
        let err = handler.handle("app.yos_check_no_such_method", serde_json::json!({})).unwrap_err();
        assert_eq!(err.code, -1);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// What counts as one of this OS's own binaries: an absolute path whose last segment is a
    /// `yantrik-` name — including a binary replaced mid-run, which the kernel reports with a
    /// " (deleted)" suffix — and nothing else. A relative path is not the kernel's answer.
    #[test]
    fn a_yantrik_binary_passes_the_peer_check_and_anything_else_does_not() {
        assert!(is_own_binary("/opt/yantrik/bin/yantrik-ui"));
        assert!(is_own_binary("/opt/yantrik/bin/yantrik-calendar (deleted)"), "a service restarted mid-call");
        assert!(is_own_binary("/opt/yantrik/bin/yantrik"), "the CLI: `yantrik ask` runs the companion's tools");
        assert!(is_own_binary("/opt/yantrik/bin/yantrik (deleted)"));
        assert!(!is_own_binary("/opt/yantrik/bin/yantrikish"), "a name that only starts like ours");
        assert!(!is_own_binary("/usr/bin/python3"));
        assert!(!is_own_binary("yantrik-ui"), "not an absolute path");
        assert!(!is_own_binary(""));
    }
}
