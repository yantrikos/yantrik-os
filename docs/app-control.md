# The app control surface — a guide

Every Yantrik app publishes what it is holding, and what it can be asked to do, over the same
JSON-RPC socket bus the services use. This page is the guide: why, how to add a surface, and the
judgement calls. **The protocol itself — framing, names, envelopes, the revision, every refusal
and error code, word for word — is [surface-protocol.md](surface-protocol.md)**, version 1, with
JSON Schemas in [schema/](schema/) and the permission rule generated from the code into
[surface-vectors.json](../deploy/yantrik-os/surface-vectors.json). Where this page and that one
differ, that one is right.

Two methods:

```
app.describe {}
  → { protocol, app, summary, state, revision,
      actions: [{ name, description, permission, settles, parameters }] }

app.act { action, args?, expect_revision?, grant?, agent_token? }
  → { app, action_id, accepted, settled, result, revision, summary, state }
```

`accepted` is always true and never means done; `settled: false` means the work was started and
has not finished. `revision` is a fingerprint of the summary and the state; `expect_revision` makes
an act refuse (`STALE:`) if the app has moved on since the caller looked. `grant` carries a
person's Allow; `agent_token` says which of the person's agents a call is for. The spec has the
detail of each.

## Three ways to see, in order

1. **`app.describe`** — our own apps, which publish their state because we wrote them.
2. **AT-SPI** — everyone else's. GTK, Qt, Chromium and Firefox publish their whole widget tree over
   D-Bus for screen readers, and have for twenty years. `a11y-service` reads it; the companion sees
   `list_readable_windows`, `describe_window` and `window_action`.
3. **`analyze_screen`** — a screenshot and a vision model, for what neither can see: a game, a
   video, a remote desktop, an application with no accessibility bridge.

The rule was first written as *semantic for ours, visual for theirs*. That was too pessimistic:
most of theirs is semantic too, and vision belongs third rather than second. A zenity dialog reads
as eleven elements in **1.4 KB** with no GPU — and its Cancel button can be pressed by name,
through the same interface a screen reader uses, with no synthetic pointer.

## Why, rather than a screenshot

The companion can already see the desktop: `grim` takes a picture, the PNG is base64'd and posted
to a vision model, and the model reports what the pixels look like. That is the wrong answer for
our own windows. Notes knows exactly which note is open. Asking a vision model to *infer* that
from a photograph of a window we wrote is slow, lossy, expensive, and needs a GPU that a VPS does
not have. So an app says it instead: a few hundred bytes of exact truth, always current.

**Read it if it will tell you; photograph it only if it will not.** `act` is the same surface
turned around: an app already exposes its capabilities to its own buttons, and publishing them by
name means driving our own software never needs a synthetic mouse either.

## What a caller sees

```
$ yos describe calendar
Calendar — September 2026, 3 events on the 23rd
revision: <16 hex digits>
{ … the state, one key per line … }
  act: delete_event(id?, title?, date?)  [sensitive, settles on return]
       Take an event off the calendar. It is not recoverable
  …
```

(Illustrative; `yos describe <name>` on a running desktop prints the real one.) Minds reach the same
surface through the MCP bridge (`os_apps`, `os_describe`, `os_act`) and the built-in companion's
`list_apps`, `describe_app` and `app_action`.

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
`describe` should not catch a half-built window — and before `run()`. The runtime publishes
`protocol: 1`, computes the revision, and runs every check in the spec's order before your handler
sees a call. Then run `yos check notes` (below) and fix what it says.

**A surface with no window** — a service, an adapter for somebody else's program, a tool — uses the
same dispatch without Slint: `yantrik_surface::Surface` (re-exported by `yantrik-service-sdk`).
`Surface::new(id).describe(…).action(…).serve()` binds `app-<id>.sock`; a service that already
serves methods of its own keeps its `ServiceHandler` and hands `app.describe` / `app.act` to
`Surface::answer`. `examples/hello-surface` is the smallest complete one, and [`docs/sdk/`](sdk/README.md)
the guide for writing one.
Like any app, it is found while closed by the keys in its `.desktop` file (below).

