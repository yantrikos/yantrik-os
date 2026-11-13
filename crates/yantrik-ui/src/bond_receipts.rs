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
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use slint::ComponentHandle;
use rusqlite::{Connection, OpenFlags};
use yantrik_companion::bond::{ActionFacts, BondTracker, Receipt, ReceiptKind};
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

/// The one door a receipt, or the person's Forget, goes through. It writes **on the caller's
/// thread, at the press**, on its own connection to the store, and holds one lock across the
/// write, so presses are written in the order they were made and each is stamped when pressed. When
/// the call returns the row is in the store: there is no pending state in memory for a shell
/// restart to drop. The worker is only told to look again, and does that when it gets to it.
pub(crate) struct Door {
    bridge: Arc<CompanionBridge>,
    conn: Mutex<Option<Connection>>,
    ui: Mutex<Option<slint::Weak<crate::App>>>,
}

fn open(path: &Path) -> Result<Connection, String> {
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(path, flags).map_err(|e| e.to_string())?;
    conn.busy_timeout(Duration::from_millis(BUSY_MS)).map_err(|e| e.to_string())?;
    // Idempotent, and the worker makes the same tables at its start.
    BondTracker::ensure_tables(&conn);
    Ok(conn)
}

fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

impl Door {
    pub(crate) fn new(bridge: Arc<CompanionBridge>) -> Self {
        Self { bridge, conn: Mutex::new(None), ui: Mutex::new(None) }
    }

    /// Where Forget's failure is said (the Bond screen), and the bond is shown from.
    fn show_on(&self, ui: slint::Weak<crate::App>) {
        *self.ui.lock().unwrap_or_else(|e| e.into_inner()) = Some(ui);
    }

    /// Run `f` on the store's connection, in turn with every other press.
    fn with_store<T>(&self, f: impl FnOnce(&Connection) -> T) -> Result<T, String> {
        let mut slot = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_none() {
            *slot = Some(open(Path::new(self.bridge.memory_db_path()))?);
        }
        let conn = slot.as_ref().expect("opened above");
        let out = f(conn);
        self.show(conn);
        Ok(out)
    }

    /// What the store says now, to the Bond screen and `describe shell`, from this connection, so
    /// what is shown is what was written; then the worker re-reads its own copy of the level.
    fn show(&self, conn: &Connection) {
        let snapshot = crate::bridge::bond_snapshot_of(conn);
        crate::bond_describe::publish(
            yantrik_companion::bond_table::rows(conn),
            BondTracker::why_it_fell(conn, crate::bond_describe::FELL_PUBLISHED),
        );
        if let Some(ui) = self.ui.lock().unwrap_or_else(|e| e.into_inner()).clone() {
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui.upgrade() {
                    ui.set_bond_data(snapshot.to_ui());
                }
            });
        }
        self.bridge.bond_command(BondCommand::Refresh);
    }

    /// A thing the person did. Stamped now, written now, in press order. `false` when it was not
    /// written (the bond is off, the companion is incognito, the store would not open).
    pub(crate) fn receipt(&self, receipt: &Receipt) -> bool {
        if !self.bridge.bond_recording() {
            return false;
        }
        match self.with_store(|conn| BondTracker::record_receipt_at(conn, receipt, now())) {
            Ok(written) => written,
            Err(why) => {
                tracing::warn!(%why, kind = receipt.kind.as_str(), "bond: a receipt could not be written");
                false
            }
        }
    }

    /// The person's Forget. Done when this returns: the count is logged, and a failure is said on
    /// the Bond screen, never left looking as if it had worked.
    pub(crate) fn forget(&self) -> Result<usize, String> {
        let done = self.with_store(BondTracker::forget_receipts);
        let said = match &done {
            Ok(count) => {
                tracing::info!(count, "bond: forgot {count} receipts");
                String::new()
            }
            Err(why) => {
                tracing::warn!(%why, "bond: Forget did not happen");
                format!("Nothing was forgotten: {why}. Try again in a moment.")
            }
        };
        if let Some(ui) = self.ui.lock().unwrap_or_else(|e| e.into_inner()).clone() {
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
        door.receipt(&receipt);
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
        BondTracker::ensure_tables(&Connection::open(&path).unwrap());
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
            assert!(t.door.receipt(&refusal(id)));
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
        assert!(t.door.receipt(&refusal("pressed")));
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
        assert!(!t.door.receipt(&refusal("b")), "no receipt is kept while incognito");
        assert_eq!(t.door.forget(), Ok(1));
    }

    #[test]
    fn a_forget_that_could_not_happen_says_so() {
        let (tx, _queue) = crossbeam_channel::unbounded();
        let bridge = Arc::new(CompanionBridge::with_stub_worker(tx, "/nonexistent-dir/memory.db"));
        assert!(Door::new(bridge).forget().is_err(), "an error, never a silent success");
    }
}
