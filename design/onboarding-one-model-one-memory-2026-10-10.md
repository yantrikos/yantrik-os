# Onboarding: one model, one memory, every mind paired (2026-10-10)

Pranab, 10 Oct:

> The desktop chat channel should be auto approved and paired automatically during installation. My vision is you set the model from our interface and the provider and then that can be injected into the config of the harnesses. So that for each one we don't need to choose models and providers. That is the seamless way. Also every one of these providers should be attached to YantrikDB memory by default, that is non negotiable.

This revises two earlier decisions:
- the 30 Sep `provider_handoff` design, an explicit per-harness "Use my Yantrik provider" button with a card;
- #675's opt-in **Use Yantrik models** button.

Both become the **default at install**, with a per-harness opt-out. Memory has been a release requirement since 6 Oct (#677) and is now non-negotiable.

## What already exists (built, not merged)

- **#675, "One model picker for every mind"** (feat/model-gateway, CI green on 6 Oct, never reviewed or merged):
  - AI Accounts: one list of providers, keys read only at the moment of use;
  - the **local model gateway**: OpenAI-compatible on `127.0.0.1:7460`, with per-harness `ygw-` tokens stored as SHA-256, `<account>/<model>` routing, effort mapping, private-context and Private-mode enforcement, and a content-free call log;
  - `provider_handoff` adapters that write each harness's own config with a gateway token, **never a key**, and `"model": "picked"`: Pi, Hermes, OpenClaw, DeepSeek, the companion and the Mind (over its memory socket);
  - the one picker `[Mind ▾] [Model ▾] [Effort ▾] [📎]`;
  - an honest "connected · model" status.
- **#677, "Ship YantrikDB as every mind's memory in the ISO"** (an issue, not started): the plugin and wheels baked in offline, Hermes installed with `memory.provider: yantrikdb` in the shared `yantrik` mode, every harness the same, the Mind in Settings → Harnesses, and a boot test that proves recall.
- **The first boot's AI step** (`onboarding.rs`, `onboarding.slint`: "Testing your AI…") and the installer's full provider selection (feedback, 2026-09).

`"model": "picked"` is the key to "seamless". A harness's config never names a model: the gateway resolves "whatever the person picked" on every call. **Changing the model in our interface changes it for every mind at once, with no config rewrite and no restart.**

## The flow

1. **Install or first boot: one AI step.**
   - The person chooses a provider (the full list, as now) and a model.
   - Private context: see Decision 1.
   - That becomes the default account and the default pick.
2. **Every harness is wired as it is installed** (by the ISO, the installer, or Settings → Harnesses → Install later), with no per-harness question. The harness installer does four things:
   1. **Model:** it registers a gateway token for the harness and writes the harness's own config with the gateway URL, the token and `model: picked`, through #675's adapter for that harness.
   2. **Memory:** it sets YantrikDB as the harness's memory provider, pointed at the shared memory (`yantrik` mode), from the plugin and wheels baked into the image (#677). It is offline and needs no TTY.
   3. **Desktop channel, paired** (next section).
   4. **Proof:** a real gateway call ("connected · <model>"), a memory write-then-recall probe ("memory · YantrikDB · recalled"), and an attach seen on the harness socket ("paired").

   A harness is never reported installed until all three proofs pass. Any that fails is shown with its one-press fix.
3. **The summary at the end of onboarding** shows one row per mind: "Hermes · connected · deepseek-v4-pro · memory: YantrikDB · paired". The rows are seen, not assumed.
4. **Afterwards:** a harness installed later is wired the same way and says so in one toast: "Hermes now uses your model and your memory · Undo". Each harness's row in Settings keeps **Use its own models** (opt-out of the model) and **Revert** (#675's restore of the person's original file).

## The desktop channel, paired automatically

"Auto-approved" means **paired with the desktop owner, and only them**: never "all users", and never a wildcard. The trust anchor is the harness socket, which already checks the kernel's peer credentials.

| Harness | What the installer does |
|---|---|
| **Hermes** | Enables the desktop platform plugin (`hermes plugins enable yantrik-desktop`). Writes the platform's allowlist to the desktop owner only (#676 / PR #696 replace `YANTRIK_ALLOW_ALL_USERS=true`). Sets `platform_toolsets.yantrik` to the desktop's tools (its README's list: no ungraded terminal, file or code route on the desktop platform). Restarts the gateway and waits for the attach. |
| **OpenClaw** | Sets `gateway.http.endpoints.chatCompletions.enabled: true`. Adds the bridge (`openclaw mcp add yantrik-os --command /opt/yantrik/bin/yos-mcp --env YOS_MCP_REQUESTER=OpenClaw`). Sets the tool profile to `minimal` + `bundle-mcp`. Puts the gateway's shared token where our harness reads it (`token_env`). No device pairing: our harness uses the HTTP route that needs none. Waits for the attach. |
| **Pi** | The extension is already passed with `-e` for desktop conversations. `pi.json` names the gateway as the provider. Waits for the attach. |
| **DeepSeek** | `deepseek.json` names the gateway endpoint, and its key env holds the gateway token. Waits for the attach. |
| **Yantrik Mind** | Listed in Settings → Harnesses (#677). Install starts its unit. The model goes over its memory socket's `POST /provider` (#675's contract, accepted by mind-72's E.PROV1). It already attaches as `mind` from its own account. |
| **Built-in companion** | Uses the default account directly. |
| **Claude Code, Codex, Gemini CLI** | Never wired silently: they are the person's own tools with their own subscriptions. One person-pressed **Give it the desktop's tools** (many-minds fix D) adds the bridge **and** YantrikDB's MCP memory server to that CLI's config. Their models stay their own (an Anthropic or OpenAI subscription is not ours to redirect). |

## One memory, many minds (non-negotiable)

- **One shared YantrikDB** for the person (`yantrik` mode), not one per harness. It is the same memory the Mind and the desktop use.
- **Every write carries its harness key as provenance** (`source: hermes`, `hermes.1`, `pi:c-…`), so a contradiction between minds surfaces as a decision ("Hermes remembers X; the Mind remembers Y"), as the many-minds design's thread-tagged memory does. The paper's pilot is why this matters: a provenance label alone did not stop models adopting a false memory, so the system reconciles conflicts instead of relying on labels.
- **Access.** Each harness reaches memory through YantrikDB's own plugin and config for that program, wherever it runs, including a terminal `hermes chat`. The host-handed `memory_credential` (memory grants, #447) stays the gate for the protocol path. For installed harnesses it is now granted by default at install. Self-attached dotted keys (`hermes.1`) still get no **host-handed** credential by default (yantrik-mind-72's rule), but the harness program's own YantrikDB config covers them. That is consistent with "every mind on YantrikDB", and the provenance tag tells them apart.
- **Private mode** still stops every harness, memory included (the harness socket refuses everything while it is on).

## Security notes

- **Keys never reach a harness.** Configs get only the gateway token; keys stay in AI Accounts and are read at the moment of a forward (#675). Moving keys saved in `providers.yaml` into the vault is still open and belongs with #538 (credentials).
- **Default-on writes into the person's harness configs happen at install**, an action the person took, so there is no per-harness card. Every write keeps the original for Revert, and the onboarding summary lists every file written (#675's card text, shown once, together).
- **Private context** (Decision 1) is enforced in the gateway, not in each harness.

## Decisions for Pranab

1. **Private context at onboarding.** With YantrikDB memory on every mind, every mind sends what it remembers about the person to the model. #675 blocks that for any cloud account until the person allows it. Recommendation: the provider step shows one plain line, pre-checked: "Your minds will send what they remember about you to <provider> when they ask it something." A local model needs no line. Unchecked, minds still work with that provider, but without memory context, and the row says so.
2. **Later installs.** Recommendation: a harness installed after onboarding is wired automatically too, with the "now uses your model and your memory · Undo" toast.

## Work

1. **#675:** rebase onto today's main, then a Claude security review (new listener, tokens, config writes). Then change the handoff from button to install-time default, keeping the button as "Use Yantrik models" for re-wiring and adding the opt-out.
2. **#677:** bake the YantrikDB plugin and wheels into the ISO; restore and upstream the plugin's `yantrik` mode; installers set the memory provider; the three proofs; the boot test.
3. **Pairing per harness** (the table above). Hermes depends on #696 (owner-only allowlist).
4. **The onboarding AI step** writes the default account and pick, and Decision 1's line. The end-of-onboarding summary shows rows seen, not assumed.
5. **Memory provenance per harness key** (with the many-minds task 22).