**Parameters are typed, and the type is checked.** `Param::text`, `number`, `integer`, `flag`,
`one_of` (an enum), `array` (of a type) and `object`, each optionally `.default(…)`; `describe`
publishes them as JSON Schema, and your handler always reads the declared type: what a caller
sends that converts without loss is converted (`"12"` for an integer, `67` for text), and anything
else is refused before your handler runs (spec §4, §5 step 7). Declare what the handler reads: an
id read with `as_i64` is an `integer`, not a `number`. `Action::expected_seconds(n)` tells a caller how long to wait.

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
is plainly looking at it, or every describe pays for it. Keep very small and very large floats out
of it: they are where ports compute the revision differently (spec §6).

**Say what is true, including when it is nothing.** An app whose data is a stub should not publish
a surface over it. A vision model guessing at a screenshot is bad; a confident lie in JSON is
worse.

**No secret travels in `args`.** `args` is what the approval card draws and the audit log keeps. A
passphrase, a PIN or a password among them is shown to whoever reads either; `yos check` fails a
parameter named like one.

### Grades, and who is asked

Every action declares its grade — `safe`, `standard`, `sensitive`, `dangerous` — and the default is
`standard`:

```rust
Action::new("kill_process", "End a running process by pid").risk("dangerous")
```

Apps do not have one risk level: reading which note is open and killing a process arrive through
the same door. Grade within an app rather than across it. Starting a container is recoverable and
stays `standard`; stopping one interrupts what it was serving and is `sensitive`; removing one
takes its writable layer with it and is `dangerous`.

**The description counts too.** Say in it when an action cannot be taken back — "It is not
recoverable", "cannot be undone", "permanently" (the spec lists the seven phrases). The dispatch
reads it: in every mode but bypass, such an action is asked about whatever its grade above `safe`,
and no session rule covers it. Calendar's `delete_event` is `sensitive` and says so; before this
rule reached the dispatch, `yos act` ran it in auto with nobody asked while the MCP bridge and the
shell would have asked.

The check is the app's own dispatch, the same on every door — the MCP bridge, `yos act`, the
companion, and a raw JSON-RPC client on the socket all meet it. Above the machine's ceiling
(`tool_permission` in `settings.yaml`) nothing runs:

```
CEILING: system-monitor.kill_process is graded `dangerous`, above this machine's `sensitive`
ceiling (`tool_permission` in ~/.config/yantrik/settings.yaml), so it was not run. An action at
that grade needs a person to authorise it directly — raise the ceiling in Settings if that is the
intent.
```

Under it, the person's mode decides what runs unasked, and anything past that needs a **grant** —
the `request_id` the shell's `request_approval` minted and a person's Allow turned into one:

```
GRANT: calendar.delete_event is graded `sensitive` and its own description says it cannot be
undone, and this machine is in auto mode, which asks before anything that cannot be undone — so it
was not run. Ask the shell for approval first (`request_approval` with this app, action and these
exact arguments, poll `approval_status`, then send the granted request_id as `grant` on app.act —
`yos act` does all of that for you), or have the person at the machine press Allow when the card
appears.
```

`yos act`, the MCP bridge and the companion's `app_action` put the card up, wait for the person,
and act again with the grant. The dispatch spends a grant through the shell only once the ceiling
has passed — an Allow is never used up on an act the ceiling refuses — and only after checking
that the process answering as the shell is the shell. `describe` needs nothing.

The companion also holds itself to its own cap, `tools.max_permission`, before it sends anything:
`Permission denied: 'system-monitor.kill_process' is declared dangerous but max is standard`. That
is the companion's courtesy on the same ladder, not the boundary; the dispatch's is.

In `plan` mode the shell raises no card, so nothing above `standard` runs on any door. Plan's
refusal of `standard` itself stays the MCP bridge's and the shell's: the desktop's own processes
make `standard` calls on these sockets (an app starting its service through the shell's
`start_service`, a second launch handing its file to the open window), and the dispatch cannot
tell them from a mind until callers carry an identity (#43). That is the one place the doors
differ, and the vectors name it.

A mind running as one of the person's agents also carries an **agent token** beside `args`, the
same way: `{action, args, grant?, agent_token?}`. It says which agent the call is for, and it is
never an argument. The dispatch hands it to the handler as `control::agent_token()`, and removes any
`agent_token` a caller put inside `args`. `yos act` sends it from `--agent-token` or
`YANTRIK_AGENT_TOKEN`. The shell's `agent_run` / `agent_job` / `agent_input` / `agent_kill` resolve
it against the kernel's account of the caller (see `design/agents-workspace-2026-09-23.md`,
decision 3). The MCP bridge started with `YANTRIK_AGENT_TOKEN` passes it to `yos` through the
environment — never on the command line — and offers the agent's terminal as `run_command`,
`command_status`, `command_input` and `command_kill`. See [harness.md](harness.md).

