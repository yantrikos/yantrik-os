//! Where the person's answers become bond receipts.
//!
//! The bond is scored only from what the person *did*: a card Allowed or refused, a For-you item
//! marked Useful or Not useful, something the machine did on its own that they undid. Each of
//! those already has a place it is decided; this is the one door they hand a receipt through, to
//! the companion thread that owns the store. Nothing here reads a word the person typed.
//!
//! It only writes receipts. It never grants anything: an offer to stop asking (`bond_describe`)
//! is applied by the person's own press on a card, through `mind_mode::person_add_rule`, and no
//! path from here reaches a tool permission or the machine ceiling. The one other thing a person
//! can do with the bond is forget it ([`forget`]), from a button.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use slint::ComponentHandle;
use rusqlite::{Connection, OpenFlags};
use yantrik_companion::bond::{ActionFacts, BondTracker, PressStamp, Receipt, ReceiptKind};
use yantrik_companion::CompanionService;
use yantrik_ipc_contracts::control_surface::{fetches_address, leaves_machine};
use yantrik_ipc_contracts::notifications::notice::FeedbackKind;

use crate::approvals::{self, Card};
use crate::bridge::CompanionBridge;

static DOOR: OnceLock<Door> = OnceLock::new();

/// How long a write waits for another statement on the store (WAL mode, as the worker's own
/// connection and `vault_lane`). Never for a turn: the worker holds the store only per statement.
const BUSY_MS: u64 = 1500;

/// Give the door the companion to send to. Called once, where the shell wires its approvals.
pub fn attach(bridge: Arc<CompanionBridge>) {
    let _ = DOOR.set(Door::new(bridge));
}

/// What the worker is still asked to do about the bond: look again. A receipt or the person's
/// Forget is never a command: the worker's one queue runs a whole LLM turn at a time, and a
/// person's refusal or Forget must not wait behind one, nor vanish with a restart while it did.
pub enum BondCommand {
    /// Re-read the level and score, which the door has just changed in the store.
    Refresh,
}

impl BondCommand {
    /// Run it on the companion's own thread, where its copy of the level lives.
    pub fn apply(self, companion: &mut CompanionService) {
        match self {
            Self::Refresh => companion.refresh_bond(),
        }
    }
}

/// Bumped by every write the door makes, so the worker's `push_bond` can tell that a snapshot it
/// took before that write is older than what the Bond screen was just given.
static DOOR_WRITES: AtomicU64 = AtomicU64::new(0);

/// How many writes the door has made. `push_bond` reads it before it reads the store, and drops
/// its snapshot if it has moved by the time the snapshot would be shown.
pub(crate) fn door_writes() -> u64 {
    DOOR_WRITES.load(Ordering::SeqCst)
}

/// The one door a receipt, or the person's Forget, goes through. It writes **on the caller's
/// thread, at the press**, on its own connection to the store, and holds one lock across the
/// write, so presses are written in the order they were made and each is stamped when pressed. When
/// the call returns the row is in the store, or it is an `Err`: there is no pending state in
/// memory for a shell restart to drop, except one refusal that timed out and is retried once (see
/// [`Door::receipt_retrying`]). The worker is only told to look again, and does that when it gets
/// to it. The door never creates tables and never prunes: the worker does both, at its start.
pub(crate) struct Door {
    bridge: Arc<CompanionBridge>,
    conn: Mutex<Option<Connection>>,
    ui: Mutex<Option<slint::Weak<crate::App>>>,
}

/// Open the store for the door. Only reads: the tables are the worker's to create, so if they
/// are not there yet this is an `Err`, never a write that could wait on the worker or panic.
fn open(path: &Path) -> Result<Connection, String> {
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(path, flags).map_err(|e| e.to_string())?;
    conn.busy_timeout(Duration::from_millis(BUSY_MS)).map_err(|e| e.to_string())?;
    conn.prepare("SELECT event_id, event_type, delta, context, created_at FROM bond_events")
        .map_err(|e| format!("the bond tables are not ready ({e})"))?;
    Ok(conn)
}

fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// What the store says now, to the Bond screen and `describe shell`, from the given connection,
/// so what is shown is what was written; then the worker re-reads its own copy of the level.
fn show(bridge: &CompanionBridge, ui: Option<slint::Weak<crate::App>>, conn: &Connection) {
    DOOR_WRITES.fetch_add(1, Ordering::SeqCst);
    let snapshot = crate::bridge::bond_snapshot_of(conn);
    crate::bond_describe::publish(
        yantrik_companion::bond_table::rows(conn),
        BondTracker::why_it_fell(conn, crate::bond_describe::FELL_PUBLISHED),
    );
    if let Some(ui) = ui {
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui.upgrade() {
                ui.set_bond_data(snapshot.to_ui());
            }
        });
    }
    bridge.bond_command(BondCommand::Refresh);
}

/// What the Bond screen says about a Forget: empty when it happened.
pub(crate) fn forget_message(done: &Result<usize, String>) -> String {
    match done {
        Ok(_) => String::new(),
        Err(why) => format!("Nothing was forgotten: {why}. Try again in a moment."),
    }
}

impl Door {
    pub(crate) fn new(bridge: Arc<CompanionBridge>) -> Self {
        Self { bridge, conn: Mutex::new(None), ui: Mutex::new(None) }
    }

    /// Where Forget's failure is said (the Bond screen), and the bond is shown from.
    fn show_on(&self, ui: slint::Weak<crate::App>) {
        *self.ui.lock().unwrap_or_else(|e| e.into_inner()) = Some(ui);
    }

    fn ui(&self) -> Option<slint::Weak<crate::App>> {
        self.ui.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Run `f` on the store's connection, in turn with every other press. An error from the
    /// store (busy, locked, not open) is returned as it is.
    fn with_store<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T, String> {
        let mut slot = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_none() {
            *slot = Some(open(Path::new(self.bridge.memory_db_path()))?);
        }
        let Some(conn) = slot.as_ref() else {
            return Err("the bond store is not open".to_string());
        };
        let out = f(conn).map_err(|e| e.to_string())?;
        show(&self.bridge, self.ui(), conn);
        Ok(out)
    }

    /// A thing the person did. Stamped now, written now, in press order. `Ok(false)` when it was
    /// not written by choice (the bond is off, the companion is incognito, or the same record was
    /// already there). `Err` when the store would not take it: that is logged at warn with the
    /// receipt's kind, and retried once (see [`Self::receipt_retrying`]).
    pub(crate) fn receipt(&self, receipt: &Receipt) -> Result<bool, String> {
        self.receipt_retrying(receipt).0
    }

    /// [`Self::receipt`], also giving the retry's handle when the first write failed. The press's
    /// stamp is taken inside the connection lock and is kept for the retry, which runs off the UI
    /// thread, on its own connection, with that same stamp, once. If the retry fails too it warns
    /// again and the handle yields `false`: a refusal is never dropped silently.
    pub(crate) fn receipt_retrying(
        &self,
        receipt: &Receipt,
    ) -> (Result<bool, String>, Option<std::thread::JoinHandle<bool>>) {
        if !self.bridge.bond_recording() {
            return (Ok(false), None);
        }
        let kind = receipt.kind.as_str();
        let stamp = Mutex::new(None);
        let first = self.with_store(|conn| {
            let at = PressStamp::now();
            *stamp.lock().unwrap_or_else(|e| e.into_inner()) = Some(at);
            BondTracker::try_record_receipt(conn, receipt, at)
        });
        let Err(why) = &first else {
            return (first, None);
        };
        tracing::warn!(%why, kind, "bond: a receipt could not be written; trying once more");
        let at = stamp.into_inner().unwrap_or_else(|e| e.into_inner()).unwrap_or_else(PressStamp::now);
        let (bridge, ui, receipt) = (self.bridge.clone(), self.ui(), receipt.clone());
        let retry = std::thread::spawn(move || {
            let written = open(Path::new(bridge.memory_db_path()))
                .and_then(|conn| {
                    let out = BondTracker::try_record_receipt(&conn, &receipt, at).map_err(|e| e.to_string())?;
                    show(&bridge, ui, &conn);
                    Ok(out)
                });
            match written {
                Ok(_) => true,
                Err(why) => {
                    tracing::warn!(%why, kind, "bond: a receipt was lost: the retry failed too");
                    false
                }
            }
        });
        (first, Some(retry))
    }

    /// The person's Forget. Done when this returns: the count is logged, and a failure is said on
    /// the Bond screen, never left looking as if it had worked.
    pub(crate) fn forget(&self) -> Result<usize, String> {
        let done = self.with_store(BondTracker::try_forget_receipts);
        match &done {
            Ok(count) => tracing::info!(count, "bond: forgot {count} receipts"),
            Err(why) => tracing::warn!(%why, "bond: Forget did not happen"),
        }
        let said = forget_message(&done);
        if let Some(ui) = self.ui() {
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui.upgrade() {
                    ui.set_bond_forget_error(said.into());
                }
            });
        }
        done
    }
}

