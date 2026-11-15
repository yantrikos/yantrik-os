//! Reading the accessibility bus, and sorting what comes off it (#1256).
//!
//! The reader thread does this and nothing else: no calls, no locks, nothing that can stall. Each
//! signal is sorted by its name into a [`Raw`] and handed to the watcher without ever waiting
//! for it. When the watcher is behind and the queue between them is full, the signal is counted
//! and dropped and the overflow flag is raised, so the watcher can say "gap" and make the model
//! right again. Backing up into the bus instead would stall every application's announcements.
//!
//! # What is read from a signal, and what is not
//!
//! A signal carries a free-form value. For a children-changed signal it is the child's
//! reference, and for a name change it is the new name: both are read. For a text-changed signal
//! it is the text that was typed, and it is never read: the signal is sorted by name alone, and
//! [`Raw::Text`] has nowhere to put it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError};
use std::sync::Arc;

use zbus::zvariant::{ObjectPath, Value};

use crate::screen::Key;

/// A signal, sorted by what it is about. Nothing here was asked of anyone.
#[derive(Debug, PartialEq, Eq)]
pub enum Raw {
    Created(Key),
    Activated(Key),
    Deactivated(Key),
    Destroyed(Key),
    Focused(Key),
    Children { source: Key, added: bool, child: Option<Key> },
    Text(Key),
    Name { source: Key, name: Option<String> },
    /// A connection left the bus: whatever it owned went with it.
    Vanished(String),
}

/// What the reader counts, and the flag it raises when it had to drop.
#[derive(Default)]
pub struct Tally {
    /// Signals sorted into something the model has a use for.
    pub seen: AtomicU64,
    /// Signals dropped because the watcher was behind.
    pub dropped: AtomicU64,
    /// Raised on a drop; the watcher lowers it when it has told the gap.
    pub overflowed: AtomicBool,
}

/// The application root, and the registry's own root, live at this path on every connection.
pub fn is_root(path: &str) -> bool {
    path.ends_with("/accessible/root")
}

/// Sort one event signal. `any` is the signal's free-form value, read only where it is a
/// reference or a name: never for text.
fn sort(interface: &str, member: &str, source: Key, detail: &str, detail1: i32, any: &Value<'_>) -> Option<Raw> {
    match (interface, member) {
        ("org.a11y.atspi.Event.Window", "Create") => Some(Raw::Created(source)),
        ("org.a11y.atspi.Event.Window", "Activate") => Some(Raw::Activated(source)),
        ("org.a11y.atspi.Event.Window", "Deactivate") => Some(Raw::Deactivated(source)),
        ("org.a11y.atspi.Event.Window", "Destroy" | "Close") => Some(Raw::Destroyed(source)),
        ("org.a11y.atspi.Event.Object", "StateChanged") => (detail == "focused" && detail1 != 0).then_some(Raw::Focused(source)),
        ("org.a11y.atspi.Event.Object", "ChildrenChanged") => {
            let added = detail.starts_with("add");
            (added || detail.starts_with("remove")).then(|| Raw::Children { source, added, child: reference_in(any) })
        }
        ("org.a11y.atspi.Event.Object", "TextChanged") => Some(Raw::Text(source)),
        ("org.a11y.atspi.Event.Object", "PropertyChange") => (detail == "accessible-name").then(|| Raw::Name { source, name: string_in(any) }),
        _ => None,
    }
}

fn unwrapped<'a>(mut value: &'a Value<'a>) -> &'a Value<'a> {
    while let Value::Value(inner) = value {
        value = inner;
    }
    value
}

/// The object reference a children-changed signal carries, when it carries one.
fn reference_in(any: &Value<'_>) -> Option<Key> {
    let Value::Structure(pair) = unwrapped(any) else { return None };
    match pair.fields() {
        [Value::Str(bus), Value::ObjectPath(path)] => Some((bus.as_str().to_string(), path.as_str().to_string())),
        _ => None,
    }
}

fn string_in(any: &Value<'_>) -> Option<String> {
    match unwrapped(any) {
        Value::Str(text) => Some(text.as_str().to_string()),
        _ => None,
    }
}

/// Sort one message off the bus, or `None` for anything the model has no use for.
fn parse(message: &zbus::Message) -> Option<Raw> {
    let header = message.header();
    let interface = header.interface()?.as_str();
    let member = header.member()?.as_str();
    let body = message.body();
    if interface == "org.freedesktop.DBus" {
        if member != "NameOwnerChanged" {
            return None;
        }
        let (name, old, new): (&str, &str, &str) = body.deserialize().ok()?;
        return (name.starts_with(':') && !old.is_empty() && new.is_empty()).then(|| Raw::Vanished(name.to_string()));
    }
    let source: Key = (header.sender()?.as_str().to_string(), header.path()?.as_str().to_string());
    // Two shapes are in use: `siiva{sv}` (GTK, Chromium, Slint) and the older `siiv(so)` (Qt).
    type Current<'a> = (&'a str, i32, i32, Value<'a>, HashMap<&'a str, Value<'a>>);
    type Older<'a> = (&'a str, i32, i32, Value<'a>, (&'a str, ObjectPath<'a>));
    if let Ok((detail, detail1, _, any, _)) = body.deserialize::<Current<'_>>() {
        return sort(interface, member, source, detail, detail1, &any);
    }
    let (detail, detail1, _, any, _) = body.deserialize::<Older<'_>>().ok()?;
    sort(interface, member, source, detail, detail1, &any)
}

/// Read the bus until it closes, handing each sorted signal to the watcher without ever waiting
/// for it. Returning drops the sender, which is how the watcher learns the stream ended.
pub fn read(signals: zbus::blocking::MessageIterator, raw: SyncSender<Raw>, tally: Arc<Tally>) {
    for message in signals {
        let Ok(message) = message else { break };
        let Some(sorted) = parse(&message) else { continue };
        tally.seen.fetch_add(1, Ordering::Relaxed);
        match raw.try_send(sorted) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                tally.dropped.fetch_add(1, Ordering::Relaxed);
                tally.overflowed.store(true, Ordering::Relaxed);
            }
            Err(TrySendError::Disconnected(_)) => return,
        }
    }
}

#[cfg(test)]
#[path = "signals_tests.rs"]
mod tests;
