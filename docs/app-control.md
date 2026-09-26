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
spent and before the handler runs: a window's dispatch here, a service through
`yantrik_service_sdk::reach::permits` before `gate::permit`. The shell publishes each such agent's
reach in `~/.config/yantrik/agent-reach.json`, keyed by the SHA-256 of its token (never the token),
and an act outside it is refused with `REACH:` and a sentence naming the role
(`yantrik_ipc_transport::reach`). One act is decided by its arguments: `shell.open_app` is within
a reach that names the app it opens, whatever the ceiling — a closed app cannot be read, so a
reach that named it and could not open it named an app the role could never use, and opening a
window is not an act on its data. Every act on the data still meets the surfaces and the ceiling
once the app answers. A token with no reach is not held; a call with no token is the person's.

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

### Every action, and the grade it holds

#48 asked the question of two actions in Weather: `add_location` and `set_units` wrote choices
into `~/.config/yantrik/weather.json` that were still standing after a restart, and both were
graded the same as opening a window. The fix is a rule, so the rule was walked across every
published action on every app and service surface: **an action whose effect outlives the turn
that made it — a persistent setting, stored configuration, anything a restart reads back — is at
least `sensitive`; showing, reading and opening stay `standard` or `safe`.** Five actions moved:
Weather's two, and the shell's `pin_app`, `use_harness` and `set_do_not_disturb`, each of which
writes a setting into the shell's own settings file. This table is the record of the walk.

One distinction the table leans on, because it is what kept the walk from regrading every save:
**configuration against content**. A choice about how the app or the machine behaves from now on
— which mind answers, which units readings arrive in, what sits on START — is configuration, and
configuration asks first. A thing the person owns — a note, an event, a document, a scene — is
content, and a content action a paired action takes back stays `standard` (`add_event` beside
`delete_own_event`; [the grade guide](sdk/grades.md) argues the line).

