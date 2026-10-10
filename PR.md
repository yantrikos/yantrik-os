# A Blender that is rendering is reported as a Blender that died

Branch: `qwen/95b-blender-render-describe-progress` · off `origin/main` at `0a366af`

Closes #95.

## What a person hit

`render` is published with `timeout=1800.0`, and its own description, three lines above at
`apps/blender/addon/yantrik_blender/surface.py:122`, says "With Cycles on a big scene this can
take minutes." A Cycles frame on a real scene runs for minutes.

While it runs, `bpy.ops.render.render` holds Blender's **main thread**, and the main thread is
the only thing that serves the app's control socket: `QueuedBridge.pump()` (`bridge.py:78`) is
what runs queued jobs, and it is reached from a `bpy.app.timers` callback in a window or the
bootstrap's own loop in `blender -b`. Neither turns up from inside a render op. So a caller's
`app.describe` sits in the queue, `submit`'s `job.done.wait(timeout)` expires (`bridge.py:67`),
`BridgeTimeout` becomes `NotAnswered` (`apps/blender/addon/yantrik_blender/surface.py:196`), and
the caller is handed:

```
-32000  app did not answer within 10s
```

That is the sentence for a dead app, said about an app doing exactly what it was asked to do.
Three things go wrong in that window — the one window a render guarantees:

- **The gate refuses to ask the person.** Before an act graded `sensitive` or `dangerous` puts
  a card up, `crates/yantrik-ui/src/control_approvals.rs:885` does an `app.describe` with a 500 ms
  timeout to read the action's grade. Mid-render it fails, and the act dies *before* the card:
  "`blender` did not say what `save` is graded (…), so nothing was put in front of the person."
  So while Blender renders, `save`, `open` and `run_python` cannot be approved at all — not
  refused by the gate, never offered by it.
- **`open_app`'s glance goes empty.** `crates/yantrik-ui/src/app_glance.rs:23` asks
  `app.describe` for the app's summary and action list, 800 ms, and returns `None` when nothing
  answers — so a mind that opens a rendering Blender is told nothing about it, which is the exact
  failure `app_glance.rs`'s own header was written to fix.
- **A harness polling to see how the render is going is told the app is gone** — the one question
  it was asking.

## What this changes

The rule: **a describe may be answered from what the app kept readable while its thread was
busy; an act may not.** A describe asks for nothing, so a state beside it is a fact. An act whose
turn never came has done nothing, and a state beside it reads as "done".

### `sdk/python/yantrik_surface/surface.py` — one override point, `busy_answer`

`describe_json` now passes a `busy` callback into `_turn` (`surface.py:744`). On `NotAnswered`,
`_turn` gives the app one chance to say what it is doing before it falls back to the transport's
words — and only for a caller that passed `busy`, so `act` is untouched and keeps failing exactly
as it did.

`busy_answer()` (`surface.py:667`) is the new app hook. It defaults to `None`, which is the
timeout, so every app that does not implement it is unchanged. It is documented as running on
the **caller's** thread and never the app's, so nothing it reads may need the app's thread —
that is the whole point of it.

The busy answer is a real describe, not a stub: the same `actions` list, so a caller can still
plan what to ask after the frame, and a `revision` computed from the summary and state it is
actually returning (`_as_read`, `surface.py:715`), so an act carrying it is guarded against the
state it was given and not some other one.

### `apps/blender/addon/yantrik_blender/scene.py` — the render says what it is before it takes the thread

`_do_render` publishes `self.rendering = {"output", "started", "as_of"}` (`scene.py:871`) one
statement before the render op takes the thread and clears it in a `finally`. `as_of` is
`self.snapshot()`, read **on the main thread** at the last moment reading is possible — the
freshest scene anyone will get until the render finishes.

`busy_snapshot()` (`scene.py:404`) builds the answer from that alone: no `bpy` call, so the
socket's own thread may run it. It returns the as-of state plus a `rendering` block —

```
Blender — 3 objects, cycles 1920x1080, rendering to hero.png (87s)

state: {"objects_total": 3, …, "rendering": {"output": "/tmp/hero.png", "seconds": 87}}
```

— and `None` when nothing is in flight, which leaves the timeout standing for every other kind
of silence. The summary says what is running and for how long rather than passing the as-of
picture off as current, because the state beside it cannot be more current than the moment the
render took the thread.

`_do_render`/`_render` is a split so the publish/clear wraps the whole op without re-indenting
it. Nothing about what a render does changed.

### `apps/blender/addon/yantrik_blender/surface.py` — Blender's side of the hook

`busy_answer` returns `self.scene.busy_snapshot()` (`surface.py:181`), plus a paragraph in the
module docstring saying why the file has a second way to answer a describe.

## How this was verified

`tests/blender-core/test_dispatch.py` — `TestDescribeDuringARender`, 3 cases through a bridge
that stops answering the moment the render op is entered and starts again when it leaves. The
hold is what makes them test the busy path and not the happy one: anything that reaches for the
main thread inside that window meets the timeout and comes back an error.

