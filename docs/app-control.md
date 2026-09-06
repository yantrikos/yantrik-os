# The app control surface

Every Yantrik app publishes what it is holding, and what it can be asked to do, over the same
JSON-RPC socket bus the services use. Two methods:

```
app.describe {}                → { app, summary, state, actions: [...] }
app.act      { action, args }  → { result }
```

## Three ways to see, in order

1. **`app.describe`** — our own apps, which publish their state because we wrote them.
2. **AT-SPI** — everyone else's. GTK, Qt, Chromium and Firefox publish their whole widget tree over
   D-Bus for screen readers, and have for twenty years. `a11y-service` reads it; the companion sees
   `list_readable_windows`, `describe_window` and `window_action`.
3. **`analyze_screen`** — a screenshot and a vision model, for what neither can see: a game, a
   video, a remote desktop, an application with no accessibility bridge.

The rule below was first written as *semantic for ours, visual for theirs*. That was too
pessimistic: most of theirs is semantic too, and vision belongs third rather than second. A zenity
dialog reads as eleven elements in **1.4 KB** with no GPU — and its Cancel button can be pressed by
name, through the same interface a screen reader uses, with no synthetic pointer.

## Why, rather than a screenshot

The companion can already see the desktop: `grim` takes a picture, the PNG is base64'd and posted
to a vision model, and the model reports what the pixels look like.

That is the wrong answer for our own windows. Notes knows exactly which note is open. Asking a
vision model to *infer* that from a photograph of a window we wrote is slow, lossy, expensive, and
needs a GPU that a VPS does not have. So an app says it instead: a few hundred bytes of exact
truth, always current.

**Read it if it will tell you; photograph it only if it will not.** Ours tell you because we made
them; most of theirs tell you because screen readers needed them to.

`act` is the same surface turned around. An app already exposes its capabilities to its own
buttons; publishing them by name means driving our own software never needs a synthetic mouse
either.

## What a caller sees

```
$ list_apps
Open Yantrik apps (3):
notes: Notes — “Kernel asks”, 412 words, unsaved
  can: open_note, new_note, save, append, search, set_folder
shell: Yantrik — desktop screen, 2 windows open, calendar, email and notes not running
  can: open_app, show_screen, focus_window, set_do_not_disturb, lock
system-monitor: System — Healthy, CPU 4%, memory 9% (2.7 GB of 31.3 GB), up 8d 8h
  can: sort_processes, filter_processes, kill_process
```

Three tools reach it: `list_apps` and `describe_app` are Safe, `app_action` is Standard.

## Adding a surface to an app

```rust
use yantrik_app_runtime::control::{Action, App, Param, View};

App::new("notes")
    .describe({
        let ui = app.as_weak();
        move || {
            let Some(ui) = ui.upgrade() else { return View::new("Notes — closing") };
            View::new(format!("Notes — “{}”, {} words", ui.get_current_title(), ui.get_meta_word_count()))
                .with("open_note", ui.get_current_title().to_string())
                .with("unsaved", ui.get_is_modified())
        }
    })
    .action(
        Action::new("open_note", "Open a note by title").arg(Param::text("title")),
        move |args| { /* … */ Ok(serde_json::json!({ "opened": true })) },
    )
    .serve();
```

Call `serve()` from the thread that owns the window, **after** the app's initial load — a first
`describe` should not catch a half-built window — and before `run()`.

### Rules that are not optional

**Every action calls the callback the button calls.** `ui.invoke_open_note(…)`, not a
reimplementation. One code path, so an action cannot drift away from what the app actually does.

Watch for two-way bindings. A Slint toggle usually flips its own `in-out` property and *then*
calls the callback, so `invoke_toggle_units()` alone is a no-op — the action has to do both halves
in the same order the switch does. Read the `.slint` before assuming `invoke_x()` is the whole
gesture.

**The summary is one line a person could read.** A caller surveying every open window pays one
line per app, not sixteen state objects. Put the thing worth knowing first: a failed service leads
the shell's summary ahead of the window count, because trouble is the reason to look.

**The state is a glance, not a transcript.** Titles, not bodies. Ten processes, not four hundred.
The largest thing a surface carries — a log tail, a document body — goes in only when the caller
is plainly looking at it, or every describe pays for it.

