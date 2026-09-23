# yantrik_surface — put your app where a mind can find it

Yantrik OS runs any Debian program. What makes an app *of this desktop* is a **surface**: a
socket on which the app says what it holds (`app.describe`) and takes instruction
(`app.act`), under the grades the person set. A mind finds it with `yos ls`, reads it with
`yos describe`, and acts in it with `yos act` — or through the MCP bridge, the companion, a
harness — and every one of those doors meets the same ceiling, mode and grants.

This package is that surface for Python: one import, standard library only, Python 3.11+.
It is a port of the Rust runtime's dispatch (`yantrik-app-runtime::control`), not an opinion
of its own: the same envelopes, the same revision hash, the same checks in the same order, the
same refusal sentences to the punctuation. A caller cannot tell a surface built on it from one
of the apps this OS ships. Blender's add-on (`apps/blender/addon`) is built on it.

## Install

From a checkout:

```sh
pip install ./sdk/python
```

Or copy `sdk/python/yantrik_surface/` beside your program. It has no dependencies, which is
how the Blender add-on carries it: a release puts it next to the add-on, so it works in
whatever Python the host program brings.

## Five minutes

[`examples/hello_surface.py`](../../examples/hello_surface.py) (at the top of the repository,
beside its Rust twin), the smallest complete surface:

```python
from typing import Annotated
from yantrik_surface import Refusal, Surface

items = []
surface = Surface("hello", summary=lambda: "Hello — %d items" % len(items))

@surface.view
def state():
    return {"items": list(items)}

@surface.action("add", grade="standard")
def add(text: Annotated[str, "what to put on the list"],
        count: Annotated[int, "how many times, 1 to 100"] = 1) -> dict:
    """Add an item to the list, `count` times."""
    if not text.strip():
        raise Refusal("`text` is empty; say what to add")
    items.extend([text] * count)
    return {"added": text, "count": count, "items": len(items)}

@surface.action("clear", grade="sensitive", settles="later", expected_seconds=5)
def clear() -> dict:
    """Empty the list, item by item, within about five seconds. It cannot be undone."""
    ...  # start the work on a thread of its own and return at once

surface.serve()   # binds app-hello.sock in the session's socket directory
```

Run it, then from another terminal on the same session (this transcript is the real one):

```text
$ python3 examples/hello_surface.py
[yantrik] hello answering on /run/user/1000/yantrik/app-hello.sock (2 actions)

$ yos describe hello
Hello — 0 items
revision: 308360b83e9011c9
{
  "items": [],
  "clearing": false
}
  act: add(text, count?)  [standard, settles on return]
       Add an item to the list, `count` times.
         text: string — what to put on the list
         count?: integer — how many times, 1 to 100
  act: clear()  [sensitive, settles later]
       Empty the list, item by item, within about five seconds. The answer comes at once …

$ yos act hello add text=eggs count=2
Hello — 2 items
accepted: True, settled: True
revision: 4d3aebd0444bf8b9
{
  "added": "eggs",
  "count": 2,
  "items": 2
}
(state omitted; `yos describe hello`, or re-run with --full)

$ yos act hello add text=milk count=lots
yos: hello.app.act refused: `add` argument `count` must be an integer, and a string arrived

$ yos act hello clear
asking — a card is on the screen (120 s)
Hello — 2 items, clearing
accepted: True, settled: False
revision: fca5e8b433e35e25
{
  "clearing": 2
}
(state omitted; `yos describe hello`, or re-run with --full)

$ yos describe hello --brief
Hello — 0 items
revision: 308360b83e9011c9
…
```

`clear` is `sensitive`, so in `ask` mode the dispatch refuses it with `GRANT:`; `yos` puts an
approval card in front of the person, waits for their Allow, and acts again carrying the grant,
which this package spends through the shell before the handler runs.

## What a caller gets

`app.describe`:

```json
{"app": "hello", "protocol": 1, "summary": "Hello — 1 item", "state": {"items": ["milk"]},
 "revision": "9b0e…", "actions": [{"name": "add", "description": "…", "permission": "standard",
 "settles": "on return", "parameters": {"type": "object", "properties": {…}, "required": ["text"]}}]}
```

`app.act {action, args, expect_revision?, grant?, agent_token?}`:

```json
{"app": "hello", "action_id": "app-hello#3", "accepted": true, "settled": true,
 "result": {"added": "milk", "count": 1}, "revision": "…", "summary": "…", "state": {…}}
```

