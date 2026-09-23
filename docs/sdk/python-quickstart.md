# Python quickstart

You will build the smallest complete surface in Python — a list a mind can read, add to and clear —
run it, and drive it the way a mind does. It is
[`examples/hello_surface.py`](../../examples/hello_surface.py); every line below is quoted from it.

You need Linux and Python 3.11 or newer. The SDK, `yantrik_surface`, is standard library only: one
directory you install (`pip install ./sdk/python` from a checkout of yantrik-os) or copy beside
your program. `yos`, the command a mind's tools run, is `deploy/yantrik-os/yos` in the repository
and `/opt/yantrik/bin/yos` on a Yantrik machine.

## The surface

<!-- from: examples/hello_surface.py -->
```python
from yantrik_surface import Refusal, Surface  # noqa: E402

items = []
clearing = threading.Event()

surface = Surface(
    "hello",
    summary=lambda: "Hello — %d item%s%s" % (
        len(items), "" if len(items) == 1 else "s", ", clearing" if clearing.is_set() else ""),
)
```

`"hello"` is the id: the surface publishes it as `app` and binds `app-hello.sock`. `summary` is the
one line a person could read — text, or a function that returns it. The state is a function too:

<!-- from: examples/hello_surface.py -->
```python
@surface.view
def state():
    return {"items": list(items), "clearing": clearing.is_set()}
```

## An action

A decorated function is an action. Its parameters come from the signature: the type hints are the
published types, a default makes an argument optional, and `Annotated[T, "..."]` describes it. The
first paragraph of the docstring is the description — the sentence on the approval card and in a
mind's context, so it is required.

<!-- from: examples/hello_surface.py -->
```python
@surface.action("add", grade="standard")
def add(text: Annotated[str, "what to put on the list"],
        count: Annotated[int, "how many times, 1 to 100"] = 1) -> dict:
    """Add an item to the list, `count` times."""
    text = text.strip()
    if not text:
        raise Refusal("`text` is empty; say what to add")
    if not 1 <= count <= 100:
        raise Refusal("`count` must be from 1 to 100; %d is not" % count)
    items.extend([text] * count)
    return {"added": text, "count": count, "items": len(items)}
```

By the time `add` runs, `text` is present and a string and `count` is an integer: the dispatch
checked them against the hints and refused anything else. What the handler checks is what only it
knows — that the text says something, that the count is sensible. `raise Refusal("a sentence")`
is the caller's answer, word for word: say what was asked, what is wrong, what to do instead.

| hint | published as |
| --- | --- |
| `str` | `{"type": "string"}` |
| `int` | `{"type": "integer"}` — `3`, not `3.0` |
| `float` | `{"type": "number"}` |
| `bool` | `{"type": "boolean"}` |
| `Literal["a", "b"]` | `{"type": "string", "enum": ["a", "b"]}` |
| `list[str]` | `{"type": "array", "items": {"type": "string"}}` |
| `dict` | `{"type": "object"}` |
| `x: int = 3` | a `default`, and not required |
| `x: Optional[str] = None` | not required, and no default published |

## An action that takes time

`clear` empties the list item by item over a few seconds. It returns at once, so it says it
**settles later**: the answer carries `settled: false`, and a caller watches `describe` (here,
`clearing`) instead of taking the call for the finished work.

<!-- from: examples/hello_surface.py -->
```python
@surface.action("clear", grade="sensitive", settles="later", expected_seconds=5)
def clear() -> dict:
    """Empty the list, item by item, within about five seconds. The answer comes at once and
    says `settled: false`; describe shows `clearing` until it is done. It cannot be undone."""
```

It is `sensitive` because it throws the list away, and its description says it cannot be undone,
which makes the dispatch ask the person about it in every mode but `bypass`
([Choosing a grade](grades.md)). When the caller is owed the *result* of slow work instead — an
exit code, a finished export — return `Later(work)` from the handler: the reply waits for `work()`
without holding the program up ([Designing a describe](describe.md#settles-later-or-answered-later)).

## Serving it

<!-- from: examples/hello_surface.py -->
```python
if __name__ == "__main__":
    surface.serve()
```

`serve()` binds the socket in the session's socket directory and answers until Ctrl-C or SIGTERM,
then takes the socket away. It refuses to start if another live process already answers under
the name. `serve_in_thread()` returns at once, for a program with a main loop of its own.

## Drive it

```sh
python3 examples/hello_surface.py
```

From another terminal on the same session (a real run, on a machine in `ask` mode):

```text
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
       Empty the list, item by item, within about five seconds. The answer comes at once and says `settled: false`; describe shows `clearing` until it is done. It cannot be undone.

$ yos act hello add text=milk count=2
Hello — 2 items
accepted: True, settled: True
revision: ec6faecfdbf7d9cf
{
  "added": "milk",
  "count": 2,
  "items": 2
}
(state omitted; `yos describe hello`, or re-run with --full)

$ yos act hello add text=eggs count=lots
yos: hello.app.act refused: `add` argument `count` must be an integer, and a string arrived
```

`yos act hello clear` would put an approval card in front of the person and act again with their
grant; the surface spends the grant through the shell before `clear` runs.

## What you did not write

The same as in Rust, because it is the same dispatch in another language — the same checks in the
same order, the same sentences to the punctuation, the same revision hash. The argument checks and
conversions, the answer with the view after it, `STALE:` for an act on a view that moved, the
machine's ceiling and the person's mode, spending a grant, who is calling (`caller()`) and for
which agent (`agent_token()`). The Python SDK replays the vectors generated from the Rust dispatch
in its own tests ([Checking your surface](checking.md#the-vectors)).

A call carrying an agent's token is held to that agent's **reach** as the Rust dispatch holds it
(step 3 of the protocol's order; [Choosing a grade](grades.md#an-agents-reach)), and the reach's
own vectors are replayed too.

## Test it in-process

`surface.describe_json()` and `surface.act({...})` need no socket. Point `HOME` at a directory of
the test's own to pin the ceiling and the mode. From the template's tests:

<!-- from: templates/python-surface/tests/test_my_surface.py -->
```python
    def test_remove_waits_for_the_person_in_ask_and_in_auto(self):
        self.act("add", title="Water the plants")
        self.assertTrue(self.refused("remove", index=0).startswith(
            "GRANT: my-surface.remove is graded `sensitive`"))
        # Auto runs `sensitive` unasked — but not what its own description says cannot be undone.
        self.mode_is("auto")
        self.assertIn("its own description says it cannot be undone", self.refused("remove", index=0))
        self.assertEqual(len(self.program.tasks), 1, "nothing was removed")
        # Bypass runs everything under the ceiling.
        self.mode_is("bypass")
        self.assertEqual(self.act("remove", index=0)["result"],
                         {"removed": "Water the plants", "left": 0})
```

Then hold the running program to the protocol with `yos check` — [Checking your surface](checking.md).

## When your state belongs to one thread

By default every describe and act runs on the connection's thread under one lock, which makes the
revision check and the handler one atomic turn. A program whose state belongs to one thread — a
GTK or Qt main loop, Blender's main thread — subclasses `Surface` and overrides
`run_on_app_thread(fn, timeout)` to run `fn` there. [Wrap an app you did not write](wrap-an-app.md)
walks through Blender's.

## Next

- Copy [`templates/python-surface`](../../templates/python-surface/README.md): a to-do list with a
  `safe`, two `standard` and a `sensitive` action, a `.desktop` file and tests.
- [Designing a describe a mind can use](describe.md), then
  [Being found while closed](found-while-closed.md).
- [`sdk/python/README.md`](../../sdk/python/README.md) is the package's full reference.
