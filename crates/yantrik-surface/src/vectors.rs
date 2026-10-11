//! `deploy/yantrik-os/dispatch-vectors.json`: this dispatch's argument rules and its order,
//! written out for every other implementation to replay — the Python surface SDK first
//! (`sdk/python/tests/test_dispatch_vectors.py`).
//!
//! `surface-vectors.json` is the gate's decision written out; this is what the dispatch does
//! around it. Two sections:
//!
//! * `coerce` — one declared argument and the arguments a caller sent: what the handler reads,
//!   or the exact refusal. Every conversion the dispatch makes, and the near misses it does not.
//! * `order` — one action on a surface, a machine's ceiling and mode, the grants a person allowed,
//!   and a sequence of calls: for each, what the dispatch answered (the exact refusal when it
//!   refused), what the handler read when it ran, and which grants had been spent after it. The
//!   point of the section: a call its own arguments refuse leaves its grant unspent, and the
//!   same grant then runs the call made right. Every refusal recorded is the dispatch's or the
//!   gate's, never the stand-in shell's, whose words are its own.
//!
//! Generated, never edited: the file is what this module produces from the tables below by
//! running the real dispatch ([`Surface::act`]) against the crate's stand-in shell, and
//! `dispatch_vectors_are_what_this_dispatch_does` fails the build when the two differ. After a
//! deliberate change, rewrite it:
//!
//! ```text
//! YANTRIK_WRITE_VECTORS=1 cargo test -p yantrik-surface --lib dispatch_vectors_write
//! ```

use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{json, Value};
use yantrik_ipc_transport::gate::{Authority, Mode};

use crate::{as_declared, check_arguments, stand_in, Action, Param, Surface, View};

/// One declared argument, as a replayer rebuilds it: its name, whether it is required, and the
/// JSON Schema property `describe` publishes for it.
fn declared(p: &Param) -> Value {
    let mut out = p.schema();
    out["name"] = p.name.clone().into();
    out["required"] = p.required.into();
    out
}

