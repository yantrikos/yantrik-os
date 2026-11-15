//! One connection to the accessibility bus after another, for as long as the service runs (#1256).
//!
//! The watcher thread (`a11y-watch`) connects, starts the reader (`a11y-signals`), subscribes,
//! registers for the events it wants and follows until the stream ends, then does it all again
//! after a pause that doubles up to a ceiling: accessibility often starts after the session does,
//! and a bus that goes away comes back. What it does with each signal is in `watch`.
//!
//! The reader is started before anything is asked of the bus, so a busy bus can never fill the
//! connection's queue while the watcher waits for an answer that is behind it. Each connection is
//! closed on the way out whatever happened, which is what ends its reader: a session that failed
//! half-way must not leave a reader behind for the next attempt to add to.

use std::sync::mpsc;
use std::time::Duration;

use crate::atspi::Atspi;
use crate::handles;
use crate::screen::Kind;
use crate::sight::Sight;
use crate::signals;
use crate::watch::{register, within, Watcher, CALL_LIMIT, RAW_QUEUE};

/// The longest connecting to the accessibility bus may take.
const CONNECT_LIMIT: Duration = Duration::from_secs(5);

/// The longest pause between attempts to reach the accessibility bus.
const RETRY_CAP: Duration = Duration::from_secs(30);

/// Connections whose close did not finish, kept to be closed again, at most.
const MAX_STUCK: usize = 4;

/// Read, subscribe, register, and follow until the stream ends.
fn listen(runtime: &tokio::runtime::Runtime, atspi: &Atspi, sight: &Sight, handles: &handles::Shared, private: fn() -> bool) -> Result<(), String> {
    // The reader first, before anything is asked of the bus. Every message that arrives on the
    // connection goes through its queue, and a full queue stops the answers arriving too: a
    // busy bus must never be able to fill it while the watcher waits for an answer behind it.
    let connection = zbus::blocking::Connection::from(atspi.connection().clone());
    let messages = zbus::blocking::MessageIterator::from(&connection);
    let (raw_tx, raw_rx) = mpsc::sync_channel(RAW_QUEUE);
    let tally = sight.tally();
    std::thread::Builder::new()
        .name("a11y-signals".into())
        .spawn(move || signals::read(messages, raw_tx, tally))
        .map_err(|e| format!("no signal reader thread: {e}"))?;

    // Subscribed before anything is listed, so nothing that happens during the listing is lost.
    within(runtime, CALL_LIMIT, atspi.subscribe()).ok_or("the accessibility bus did not take the subscription in time")??;
    let registry = within(runtime, CALL_LIMIT, atspi.registry_owner()).flatten().ok_or("the accessibility registry is not on the bus")?;
    let registered = register(runtime, atspi);
    if registered == 0 {
        return Err("the accessibility registry registered none of the events".into());
    }

    sight.set_listening(true);
    tracing::info!(events = registered, "Listening to the accessibility bus");
    let mut watcher = Watcher::new(runtime, atspi, sight, handles, private, registry);
    watcher.follow(&raw_rx);
    Ok(())
}

/// Close what an earlier session could not, once more. Each one that still will not close is
/// kept for the next try.
fn close_stuck(runtime: &tokio::runtime::Runtime, stuck: &mut Vec<zbus::Connection>) {
    let before = std::mem::take(stuck);
    for connection in before {
        if within(runtime, CALL_LIMIT, connection.clone().close()).is_none() {
            keep_newest(stuck, connection, MAX_STUCK);
        }
    }
}

/// Keep `item`, and at most `max` of them: the newest.
fn keep_newest<T>(list: &mut Vec<T>, item: T, max: usize) {
    list.push(item);
    if list.len() > max {
        list.remove(0);
    }
}

/// One connection to the bus, from first to last. The connection is closed on the way out
/// whatever happened, which is what ends its reader thread: a session that failed half-way must
/// not leave a reader behind for the next attempt to add to. A close that did not finish (a send
/// stuck on a wedged bus) is tried again before the next session (#1257 review, N8).
fn session(runtime: &tokio::runtime::Runtime, sight: &Sight, handles: &handles::Shared, private: fn() -> bool, stuck: &mut Vec<zbus::Connection>) -> Result<(), String> {
    close_stuck(runtime, stuck);
    let atspi = within(runtime, CONNECT_LIMIT, Atspi::connect(handles.clone())).ok_or("the accessibility bus did not answer in time")??;
    let outcome = listen(runtime, &atspi, sight, handles, private);
    let connection = atspi.connection().clone();
    if within(runtime, CALL_LIMIT, connection.clone().close()).is_none() {
        keep_newest(stuck, connection, MAX_STUCK);
    }
    outcome
}

/// Start listening on a thread of its own, and hand back the reader's side. Returns at once;
/// the bus is connected to lazily and retried, because accessibility often starts after the
/// session does.
pub fn start(handles: handles::Shared, private: fn() -> bool) -> Sight {
    let sight = Sight::idle();
    let feed = sight.clone();
    let spawned = std::thread::Builder::new().name("a11y-watch".into()).spawn(move || watch_thread(feed, handles, private));
    if let Err(e) = spawned {
        tracing::error!(error = %e, "No watcher thread; the screen model will stay empty");
    }
    sight
}

fn watch_thread(sight: Sight, handles: handles::Shared, private: fn() -> bool) {
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(e) => {
            tracing::error!(error = %e, "No runtime; the screen model will stay empty");
            return;
        }
    };
    let mut pause = Duration::from_secs(1);
    let mut last_why = String::new();
    let mut stuck = Vec::new();
    loop {
        match session(&runtime, &sight, &handles, private, &mut stuck) {
            Ok(()) => {
                tracing::warn!("The accessibility signal stream ended; connecting again");
                pause = Duration::from_secs(1);
            }
            // Said once per reason, not once per attempt: a machine with no accessibility bus
            // would otherwise write the same line every thirty seconds for ever.
            Err(why) if why != last_why => {
                tracing::warn!(%why, "Not listening to the accessibility bus; will keep trying");
                last_why = why;
            }
            Err(_) => {}
        }
        if sight.set_listening(false) {
            // The model is on its own until the next listing: a reader should know.
            let _model = sight.screen();
            // A gap told while private is dropped; the next session tells it when the private
            // time is over (#1257 review, N5).
            let hidden = private();
            if hidden {
                sight.set_private_seen(true);
            }
            sight.record(&handles, hidden, vec![Kind::Gap]);
        }
        std::thread::sleep(pause);
        pause = (pause * 2).min(RETRY_CAP);
    }
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
