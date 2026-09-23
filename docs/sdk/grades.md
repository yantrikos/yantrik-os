# Choosing a grade

Every action carries a **grade**, and the grade decides who has to agree before the action runs.
Four levels, in order:

| grade | for | what a person sees |
| --- | --- | --- |
| `safe` | reading; changes nothing | never asked — not even in `plan` mode |
| `standard` | changing the program's own state, in a way that can be taken back | not asked in the default mode |
| `sensitive` | overwriting, sending, spending, anything that leaves the machine, anything the person would want to see first | a card to Allow, in the default mode |
| `dangerous` | destroying work, ending what someone else is running, running arbitrary code | refused outright under the ceiling this OS ships with; with the ceiling raised, a card in every mode short of `bypass` |

An action declared without a grade is `standard`. The grade is published in `describe`, so a mind
can see it before it calls, and the dispatch reads it again when the call arrives.

## How this OS's own apps chose

Grades are judgement, so here are the judgements, with the reasons their authors wrote beside them.

**`safe`: it reads.** Searching a document, finding a process, reading a spreadsheet's cells.

<!-- from: services/system-monitor-service/src/main.rs -->
```rust
        Action::new("find_process", "Find running processes by name, for a pid")
            .risk("safe")
```

**`standard`: it changes something, and something else takes it back.** A calendar's `add_event`
is undone by `delete_event`; opening a file is undone by closing it; writing text into a
LibreOffice document is undone by its Undo, and nothing reaches the disk until `save`. Most actions
are `standard`, which is why it is the default.

**`sensitive`: the person should see it first.** Deleting one named thing with no trash — the
calendar's author explains why that is `sensitive` and not `dangerous`:

<!-- from: apps/calendar/src/main.rs -->
```rust
            // Graded `sensitive`, and the reason is that there is no trash. A delete here removes
            // the file the event lives in; nothing on this machine keeps a copy, so an appointment
            // a person or a mind put on the calendar is gone and its time with it. That is a
            // different thing from `add_event`, which is `standard` because it is undone by this
            // action, and from `update_event`, which moves something that still exists. It sits
            // below `dangerous` because it destroys one named thing the caller asked for by name,
            // not a range and not a directory — the range delete Google sync used to do, which
            // took every local event in the window with it, would have been the other grade.
            Action::new("delete_event", "Take an event off the calendar. It is not recoverable")
                .risk("sensitive")
```

Stopping a container interrupts whatever it was serving, though it can be started again:

<!-- from: apps/container-manager/src/main.rs -->
```rust
            // Recoverable, but it interrupts whatever the container was serving.
            Action::new("stop", "Stop a running container")
                .arg(Param::text("container"))
                .risk("sensitive"),
```

And a grade can be about what an action does to the *person* rather than to the machine. Email's
sign-in changes nothing by itself, and is still `sensitive`:

<!-- from: apps/email/src/main.rs -->
```rust
            // `sensitive` rather than `standard`, and the grade is about what it does to the
            // person rather than to the machine. By itself it changes nothing — no account is
            // written, no mailbox is touched, and the flow cannot complete without somebody
            // choosing an account and reading a consent screen that says full access to Gmail.
            // But it puts that screen in front of them, unasked, on their own display, and a
            // consent page a person did not go looking for is exactly the shape of the thing
            // they should not be trained to click through. So it is above `standard`, where a
            // `tool_permission: standard` ceiling refuses it, and the default ceiling allows it.
```

**`dangerous`: it destroys, or cannot be walked back.** Ending a process somebody else is running;
deleting a container and its writable layer; leaving the Wi-Fi on a machine reached over Wi-Fi.

<!-- from: services/system-monitor-service/src/main.rs -->
```rust
        Action::new("kill_process", "End a running process by PID")
            .risk("dangerous")
```

<!-- from: apps/network-manager/src/main.rs -->
```rust
            // Dangerous. See the note above this function: on a machine reached over the network
            // this takes away the channel the undo would travel on.
            Action::new(
                "wifi_disconnect",
                "Leave the Wi-Fi network this machine is on. On a machine reached over Wi-Fi \
                 this ends that connection and cannot be undone remotely.",
            )
            .risk("dangerous"),
```

