# Pi as the desktop's mind

A harness that gives the Yantrik OS conversation to [Pi](https://www.npmjs.com/package/@earendil-works/pi-coding-agent),
driven over its RPC mode. Pi brings its own providers, keys, models and agent loop; this carries
turns between the desktop and `pi --mode rpc` and gives it the desktop's tools.

Two parts:

- `yantrik_pi.py` — the harness process. Stdlib Python, nothing to install.
- `extension/yantrik-os.ts` — a Pi extension that registers the desktop's tools by proxying to
  `yos-mcp`. Pi has no MCP client, so this is it.

## Install

The desktop can do all of this for you: **Settings → Harnesses** lists Pi whether or not it is
installed, says which of the steps below is still missing, and has an *Install* button that runs
the `npm install` line with its output on the row and a *Start* button for the unit. What follows
is the same thing by hand.

```sh
npm install -g --ignore-scripts @earendil-works/pi-coding-agent
pi --version                                    # Node 22+; Debian's Node 20 fails to start it

mkdir -p ~/.config/yantrik
$EDITOR ~/.config/yantrik/pi.json               # see below

python3 /opt/yantrik/share/harnesses/pi/yantrik_pi.py     # foreground first

cp /opt/yantrik/share/harnesses/pi/yantrik-pi.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now yantrik-pi

yos act shell use_harness id=pi                 # once it appears in the picker
```

The extension does **not** need copying anywhere: the harness passes it with `-e`, so it is
loaded for this Pi and not for every Pi you run. Copying it to `~/.pi/agent/extensions/` also
works if you want it everywhere, which on a desktop you usually do not.

## The config file

`~/.config/yantrik/pi.json`. Optional — with no file at all, `pi` is run with its own defaults,
and Pi's default provider is `google`, which is rarely what you meant.

```json
{
  "command": "/home/yantrik/.npm-global/bin/pi",
  "path": "/home/yantrik/.local/node/bin:/home/yantrik/.npm-global/bin",
  "provider": "ollama",
  "model": "ollama/deepseek-v3.1:671b"
}
```

| key | default | what it is |
|-----|---------|------------|
| `command` | `pi` | The binary, as a string (split like a shell would) or a list. |
| `path` | — | Prepended to `PATH`. A user service does not get your login shell's PATH, and `pi` and `node` are usually in `~/.npm-global/bin` and `~/.local/node/bin`. |
| `env` | — | Extra environment for the Pi process. |
| `provider`, `model` | Pi's own | Passed as `--provider` / `--model`. `--model` takes `provider/id`. |
| `extension` | the file beside this README | Passed as `-e`. Empty string to load none. |
| `yos_mcp` | `/opt/yantrik/bin/yos-mcp` | Where the extension finds the bridge, passed to Pi as `YOS_MCP_BIN`. Only needed for a checkout that is not installed. |
| `builtin_tools` | `false` | See below. |
| `extra_args` | `[]` | Anything else to put on Pi's command line. |
| `append_system_prompt` | a short desktop prompt | What Pi is told about where it is running. |
| `silence_timeout` | `420` | Seconds of total silence from Pi before the turn is failed rather than left hanging. |

Provider keys are Pi's own business: they live in `~/.pi/agent`, this harness never reads them,
and the desktop has nowhere to put one. A custom OpenAI-compatible provider goes in
`~/.pi/agent/models.json`, where `apiKey` can reference an environment variable rather than
holding a key:

```json
{
  "providers": {
    "ollama": {
      "baseUrl": "https://ollama.com/v1",
      "api": "openai-completions",
      "apiKey": "$OLLAMA_API_KEY"
    }
  }
}
```

## Pi's own tools are off, and that is a decision

Pi arrives with `bash`, `read`, `write` and `edit`. On this desktop they are a **second,
ungraded route to everything the apps already offer**: a file written through Pi's `write` is
a file written with no app, no grade, no card and nothing on screen, while the same edit through
the Files or Notes app carries the app's permission grade, obeys the person's mind mode, and is
visible while it happens. This is the same lesson `docs/harness.md` records for Hermes, which
arrived with its own `terminal` and `file` toolset and spent most of its first long job waiting
on its own approval prompts instead of doing the work through the desktop.

So `--no-builtin-tools` is passed unless you set `"builtin_tools": true`. That is your call:
for coding work in a checkout, Pi's own tools are the whole point, and turning them on is one
line. Just know which surface you have turned on, because only one of the two is graded.

`bash` comes back anyway, graded. With Pi's own tools off and the bridge offering the agent's
terminal (it does when Pi runs as one of the person's agents), the extension registers a `bash`
with Pi's exact parameters on top of the bridge's `run_command`: the model keeps the tool it
was trained on, and each command runs in the agent's own terminal in its pane on the desktop —
asked about in `ask` mode, watched, answerable and stoppable there — rather than as an unseen
child of Pi. With Pi's own tools on, the extension leaves Pi's `bash` alone.

## The extension

`extension/yantrik-os.ts` asks `yos-mcp` for its tool list at load and registers each one with
the description and JSON schema the bridge published, then proxies every call straight through.
It decides nothing: modes, permission grades, approval cards and the taint rule all live in the
bridge, and an extension that re-checked any of them would be a second policy to keep in sync
with the one the machine actually enforces.

`isError` comes back exactly as the bridge set it. In particular a `REFUSED` answer arrives
**unflagged**, because the desktop ran, was healthy, and said no — flagging it teaches a model to
retry a refusal, and clients that count errors have switched the whole desktop off over three of
them.

It is dependency-free (`typebox` and node built-ins, both of which Pi resolves for an extension)
and the `tools/list` at load is done synchronously, so registration is complete before the
extension's factory returns whether or not Pi awaits an async one. If the bridge cannot be
reached at all, a small fallback table keeps the tools existing so Pi still starts.

## In the conversation

- `/stop` sends Pi's own `abort`, so the tool it is in the middle of stops too.
- `/new` sends `new_session`: Pi keeps the conversation, so Pi is the one that has to forget it.
- Each conversation the desktop starts with Pi (each agent in the Agents view, and the Lens's
  own `main`) is its own `pi --mode rpc` process, started when its first message arrives and
  stopped when the agent is stopped. Its environment carries that agent's
  `YANTRIK_AGENT_TOKEN`, so the `yos-mcp` the extension starts can say which agent is asking.
- Each process runs in the conversation's own directory —
  `~/.local/share/yantrik/minds/pi/<conversation>` (under `XDG_DATA_HOME` when that is set) —
  and never in your home folder, because Pi reads the instruction files (`CLAUDE.md`,
  `AGENTS.md`) of its working directory and its parents: your own `~/CLAUDE.md`, written for
  your coding work, must not steer the desktop's mind (#183). Instructions for the desktop's
  Pi belong in that directory, on purpose.
- A command of the agent's that finishes after its call returned is told to Pi at the start of
  the next turn, in front of your message (`[From the desktop, since your last turn: …]`).
- A tool execution is a card in the agent's pane — its arguments, its output streamed into it,
  ✓ or ✗ when it ends — and still the trail line `⚙️ os_act calendar.add_event` in the text.
- Pi's thinking goes beside the answer as a folded "thinking" line in the pane, never into the
  answer itself; what each model call cost goes to the agent's details.
- If Pi opens a dialog of its own (`confirm`, `select`, `input`), it is **declined** and the
  question is repeated in the conversation. This desktop asks for permission with its own card,
  which you see and answer and which the machine records; a harness that clicked "yes" for you
  would be a second approval path nobody can see.
- If Pi exits mid-answer, the turn is failed with a sentence and Pi is started again for the
  next message. If Pi says nothing at all for `silence_timeout` seconds, the turn is failed
  rather than left open — the default is 420s because an `os_act` can legitimately wait about
  270s for you to answer a card, with no events in between.

## When something goes wrong

`journalctl --user -u yantrik-pi -f`. The two usual causes are `pi` or `node` not being on the
service's `PATH` (set `path` in the config) and Node being too old — Pi needs 22+, and Debian
13's Node 20 fails with `enableCompileCache`.
