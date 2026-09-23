# Checking your surface

`yos check` reads [the protocol](../surface-protocol.md) as a program and holds a running surface
to it. It never runs one of your actions: every act it sends is one a protocol dispatch refuses
before a handler — and each also carries a stale `expect_revision`, so a surface that skipped an
argument check is still stopped by its revision guard. It prints what it saw, and exits non-zero
when something fails, so you can run it in your own CI.

```sh
yos check counter                 # a surface, by name
yos check ./app-counter.sock      # a socket, while you are writing one
yos check --all                   # every surface that answers on this machine
yos check counter --json          # the same, for a machine to read
```

`yos` is `deploy/yantrik-os/yos` in the repository and `/opt/yantrik/bin/yos` on a Yantrik
machine; it is one file of standard-library Python.

## What it holds a surface to

| check | passes when |
| --- | --- |
| `ping` | `rpc.ping` answers `"pong"` with the id it was sent |
| `describe` | `app.describe` answers; the median of three reads is under 500 ms (a warning above) |
| `protocol` | `protocol` is present and is `1` |
| `schema` | the describe matches [`describe.schema.json`](../schema/describe.schema.json) |
| `grades` | every action is graded `safe`, `standard`, `sensitive` or `dangerous` |
| `params` | every parameter has a type the protocol defines (an array's items too), and every required one is declared |
| `secrets` | no parameter is named like a secret — `passphrase`, `password`, `passwd`, `pin`, `secret`, `credential`, `unlock` (the shell's rule) |
| `revision` | the published revision is the protocol's hash of the summary and the state (a warning when a float may render differently) |
| `steady` | an unchanged view keeps its revision across reads |
| `method`, `empty`, `unknown` | an unknown method, an act with no action and an unknown action are refused with the right code and words |
| `missing`, `undeclared`, `types`, `stale` | a missing argument, an undeclared one, one of the wrong type, and a stale `expect_revision` are refused with the right code and words |

The last four are sent only to a surface that publishes `protocol: 1` and refused the unknown
action exactly as the protocol says, and they name an action graded `safe` or `standard` that
does not say it cannot be undone. A surface with no such action, or none that requires an
argument, gets `skip` for them, which is not a failure.

A real run against [`examples/hello_surface.py`](../../examples/hello_surface.py):

```text
$ yos check hello
yos check hello  (/run/user/1000/yantrik/app-hello.sock)
  pass  ping        rpc.ping answered "pong" with the id it was sent (0.8 ms)
  pass  describe    answered in 0.4 ms, the median of 3 reads (0.4, 0.3, 0.4 ms)
  pass  protocol    protocol 1
  pass  schema      describe matches docs/schema/describe.schema.json
  pass  grades      2 actions: 1 standard, 1 sensitive
  pass  params      2 parameters, each typed string | number | integer | boolean | array | object
  pass  secrets     no parameter named like a secret (passphrase, password, passwd, pin, secret, credential, unlock)
  pass  revision    ec6faecfdbf7d9cf is FNV-1a over the summary and the state
  pass  steady      3 reads of an unchanged view, one revision
  pass  method      app.yos_check_no_such_method → -32601 unknown method `app.yos_check_no_such_method`; this app serves app.describe, app.act
  pass  empty       app.act with no action → -32602 act needs a non-empty `action`
  pass  unknown     unknown action → -32602 unknown action `yos-check-no-such-action`; this app offers: add, clear
  pass  missing     add with no arguments → -32602 `add` needs argument `text`
  pass  undeclared  add with `yos_check_undeclared` → -32602 `add` has no argument `yos_check_undeclared`; it takes: text, count
  pass  types       add with count="yos-check" → -32602 `add` argument `count` must be an integer, and a string arrived
  pass  stale       add acting on 1390513024082630 → -32602 STALE: this app is at revision ec6faecfdbf7d9cf and you acted on 1390513024082630. It now reports: Hello — 2 items. Read it again before deciding.
  0 failed, 0 warned, 16 passed, 0 skipped
```

A surface built on either SDK passes every check without effort; the checks exist for everything
else — a surface written by hand, a port of the dispatch to another language, a declaration the SDK
could not catch (a parameter named `pin`), and a state whose revision never settles.

## In your own CI

Serve the program on a private machine — `HOME` and `XDG_RUNTIME_DIR` in a scratch directory, so
nothing on the developer's desktop is read or touched — and run `yos check <id> --json`. Both
templates do; the Rust one looks for `yos` wherever it may be and skips, saying so, where there is
none:

<!-- from: templates/rust-surface/tests/yos_check.rs -->
```rust
fn yos() -> Option<Command> {
    let in_repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../deploy/yantrik-os/yos");
    let candidates = [std::env::var_os("YOS").map(PathBuf::from), Some(in_repo), Some("/opt/yantrik/bin/yos".into())];
    for path in candidates.into_iter().flatten() {
        if path.is_file() {
            let mut command = Command::new("python3");
            command.arg(path);
            return Some(command);
        }
    }
    Command::new("yos").arg("--help").output().ok().map(|_| Command::new("yos"))
}
```

<!-- from: templates/rust-surface/tests/yos_check.rs -->
```rust
    let out = machine.on_it(check.args(["check", "my-surface", "--json"])).output().expect("run yos");
```

In this repository CI runs `yos check` against `examples/hello-surface` (Rust), `examples/hello_surface.py`
(Python), both templates, and the LibreOffice adapter over a fake LibreOffice.

## When a check fails

- **`schema` or `params`**: a parameter with a type the protocol does not define, or a `required`
  name with no property. Both SDKs refuse these where you declare them (`registry().problems()` in
  Rust; a `ValueError` in Python), so this is a hand-written surface.
- **`secrets`**: rename the parameter — and then ask whether a secret should be an argument at all.
  Arguments are what an approval card shows and the audit log keeps.
- **`revision`** fails: the published revision is not the hash of what was published beside it —
  usually a surface computing it from a different read than the state it sent.
- **`steady`** skipped, or failing: something in the summary or state changes on every read — a
  clock, a timer, a rate. Take it out, or round it to what a decision needs.
- **`describe`** warns: over 500 ms. `describe` runs around every act; make it cheaper, and move
  what is expensive to read behind an action.
- **`unknown`**, **`missing`**, **`undeclared`**, **`types`**: the refusal's words differ from the
  protocol's. A client branches on these sentences; use an SDK, or copy them exactly.

## The vectors

Three files hold the dispatch's behaviour as data, generated from the Rust code, so an
implementation in another language can prove it decides the same way.

- **[`surface-vectors.json`](../../deploy/yantrik-os/surface-vectors.json)** — the decision: grade ×
  ceiling × mode × session rule × grant × "cannot be undone", 640 cases, each with its outcome, the
  exact sentence, and what a door that raises cards must do. Generated from
  `yantrik_ipc_transport::gate::decide` by
  `YANTRIK_WRITE_VECTORS=1 cargo test -p yantrik-ipc-transport --lib surface_vectors_write`.
- **[`dispatch-vectors.json`](../../deploy/yantrik-os/dispatch-vectors.json)** — what the dispatch
  does around the decision: `coerce`, every conversion without loss and every refusal of an
  argument's type; and `order`, calls made in turn against a stand-in shell holding grants, with
  what each call answered and which grants were spent after it. Generated from the
  `yantrik-surface` crate by
  `YANTRIK_WRITE_VECTORS=1 cargo test -p yantrik-surface --lib dispatch_vectors_write`.
- **[`reach-vectors.json`](../../deploy/yantrik-os/reach-vectors.json)** — an agent's reach:
  `within`, calls held to a role's surfaces and ceiling (and the apps it may open); and `standing`,
  what the shell answered about a token and what a door decides from it. Generated from
  `yantrik_ipc_transport::reach` by
  `YANTRIK_WRITE_VECTORS=1 cargo test -p yantrik-ipc-transport --lib reach_vectors_write`.

A row of `coerce` — an integer sent for a text parameter is its digits:

<!-- from: deploy/yantrik-os/dispatch-vectors.json -->
```json
    {"args":{"x":67},"handler_gets":{"x":"67"},"param":{"description":"","name":"x","required":true,"type":"string"}},
```

and two it refuses, naming the kind that arrived and never the value:

<!-- from: deploy/yantrik-os/dispatch-vectors.json -->
```json
    {"args":{"x":"12abc"},"param":{"description":"","name":"x","required":true,"type":"integer"},"refusal":"`act` argument `x` must be an integer, and a string arrived"},
    {"args":{"x":" 12"},"param":{"description":"","name":"x","required":true,"type":"integer"},"refusal":"`act` argument `x` must be an integer, and a string arrived"},
```

The Rust crates fail their own build when the files are not what the code does. The Python SDK
replays both files through its real dispatch; the Blender add-on, the MCP bridge and the shell's
mode table replay the decision. A port replays them the same way — row by row, to the byte of
every sentence:

<!-- from: sdk/python/tests/test_dispatch_vectors.py -->
```python
        for row in rows:
            spec = Action("act", "An action with one argument", params=[param_of(row["param"])])
            refusal = check_arguments(spec, row["args"])
            if "refusal" in row:
                if refusal != row["refusal"]:
                    wrong.append((row, refusal))
                continue
```

You do not need the vectors to write a surface with an SDK — the SDK has already replayed them.
You need them to write a new SDK.
