# Two lints for the apps

Written 20 September 2026, out of the survey in
[design/apps-plan-2026-09-20.md](../../design/apps-plan-2026-09-20.md). Calendar and Images both
photographed as working and were facades; the survey of the other eleven apps found the same two
faults everywhere, and neither is visible to a casual read of either half of an app. These are the
two checks that plan asks for, "the machinery, before more apps".

Everything here is Python 3 standard library. It reads source text under `apps/`; it starts nothing,
opens nothing and writes nothing outside this directory.

```sh
cd tests/app-lints
python3 run.py                  # every app, graded against baseline.json
python3 run.py --app notes      # one app
python3 run.py --show-known     # the known debt, item by item
python3 run.py --show-warnings  # in-out properties never written from Rust
python3 run.py --ignore-baseline   # the whole debt, as if nothing were forgiven
python3 run.py --json           # the same result, machine-readable
python3 -m unittest discover    # the lints' own tests
```

`run.py` exits nonzero when there is a violation that is not in the baseline, when a baseline entry
has been fixed and not removed, or when the allowlist is malformed.

| File | What it is |
|---|---|
| `run.py` | Both lints over every app, graded against the baseline. The thing to run. |
| `lint_unset_properties.py` | Disease A. Also runs standalone. |
| `lint_dead_handlers.py` | Disease B. Also runs standalone. |
| `allowlist.toml` | Deliberate exemptions, each with a written reason. |
| `baseline.json` | Today's debt, so the check is adoptable without being switched off. |
| `appscan.py`, `srctext.py` | Finding the apps; masking comments and strings so brackets can be counted. |
| `test_lints.py` | 65 unit tests over fixture strings, plus the must-catch findings in this tree. |

## Lint 1 -- the `in` property nothing ever writes

A Slint window declares `in property <string> doc-file-path;`. Slint gives it the type's default.
The Rust side never calls the generated `set_doc_file_path`. The default is then drawn on screen and
read by everyone as a measurement.

The three that motivated it, all still in the tree:

- **Spreadsheet** never populated `cell-grid`, `row-count` or `col-count`. `row_count() == 0`, so
  the guards in cell-click, cell-edit and the formula bar all failed, behind a perfect blank 50x26
  grid nobody could type into. (Fixed in October 2026, PR #727: the app is clean under both lints.)
- **Network Manager** never sets `firewall-enabled` or `wifi-enabled`. "Firewall: Off", drawn in
  warning colour and recorded by the September audit as a finding, is a hardcoded default the app
  never looked up.
- **Document Editor** never sets `doc-file-path`, so Save always takes its early return and the
  `fs::write` beneath it is dead code.

The check, for every app with a `ui/app.slint`: parse the `in` and `in-out` properties declared
directly on the exported window component, and assert that something under `apps/<app>/src/` calls
the generated setter. Slint's `foo-bar` and `foo_bar` are the same identifier and both become Rust's
`set_foo_bar`.

Two severities, because the two kinds of property do not promise the same thing:

- an **`in`** property is written by Rust by definition. Never written is a **failure**.
- an **`in-out`** property may legitimately be driven by the UI alone -- a text field the user types
  into, a panel the user opens. Text Editor's `font-pixels`, `show-replace`, `match-case` and
  `syntax` are all of this kind and all correct. Never written from Rust is printed under
  `--show-warnings`, counted per app, and **does not fail the run**.

Deliberate decisions about false positives:

- **The receiver is never matched on.** A setter is a call to `.set_<name>(` or `::set_<name>(`
  anywhere in the app's source, whatever the handle is called -- `ui`, `u`, a weak upgrade, a helper
  function, `app.global::<ThemeMode>().set_dark(...)`. Insisting on `app.` would have failed almost
  every app in this repo, because almost every handler upgrades a weak handle first.
- **Comments and strings are masked first**, so a commented-out setter is not a call and a setter
  named inside a `tracing::info!` string is not a call either.
- **Only the window's own properties count.** A property declared inside a child element in the same
  file belongs to that child and has no generated setter on the window.
- **A meaningful static default is not distinguishable from a stale reading.** `in property <string>
  ending: "LF";` might be correct configuration or might be a line-ending indicator nobody measures.
  The lint cannot tell, which is what the allowlist is for.

## Lint 2 -- the callback that only says it was pressed

```rust
app.on_snip_copy(|id| {
    tracing::info!("Copy snippet {} to clipboard", id);
});
```

Copy is what a snippet manager is for. Nothing is copied. Snippets' Save is the same shape with a
different disguise -- `let _ = (code, tags); // suppress unused warnings` -- and the next selection
overwrites the editor from the stale model, so edits revert while you watch.

The check: find every `.on_<name>(` registration in the app's Rust, extract the closure body with
real brace and paren matching, and fail it when the body does nothing. "Does nothing" is exactly
four shapes and deliberately no more:

1. an empty body -- `|| {}`, `|_| {}`, a body that is only comments;
2. `tracing::{info,debug,warn,trace,error}!` (or `log::*!`) and nothing else;
3. `println!` / `eprintln!` / `print!` / `eprint!` / `dbg!` and nothing else;
4. `let _ = <inert>;` -- a discard whose right-hand side calls nothing.

Deliberate decisions about false positives:

- **A body that logs and also works is alive.** One statement that does something is enough.
- **Clause 4 is narrower than "any `let _ =`".** Download Manager's every handler is
  `let _ = settle(&ui, &engine, command(&engine, id));`, and it is the one app the September audit
  called a gold standard: the discard is of a `Result` whose work has already happened. So a discard
  counts as dead only when its right-hand side is *inert* -- identifiers, field accesses, tuples,
  literals, indexes -- which is what `let _ = (code, tags)` is and what `let _ = settle(...)` is not.
  Anything with a call, a macro, a `?`, a block or a control keyword in it is work.
- **A function path or a factory call is not read at all.** `app.on_ai_reply_suggest(handler)` and
  `app.on_dl_cancel(id_command(app, &engine, |e, id| e.cancel(id)))` hand the closure in from
  somewhere this lint cannot follow. They are reported as neither dead nor alive, because guessing
  would be worse than silence.
- **`ui.window().on_close_requested(...)` is skipped.** It is `slint::Window`'s own API, not a
  generated callback, and it is not a control anybody can press.
- **A callback returning a bare constant is not flagged.** `|| Mode::Compact` is not one of the four
  shapes. It may well be dead; the lint does not say so.
- **A callback registered in more than one place** is dead only when every closure registration of
  it is dead, and is left alone entirely if any registration delegates to a function.

## The baseline

The repo has 147 unset `in` properties and 226 dead handlers as this is written. A check that fails
on all of them on its first run gets switched off inside a week, so this one is graded against
`baseline.json`:

- a failure recorded in the baseline is **known debt**. It is counted and printed on every run so it
  stays visible, and it does not fail the run.
- a failure not in the baseline is **new rot**. It fails the run.
- a baseline entry with no matching failure has been fixed, and the run says
  `stale baseline entry -- remove it` and fails. The file cannot silently drift away from the truth,
  and the debt count can only go down.

Rewrite it with `python3 run.py --baseline`. That is the one operation that can make the numbers go
up, so it belongs in a commit of its own with a reason. Entries are keyed by property or callback
**name**, never by line number, so an edit anywhere above them does not invalidate the file.

The baseline in this directory was written on 20 September 2026 against
`desktop/mind-link-pins-location`. Apps were being fixed while it was written; if `run.py` reports
stale entries on a fresh checkout, those are items fixed since, and deleting their lines is the
whole of the fix.

| App | unset `in` | `in-out` never set | dead handlers | registrations |
|---|---|---|---|---|
| calendar | 11 | 4 | 7 | 19 |
| container-manager | 0 | 5 | 3 | 18 |
| document-editor | 16 | 12 | 53 | 60 |
| download-manager | 5 | 1 | 0 | 17 |
| email | 9 | 14 | 11 | 30 |
| image-viewer | 3 | 1 | 3 (+5 allowed) | 14 |
| music-player | 10 | 10 | 23 | 34 |
| network-manager | 34 | 7 | 19 | 24 |
| notes | 0 | 0 | 0 | 6 |
| presentation | 16 | 12 | 40 | 50 |
| snippet-manager | 12 | 6 | 16 | 19 |
| spreadsheet | 21 | 9 | 44 | 47 |
| system-monitor | 10 | 2 | 4 | 11 |
| terminal | 0 | 0 | 0 | 10 |
| text-editor | 0 | 4 | 0 | 7 |
| weather | 0 | 2 | 3 | 12 |
| **total** | **147** | **89** | **226** | **378** |

Notes and Terminal are clean on both lints. Text Editor is clean on both and carries only four
`in-out` properties the UI drives itself. Those three are the bar.

## The allowlist

`allowlist.toml` is the other exit, and a narrower one. The baseline says an item is wrong but old.
The allowlist says an item is **right**, and must say why in prose:

```toml
[dead-handler]
image-viewer = [
  { name = "toggle_fit", reason = "The shared viewer component applies fit to its own property when the button is pressed; the callback is only the notification that it did..." },
]
```

An entry without a reason is itself an error and fails the run. That is the point of the file: a
dead control has to be argued for in writing before it is exempted, and the argument is then in the
tree where the next person can disagree with it.

The one section here today is Image Viewer's five rotate, flip and fit handlers. They are empty on
purpose: the shared component already rotates its own property when its button is pressed, and the
app's handlers used to apply it a second time, which turned a 90-degree press into 180 -- see
`design/images-2026-09-20.md`. An item may be in the allowlist or in the baseline, never both; the
run reports the overlap as a stale baseline entry.

Names are as the lints print them: Slint spelling for properties (`doc-file-path`), the generated
Rust spelling without the `on_` for callbacks (`snip_save`).

## The shelf

`shelved.toml` is the third register, and it is about whole apps rather than items:

```toml
[music-player]
reason = """
Nothing plays audio. There is no playback engine, no scanner and no library behind the screen...
"""
```

A shelved app is in the tree and not in the build. Music and ySheets were taken off the shelf on
20 September 2026 -- removed from the launcher, the Lens, the command palette, the pins, the
release bundle and every name a mind could open them by, because neither had anything under its
screen. ySheets came back in October 2026 (#727) and Music on 16 October 2026 (#728); both are
linted like any shipped app again and both are clean on both lints. The file holds no entry
today, and the example above is Music's old one. See `design/shelved-2026-09-20.md`, and `SHELVED` in
`crates/yantrik-ui/src/wire/dock.rs`, which is the list the shell itself consults.

They are still workspace members, they still compile, and they are still linted. What this file
changes is where their debt is reported:

- their findings print under a **SHELVED** heading with the reason the app is on the shelf,
- they are left out of the shipping totals, so "how much debt is in this build" is a true number,
- and they do not fail the run. Nobody has been asked to fix an app that is not shipped, and a
  check that goes red every day about work nobody is doing is a check that gets switched off --
  which is why these lints are graded against a baseline to begin with.

An app named on the shelf that does not exist under `apps/` is an error, as is an entry with no
reason. Taking an app off the shelf has to be argued for in prose, the same way exempting a dead
control does.

The two apps carry 98 of the 203 findings the lints see, so the shipping total reads 105.

## What these lints cannot see

They are heuristics over source text. They do not build anything, they do not run anything, and they
have no idea what any of it means. Honestly:

- **A setter that is called but with the wrong thing is invisible.** `ui.set_row_count(0)` passes.
  So does a setter called only on a path that never executes, or inside a handler that is itself
  dead. Lint 1 asks whether the property is ever written, not whether the value is true.
- **A handler that does something useless is alive.** `ui.set_status("Saved".into())` with no write
  behind it is fabricated success, which is the survey's first disease, and no lint here looks for
  it. The conformance runner, which checks an action against the store, is where that belongs.
- **Nothing follows a call.** A handler that calls a helper is alive even if the helper is a stub. A
  setter reached only from dead code still counts as a call.
- **The window's own file is all that is parsed.** These apps alias almost every property onto a
  shared component in `crates/yantrik-ui-slint/ui/`, and a property that component declares but the
  window does not is out of scope, as is anything the shell's own screens do.
- **A property named like a model method would collide.** Setters are matched by name only, so a
  Slint property called `row-data` would be satisfied by any `VecModel::set_row_data` call in the
  app. No app has one; if one appears, this is where it will go wrong.
- **Globals are not distinguished.** `app.global::<ThemeMode>().set_dark(true)` counts as a write of
  a window property named `dark`, if one existed.
- **Macro-generated registrations are invisible**, and a closure assembled by a macro body cannot be
  read.
- **A dead handler that is allowlisted is still a dead control.** The allowlist records an argument,
  not a fix. Point 7 of the contract in the plan -- "a button does something or is absent" -- is not
  satisfied by a reason string.

The two lints are the cheap half of the contract. The conformance runner, which opens an app, does
its one job and checks the store, is the half that can tell whether any of it is true.
