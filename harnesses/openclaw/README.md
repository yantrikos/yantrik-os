# OpenClaw as the desktop's mind

A harness that gives the Yantrik OS conversation to [OpenClaw](https://github.com/openclaw/openclaw)
— the local-first personal agent with a gateway daemon on 127.0.0.1, a primary agent that spawns
sub-agents, and its own persistent memory. Python 3.10+, stdlib only, no pip install and nothing
to build.

OpenClaw keeps everything that makes it OpenClaw: its model, its keys, its channels, its memory
and its MCP servers, all in `~/.openclaw/openclaw.json` where it already keeps them. This harness
carries text between OpenClaw and the desktop's chat panel, and closes every turn exactly once.
It reads none of OpenClaw's configuration and the desktop protocol has no field that could carry
any of it.

**Install OpenClaw from its own documentation.** This directory assumes a working install and
says nothing about how to get one. One thing worth knowing before you start: OpenClaw's own
`engines` field is strict, and recent releases want Node 24 or 26. `openclaw@2026.9.1` is the
newest release that still accepts Node 22, which is what the Yantrik OS image carries.

## What was checked, and against what

This was written blind — no OpenClaw checkout, no network — and then run against **OpenClaw
2026.9.1** on a live Yantrik OS machine. Three of the blind guesses were wrong, and the table
that used to sit here saying which parts were assumed has been replaced by what the install
actually does:

| Guess | What is true |
|---|---|
| `openclaw agent "<message>"` | The message is `--message <text>`; a positional argument is ignored. |
| `--session <name>` | There is no `--session`. It is `--session-key <key>`; a bare key scopes to the selected agent. |
| `--json` streams JSON lines | `--json` prints **one indented JSON document when the turn is over**. Read line by line it is neither JSON nor an answer — it is the raw document, in the person's chat panel. |
| The gateway speaks a `{type:"message", text}` envelope on some path | The WebSocket control plane is `{type:"req", id, method, params}` at the bare root path, and it is not usable from here (below). |
| Tools need a scope approved on a dashboard | Tools need `bundle-mcp` in the tool profile. There is no dashboard step. |

## The two routes

**`"route": "cli"` — the default.** One `openclaw agent --json … --message <text>` per turn. It
needs no OpenClaw configuration beyond a working install, and its failure modes are all visible:
a flag OpenClaw does not recognise is a non-zero exit with a sentence naming `args`. What it
cannot do is stream — nothing arrives until the run is over — and it pays Node's start-up on
every turn, which on a modest machine is about five seconds before the model is even asked.

**`"route": "gateway"` — streaming.** `POST /v1/chat/completions` on the gateway's own port,
with Server-Sent Events. OpenClaw's docs describe this route as "a normal Gateway agent run (same
codepath as `openclaw agent`)", so it is the same agent, the same tools and the same memory; it
just streams and starts no process. Two things to set up first:

```json5
// ~/.openclaw/openclaw.json
{
  gateway: {
    http: { endpoints: { chatCompletions: { enabled: true } } },
  },
}
```

and the gateway's own token, because `gateway.auth.mode` is `token` by default. Put the variable
in an `EnvironmentFile` the unit names, and `"token_env"` in this harness's config. If the route
is off, the turn fails with a sentence naming that exact setting rather than a 404.

### Why not the gateway's WebSocket

Because a stdlib-only Python client cannot authorize on it. The framing is RFC 6455 and the
protocol is `{type:"req", id, method, params}` with `connect` as the mandatory first frame — all
of which is easy. The problem is what comes back:

```
{"ok": true, "payload": {"auth": {"role": "operator", "scopes": []}}}
```

An authenticated connection with **no scopes**, so `chat.send` answers `FORBIDDEN / missing
scope: operator.write`. Scopes come from device pairing: an Ed25519 identity signing a
challenge-bound payload, approved once with `openclaw devices approve`. There is no Ed25519 in
the standard library, and the HTTP route above reaches the same agent without any of it.

## Install

**Settings → Harnesses** lists OpenClaw whether or not it is installed. Its *Install* button
runs OpenClaw's own documented route, `npm install -g openclaw` (pinned to a tested version), through
`harnesses/lib/install/npm.sh`: a pinned Node 24 into `~/.local` first when there is none new
enough (the image ships none), then OpenClaw into `~/.local/bin`. Onboarding asks questions, so
it is yours to run: `openclaw onboard`. Once `openclaw` is there the *Start* button enables the
unit below.

```sh
mkdir -p ~/.config/yantrik
$EDITOR ~/.config/yantrik/openclaw.json          # optional — see below

# run it in the foreground first, to see what it says
python3 /opt/yantrik/share/harnesses/openclaw/yantrik_openclaw.py

# then as a user service
cp /opt/yantrik/share/harnesses/openclaw/yantrik-openclaw.service ~/.config/systemd/user/
$EDITOR ~/.config/systemd/user/yantrik-openclaw.service   # PATH, and the token's EnvironmentFile
systemctl --user daemon-reload
systemctl --user enable --now yantrik-openclaw

yos act shell use_harness id=openclaw            # once it appears in the picker
```

