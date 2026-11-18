//! An in-memory calendar-service, a table of callers and a clock a test sets: what an app under
//! test talks to instead of a socket, `/proc` and the machine's time.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use serde_json::{json, Value};
use yantrik_app::Paths;
use yantrik_ipc_contracts::calendar::{
    method, CalendarEvent, CreateEventParams, DeleteEventParams, EventsParams, GetEventParams,
    UpdateEventParams, DEFAULT_REMINDER_MINUTES,
};

use super::{CalendarApi, Machine, RpcFailure, NOT_REACHED};
use crate::ownership;
use crate::today::FakeClock;
use crate::views::WeekStart;
use crate::who::{Caller, Who};

#[derive(Default)]
struct Store {
    events: Vec<CalendarEvent>,
    next_id: u64,
    /// Moves on every write, as the newest modification time does.
    writes: u64,
    down: bool,
    /// Calls made through the door that starts the service, while it was down.
    start_attempts: usize,
    /// Methods that fail, once each, before they answer.
    failing: HashMap<String, VecDeque<RpcFailure>>,
    /// Methods whose call panics, as a parser bug would.
    panics: Vec<String>,
    asked: Vec<(String, Value)>,
}

/// The fake service. Cloning shares it.
#[derive(Clone, Default)]
pub struct FakeCalendar(Arc<Mutex<Store>>);

fn refuse(code: i32, message: impl Into<String>) -> RpcFailure {
    RpcFailure { code, message: message.into() }
}

fn day_of(iso: &str) -> &str {
    iso.get(..10).unwrap_or(iso)
}

impl FakeCalendar {
    fn store(&self) -> MutexGuard<'_, Store> {
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// One event already stored, as another writer (or an earlier run) left it.
    pub fn holding(self, id: &str, title: &str, start: &str, end: &str) -> FakeCalendar {
        self.put(id, title, start, end, None);
        self
    }

    /// An event stored now by somebody else: the mind's own tools, or Google sync.
    pub fn put(&self, id: &str, title: &str, start: &str, end: &str, creator: Option<&str>) {
        let mut store = self.store();
        store.writes += 1;
        store.events.push(CalendarEvent {
            id: id.into(),
            title: title.into(),
            description: String::new(),
            start: start.into(),
            end: end.into(),
            is_all_day: false,
            location: None,
            attendees: Vec::new(),
            recurrence: None,
            calendar_id: "default".into(),
            remote_id: None,
            creator: creator.map(str::to_string),
            reminder_minutes: DEFAULT_REMINDER_MINUTES,
        });
    }

    /// The service stops listening (or comes back).
    pub fn set_down(&self, down: bool) {
        self.store().down = down;
    }

    /// `method` fails once with this code and sentence before it answers again.
    pub fn fail_once(&self, method: &str, code: i32, message: &str) {
        self.store().failing.entry(method.into()).or_default().push_back(refuse(code, message));
    }

    pub fn panicking(&self, method: &str) {
        self.store().panics.push(method.into());
    }

    /// How many times the service was asked to start: a call through `call` while it was down.
    pub fn start_attempts(&self) -> usize {
        self.store().start_attempts
    }

    pub fn stored(&self) -> Vec<CalendarEvent> {
        self.store().events.clone()
    }

    pub fn asked_for(&self, method: &str) -> Vec<Value> {
        self.store().asked.iter().filter(|(m, _)| m == method).map(|(_, p)| p.clone()).collect()
    }

