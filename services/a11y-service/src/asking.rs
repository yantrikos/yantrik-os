//! Asking an application one thing, within its limit and its share (#1257 review, N2 and H1).

use std::future::Future;
use std::time::{Duration, Instant};

use zbus::zvariant::ObjectPath;

use crate::budget::Patience;
use crate::handles::Reference;
use crate::screen::Key;

/// The longest one question to an application, or to the bus, may take.
pub const CALL_LIMIT: Duration = Duration::from_millis(400);

pub fn key_of(reference: &Reference) -> Key {
    (reference.0.clone(), reference.1.as_str().to_string())
}

pub fn reference_of(key: &Key) -> Option<Reference> {
    ObjectPath::try_from(key.1.as_str()).ok().map(|path| (key.0.clone(), path.into()))
}

/// Wait for one answer, for at most `limit`.
pub fn within<T>(runtime: &tokio::runtime::Runtime, limit: Duration, question: impl Future<Output = T>) -> Option<T> {
    runtime.block_on(async { tokio::time::timeout(limit, question).await.ok() })
}

/// What came of asking.
#[derive(Debug, PartialEq, Eq)]
pub enum Asked<T> {
    /// Nobody was asked: the connection has used its share, or is being left alone. It may be
    /// asked later.
    Refused,
    /// It did not answer in time.
    Silent,
    Answer(T),
}

impl<T> Asked<T> {
    pub fn answer(self) -> Option<T> {
        match self {
            Self::Answer(answer) => Some(answer),
            Self::Refused | Self::Silent => None,
        }
    }
}

/// Ask an application one thing, if it may be asked now, and charge it for the time it took. One
/// that did not answer is also remembered as slow.
pub fn ask<T>(runtime: &tokio::runtime::Runtime, patience: &mut Patience, connection: &str, question: impl Future<Output = T>) -> Asked<T> {
    if !patience.allows(connection, Instant::now()) {
        return Asked::Refused;
    }
    let started = Instant::now();
    let answer = within(runtime, CALL_LIMIT, question);
    let now = Instant::now();
    patience.spent(connection, now.duration_since(started), now);
    match answer {
        Some(answer) => Asked::Answer(answer),
        None => {
            patience.timed_out(connection, now);
            Asked::Silent
        }
    }
}
