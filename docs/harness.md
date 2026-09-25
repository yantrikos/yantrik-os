# Attaching a harness

Yantrik OS does not run your agent. It does not hold your endpoint, your model name or your API
key, and it has nowhere to put them. `yantrik-mind` has its own setup, hermes-agent has its own,
OpenClaw has its own — that work is done, and doing it again in this OS would mean doing it worse
and keeping it in sync forever.

What the OS owns is **which mind the person is talking to**. This is the interface for becoming
one of the candidates.

## The whole thing

Seven methods on the `harness` socket, all spoken by the harness:

```text
harness.attach   {id, name, detail?, tools?, memory?, conversations?}  → {session}
harness.poll     {session}              → {turn_id, text, context, conversation, agent_token} | {}
                                          … either may also carry cancelled: [turn_id], ended: [conversation]
harness.chunk    {session, turn_id, delta}             → {}
harness.event    {session, turn_id, event}             → {}      (optional — see below)
harness.complete {session, turn_id}                    → {}
harness.fail     {session, turn_id, error}             → {}
harness.detach   {session}                             → {}
```

Attach, then loop: ask for a turn, stream the answer back in pieces, say you are done.
`crates/yantrik-harness/examples/echo_harness.rs` is a working one end to end, and the only part
a real harness replaces is the function that produces the answer. `harness.event`,
`conversations`, `conversation`, `agent_token`, `cancelled` and `ended` are all optional to use:
a harness that knows none of them works exactly as it always did.

## Why the harness dials in

The OS never connects to you. That is deliberate, and three useful things follow:

- **Anything with a JSON-RPC client can be a harness.** No callback URL, no inbound port, no
  reachability requirement. A harness in a container or behind NAT works like one running beside
  the shell.
- **The OS stores nothing about you.** There is no config file to install and no field anywhere
  for an endpoint or a credential. The protocol has no place to put one, and there is a test that
  fails if a field is ever added.
- **Being attached is what makes you exist.** You appear in the picker when you attach and are
  gone when you stop polling. Nothing to deregister, and nothing can be listed that is not
  actually there.

## …and the one place that rule is wrong

"Being attached is what makes you exist" is right for the *answering* list — you cannot be handed
a turn if you are not there — and it was wrong for **Settings → Harnesses**, which is the page a
person opens *because* a mind is missing. A harness the image ships but nobody installed, or one
whose unit died with a shell restart, was simply absent from it: no hint that the machine carries
it, that it needs an `npm install`, that it needs a config file, or that a unit merely needs
starting.

So a harness that ships with the image also carries a **manifest** beside its code, and the
Settings list is the manifests rather than the attach registry:

```yaml
# /opt/yantrik/share/harnesses/pi/harness.yaml
id: pi
name: Pi
detail: The pi coding agent, over its RPC mode   # only until it attaches and says better
docs: README.md
unit: yantrik-pi.service                          # omit for a harness something else starts

requires:                                         # missing any → "Not installed"
  - binary: pi                                    # on PATH
    why: Pi itself, which npm installs per user
  - file: yantrik_pi.py                           # beside the manifest, absolute, or ~/…
    why: the harness script

install:                                          # omit and the row names the docs instead
  command: npm install -g --ignore-scripts @earendil-works/pi-coding-agent
  doing: fetching @earendil-works/pi-coding-agent

setup:                                            # missing any → "Needs setup"
  - config: pi.json                               # ~/.config/yantrik/pi.json
    why: which provider and model Pi should use
```

From that, `PATH`, the config directory, systemd and the attach registry, each row derives one
state — **Not installed**, **Installing**, **Needs setup**, **Ready to start**, **Starting up**,
**Would not start**, **Attached**, **Answering** — and offers the one thing that moves it on:
*Install* runs `install.command` as a user job with its output streamed into the row, *Start*
does `systemctl --user enable --now <unit>`, and *Use this* appears only once something has
actually attached. The same list is in `describe shell` under `harnesses`, and the same two jobs
are `install_harness` and `start_harness`, both graded `sensitive` because they change the
machine.

Two rules this page keeps:

- **A manifest names a file; it never opens one.** A "Needs setup" row says
  `~/.config/yantrik/deepseek.json` and the name of the variable that holds the key, and that is
  the end of it. Nothing in the catalogue reads a config, so nothing it can print contains a
  credential, and there is a test that writes a key into a fixture and asserts no row carries it.
