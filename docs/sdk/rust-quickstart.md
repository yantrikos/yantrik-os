# Rust quickstart

You will build the smallest complete surface — a counter a mind can read, add to, and, with the
person's Allow, reset — run it, and drive it the way a mind does. It is
[`examples/hello-surface`](../../examples/hello-surface/src/main.rs); every line of it below is
quoted from that file.

You need Linux and a Rust toolchain. Nothing here needs a Yantrik desktop: `yos`, the command a
mind's tools run, is a Python script in the repository (`deploy/yantrik-os/yos`), and on a Yantrik
machine it is `/opt/yantrik/bin/yos`.

## The dependency

One crate, `yantrik-surface`: the dispatch with no UI in it.

<!-- from: examples/hello-surface/Cargo.toml -->
```toml
[dependencies]
yantrik-surface = { workspace = true }
serde_json = { workspace = true }
```

Inside the yantrik-os workspace that is all. Outside it, point at the repository:
`yantrik-surface = { git = "https://github.com/yantrikos/yantrik-os" }` (the
[template](../../templates/rust-surface/README.md) says how).

## What the program holds

Your program keeps its state however it likes. The counter's is two fields behind a lock, because
the socket answers on more than one thread.

<!-- from: examples/hello-surface/src/main.rs -->
```rust
#[derive(Default)]
struct Counter {
    count: i64,
    /// Who changed it last, as the kernel reported them — never as they said.
    last_by: Option<i32>,
}
```

## What it reports

`describe` is a closure that returns a `View`: one line a person could read, and a state object in
the program's own words. It runs on every `app.describe` and around every `app.act`, so keep it
cheap.

<!-- from: examples/hello-surface/src/main.rs -->
```rust
    let surface = Surface::new("counter")
        // What a mind reads first: one line a person could read, and a small state object.
        .describe({
            let counter = counter.clone();
            move || {
                let c = counter.lock().unwrap_or_else(|e| e.into_inner());
                let by = c.last_by.map_or("nobody yet".to_string(), |pid| format!("pid {pid}"));
                View::new(format!("Counter — {}, last changed by {by}", c.count))
                    .with("count", c.count)
                    .with("last_changed_by_pid", c.last_by)
            }
        })
```

`Surface::new("counter")` is the id: the surface publishes it as `app` and binds
`app-counter.sock`. An id is lowercase words joined by `-`.

## What it offers

Each action is an `Action` — a name, a description, a grade, its parameters — and a handler that
gets the arguments as JSON and returns a result or a sentence refusing.

<!-- from: examples/hello-surface/src/main.rs -->
```rust
        .action(
            Action::new("increment", "Add to the counter")
                .arg(Param::integer("by").default(1).describe("How much to add; negative to subtract")),
            {
                let counter = counter.clone();
                move |args| {
                    // Present and an integer: the dispatch checked it, and filled in the default.
                    let by = args["by"].as_i64().unwrap_or(1);
                    let mut c = counter.lock().unwrap_or_else(|e| e.into_inner());
                    c.count = c.count.checked_add(by).ok_or("that would overflow the counter")?;
                    c.last_by = caller().map(|who| who.pid);
                    Ok(serde_json::json!({ "count": c.count }))
                }
            },
        )
```

`increment` has no `.risk(...)`, so it is `standard`: it changes something, recoverably.
`Param::integer("by").default(1)` publishes a JSON Schema integer with a default, so a caller may
leave it out and the handler still reads 1. The other parameter kinds are `Param::text`, `number`,
`flag` (a boolean), `one_of(name, &[...])` (an enum), `array(name, item_type)` and `object`.

`reset` throws away what was counted, so it is `sensitive`:

<!-- from: examples/hello-surface/src/main.rs -->
```rust
        .action(
            Action::new("reset", "Set the counter back to zero, forgetting what was counted")
                .risk("sensitive"),
```

[Choosing a grade](grades.md) is how to decide.

## Serving it

<!-- from: examples/hello-surface/src/main.rs -->
```rust
    // An author's own check: every declaration is one the dispatch can enforce.
    for problem in surface.registry().problems() {
        eprintln!("hello-surface: {problem}");
    }
    eprintln!("hello-surface: serving `counter` on {}; ctrl-c to stop", surface.address());
    if let Err(e) = surface.serve() {
        eprintln!("hello-surface: could not serve: {e}");
        std::process::exit(1);
    }
```

`serve()` binds the socket in the session's socket directory (`$XDG_RUNTIME_DIR/yantrik`) and
answers until the process ends. It refuses to start if another live process already answers
under the name.

## Drive it

```sh
cargo run -p hello-surface
```

From another terminal on the same session (this transcript is a real run, on a machine in `ask`
mode):

