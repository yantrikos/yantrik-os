//! The harness host's notice desk: where a mind's notice is posted, and where its feedback and the
//! person's initiative settings are read from (design/initiative-inbox-2026-10-16.md).
//!
//! The host is transport-free; this is the transport. A notice goes to the notifications service
//! as `notifications.add` with a `notice` beside it, from this process — the desktop, which is the
//! only caller the service honours a notice from — naming the harness as the host established it
//! at attach, with the process line the approval card would print for its pid.
//!
//! Every method runs on the harness socket's thread, never the UI thread, and never with the
//! host's lock held.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use yantrik_harness::notice::{NoticeDesk, NoticeSend};
use yantrik_ipc_contracts::notifications::notice::{Initiative, NoticeHarness, NoticePost};
use yantrik_ipc_contracts::notifications::{Since, ADD, NOTICE_FEEDBACK_POLL};
use yantrik_ipc_transport::SyncRpcClient;

const SERVICE: &str = "notifications";

/// How long a post may take. Longer than the shell's poll gets: this answer is a promise to a
/// mind that its card is on the disk, and the store fsyncs before it gives it.
const POST_TIMEOUT: Duration = Duration::from_secs(5);

/// The longest the feedback for one harness is served from memory without asking the store. A
/// harness polls five times a second; the store's revision, which the shell's notification poll
/// keeps here, says when anything changed, and this bounds how wrong a missed change can be.
const FEEDBACK_FRESH: Duration = Duration::from_secs(30);

/// The person's settings as the shell's notification poll last read them from the store.
static INITIATIVE: Mutex<Option<Initiative>> = Mutex::new(None);
/// The store's revision as that poll last saw it.
static REVISION: AtomicU64 = AtomicU64::new(0);

/// Called by the shell's notification poll with every answer it gets: the settings a harness is
/// told on its next poll, and whether anything in the store moved.
pub fn observe(since: &Since) {
    if let Some(settings) = &since.initiative {
        if let Ok(mut held) = INITIATIVE.lock() {
            *held = Some(settings.clone());
        }
    }
    REVISION.store(since.revision, Ordering::Relaxed);
}

/// The settings as last read, for the Settings screen.
pub fn initiative() -> Initiative {
    INITIATIVE.lock().ok().and_then(|s| s.clone()).unwrap_or_default()
}

/// The settings as the store just answered a change with, so the screen and the next harness
/// poll need not wait for the notification poll's next second.
pub fn set_initiative(settings: Initiative) {
    if let Ok(mut held) = INITIATIVE.lock() {
        *held = Some(settings);
    }
}

/// One harness's feedback, as last read from the store.
struct Cached {
    revision: u64,
    acked: u64,
    at: Instant,
    items: Vec<serde_json::Value>,
}

#[derive(Default)]
pub struct ShellDesk {
    feedback: Mutex<HashMap<String, Cached>>,
}

impl ShellDesk {
    pub fn new() -> ShellDesk {
        ShellDesk::default()
    }
}

/// The program behind `pid`, by the name of its executable, as a harness id is bound to it. Empty
/// when `/proc` cannot say.
fn program(pid: Option<u32>) -> String {
    let found = yantrik_ipc_transport::peer_identity::resolve(pid.and_then(|p| i32::try_from(p).ok()));
    let exe = found.direct.map(|f| f.exe).unwrap_or_default();
    // A binary replaced by an update is still the same program to the person.
    let exe = exe.trim_end_matches(" (deleted)");
    exe.rsplit('/').next().unwrap_or_default().to_string()
}

fn call(method: &str, params: serde_json::Value, timeout: Duration) -> Result<serde_json::Value, String> {
    yantrik_app_runtime::service::ensure(SERVICE).map_err(|e| e.to_string())?;
    SyncRpcClient::for_service(SERVICE).with_timeout(timeout).call(method, params).map_err(|e| e.message)
}

impl NoticeDesk for ShellDesk {
    fn post(&self, send: &NoticeSend) -> Result<serde_json::Value, String> {
        let verified = yantrik_ipc_transport::peer_identity::resolve(send.pid.and_then(|p| i32::try_from(p).ok())).line();
        let post = NoticePost {
            notice: send.notice.clone(),
            harness: NoticeHarness {
                id: send.harness.clone(),
                uid: send.uid,
                verified,
                name: send.name.clone(),
                program: program(send.pid),
            },
            os_id: send.os_id,
        };
        // `title` because `notifications.add` needs one for the plain notification a notice also
        // is; the store takes the notice's own, clamped, when it honours the notice.
        let title = send.notice["title"].as_str().unwrap_or("Notice").to_string();
        let params = serde_json::json!({ "app": send.name, "title": title, "notice": post });
        call(ADD, params, POST_TIMEOUT)
    }

    fn initiative(&self) -> serde_json::Value {
        initiative().wire()
    }

    fn feedback(&self, harness: &str, pid: Option<u32>, uid: Option<u32>, ack: Option<u64>) -> Result<Vec<serde_json::Value>, String> {
        let revision = REVISION.load(Ordering::Relaxed);
        let mut cache = self.feedback.lock().map_err(|_| "the feedback cache is poisoned".to_string())?;
        if let Some(held) = cache.get(harness) {
            let nothing_new = held.revision == revision && ack.is_none_or(|a| a <= held.acked) && held.at.elapsed() < FEEDBACK_FRESH;
            if nothing_new {
                return Ok(held.items.clone());
            }
        }
        let mut params = serde_json::json!({ "harness": harness, "program": program(pid) });
        if let Some(uid) = uid {
            params["uid"] = uid.into();
        }
        if let Some(ack) = ack {
            params["ack"] = ack.into();
        }
        let reply = call(NOTICE_FEEDBACK_POLL, params, Duration::from_secs(2))?;
        let items = reply["notice_feedback"].as_array().cloned().unwrap_or_default();
        let acked = ack.unwrap_or(0).max(cache.get(harness).map_or(0, |c| c.acked));
        cache.insert(harness.to_string(), Cached { revision, acked, at: Instant::now(), items: items.clone() });
        Ok(items)
    }
}
