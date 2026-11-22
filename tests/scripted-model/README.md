# Scripted test model

**For test VMs only.** A deterministic stand-in for a model, so machine tests and the core-flow
acceptance run pass or fail on the OS and never on a free model's quota (#1362). It answers from
scenario files. It holds no key, checks none, makes no outbound connection, and binds `127.0.0.1`.
Python standard library only.

## The APIs it speaks, and which OS path uses which

| Path to a model | API it uses | Served here |
|---|---|---|
| Companion, OpenAI-compatible and custom providers (`yantrik-ml` `llm/api.rs`) | `GET /v1/models`, `POST /v1/chat/completions` (SSE, `tool_calls`) | yes |
| Companion, Ollama-type provider (`llm/api.rs` native path; Settings probes `/api/tags`) | `GET /api/tags`, `POST /api/chat` (NDJSON, `message.tool_calls`) | yes |
| Companion, Anthropic account (`provider/anthropic.rs`) | `GET /v1/models`, `POST /v1/messages` (SSE, `tool_use`; `x-api-key` ignored) | yes |
| "Yantrik models" gateway (`yantrik-gateway`, 127.0.0.1:7460) | its own front is OpenAI-style (`/v1/models`, `/v1/chat/completions`); upstream it calls an account's API, so the scripted model is added as an account of type custom/OpenAI | yes, as the upstream |
| Mind harnesses Hermes, DeepSeek, OpenClaw | OpenAI chat completions (`/v1/models`, `/v1/chat/completions`) against the gateway or a base URL | yes |
| Mind harness Pi | a provider and model chosen in Pi's own config; OpenAI-compatible base URL | yes |
| Claude Code (`claude_cli.rs`) | the `claude` CLI's own Anthropic Messages calls; honours `ANTHROPIC_BASE_URL` | yes, `/v1/messages`; **not run against a real `claude` CLI** |

Not served: `/api/generate` (only the live gate's AIG lane mentions it). Both `/v1/x` and `/x` work.

## Scenarios

A scenario file is JSON, see `sm_scenario.py`:

```json
{"rules": [{"name": "greet", "match": {"contains": "hello"},
            "script": [{"text": "Looking."},
                       {"text": "Running.", "tool_calls": [{"name": "run_command", "arguments": {"command": "uname -a"}}], "delay_s": 2},
                       {"text": "After the result."}]}],
 "default": {"text": "optional replacement for the fixed sentence"}}
```

Rules are tried in order against the **last user message**; `match` takes `contains`, a substring. There is no regex (a pattern could backtrack for minutes and pin the server).
The step is the number of assistant messages after that user message in the history the client sends,
so a conversation's position is in its own request, and two conversations never interfere. Past the end
of a script it answers one closing sentence. With no rule matched it answers
`This is the scripted test model's fixed answer.`

Shipped (all loaded by default; ask for one by putting its name in the message):

- `markdown-all` – one reply with every element the Lens renders (#1349), including a `javascript:` link that must not open.
- `commands-20` – 20 `run_command` calls (the agent bridge's tool; argument `command`), then a summary (#1357).
- `slow-task` – seven steps, a tool call about every 4 s (#1355). `--delay-scale 0` removes the waits.

## Running it

By hand: `./scripted_model.py --port 7471 --deadline 3600` (`--scenario my.json` or `--scenario slow-task`, repeatable;
`--log FILE` also appends the log lines). `--deadline` stops it after that many seconds; always give one on a shared VM.

As a user service with a deadline (no file of the person's is edited):

```sh
systemd-run --user --unit=scripted-model --property=RuntimeMaxSec=3600 \
  /path/to/tests/scripted-model/scripted_model.py --port 7471
```

Each request is one log line on stderr (`journalctl --user -u scripted-model`): time, API, path, stream flag, rule matched, step.
Request content is never logged.

## Adding it as a provider

Through the OS's own Settings, never by editing config files: Settings → AI → add a provider of type
**Custom / OpenAI-compatible**, base URL `http://127.0.0.1:7471/v1`, any key text, model `scripted-model`
(for an Ollama-type provider use `http://127.0.0.1:7471`). Hermes, DeepSeek and the other harnesses are pointed
at it by their own Settings → Minds rows, or through the gateway by adding it as an account.
I found no agent-drivable (`yos act`) action that adds a provider, so none is documented here.

## Tests

`python3 -m unittest discover -s tests/scripted-model -v` (also collected by `pytest tests/scripted-model`). CI runs the first form.
