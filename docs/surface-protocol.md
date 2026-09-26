# The surface protocol, version 1

*Normative. The guide, with examples and the reasons behind the rules, is
[app-control.md](app-control.md). JSON Schemas: [schema/describe.schema.json](schema/describe.schema.json),
[schema/act.schema.json](schema/act.schema.json). The policy, generated from the code:
[deploy/yantrik-os/surface-vectors.json](../deploy/yantrik-os/surface-vectors.json). Why it
exists and what comes after it: [design/surface-sdk-2026-09-23.md](../design/surface-sdk-2026-09-23.md).*

A **surface** is a process that lets a mind find it, read what it shows, and act in it under the
person's grades. Every app in this repository publishes one; so can anything else. This document
says what a surface and a client must do to understand each other. It describes what the code
does today — `yantrik-ipc-transport` (the wire, the gate), `yantrik-ipc-contracts::control_surface`
(the envelopes) and `yantrik-app-runtime::control` (the dispatch) — and where a port or an older
surface differs, it says so under [Known deviations](#known-deviations) rather than pretending
otherwise.

"MUST", "SHOULD" and "MAY" mean what RFC 2119 says they mean.

## 1. Framing

- A surface listens on a **unix stream socket** (§2). A client connects, writes requests, and reads
  replies on the same connection.
- Every message is **one JSON object on one line**, UTF-8, ended by `\n`. A request is answered by
  exactly one reply line. A connection MAY carry many requests; they are answered one at a time, in
  the order they were sent.
- A request is JSON-RPC 2.0:

  ```json
  {"jsonrpc":"2.0","id":1,"method":"app.describe","params":{}}
  ```

  `jsonrpc`, `method` and `id` MUST be present (`id` is a number or a string; the transport does
  not check `jsonrpc`'s value, and a client MUST send `"2.0"`). `params` MAY be absent, which is
  `null`. Batches (a JSON array) and notifications (no `id`) are not served: they are answered
  as a parse error.
- A reply carries the request's `id`, and either `result` or `error`:

  ```json
  {"jsonrpc":"2.0","result":"pong","id":1}
  {"jsonrpc":"2.0","error":{"code":-32602,"message":"`open_note` needs argument `title`"},"id":2}
  ```

  A line that does not parse as a request is answered with `-32700`, `"Parse error: …"` and
  `"id": null`.
- The transport answers two methods itself, before any surface code runs:
  - `rpc.ping` → `"pong"`.
  - `rpc.service_id` → the socket's service id (`"app-notes"`, `"weather"`).

## 2. Where surfaces are: the socket directory chain

There is one chain, the transport's (`yantrik_ipc_transport::server::socket_dir`):

1. `$XDG_RUNTIME_DIR/yantrik`, when `XDG_RUNTIME_DIR` is set and not empty;
2. `/run/yantrik`;
3. `/tmp/yantrik-<uid>`.

A **server** binds in the first directory of the chain that exists or can be created and that is,
or can be made, mode `0700`. The socket file itself is made `0600` after binding.

A **client** MUST search all three, in this order, and MUST NOT create any of them. A client started
without `XDG_RUNTIME_DIR` (sudo, cron, a bare ssh command) cannot know the session's value; it MAY
look in `/run/user/<uid>/yantrik` in the first step's place, which is where logind puts it. A server
MUST NOT: a server that binds somewhere the chain does not name is a server clients cannot find.

A socket file outlives a crashed process. A client that finds a file nobody is listening on
(`ECONNREFUSED`) MUST treat it as "not running" and go on to its next candidate.

## 3. Names

- An **app's** surface is `app-<id>.sock`; a **service's** is `<id>.sock`. They are different
  things and do not share a socket: `notes.sock` stores notes, `app-notes.sock` is the window a
  person is looking at.
- `<id>` is the id the surface publishes as `app` in `describe` — lowercase words joined by `-`
  (`download-manager`, `image-viewer`).
- **Aliases.** A surface known by other names links each of them at its socket:
  `app-<alias>.sock` is a symlink to `app-<id>.sock`, with a relative target. It is one surface,
  not a second listener, so every name lands in the same process and reads the same revision. A
  client listing surfaces MUST NOT list a symlink as a surface of its own.
- **Resolving a name.** A client folds what it was given — trim, lowercase, `_` and space become
  `-` — and tries `app-<name>` before `<name>`, so a name reaches the window when it is open and
  the service behind it when it is not. `shell` is `app-shell`.
- **Declaring a surface.** A surface that is to be found while it is not running — listed in
  `describe shell` → `apps`, opened by `open_app`, reached by a notification button, approved while
  its window is shut — SHOULD declare itself in the `[Desktop Entry]` group of its application's
  `.desktop` file: `X-Yantrik-Surface=<id>` (required for the rest to count; a value that is not a
  name as above declares nothing), `X-Yantrik-Purpose=<one line>`, `X-Yantrik-Aliases=<a>;<b>`
  (each linked at the socket by the shell, as above, unless the desktop or another surface already
  holds the name) and `X-Yantrik-Adapter=<command>` (a separate process the shell starts beside the
  app with `YANTRIK_SURFACE` and `YANTRIK_APP_PID` set, and stops with SIGTERM when the app exits).
  [app-control.md](app-control.md#findable-while-closed-the-desktop-keys) has the detail. A surface
  that declares nothing is still a surface while it answers; it is only invisible while it does
  not.
- **Owned names.** A name belongs to the process answering on it:
  - A server MUST NOT bind over a live socket. Before binding it connects to whatever is at its
    path and sends `rpc.ping`. If anything answers — or accepts the connection and stays silent
    for a second — the bind is refused and the server does not start:

    ```
    another instance owns /run/user/1000/yantrik/app-notes.sock: it answered rpc.ping as
    `app-notes`. Refusing to start rather than take the name from a running process — stop that
    one first, or talk to it.
    ```

    Only a socket nobody is listening on — a crashed process's — is removed. A symlink or a
    regular file at the path is not a listener and is removed as before. A server that was refused
    MUST NOT remove the file on its way out.
  - `app-shell` is where a person's Allow is asked for and spent. A client that spends a grant, or
    trusts what the shell says, MUST check who is listening before it writes anything: the kernel's
    account of the peer (`SO_PEERCRED`, taken at `listen` time) gives its pid, `/proc/<pid>/exe`
    its program, and that program MUST be a `yantrik-ui` binary (the file name is `yantrik-ui`;
    Linux's ` (deleted)` suffix, left by an update that replaced the binary under a running shell,
    is allowed). The app runtime's grant spending (`gate::spend_grant`), the Python surface SDK's
    (`yantrik_surface.gate.spend_through_shell`), `yos` and, through `yos`, `yos-mcp` do this. The
    rule is `yantrik_ipc_transport::owner::must_be_the_shell`.

  These stop accidents and casual impersonation — a second copy, a stray test server, a script
  that picked the wrong name. Anything already running as the person can build a binary called
  `yantrik-ui`; the uid is the boundary, and nothing on the socket moves it.

## 4. `app.describe`

`params` is ignored. The result (schema: [describe.schema.json](schema/describe.schema.json); this
one is `describe_json`'s own output, from the vectors' `envelopes`):

```json
{
  "protocol": 1,
  "app": "weather",
  "summary": "Weather — 21°C in Dallas",
  "state": {"temp": 21},
  "revision": "e09e606e30c937f6",
  "actions": [
    {
      "name": "refresh",
      "description": "Fetch the weather again",
      "permission": "safe",
      "settles": "on return",
      "parameters": {"type": "object", "properties": {}, "required": []}
    },
    {
      "name": "set_location",
      "description": "Show the weather somewhere else",
      "permission": "standard",
      "settles": "later",
      "parameters": {
        "type": "object",
        "properties": {
          "label": {"type": "string", "description": ""},
          "lat": {"type": "number", "description": "Latitude"},
          "lon": {"type": "number", "description": "Longitude"}
        },
        "required": ["lat", "lon"]
      }
    }
  ]
}
```

| key | |
| --- | --- |
| `protocol` | `1`. A describe without it is from before this document; a client SHOULD still read it. A client MUST ignore keys it does not know. |
| `app` | The surface's id (§3). |
| `summary` | One line a person could read. |
| `state` | An object, in the app's own vocabulary. |
| `revision` | §6. |
| `actions` | Every action the surface offers, in the order it declared them. |

An **action**:

| key | |
| --- | --- |
| `name` | What `app.act` calls it. |
| `description` | What it does, in the app's words. The dispatch reads it (§7): wording that says the action cannot be undone changes when a person is asked. |
| `permission` | Its **grade**: `safe` < `standard` < `sensitive` < `dangerous`. An action declared without one is `standard`. |
| `settles` | `"on return"` — the work is done when the act answers — or `"later"` — the act only starts it. |
| `parameters` | A JSON Schema object: `type` `"object"`, `properties` (each `{type, description}` and, when declared, `enum`, `items` and `default`; `description` MAY be empty), `required` (names, in declaration order). |
| `expected_seconds` | Optional integer: how long a call usually takes to answer, when the app knows it is more than a moment (a render, an export). A client SHOULD size its timeout by it; absent, the client keeps its own. |
| `explains` | Optional boolean, present and `true` only when the action can say what ONE call of it does, with that call's own arguments. The sentence itself comes from `app.explain` (below), because it depends on arguments `describe` never sees; absent, the action says nothing per call. |

**Parameter types.** `type` is one of `string`, `number`, `integer`, `boolean`, `array`,
`object`, and the dispatch checks it (§5, step 7): an argument of another type is refused before
the handler runs — unless it converts to the declared type without loss (below). What each
accepts as it is:

| declared | published | accepts |
| --- | --- | --- |
| `string` | `{"type": "string"}` | a JSON string — not a number, even one that spells an id |
| `number` | `{"type": "number"}` | any JSON number |
| `integer` | `{"type": "integer"}` | a JSON number written without a fraction or exponent: `3`, not `3.0` or `3e0` (stricter than JSON Schema, on purpose: a handler reading `3.0` as an integer finds nothing) |
| `boolean` | `{"type": "boolean"}` | `true` or `false` |
| enum | `{"type": "string", "enum": ["low", "normal"]}` | one of the listed strings, exactly |
| `array` | `{"type": "array", "items": {"type": "string"}}` | a JSON array whose every item is of the `items` type |
| `object` | `{"type": "object"}` | a JSON object |

`null` for an optional argument is the same as leaving it out; for a required one it is a value of
the wrong type. A parameter MAY carry `default`: what the handler is given when the caller leaves
the argument out (or sends `null`), and a parameter with a default is not required. A client SHOULD
send each value as its declared type (`yos act` reads `key=value` by the declared type, so `id=67`
is the string `"67"` for a `string` parameter and the number `67` for an `integer` one).

**Conversion without loss.** A handler always receives the type it declared; a caller is met
halfway. A value that is not of the declared type but converts to it without losing anything is
converted — after every check and after any grant is spent (§5, step 12), so the call as sent is
what is checked and what a grant is bound to — and anything else is refused:

| declared | arrives as | the handler reads |
| --- | --- | --- |
| `string`, and an enum | an integer: `67`, `-3` | its decimal digits: `"67"`; an enum's list is then checked on those |
| `integer` | a string that is exactly an integer: `"12"`, `"-4"` | the integer |
| `number` | a string that is exactly an integer or a decimal: `"12"`, `"1.5"` | the number |
| `boolean` | `"true"` or `"false"` | `true` or `false` |

"Exactly" is a grammar, not a best effort: `-?(0|[1-9][0-9]*)` in ASCII digits, and for a number
optionally `.` and one digit or more after it; an integer string outside what a JSON integer holds
(`i64` below zero, `u64` from zero) is not one. So `"1.5"` for an integer, `"12abc"`, `" 12"`,
`"12 "`, `"+12"`, `"012"`, `"1e3"`, `".5"`, `"1."`, `"1_000"` and other scripts' digits are
refused, as is a number with a fraction for text (its text is not one thing: `1.5`, `1.50`,
`1.5e0`), `"True"` and `1` for a boolean, a value in another case for an enum (`Low` is not `low`),
and anything for an array or an object, or inside one. A declared `default` is the author's and is
held to the exact type. `deploy/yantrik-os/dispatch-vectors.json` (`coerce`) is every row of this,
generated from the Rust dispatch and replayed by the Python SDK.

Type checking and conversion are additions to version 1 (see Changes): a client that predates
them already handles the refusal, which is `-32602` like every other in §5.

`describe` is never gated: reading a surface needs no grade, no mode and no grant.

### `app.explain` (optional)

The sentence about ONE call (#137): what this action does with THESE arguments, in the app's own
words, for an approval card to draw under the argument box — after the published purpose, the same
for every call of the action, and the arguments themselves.

`params`:

| key | |
| --- | --- |
| `action` | Required, non-empty: an action's `name`. |
| `args` | The arguments of the call being explained, shaped as `app.act` takes them. Absent means none. |

The result is `{app, action, explanation}`. `explanation` is one bounded sentence, or empty when
the app has nothing to say about these particular arguments — an honest absence, not an error.

Reading, not acting: no grade, no mode, no grant, no reach — it changes nothing and spends
nothing, so like `describe` it is never gated. An action that declared no explainer is refused
with `-32602` and a sentence saying so, which is an answer, and what tells the asker to draw no
line; a surface that has not implemented the method at all answers `-32601` like any method it
does not serve. A client MUST treat any error, and any slowness, as "nothing to show", and MUST
NOT make the thing it was about to draw — the card — depend on this answer: a missing line is
never a missing card. The `explains` flag on `describe` exists so a client need not ask a surface
that cannot answer.

## 5. `app.act`

`params` (schema: [act.schema.json](schema/act.schema.json), `$defs/params`):

| key | |
| --- | --- |
| `action` | Required, non-empty: an action's `name`. |
| `args` | An object of arguments by name. Absent means none. |
| `expect_revision` | Optional string: the `revision` the caller decided on. |
| `grant` | Optional string: a grant (§8). An empty or non-string value is no grant. |
| `agent_token` | Optional string: which of the person's agents the call is for. It travels **beside** `args`, never inside them — `args` is what an approval card shows and the audit log keeps. A copy found inside `args` is removed and not used. The dispatch hands it to the handler; what it is worth is the handler's business. |

The dispatch takes these steps, **in this order**, and the first that refuses ends the call. Every
refusal of the act itself is error `-32602`, whose `message` is the sentence given here, verbatim:

| # | step | refusal (`-32602` unless shown) |
| --- | --- | --- |
| 1 | an `action` | `` act needs a non-empty `action` `` |
| 2 | the action exists | `` unknown action `<name>`; this app offers: <a>, <b>, … `` (every action, in declaration order) |
| 3 | the calling agent's reach, when the call carries an `agent_token` whose role has one (`yantrik_ipc_transport::reach`) | `REACH: …` — the act is outside the role's surfaces, or above its ceiling; a reach file that cannot be read refuses every token-carrying call. One act is read against its arguments: `shell.open_app` is within a reach that names the app in `name`, whatever the ceiling — opening a window is not an act on its data |
| 4 | `args` is an object (absent or `null` is none) | `` `<action>` takes its arguments as an object of named values, and <kind> arrived `` |
| 5 | every required argument is present | `` `<action>` needs argument `<param>` `` (the first missing, in declaration order) |
| 6 | no argument the action does not declare | `` `<action>` has no argument `<key>`; it takes: <p1>, <p2>, … `` — or, for an action with none, `` `<action>` takes no arguments, but `<key>` was given `` (the first undeclared key in sorted order; the list in declaration order) |
| 7 | every argument is of its declared type, or converts to it without loss (§4) | `` `<action>` argument `<param>` must be <wanted>, and <kind> arrived `` — or, for an enum, `` `<action>` argument `<param>` must be one of `<v1>`, `<v2>`, …, and another string arrived `` — or, for an array item, `` `<action>` argument `<param>` must be <wanted>, and `<param>[<i>]` is <kind> `` (the first wrong argument in declaration order) |
| 8 | the ceiling, on the grade | `CEILING: …` (§7) |
| 9 | the grant, if the call carries one — spent only now, past the arguments and the ceiling, against the arguments **as sent** | ``GRANT: `<id>` does not authorise <app>.<action> — <the shell's reason> Nothing was run; a grant covers one action, once, with the arguments the person was shown.`` |
| 10 | the mode, the session rules and the description | `GRANT: …` (§7) |
| 11 | `expect_revision`, when given, is the current revision | `STALE: this app is at revision <current> and you acted on <expected>. It now reports: <summary>. Read it again before deciding.` |
| 12 | the handler, with every argument converted to its declared type (§4) and every declared default filled in | the handler's own sentence, as it returned it |

Steps 4 to 7 come before the ceiling and the grant so that a call its own arguments refuse is
refused for them — and a person's Allow is never used up on a call that was never going to run:
the same grant then runs the call made right. A grant is bound to the arguments as they were sent
and shown on the card, never to the converted form the handler reads. The `order` section of
`deploy/yantrik-os/dispatch-vectors.json` holds this order to the sentence, with the grants spent
after each call, and every implementation replays it.

In step 7, `<wanted>` is `a string`, `a number`, `an integer`, `a boolean`, `an object`, `an array`,
or `an array of <items>s` (`strings`, `numbers`, `integers`, `booleans`, `objects`, `arrays`), and
`<kind>` names what arrived — `null`, `a boolean`, `a number`, `a number with a fraction` (where an
integer was wanted), `a string`, `an array`, `an object` — **never its value**: a number a caller
sends may be a PIN, a year of birth or a dose, and a refusal is shown, logged and handed to a model.
The declaration is what a caller corrects from.

Steps 11 and 12 happen in **one turn** of the surface's own serialization (a window's UI thread):
between the revision check and the handler nothing else can change what the app shows, and the
view in the reply is read in the same turn, after the handler.

An accepted act answers (schema: `act.schema.json`, the root; `act_json`'s own output):

```json
{
  "app": "weather",
  "action_id": "app-weather#1",
  "accepted": true,
  "settled": false,
  "result": {"refreshing": true},
  "revision": "e09e606e30c937f6",
  "summary": "Weather — 21°C in Dallas",
  "state": {"temp": 21}
}
```

| key | |
| --- | --- |
| `accepted` | Always `true`: the guard passed and the handler ran. It never means "done". |
| `settled` | `true` when the action's `settles` is `"on return"`, `false` when `"later"`: the work was started and is not finished. A client MUST NOT report a `settled: false` act as complete. |
| `action_id` | A name for this dispatch, unique within the surface's run: `<service-id>#<n>`, where `<service-id>` is the socket's name — `app-notes#7` for a window, `weather#12` for a service. |
| `result` | The handler's answer, any JSON. |
| `revision`, `summary`, `state` | The view **after** the action. |

A handler that owes its caller a result that takes longer than the surface's turn may finish its
answer off that turn (`yantrik_surface::answer_later`, re-exported as `control::answer_later`): the
reply then waits for the work, `result` is the work's value (or its refusal, as `-32602`), and the
view is read again afterwards. Such an action SHOULD declare `expected_seconds` (§4).

## 6. `revision`

A fingerprint of everything a view reports, so a caller can tell "has what I read changed".

    revision = FNV-1a-64( UTF-8(summary) ‖ 0x00 ‖ UTF-8(render(state)) ), as 16 lowercase hex digits

FNV-1a 64: offset basis `0xcbf29ce484222325`, prime `0x100000001b3`; for each byte, xor then
multiply modulo 2⁶⁴. `render` is compact JSON exactly as `serde_json` writes a `Value`: object keys
sorted by their UTF-8 bytes, no whitespace, non-ASCII characters written as themselves (not
`\u` escapes), `"`, `\` and control characters escaped (`\n`, `\t`, `\b`, `\f`, `\r` short, the
others `\u00xx` lowercase). Numbers are written as `serde_json` writes them — integers as integers,
floats in their shortest round-trip form with a fraction or exponent (`21.5`, `0.0`, `1e+16`,
`1.5e-7`). Actions are not part of it.

Test vectors (`surface-vectors.json`, `revision`, generated from `View::revision`):

| summary | state | revision |
| --- | --- | --- |
| `""` | `{}` | `d884b5186b651423` |
| `keys are sorted, not kept in the order they were added` | `{"Mango":"capitals sort before lowercase","apple":{"b":[3,2,1],"y":null},"zebra":1}` | `f0c34e59c9df9e08` |
| `Notes — “Kernel asks”, 412 words, unsaved` | `{"tags":["ünïcødé","🙂","a/b"],"title":"Kernel asks","unsaved":true,"words":412}` | `ad34b9454af7d883` |
| `numbers` | `{"float":21.5,"int":42,"negative":-7,"past_2_53":9007199254740993,"small":0.001,"zero_float":0.0}` | `410ffd85d36f8e32` |
| `Blender — "monkey.blend", 3 objects, Cycles 1920x1080` | (the file's second vector) | `6d6dd36469ee8664` |

Floats are where ports go wrong: `json.dumps` in Python writes `1e-05` where `serde_json` writes
`0.00001`, and `1.5e-07` for `1.5e-7`. The vectors' `revision_float_edges` pin those renderings.
A port MUST match them; a surface SHOULD keep very small and very large floats out of `state`.

A revision is compared for equality only. `expect_revision` (§5, step 11) is the atomic guard;
comparing revisions in the client and then acting rebuilds the race it closes.

## 7. The decision: grades, ceiling, mode, grant

Every `app.act` meets one rule, whichever door it came through — a mind's tools, `yos act`, a raw
client on the socket. It is `yantrik_ipc_transport::gate::decide`, and it is **generated into
[surface-vectors.json](../deploy/yantrik-os/surface-vectors.json)**: 640 cases (grade ×
ceiling × mode × session rule × grant × whether the description says it cannot be undone), each
with its outcome and the exact sentence. Every implementation replays the file in its tests.

**Inputs.**

- The action's **grade**, as the surface publishes it now (an app MAY re-grade an action while it
  runs; the dispatch reads the published grade at the moment the call arrives).
- The action's **description**. It **cannot be undone** when, lowercased, it contains any of:
  `not recoverable`, `cannot be undone`, `can't be undone`, `irreversible`, `permanently`,
  `permanent`, `no undo` (`gate::UNRECOVERABLE_PHRASES`). This never applies to a `safe` action.
- The machine's **ceiling**: `tool_permission` in `~/.config/yantrik/settings.yaml` (the first
  such line; quotes trimmed; a missing file or a value off the ladder is `sensitive`). Read per
  call.
- The person's **mode**, from `mind-mode.json` beside the settings file, which the shell writes:
  `{"mode": "plan|ask|auto|bypass", "previous": …, "bypass_expires_unix": <n>|null,
  "session_rules": [{"app": …, "action": …}], "shell_pid": <n>, "shell_start_ticks": <n>,
  "boot_id": "<uuid>"}`. A missing or unreadable file, or a mode it does not name, is `ask`. A
  `bypass` whose `bypass_expires_unix` has passed is its `previous` mode (or `ask`). The last
  three fields name the shell that wrote the file and the boot it wrote in — its pid, that
  pid's start time (field 22 of `/proc/<pid>/stat`, ticks since boot), and
  `/proc/sys/kernel/random/boot_id`, which the kernel picks fresh on every boot — and a file
  whose shell cannot be found (pid gone; alive under a different start time: reused; exited
  but unreaped: a zombie, state `Z`/`X`; or recorded under a boot that has ended) is `ask`,
  rules and all, so a shell that died in bypass does not leave it in force (#154, #333). Less
  than the whole identity — any of the three missing — fails closed the same way. A file that
  names no shell at all is read as before. Read per call.
- Whether the call carries a **grant** the shell spent (§8).

**The rule**, in order:

1. A grade off the ladder is refused:
   ``CEILING: <app>.<action> is graded `<grade>`, which is not a level this OS defines (safe < standard < sensitive < dangerous), so it was not run.``
2. A grade above the ceiling is refused — whatever the mode, whatever the grant:
   ``CEILING: <app>.<action> is graded `<grade>`, above this machine's `<ceiling>` ceiling (`tool_permission` in ~/.config/yantrik/settings.yaml), so it was not run. An action at that grade needs a person to authorise it directly — raise the ceiling in Settings if that is the intent.``
3. A spent grant runs it.
4. Each mode runs unasked up to a grade — `plan` `safe`, `ask` `standard`, `auto` `sensitive`,
   `bypass` `dangerous` — but never less than **`standard`** on a socket (the *socket floor*: the
   desktop's own processes call `standard` actions to work, and cannot yet be told from a mind,
   #43). In every mode but `bypass`, an action that **cannot be undone** is asked about whatever
   its grade above `safe`. Anything else runs.
5. A **session rule** for exactly this `app.action` runs it — unless it cannot be undone, and never
   in `plan` mode, which raises no card and so has no standing answers.
6. Otherwise it is refused with `GRANT:`, in one of four sentences:

   - the grade, not plan:
     ``GRANT: <app>.<action> is graded `<grade>` and this machine is in <mode> mode, which runs nothing above `<allowed>` without asking — so it was not run. Ask the shell for approval first (`request_approval` with this app, action and these exact arguments, poll `approval_status`, then send the granted request_id as `grant` on app.act — `yos act` does all of that for you), or have the person at the machine press Allow when the card appears.``
   - cannot be undone, not plan:
     ``GRANT: <app>.<action> is graded `<grade>` and its own description says it cannot be undone, and this machine is in <mode> mode, which asks before anything that cannot be undone — so it was not run. Ask the shell for approval first (…same as above…), or have the person at the machine press Allow when the card appears.``
   - the grade, plan:
     ``GRANT: <app>.<action> is graded `<grade>` and this machine is in plan mode, which raises no card for anything above `standard` — so it was not run. Say what you would do and let the person decide; they switch the mode from the chip in the status bar.``
   - cannot be undone, plan:
     ``GRANT: <app>.<action> is graded `<grade>` and its own description says it cannot be undone, and this machine is in plan mode, which raises no card for that — so it was not run. Say what you would do and let the person decide; they switch the mode from the chip in the status bar.``

   Every `GRANT:` sentence names the grade as `` graded `<grade>` ``, which is where `yos act`
   reads the grade to ask with.

A client branches on the prefix — `CEILING:`, `GRANT:`, `STALE:` — never on the words after it.

### Where the doors differ

A door that raises cards — the shell's `request_approval` (its table is `mind_mode::Modes::decide`)
and the MCP bridge (`yos-mcp`'s `decide`) — answers the same inputs, less the grant, with *run*,
*ask* or *refuse*. The vectors' `door` column says what it must answer: *ask* exactly where the
dispatch refuses with `GRANT:`, *refuse* where the dispatch refuses with `CEILING:` or the machine is
in `plan`, *run* otherwise. Both tables replay that column. They differ from the dispatch in one
place, and each vector where they do carries a `note`: in `plan` mode a `standard` action that can
be undone runs on the socket (the socket floor) and is refused at those doors — the person's own
desktop keeps working in plan, while a mind is told to say what it would do.

## 8. Grants

A grant is a person's Allow for exactly one call. To get one, a client asks the shell (`app-shell`,
all `safe` actions):

1. `request_approval {app, action, grade, args_json, purpose?, requester?}` → `{request_id,
   status: "pending"}` puts a card on the screen (or `{status: "not_needed"}` when the mode runs it
   unasked; or a refusal, for plan mode or above the ceiling);
2. `approval_status {request_id}` → `pending`, `granted`, `denied`, `expired` or `consumed`;
3. on `granted`, `app.act` again with the same `action` and `args`, and `grant: <request_id>`.

The surface's dispatch spends the grant through the shell's `consume_approval {request_id, app,
action, args_json}` — once, only past the ceiling, and only to a `yantrik-ui` process (§3) — and
the shell refuses it unless it is granted, unspent, unexpired and bound to exactly this app,
action and these arguments. A grant that does not hold ends the call (§5, step 9). `yos act`, the
MCP bridge and the companion's `app_action` do these three steps on a caller's behalf.

## 9. Error codes

| code | meaning | when |
| --- | --- | --- |
| `-32700` | not a request | the line is not JSON, not an object, or lacks `jsonrpc`, `method` or `id`. `"Parse error: …"`, `id` null. |
| `-32601` | unknown method | `` unknown method `<m>`; this app serves app.describe, app.act `` |
| `-32602` | the act was refused | every refusal in §4's `app.explain`, §5 and §7, and a handler's own. |
| `-32000` | the surface did not answer | `app did not answer within 3s` (a window's turn did not come in time); `this app published no control surface`; `app is not accepting requests: …`. |

A client SHOULD treat `-32602` as an answer (the app is healthy and said no) and `-32000` as the app
being unavailable.

## 10. What `yos check` holds a surface to

`yos check <surface>` (or a socket path, or `--all`; `--json` for a machine) reads this document as
a program and exits non-zero when a surface breaks it:

| check | |
| --- | --- |
| `ping` | `rpc.ping` answers `"pong"` with the id it was sent. |
| `describe` | answers; the median of three reads is under 500 ms (a warning above). |
| `protocol` | `protocol` is present and is `1`. |
| `schema` | the describe matches `describe.schema.json`. |
| `grades` | every action is graded on the ladder. |
| `params` | every parameter has a type from §4 (an array's `items` too), and every required one is declared. |
| `secrets` | no parameter is named like a secret — the shell's rule: `passphrase`, `password`, `passwd`, `pin`, `secret`, `credential`, `unlock` (`pinned` excepted). |
| `revision` | the published revision is §6's hash of the summary and state (a warning when a float may render differently). |
| `steady` | an unchanged view keeps its revision across reads. |
| `method`, `empty`, `unknown` | an unknown method, an empty action and an unknown action are refused with the right code and words. |
| `missing`, `undeclared`, `types`, `stale` | a missing argument, an undeclared one, one of the wrong type (one no dispatch converts: `true` for text, a non-numeric string for anything else) and a stale `expect_revision` are refused with the right code and words. `types` is a warning, not a failure, when the revision guard refused the mistyped argument instead: type checking is an addition to version 1. |

It never runs an action. The last four are sent only to a surface that publishes `protocol: 1`
and refused the unknown action exactly as §5 says; they name an action graded `safe` or `standard`
that does not say it cannot be undone, with every other required argument given a value its
declaration accepts (an enum's first value, a declared default), and they all carry a stale
`expect_revision` besides, so a surface that skipped an argument check is still stopped by its
revision guard before a handler runs.

## Known deviations

What the code in this repository does that this document does not, so nobody mistakes it for the
protocol:

- **network-service and calendar-service** answer `app.describe` and no `app.act` at all: an act
  there is `-32601 Unknown method: app.act`, where a surface with no actions owes
  `` unknown action `<name>`; this app offers: `` with an empty list.
- **Services that are not surfaces** answer an unknown method in their own ways — the companion's
  socket with `-32602`, the harness host with `-32000` — where the transport's convention is
  `-32601`. They publish no `describe`, so they are outside this protocol; `yos check --all`
  skips them, and `yos ls` (a mind's `os_apps`) does not list them — only `yos ls --all` names
  them, as the desktop's own.
- **Rust dispatch:** a non-string `expect_revision` is ignored (no guard). A client MUST send a
  string.

## Changes

- **1** (this document): what the surfaces already did, written down, plus `protocol: 1` in
  `describe`; the "cannot be undone" rule and "no session rule in plan" moved into the dispatch so
  every door decides alike; owned names (bind only over a dead socket; the shell's peer checked
  before a grant is spent); surfaces declared in `.desktop` files, so they are found while closed
  (§3).
- **1, extended** (the surface SDK's piece B, `crates/yantrik-surface`): parameters gain `integer`
  enforced as written, `array` with `items`, `object`, `enum` and `default`, and actions gain
  `expected_seconds`; the dispatch refuses a non-object `args` (§5, step 4) and an argument of the wrong
  type (step 7), both `-32602`, which a version-1 client already treats as an answer; the three
  services that answered `app.act` themselves dispatch through the shared crate, so they now keep
  §5 — `-32602` and "this app offers" for an unknown action, per-call `action_id`s, undeclared
  arguments and `expect_revision` refused. The Python SDK (`sdk/python/yantrik_surface`) publishes
  the same shapes and refuses in the same words. Nothing a version-1 client relied on changed
  shape.
- **1, extended: arguments before the grant** (the coordinator's call on #191): the arguments
  (§5, steps 4–7) are checked before the ceiling, the mode and any grant, where they used to come
  after — so a malformed call is refused without using up the person's Allow, and the same grant
  then runs the call made right. A grant is spent against the arguments as sent. The agent's reach
  (#188) is written in as step 3. A caller over the ceiling with a mistake in its arguments now
  hears about the mistake first, and about the ceiling once it is fixed; that is the price of
  never spending a grant on a call that cannot run.
- **1, extended: conversion without loss** (the coordinator's call on #191): an integer for text,
  a string that is exactly a number for a number or an integer, and `"true"`/`"false"` for a
  boolean are converted to the declared type for the handler (§4) instead of refused; everything
  else is refused in step 7's sentence, as before. Handlers keep the strictness; callers — a model
  sending `which: 1` — are not bounced. The Rust dispatch and the Python SDK convert identically,
  held by `deploy/yantrik-os/dispatch-vectors.json`.
- **1, extended: the sentence about one call** (#137): an action MAY declare that it can say what
  ONE call of it does, with that call's own arguments — `"explains": true` on `describe`, and the
  sentence from a second method, `app.explain` (§4), because it depends on arguments `describe`
  never sees. An approval card draws it under the argument box, after the published purpose and
  the arguments. A
  surface that has not implemented the method answers `-32601`, an action that declared no
  explainer refuses with `-32602`, and in every case — error, refusal, slowness — the card is
  exactly what it was: the flag lets a client skip the question, and nothing that was about to be
  drawn depends on the answer. Studio's `set_backend` is the first to use it.