- **Only attachment makes a mind answerable.** The picker, the quick switcher and `use_harness`
  are unchanged and still work off the attach registry alone. A row on this page saying "Ready to
  start" is a row that cannot be selected, which is the truth.

Nothing about this is required of a harness. A manifest is how something gets *listed before it
runs*; attaching is still the whole protocol, and a harness that just attaches works exactly as
it always did — it appears in both lists the moment it does.

## Driving the desktop

Separate, and it already exists. An attached harness reads and steers the OS through the control
surface every app publishes — `app.describe` and `app.act`, or the `yos` command — which is
graded `safe`/`standard`/`sensitive`/`dangerous` and enforced against your ceiling. See
[app-control.md](app-control.md).

What day it is, is a read. `describe shell` carries `clock` — an object with `date`,
`weekday`, `time`, `utc_offset` and `zone`, like `2026-09-23`, `Wednesday`, `18:31`,
`-05:00`, `America/Chicago` (`zone` is empty on a machine that names none) — and
`describe calendar` carries `today`. A mind that needs the date reads it there rather than
running `date` through `shell.agent_run`: that is graded sensitive, so learning the day
raises an approval card for the person, and the desktop already knows the answer.

Attaching is about the conversation. Driving is about the desktop. Keeping them apart means a
harness can do either without the other: a mind that only talks never needs permissions, and a
script that only acts never needs to attach.

## Agents: conversations, tokens and events

The desktop's **Agents** view (`design/agents-workspace-2026-09-23.md`) shows one pane per
agent, and an agent is **one conversation with one mind**: `<harness>:<conversation>`, like
`pi:c-7f3a91`.

**Conversations.** A harness that can hold more than one conversation at a time — a process per
conversation, a history per conversation — says `conversations: true` when it attaches. The
desktop then issues conversation ids itself (`c-` and six random hex digits, never issued twice,
so an id from an earlier session cannot name a live agent), and every turn names its
`conversation`. A harness that does not say so has the one agent `<id>:main` and gets every turn
in `main`; the Agents view says "holds one conversation at a time" rather than pretending
otherwise. The Lens always talks to the answering mind's `main`. The desktop runs at most six
live agents at once, across every mind, and says so when asked for a seventh.

**One turn at a time per conversation, first in first out.** The desktop hands a conversation its
next turn only once the one in flight is completed or failed; different conversations run at
once. `/stop` alone does not wait, because it is how a person interrupts the turn that is
running. (Turns used to be handed out newest first; they are not any more.)

