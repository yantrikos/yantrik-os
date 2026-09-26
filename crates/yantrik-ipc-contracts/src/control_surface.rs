//! The `app.describe` / `app.act` envelope, shared by every surface that speaks it.
//!
//! Two kinds of surface publish this vocabulary and they must produce byte-identical output, or
//! a caller reading one and a caller reading the other disagree about what an app is and what it
//! grades:
//!
//! * A **Slint window** (the shell, notes, email) answers from a live view-model on the UI
//!   thread. `yantrik-app-runtime::control` owns the thread hand-off.
//! * A **standalone service** (weather, system-monitor, notifications) answers synchronously
//!   inside `ServiceHandler::handle`, with no Slint and no UI thread.
//!
//! Both dispatch through `yantrik-surface`, which holds the registry, the argument checks and the
//! revision guard with no UI dependency. Both need the same `View`, the same action schema, and
//! the same revision hash. Those are pure data with no dependency on Slint or tokio, so they live
//! here — the one crate every side already depends on — rather than in the Slint runtime, which
//! a headless service must not pull in. `yantrik-surface` and `yantrik-app-runtime::control`
//! re-export these types, so existing `control::View` / `control::Action` callers are unaffected.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// One app's account of itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct View {
    /// One line a person could read: `Notes — editing "Kernel asks", 412 words, unsaved`.
    ///
    /// Present so a caller surveying every open window pays one line per app instead of parsing
    /// sixteen state objects.
    pub summary: String,
    /// The structured view-model. An object; keys are the app's own vocabulary.
    pub state: serde_json::Value,
}

impl View {
    pub fn new(summary: impl Into<String>) -> Self {
        Self { summary: summary.into(), state: serde_json::json!({}) }
    }

    /// Add one field to the state object.
    pub fn with(mut self, key: &str, value: impl Into<serde_json::Value>) -> Self {
        if let Some(map) = self.state.as_object_mut() {
            map.insert(key.to_string(), value.into());
        }
        self
    }

    /// Replace the whole state object at once, for an app that builds it elsewhere.
    pub fn state(mut self, state: serde_json::Value) -> Self {
        self.state = state;
        self
    }