The same token decides which agent's pane an approval card is drawn in (`request_approval` puts
it on `Verified.agent`, and the card names it), and who is asking for `new_agent` (sensitive),
`send_to_agent` and `stop_agent` (standard), and `read_agent` and `show_agent` (safe) —
`control_agents.rs`. A call with no token is the person's; one whose token is not believed is
refused. `consume_approval` refuses a grant asked for another agent when the caller carries a
token. An app's own dispatch spends a grant without one (#116), so a grant is not yet bound to
its agent on that path.

`shell.run_recipe {recipe, inputs?}` starts a recipe — a formation among them, whose Agent steps
hand work to catalog roles through `hand_off` (see [harness.md](harness.md), Formations). It is
**sensitive**: it starts agents. Its run is given the leave for its agents that the companion's own
`run_recipe` tool, graded standard, is never given. `answer_recipe`, `pause_recipe`,
`resume_recipe` and `cancel_recipe` are standard.

An agent started from a role in the agent catalog (`shell.hand_off`) is also held to the role's
**reach** — the surfaces it may touch (`notes`, `shell.agent_run`, `shell.agent_*`) and a grade
ceiling narrower than the machine's. Every door that lifts a token checks it before any grant is
spent and before the handler runs: a window's dispatch here, a service's through the same
`yantrik-surface` dispatch (`yantrik_service_sdk::reach::permits` for one that answers `app.act`
itself). An act outside it is refused with `REACH:` and a sentence naming the role, what it may
touch and what it may open (`yantrik_ipc_transport::reach`).

A role may **open the apps its reach names**: `shell.open_app name=notes` (and `show_app`, which
brings an open one forward) is within a reach that names `notes`, `notes.<action>` or
`notes.<prefix>*`, whatever the role's ceiling — opening a window is not an act on the app's data,
and a Planner held to `calendar, notes · safe` must be able to read them while they are closed.
The name is resolved the way `open_app` opens it and the catalog names apps, so an alias works
(`text-editor` is the Editor), and nothing else is opened on a reach's say-so: not another app,
not a screen, not the launcher, not the browser. Inside the app the role is held to its ceiling as
before, and the machine's ceiling and the person's mode still decide the opening itself. The role's
first turn says which apps it may open.

The **shell keeps every agent's reach**, as it keeps grants. A door in another process asks it
what a token is — `agent.reach {token_sha256}` on `app-shell.sock`, by the token's SHA-256, never
the token, answered on the shell's RPC thread and only by a `yantrik-ui` process — and is told
`held` (with the reach), `plain` (a live agent with no role, which only the gate decides for) or
`unknown`. It fails closed: a token no live agent carries — its agent stopped, or the shell
restarted since it was handed out — is refused, and so is every token-carrying call while the
shell does not answer. A call with no token is the person's own and asks nothing. (The reach used
to be a file, `~/.config/yantrik/agent-reach.json`, that each door read; a missing file meant no
reach for anyone, so anything running as the person could delete it and lift every reach (#189).
The shell now removes a file an older build left.)

