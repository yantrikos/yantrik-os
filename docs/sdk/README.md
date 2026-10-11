# Putting your program where a mind can find it

Yantrik OS is Debian underneath, so every Debian program installs and runs. What is different is
that the person has **minds** working for them — AI agents that run on the machine, from the
built-in companion to a coding agent a person started — and a mind can **find** a program,
**read** what it shows, and **act** in it, under rules the person set. A program takes part by
publishing a **surface**: a unix socket on which it answers two questions.

- **`app.describe`** — what are you holding? One line a person could read (the *summary*), a small
  object in the program's own words (the *state*), a fingerprint of both (the *revision*), and the
  *actions* it offers, each with its arguments and a **grade**.
- **`app.act`** — do this. The call names an action and its arguments; the answer carries the
  action's result and the view after it.

Every call to `app.act` meets the same rule, whoever sent it: the action's grade (`safe` <
`standard` < `sensitive` < `dangerous`) against the machine's **ceiling**, the person's **mode**
(`plan`, `ask`, `auto`, `bypass`, `bypass_all`), and any **grant** — a person's Allow for exactly one call. You
do not write that rule. The SDK carries it, in Rust and in Python, word for word the same, and so
does every door a mind comes through.

A mind finds a running surface by its socket. It finds a closed one by the program's `.desktop`
file, which every Debian program already ships, with a few keys added.

## Read in this order

1. **[Rust quickstart](rust-quickstart.md)** or **[Python quickstart](python-quickstart.md)** —
   the smallest complete surface, run and driven the way a mind drives it.
2. **[Choosing a grade](grades.md)** — what each grade is for, with the grades this OS's own apps
   chose and why, and what the ceiling, the mode and an agent's reach do to a call.
3. **[Designing a describe a mind can use](describe.md)** — the summary, the state, the action
   descriptions, `expected_seconds`, "settles later", and what makes a mind pick the right action.
4. **[Wrap an app you did not write](wrap-an-app.md)** — Blender, from inside; LibreOffice, from
   beside it.
5. **[Being found while closed](found-while-closed.md)** — the `.desktop` keys.
6. **[Checking your surface](checking.md)** — `yos check`, and the vectors every implementation
   replays.

The normative protocol is [docs/surface-protocol.md](../surface-protocol.md); where this guide and
it differ, it is right. [docs/app-control.md](../app-control.md) covers the apps inside this
repository.

## What to start from

| | Rust | Python |
| --- | --- | --- |
| the SDK | [`crates/yantrik-surface`](../../crates/yantrik-surface/src/lib.rs) | [`sdk/python/yantrik_surface`](../../sdk/python/README.md) (standard library only, Python 3.11+) |
| the smallest example | [`examples/hello-surface`](../../examples/hello-surface/src/main.rs) (and [`examples/hello-window`](../../examples/hello-window/src/main.rs) with a window) | [`examples/hello_surface.py`](../../examples/hello_surface.py) |
| a template to copy | [`templates/rust-surface`](../../templates/rust-surface/README.md) | [`templates/python-surface`](../../templates/python-surface/README.md) |
| a program somebody else wrote | | [`apps/blender/addon`](../../apps/blender/addon/yantrik_blender/surface.py), [`adapters/libreoffice`](../../adapters/libreoffice/README.md) |

## How the code here is kept true

Every code block in these pages is quoted from a file this repository builds and tests, or is the
output of running something. [`samples/test_guide.py`](samples/test_guide.py) finds each quote in
its file, line for line, regenerates each output, and fails when either has drifted; CI runs it.
[`samples/tests/desktop_files.rs`](samples/tests/desktop_files.rs) reads every `.desktop` file the
guide quotes with the shell's own parser. The transcripts of `yos` sessions were captured from
real runs of the examples (socket paths shown as a desktop session has them, long refusals cut
with `…`).