- a describe mid-render **is answered**: the as-of summary plus `rendering to x.png (0s)`, the
  `rendering` state block, the revision of the state actually returned, all 14 action names still
  listed;
- the render **still gets its own answer** when the thread comes back — `accepted`, the path,
  non-zero bytes. Answering from the busy path does not cost the render its result;
- an act mid-render **still fails** with `app did not answer within 30s`, unchanged.

`tests/blender-core/test_scene.py` — one case with a spy **inside** `bpy.ops.render.render`, the
only place a mid-render describe can be observed: the summary and state are the as-of ones, the
`rendering` block is there, and `busy_snapshot()` is `None` again once the render is over.

`sdk/python/tests/test_refusals.py` — the same contract for any app written against the SDK, with
a `Busy` surface that reports an export in flight: described as busy, revision of the state it
gave, actions intact, and its `act` still refused with the transport's words. The other 33 tests
in that file cover the default — an app with nothing to say still times out.

**Suites run:** `sdk/python/tests/test_refusals.py` 34 passed; `sdk/python/tests/test_later.py`
5 passed; `tests/blender-core/test_dispatch.py` + `test_scene.py` 128 passed, 2 failed; the six
socket-free SDK files 77 passed, 11 failed; `tests/blender-core` as a whole 143 passed, 13
failed. Every failure is environmental and accounted for below.

## What I could not verify

This machine is Windows and this suite is Linux-only (Unix domain sockets, `grp`/`pwd`, and no
`AF_UNIX` in this Python at all). I ran the pure-logic tests against a throwaway stub for
`socketserver.ThreadingUnixStreamServer`, deleted before committing. **Nothing was run on Linux
or against a real Blender.**

- **The 2 failures in `test_scene.py` are path normalization, not this change.**
  `test_open_refuses_a_file_that_is_not_there` (`:541`) and
  `test_a_missing_file_is_refused_by_name` (`:324`) assert a refusal naming
  `` `/tmp/no-such.blend` `` and get one naming `` `F:\tmp\no-such.blend` ``, because
  `os.path.abspath` (`scene.py:776`, `:936`) rewrites it here. Both sites are outside the three
  hunks this branch adds.
- **The other 11 are the transport this box has no equivalent of**: 8 in `test_wire.py`
  (socket dir, server roundtrip), 2 in `test_startup.py` (a real AF_UNIX server), 1 in
  `test_release_layout.py` (the bootstrap's socket beside the SDK).
- **The 11 in the SDK batch** are 10 in `test_example.py` against `os.getuid`, which does not
  exist here (`yantrik_surface/wire.py:231`, `"/tmp/yantrik-%d" % os.getuid()`), and 1 in
  `test_gate.py` that splits a path on `"/"`.
- **`sdk/python/tests/test_stopping.py` hangs here** — it waits on the Unix transport. Not run.
- **The 87-second render in the example above is illustrative.** The `(87s)` line is
  `int(time.monotonic() - started)`; the tests pin the format at `(0s)` because a fake render op
  is instant. How the number behaves over minutes of real Cycles is unobserved.
- **The three shell-side symptoms are read off the code, not reproduced.** The grade-lookup
  failure and the empty glance follow from the timeouts named above and the `describe` call at
  each site; I did not run the shell with a rendering Blender attached.

## Next thing somebody should look at

**The Rust runtime has the same hole and this branch does not touch it.**
`crates/yantrik-app-runtime/src/control.rs:517` does `rx.recv_timeout(UI_ROUNDTRIP)` — 3 seconds,
tighter than Blender's 10 — and on expiry `unanswered()` raises `-32000 "app did not answer
within 3s"` for `app.describe` as readily as for `app.act` (`control.rs:596`). Any Rust app whose
UI thread is tied up with something it could describe — an export, an import, a transcode — is
reported dead in exactly the way Blender was. The fix has the same shape: a `busy_answer`-shaped
hook on the registry, consulted on the timeout path for `app.describe` only. The Python SDK is
the port of that crate, so the two should not drift on this.

Two smaller ones found while reading and deliberately left alone:

- `crates/yantrik-ui/src/app_glance.rs:30` ends with `.ok()?`, so a describe that *answers slowly
  but correctly* and a describe that never answer both produce `None`. With this change Blender
  answers mid-render, so the case is rarer; but an 800 ms budget against a 30 s default
  `act_timeout` app is still a thin margin, and the caller cannot tell "busy" from "absent".
- `busy_snapshot` reports `seconds` as an `int` and the summary repeats it. A render that has run
  an hour says `(3600s)`. Fine for a line a mind reads; worth knowing before anything tries to
  format it for a person.

---

Co-Authored-By: Claude Code <noreply@anthropic.com>

🤖 Generated with [Claude Code](https://claude.com/claude-code)