Nothing is enabled by default. The image ships this as source and starts nothing: a machine that
has never been configured never talks to a provider.

A user service does not inherit a login shell's PATH, and `openclaw` and `node` are per-user
installs. The unit puts `~/.local/bin` on its PATH, which is where *Install* puts both; for an
install anywhere else, set `path` in the config. Without one or the other the CLI route cannot
start OpenClaw at all, and even the picker's version line comes up blank.

## Giving OpenClaw the desktop's tools

OpenClaw has an MCP client of its own, so the desktop's tools do **not** go through this harness.
One command registers the bridge and probes it before saving:

```sh
openclaw mcp add yantrik-os --command /opt/yantrik/bin/yos-mcp --env YOS_MCP_REQUESTER=OpenClaw
openclaw mcp probe          # → yantrik-os: 11 tools
```

which writes the same thing you would write by hand:

```json
{
  "mcp": {
    "servers": {
      "yantrik-os": {
        "command": "/opt/yantrik/bin/yos-mcp",
        "env": { "YOS_MCP_REQUESTER": "OpenClaw" }
      }
    }
  }
}
```

`YOS_MCP_REQUESTER` is what the desktop's approval card prints on its **`says the caller`** line.
Without it the card reads "the mind on this desktop is asking to use this machine", which is true
and useless. It is a label and nothing more: the machine establishes who is really calling from
the socket's peer credentials and prints that separately, under `verified by this machine`, and
says so when the two disagree.

**The tools arrive prefixed.** OpenClaw exposes a configured MCP server's tools under a prefix
derived from the server's name, so the entry above turns `os_apps` into `yantrik-os__os_apps`.
Asking a live agent to list its own tools is how this was settled, and it is why the harness's
preamble names them that way — a mind told to "start with os_apps" and shown no such tool spends
its first turn guessing.

### The tool profile is the gate, not a dashboard

A configured MCP server is exposed as a plugin-owned tool under the `bundle-mcp` plugin id, and
`tools.profile` decides whether the agent sees it. The `coding` and `messaging` profiles allow
`bundle-mcp` implicitly; `minimal` does not. So the setting that makes the desktop reachable is:

```json5
{ tools: { profile: "minimal", alsoAllow: ["bundle-mcp"] } }
```

which is also the setting that answers the question every harness in this repo has had to answer:
**give the desktop's tools, not OpenClaw's own.** OpenClaw arrives with shell, file and browser
tools, and on this desktop they are a second, *ungraded* route to everything the apps already
offer — no card, no mode, no grade, no audit line. `minimal` plus `bundle-mcp` leaves the agent
with `session_status` and the eleven `yantrik-os__*` tools and nothing else. Leaving OpenClaw's
own tools on is the person's call, not this harness's, and it is worth making on purpose.

After editing `mcp.servers`, `openclaw mcp reload` is enough — the gateway does not need
restarting. Changing `gateway.auth` or `gateway.http` does need a restart, and OpenClaw's config
watcher performs it itself.

## The config file

`~/.config/yantrik/openclaw.json`, **written by you**, and optional — with no file at all this
harness runs `openclaw agent --json` with OpenClaw's own defaults, which is a working setup for
somebody who has already configured OpenClaw.

```json
{
  "route": "gateway",
  "token_env": "OPENCLAW_GATEWAY_TOKEN",
  "model": "ollama-cloud/kimi-k3",
  "path": "/home/you/.local/bin"
}
```

| key | default | what it is |
|-----|---------|------------|
| `route` | `cli` | `cli` (per-turn `openclaw agent`) or `gateway` (the streaming HTTP route). |
| `command` | `openclaw` | How to run the CLI. A string is split like a shell would; a list is taken as-is. |
| `args` | `["agent", "--json"]` | The subcommand and output flag. Replace wholesale if a future build disagrees; the rest of the flags are built around it. |
| `extra_args` | — | Anything else to pass, before the message. |
| `local` | `false` | Adds `--local`, which runs OpenClaw's embedded agent instead of going through the daemon. |
| `agent` | — | Which agent answers. `--agent` on the CLI route, `openclaw/<id>` as the model target on the other. Omitted means OpenClaw's default agent. |
| `session` | `yantrik-desktop` | The session key. `/new` moves it to `-1`, `-2`, … |
| `model` | — | Backend model override: `--model` on the CLI route, `x-openclaw-model` on the gateway route. Also the left half of the picker's `detail` line. |
| `gateway_url` | `http://127.0.0.1:18789` | Gateway route only. `ws://` and `wss://` are accepted and translated, so an older config keeps working. |
| `token` / `token_env` | — | The gateway's shared token. Prefer `token_env`. |
| `connect_attempts` | `3` | How many times to try the gateway before answering "it is not running". |
| `connect_backoff` | `0.5` | Seconds before the second try, doubling. |
| `connect_timeout` | `10` | Seconds for one connection attempt. |
| `silence_timeout` | `600` | How long OpenClaw may say nothing before the turn is failed rather than left hanging. |
| `preamble` | a short desktop prompt | Sent once per session, ahead of the first message. `""` turns it off. |
| `path` | — | Prepended to `PATH`, for a per-user `openclaw`/`node` install. |
| `env` | — | Extra environment for the CLI. |

