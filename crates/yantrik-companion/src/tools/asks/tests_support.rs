//! What the asking tests share: sample mail and events, a calendar that keeps what it is told,
//! and the quoting the cards use. The fake shell and the situations every tool must get right
//! are `yantrik_companion_tools::fake_shell` and `asks_scenarios`.

use std::sync::Mutex;

use yantrik_companion_tools::asks::quoted;
use yantrik_ipc_contracts::calendar::{
    Attendee, AttendeeStatus, CalendarEvent, CreateEventParams, EventsParams, UpdateEventParams, UpsertRemoteEventParams,
};

use crate::calendar::backend::CalendarBackend;
use crate::email::db::CachedEmail;

pub fn q(text: &str) -> serde_json::Value {
    quoted(text)
}

pub fn mail(subject: &str) -> CachedEmail {
    CachedEmail {
        id: 7,
        account_id: 1,
        uid: 42,
        folder: "INBOX".into(),
        from_addr: "news@example.com".into(),
        from_name: "The Weekly".into(),
        to_addr: "ada@example.com".into(),
        subject: subject.into(),
        // 2026-11-12 09:30:00 UTC
        date_ts: 1_794_475_800.0,
        body_preview: String::new(),
        body_full: String::new(),
        is_read: false,
        is_flagged: false,
        message_id: String::new(),
        in_reply_to: String::new(),
        ai_summary: String::new(),
        importance: String::new(),
    }
}

pub fn event(title: &str) -> CalendarEvent {
    serde_json::from_value(serde_json::json!({
        "id": "ev1", "title": title, "description": "", "start": "2026-11-18T09:00:00", "end": "2026-11-18T10:00:00",
        "is_all_day": false, "location": "Main Street 4", "attendees": [{"name": "Dr Rao", "email": "rao@example.com", "status": "Pending"}],
        "recurrence": null, "calendar_id": "local", "remote_id": null,
    }))
    .expect("a calendar event")
}

pub fn attendee(email: &str) -> Attendee {
    Attendee { name: String::new(), email: email.into(), status: AttendeeStatus::Pending }
}

/// A calendar that holds one event and counts what it is told to do.
pub struct Calendar {
    pub event: Mutex<Option<CalendarEvent>>,
    pub deletes: Mutex<usize>,
    pub updates: Mutex<usize>,
}

impl Calendar {
    pub fn with(event: CalendarEvent) -> Calendar {
        Calendar { event: Mutex::new(Some(event)), deletes: Mutex::new(0), updates: Mutex::new(0) }
    }

    pub fn stored(&self) -> Option<CalendarEvent> {
        self.event.lock().unwrap().clone()
    }
}

impl CalendarBackend for Calendar {
    fn list(&self, _: &EventsParams) -> Result<Vec<CalendarEvent>, String> {
        Ok(self.stored().into_iter().collect())
    }
    fn get(&self, id: &str) -> Result<CalendarEvent, String> {
        self.stored().filter(|e| e.id == id).ok_or_else(|| format!("no event {id}"))
    }
    fn create(&self, _: &CreateEventParams) -> Result<CalendarEvent, String> {
        Err("not used".into())
    }
    fn update(&self, params: &UpdateEventParams) -> Result<CalendarEvent, String> {
        *self.updates.lock().unwrap() += 1;
        let mut slot = self.event.lock().unwrap();
        let event = slot.as_mut().ok_or("no event")?;
        if let Some(title) = &params.title {
            event.title = title.clone();
        }
        if let Some(start) = &params.start {
            event.start = start.clone();
        }
        Ok(event.clone())
    }
    fn delete(&self, id: &str) -> Result<(), String> {
        *self.deletes.lock().unwrap() += 1;
        let mut slot = self.event.lock().unwrap();
        if slot.as_ref().is_some_and(|e| e.id == id) {
            *slot = None;
        }
        Ok(())
    }
    fn upsert_remote(&self, _: &UpsertRemoteEventParams) -> Result<CalendarEvent, String> {
        Err("not used".into())
    }
}

/// A database with the tables the planning tools read and write.
pub fn db() -> yantrikdb_core::YantrikDB {
    let db = yantrikdb_core::YantrikDB::new(":memory:", 384).expect("in-memory database");
    {
        let conn = db.conn();
        crate::recipe::RecipeStore::ensure_tables(&conn);
        crate::scheduler::Scheduler::ensure_table(&conn);
        crate::automation::AutomationStore::ensure_table(&conn);
        crate::task_queue::TaskQueue::ensure_table(&conn);
    }
    db
}

pub fn ctx(db: &yantrikdb_core::YantrikDB) -> crate::tools::ToolContext<'_> {
    crate::tools::ToolContext {
        db,
        max_permission: crate::tools::PermissionLevel::Standard,
        registry_metadata: None,
        task_manager: None,
        incognito: false,
        agent_spawner: None,
    }
}