fn send(receipt: Receipt) {
    if let Some(door) = DOOR.get() {
        let _ = door.receipt(&receipt);
    }
}

/// Wire the one callback that forgets, and give the door its companion. Called once, where the
/// shell wires its approvals.
pub fn wire(ui: &crate::App, bridge: Arc<CompanionBridge>) {
    attach(bridge);
    if let Some(door) = DOOR.get() {
        door.show_on(ui.as_weak());
    }
    ui.on_forget_bond_history(forget);
}

/// The person pressed "forget the bond history". A UI callback and nothing else: it is not on the
/// control socket, so no Mind can call it, and it empties the receipts, the table and the offers.
/// It is done when this returns (see [`Door`]).
pub fn forget() {
    if let Some(door) = DOOR.get() {
        let _ = door.forget();
    }
}

/// What the card says about its action, read the one way the gate reads it: the app's own
/// sentence, whole (`said`), never the caller's paraphrase.
pub(crate) fn card_facts(card: &Card) -> ActionFacts {
    ActionFacts {
        grade: card.grade.clone(),
        cannot_be_undone: approvals::unrecoverable(&card.said),
        open_ended: approvals::open_ended(&card.said),
        fetches_address: fetches_address(&card.said).is_some(),
        leaves_machine: leaves_machine(&card.said),
        no_session_rule: !card.can_session,
    }
}

/// The receipt for a person's answer to a card. Pure, so each way in is a test.
pub(crate) fn card_receipt(card: &Card, allowed: bool) -> Receipt {
    Receipt {
        kind: if allowed { ReceiptKind::CardAllowed } else { ReceiptKind::CardRefused },
        app: card.app.clone(),
        action: card.action.clone(),
        facts: card_facts(card),
        source_id: card.id.clone(),
    }
}

/// The person pressed Allow or Deny on a card. A card that expired unanswered, or was withdrawn
/// (`approvals_bond::withdraw`), is neither and never comes through here.
pub fn card_answered(card: Option<&Card>, allowed: bool) {
    if let Some(card) = card {
        send(card_receipt(card, allowed));
    }
}

/// The receipt for feedback on a For-you item, or `None` for the kinds that say nothing about
/// whether it was useful ("not now" is about timing, "never" about the family).
///
/// It is filed under the notice's sender harness (`harness`, empty when unknown), so one Mind
/// posting many notices that a person habitually marks Useful is held to the same per-source
/// cap as one card raised over and over ([`yantrik_companion::bond::PAIR_RISE_CAP`]).
pub(crate) fn for_you_receipt(id: &str, harness: &str, kind: &FeedbackKind) -> Option<Receipt> {
    let kind = match kind {
        FeedbackKind::Useful => ReceiptKind::ForYouUseful,
        FeedbackKind::NotUseful => ReceiptKind::ForYouNotUseful,
        _ => return None,
    };
    let mut receipt = Receipt::bare(kind, id);
    receipt.app = if harness.is_empty() { "unknown-harness".to_string() } else { harness.to_string() };
    receipt.action = "for_you".to_string();
    Some(receipt)
}