Never `ok`, never `done`: `accepted` says the handler ran, `settled` whether the work finished.
The answer carries the state the action left, so a caller never needs a second round trip.
`revision` is a fingerprint of summary and state (FNV-1a-64, byte-identical to the Rust
`View::revision()`); a caller that sends it back as `expect_revision` gets `STALE:` instead of
an act on a world that has moved.

## Grades

Every action carries one. Choose by what the action can cost the person:

| grade | for | e.g. |
|---|---|---|
| `safe` | reading; changes nothing | search, preview |
| `standard` (default) | changing the app's own state, recoverably | add, open, move, rename |
| `sensitive` | overwriting, sending, spending, anything that leaves the machine | save over a file, send mail, call a paid API |
| `dangerous` | destroying work, running arbitrary code | delete for good, `run_python` |

Say in the description what cannot be undone ("It is not recoverable", "cannot be undone",
"permanently"…): the approval card shows it in red, and the dispatch asks about such an action in
every mode but `bypass`, whatever its grade above `safe` — and no session rule covers it.

What happens to a call, in order — the same order every door uses (docs/surface-protocol.md
§5). First, for a call carrying an agent token, the agent's reach (see *Who is calling*). Then its
arguments: a missing one, one the action does not take, and one of the wrong type
are refused before anything else is asked, so a person's Allow is never used up on a call that was
never going to run. Then:

1. **The ceiling** — `tool_permission` in `~/.config/yantrik/settings.yaml`, `sensitive` when
   unset. Above it nothing runs: not with a mode, not with a grant. Refused with `CEILING:`.
2. **The grant** — if the call carries one, it is spent through the shell's
   `consume_approval`, bound to this app, this action and these exact arguments, as they were
   sent. Only after the arguments and the ceiling have passed, so a person's Allow is never used up
   on an act that cannot run; and only
   to the desktop's own shell — the process listening on `app-shell.sock` must be a
   `yantrik-ui` binary, or the grant is not offered to it.
3. **The mode** — `plan`, `ask`, `auto` or `bypass`, published by the shell in
   `mind-mode.json`. Above what the mode runs unasked (`ask` runs `standard`, `auto` runs
   `sensitive`, `bypass` everything under the ceiling), or anything whose description says it
   cannot be undone, with no grant and no session rule (none in plan), the call is refused with
   `GRANT:`, which says how to get one. `standard` runs unasked in every mode on a socket,
   because the desktop's own processes make standard calls.

`describe` is always free. Grades can change while the app runs: `surface.regrade("generate",
"sensitive")` when a setting makes an action costlier.

## Parameters

From the handler's signature, with its hints and defaults:

| Python | published |
|---|---|
| `str` | `{"type": "string"}` |
| `int` | `{"type": "integer"}` — `3`, not `3.0`, not `true` |
| `float` | `{"type": "number"}` — an integer is a number |
| `bool` | `{"type": "boolean"}` |
| `Literal["a", "b"]` | `{"type": "string", "enum": ["a", "b"]}` — only text can be one of a list |
| `list[str]` | `{"type": "array", "items": {"type": "string"}}` |
| `dict` | `{"type": "object"}` |
| `x: int = 3` | `…, "default": 3`, and not required |
| `x: Optional[str] = None` | not required, no default published |

These are the Rust `yantrik-surface` crate's types, published the way it publishes them. Describe
a parameter with `Annotated[str, "what it is"]` or `params={"name": "what it is"}`; the action's
purpose is `description=` or the first paragraph of the docstring (one is required — it is the
sentence on the approval card). A declaration that could never be called right — a default of
the wrong type, an enum of numbers, `*args` — is refused where you write it, not at the first
call.

Before your handler runs, the dispatch refuses a missing argument, one the action does not take,
and one of the wrong type, in the crate's sentences — `` `add` argument `count` must be an
integer, and a string arrived `` names the kind that arrived, never the value, which may be a
PIN. Your handler always gets the type it declared, and a caller is met halfway: what converts
without loss is converted first — an integer for `str` becomes its digits (`which: 1` is `"1"`),
a string that is exactly a number for `int` or `float` becomes the number (`"12"` is 12; `"1.5"`,
`"12abc"` and `" 12"` are not integers), and `"true"`/`"false"` for `bool` become the booleans.
Nothing else is converted, and nothing in a list or a dict. `null` for an optional argument is the
same as leaving it out, and the handler gets the declared default. For a table of actions rather than decorated functions, build
`Action(name, purpose, grade, [Param(...), ...])` and `surface.add_action(spec, handler)`, where
the handler takes the arguments as one dict.

## Refusals

