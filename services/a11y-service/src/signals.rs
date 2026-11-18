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
//! A signal is sorted by its interface and member first, and its body is read only where the
//! model needs something from it: the state that changed, the child that came or went, the new
//! name. A text-changed signal's body is the text that was typed, and it is never read at all:
//! not parsed, not copied. A name is cut to its limit as it is read.
//!
//! # Who is believed
//!
//! Anyone on the bus can send a signal with any name. Two kinds matter enough to check: a
//! `NameOwnerChanged` is believed only from the bus itself, and what a signal says about a
//! child is believed only as far as the watcher's own checks allow ([`root_change`]).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError};
use std::sync::Arc;

use zbus::zvariant::{ObjectPath, Value};

use crate::limits;
use crate::screen::Key;

pub const WINDOW_EVENTS: &str = "org.a11y.atspi.Event.Window";
pub const OBJECT_EVENTS: &str = "org.a11y.atspi.Event.Object";
pub const BUS: &str = "org.freedesktop.DBus";
pub const REGISTRY: &str = "org.a11y.atspi.Registry";

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
    /// The registry has a new owner: what was registered with the old one is gone.
    RegistryRestarted,
}

/// What the reader counts, and the flag it raises when it had to drop.
#[derive(Default)]
pub struct Tally {
    /// Signals dropped because the watcher was behind.
    pub dropped: AtomicU64,
    /// Raised on a drop; the watcher lowers it when it has told the gap.
    pub overflowed: AtomicBool,
}

/// The application root, and the registry's own root, live at this path on every connection.
pub fn is_root(path: &str) -> bool {
    path.ends_with("/accessible/root")
}

/// What a children-changed signal on a root means for the model.
#[derive(Debug, PartialEq, Eq)]
pub enum RootChange {
    /// An application left: its windows went with it, whichever they were.
    Relist,
    /// A window of the announcing application came.
    Learn(Key),
    /// A window of the announcing application went.
    Closed(Key),
    /// Not something to act on.
    Ignore,
}

/// Decide what a children-changed signal on a root is believed to say (#1257 review, H2 and M2).
///
/// Two rules, because any application can emit this signal about anything:
/// - that an *application* came or went is believed only from the registry itself;
/// - that a *window* came or went is believed only from the connection that owns the window.
///   Without that, one application could close another's windows in the model with one signal.
pub fn root_change(source: &Key, added: bool, child: Option<Key>, registry: &str) -> RootChange {
    match child {
        Some(child) if is_root(&child.1) => {
            if source.0 == registry && !added {
                RootChange::Relist
            } else {
                // An application that came announces its own windows.
                RootChange::Ignore
            }
        }
        Some(child) if child.0 != source.0 => RootChange::Ignore,
        Some(child) if added => RootChange::Learn(child),
        Some(child) => RootChange::Closed(child),
        // No child named: something changed under the registry, and only a look can say what.
        None if source.0 == registry => RootChange::Relist,
        None => RootChange::Ignore,
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

/// The new name a name-change signal carries, cut to the length a name is kept at.
fn name_in(any: &Value<'_>) -> Option<String> {
    match unwrapped(any) {
        Value::Str(text) => Some(limits::name(text.as_str())),
        _ => None,
    }
}

/// Sort one object event whose body the model needs. `any` is the signal's free-form value.
fn sort_object(member: &str, source: Key, detail: &str, detail1: i32, any: &Value<'_>) -> Option<Raw> {
    match member {
        "StateChanged" => (detail == "focused" && detail1 != 0).then_some(Raw::Focused(source)),
        "ChildrenChanged" => {
            let added = detail.starts_with("add");
            (added || detail.starts_with("remove")).then(|| Raw::Children { source, added, child: reference_in(any) })
        }
        "PropertyChange" => (detail == "accessible-name").then(|| Raw::Name { source, name: name_in(any) }),
        _ => None,
    }
}

/// Sort one message off the bus, or `None` for anything the model has no use for.
pub fn parse(message: &zbus::Message) -> Option<Raw> {
    if message.message_type() != zbus::message::Type::Signal {
        return None;
    }
    let header = message.header();
    let interface = header.interface()?.as_str();
    let member = header.member()?.as_str();
    let sender = header.sender()?.as_str();
    if interface == BUS {
        // Only the bus says who owns a name. The same words from anyone else are just words.
        if member != "NameOwnerChanged" || sender != BUS {
            return None;
        }
        let body = message.body();
        let (name, old, new): (&str, &str, &str) = body.deserialize().ok()?;
        if name == REGISTRY {
            return (!new.is_empty()).then_some(Raw::RegistryRestarted);
        }
        return (name.starts_with(':') && !old.is_empty() && new.is_empty()).then(|| Raw::Vanished(name.to_string()));
    }
    let source: Key = (sender.to_string(), header.path()?.as_str().to_string());
    match (interface, member) {
        (WINDOW_EVENTS, "Create") => Some(Raw::Created(source)),
        (WINDOW_EVENTS, "Activate") => Some(Raw::Activated(source)),
        (WINDOW_EVENTS, "Deactivate") => Some(Raw::Deactivated(source)),
        (WINDOW_EVENTS, "Destroy" | "Close") => Some(Raw::Destroyed(source)),
        // The body is what was typed. It is not read.
        (OBJECT_EVENTS, "TextChanged") => Some(Raw::Text(source)),
        (OBJECT_EVENTS, "StateChanged" | "ChildrenChanged" | "PropertyChange") => {
            let body = message.body();
            // Two shapes are in use: `siiva{sv}` (GTK, Chromium, Slint) and the older
            // `siiv(so)` (Qt).
            type Current<'a> = (&'a str, i32, i32, Value<'a>, HashMap<&'a str, Value<'a>>);
            type Older<'a> = (&'a str, i32, i32, Value<'a>, (&'a str, ObjectPath<'a>));
            if let Ok((detail, detail1, _, any, _)) = body.deserialize::<Current<'_>>() {
                return sort_object(member, source, detail, detail1, &any);
            }
            let (detail, detail1, _, any, _) = body.deserialize::<Older<'_>>().ok()?;
            sort_object(member, source, detail, detail1, &any)
        }
        _ => None,
    }
}

/// Read the bus until it closes, handing each sorted signal to the watcher without ever waiting
/// for it. Returning drops the sender, which is how the watcher learns the stream ended.
pub fn read(messages: zbus::blocking::MessageIterator, raw: SyncSender<Raw>, tally: Arc<Tally>) {
    for message in messages {
        let Ok(message) = message else { break };
        let Some(sorted) = parse(&message) else { continue };
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
