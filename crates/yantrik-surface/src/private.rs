//! Private mode, as a surface answers it (#924).
//!
//! While the person has Private mode on (`yantrik_ipc_transport::privacy`), no agent sees what an
//! app holds. The mind door refuses every request outright, but the person's own socket cannot
//! tell an agent from the person: a harness running as the person sends `describe` with no token,
//! and so does `yos`. So every surface answers `app.describe` in Private mode with what it is and
//! nothing of what it holds:
//!
//! - its id, its actions and their grades, as always: the shell builds approval cards from them,
//!   and an action's declaration is the author's, not the person's;
//! - `"private": true`, beside `app` and inside `state`;
//! - [`PRIVATE_SUMMARY`] in place of the app's own line;
//! - what the framework knows about its windows (`open`, `windows_open`), for a surface that has
//!   them, so the shell and a Mind deciding whether to wait still know the app is there;
//! - and of the app's own state, only the keys it listed as not personal
//!   ([`Registry::show_in_private`]), each by name. None by default.
//!
//! The call is answered, not refused: a refusal would read as "the app is gone". The revision is
//! the private view's, so it moves only when what is shown moves, and an `act`'s reply carries the
//! same view, so no method reads around it. Nothing is cached: whoever serves the call reads
//! Private mode for that call (the window's RPC thread, a service's worker), and an in-process
//! read asks the registry's privacy source each time.

use serde_json::{Map, Value};
use yantrik_ipc_contracts::control_surface::View;
use yantrik_ipc_contracts::email::ServiceError;

use crate::call::refusal;
use crate::context::{off_the_reactor, Later};
use crate::registry::Registry;

/// The line every surface's `summary` is while the person is in Private mode.
pub const PRIVATE_SUMMARY: &str = "Private mode is on; this app's contents are hidden.";

/// The keys the private view writes itself. An app cannot list one of these as a key of its own
/// to show: the private view's word for them is the only one.
pub const RESERVED_KEYS: &[&str] = &["private", "open", "windows_open"];

/// The keys of [`RESERVED_KEYS`] a surface's presence supplies: what is known of its windows.
const WINDOW_KEYS: [&str; 2] = ["open", "windows_open"];

/// Beside a withheld `result`: why it is `null`.
pub const WITHHELD: &str = "Private mode is on; what this action answered is not shown to this caller.";

/// A handler's refusal, as a caller the kernel does not vouch for reads it in Private mode.
pub const WITHHELD_REFUSAL: &str =
    "PRIVATE: the app refused this call, and while the person has Private mode on its reason is not shown to this caller.";

/// Who one reply is for: the pid the kernel stamped on the call, and the surface it is to — a
/// listed reader is vouched for toward its own surface only (`PRIVATE_READERS`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplyTo<'a> {
    pub pid: Option<i32>,
    pub surface: &'a str,
}

/// What one `app.act` reply may carry of the app (#924; #926 review, M1).
///
/// In Private mode every reply carries the private view. The action's own `result`, and a
/// handler's own refusal, reach only a caller the kernel vouches for
/// (`yantrik_ipc_transport::private_owner::reads_while_private`: the shell, the app's own installed
/// program, the short list beside it). Everyone else — a harness the person started by hand, a
/// script, `yos` — is told the action ran, or was refused, and nothing of what it said.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hold {
    /// The person is not private: the reply is the app's own.
    Open,
    /// Private, and the kernel vouches for the caller: the private view, the action's own answer.
    Vouched,
    /// Private, and nobody vouches for the caller: the private view, and nothing the action said.
    Withheld,
}

impl Hold {
    pub fn of(private: bool, vouched: bool) -> Hold {
        match (private, vouched) {
            (false, _) => Hold::Open,
            (true, true) => Hold::Vouched,
            (true, false) => Hold::Withheld,
        }
    }

    /// For a door: Private mode as it read it, and who the reply is for. The caller is looked up
    /// (`/proc`) only while the person is private.
    pub fn for_caller(private: bool, to: ReplyTo<'_>) -> Hold {
        let vouched = private
            && to.pid.is_some_and(|pid| yantrik_ipc_transport::private_owner::reads_while_private(pid, to.surface));
        Hold::of(private, vouched)
    }