    fn answer(store: &mut Store, method_name: &str, params: Value) -> Result<Value, RpcFailure> {
        let bad = |e: serde_json::Error| refuse(-32602, e.to_string());
        match method_name {
            method::EVENTS => {
                let p: EventsParams = serde_json::from_value(params).map_err(bad)?;
                let (from, to) = (day_of(&p.start_date).to_string(), day_of(&p.end_date).to_string());
                let mut hits: Vec<CalendarEvent> = store
                    .events
                    .iter()
                    .filter(|e| day_of(&e.start) <= to.as_str() && day_of(&e.end) >= from.as_str())
                    .cloned()
                    .collect();
                hits.sort_by(|a, b| a.start.cmp(&b.start));
                Ok(json!(hits))
            }
            method::GET_EVENT => {
                let p: GetEventParams = serde_json::from_value(params).map_err(bad)?;
                store
                    .events
                    .iter()
                    .find(|e| e.id == p.id)
                    .map(|e| json!(e))
                    .ok_or_else(|| refuse(-32602, format!("no event with id {}", p.id)))
            }
            method::CREATE_EVENT => {
                let p: CreateEventParams = serde_json::from_value(params).map_err(bad)?;
                store.next_id += 1;
                store.writes += 1;
                let event = CalendarEvent {
                    id: format!("ev-{}", store.next_id),
                    title: p.title,
                    description: p.description,
                    start: p.start,
                    end: p.end,
                    is_all_day: p.is_all_day,
                    location: p.location,
                    attendees: p.attendees,
                    recurrence: None,
                    calendar_id: "default".into(),
                    remote_id: None,
                    creator: p.creator,
                    reminder_minutes: p.reminder_minutes.unwrap_or(DEFAULT_REMINDER_MINUTES),
                };
                store.events.push(event.clone());
                Ok(json!(event))
            }
            method::DELETE_EVENT => {
                let p: DeleteEventParams = serde_json::from_value(params).map_err(bad)?;
                let before = store.events.len();
                store.events.retain(|e| e.id != p.id);
                if store.events.len() == before {
                    return Err(refuse(-32602, format!("no event with id {}", p.id)));
                }
                store.writes += 1;
                Ok(json!({ "deleted": p.id }))
            }
            method::UPDATE_EVENT => {
                let p: UpdateEventParams = serde_json::from_value(params).map_err(bad)?;
                let Some(event) = store.events.iter_mut().find(|e| e.id == p.id) else {
                    return Err(refuse(-32602, format!("no event with id {}", p.id)));
                };
                if let Some(title) = p.title {
                    event.title = title;
                }
                if let Some(start) = p.start {
                    event.start = start;
                }
                if let Some(end) = p.end {
                    event.end = end;
                }
                if let Some(description) = p.description {
                    event.description = description;
                }
                if let Some(minutes) = p.reminder_minutes {
                    event.reminder_minutes = minutes;
                }
                let event = event.clone();
                store.writes += 1;
                Ok(json!(event))
            }
            method::REVISION => Ok(json!({ "events": store.events.len(), "newest_nanos": store.writes })),
            other => Err(refuse(-32601, format!("Unknown method: {other}"))),
        }
    }

    fn take(&self, method_name: &str, params: Value) -> Result<Value, RpcFailure> {
        let mut store = self.store();
        store.asked.push((method_name.into(), params.clone()));
        if store.panics.iter().any(|m| m == method_name) {
            drop(store);
            panic!("the fake calendar service was told to panic on {method_name}");
        }
        if let Some(failure) = store.failing.get_mut(method_name).and_then(VecDeque::pop_front) {
            return Err(failure);
        }
        FakeCalendar::answer(&mut store, method_name, params)
    }
}

impl CalendarApi for FakeCalendar {
    fn call(&self, method_name: &str, params: Value) -> Result<Value, RpcFailure> {
        {
            let mut store = self.store();
            if store.down {
                store.start_attempts += 1;
                return Err(refuse(NOT_REACHED, "the calendar service did not come up"));
            }
        }
        self.take(method_name, params)
    }

    fn is_up(&self) -> bool {
        !self.store().down
    }

    fn call_running(&self, method_name: &str, params: Value) -> Result<Value, RpcFailure> {
        if self.store().down {
            return Err(refuse(NOT_REACHED, "Service at calendar.sock is unreachable"));
        }
        self.take(method_name, params)
    }
}

/// Callers by agent token: a call carrying token `t` is the agent `t`; any other is nobody.
#[derive(Debug, Clone, Copy, Default)]
pub struct FakeWho;

impl Who for FakeWho {
    fn identify(&self, caller: &Caller) -> Option<String> {
        caller.agent_token.as_deref().map(ownership::agent_identity)
    }
}

/// What an app under test is given: the store, the callers, a clock, a Sunday week.
pub fn machine(store: &FakeCalendar, clock: &Arc<FakeClock>) -> Machine {
    Machine {
        api: Arc::new(store.clone()),
        who: Arc::new(FakeWho),
        clock: clock.clone(),
        week_start: WeekStart::Sunday,
    }
}

fn installed() -> &'static Mutex<HashMap<PathBuf, Machine>> {
    static INSTALLED: OnceLock<Mutex<HashMap<PathBuf, Machine>>> = OnceLock::new();
    INSTALLED.get_or_init(Default::default)
}

/// The machine an app started at `paths` talks to. Each test's paths are its own.
pub fn install(paths: &Paths, machine: Machine) {
    installed().lock().unwrap_or_else(|p| p.into_inner()).insert(paths.config_file().to_path_buf(), machine);
}

pub fn for_app(paths: &Paths) -> Machine {
    let found = installed().lock().unwrap_or_else(|p| p.into_inner()).get(paths.config_file()).cloned();
    found.unwrap_or_else(|| machine(&FakeCalendar::default(), &Arc::new(FakeClock::at("2026-09-23 10:00:00"))))
}