**Agent tokens.** Every turn carries `agent_token`: 128 random bits as hex, minted when the agent
was created and the same for every turn of that conversation. Pass it to the tools you start for
that conversation as `YANTRIK_AGENT_TOKEN` — the `yos-mcp` bridge a conversation uses — and to
nothing else: not the model, not a log. The shell resolves a token back to its agent, and checks
the caller descends from the process that attached (the kernel's `SO_PEERCRED` at `attach`), so an
act can be recorded against the agent that asked. The limit is stated in the design: processes of
the same user can read each other's environment, so this stops confusion and casual
impersonation, not a hostile program running as the person.

A harness that attached without a pid the kernel could report (the TCP dev path) vouches for
nobody: every call carrying one of its tokens is refused, and the refusal says why.

**What the bridge does with it.** A `yos-mcp` started with `YANTRIK_AGENT_TOKEN`:

- carries it on every act, beside the arguments — through `yos`'s environment, which `yos` sends
  as the top-level `agent_token` of `app.act`. Never on a command line (any user can read another
  process's), never among the arguments, and so never on an approval card or in
  `mind-audit.jsonl`; an `agent_token` a model puts into `args` is dropped, and nothing the bridge
  returns or logs repeats the token;
- runs `os_act terminal.run {command}` as `shell.agent_run {command}` instead: the command gets a
  terminal of the agent's own in its pane, the person's Terminal is not opened, typed into or
  raised, and the answer carries the exit code.

It also offers the agent's terminal as tools of their own, listed only with a token:

| tool | runs | grade |
|---|---|---|
| `run_command {command, cwd?, wait_seconds?}` | `shell.agent_run` | sensitive: a card in `ask` mode |
| `command_status {job, wait_seconds?}` | `shell.agent_job` | standard |
| `command_input {job, text}` | `shell.agent_input` | sensitive |
| `command_kill {job}` | `shell.agent_kill` | standard |

`run_command` answers when the command has ended — exit code, the directory it ended in, the end
of its output — or, still going after `wait_seconds` (120 by default, at most 600), with a `job`
id. Each answer also carries the shell's own account under `_meta` (`yantrik/command`), for a
client that wants the exit code as a number. A call that waits for a command can take as long as
its wait on top of an `os_act`'s own budget, so a client allows `wait_seconds` more than its
usual timeout (`mcp_timeout` in the Python library does, and so does pi's extension).

And it offers the ways to hand work to another agent — also only with a token:

| tool | runs | grade |
|---|---|---|
| `new_agent {mind, task}` | `shell.new_agent` | sensitive: a card in `ask` mode, in the asking agent's pane |
| `send_to_agent {agent, text}` | `shell.send_to_agent` | standard |
| `stop_agent {agent}` | `shell.stop_agent` | standard |
| `read_agent {agent, last?}` | `shell.read_agent` | safe |

`new_agent` answers at once with the new agent's id (`pi:c-1a2b3c`); its row on the Agents screen
says who started it. The shell holds the caller to its token, never to an argument: an agent
another agent started cannot start agents of its own, one agent holds at most three running at
once, the desktop's cap of six still applies, and an agent may send to, stop and read only the
agents it started (and read itself). A child starts with nothing of its parent's — its first turn
is its task and the desktop's context, with a token of its own, and a grant asked for one agent
is not spent by another through `consume_approval`. Stop on a parent stops its children.
`read_agent` answers with the agent's recent turns as text, and counts as reading private state
for the bridge's taint rule: after it, the bridge will not type into a page.

And one more, for handing work to a role from the desktop's **agent catalog** rather than to a
mind (design/desk-and-mind-2026-09-23.md, section 5):

| tool | runs | grade |
|---|---|---|
| `hand_off {role, task, context?, wait_seconds?}` | `shell.hand_off` | sensitive, as `new_agent` |

A role — Researcher, Planner, Coder, Reviewer, Red team, Writer, Chair, Scribe, or one of the
person's own in `~/.config/yantrik/agents/*.toml` — names the minds that run it, best first; the
shell starts it on the first one attached that holds a conversation per agent, with the role's
standing instructions, the task and the context as its first turn. `describe shell` lists the roles
under `catalog`. The same rules as `new_agent` apply, and the role's **reach** caps it further: its
agent is held to the role's surfaces and grade ceiling on every door that carries its token, and
anything else is refused with `REACH:` (the bridge relays that as a policy answer, and never puts a
question outside the reach to the person) — except that the role may open an app its reach names:
`shell.open_app` with its name is within the reach, whatever the ceiling, because a closed app
cannot be read and opening a window is not an act on its data. With `wait_seconds` the call waits for the role's
answer and hands it back — which counts as reading private state — so a client allows
`wait_seconds` more, as for `run_command`.

### Formations

A **formation** is a recipe whose steps are catalog roles (design/desk-and-mind-2026-09-23.md,
section 6): an `Agent {role, prompt, store_as, context?}` step hands a turn to a role through the
same `hand_off`, and keeps its answer for the steps after it. Agent steps that do not read each
other's answers work at the same time — at most three at once; a fourth waits for a place — and a
step that reads an answer waits for it, without holding up the companion's worker. Four ship as
built-ins: **Council** (three seats answer one question at once, the Chair weighs them), **Red
team** (author, attacker, two rounds), **Build** (Planner → Coder → Reviewer, the findings back to
the Coder once) and **Writers' room** (three writers, one voice each, the Scribe assembles).
`describe shell` → `recipes` → `formations` lists them with their inputs.

A mind starts one with `os_act shell run_recipe {recipe, inputs}` — **sensitive**, so in `ask`
mode the person sees the card first — and the person with Start on the Recipes screen. Either is
the run's leave for its agents, and it records the digest of each role's definition as it is at
that moment: a role whose definition changes before its step (a file in `~/.config/yantrik/agents`
replacing it, with another mind, brief or reach), or one the run names only later, is refused with
a sentence, never run. The companion's own `run_recipe` tool, graded standard, refuses a
formation. A run nobody started at the desk (a trigger or a timer, once #187 wires them) asks the
person on a card naming the recipe and the role before any role above `safe`; denied or left to
expire, the step fails.

The hand-off is from the recipe: each agent's row and its approval cards say "Council recipe →
Reviewer", and every start without a card of its own is written to the record of unasked actions
under the recipe's name. It is held to its role's reach, and it is a child — an agent a recipe
started cannot start agents of its own; an agent another agent or a recipe started cannot start a
formation. No place under the desktop's cap of six, or under the asking agent's three, queues the
start rather than racing the person's own: the recipe waits at the step, **needs you** on the
Recipes screen and in the mind panel with the reason, for at most 30 minutes, then fails saying
what it waited for. An agent waiting on the person in its own pane makes its recipe need them too;
a card of its that expires unanswered, or that the person denies, fails the step — never "done"
with whatever it said after. An agent that fails, runs past its role's minutes, or is gone after a
restart fails the recipe with the reason; one that answered before a restart is heard from its
saved session. `cancel_recipe` lets its agents go.

An approval a call with a token asks for is drawn in that agent's pane as well as in the Lens —
the same card under the same request id, so answering either answers both — and the card names
the agent. A token that does not check out puts the card in no pane and says so on it.

Without a token, the bridge behaves exactly as it did.

**Notes for the next turn.** A command still running when its call returned (`running: true`)
that finishes later is reported to its agent at the start of its next turn: the turn's `context`
carries `"notes": ["Your command `make` (job …) finished after the call that started it had
returned: exit code 0, after 2m 14s, in /home/me/proj. Its last lines: …"]`. A note is
delivered once, an agent holds at most eight, and they end with the agent. Not on `/stop` or
`/new`, which the harness answers itself; the turn after carries them. A harness whose model sees
nothing else of the context should show it these: `turn.notes_before(turn.text)` puts them in
front of the person's message, which is what pi does.

**Stopping.** When the person stops an agent, the turns waiting for it are failed, the one in
flight is settled for the reader at once, and the harness's next poll carries `cancelled: [turn_id]`
(stop working on it) and, for a harness with conversations, `ended: [conversation]` (let go of its
process and history). Both are advisory. Whatever the harness still sends for a cancelled turn is
answered `{"dropped": true}`, and closing it is accepted. A harness that attaches again — it
restarted, or the desktop did — gets new agents; the old conversations and their tokens are gone.

**Events.** Text still travels as `harness.chunk`. Beside it, `harness.event` says what the agent
is *doing*, and the Agents view draws each tool call as a card:

| `kind` | carries | shown as |
|---|---|---|
| `tool_start` | `call`, `name`, `target`, `args` | a card opens, running |
| `tool_output` | `call`, `stream` (`stdout`/`stderr`/`terminal`), `delta` | text inside that card |
| `tool_end` | `call`, `ok`, `summary`, `exit_code?` | the card settles ✓ / ✗ |
| `thinking` | `delta` | a folded "thinking" line |
| `status` | `text` | the agent's state line |
| `usage` | `model`, `input_tokens?`, `output_tokens?`, `cost_usd?` | the details panel; they add up |

`call` is your own id for the call (pi's `toolCallId`, an OpenAI `tool_call.id`), unique within
the turn. The types are `crates/yantrik-harness/src/event.rs`. The desktop enforces a lifecycle,
and a refusal is an answer (`{"refused": why}`), never an error — an event that could not be shown
is not a reason to lose the turn:

- an event is accepted only for a turn in flight, from the session that holds it;
- a call's events come in order: `tool_start`, its output, one `tool_end`;
- `complete` and `fail` are the end: events after them are dropped and counted, and a call still
  open is settled for the reader as *interrupted* — as it is when a harness detaches, restarts,
  goes quiet or is stopped;
- one event is at most 64 KiB as JSON; send a long output as several `tool_output` events;
- an event of a kind the desktop does not know is ignored (`{"ignored": …}`) — a newer harness
  must not break an older desktop — and a malformed one of a kind it knows is logged and counted.

**Events are the harness's claims.** The pane marks cards from events as *reported*, apart from
what the shell verified itself. Keep writing the trail line (`⚙️ …`, below) into the text as
well: every panel that draws no cards, and every reader of the transcript, still sees the call.

**In Python** (`harnesses/lib/yantrik_harness.py`), a `Turn` has all of it:

```python
turn.conversation, turn.agent_token            # which agent this turn is for
turn.notes, turn.notes_before(turn.text)        # what the desktop has to tell it since its last turn
turn.tool_start(call, name, target="", args={}) # also writes the trail line (trail=False not to)
turn.tool_output(call, delta, stream="stdout")  # cut into pieces under 64 KiB for you
turn.tool_end(call, ok, summary="", exit_code=None)
turn.thinking(delta); turn.status(text)
turn.usage(model="", input_tokens=None, output_tokens=None, cost_usd=None)
```

and a mind that holds one conversation holds many by being made once per conversation:

```python
Harness("pi", "Pi", PerConversation(lambda conversation, token: PiMind(config, token=token)))
```

`PerConversation` announces `conversations`, makes a handler when a conversation's first turn
arrives, closes it (`close()`) when the desktop ends that conversation or a turn arrives under a
new token, and keeps `concurrent = False` per conversation: a second message to the same
conversation still gets "still working on the previous request", while different conversations
run at once. `/stop` and `/new` act on their own conversation only. Against a desktop too old for
`harness.event`, the events are skipped after one log line and the trail lines carry on.

## Rules worth knowing

- **`id` is what a person types to select you**, so it cannot be empty or contain spaces.
- **You cannot attach over a built-in.** The companion is compiled into the shell; a client
  taking its id could leave a machine with no working mind and no way to say so.
- **Re-attaching under the same id replaces the old you.** That is what a harness that crashed
  and came back should get, and any turn the old one owed is failed rather than left hanging.
- **Stop polling and you are dropped** after 90 seconds, with anything you owed failed so nobody
  is left waiting on an answer that is not coming.
- **Nothing waiting is an ordinary reply**, not an error. You will poll far more often than a
  person types.
- **A tool call is a line of the answer beginning with `⚙️`.** `chunk` carries text and nothing
  else, so a call goes into the text: `⚙️ os_act studio.generate {"args":{"prompt":"a red
  kite"}}` — the tool's name, what it touched, and the rest of its arguments as one JSON object
  on the same line, which is what `harnesses/lib/yantrik_harness.py`'s `tool_trail` writes. The
  shell renders that line as the call — name, target, `key="value"` — cut to the panel's width,
  with the whole arguments a click away, and reads the same calls out under `calls` in
  `describe shell`'s `conversation`. It also understands Hermes' own progress lines
  (`⚙️ name...`, `⚙️ name: "preview"`, and the verbose `⚙️ name([...])` with the arguments on the
  line after), so a harness that already writes its calls down does not need this form. Anything
  a line carries is what the panel can show: Hermes in its default mode sends the name alone
  for an MCP tool, and the panel shows the name alone. A harness that also sends
  `harness.event` (above) keeps writing this line: the event is the call's card in the Agents
  view, the line is the call in the text, and `turn.tool_start` writes both.

## Five harnesses exist

**Yantrik Mind** attaches from its own process (`crates/mind-core/src/harness.rs` in its repo).
It is the reference for a mind written in Rust that already has its own model and memory.

**Hermes Agent** attaches through a plugin this repo ships, `harnesses/hermes`, because Hermes
is a gateway with its own platforms (Telegram, Slack, IRC) and this makes the desktop one more
of them. To install it on a machine that already runs Hermes:

```sh
cp -r harnesses/hermes ~/.hermes/plugins/yantrik
hermes plugins enable yantrik-desktop
systemctl --user restart hermes-gateway    # or however Hermes is started
yos act shell use_harness id=hermes        # once it appears in the picker
```

Hermes keeps its model, endpoint, keys and memory in `~/.hermes`, as it always has. The plugin
reads none of it except the model name, which it passes as the `detail` the picker shows.

**Give the desktop platform the desktop's tools, not Hermes's own.** Hermes arrives with a
`terminal`, `file`, `code_execution`, `browser` and `web` toolset of its own. On this desktop
they are a second, ungraded route to everything the apps already offer — and each `terminal`
call stops on Hermes's own approval, which reaches the person as a paragraph of text to answer
with `/approve`, five minutes at a time. The first long job given to it (research, slides,
calendar, a checklist) spent most of its life waiting on those. With them off it did the same
job through the Terminal, Browser, Presentation, Calendar and Notes *apps*, where every action
carries the app's grade, the person's mode decides what is asked, and the work is on screen.
The `hermes tools` command does not know plugin platforms, so set it in `~/.hermes/config.yaml`:

```yaml
platform_toolsets:
  yantrik: [skills, todo, memory, session_search, clarify, delegation, yantrik_os]
delegation:
  max_iterations: 25        # a research sub-agent that may take 50 turns will take 50
```

`delegation` is worth keeping: a sub-agent inherits the desktop's tools, so "spin up a research
agent" works and its work is as visible and as graded as the parent's.

Two things a gateway-shaped harness has to get right, both learned by running one:

- **Close every turn exactly once.** The desktop is waiting on the turn it handed over, and a
  gateway has paths that answer without going through its own completion hook — a `/stop`, a
  command answered inline. Anything that leaves a turn open leaves the desktop waiting forever,
  and a heartbeat keeps it waiting convincingly.
- **A message that arrives while you are working is a turn too.** Queueing it behind the current
  one is fine for a chat app, where nothing is owed; here the turn it came from is owed an
  answer. Answer it — even if the answer is "still working on the last one". (The desktop now
  does the queueing itself — a conversation is handed one turn at a time, and only `/stop`
  arrives mid-turn — so this is the rule for an older desktop, and a backstop for this one.)

**Pi** (`harnesses/pi`) is the [pi coding agent](https://www.npmjs.com/package/@earendil-works/pi-coding-agent)
driven over its RPC mode: `pi --mode rpc` on a pipe, one JSON line per message, each desktop turn
fed in as a `prompt`. It assumes Pi brings everything — provider, key, model, session, loop — and
that the harness's whole job is carrying text and closing turns.

Pi has no MCP client, so the desktop's tools reach it through a Pi extension
(`harnesses/pi/extension/yantrik-os.ts`) that asks `yos-mcp` for its tool list and proxies every
call to it. The extension decides nothing: modes, grades, cards and the taint rule stay in the
bridge, which is the only place they can be kept correct.

Pi's own tools are off (`--no-builtin-tools`) unless the person turns them on, and when they are
off and the bridge offers `run_command` — Pi is running as one of the person's agents — the
extension registers a `bash` of its own with Pi's exact shape (`{command, timeout?}`, timeout in
seconds, none by default) on top of `run_command`. Pi keeps the tool it was trained on, answering
as Pi's does ("Command exited with code N", "Command timed out after N seconds", the command
stopped), and the command runs in the agent's terminal in its pane. A command that stops to ask
for input comes back still running, saying so, rather than hanging: the person can answer it in
its card. With Pi's built-in tools on, the extension never shadows them.

Three things it taught, all about ending:

- **An agent has more ways to finish than to start.** `agent_settled`, an `agent_end` that is
  never followed by one, a `response` that says the prompt was refused, and the process exiting
  are four different endings, and each one is a turn the desktop is holding open. They all have
  to arrive at the same single close.
- **Silence is not an ending, but it has to become one.** A harness that waits forever on a mind
  that has stopped talking leaves the person watching a cursor. Failing after a while is worse
  than answering and better than hanging — and the timeout has to exceed the longest legitimate
  silence, which on this desktop is an `os_act` waiting about 270 seconds for someone to answer
  an approval card.
- **A harness must not answer a dialog.** Pi can open its own `confirm`, and answering it is the
  most natural thing in the world to automate. It is declined, and the question is repeated into
  the conversation instead: this desktop already asks for permission in a way the person sees and
  the machine records, and a second approval path that nobody can see is worse than an
  inconvenient one.

Pi's own `bash`, `read`, `write` and `edit` are off by default, for the reason in the Hermes
section above — on this desktop they are an ungraded second route to what the apps already do.
Turning them on is one line in `~/.config/yantrik/pi.json` and is the person's call.

**DeepSeek** (`harnesses/deepseek`) is the opposite end: no agent, just a model. It is a plain
tool-calling loop over an OpenAI-compatible `/chat/completions` — stream the answer, collect the
tool calls, run them through `yos-mcp`, append the results, go again — and it is the reference
for attaching something that is only an endpoint and a model name. Nothing in it is
DeepSeek-specific but the defaults; it is tested against a fake server and runs unchanged against
any OpenAI-compatible endpoint, which is how it can be exercised on a machine with no DeepSeek
key at all.

What it assumes, and what that cost:

- **Tool calls arrive in pieces.** A streamed `tool_calls` delta splits the function name, the
  id and the arguments across chunks, several calls can be in flight in one assistant message,
  and some providers resend the whole name on every chunk instead of a fragment. Concatenating
  blindly turns `os_act` into `os_actos_act` and the call comes back as an unknown tool.
- **A model's thinking is not its answer.** `reasoning_content` is dropped rather than streamed:
  the person asked a question, and on a panel the deliberation reads as rambling.
- **The key exists in exactly one place.** It goes in the `Authorization` header and nowhere
  else — not into a log, a chunk, an exception, or the conversation history if the provider
  echoes it back. Every string the module can produce is redacted, and a test drives the whole
  loop against a server that deliberately echoes the header to prove it.
- **Everything the mind reads goes to the provider.** Every tool result — the note it opened, the
  page it read, the calendar it looked at — is in the next request, because that is what a
  tool-calling loop is. The README says so in those words, and the config file is the person's
  rather than the machine's.

It is also where the **decider** is, because this is the harness whose loop is small enough to
see the seam in. A `decider` block in `~/.config/yantrik/deepseek.json` names a decision model —
Jev (`POST /v1/systemone`, TypeSafe), Kev (the same wire, served locally) or a `/v1/decide`
endpoint — and each step it is asked, in one request, whether the request is finished, which app
the next step uses, and which of that app's actions. Above a confidence gate the chat model is
then given `os_act` as its only tool, with `tool_choice` on it and the app and action already
fixed in the schema, so all it writes is the arguments. Below the gate the generative tool call
happens exactly as before and the disagreement is logged. No block means none of it runs, and
the whole thing is tested offline against a fake System One server.

**It picks; it does not write.** That line is the design and not a caveat. The decider chooses
among things the desktop already published — an app in `os_apps`, an action in `os_describe` —
and answers one yes/no question; every word that reaches an app is still the generator's. Four
things that shape carries with it:

- **A decision model can only choose an option it was given**, so the option list has to be
  complete or not offered at all. An `os_describe` answer that was too long to keep whole is a
  partial action list, and a partial list is worse than none, because it does not make the
  answer uncertain — it makes it confidently wrong. `none of these` is always among the apps,
  which is what turns "nothing here applies" into an answer rather than a wrong app.
- **The cheap half cannot depend on the expensive half's habits.** Attached to a live desktop,
  Kev answered nothing at all on the first real question put to it: the system prompt asks the
  model to start with `os_apps`, and a model that already knows this desktop went straight to
  `os_describe` — a good answer that left the decider with no list of apps and nothing it could
  be asked. So the harness reads `os_apps` itself at the start of a question, once, when the
  conversation does not already hold a listing. It shows in the trail like any other tool call,
  because a tool call the person cannot see is worse than a line they did not need.
- **Standing aside has to be as loud as falling back.** The first version of this returned
  quietly when there was nothing to ask, and the failure above was invisible in the journal —
  indistinguishable from a decider that was asked and disagreed. Every step the decider is not
  asked now says so and why.
- **Questions in one request cannot read each other, and the id is not sent to the model.** So
  each question's instructions carry their whole meaning, and the speculative ones say so —
  "suppose the next step is taken with `calendar`, whether or not another app would be a better
  choice; that is decided elsewhere".
- **A refusal is not a thing to decide around.** After a `REFUSED` the decider is stood down for
  the rest of that question. What a refusal means is in the system prompt, which the generator
  reads and the decider is not shown, and the wrong reading of it — another route to the same
  thing — is exactly what a fast pick would produce.
- **The gate is the whole of the protection, and it is a probability rather than a measured
  accuracy rate.** A pinned step is a step the model is forced to take, so the only honest thing
  to do with the number is log it: one line per step with the pick, the probability, the latency,
  whether the gate held, and what the generator picked on every step where both of them did.

**OpenClaw** (`harnesses/openclaw`) is the local-first personal agent: a gateway daemon on
127.0.0.1, a primary agent that spawns sub-agents, its own channels, and persistent memory. It is
the only one attached with `memory=true` and the only one whose tools this repo does not carry —
OpenClaw has an MCP client of its own, so `yos-mcp` is registered in the person's
`~/.openclaw/openclaw.json` under `mcp.servers` with `env.YOS_MCP_REQUESTER=OpenClaw`. Nothing
about that passes through this harness, which is the right shape: a mind that already knows how
to hold tools should be given the tools, not a proxy for them.

This harness was written on a machine with no OpenClaw checkout and no network, and then run
against **OpenClaw 2026.9.1** on the live image. Everything below is what that install does;
where it disagreed with the blind version, the disagreement is recorded rather than quietly
fixed, because the failure modes are the interesting part.

- **`openclaw agent`'s flags were guessed wrong twice, and `--json` is not a stream.** The
  message is `--message <text>`, not a positional; the session is `--session-key <key>`, and
  there is no `--session`; and `--json` prints one indented JSON document when the turn is over
  (`{runId, status, summary, result: {payloads, meta}}`). A JSON-lines reader pointed at that
  document fails on every line and emits each one as text, so the first thing the person would
  have seen on this route is the raw JSON in their chat panel. The harness now collects stdout
  whole, and a run that prints nothing readable is reported as that rather than as an empty
  answer — an empty bubble cannot be told from a hang.
- **The tool trail exists on one route and not the other, and it says so.** The CLI route builds
  it from the run's own `result.meta.toolSummary`, which names the tools and not their arguments.
  The gateway route has none: OpenClaw's internal tool calls stay internal on that surface.
- **The streaming route is HTTP, not the WebSocket, and that is an authorization fact rather
  than a spelling one.** The gateway's control plane is `{type:"req", id, method, params}` at the
  bare root path — the blind envelope was wrong, and so was the path probing. But correcting the
  spelling would not have helped: a `connect` carrying only the shared gateway token is accepted
  with `auth.scopes: []`, so `chat.send` answers `FORBIDDEN / missing scope: operator.write`.
  Scopes come from device pairing — an Ed25519 identity signing a challenge-bound payload,
  approved with `openclaw devices approve` — and there is no Ed25519 in the standard library.
  The harness therefore uses the gateway's OpenAI-compatible route,
  `POST /v1/chat/completions` with `stream: true` and `x-openclaw-session-key`, which OpenClaw's
  own docs describe as "a normal Gateway agent run (same codepath as `openclaw agent`)". It is
  off until `gateway.http.endpoints.chatCompletions.enabled` is true, and the 404 that says so
  names that setting.
- **Tools are gated by the tool profile, not by a dashboard approval.** The blind version told
  people to approve a scope on the gateway's dashboard; there is no such step. A configured MCP
  server is a plugin-owned tool under `bundle-mcp`, and `tools.profile` decides whether the agent
  sees it — `minimal` plus `alsoAllow: ["bundle-mcp"]` is what leaves OpenClaw with the desktop's
  eleven tools and none of its own ungraded shell, file and browser tools. OpenClaw also prefixes
  a server's tools with its name, so the desktop's `os_apps` reaches the model as
  `yantrik-os__os_apps`; the harness's preamble names them that way, because a mind told to start
  with a tool it cannot see spends its first turn guessing.
- **An unrecognised event is reported as unrecognised.** The failure this exists for is a
  protocol mismatch that looks exactly like an agent which has gone quiet: a decoder that
  silently drops what it does not know turns a five-minute config fix into an afternoon.
- **A daemon that is not running is an answer, not a wait.** Connecting is retried with backoff
  and then the turn is failed with the sentence that names the command to run, which is the whole
  difference between a harness a person can debug and a cursor that never stops blinking.
- **Ending a stream is not the same as closing it.** Two versions of one bug. The shared
  `end_process` closes a child's stdout before terminating it, which deadlocks when a reader
  thread is blocked inside that stream, so the CLI route signals the child first. And
  `http.client` hands the socket to the response for any reply that will close the connection,
  clearing `HTTPConnection.sock` — so `conn.close()` on a live stream is a no-op and the reader
  blocks forever. The gateway route keeps the socket it captured before `getresponse()` and shuts
  that down. A `/stop` that leaves the reader blocked is a turn that never closes.

All four ship as source in the image and none is started: a machine that has not been configured
never talks to a provider. `harnesses/lib/yantrik_harness.py` is the half they share — attach,
poll, heartbeat, `/stop`, `/new`, the MCP client, and one `_close` that every path out of a turn
goes through — and `harnesses/tests` runs all of it offline against a fake desktop, a fake
bridge, a fake chat API, a fake System One endpoint, a fake `pi` and a fake OpenClaw gateway. The
fakes answer the way the real things answered when each harness was run against them, which is
the only reason the offline suite is worth anything: a fake that agrees with a guess proves the
guess is self-consistent and nothing else.

Pi and DeepSeek both hold conversations and send events. **Pi** runs one `pi --mode rpc`
process per conversation, started with that agent's `YANTRIK_AGENT_TOKEN` when its first message
arrives and stopped when the desktop ends it; `tool_execution_start / _update / _end` become a
card each (Pi reports a running call's output accumulated, so only what is new is sent), its
`thinking_delta` a folded thinking line, and each assistant `message_end`'s usage a `usage`
event. **DeepSeek** keeps a history and a `yos-mcp` bridge per conversation (the bridge is where
the token goes), sends each tool call it runs as a card with its result, `reasoning_content` as
thinking, and the token counts the API reports at the end of the stream
(`stream_options.include_usage`; `"include_usage": false` in its config for a server that
rejects the field).

## What is not here yet

Tasks, approvals as a first-class message and automations are designed
(`design/hermes-on-yantrik.html`, `design/agents-workspace-2026-09-23.md`) but not in the
protocol: an approval is drawn by the shell from the act that needs it, and a long task is one
turn with its progress streamed as text and events. Hermes and OpenClaw hold one conversation and
send no events yet; their calls reach the pane through their trail lines.