/// The person pressed Useful or Not useful on a For-you item.
/// `harness` is the id of the harness that sent the notice.
pub fn for_you_answered(id: &str, harness: &str, kind: &FeedbackKind) {
    if let Some(receipt) = for_you_receipt(id, harness, kind) {
        send(receipt);
    }
}

/// The receipt for an undo. It has no record of its own, so it carries no id and the store gives
/// it a fresh one: pressing Undo on the next thing the machine did is another receipt, however
/// alike the two are.
pub(crate) fn undo_receipt(app: &str, action: &str) -> Receipt {
    Receipt {
        kind: ReceiptKind::Undone,
        app: app.to_string(),
        action: action.to_string(),
        facts: ActionFacts::default(),
        source_id: String::new(),
    }
}

/// The person undid something the machine did on its own: a wired model, a Mind's archive or
/// trash. Never the person's own archive or trash, which is theirs.
pub fn undone(app: &str, action: &str) {
    send(undo_receipt(app, action));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approvals::{Store, Verified};
    use crate::approvals_bond;
    use std::time::Instant;

    fn raise(store: &mut Store, app: &str, action: &str, purpose: &str) -> Card {
        let now = Instant::now();
        let id = store
            .request("m", Verified::default(), app, action, serde_json::json!({}), "sensitive", purpose, "", "", now, "12:00")
            .unwrap()
            .id;
        store.cards(now).into_iter().find(|c| c.id == id).unwrap()
    }

    #[test]
    fn a_card_id_carries_the_boot_so_a_restart_cannot_repeat_one() {
        // Both boots count from 1; only the boot part of the id tells them apart.
        let mut store = Store::new();
        let card = raise(&mut store, "notes", "create", "Make a note.");
        assert_eq!(card.id, format!("appr-{}-1", approvals_bond::boot()));
        assert_ne!(approvals_bond::fresh_boot(), approvals_bond::boot());
        let earlier = format!("appr-{}-1", approvals_bond::fresh_boot());
        assert_ne!(card_receipt(&card, true).source_id, earlier);
    }

    #[test]
    fn a_card_without_the_session_button_says_so_in_its_receipt() {
        let (mut store, now) = (Store::new(), Instant::now());
        let id = store
            .request("m", Verified::default(), "notes", "create", serde_json::json!({}), "sensitive", "Make a note.", "", "Because of this one", now, "12:00")
            .unwrap()
            .id;
        let explained = store.cards(now).into_iter().find(|c| c.id == id).unwrap();
        assert!(!explained.can_session, "the premise: a per-call sentence closes the standing yes");
        assert!(card_receipt(&explained, true).facts.no_session_rule);
        let plain = raise(&mut Store::new(), "notes", "create", "Make a note.");
        assert!(plain.can_session, "the premise: an ordinary card draws the session button");
        assert!(!card_receipt(&plain, true).facts.no_session_rule);
    }

    #[test]
    fn the_receipt_carries_what_the_app_said_about_the_action() {
        let mut store = Store::new();
        let gone = raise(&mut store, "calendar", "delete_event", "Delete an event. It is not recoverable.");
        let facts = card_receipt(&gone, true).facts;
        assert!(facts.cannot_be_undone && !facts.leaves_machine);
        assert_eq!(facts.grade, "sensitive");
        let plain = raise(&mut store, "notes", "create", "Make a note.");
        assert_eq!(card_receipt(&plain, true).facts, ActionFacts { grade: "sensitive".into(), ..Default::default() });
    }

    #[test]
    fn an_undo_has_no_id_of_its_own_so_each_one_counts() {
        assert!(undo_receipt("models", "wire_harness").source_id.is_empty());
        assert_eq!(undo_receipt("desktop", "archive").kind, ReceiptKind::Undone);
    }

    #[test]
    fn only_useful_and_not_useful_are_receipts() {
        assert!(for_you_receipt("n1", "mind-a", &FeedbackKind::Useful).is_some());
        assert!(for_you_receipt("n1", "mind-a", &FeedbackKind::NotUseful).is_some());
        assert!(for_you_receipt("n1", "mind-a", &FeedbackKind::NotNow).is_none());
    }

    #[test]
    fn a_for_you_receipt_belongs_to_its_senders_harness_so_it_is_capped() {
        let one = for_you_receipt("n1", "mind-a", &FeedbackKind::Useful).unwrap();
        assert_eq!((one.app.as_str(), one.action.as_str()), ("mind-a", "for_you"));
        let unknown = for_you_receipt("n2", "", &FeedbackKind::Useful).unwrap();
        assert!(!unknown.app.is_empty(), "an unknown sender is still one source, never uncapped");
        // Many notices from one harness, all marked Useful: the fold stops at the cap.
        let mut fold = yantrik_companion::bond::ScoreFold::default();
        for _ in 0..1000 {
            fold.add(one.kind.delta(), &one.app, &one.action);
        }
        assert!(fold.finish().0 <= yantrik_companion::bond::PAIR_RISE_CAP + 1e-9);
    }
}

