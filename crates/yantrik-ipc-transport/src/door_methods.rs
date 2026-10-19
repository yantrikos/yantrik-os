//! The methods a mind may call at a door, held by the service's side of the socket (#411).
//!
//! Found 16 Oct 2026 on VM 541: the Mind could not read the desktop calendar, and the door served
//! every method a handler knew, so putting calendar-service on it would have handed a mind the raw
//! `calendar.create_event` and `calendar.delete_event` beside the graded surface. The policy is
//! `mind_door::mind_methods`, one table; this is where a door enforces it, in front of the
//! service's own handler, so a service admitted with `SurfaceOnly` never hears anything else from
//! a mind, whatever its handler would have answered.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use yantrik_ipc_contracts::email::ServiceError;

use crate::mind_door::{MindMethods, SURFACE_METHODS};
use crate::server::{PeerCred, ServiceHandler};
use crate::RPC_METHOD_NOT_FOUND;

/// How many refused method names one door remembers, to log each once. A mind writes the names,
/// so the set is bounded: past this, one more line says the rest go unlogged.
const REMEMBERED: usize = 64;

/// The longest method name a refusal repeats back. A mind wrote it.
const NAME_CHARS: usize = 80;

/// `inner`, as a mind on a door with `methods` may reach it. `Every` is `inner` itself: the
/// harness and the app surfaces are served exactly as before.
pub fn hold_to(methods: MindMethods, inner: Arc<dyn ServiceHandler>) -> Arc<dyn ServiceHandler> {
    match methods {
        MindMethods::Every => inner,
        MindMethods::SurfaceOnly => Arc::new(MethodDoor { inner, methods, refused: Mutex::new(Refused::default()) }),
    }
}

#[derive(Default)]
struct Refused {
    names: HashSet<String>,
    overflowed: bool,
}

/// A handler that answers a method its door's policy does not admit with a refusal, and passes
/// every other on.
struct MethodDoor {
    inner: Arc<dyn ServiceHandler>,
    methods: MindMethods,
    refused: Mutex<Refused>,
}

impl MethodDoor {
    /// Log a refusal once per method name, so a mind retrying the same call does not fill the
    /// journal and the first try is still on record.
    fn log_once(&self, method: &str, peer: Option<PeerCred>) {
        let mut refused = self.refused.lock().unwrap_or_else(|e| e.into_inner());
        if refused.names.contains(method) {
            return;
        }
        if refused.names.len() >= REMEMBERED {
            if !refused.overflowed {
                refused.overflowed = true;
                tracing::warn!(service = self.inner.service_id(), "a mind called more refused methods at the mind door; the rest are refused without a line each");
            }
            return;
        }
        refused.names.insert(method.to_string());
        tracing::warn!(
            service = self.inner.service_id(),
            method = %method,
            caller_pid = peer.map(|p| p.pid).unwrap_or(0),
            caller_uid = peer.map(|p| p.uid).unwrap_or(0),
            "refused a mind a method this door does not serve to minds (#411); logged once per method"
        );
    }
}

/// A method name a mind wrote, as one short plain line.
fn shown(method: &str) -> String {
    let line = crate::plain_text::one_line(method);
    match line.char_indices().nth(NAME_CHARS) {
        Some((at, _)) => format!("{}…", &line[..at]),
        None => line,
    }
}

impl ServiceHandler for MethodDoor {
    fn service_id(&self) -> &str {
        self.inner.service_id()
    }

    fn handle(&self, method: &str, params: serde_json::Value) -> Result<serde_json::Value, ServiceError> {
        self.handle_from(method, params, None)
    }

    fn handle_from(
        &self,
        method: &str,
        params: serde_json::Value,
        peer: Option<PeerCred>,
    ) -> Result<serde_json::Value, ServiceError> {
        if self.methods.admits(method) {
            return self.inner.handle_from(method, params, peer);
        }
        let method = shown(method);
        self.log_once(&method, peer);
        // Method-not-found, the protocol's answer for a name this socket does not serve to this
        // caller: an application answer, so a client's circuit breaker stays shut, and a mind is
        // told what it may call instead.
        Err(ServiceError {
            code: RPC_METHOD_NOT_FOUND,
            message: format!(
                "`{method}` is not served to a mind at the {} door: a mind may call {} here, and \
                 nothing else (#411). Nothing was run.",
                self.inner.service_id(),
                SURFACE_METHODS.join(", ")
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Echo;

    impl ServiceHandler for Echo {
        fn service_id(&self) -> &str {
            "calendar"
        }
        fn handle(&self, method: &str, _: serde_json::Value) -> Result<serde_json::Value, ServiceError> {
            Ok(serde_json::json!(method))
        }
    }

    #[test]
    fn a_surface_only_door_passes_the_surface_and_refuses_the_rest_by_name() {
        let door = hold_to(MindMethods::SurfaceOnly, Arc::new(Echo));
        for method in SURFACE_METHODS {
            assert_eq!(door.handle_from(method, serde_json::json!({}), None).unwrap(), method);
        }
        for method in ["calendar.delete_event", "calendar.create_event", "calendar.events", "anything"] {
            let err = door.handle_from(method, serde_json::json!({}), None).unwrap_err();
            assert_eq!(err.code, RPC_METHOD_NOT_FOUND, "{method}");
            assert!(err.message.starts_with(&format!("`{method}` is not served to a mind at the calendar door")), "{}", err.message);
            assert!(err.message.contains("app.describe, app.act, app.explain"), "{}", err.message);
        }
    }

    #[test]
    fn an_every_door_is_the_handler_itself() {
        let inner: Arc<dyn ServiceHandler> = Arc::new(Echo);
        let door = hold_to(MindMethods::Every, inner.clone());
        assert!(Arc::ptr_eq(&door, &inner), "harness and app-* doors are served exactly as before");
    }

    #[test]
    fn a_refused_name_is_repeated_back_as_one_short_line_and_remembered_within_bounds() {
        let long = format!("x\n{}", "y".repeat(500));
        let shown = shown(&long);
        assert!(!shown.contains('\n') && shown.chars().count() == NAME_CHARS + 1, "{shown}");
        let door = MethodDoor { inner: Arc::new(Echo), methods: MindMethods::SurfaceOnly, refused: Mutex::new(Refused::default()) };
        for n in 0..(REMEMBERED + 10) {
            let _ = door.handle_from(&format!("m{n}"), serde_json::json!({}), None);
            let _ = door.handle_from(&format!("m{n}"), serde_json::json!({}), None);
        }
        let refused = door.refused.lock().unwrap();
        assert_eq!(refused.names.len(), REMEMBERED);
        assert!(refused.overflowed);
    }
}