**Say what is true, including when it is nothing.** An app whose data is a stub should not publish
a surface over it. A vision model guessing at a screenshot is bad; a confident lie in JSON is
worse.

### Risk

Every action declares its own, in the companion's vocabulary — `safe`, `standard`, `sensitive`,
`dangerous` — and the default is `standard`:

```rust
Action::new("kill_process", "End a running process by pid").risk("dangerous")
```

Apps do not have one risk level: reading which note is open and killing a process arrive through
the same door. `app_action` is itself only Standard, so without per-action risk it would be a door
to everything any app publishes. It reads the app's declaration and refuses anything above the
configured ceiling:

```
Permission denied: 'system-monitor.kill_process' is declared dangerous but max is standard
Permission denied: 'shell.lock' is declared sensitive but max is standard
```

An action with no declaration is treated as `standard`. An unknown risk is never treated as no
risk.

Grade within an app rather than across it. Starting a container is recoverable and stays
`standard`; stopping one interrupts what it was serving and is `sensitive`; removing one takes its
writable layer with it and is `dangerous`.

**Some things do not belong on the surface at all.** Email publishes `compose` and not `send`: a
draft can be read before it leaves, and mail that has gone cannot be taken back. The terminal
publishes no way to run commands, because `run_command` already does that on a worker thread while
this terminal would block the UI thread for the length of the command. A second, worse path to
something we already do properly is not a feature.

## Threading

Both closures run on the UI thread, because that is the only thread allowed to touch a Slint
window. The RPC server runs on its own thread and hands work across with
`slint::invoke_from_event_loop`, then waits on a channel with a three-second budget.

That budget exists for a UI thread that is genuinely stuck. Reading a handful of properties costs
microseconds, and taking the answer from the live UI is the whole point — a cached copy would be
exactly the stale second-hand account this replaces.

An action that would take real time must not run inline. Do what the app's own button does and
hand off to a worker.

## Sockets

Service ids are prefixed `app-<id>`, so `app-notes.sock` beside `notes.sock`. notes-service stores
notes; the app is the window someone is looking at. They are different things and must not share a
socket.

The id is the app's own name, not its binary's: Container Manager publishes `containers`, because
that is what a caller would think to ask for.

`list_apps` finds surfaces by listing `app-*.sock` in the session's socket directory. A socket file
outlives a crash, so anything that fails to answer is simply left out — a stale socket is not news.

## Verifying one

`scripts/app-control-probe.sh` starts every app with a surface and drives it from an unrelated
process, which is the position the companion is in. It checks the contract each surface owes
regardless of app — it names itself, publishes a non-empty summary, some state and some actions;
every action declares a valid risk and types every argument; an unknown action lists the real ones;
a missing argument is named rather than guessed at — and then a few things specific to each app.

`scripts/companion-tool-probe.sh` drives the whole chain from outside the shell: socket → companion
RPC → worker → tool registry gate → app socket → the window's own callbacks, and asserts that a
`dangerous` action is refused under a `standard` ceiling.

Add both when you add a surface. Two real bugs surfaced in the writing of these: Weather's
`set_units` refetching in the units already showing, and Notes' search silently leaving the whole
list on screen when notes-service was down.

## Surfaces today

| App | Publishes | Notably |
| --- | --- | --- |
| `shell` | screen, windows, services, status bar, bond | a failed service leads the summary |
| `notes` | open note, word count, unsaved, vault list | `append` saves; AI suggestions do not |
| `email` | folder, unread, selected message with body | composes, never sends |
| `calendar` | month, selected day's events, busy days | |
| `system-monitor` | health, CPU, memory, disks, top processes | `kill_process` is `dangerous` |
| `weather` | conditions, alerts, forecast, units | reports what the user sees, not a fresh query |
| `containers` | containers, images, volumes, open log tail | `stop` sensitive, `remove` dangerous |
| `terminal` | directory, last command and its exit code | read-only by design |

Windows from other applications are not in this table and never will be: they publish through
AT-SPI instead, and `a11y-service` reads whatever the toolkit chose to expose. What Yantrik
controls there is not the contents but the order in which the three ways are tried.