/// The person's refusal and Forget, with the worker inside a turn (`queue_turn_for_test` puts one on
/// a queue nobody takes from, as a long LLM turn does): each is in the store when the call returns.
#[cfg(test)]
mod door_tests {
    use super::*;
    use crate::bridge::CompanionCommand;
    use std::path::PathBuf;
    use std::time::Instant;
    use yantrik_companion::bond::RECEIPT_EVENT;

    struct Busy {
        door: Door,
        bridge: Arc<CompanionBridge>,
        path: PathBuf,
        queue: crossbeam_channel::Receiver<CompanionCommand>,
        _dir: tempfile::TempDir,
    }

    fn busy() -> Busy {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("memory.db");
        let setup = Connection::open(&path).unwrap();
        setup.pragma_update(None, "journal_mode", "WAL").unwrap(); // as the real store
        BondTracker::ensure_tables(&setup);
        drop(setup);
        let (tx, queue) = crossbeam_channel::unbounded();
        let bridge = Arc::new(CompanionBridge::with_stub_worker(tx, path.to_str().unwrap()));
        bridge.queue_turn_for_test();
        Busy { door: Door::new(bridge.clone()), bridge, path, queue, _dir: dir }
    }

    fn refusal(id: &str) -> Receipt {
        Receipt {
            kind: ReceiptKind::CardRefused,
            app: "notes".into(),
            action: "create".into(),
            facts: ActionFacts::default(),
            source_id: id.into(),
        }
    }