    /// A short fingerprint of everything this view reports.
    ///
    /// Not a version counter: nothing increments it, and two states can only ever be compared for
    /// difference, never ordered. That is all a caller needs — the question is only ever *has
    /// what I looked at changed since I looked* — and a hash of the answer settles it without
    /// asking every app to maintain a counter it would eventually forget to bump.
    ///
    /// It deliberately excludes `actions`, which are fixed for the life of the app: including
    /// them would drag a constant through every comparison and change nothing.
    pub fn revision(&self) -> String {
        // FNV-1a, written out rather than `DefaultHasher`, because this value crosses a socket and
        // turns up in logs: it has to mean the same thing on both sides of the wire and in
        // tomorrow's build, which `DefaultHasher` explicitly does not promise.
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        let mut eat = |bytes: &[u8]| {
            for b in bytes {
                hash ^= *b as u64;
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        };
        eat(self.summary.as_bytes());
        // A separator, so a summary ending mid-word cannot collide with a state beginning there.
        eat(&[0]);
        // `to_string` on a `serde_json::Value` renders object keys in sorted order, so the same
        // state always produces the same bytes regardless of the order the app inserted them.
        eat(self.state.to_string().as_bytes());
        format!("{hash:016x}")
    }
}

/// The JSON Schema types an argument can be declared with, in the words `describe` publishes.
///
/// `enum` is not among them because JSON Schema does not make it a type: an enum is a `string`
/// with a list of `values` beside it, which is how it is published and how it is checked.
pub const PARAM_TYPES: [&str; 6] = ["string", "number", "integer", "boolean", "array", "object"];

/// One argument of an action.
///
/// Published as a JSON Schema property, and checked by the dispatch against what a call carries:
/// an argument of the wrong type is refused with a sentence that names what was wanted and what
/// arrived, before the handler runs. The handler can rely on the declaration.
///
/// | constructor              | published                                        | accepts                    |
/// |--------------------------|--------------------------------------------------|----------------------------|
/// | [`Param::text`]          | `{"type":"string"}`                              | a JSON string              |
/// | [`Param::number`]        | `{"type":"number"}`                              | any JSON number            |
/// | [`Param::integer`]       | `{"type":"integer"}`                             | a number written whole: `3`, not `3.0` |
/// | [`Param::flag`]          | `{"type":"boolean"}`                             | `true` or `false`          |
/// | [`Param::one_of`]        | `{"type":"string","enum":[…]}`                   | one of the listed strings  |
/// | [`Param::array`]         | `{"type":"array","items":{"type":…}}`            | a JSON array, every item of the item type |
/// | [`Param::object`]        | `{"type":"object"}`                              | a JSON object              |
///
/// Every property also carries `description`, and `default` when one is declared. `null` for an
/// optional argument is the same as leaving it out.
#[derive(Clone, Debug)]
pub struct Param {
    pub name: String,
    /// JSON Schema type: one of [`PARAM_TYPES`].
    pub kind: &'static str,
    pub required: bool,
    pub description: String,
    /// The only values a `string` argument may take, published as `enum`. Empty for any string.
    pub values: Vec<String>,
    /// Whether this argument was declared with [`Param::one_of`], even with no values listed.
    /// `values` alone cannot say that: an `one_of` with an empty list is indistinguishable from
    /// a plain string, and only this marker lets the surface tell its author that the list
    /// leaves nothing to be given, the way the Python SDK refuses one at declaration.
    pub enumerated: bool,
    /// The type of every item of an `array` argument, published as `items`. `None` otherwise.
    pub items: Option<&'static str>,
    /// What the handler is given when the caller leaves this argument out, published as
    /// `default`. Declaring one makes the argument optional.
    pub default: Option<serde_json::Value>,
}

impl Param {
    fn of(name: &str, kind: &'static str) -> Self {
        Self {
            name: name.into(),
            kind,
            required: true,
            description: String::new(),
            values: Vec::new(),
            enumerated: false,
            items: None,
            default: None,
        }
    }
    pub fn text(name: &str) -> Self {
        Self::of(name, "string")
    }
    pub fn number(name: &str) -> Self {
        Self::of(name, "number")
    }
    /// A whole number: a pid, a count, an index. `3.0` is refused, because a handler reading it
    /// with `as_u64` would find nothing there and blame the caller for leaving it out.
    pub fn integer(name: &str) -> Self {
        Self::of(name, "integer")
    }
    pub fn flag(name: &str) -> Self {
        Self::of(name, "boolean")
    }
    /// One of a fixed set of strings, published as JSON Schema `enum` so a caller can see the
    /// choices before it guesses, and refused by the dispatch with the choices named when it
    /// guesses anyway.
    pub fn one_of(name: &str, values: &[&str]) -> Self {
        let mut p = Self::of(name, "string");
        p.enumerated = true;
        p.values = values.iter().map(|v| v.to_string()).collect();
        p
    }
    /// A list whose every item is `item` — one of [`PARAM_TYPES`] except `array`: `"string"`,
    /// `"number"`, `"integer"`, `"boolean"` or `"object"`.
    pub fn array(name: &str, item: &'static str) -> Self {
        let mut p = Self::of(name, "array");
        p.items = Some(item);
        p
    }
    /// A JSON object, handed to the handler as it arrived. For arguments that are themselves a
    /// set of named values — another action's arguments, a settings patch.
    pub fn object(name: &str) -> Self {
        Self::of(name, "object")
    }
    /// Mark this argument optional. The handler must cope with it being absent.
    pub fn optional(mut self) -> Self {
        self.required = false;
        self
    }
    /// What the handler is given when the caller leaves this argument out. Makes it optional:
    /// an argument with a default is one the caller may omit.
    pub fn default(mut self, value: impl Into<serde_json::Value>) -> Self {
        self.default = Some(value.into());
        self.required = false;
        self
    }
    pub fn describe(mut self, description: &str) -> Self {
        self.description = description.into();
        self
    }