/// `(what is declared, what arrives)` for the `coerce` section, as `args` objects so that an
/// argument left out, sent as `null`, or filled by a default can be said.
fn coerce_cases() -> Vec<(Param, Value)> {
    let text = || Param::text("x");
    let integer = || Param::integer("x");
    let number = || Param::number("x");
    let flag = || Param::flag("x");
    let level = || Param::one_of("x", &["low", "normal", "critical"]);
    let digits = || Param::one_of("x", &["1", "2"]);
    let mut cases = vec![
        // An integer for text is its decimal digits; nothing else is text.
        (text(), json!({"x": "67"})),
        (text(), json!({"x": 67})),
        (text(), json!({"x": -3})),
        (text(), json!({"x": 0})),
        (text(), json!({"x": u64::MAX})),
        (text(), json!({"x": i64::MIN})),
        (text(), json!({"x": 1.5})),
        (text(), serde_json::from_str(r#"{"x": 3.0}"#).unwrap()),
        (text(), json!({"x": true})),
        (text(), json!({"x": [1]})),
        (text(), json!({"x": {"id": 1}})),
        (text(), json!({"x": null})),
    ];
    // A string that is exactly an integer is one; nothing near it is.
    for given in [
        json!("12"), json!("-4"), json!("0"), json!("-0"), json!("18446744073709551615"),
        json!("-9223372036854775808"), json!("18446744073709551616"), json!("-9223372036854775809"),
        json!("1.5"), json!("12abc"), json!(" 12"), json!("12 "), json!("+12"), json!("012"), json!(""),
        json!("-"), json!("1e3"), json!("0x10"), json!("1_000"), json!("١٢"), json!(12), json!(2.5),
        json!(true),
    ] {
        cases.push((integer(), json!({ "x": given })));
    }
    // A string that is exactly an integer or a decimal is a number.
    for given in [
        json!("12"), json!("1.5"), json!("-0.25"), json!("0.1"), json!("1e3"), json!(".5"), json!("1."),
        json!("1.5x"), json!("NaN"), json!("inf"), json!("-"), json!("01.5"), json!("1_0.5"), json!(" 1.5"),
        json!("18446744073709551616"), json!(3), json!(1.5), json!(false),
    ] {
        cases.push((number(), json!({ "x": given })));
    }
    // The words for true and false, exactly.
    for given in [json!("true"), json!("false"), json!(true), json!("True"), json!("1"), json!("yes"), json!(1)] {
        cases.push((flag(), json!({ "x": given })));
    }
    // An enum is exact, and an integer for one is its digits, checked against the list.
    for given in [json!("low"), json!("Low"), json!("urgent"), json!(1), json!(1.5)] {
        cases.push((level(), json!({ "x": given })));
    }
    for given in [json!("2"), json!(2), json!(3)] {
        cases.push((digits(), json!({ "x": given })));
    }
    // Nothing is converted into or inside an array or an object.
    cases.extend([
        (Param::array("x", "integer"), json!({"x": [1, 2]})),
        (Param::array("x", "integer"), json!({"x": ["1", "2"]})),
        (Param::array("x", "integer"), json!({"x": "[1,2]"})),
        (Param::array("x", "string"), json!({"x": [1]})),
        (Param::object("x"), json!({"x": {}})),
        (Param::object("x"), json!({"x": "{}"})),
    ]);
    // Left out, null, and a default.
    cases.extend([
        (Param::integer("x").default(5), json!({})),
        (Param::integer("x").default(5), json!({"x": null})),
        (Param::integer("x").default(5), json!({"x": "6"})),
        (Param::integer("x").optional(), json!({"x": null})),
        (Param::integer("x").optional(), json!({})),
        (integer(), json!({"x": null})),
        (integer(), json!({})),
    ]);
    cases
}

fn coerce_section() -> Vec<Value> {
    coerce_cases()
        .into_iter()
        .map(|(p, args)| {
            let spec = Action::new("act", "An action with one argument").arg(p.clone());
            let mut out = json!({ "param": declared(&p), "args": args });
            match check_arguments(&spec, &args) {
                Ok(()) => out["handler_gets"] = as_declared(&spec, &args).into_owned(),
                Err(refusal) => out["refusal"] = refusal.into(),
            }
            out
        })
        .collect()
}

/// One vector of the `order` section: an action at a grade, the machine, the grants a person
/// allowed (by name, with the arguments the card showed), and the calls in turn.
struct Order {
    id: &'static str,
    grade: &'static str,
    ceiling: &'static str,
    mode: &'static str,
    allowed: Vec<(&'static str, Value)>,
    calls: Vec<(Value, Option<&'static str>)>,
}

/// The action every `order` vector publishes, at the vector's grade.
fn move_event(grade: &'static str) -> Action {
    Action::new("move_event", "Move an event to another day")
        .risk(grade)
        .arg(Param::integer("id").describe("The event"))
        .arg(Param::text("to").describe("The day"))
        .arg(Param::flag("notify").default(false).describe("Tell the people invited"))
}

fn order_cases() -> Vec<Order> {
    let right = || json!({"id": 3, "to": "friday"});
    vec![
        Order {
            id: "arguments-its-own-arguments-refuse-leave-the-grant-unspent",
            grade: "sensitive",
            ceiling: "sensitive",
            mode: "ask",
            allowed: vec![("g1", right())],
            calls: vec![
                (json!({"id": "three", "to": "friday"}), Some("g1")),
                (json!({"id": 3}), Some("g1")),
                (json!({"id": 3, "to": "friday", "when": "now"}), Some("g1")),
                (json!(["friday"]), Some("g1")),
                (right(), Some("g1")),
            ],
        },
        Order {
            id: "a-grant-is-bound-to-the-arguments-as-sent-and-the-handler-reads-them-converted",
            grade: "sensitive",
            ceiling: "sensitive",
            mode: "ask",
            allowed: vec![("g1", json!({"id": "3", "to": "friday"}))],
            calls: vec![(json!({"id": "3", "to": "friday"}), Some("g1"))],
        },
        Order {
            id: "the-arguments-are-answered-before-the-ceiling",
            grade: "dangerous",
            ceiling: "sensitive",
            mode: "bypass",
            allowed: vec![],
            calls: vec![(json!({"id": "x", "to": "friday"}), None), (right(), None)],
        },
        Order {
            id: "above-the-ceiling-a-grant-is-not-spent",
            grade: "dangerous",
            ceiling: "sensitive",
            mode: "ask",
            allowed: vec![("g1", right())],
            calls: vec![(right(), Some("g1"))],
        },
        Order {
            id: "the-arguments-are-answered-before-the-mode",
            grade: "sensitive",
            ceiling: "sensitive",
            mode: "ask",
            allowed: vec![],
            calls: vec![(json!({"to": "friday"}), None), (right(), None)],
        },
        Order {
            id: "a-grant-stands-in-for-plan-mode-and-only-with-the-arguments-right",
            grade: "sensitive",
            ceiling: "sensitive",
            mode: "plan",
            allowed: vec![("g1", right())],
            calls: vec![(right(), None), (json!({"id": 3, "to": 5.5}), Some("g1")), (right(), Some("g1"))],
        },
        Order {
            id: "converted-for-the-handler-and-a-grant-spent-where-the-mode-would-not-ask",
            grade: "standard",
            ceiling: "sensitive",
            mode: "ask",
            allowed: vec![("g2", json!({"id": "12", "to": 5, "notify": "true"}))],
            calls: vec![
                (json!({"id": "12", "to": 5, "notify": "true"}), None),
                (json!({"id": 12, "to": "x", "notify": null}), None),
                (json!({"id": "12", "to": 5, "notify": "true"}), Some("g2")),
            ],
        },
        Order {
            id: "not-converted-and-refused-as-before",
            grade: "standard",
            ceiling: "sensitive",
            mode: "ask",
            allowed: vec![],
            calls: vec![
                (json!({"id": "12abc", "to": "x"}), None),
                (json!({"id": 1.5, "to": "x"}), None),
                (json!({"id": 1, "to": 1.5}), None),
                (json!({"id": 1, "to": "x", "notify": "yes"}), None),
                (json!({"id": 1, "to": "x", "notify": 1}), None),
            ],
        },
    ]
}

/// Grant ids unique to one run of the table: the stand-in shell is process-wide, and a grant
/// holds once in a process. The ids never appear in an answer the vectors record.
static RUN: AtomicUsize = AtomicUsize::new(0);

fn order_section() -> Vec<Value> {
    let run = RUN.fetch_add(1, Ordering::Relaxed);
    order_cases()
        .into_iter()
        .map(|v| {
            let spec = move_event(v.grade);
            let real = |g: &str| format!("dispatch-vector-{run}-{}-{g}", v.id);
            for (g, args) in &v.allowed {
                stand_in::allow(&real(g), "calendar", &spec.name, args.clone());
            }
            let surface = Surface::new("calendar")
                .describe(|| View::new("Calendar"))
                .action(spec.clone(), |args| Ok(json!({ "got": args })));
            let calls: Vec<Value> = v
                .calls
                .iter()
                .map(|(args, grant)| {
                    let mut params = json!({ "action": spec.name, "args": args });
                    if let Some(g) = grant {
                        params["grant"] = real(g).into();
                    }
                    let authority = Authority { ceiling: v.ceiling.into(), mode: Mode::named(v.mode), granted: false, asks_above: None };
                    let mut out = json!({ "args": args, "grant": grant });
                    match surface.act(&params, None, authority) {
                        Ok(answer) => {
                            out["outcome"] = "allow".into();
                            out["handler_gets"] = answer["result"]["got"].clone();
                        }
                        Err(refused) => {
                            assert_eq!(refused.code, crate::REFUSED, "{}", refused.message);
                            out["outcome"] = "refused".into();
                            out["refusal"] = refused.message.into();
                        }
                    }
                    let spent: Vec<&str> =
                        v.allowed.iter().map(|(g, _)| *g).filter(|g| stand_in::spent(&real(g))).collect();
                    out["spent"] = json!(spent);
                    out
                })
                .collect();
            json!({
                "id": v.id,
                "app": "calendar",
                "action": {
                    "name": spec.name,
                    "description": spec.description,
                    "permission": spec.permission,
                    "params": spec.params.iter().map(declared).collect::<Vec<_>>(),
                },
                "ceiling": v.ceiling,
                "mode": v.mode,
                "allowed": v.allowed.iter().map(|(g, args)| json!({"grant": g, "args": args})).collect::<Vec<_>>(),
                "calls": calls,
            })
        })
        .collect()
}

fn document() -> String {
    let header = json!([
        "Generated. Do not hand-edit: YANTRIK_WRITE_VECTORS=1 cargo test -p yantrik-surface --lib dispatch_vectors_write",
        "",
        "What the surface dispatch does around the gate (docs/surface-protocol.md, sections 4 and 5), written out from",
        "crates/yantrik-surface for every other implementation to replay: the Python SDK replays both sections.",
        "",
        "coerce: one declared argument (`param`: its name, whether it is required, and the JSON Schema property describe",
        "publishes) and the `args` a caller sent to an action called `act` that takes only it. `handler_gets` is the",
        "arguments the handler reads — converted without loss where they convert, defaults filled in — or `refusal`",
        "is the dispatch's exact sentence.",
        "",
        "order: one action on the surface `app`, the machine's `ceiling` and `mode`, the grants a person `allowed` (a",
        "stand-in shell holds each once, for exactly those arguments), and `calls` made in turn. For each call:",
        "`outcome` allow | refused, the exact `refusal`, what the handler read (`handler_gets`), and the grants `spent`",
        "after it. A call its own arguments refuse leaves its grant unspent; a grant is bound to the arguments as sent.",
    ]);
    let lines = |items: &[Value]| {
        items.iter().map(|v| format!("    {v}")).collect::<Vec<_>>().join(",\n")
    };
    format!(
        "{{\n  \"_\": {},\n  \"coerce\": [\n{}\n  ],\n  \"order\": [\n{}\n  ]\n}}\n",
        serde_json::to_string_pretty(&header).unwrap().replace("\n", "\n  "),
        lines(&coerce_section()),
        lines(&order_section()),
    )
}

fn vectors_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../deploy/yantrik-os/dispatch-vectors.json")
}

/// Write the vectors out. Gated, because a test that rewrites its own expectation is not a test.
#[test]
fn dispatch_vectors_write() {
    if std::env::var("YANTRIK_WRITE_VECTORS").as_deref() != Ok("1") {
        return;
    }
    std::fs::write(vectors_path(), document()).expect("write the vectors");
}

#[test]
fn dispatch_vectors_are_what_this_dispatch_does() {
    let path = vectors_path();
    let checked_in = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert!(
        checked_in == document(),
        "deploy/yantrik-os/dispatch-vectors.json is not what this dispatch does any more. If the change \
         is deliberate, rewrite it and change every port with it:\n  \
         YANTRIK_WRITE_VECTORS=1 cargo test -p yantrik-surface --lib dispatch_vectors_write"
    );
}

/// The file says what the coordinator's rules say, not only whatever the code happened to do: a
/// few load-bearing rows, checked by meaning.
#[test]
fn the_vectors_hold_the_rules_they_exist_for() {
    let doc: Value = serde_json::from_str(&document()).expect("json");
    let coerce = doc["coerce"].as_array().unwrap();
    let row = |kind: &str, given: Value| {
        coerce
            .iter()
            .find(|r| r["param"]["type"] == kind && r["param"].get("enum").is_none() && r["args"]["x"] == given)
            .unwrap_or_else(|| panic!("no row for {kind} {given}"))
            .clone()
    };
    assert_eq!(row("string", json!(67))["handler_gets"]["x"], "67");
    assert_eq!(row("integer", json!("12"))["handler_gets"]["x"], 12);
    assert_eq!(row("number", json!("1.5"))["handler_gets"]["x"], 1.5);
    assert!(row("integer", json!("1.5"))["refusal"].is_string());
    assert!(row("integer", json!("12abc"))["refusal"].is_string());
    assert!(row("integer", json!(" 12"))["refusal"].is_string());
    assert_eq!(row("boolean", json!("true"))["handler_gets"]["x"], true);
    assert!(row("boolean", json!("True"))["refusal"].is_string());

    let order = doc["order"].as_array().unwrap();
    let first = &order[0]["calls"];
    for refused in &first.as_array().unwrap()[..4] {
        assert_eq!(refused["outcome"], "refused", "{refused}");
        assert_eq!(refused["spent"], json!([]), "a malformed call spent its grant: {refused}");
    }
    assert_eq!(first[4]["outcome"], "allow");
    assert_eq!(first[4]["spent"], json!(["g1"]), "and the same grant then ran the call made right");
}