| Surface | Action | Grade | Why |
| --- | --- | --- | --- |
| shell | `read_message`, `open_lens`, `show_desktop` | safe | reads and showings; nothing written |
| shell | `open_app`, `start_service`, `refresh_apps`, `send_message` | standard | launch, start, rescan, say a line to the desktop — undone by closing or stopping, nothing stored |
| shell | `show_screen`, `show_app`, `focus_window`, `close_window`, `minimise_window`, `maximise_window` | standard | moving and placing windows; the state they change dies with the session |
| shell | `pin_app` | sensitive | **regraded (#48)**: writes the START pin list into the shell's settings, and it is still pinned after a restart |
| shell | `use_harness` | sensitive | **regraded (#48)**: writes the preferred mind into the shell's settings — a choice about who answers from now on, across restarts |
| shell | `set_do_not_disturb` | sensitive | **regraded (#48)**: writes `dnd_mode` into the shell's settings; left on, it swallows every notification that follows, quietly, until somebody notices |
| shell | `report_problem` | sensitive | sends what it carries out of the machine |
| shell | `install_harness`, `start_harness` | sensitive | fetches software onto the machine; decides what it runs on every login |
| shell | `set_mind_panel` | safe | showing: how much of one panel is drawn, remembered in the panel's own file; nothing sent, run or granted |
| shell | `lock` | safe | only takes access away — Super+L must lock, not ask about locking (#215) |
| shell | `files_go`, `files_enter`, `files_open`, `files_up`, `files_new_folder`, `files_new_file`, `files_select`, `files_view`, `files_rename`, `files_copy`, `files_cut`, `files_paste`, `files_trash_selected`, `files_undo_trash`, `files_toggle_trash`, `files_refresh`, `files_cancel`, `files_terminal` | standard | driving the Files screen; what they change is content, taken back by the paired verb or recovered from Trash |
| shell | `files_delete` | dangerous | destroys without a trash |
| shell | `check_update` | safe | reads |
| shell | `set_update_channel` | sensitive | stored configuration: decides where every later update comes from |
| shell | `apply_update` | dangerous | replaces the system |
| shell | `installer_set`, `installer_go_to` | standard | driving the installer's own screens |
| shell | `installer_install`, `installer_reboot` | dangerous | writes the disk; ends the session |
| shell | `run_recipe` | sensitive | starts agents |
| shell | `answer_recipe`, `pause_recipe`, `resume_recipe`, `cancel_recipe` | standard | steering a run already started |
| shell | `new_agent`, `hand_off` | sensitive | starts an agent, with what that costs and whatever reach the role carries |
| shell | `send_to_agent`, `stop_agent` | standard | talking to, or stopping, an agent the person started |
| shell | `read_agent`, `show_agent` | safe | reads and showings |
| shell | `agent_run`, `agent_input` | sensitive | arbitrary commands; typing into a live shell |
| shell | `agent_job`, `agent_kill` | standard | reading a job's state; ending a job the caller's token owns |
| shell | `request_approval`, `approval_status`, `consume_approval`, `set_mind_mode`, `record_unasked_action`, `show_mind_audit`, `close_mind_menu` | safe | the approval machinery itself, which must never act; `set_mind_mode` refuses every loosening, so it can only tighten |
| arcade | `new_character`, `new_game`, `update_game`, `update_character`, `build`, `play`, `verify`, `screenshot` | standard | editing and building library content, editable again |
| arcade | `delete` | sensitive | destroys the one named game |
| calendar | `select_day`, `show_month`, `go_to_today`, `set_view` | standard | moving around the calendar |
| calendar | `add_event`, `update_own_event`, `delete_own_event` | standard | content, paired: what `add_event` writes, `update_own_event` moves and `delete_own_event` takes back — the caller's own events, by the #201 record |
| calendar | `update_event`, `delete_event` | sensitive | reach any stored event — the person's own, a Google-synced one, another caller's — so the person sees a card (#332); a delete has no trash, and its description says the event is not recoverable |
| containers | `refresh`, `start`, `show_logs` | standard | reads, and starting what is stopped |
| containers | `stop`, `restart` | sensitive | interrupts what the container was serving |
| containers | `remove` | dangerous | the writable layer goes with it |
| documents | `find`, `show` | safe | reads |
| documents | `open`, `new`, `save`, `save_as`, `set_content`, `append`, `replace_all`, `export_markdown` | standard | content in the app's own library in ~/Documents, every step editable again |
| download-manager | `add`, `pause`, `resume`, `retry`, `verify`, `pause_all`, `resume_all`, `clear_completed`, `open_folder` | standard | steering the queue |
| download-manager | `cancel` | sensitive | ends a transfer in flight |
| email | `open_message`, `select_folder`, `search`, `mark_read`, `flag`, `compose` | standard | reads and a draft; `send` is deliberately not published — mail that has gone cannot be taken back |
| email | `begin_google_sign_in` | sensitive | puts a full-access consent screen in front of the person, unasked |
| image-viewer | `open`, `show`, `next`, `previous`, `rotate`, `fit`, `toggle_info` | standard | showing pictures; `rotate` turns what is on screen, the file is unchanged |
| network | `refresh`, `wifi_scan` | standard | reads that trigger a radio scan |
| network | `wifi_connect`, `wifi_forget` | sensitive | changes what the machine joins; forgetting deletes a stored credential |
| network | `wifi_disconnect`, `wifi_radio` | dangerous | on a machine reached over Wi-Fi, takes away the channel the undo would travel on |
| notes | `new_note`, `open_note`, `set_title`, `append`, `search`, `set_folder`, `notebook`, `tags`, `save`, `restore`, `copy`, `reload`, `preview`, `focus`, `undo`, `redo` | standard | library content, with `undo` beside it |
| notes | `set_content`, `trash`, `import`, `export` | sensitive | replaces a note's whole body; moves things in and out of the library and the filesystem |
| presentation | `open`, `show`, `save`, `save_as`, `new_deck`, `add_slide`, `set_slide`, `move_slide`, `go_to`, `next`, `previous`, `present`, `export_markdown` | standard | deck content, editable again |
| presentation | `delete_slide` | sensitive | takes the slide and its contents off the deck |
| snippets | `open`, `search`, `show` | safe | reads |
| snippets | `new`, `save`, `copy`, `toggle_favorite`, `import` | standard | library content, paired verbs |
| snippets | `delete` | sensitive | no trash |
| studio | `refresh` | safe | reads |
| studio | `generate`, `variations` | standard ⇄ sensitive | regraded at runtime: `standard` to a model on the machine, `sensitive` when the backend sends the prompt to a hosted service |
| studio | `set_backend` | sensitive | stored configuration: written into the settings file, it decides where every later prompt goes |
| studio | `upscale`, `open`, `delete`, `cancel`, `cancel_all` | standard | library content and job steering |
| system-monitor | `sort_processes`, `filter_processes` | standard | view state |
| system-monitor | `kill_process` | dangerous | ends what somebody else is running |
| terminal | `run`, `send_input`, `new_tab`, `open_directory` | sensitive | arbitrary commands in the window the person is looking at |
| editor | `new`, `open`, `save`, `save_as`, `show`, `close`, `cancel`, `find`, `find-next`, `find-prev`, `replace_text`, `replace`, `replace-all`, `append`, `undo`, `redo`, `select_tab` | standard | buffer and file content, `undo` beside it; `close` refuses unsaved changes rather than deciding about them |
| editor | `discard`, `set_content` | sensitive | throws work away; replaces the buffer wholesale |
| weather | `refresh` | standard | fetches again, stores nothing |
| weather | `show_location` | standard | a showing: moves an already-saved place onto the screen — the prefs line it touches is which place was being shown, not what is saved |
| weather | `add_location` | sensitive | **regraded (#48)**: saves a place into `~/.config/yantrik/weather.json`, still saved after a restart |
| weather | `set_units` | sensitive | **regraded (#48)**: the stored unit choice decides the units every future reading arrives in |
| weather-service | `set_location` | standard | remembered in memory for this run only; the service writes no file — the app's `add_location` is the storing one |
| system-monitor-service | `find_process` | safe | reads |
| system-monitor-service | `kill_process` | dangerous | ends what somebody else is running |
| notifications-service | `notify`, `dismiss`, `dismiss_all`, `mark_read` | standard | a notification changes nothing and reaches nowhere outside this machine |
| blender | `new_scene`, `add_primitive`, `delete_object`, `transform`, `set_material`, `set_camera`, `set_light`, `import_model`, `set_render`, `screenshot` | standard | scene content, editable again |
| blender | `render`, `save`, `open` | sensitive | writes files on the machine; `open` replaces the scene in front of the person |
| blender | `run_python` | dangerous | arbitrary code can do anything the person can |
| libreoffice | `read_text`, `read_cells` | safe | reads |
| libreoffice | `open`, `write_text`, `write_cells`, `save_as`, `export_pdf`, `close` | standard | nothing reaches the disk until `save`; `save_as` and `export_pdf` refuse a path where a file already is, and `close` refuses unsaved changes |
| libreoffice | `save` | sensitive | replaces the file the document came from |

The walk also looked at, and left: `weather.show_location` (a showing — regrading it would make
"show me London" a card, and the issue named only the two that store); `shell.set_mind_panel` and
`shell.lock` (showing, and #215); the approvals surface (its seven `safe`s are the tested
security property that the surface which asks can never act); and every first-party `save`
(content in the app's own library, against LibreOffice's `save`, which replaces an outside file
the document came from). Two places grade the same verb differently, and the walk left both:
studio's `delete` stays `standard` while arcade's is `sensitive`, though both move to the Trash —
arcade's author graded a built game as work somebody asked for, the way Notes grades its own
`trash` — and `files_delete` (`dangerous`, no trash) sits beside `files_trash_selected`
(`standard`, recoverable). None of the four writes configuration, so none is #48's rule; whether
every trash-move of finished work deserves a card is a judgement for its own issue.

### The methods a service answers beside the gate

Every grade above is enforced in the `app.act` dispatch. A service also answers **its own**
JSON-RPC methods on the same socket — `sysmon.kill_process`, `network.wifi_connect` — and those
meet no gate at all: a caller that speaks JSON-RPC directly can use the method instead of the
action, and the ceiling, the mind's mode and the grant are no part of it (#161). Beside the
`dangerous` `kill_process` action, which nothing runs without a grant, the raw method signals any
pid it is handed; beside the `sensitive` `wifi_connect` action, the raw method joins any network.

Leaving them open is deliberate, and it ends at #43. These methods are not back doors somebody
forgot; they are the doors the desktop itself walks in through. System Monitor's End button
delivers its SIGTERM by calling `sysmon.kill_process`; every app that raises a notification goes
through an app-runtime helper calling `notifications.add`; the shell's own notification wire
polls `notifications.since`. Until a service can tell the person's own window from any other peer
on the socket — which is what #43 gives it — gating the method would deny the person's own
buttons. So each one is listed here instead: every method every service answers, whether it
changes anything, and the graded `app.act` action that does the same thing where one exists.
`tests/service-methods` walks each service's dispatch and fails when a method has no row here, or
a row here has no method behind it, so a new method — mutating or not — cannot join a service
until somebody has written down what it is beside the gate.

Two services took an interim step in the meantime (#332): the calendar and network sockets answer
their raw methods only to a process the kernel's peer credentials identify as one of the desktop's
own binaries — `/proc/<pid>/exe` pointing at a `yantrik-*` program — and refuse anything else with
a sentence pointing at `app.act`, which still answers any caller under the ceiling, the mode and
the grant. That is a check of the executable, not of the person: code running as the same user can
be the shell's own child and wear its name, so the #154 limits stand, and what each method is
worth stays with #43. The table below lists every method all the same — a peer check is not a
grade, and it records what stands beside both until the methods themselves can be gated.

<!-- service-methods: kept honest by `python3 -m unittest discover -s tests/service-methods`, which reads every service's dispatch and holds the two lists together. `read` changes nothing that outlives the call; `change` does. -->
| Service | Method | | Gated `app.act` beside it | What it is, and who calls it |
| --- | --- | --- | --- | --- |
| system-monitor | `sysmon.snapshot` | read | — | the machine's numbers; the window polls, and the surface's `describe` reads the same |
| system-monitor | `sysmon.processes` | read | — | the process list, sorted and capped; the window polls |
| system-monitor | `sysmon.kill_process` | change | `kill_process` (service surface) — dangerous | SIGTERM to one pid. The window's End button calls the method; Force Kill is SIGKILL and goes local, because the method takes no signal |
| network | `network.interfaces` | read | — | what the Network Manager window draws; it polls these six |
| network | `network.status` | read | — | |
| network | `network.dns` | read | — | |
| network | `network.wifi_state` | read | — | |
| network | `network.wifi_known` | read | — | |
| network | `network.firewall` | read | — | |
| network | `network.wifi_scan` | change | `wifi_scan` (app) — standard | asks the radio to rescan; both doors share one gate of at most one rescan per ten seconds (#332) |
| network | `network.wifi_radio` | change | `wifi_radio` (app) — dangerous | turns Wi-Fi off; on a machine reached over Wi-Fi, takes away the channel the undo would travel on |
| network | `network.wifi_connect` | change | `wifi_connect` (app) — sensitive | joins a network, storing the credential |
| network | `network.dns_set` | change | none | writes the machine's resolvers. The gated door is the companion's `network_dns_set` tool (sensitive), not an action |
| network | `network.wifi_disconnect` | change | `wifi_disconnect` (app) — dangerous | leaves the joined network |
| network | `network.wifi_forget` | change | `wifi_forget` (app) — sensitive | deletes a stored credential |
| calendar | `calendar.events` | read | — | the Calendar app's model; it reads these three |
| calendar | `calendar.get_event` | read | — | |
| calendar | `calendar.revision` | read | — | |
| calendar | `calendar.create_event` | change | `add_event` (app) — standard | writes an event file |
| calendar | `calendar.update_event` | change | `update_event` (app) — sensitive, `update_own_event` (app) — standard | rewrites an event file; the app's split of #332/#201 decides which of the two a caller gets |
| calendar | `calendar.delete_event` | change | `delete_event` (app) — sensitive | removes the file the event lives in; no trash |
| calendar | `calendar.upsert_remote` | change | none | stores what a CalDAV sync fetched; the companion's sync is the graded door in front of it |
| notes | `notes.list` | read | — | the library, enumerated |
| notes | `notes.get` | read | — | one note |
| notes | `notes.search` | read | — | full-text over the library |
| notes | `notes.create` | change | `new_note` (app) — standard | writes a note file. **No caller on the desktop**: the Notes app keeps its own library folder |
| notes | `notes.update` | change | `set_content` (app) — sensitive | rewrites a note file. No caller on the desktop |
| notes | `notes.delete` | change | none | removes the note file outright — no trash; the app's `trash` (sensitive) is the recoverable one. No caller on the desktop |
| notes | `notes.set_pinned` | change | none | rewrites the note's pinned flag. No caller on the desktop |
| notes | `notes.set_tags` | change | `tags` (app) — standard | rewrites the note's tags. No caller on the desktop |
| notifications | `notifications.list` | read | — | the centre's contents |
| notifications | `notifications.since` | read | — | what changed since a revision; the shell's notification wire polls it |
| notifications | `notifications.add` | change | `notify` (service surface) — standard | stores and shows one notification. Every app's notify goes through an app-runtime helper that calls this method |
| notifications | `notifications.dismiss` | change | `dismiss` (service surface) — standard | takes one off the screen |
| notifications | `notifications.dismiss_all` | change | `dismiss_all` (service surface) — standard | takes them all off |
| notifications | `notifications.mark_read` | change | `mark_read` (service surface) — standard | clears the unread badge |
| notifications | `notifications.action` | change | none | invokes a notification's button and tells the sender. The shell calls it when the person clicks; the click is the authority, which is why there is no action |
| email | `email.accounts` | read | — | the configured accounts, secrets stripped |
| email | `email.oauth_status` | read | — | where a sign-in stands |
| email | `email.test_account` | read | — | dials the account's server to check the credentials; changes nothing |
| email | `email.list_folders` | read | — | the mailbox tree; the Email app reads these four |
| email | `email.list_messages` | read | — | |
| email | `email.get_message` | read | — | |
| email | `email.search` | read | — | |
| email | `email.save_account` | change | none | writes the account configuration, password included. The app's settings screen calls it |
| email | `email.oauth_begin` | change | `begin_google_sign_in` (app) — sensitive | starts the OAuth dance: a browser opens on a full-access consent screen |
| email | `email.oauth_cancel` | change | none | abandons a sign-in in flight |
| email | `email.send_message` | change | none | **sends mail.** The app publishes `compose` and deliberately never `send` — mail that has gone cannot be taken back — but the method sends |
| email | `email.mark_read` | change | `mark_read` (app) — standard | sets the read flag on the server |
| email | `email.mark_starred` | change | `flag` (app) — standard | sets the star |
| email | `email.move_message` | change | none | moves a message between folders on the server |
| email | `email.delete_message` | change | none | deletes a message on the server |
| a11y | `a11y.windows` | read | — | the foreign window list |
| a11y | `a11y.describe` | read | — | one window's control tree |
| a11y | `a11y.status` | read | — | whether AT-SPI is answering |
| a11y | `a11y.act` | change | none | clicks and types in somebody else's window. The graded door is the companion's `window_action` tool (standard), not an action |
| weather | `weather.current` | read | — | the forecast; the Weather app fetches these seven. `current`, `hourly` and `daily` remember the asked-for place in memory for this run — the same remembering the surface's `set_location` (standard) does |
| weather | `weather.hourly` | read | — | |
| weather | `weather.daily` | read | — | |
| weather | `weather.alerts` | read | — | |
| weather | `weather.air_quality` | read | — | |
| weather | `weather.suggest` | read | — | |
| weather | `weather.geocode` | read | — | |
| perception | `perception.since` | read | — | the kernel's account of what is happening, long-polled; root-only socket |
| perception | `perception.snapshot` | read | — | counts, sources, uptime |
| perception | `perception.scope` | read | — | what it may see and what the kernel is enforcing |
| perception-journal | `journal.status` | read | — | the stored-observation journal's state |
| perception-journal | `journal.since` | read | — | stored observations since a sequence |
<!-- /service-methods -->

The `none`s are the point of the table, and they are not all the same `none`. `email.send_message`
is a door the app deliberately does not have — `send` is unpublished because mail that has gone
cannot be taken back — yet the method sends, and that is exactly the asymmetry #161 is about.
`notes.delete` removes a file outright beside an app whose own deleting goes to Trash; and all
five notes writers have no desktop caller at all, the Notes app keeping its library folder itself
— they are candidates for deletion, or for a gate, the day #43 lands. Until then they at least
stay inside that folder: a note id is one plain file name there, and an id that is a path
(`../../x`, `/home/…`) is refused, where it used to reach any `.md` file the person owns.
`notifications.action` and
`calendar.upsert_remote` are methods whose only caller is the desktop acting for the person — a
click, a sync — and are listed so that when #43 can tell callers apart, the choice about each one
is already written down. Nothing in this section changes a grade or a gate decision; it records
what stands beside them until the methods themselves can be gated.

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
| `editor` | `app-editor.sock` | `text-editor` | write a new text or code file (new with its text, then save_as a path), or open and edit one by path |
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