    /// This argument as a JSON Schema property.
    pub fn schema(&self) -> serde_json::Value {
        let mut property = serde_json::json!({ "type": self.kind, "description": self.description });
        if !self.values.is_empty() {
            property["enum"] = serde_json::json!(self.values);
        }
        if let Some(item) = self.items {
            property["items"] = serde_json::json!({ "type": item });
        }
        if let Some(default) = &self.default {
            property["default"] = default.clone();
        }
        property
    }
}

/// The sentence an app says about ONE call of one of its actions, with that call's own
/// arguments (#137).
///
/// An approval card used to show the action's published purpose — the same paragraph for every
/// call of it — and the arguments, with nothing in between: `studio.set_backend kind=openai-images`
/// and `kind=fake` carried the same words although one sends every later prompt to a hosted
/// service and the other keeps it on the machine. Only the app knows which is which, so the app
/// says it, per call, and the shell asks when it builds the card (`app.explain`).
///
/// An `Arc` around the closure because [`Action`] is `Clone` (a registry clones its specs to
/// publish a regrade) and must stay `Send + Sync` (a service shares one surface across its socket
/// workers) — a bare `dyn Fn` is neither, and for the same reason `Debug` is written by hand.
#[derive(Clone)]
pub struct Explainer(Arc<dyn Fn(&serde_json::Value) -> String + Send + Sync>);

impl Explainer {
    /// What the app says about one call with these arguments. Empty when the app has nothing to
    /// say about THIS call — an honest absence, not a failure, and the card draws no line.
    pub fn sentence(&self, args: &serde_json::Value) -> String {
        (self.0)(args)
    }
}

impl std::fmt::Debug for Explainer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Explainer(..)")
    }
}

/// One thing an app can be asked to do.
#[derive(Clone, Debug)]
pub struct Action {
    pub name: String,
    pub description: String,
    pub params: Vec<Param>,
    /// How much damage this can do, in the companion's vocabulary:
    /// `safe`, `standard`, `sensitive`, `dangerous`.
    ///
    /// Declared per action rather than per surface because apps do not have one risk level:
    /// reading which note is open and killing a process arrive through the same door. The
    /// caller compares this against its own ceiling; the app states the fact.
    pub permission: &'static str,
    /// Whether the handler finishes the work or only starts it.
    ///
    /// Declared by the app because the app is the only thing that knows. A handler that hands off
    /// to a worker returns long before the result exists, and a caller told only that the call
    /// succeeded would report a build as finished the moment it began.
    pub deferred: bool,
    /// How long a call usually takes to answer, in seconds, when the app knows it is more than a
    /// moment: a render, an export, a command whose exit code is the answer. Published as
    /// `expected_seconds` so a caller can size its timeout to the action instead of guessing one
    /// number for every action on the machine. `None` — the default — says nothing, and a caller
    /// keeps its own.
    pub expected_seconds: Option<u32>,
    /// The sentence about ONE call of this action, with that call's own arguments, when the app
    /// can say one (#137). Published as `explains: true` — the fact, never the sentence, which
    /// depends on arguments `describe` does not have; a client that saw the flag asks for it with
    /// `app.explain` when it builds an approval card. `None` — the default — publishes exactly
    /// what the action always did, and the card is exactly what it was.
    pub explainer: Option<Explainer>,
}