The rule lives in `yantrik_ipc_transport::gate` (re-exported as `yantrik_app_runtime::control`), so
a service that answers `app.act` in its own handler meets it too, without linking Slint. System
Monitor, Notifications and Weather do: each lifts an agent token off the call, then calls
`gate::permit` with the grade **and the description** from the table it publishes in `describe`,
and refuses in the same words. Their dispatch is otherwise their own and not yet the protocol's
(the spec's *Known deviations*); the surface SDK moves them onto the shared one.

**Some things do not belong on the surface at all.** Email publishes `compose` and not `send`: a
draft can be read before it leaves, and mail that has gone cannot be taken back. A second, worse
path to something we already do properly is not a feature either.

The terminal is the worked example of that judgement being revisited. It published no way to run
commands, because the companion's `run_command` already did that on a worker thread. What changed
the answer was noticing what `run_command` cannot do — run something in the window the person is
*looking at*, so they see what the agent ran. That is a different capability, so `run` exists, is
`sensitive`, and shares one `exec_command` with the key handler.

**And some things should not have a surface yet.** The download manager had none while every
button only logged a line, and got one once a real transfer engine was behind it.

## Threading

Both closures run on the UI thread, because that is the only thread allowed to touch a Slint
window. The RPC server runs on its own thread and hands work across with
`slint::invoke_from_event_loop`, then waits on a channel with a three-second budget
(`app did not answer within 3s`, `-32000`, when the window is stuck).

An action that would take real time must not run inline. Do what the app's own button does and
hand off to a worker, and declare it with `.defers()` so it publishes `settles: later` and answers
`settled: false`. When the caller is owed the *result* of that time — an exit code, not "started" —
hand the rest of the answer to `control::answer_later`.

## Sockets and names

`app-<id>.sock` for an app, `<id>.sock` for a service, in the first directory of the chain
`$XDG_RUNTIME_DIR/yantrik` → `/run/yantrik` → `/tmp/yantrik-<uid>` (spec §2). notes-service stores
notes; the app is the window someone is looking at. They must not share a socket.

The id is the app's own name, not its binary's: Container Manager publishes `containers`. It is
also `container-manager` in `/opt/yantrik/bin`, in the launcher and in `open_app`, so its
`.desktop` file says so (`X-Yantrik-Aliases`, below), the shell links each alias at the socket the
app binds, and `yos ls` shows them beside the app. A symlink rather than a second listener, because
it is one surface.

A name belongs to whoever is answering on it. A second copy of an app, or a service started twice,
used to take the socket from the running one; now the bind pings first and refuses to start while
anything answers, saying which instance owns the name. Only a socket left by a crashed process is
replaced.

## Findable while closed: the `.desktop` keys

A running surface is found by its socket. A closed one is found by its `.desktop` file — the file
every Debian app already ships — with four keys in its `[Desktop Entry]` group:

```ini
X-Yantrik-Surface=libreoffice
X-Yantrik-Purpose=Documents, spreadsheets and slides: open, read, edit and export them
X-Yantrik-Aliases=writer;calc;impress
X-Yantrik-Adapter=/usr/lib/yantrik/adapters/libreoffice
```

- **`X-Yantrik-Surface`** — the id the surface publishes as `app` and binds as `app-<id>.sock`:
  lowercase words of letters and digits joined by `-`. Anything else is refused (and logged), not
  rewritten.
- **`X-Yantrik-Purpose`** — what the app is *for*, in one line. It is what a mind reads when it
  chooses between apps it cannot see, so write it against the app it is most likely to be confused
  with: Studio's says it *makes* pictures, Images' that it *views* them.
- **`X-Yantrik-Aliases`** — other names, `;`-separated. The shell links each one at the surface's
  socket (`app-<alias>.sock` → `app-<id>.sock`), so `yos describe <alias>` works for any surface,
  whatever it was written with. A name the desktop answers to (`shell`, a screen, a section of
  Settings) or that another app already holds is not handed out twice: ids are settled before
  aliases, and between two apps the first in the catalogue's order keeps a name.
- **`X-Yantrik-Adapter`** — a command that provides the surface for an app that cannot host one
  (LibreOffice through UNO, GIMP through its Python). When the shell launches the app it starts
  the adapter beside it with `YANTRIK_SURFACE=<id>` and `YANTRIK_APP_PID=<pid>` in its environment,
  and sends it SIGTERM when that process exits. It is not started while anything already answers
  on `app-<id>.sock`. An adapter should also exit on its own when the app it drives goes away.

With those keys the app is listed in `describe shell` → `apps` whether or not it is running (name,
`describe_as`, `for`, `aliases`, `title`, `running`), `open_app` opens it by its id, any alias or its
`Name`, a button on one of its notifications reaches it (opening it first if it is closed), an
approval can be asked for its actions, and `yos ls` / `os_apps` show it as `(closed)` with its
purpose. The shell notices a new or changed entry within a few seconds; `act shell refresh_apps`
rescans at once. This OS's own apps declare themselves with exactly these keys
(`apps/desktop-files/`), and get nothing more — the shell's own table is left with only what no
`.desktop` file can say: its screens and sections, the launcher, "whichever browser is installed",
and Blender's addon-and-display check.

## Checking one

```
$ yos check notes
yos check notes  (/run/user/1000/yantrik/app-notes.sock)
  pass  ping        rpc.ping answered "pong" with the id it was sent (… ms)
  pass  describe    answered in … ms, the median of 3 reads (…)
  pass  protocol    protocol 1
  pass  schema      describe matches docs/schema/describe.schema.json
  …
  pass  stale       <action> acting on <stale> → -32602 STALE: this app is at revision …
  0 failed, 0 warned, 15 passed, 0 skipped
```

(The shape of it; `deploy/yantrik-os/yos-selftest.py` runs it against a surface that keeps the
protocol and one that keeps nothing, and the runtime's own tests run it against the real dispatch.)

`yos check <name>` — or a socket path while you are writing one, or `--all` for every surface on the
machine, with `--json` for CI — holds a surface to the spec: the describe against the schema, every
grade on the ladder, every parameter typed, no parameter named like a secret, `protocol` present, the
revision computed as the spec says and steady while the view is, describe under 500 ms, and an
unknown method, an empty action, an unknown action, a missing argument, an undeclared one, one of
the wrong type and a stale `expect_revision` each refused with the right code and the spec's words. It prints what it
saw and exits non-zero on a failure. It never runs an action: every act it sends is one the
protocol's dispatch refuses before a handler (the spec, §10, says how it makes sure).

`scripts/app-control-probe.sh` is now a thin wrapper that runs `yos check` over the surfaces it is
given (or every one that answers). What the old script also did — drive each app through a real
behaviour and look at the result — is what `tests/conformance` does per app, on a live machine.

`scripts/companion-tool-probe.sh` drives the whole chain from outside the shell: socket → companion
RPC → worker → tool registry gate → app socket → the window's own callbacks.

## Surfaces today

Every surface this desktop ships, the other names it answers to and what it is for — generated
from the `X-Yantrik-*` keys in `apps/desktop-files`, so it is the list the shell reads. What each one
publishes right now is `yos describe <name>`; whether it keeps the protocol, `yos check <name>`.

<!-- surfaces: generated from the X-Yantrik-* keys in apps/desktop-files by `YANTRIK_WRITE_DOCS=1 cargo test -p yantrik-ui --bin yantrik-ui the_guide_lists_every_surface` -->
| Surface | Socket | Also answers to | For |
| --- | --- | --- | --- |
| `arcade` | `app-arcade.sock` | — | game making: small JSON specs in, one playable HTML game out |
| `blender` | `app-blender.sock` | — | 3D scenes: model them, light them, render them |
| `calendar` | `app-calendar.sock` | — | events and appointments |
| `containers` | `app-containers.sock` | `container-manager` | Docker or Podman containers |
| `documents` | `app-documents.sock` | `document-editor` | written documents — reports, letters, plans — saved as files in ~/Documents |
| `download-manager` | `app-download-manager.sock` | `downloads` | fetch a URL to a file, with progress |
| `editor` | `app-editor.sock` | `text-editor` | plain-text and code files, opened and saved by path |
| `email` | `app-email.sock` | — | read and send mail |
| `image-viewer` | `app-image-viewer.sock` | `images`, `image` | view pictures |
| `network` | `app-network.sock` | `network-manager` | this machine's connections, Wi-Fi and firewall state |
| `notes` | `app-notes.sock` | — | quick markdown notes kept in the notes library, not files you name |
| `presentation` | `app-presentation.sock` | `slides` | slide decks |
| `shell` | `app-shell.sock` | — | the desktop itself: screens, windows, files, opening apps, approvals |
| `snippets` | `app-snippets.sock` | `snippet-manager` | reusable pieces of code and text |
| `studio` | `app-studio.sock` | — | make pictures from a sentence, on your own GPU or a hosted service; they land as files |
| `system-monitor` | `app-system-monitor.sock` | `sysmonitor` | CPU, memory, disk and processes |
| `terminal` | `app-terminal.sock` | — | a shell: run commands |
| `weather` | `app-weather.sock` | — | current conditions and forecast |
<!-- /surfaces -->

Beside them, five services answer `app.describe` on their own sockets — `weather`,
`system-monitor`, `notifications`, `network`, `calendar` — and the first three take `app.act` too.
Blender's surface is served by a Python addon inside Blender (`apps/blender/addon`), a port of the
runtime's dispatch.

Windows from other applications are not in this list unless their `.desktop` file declares a
surface (above) — an adapter is how one gets there. The rest publish through AT-SPI, and
`a11y-service` reads whatever the toolkit chose to expose.