    pub fn is_private(self) -> bool {
        self != Hold::Open
    }

    /// A handler's refusal as this caller may read it.
    pub fn refusal(self, why: String) -> String {
        if self == Hold::Withheld {
            WITHHELD_REFUSAL.to_string()
        } else {
            why
        }
    }

    /// The reply as this caller may read it: marked while private, and for [`Hold::Withheld`] its
    /// `result` (when it has one) withheld and said so.
    pub fn seal(self, reply: &mut Value) {
        let Some(map) = reply.as_object_mut() else { return };
        if self == Hold::Open {
            map.remove("private");
            return;
        }
        map.insert("private".to_string(), Value::Bool(true));
        if self == Hold::Withheld && map.contains_key("result") {
            map.insert("result".to_string(), Value::Null);
            map.insert("withheld".to_string(), WITHHELD.into());
        }
    }
}

/// Where a registry's in-process reads learn whether the person is in Private mode. Not consulted
/// by a door that read the file itself and said so ([`Registry::describe_in`]).
pub type PrivacySource = dyn Fn() -> bool + Send + Sync;

/// The view a surface publishes while the person is in Private mode: the neutral line, the
/// marker, what `presence` says of its windows, and the app's own `shown` keys, nothing else.
///
/// `own` is the app's live view, read only when it listed keys to show; `presence` is the
/// framework's, and only its window keys are taken from it.
pub fn private_view(own: Option<&View>, shown: &[String], presence: Option<&View>) -> View {
    let mut state = Map::new();
    if let Some(own) = own.and_then(|v| v.state.as_object()) {
        for key in shown {
            if let Some(value) = own.get(key) {
                state.insert(key.clone(), value.clone());
            }
        }
    }
    if let Some(presence) = presence.and_then(|v| v.state.as_object()) {
        for key in WINDOW_KEYS {
            if let Some(value) = presence.get(key) {
                state.insert(key.to_string(), value.clone());
            }
        }
    }
    state.insert("private".to_string(), Value::Bool(true));
    View::new(PRIVATE_SUMMARY).state(Value::Object(state))
}

/// What is wrong with listing `key` as one to show in Private mode, or `None`.
pub fn show_problem(key: &str) -> Option<String> {
    if key.trim().is_empty() {
        return Some("an empty key cannot be shown in Private mode".into());
    }
    RESERVED_KEYS.contains(&key).then(|| {
        format!("`{key}` is written by the private view itself; an app cannot show its own under that name")
    })
}

impl<D: ?Sized, H: ?Sized> Registry<D, H> {
    /// Keep `key` of this surface's own `describe` state in the answer while the person is in
    /// Private mode. Default-deny: a key not listed here is left out. List a key only when its
    /// whole value says nothing about the person — a unit, a CPU figure — never a title, a name,
    /// a path or a count of their things. A reserved or empty key is logged and not listed.
    pub fn show_in_private(&mut self, key: &str) {
        if let Some(problem) = show_problem(key) {
            tracing::warn!(app = %self.app_id(), "{problem}");
            return;
        }
        if !self.shown_in_private.iter().any(|k| k == key) {
            self.shown_in_private.push(key.to_string());
        }
    }

    /// What this surface says of its windows in Private mode: a view whose state carries `open`
    /// and `windows_open`, read on every private answer. Every other key of it is dropped. A
    /// window app's framework sets this; a service has no window and sets nothing.
    pub fn set_presence(&mut self, f: Box<D>) {
        self.presence = Some(f);
    }

    /// The keys this surface shows in Private mode, in the order they were listed.
    pub fn shown_in_private(&self) -> &[String] {
        &self.shown_in_private
    }

    /// One more reason this surface's in-process reads ([`Registry::describe`], [`Registry::act`])
    /// take the person to be private: a test's pin on its own app. It can only add — the file the
    /// shell publishes is read whatever the source says — so nothing installed here makes a
    /// surface less private than the person made it. Public because the framework that pins is
    /// another crate; a door never consults it, it reads the file itself and passes the answer
    /// to [`Registry::describe_in`] and [`Registry::act_in`].
    pub fn set_privacy_source(&mut self, source: Box<PrivacySource>) {
        self.privacy = Some(source);
    }