`raise Refusal("a sentence")` from a handler: the caller gets it as it stands (JSON-RPC
-32602), exactly as a Rust handler's `Err(String)`. Say what was asked, what is wrong, and what
to do instead. Any other exception is a fault, answered as -32000 with its type and message and
printed to stderr.

## Answers that take time

- `settles="later"`: the action only *starts* the work. The answer says `settled: false`, and a
  caller watches `describe` for the result instead of taking the call for it.
  `expected_seconds` (whole seconds) publishes how long settling usually takes, so a caller can
  size its wait.
- `return Later(work)`: the caller is owed the *result* of slow work (an exit code, a finished
  export). The app is released at once; `work()` runs on the caller's connection thread, its
  return is the `result`, and the state is read again afterwards.

## Threads

By default every describe and act runs on the connection's thread under one lock — the app's
serialization domain, which makes the revision check and the handler one atomic turn. If your
state belongs to one thread (a GTK or Qt main loop, Blender's main thread), subclass `Surface`
and override `run_on_app_thread(fn, timeout)` to run `fn` there and return its result, raising
`NotAnswered` if it did not run within `timeout` seconds (the caller hears "app did not answer
within Ns"). Override `snapshot()` to return `(summary, state)` in one read.
The `Surface` subclass at the end of `apps/blender/addon/yantrik_blender/surface.py` is a complete example.

## Who is calling

Inside a handler, `caller()` is the peer's `PeerCred(pid, uid, gid)` as the kernel reports it,
and `agent_token()` the agent token the call carried beside its `args`. A token put inside `args`
is removed before anything reads them, and not used.

A call that carries a token is asked about before anything else (`yantrik_surface.reach`, the
port of `yantrik_ipc_transport::reach`): the dispatch asks the shell what the token is —
`agent.reach` on `app-shell.sock`, by its SHA-256, only of a `yantrik-ui` process — and an agent
started as a catalog role is held to the role's surfaces and ceiling, refused with `REACH:`. It
fails closed: a token no live agent carries, and every token while the shell does not answer, is
refused. A live agent with no role meets only the gate, and a call with no token asks nothing.

## Names

A surface binds `app-<id>.sock` in `$XDG_RUNTIME_DIR/yantrik` (then `/run/yantrik`, then
`/tmp/yantrik-<uid>`), directory 0700, socket 0600. Before binding it asks whatever is at the
path `rpc.ping`: a process that answers — or accepts and stays silent for a second — keeps its
name, and `serve()` exits with "another instance owns …"; only a socket nobody listens on, a
symlink or a stray file is replaced. `Surface(..., aliases=["other-name"])` links other names
beside it. `serve()` answers until Ctrl-C or SIGTERM and unbinds on the way out;
`serve_in_thread()` returns at once.

A request is JSON-RPC 2.0 with `jsonrpc`, `method` and `id`; one without an `id` (a notification)
or a batch is answered as a parse error, as the Rust transport answers it.

## Testing a surface

In-process, with no socket: `surface.describe_json()` and `surface.act({"action": "add",
"args": {"text": "x"}})` return the envelopes or raise `RpcError` with the refusal. Point `HOME`
at a temporary directory (or pass `settings_path=` and `mode_path=`) to pin the ceiling and the
mode, `spend_grant=` to stand in for the shell, and `reach_of=` for its answer about an agent
token (a function of the token returning `("plain", None)`, `("held", reach.Reach(...))` or
`("unknown", None)`). Over the socket, set `XDG_RUNTIME_DIR`,
`serve_in_thread()`, and drive it with `yos describe <id>` and `yos act <id> <action> key=value`.

This package's own conformance suite — envelopes, revision vectors produced by serde_json,
every refusal checked against the Rust source it is ported from, the gate, real sockets, and
the example driven by the real `yos`:

```sh
python3 -m unittest discover -s sdk/python/tests -v
```

## The guide

[`docs/sdk/`](../../docs/sdk/README.md) is written for someone who has never seen this OS: a
quickstart in each language, wrapping a program you did not write (Blender, and the LibreOffice
adapter in `adapters/libreoffice`, which is built only on this package), choosing a grade,
designing a `describe` a mind can use, the `.desktop` keys, and `yos check`.
`templates/python-surface` is a starting point to copy.

## Where it comes from

[`docs/surface-protocol.md`](../../docs/surface-protocol.md) is the protocol (normative, version 1)
and `design/surface-sdk-2026-09-23.md` the SDK's design. Every one of the policy vectors in
`deploy/yantrik-os/surface-vectors.json`, generated from the Rust gate, is replayed through this
dispatch by `tests/test_vectors.py` — decisions, sentences, phrases, revisions and float edges.