impl Action {
    pub fn new(name: &str, description: &str) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            params: Vec::new(),
            // Steering someone's window is not free, so the floor is `standard`, not `safe`.
            permission: "standard",
            // Most actions are a property write and are finished when they return. The ones that
            // are not have to say so.
            deferred: false,
            expected_seconds: None,
            explainer: None,
        }
    }

    /// Declare how long a call to this usually takes to answer. See
    /// [`Action::expected_seconds`](Action#structfield.expected_seconds).
    pub fn expected_seconds(mut self, seconds: u32) -> Self {
        self.expected_seconds = Some(seconds);
        self
    }

    /// Declare the sentence this action says about one call of itself, with the arguments that
    /// call carries (#137). See [`Explainer`].
    ///
    /// The closure runs while an approval card is being built, on the app's own thread, so it
    /// must be cheap and total: read the arguments, answer one sentence — empty for a call there
    /// is nothing honest to say about. It decides nothing and must act on nothing: grades, gates
    /// and grants never read it, and what a person allows stays bound to the arguments alone.
    pub fn explain(
        mut self,
        f: impl Fn(&serde_json::Value) -> String + Send + Sync + 'static,
    ) -> Self {
        self.explainer = Some(Explainer(Arc::new(f)));
        self
    }

    pub fn arg(mut self, param: Param) -> Self {
        self.params.push(param);
        self
    }

    /// Declare this action riskier (or safer) than the default `standard`.
    ///
    /// Use `dangerous` for anything that destroys work or state a person cannot get back:
    /// killing a process, deleting a file, sending mail.
    pub fn risk(mut self, permission: &'static str) -> Self {
        self.permission = permission;
        self
    }

    /// Declare that this action only *starts* the work.
    ///
    /// Anything handed to a worker thread, sent over a network, or waiting on another process.
    /// The response then says `settles: later`, and the caller has to watch for the result rather
    /// than mistake the call for the result.
    pub fn defers(mut self) -> Self {
        self.deferred = true;
        self
    }

    /// The action as JSON Schema, so a caller can hand it to a model unmodified.
    pub fn schema(&self) -> serde_json::Value {
        let mut properties = serde_json::Map::new();
        let mut required = Vec::new();
        for p in &self.params {
            properties.insert(p.name.clone(), p.schema());
            if p.required {
                required.push(serde_json::Value::String(p.name.clone()));
            }
        }
        let mut schema = serde_json::json!({
            "name": self.name,
            "description": self.description,
            "permission": self.permission,
            "settles": if self.deferred { "later" } else { "on return" },
            "parameters": {
                "type": "object",
                "properties": serde_json::Value::Object(properties),
                "required": required,
            }
        });
        // Only when declared: an action that says nothing publishes exactly what it always did.
        if let Some(seconds) = self.expected_seconds {
            schema["expected_seconds"] = seconds.into();
        }
        // Only when declared, and only the FACT: the sentence itself belongs to `app.explain`,
        // because it depends on arguments `describe` never sees (#137).
        if self.explainer.is_some() {
            schema["explains"] = true.into();
        }
        schema
    }
}

/// The version of the surface protocol this envelope speaks: `docs/surface-protocol.md`.
///
/// Published in every `describe` so a client can tell what it is talking to. A describe with no
/// `protocol` is from before the protocol was written down; version 1 is what every surface on
/// this OS already did when it was, plus this key.
pub const PROTOCOL: u32 = 1;

/// Build the reply to `app.describe` for a service that serves its own socket.
///
/// The Slint path answers describe from a registry on the UI thread; a standalone service
/// computes its state inside `handle`. This gives the service the identical envelope — same
/// keys, same revision hash, same action schema — so `yos describe weather` and
/// `yos describe shell` read the same way, and the companion's permission guard grades a
/// service action exactly as it grades a window's.
pub fn describe_json(app_id: &str, view: &View, actions: &[Action]) -> serde_json::Value {
    serde_json::json!({
        "protocol": PROTOCOL,
        "app": app_id,
        "summary": view.summary,
        "state": view.state,
        "revision": view.revision(),
        "actions": actions.iter().map(Action::schema).collect::<Vec<_>>(),
    })
}