```text
$ yos describe counter
Counter — 0, last changed by nobody yet
revision: d3fe4b5ba49409c6
{
  "count": 0,
  "last_changed_by_pid": null
}
  act: increment(by?)  [standard, settles on return]
       Add to the counter
         by?: integer — How much to add; negative to subtract
  act: reset()  [sensitive, settles on return]
       Set the counter back to zero, forgetting what was counted

$ yos act counter increment by=2
Counter — 2, last changed by pid 1735326
accepted: True, settled: True
revision: 1304f81fa0cb6d3f
{
  "count": 2
}
(state omitted; `yos describe counter`, or re-run with --full)

$ yos act counter increment by=two
yos: counter.app.act refused: `increment` argument `by` must be an integer, and a string arrived

$ yos act counter reset --no-ask
yos: GRANT: counter.reset is graded `sensitive` and this machine is in ask mode, which runs nothing above `standard` without asking — so it was not run. Ask the shell for approval first (…), or have the person at the machine press Allow when the card appears.
```

Without `--no-ask`, `yos` asks the shell to put an approval card in front of the person, waits for
Allow, and acts again carrying the grant; the dispatch spends it before the handler runs.

## What you did not write

No argument parsing, no type checks, no permission code, no revision bookkeeping, no JSON-RPC.
The dispatch in `yantrik-surface`:

- refused `by=two` before the handler ran, in a sentence that names the argument and the kind
  that arrived, never the value (a number a caller sends may be a PIN);
- converts what converts without loss — `"2"` for an integer is 2 — so a model that quotes a
  number is not bounced;
- answered every act with the view after it, so a caller never needs a second round trip;
- refuses an act made with `expect_revision` set to a revision the counter has left (`STALE:`),
  so a mind never acts on a view that has moved;
- held `reset` for the person, and would hold anything above the machine's ceiling for nobody;
- tells a handler who called ([`caller()`](../../crates/yantrik-surface/src/context.rs), the
  kernel's account of the peer) and which of the person's agents a call is for (`agent_token()`).

## Test it in-process

A surface can be called without a socket, under a ceiling and mode the test pins, so the machine
running the tests lends it neither. From the template's tests:

<!-- from: templates/rust-surface/tests/surface.rs -->
```rust
/// The machine this OS ships: a `sensitive` ceiling, in `mode`.
fn under(mode: &str) -> Authority {
    Authority { ceiling: "sensitive".into(), mode: Mode::named(mode), granted: false, asks_above: None }
}
```

<!-- from: templates/rust-surface/tests/surface.rs -->
```rust
#[test]
fn remove_waits_for_the_person_in_ask_and_in_auto() {
    let (surface, tasks) = fresh();
    act(&surface, "ask", "add", json!({ "title": "Water the plants" })).unwrap();

    let refused = act(&surface, "ask", "remove", json!({ "index": 0 })).unwrap_err();
    assert!(refused.starts_with("GRANT: my-surface.remove is graded `sensitive`"), "{refused}");
    // Auto runs `sensitive` unasked — but not what its own description says cannot be undone.
    let refused = act(&surface, "auto", "remove", json!({ "index": 0 })).unwrap_err();
    assert!(refused.contains("its own description says it cannot be undone"), "{refused}");
    assert_eq!(lock_items(&tasks).len(), 1, "nothing was removed");

    // Bypass still asks before what cannot be undone; full bypass runs everything under the
    // ceiling.
    let refused = act(&surface, "bypass", "remove", json!({ "index": 0 })).unwrap_err();
    assert!(refused.contains("its own description says it cannot be undone"), "{refused}");
    let reply = act(&surface, "bypass_all", "remove", json!({ "index": 0 })).unwrap();
    assert_eq!(reply["result"], json!({ "removed": "Water the plants", "left": 0 }));
}
```

Then hold the running program to the protocol with `yos check` — [Checking your surface](checking.md).

## A program with a window

A program with a Slint window publishes through `yantrik_app_runtime::control::App` instead. It is
the same dispatch, plus one thing a window needs: `describe` and every handler run on the thread
that owns the window, so a handler reads and changes the UI directly and nothing moves the UI
between the revision check and the handler. From
[`examples/hello-window`](../../examples/hello-window/src/main.rs):

<!-- from: examples/hello-window/src/main.rs -->
```rust
    App::new("hello-window")
        // On the window's thread: read the UI itself.
        .describe({
            let weak = weak.clone();
            move || {
                let count = weak.upgrade().map_or(0, |ui| ui.get_count());
                View::new(format!("Hello, window — {count}")).with("count", count)
            }
        })
```

and a window's `main` ends its event loop through `run_until_closed`, never `ui.run().unwrap()`:
a logout takes the display away and the loop returns an error, which is an ending, not a crash.

<!-- from: examples/hello-window/src/main.rs -->
```rust
        // Before the loop, from the window's thread.
        .serve();

    // Not `ui.run().unwrap()`: a display that goes away is an ending, not a crash.
    let closed = run_until_closed(&ui, "hello-window");
```

## Next

- Copy [`templates/rust-surface`](../../templates/rust-surface/README.md): the same shape grown
  into a to-do list with a `safe`, two `standard` and a `sensitive` action, a `.desktop` file and
  both kinds of test.
- [Designing a describe a mind can use](describe.md), then
  [Being found while closed](found-while-closed.md).