Blender's `run_python` is `dangerous` because arbitrary code can do anything the person can;
LibreOffice's `save` is `sensitive` because it replaces the file the document came from
([`adapters/libreoffice`](../../adapters/libreoffice/README.md#the-actions-and-why-each-is-graded-as-it-is)
argues each of its nine grades).

## Four rules of thumb

**Grade by the worst the arguments allow.** A grade is per action, not per call: the dispatch
decides before your handler sees the arguments. An action that overwrites a file *if one is
there* is an action that overwrites files.

**So refuse the argument that would raise the grade, or split the action.** LibreOffice's
`save_as` and `export_pdf` only ever create a file — a path where something already is, is refused
— and that refusal is what keeps them `standard`. Replacing a file is `save`, a separate action,
and `sensitive`. Its `close` refuses a document with unsaved changes rather than becoming a
`sensitive` action that sometimes throws edits away. A handler's refusal is a sentence, so say
what to do instead:

<!-- from: adapters/libreoffice/yantrik_libreoffice/office.py -->
```python
        if os.path.lexists(path):
            raise OfficeError("a file is already at %s, and this action never replaces one; "
                              "choose another path" % path)
```

**Say it when it cannot be undone.** A description that says so — any of *not recoverable*,
*cannot be undone*, *can't be undone*, *irreversible*, *permanently*, *permanent*, *no undo* —
changes how the dispatch treats the action: it is asked about in every mode but `bypass`, whatever
its grade above `safe`, and no "Allow for this session" covers it. The calendar's `delete_event`
says "It is not recoverable" for exactly that reason. Leaving the words out is a decision too:
LibreOffice's `save` describes what it replaces without them, because a person who chose `auto`
mode has said `sensitive` work may run unasked, and the words would take that away.

**Grades can move while the program runs.** Studio's `generate` sends a prompt to a hosted service
when it is configured to, and that is `sensitive`; to a model on the machine, `standard`. It
regrades when the configuration changes — `control::regrade` in Rust, `surface.regrade(name,
grade)` in Python — and a caller sees the new grade in the next `describe`.

## What happens to a call

Whoever sends it — a mind's tools, `yos act`, the MCP bridge, a raw client on the socket — an
`app.act` meets the same steps, in this order, and the first that refuses ends it
([the protocol](../surface-protocol.md#5-appact), §5):

1. the action exists;
2. **the agent's reach**, if the call carries an agent's token (see below);
3. the arguments: an object, none missing, none undeclared, each of its declared type or converting
   to it without loss — so a person's Allow is never used up on a call that was never going to run;
4. **the ceiling**, on the grade;
5. **the grant**, if the call carries one, spent now — against the arguments as they were sent;
6. **the mode**, the session rules, and the description's "cannot be undone";
7. `expect_revision`, if given (`STALE:`);
8. your handler.

### The machine's ceiling

`tool_permission` in `~/.config/yantrik/settings.yaml`, `sensitive` when unset. Nothing above it
runs — not with any mode, not with a grant:

```text
CEILING: libreoffice.save is graded `sensitive`, above this machine's `standard` ceiling (`tool_permission` in ~/.config/yantrik/settings.yaml), so it was not run. …
```

The ceiling is the person saying "nothing on this machine does more than this, whoever asks". A
`dangerous` action on a machine that ships with a `sensitive` ceiling does not run until the
person raises it in Settings.

### The person's mode

Chosen from a chip in the status bar, written by the shell to `mind-mode.json`, and read by the
dispatch on every call. Each mode runs actions unasked up to a grade: `plan` nothing above `safe`
(a mind says what it would do instead), `ask` up to `standard`, `auto` up to `sensitive`,
`bypass` everything under the ceiling (for a while — a bypass expires). Above that, the call is
refused with `GRANT:`, and the refusal says how to get the person's Allow; `yos act` and the MCP
bridge do that for a mind, putting a card on the screen and acting again with the grant. On the
ceiling this OS ships with, for an action whose description does and does not say it cannot be
undone:

<!-- output: grade-table -->
```text
sensitive ceiling                    plan             ask              auto             bypass
safe                                 runs             runs             runs             runs
standard                             runs             runs             runs             runs
sensitive                            refused: plan    asks             runs             runs
dangerous                            refused: ceiling refused: ceiling refused: ceiling refused: ceiling
safe, says it cannot be undone       runs             runs             runs             runs
standard, says it cannot be undone   refused: plan    asks             asks             runs
sensitive, says it cannot be undone  refused: plan    asks             asks             runs
dangerous, says it cannot be undone  refused: ceiling refused: ceiling refused: ceiling refused: ceiling
```

(Generated from the SDK's `gate.decide` by `samples/test_guide.py`. `standard` runs in `plan` on a
socket because the desktop's own processes make `standard` calls; the doors that raise cards — the
shell and the MCP bridge — refuse it in `plan` and tell the mind to say what it would do.)

A **session rule** — the card's "Allow for this session" — runs one action unasked for the rest of
the session, except one that says it cannot be undone, and never in `plan`.

### A grant

A person's Allow for exactly one call: this app, this action, these arguments, once. The dispatch
spends it through the shell only after the arguments and the ceiling have passed, and only to the
desktop's own shell process. You do nothing to support grants; the SDK does it.

### An agent's reach

A person can start an agent from a role — a Reviewer, a Coder, a Planner — and a role carries a
**reach**: the surfaces it may act on and a ceiling of its own, narrower than the machine's. A call
made for such an agent carries its token beside the arguments, and the dispatch — Rust or
Python — holds it to the reach before anything else is looked at:

```text
REACH: counter.reset is outside the Counter's reach, so it was not run. …
```

A reach only takes away: nothing in it lets a call past the machine's ceiling or the person's
mode. The shell keeps every agent's reach, and the dispatch asks it what a token is; a token no
live agent carries, or any token while the shell does not answer, is refused. You do nothing to
support this; the SDK does it. Grade your surface as if any agent might call it anyway, which is
what a grade is for.

## Where to go next

[Designing a describe a mind can use](describe.md) — the words the grade sits beside.