/// Build the reply to `app.act` for a standalone service.
///
/// Mirrors the Slint path's envelope so a caller cannot tell a service action from a window
/// action by its shape. `settled` is the service's to state: a handler that has finished the
/// work by the time it returns passes `true`; one that only kicked it off passes `false`.
///
/// `never `ok`, never `done``: `accepted` says the handler ran, `settled` says whether the work
/// finished. They are different questions and only the app can answer the second.
pub fn act_json(
    app_id: &str,
    action_id: &str,
    settled: bool,
    result: serde_json::Value,
    view: &View,
) -> serde_json::Value {
    serde_json::json!({
        "app": app_id,
        "action_id": action_id,
        "accepted": true,
        "settled": settled,
        "result": result,
        "revision": view.revision(),
        "summary": view.summary,
        "state": view.state,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revision_is_stable_across_key_order() {
        let a = View::new("s").with("x", 1).with("y", 2);
        let b = View::new("s").with("y", 2).with("x", 1);
        assert_eq!(a.revision(), b.revision());
    }

    #[test]
    fn revision_changes_with_state() {
        let a = View::new("s").with("x", 1);
        let b = View::new("s").with("x", 2);
        assert_ne!(a.revision(), b.revision());
    }

    #[test]
    fn schema_carries_permission_and_required() {
        let schema = Action::new("kill", "end a process")
            .risk("dangerous")
            .arg(Param::number("pid"))
            .schema();
        assert_eq!(schema["permission"], "dangerous");
        assert_eq!(schema["settles"], "on return");
        assert_eq!(schema["parameters"]["required"], serde_json::json!(["pid"]));
    }

    /// A revision pinned against the Python port, byte for byte.
    ///
    /// `sdk/python/yantrik_surface/wire.py` recomputes this hash in Python — the Python SDK, which
    /// Blender's addon is built on, because an addon inside somebody else's program cannot link
    /// this crate. `sdk/python/tests/test_revision.py` asserts the same vector with the same hex
    /// (and reads it out of this test, by this function's name), as do
    /// `tests/blender-core/test_wire.py` and `deploy/yantrik-os/surface-vectors.json`. The two
    /// implementations can only drift if one of them changes what it hashes, and whichever side
    /// moves, its test fails with this vector in the message. If this hash is ever deliberately
    /// changed, change it everywhere it is pinned in the same commit.
    #[test]
    fn revision_vector_shared_with_the_python_port() {
        let view = View::new("Blender — \"monkey.blend\", 3 objects, Cycles 1920x1080").state(
            serde_json::json!({
                "scene": "Scene",
                "file": "/tmp/monkey.blend",
                "unsaved": false,
                "objects": [
                    {
                        "name": "Suzanne",
                        "type": "MESH",
                        "location": [0.0, 0.0, 0.0],
                        "dimensions": [2.0, 2.0, 2.0]
                    }
                ],
                "objects_total": 3,
                "camera": { "name": "Camera", "location": [4.0, -4.0, 3.0] },
                "render": {
                    "engine": "cycles",
                    "resolution": "1920x1080",
                    "samples": 32,
                    "output": "/tmp/monkey.png"
                },
                "last_render": null,
                "notice": "",
                "background": true
            }),
        );
        assert_eq!(view.revision(), "6d6dd36469ee8664");
    }

    /// What an existing declaration publishes is byte for byte what it published before the
    /// richer types existed: `type` and `description`, nothing else, and neither optional key.
    #[test]
    fn the_three_original_types_publish_what_they_always_did() {
        let schema = Action::new("open", "Open")
            .arg(Param::text("title").describe("The title"))
            .arg(Param::number("zoom").optional())
            .arg(Param::flag("focus").optional())
            .schema();
        let props = &schema["parameters"]["properties"];
        assert_eq!(props["title"], serde_json::json!({"type": "string", "description": "The title"}));
        assert_eq!(props["zoom"], serde_json::json!({"type": "number", "description": ""}));
        assert_eq!(props["focus"], serde_json::json!({"type": "boolean", "description": ""}));
        assert!(schema.get("expected_seconds").is_none(), "{schema}");
        assert!(schema.get("explains").is_none(), "{schema}");
    }

    /// Each richer type as the JSON Schema a model is handed and the Python port mirrors.
    #[test]
    fn the_richer_types_publish_as_json_schema() {
        let schema = Action::new("export", "Export the document")
            .arg(Param::integer("page"))
            .arg(Param::one_of("format", &["pdf", "png"]).default("pdf"))
            .arg(Param::array("tags", "string").optional())
            .arg(Param::object("options").optional())
            .arg(Param::integer("dpi").default(150).describe("Dots per inch"))
            .expected_seconds(20)
            .schema();
        let props = &schema["parameters"]["properties"];
        assert_eq!(props["page"], serde_json::json!({"type": "integer", "description": ""}));
        assert_eq!(
            props["format"],
            serde_json::json!({"type": "string", "description": "", "enum": ["pdf", "png"], "default": "pdf"})
        );
        assert_eq!(
            props["tags"],
            serde_json::json!({"type": "array", "description": "", "items": {"type": "string"}})
        );
        assert_eq!(props["options"], serde_json::json!({"type": "object", "description": ""}));
        assert_eq!(
            props["dpi"],
            serde_json::json!({"type": "integer", "description": "Dots per inch", "default": 150})
        );
        // A default makes an argument optional; only `page` is required.
        assert_eq!(schema["parameters"]["required"], serde_json::json!(["page"]));
        assert_eq!(schema["expected_seconds"], 20);
    }

    #[test]
    fn describe_envelope_has_the_expected_keys() {
        let view = View::new("Weather — 21°C in Dallas").with("temp", 21);
        let actions = [Action::new("refresh", "refetch")];
        let out = describe_json("weather", &view, &actions);
        assert_eq!(out["protocol"], 1, "a client can tell which protocol it is reading");
        assert_eq!(out["app"], "weather");
        assert_eq!(out["summary"], "Weather — 21°C in Dallas");
        assert_eq!(out["state"]["temp"], 21);
        assert_eq!(out["actions"][0]["name"], "refresh");
        assert!(out["revision"].as_str().unwrap().len() == 16);
    }

    /// An action that can say what one call of it does publishes the fact — and only the fact:
    /// the sentence depends on arguments `describe` never has, so it travels by `app.explain`
    /// and the schema is the same for every call of the action (#137).
    #[test]
    fn an_action_that_explains_one_call_says_it_can_and_carries_no_sentence() {
        let schema = Action::new("set_backend", "Choose where pictures are made from now on")
            .arg(Param::one_of("kind", &["comfyui", "openai-images", "fake"]))
            .explain(|args| match args["kind"].as_str().unwrap_or_default() {
                "fake" => "After this, prompts stay on this machine.".to_string(),
                _ => String::new(),
            })
            .schema();
        assert_eq!(schema["explains"], serde_json::json!(true), "{schema}");
        assert!(!schema.to_string().contains("prompts stay"), "the sentence is not in describe: {schema}");

        // And what the closure does with the arguments is the app's business, per call: one
        // sentence for the call it can speak about, nothing for one it cannot.
        let action = Action::new("set_backend", "Choose where pictures are made from now on")
            .explain(|args| match args["kind"].as_str().unwrap_or_default() {
                "fake" => "After this, prompts stay on this machine.".to_string(),
                _ => String::new(),
            });
        let explainer = action.explainer.as_ref().unwrap();
        assert_eq!(explainer.sentence(&serde_json::json!({"kind": "fake"})), "After this, prompts stay on this machine.");
        assert_eq!(explainer.sentence(&serde_json::json!({"kind": "openai-images"})), "");
    }
}