    /// Whether the person is in Private mode, as this surface's in-process reads see it now: the
    /// file, or the source when it says so.
    pub fn private_now(&self) -> bool {
        self.privacy.as_ref().is_some_and(|source| source()) || yantrik_ipc_transport::privacy::is_private()
    }
}

impl<D, H> Registry<D, H>
where
    D: ?Sized + Fn() -> View,
    H: ?Sized + Fn(&Value) -> Result<Value, String>,
{
    /// What this surface publishes now: its live view, or in Private mode the private one. In
    /// Private mode the app's own `describe` runs only when it listed keys to show.
    pub fn published(&self, private: bool) -> View {
        if !private {
            return self.snapshot();
        }
        let own = (!self.shown_in_private.is_empty()).then(|| self.snapshot());
        let presence = self.presence.as_ref().map(|f| f());
        private_view(own.as_ref(), &self.shown_in_private, presence.as_ref())
    }

    /// The reply to `app.describe`, with Private mode as the door read it for this call.
    pub fn describe_in(&self, private: bool) -> Value {
        let view = self.published(private);
        let mut answer = yantrik_ipc_contracts::control_surface::describe_json(self.app_id(), &view, &self.specs());
        if private {
            mark(&mut answer);
        }
        answer
    }
}

/// [`crate::finish_later`], under Private mode as it is once the work is done: a deferred act can
/// run for minutes, and the person may have turned Private mode on meanwhile. `to` is who the
/// reply is for, for the [`Hold`] on its answer; `read` is handed whether the person is private and
/// returns the view as published under it.
pub fn finish_later_published(
    envelope: Value,
    later: Later,
    to: ReplyTo<'_>,
    read: impl FnOnce(bool) -> Option<View>,
) -> Result<Value, ServiceError> {
    let outcome = off_the_reactor(later);
    let hold = Hold::for_caller(yantrik_ipc_transport::privacy::is_private(), to);
    let result = outcome.map_err(|why| refusal(hold.refusal(why)))?;
    let mut reply = envelope;
    reply["result"] = result;
    if let Some(view) = read(hold.is_private()) {
        reply["revision"] = view.revision().into();
        reply["summary"] = view.summary.into();
        reply["state"] = view.state;
    }
    hold.seal(&mut reply);
    Ok(reply)
}

/// A reply built under `was_private`, held to Private mode as it is once the reply is built: the
/// person may have turned it on while the call waited for, or ran on, the app's thread. If it came
/// on, the app's view in the reply is replaced with the private one, which `republish` reads
/// again — or, when the app does not answer that, the private view with nothing of the app — and
/// the reply is marked. Every door calls this after its hop, as [`finish_later_published`] does
/// after work finished later (#926 review, L1).
pub fn recheck_private(
    was_private: bool,
    to: ReplyTo<'_>,
    reply: &mut Value,
    republish: impl FnOnce() -> Option<View>,
) {
    if was_private || !yantrik_ipc_transport::privacy::is_private() {
        return;
    }
    let view = republish().unwrap_or_else(|| private_view(None, &[], None));
    if let Some(map) = reply.as_object_mut() {
        if map.contains_key("summary") || map.contains_key("state") {
            map.insert("revision".to_string(), view.revision().into());
            map.insert("summary".to_string(), view.summary.into());
            map.insert("state".to_string(), view.state);
        }
    }
    Hold::for_caller(true, to).seal(reply);
}

/// A refusal built under `was_private`, held to Private mode as it is once it is built — the
/// refusal side of [`recheck_private`]: for a caller nobody vouches for, the neutral sentence.
pub fn recheck_refusal(was_private: bool, to: ReplyTo<'_>, why: String) -> String {
    if was_private || !yantrik_ipc_transport::privacy::is_private() {
        return why;
    }
    Hold::for_caller(true, to).refusal(why)
}

/// `"private": true` beside `app` in a reply.
pub fn mark(answer: &mut Value) {
    if let Some(map) = answer.as_object_mut() {
        map.insert("private".to_string(), Value::Bool(true));
    }
}
