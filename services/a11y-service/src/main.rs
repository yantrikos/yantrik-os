//! Yantrik Accessibility — reading, and driving, windows we did not write.
//!
//! # Where this sits
//!
//! There are now three ways for the companion to know what is on screen, and they should be tried
//! in this order:
//!
//!   1. `app.describe` — our own apps, which publish their state directly. Exact, and free.
//!   2. this — anything built with GTK, Qt, Chromium or Firefox, which publish their widget tree
//!      over AT-SPI because screen readers need it. Also free, also exact, no GPU.
//!   3. `analyze_screen` — a screenshot and a vision model, for the rest.
//!
//! I wrote the rule earlier as *semantic for ours, visual for theirs*. Having looked, that was too
//! pessimistic: most of theirs is semantic too, and has been for twenty years. Vision belongs
//! third, not second.
//!
//! # What it serves
//!
//! ```text
//! a11y.windows  { }                       → [{ app, title, role, pid, id }]
//! a11y.describe { window }                → { summary, elements }
//! a11y.act      { element, action }       → { did }
//! ```
//!
//! `a11y.act` is the part worth the trouble. AT-SPI's action interface is how a screen reader
//! presses a button, so the companion can press one too — by name, on a window that may not even
//! be on top, with no synthetic pointer and no guess at coordinates. It is the same argument as
//! the app control surface, extended to software we did not write.
//!
//! # Threading
//!
//! zbus is async and [`ServiceHandler::handle`] is not. Rather than nest a runtime inside a
//! runtime — which panics — the accessibility connection lives on its own thread with its own
//! current-thread runtime, and the handler talks to it over a channel. The same shape as the
//! companion bridge, for the same reason.
//!
//! # Trust
//!
//! Every string here was written by another application: window titles, labels, the text in a
//! field. They are content, not instruction, and nothing in this service interprets them. That
//! matters more than usual because the consumer is a language model, and a window titled
//! "ignore your previous instructions" is a thing anyone can make.

mod atspi;

use std::sync::mpsc::{self, Sender};
use std::time::Duration;

use yantrik_service_sdk::prelude::*;

/// How long a caller waits for the accessibility bus. Walking a large tree costs a D-Bus
/// round-trip per node, so a browser window is not instant, but nor should it be unbounded.
const TIMEOUT: Duration = Duration::from_secs(20);

enum Request {
    Windows(Sender<Result<serde_json::Value, String>>),
    Describe(String, Sender<Result<serde_json::Value, String>>),
    Act(String, String, Sender<Result<serde_json::Value, String>>),
    Status(Sender<Result<serde_json::Value, String>>),
}

fn main() {
    yantrik_service_sdk::init_tracing("a11y");

    let (tx, rx) = mpsc::channel::<Request>();
    std::thread::Builder::new()
        .name("a11y-bus".into())
        .spawn(move || bus_thread(rx))
        .expect("spawn a11y-bus thread");

    ServiceBuilder::new("a11y").handler(A11y { tx }).run();
}

/// Owns the connection and answers one request at a time.
///
/// Serialised on purpose: AT-SPI object handles are handed out by this thread and resolved by it,
/// and two concurrent walks of the same tree would race for no benefit — the cost is D-Bus
/// round-trips, which do not parallelise usefully against a single application anyway.
fn bus_thread(rx: mpsc::Receiver<Request>) {
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            tracing::error!(error = %e, "No runtime; accessibility is unavailable");
            return;
        }
    };

    // Connected lazily, and retried. Accessibility is often started *after* the session — the
    // first application with the bridge loaded brings the bus up — so refusing at startup would
    // make this permanently unavailable on a machine where it would have worked a minute later.
    let mut connection: Option<atspi::Atspi> = None;

    while let Ok(request) = rx.recv() {
        runtime.block_on(async {
            if connection.is_none() {
                match atspi::Atspi::connect().await {
                    Ok(c) => {
                        tracing::info!("Connected to the accessibility bus");
                        connection = Some(c);
                    }
                    Err(e) => {
                        let reply = Err(e);
                        match request {
                            Request::Windows(tx)
                            | Request::Describe(_, tx)
                            | Request::Act(_, _, tx)
                            | Request::Status(tx) => {
                                let _ = tx.send(reply);
                            }
                        }
                        return;
                    }
                }
            }
            let Some(atspi) = connection.as_mut() else { return };

            match request {
                Request::Windows(tx) => {
                    let reply = atspi.windows().await.map(|windows| {
                        serde_json::json!({ "windows": windows, "count": windows.len() })
                    });
                    let _ = tx.send(reply);
                }
                Request::Describe(id, tx) => {
                    let reply = atspi.describe(&id).await.map(|(summary, elements)| {
                        serde_json::json!({ "summary": summary, "elements": elements })
                    });
                    let _ = tx.send(reply);
                }
                Request::Act(element, action, tx) => {
                    let reply = atspi
                        .act(&element, &action)
                        .await
                        .map(|did| serde_json::json!({ "did": did }));
                    let _ = tx.send(reply);
                }
                Request::Status(tx) => {
                    // Reached only when the connection exists, since a failure answers above.
                    let _ = tx.send(Ok(serde_json::json!({
                        "available": true,
                        "note": "GTK, Qt, Chromium and Firefox publish their widget trees here; \
                                 an application without an accessibility bridge will not appear.",
                    })));
                }
            }
        });
    }
}

struct A11y {
    tx: Sender<Request>,
}

impl A11y {
    fn ask(
        &self,
        build: impl FnOnce(Sender<Result<serde_json::Value, String>>) -> Request,
    ) -> Result<serde_json::Value, ServiceError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx.send(build(reply_tx)).map_err(|_| ServiceError {
            code: -32000,
            message: "the accessibility thread is not running".into(),
        })?;
        reply_rx
            .recv_timeout(TIMEOUT)
            .map_err(|_| ServiceError {
                code: -32000,
                message: format!("the accessibility bus did not answer within {}s", TIMEOUT.as_secs()),
            })?
            .map_err(|message| ServiceError { code: -32000, message })
    }
}

impl ServiceHandler for A11y {
    fn service_id(&self) -> &str {
        "a11y"
    }

    fn handle(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, ServiceError> {
        match method {
            "a11y.windows" => self.ask(Request::Windows),

            "a11y.describe" => {
                let window = params["window"].as_str().unwrap_or("").trim().to_string();
                if window.is_empty() {
                    return Err(ServiceError {
                        code: -32602,
                        message: "describe needs a `window`; call a11y.windows to get one".into(),
                    });
                }
                self.ask(|tx| Request::Describe(window, tx))
            }

            "a11y.act" => {
                let element = params["element"].as_str().unwrap_or("").trim().to_string();
                let action = params["action"].as_str().unwrap_or("").trim().to_string();
                if element.is_empty() || action.is_empty() {
                    return Err(ServiceError {
                        code: -32602,
                        message: "act needs an `element` and an `action`; a11y.describe lists both"
                            .into(),
                    });
                }
                tracing::info!(element = %element, action = %action, "a11y.act");
                self.ask(|tx| Request::Act(element, action, tx))
            }

            // Cheap enough to call before offering to look at all, and it distinguishes "nothing
            // is open" from "there is no accessibility bus here", which are different problems.
            "a11y.status" => self.ask(Request::Status),

            other => Err(ServiceError {
                code: -32601,
                message: format!(
                    "unknown method `{other}`; this service serves \
                     a11y.windows, a11y.describe, a11y.act, a11y.status"
                ),
            }),
        }
    }
}