`token_env` reads the variable **in this process**. A user service does not inherit your shell, so
put it in an `EnvironmentFile` the unit names (mode 600) or in `~/.config/environment.d/`.

`silence_timeout` is 600 seconds on purpose. It is `openclaw agent`'s own default deadline, so on
the CLI route the two give up at about the same moment and the person gets OpenClaw's reason
rather than only ours. It also has to exceed the longest *legitimate* silence, and that is not the
model: an `os_act` above this session's ceiling puts an approval card on the desktop and waits
about 270 seconds for the person to answer it, inside a single tool call, with nothing on the wire
at all. On the CLI route it is effectively the whole turn's budget, because `--json` says nothing
until the run is over.

## In the conversation

- `/stop` ends what OpenClaw is doing. On the CLI route the child is signalled, and `openclaw
  agent` sends `chat.abort` for its own run before exiting — so the work really stops. On the
  gateway route there is no abort on the chat-completions surface: the stream is closed, which
  stops you being told about a run that may finish anyway. `openclaw sessions abort` is how to
  stop that run itself. Either way the turn is closed once, and within ten seconds.
- `/new` starts a fresh session. **OpenClaw's persistent memory is not touched**: it is OpenClaw's
  own and this harness has no business in it. What `/new` forgets is the conversation, not what
  OpenClaw has learned.
- **The tool trail differs by route, because the routes differ.** On the CLI route the run's own
  `toolSummary` names what was called, so the trail reads `⚙️ yantrik-os__os_act`. On the gateway
  route there is none: OpenClaw's internal tool calls stay internal, and `delta.tool_calls` on
  that surface is for tools the *caller* supplied. The answer is the same; the trail is not.
- A message typed while OpenClaw is working gets an answer straight away saying so. It is not
  queued: the desktop is owed an answer for it.
- The first message of each session carries a short preface about this desktop — the prefixed tool
  names, start with `os_apps`, describe before acting, and **a result whose first word is
  `REFUSED` is an answer, not an error**. Set `"preamble": ""` if your agent's own instructions
  already cover it.

## What leaves the machine

The harness itself opens nothing but a loopback socket and a child process. What OpenClaw then
does is OpenClaw's:

- **Everything you type on this desktop goes wherever OpenClaw's model lives.** If that is a
  cloud provider, it leaves the machine. If it is a local model, it does not. This harness cannot
  tell which, and does not ask.
- **Every tool result OpenClaw asked for goes the same way** — the note it opened, the page it
  read, what is on your calendar, what `os_apps` says is running. That is what an agent with tools
  is, and it is the reason the MCP entry and the tool profile are both deliberate steps.
- **OpenClaw remembers.** It is attached with `memory=true` because it has persistent memory of
  its own, across sessions and across `/new`. Where that memory is stored, and whether it is on
  this machine, is a question for OpenClaw's config rather than for this one.
- **The gateway is loopback.** `http://127.0.0.1:18789` does not leave the host. If you point
  `gateway_url` somewhere else, that is a network connection and a token travelling over it, and
  `https://` is then the only sensible scheme. OpenClaw's own docs are blunt about this surface:
  a valid token on it is equivalent to an operator credential, so keep it on loopback.
- **No key is held here.** The only credential this harness can hold is the gateway's own token,
  and it goes in the `Authorization` header of one loopback request and nowhere else.

## When something goes wrong

`journalctl --user -u yantrik-openclaw -f`.

| what you see | what it means |
|---|---|
| `OpenClaw's gateway is not running — start it with openclaw daemon start` | Nothing is listening on the port. Start the daemon, or set `"local": true`. |
| `its OpenAI-compatible route is switched off` | `gateway.http.endpoints.chatCompletions.enabled` is not `true`. One config key and a gateway restart. |
| `refused this harness's credentials (401 …)` | `gateway.auth.mode` wants a token this process does not have. `openclaw gateway auth-token --show` tells you what it is; `token_env` is where to name it. |
| `openclaw exited 1 without answering: error: unknown option '--json'` | `args` does not match this build. Run `openclaw agent --help`. |
| `openclaw finished without printing an answer` | It ran and printed nothing this harness could read — a `--json` shape that changed, most likely. Run the same command by hand and look at stdout. |
| `OpenClaw said nothing for 600 seconds` | Given up on rather than left hanging. It may still be working; ask again or `/new`. |
| `dropped this answer before it was finished` | The gateway went away mid-answer. The next turn reconnects by itself. |
| `unrecognised event from OpenClaw: …` | The stream carried a shape the decoder does not know. It names the shape, which is the difference between a protocol mismatch and an agent that has gone quiet. |
| It answers, but cannot touch the desktop | `tools.profile` does not allow `bundle-mcp`, or `openclaw mcp probe` cannot reach `/opt/yantrik/bin/yos-mcp`. |
