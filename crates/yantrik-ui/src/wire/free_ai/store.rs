//! The free AI keys, on the companion's worker: the one thread holding the memory database.
//!
//! Reached only through `CompanionBridge::provider_keys`, which is on the UI's own bridge and not
//! on the handle the companion socket serves, so nothing on a socket can store, read or remove a
//! key. A key goes in here and never comes back out to the UI: the card is told which values are
//! kept and their last four characters.

use std::collections::BTreeMap;

use rusqlite::Connection;
use yantrik_companion::tools::provider_keys::{self, Failure};

pub enum Op {
    /// Keep a checked value under its key-store id (`groq`, `cloudflare_account`, …).
    Store { id: String, value: String },
    Remove { id: String },
    /// Which values are kept, with their last four characters.
    Tails,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Reply {
    Done,
    Tails(BTreeMap<String, String>),
    /// The vault has a passphrase and is shut.
    Locked,
    Failed,
    /// The worker did not answer in time (made by the caller, never by `run`): the operation is
    /// still queued and may yet happen.
    NoAnswer,
}

pub fn run(conn: &Connection, op: Op) -> Reply {
    let answer = |r: Result<(), Failure>| match r {
        Ok(()) => Reply::Done,
        Err(Failure::Locked) => Reply::Locked,
        Err(Failure::Store) => Reply::Failed,
    };
    match op {
        Op::Store { id, value } => answer(provider_keys::store(conn, &id, &value)),
        Op::Remove { id } => answer(provider_keys::remove(conn, &id).map(|_| ())),
        Op::Tails => match provider_keys::load_all(conn) {
            Ok(all) => Reply::Tails(all.into_iter().map(|(id, v)| (id, provider_keys::tail(&v))).collect()),
            // Locked: the card still knows which are kept, without their tails.
            Err(Failure::Locked) => Reply::Tails(provider_keys::kept(conn).into_iter().map(|id| (id, String::new())).collect()),
            Err(Failure::Store) => Reply::Failed,
        },
    }
}