    fn stored(path: &Path) -> Vec<(String, f64)> {
        let conn = Connection::open(path).unwrap();
        let mut stmt = conn
            .prepare("SELECT json_extract(context, '$.source_id'), created_at FROM bond_events WHERE event_type = ?1 ORDER BY rowid")
            .unwrap();
        stmt.query_map([RECEIPT_EVENT], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().flatten().collect()
    }

    #[test]
    fn forget_takes_effect_at_once_while_the_worker_is_busy() {
        let t = busy();
        for id in ["a", "b", "c", "d", "e"] {
            assert_eq!(t.door.receipt(&refusal(id)), Ok(true));
        }
        assert_eq!(stored(&t.path).len(), 5);
        let began = Instant::now();
        assert_eq!(t.door.forget(), Ok(5), "it says how many it forgot");
        assert!(began.elapsed() < Duration::from_secs(2));
        assert!(stored(&t.path).is_empty(), "the receipts are gone though the worker never took a command");
        let state = BondTracker::get_state(&Connection::open(&t.path).unwrap());
        assert_eq!(state.receipts, 0);
        assert_eq!(state.bond_level, yantrik_companion::bond::BondLevel::Stranger);
    }

    #[test]
    fn a_receipt_is_stamped_at_the_press_and_written_before_the_call_returns() {
        let t = busy();
        let before = now();
        assert_eq!(t.door.receipt(&refusal("pressed")), Ok(true));
        let after = now();
        let rows = stored(&t.path);
        assert_eq!(rows.len(), 1, "in the store now, not behind the worker's turn");
        assert!(rows[0].1 >= before && rows[0].1 <= after, "stamped at the press, not at some later write");
    }

    #[test]
    fn presses_are_written_in_press_order_so_a_receipt_before_forget_never_outlives_it() {
        let t = busy();
        t.door.receipt(&refusal("before-1"));
        t.door.receipt(&refusal("before-2"));
        t.door.forget().unwrap();
        t.door.receipt(&refusal("after"));
        let ids: Vec<String> = stored(&t.path).into_iter().map(|r| r.0).collect();
        assert_eq!(ids, ["after"], "what was pressed before Forget is gone; what came after is kept");
    }

    #[test]
    fn nothing_about_a_press_waits_in_memory_for_a_restart_to_drop() {
        let t = busy();
        t.door.receipt(&refusal("kept"));
        t.door.receipt(&refusal("forgotten"));
        t.door.forget().unwrap();
        t.door.receipt(&refusal("kept-too"));
        // A restart: a new door over the same file, with the old one gone.
        let Busy { door, bridge, path, _dir, .. } = t;
        drop(door);
        let again = Door::new(bridge);
        let ids: Vec<String> = stored(&path).into_iter().map(|r| r.0).collect();
        assert_eq!(ids, ["kept-too"]);
        assert_eq!(again.forget(), Ok(1));
    }

    #[test]
    fn the_worker_is_only_asked_to_look_again() {
        let t = busy();
        t.door.receipt(&refusal("x"));
        t.door.forget().unwrap();
        let queued: Vec<CompanionCommand> = t.queue.try_iter().collect();
        assert!(matches!(queued[0], CompanionCommand::SendMessage { .. }), "the turn the worker is in");
        assert!(queued.len() == 3 && queued[1..].iter().all(|c| matches!(c, CompanionCommand::Bond(BondCommand::Refresh))));
    }

    #[test]
    fn incognito_drops_receipts_but_not_the_persons_forget() {
        let t = busy();
        t.door.receipt(&refusal("a"));
        t.bridge.set_incognito(true);
        assert_eq!(t.door.receipt(&refusal("b")), Ok(false), "no receipt is kept while incognito");
        assert_eq!(t.door.forget(), Ok(1));
    }

    /// A second connection holding the write lock, as the worker does mid-statement.
    fn lock_store(path: &Path) -> Connection {
        let holder = Connection::open(path).unwrap();
        holder.execute_batch("BEGIN IMMEDIATE").unwrap();
        holder
    }

    #[test]
    fn a_forget_on_a_busy_store_is_an_error_and_the_screen_says_so() {
        let t = busy();
        t.door.receipt(&refusal("a")).unwrap();
        t.door.receipt(&refusal("b")).unwrap();
        let holder = lock_store(&t.path);
        let began = Instant::now();
        let done = t.door.forget();
        assert!(began.elapsed() < Duration::from_secs(4), "bounded by the busy timeout");
        let why = done.clone().expect_err("a locked store is an Err, never Ok(0)");
        assert!(why.contains("locked"), "{why}");
        assert!(forget_message(&done).starts_with("Nothing was forgotten: "), "{}", forget_message(&done));
        holder.execute_batch("ROLLBACK").unwrap();
        assert_eq!(stored(&t.path).len(), 2, "the receipts are still there");
        assert!(forget_message(&t.door.forget()).is_empty(), "and Forget works once the lock frees");
        assert!(stored(&t.path).is_empty());
    }

    #[test]
    fn the_first_press_on_a_busy_store_does_not_panic() {
        let t = busy(); // the door has not opened its connection yet
        let holder = lock_store(&t.path);
        let (done, retry) = t.door.receipt_retrying(&refusal("first"));
        assert!(done.is_err(), "an Err comes back, not a panic");
        let _ = retry.map(|h| h.join().unwrap());
        holder.execute_batch("ROLLBACK").unwrap();
        // A store with no tables is an Err too, and the door creates none.
        let dir = tempfile::tempdir().unwrap();
        let empty = dir.path().join("memory.db");
        Connection::open(&empty).unwrap().execute_batch("CREATE TABLE other (x)").unwrap();
        let (tx, _q) = crossbeam_channel::unbounded();
        let bridge = Arc::new(CompanionBridge::with_stub_worker(tx, empty.to_str().unwrap()));
        assert!(Door::new(bridge).forget().is_err());
        let tables: i64 = Connection::open(&empty)
            .unwrap()
            .query_row("SELECT count(*) FROM sqlite_master WHERE name = 'bond_events'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(tables, 0, "the door never creates tables");
    }

    #[test]
    fn a_deny_on_a_busy_store_is_retried_once_with_its_press_stamp() {
        let t = busy();
        let holder = lock_store(&t.path);
        let before = now();
        let (first, retry) = t.door.receipt_retrying(&refusal("denied"));
        let after_press = now();
        assert!(first.is_err());
        let retry = retry.expect("a refusal that timed out is retried");
        holder.execute_batch("ROLLBACK").unwrap(); // the lock frees
        assert!(retry.join().unwrap(), "the retry landed");
        let rows = stored(&t.path);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, "denied");
        assert!(rows[0].1 >= before && rows[0].1 <= after_press, "the press's stamp, not the retry's");
    }

    #[test]
    fn a_deny_whose_retry_fails_too_is_reported_lost_not_dropped_silently() {
        let t = busy();
        let holder = lock_store(&t.path);
        let (first, retry) = t.door.receipt_retrying(&refusal("denied"));
        assert!(first.is_err());
        assert!(!retry.unwrap().join().unwrap(), "the retry failed (and warned)");
        holder.execute_batch("ROLLBACK").unwrap();
        assert!(stored(&t.path).is_empty());
    }

    #[test]
    fn a_press_is_one_statement() {
        let t = busy();
        // An old receipt: a press must not prune it (the worker does).
        {
            let conn = Connection::open(&t.path).unwrap();
            conn.execute(
                "INSERT INTO bond_events (event_id, event_type, delta, context, created_at)
                 VALUES ('receipt:old:x', 'receipt', 0.1, '{\"source_id\":\"x\"}', 1.0)",
                [],
            )
            .unwrap();
        }
        let conn = Connection::open(&t.path).unwrap();
        let before = conn.total_changes();
        let at = PressStamp::now();
        assert_eq!(BondTracker::try_record_receipt(&conn, &refusal("one"), at), Ok(true));
        assert_eq!(conn.total_changes() - before, 1, "one INSERT, nothing else written");
        assert_eq!(t.door.receipt(&refusal("two")), Ok(true));
        let ids: Vec<String> = stored(&t.path).into_iter().map(|r| r.0).collect();
        assert!(ids.contains(&"x".to_string()) || stored(&t.path).iter().any(|r| r.1 == 1.0), "the press pruned nothing: {ids:?}");
        BondTracker::prune_old_receipts(&conn);
        assert!(!stored(&t.path).iter().any(|r| r.1 == 1.0), "the worker's prune does it");
    }

    #[test]
    fn a_forget_that_could_not_happen_says_so() {
        let (tx, _queue) = crossbeam_channel::unbounded();
        let bridge = Arc::new(CompanionBridge::with_stub_worker(tx, "/nonexistent-dir/memory.db"));
        assert!(Door::new(bridge).forget().is_err(), "an error, never a silent success");
    }
}
