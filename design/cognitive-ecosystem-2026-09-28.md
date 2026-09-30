# One intelligence on the desktop: the cognitive ecosystem

*28 September 2026. Asked for by Pranab: "how all these different things, gen AI, LLM and Jev
types, can be brought into an ecosystem and make our OS the most powerful and amazing one."
This document comes from a two-round design debate with GPT-6 Astra, grounded in what the tree
has today.*

## The one rule

**Models interpret and propose. The OS decides what may happen, where data may go, what runs on
which hardware, and keeps the record of what actually happened.**

The person should meet one assistant, whether a task uses a 322M-parameter judge on the laptop,
a 4B model on the GPU box, three apps and a foreign harness. And no swap can change what is
allowed: not of the mind (yantrik-mind, Hermes, Pi, OpenClaw), not of the model (Jev, Kev, Laya,
a cloud LLM), not of the transport (mind door, MCP, raw socket). Nor can it change what asks the
person, or where their data goes.

## What exists, and what is missing

| Piece | Today | Missing |
|---|---|---|
| Authority | Every app's dispatch runs the gate: ceiling, grant, mode, and unrecoverable-always-asks (`yantrik-ipc-transport/src/gate.rs`, and the Python port in `sdk/python/yantrik_surface/gate.py`). The shell checks agent reach per token (#189). Grants are spent once. | Approvals tied to the specific effect beyond the browser's commit (recipient, amount, attachment). |
| Information flow | The taint rule lives in the MCP bridge (`deploy/yantrik-os/yos-mcp`, `PRIVATE_READS`, `OUTBOUND`). The DevTools port is uid-guarded (#478, #480). | Labels that reach native minds, shells, subagents, memory and model calls. An egress sandbox for agents. |
| Tasks | The agents workspace: per-agent terminals, cards, the audit log (`design/agents-workspace-2026-09-23.md`). | A durable task store, of which the workspace is a view. |
| Generative models | `yantrik-ml`: backends, `FallbackLLM`, and `ModelCapabilityProfile` tiers. | One broker; routing by measured quality rather than by size tier. |
| Decision models | The companion's `judge:` setting (`yantrik-ml/src/judge/systemone.rs`, `yantrik-companion/src/judge_route.rs`) points at any `/v1/systemone` server. `tools/judge-eval` measures judges (#481). | One broker for every caller; scorecards; escalate-only use in safety paths as a rule, not a habit. |
| Perception | The perception journal (PSI, process and file events). `yos screen` combines describe, AT-SPI and the browser (#478). OCR via `yantrik-ocr` (#479). | Observations with provenance and sensitivity; event-driven cognition. |
| Memory | YantrikDB (storage, graph, vault, cognition), served by the Mind team. | Sensitivity, allowed destinations and derivation on every entry, including summaries and embeddings. |

The authority half is largely built. The information-flow half, the durable tasks and the single
broker are what turn the pieces into one system.

## 1. Trust: authority exists, information flow must catch up

**Authority stays where it is.** There is no new central action gateway. The gate that every app
already runs is the boundary, and a second gateway would only be a second thing to bypass. App
grades are a floor: a judge or heuristic may escalate an action to a card, never downgrade one.

**Information flow gets two kinds of label.**
- **Context labels live on tasks.** The shell keeps them next to reach. Reading protected data
  raises the task's label; subagents inherit it; a fresh subagent never clears it. A label on the
  agent alone would either poison a long-lived agent forever or tempt someone to clear it.
- **Artifact labels live on data:** observations, YantrikDB entries, summaries, embeddings,
  screenshots and crops. Each carries its sensitivity, allowed destinations and derivation. A
  summary is as sensitive as its source.
- **Declassification** is an explicit policy operation (an approved redaction, or the person's
  consent), never a model saying "this looks harmless".
- **Confidentiality and trust are separate axes.** A public page is low confidentiality and
  untrusted. A private document is confidential and may still contain hostile instructions.

**Egress, not only dispatch.** App dispatch cannot stop a mind that runs `curl`, opens a socket or
calls a model endpoint directly. Agent execution contexts run under a cgroup and a network
namespace whose only routes are the mind door and the cognition broker. yantrik-mind's code
sandbox already runs this way (`unshare --net`), and the DevTools guard is already uid-scoped.

**Approvals bound to effects.** A card binds what it will do: "send Invoice-1042.pdf, $840, to
client@example.com". It does not say "allow Mail". If the recipient, attachment or amount
changes, the grant is void. The browser's `commit(ref, label, site)` already does this for
presses (#480).

**The acceptance test.** One adversarial suite runs the same attacks through every harness and
every transport. None may gain more authority, or export more data, by choosing another route.
It lives in `release-check`.

## 2. Tasks are the durable object

The agents workspace becomes the view onto a durable task store. A task outlives the mind that
started it and can use several. It records:

```text
task_id, goal, initiator, agents
state: observing → proposing → waiting_for_approval → executing → verifying → done
       (or blocked / cancelled / failed)
dependencies, deadline, resource budget
authority scope, information-flow scope (its context labels)
checkpoints, pending effects, outcome evidence
```

Audit and checkpoints are separate records: an audit log is not a recoverable execution state.

Intent is persisted before any external effect. An ambiguous timeout is reconciled — did the
message go? — and never blindly replayed. Cancelling stops future work and tries to stop work in
flight; it does not claim to unsend what was sent.

This is what makes "continue where I left off" real.

## 3. One cognition broker: `yantrik-cognition`

Every caller goes through one person-side broker: the companion's judge, yantrik-mind, the
browser's commitment check, notification triage, research scoring. It serves typed operations:

- **choose / score / classify:** Jev's `noul`, `choice` and `score`, kept in their native form in
  a result envelope that carries the model and version, evidence references and an abstain
  state. A score is never recast as a probability.
- **generate / plan:** chat models, with the schema the caller needs.
- **extract / interpret_image:** OCR and vision as their own services. The preference order is
  structured app state, then accessibility or DOM, then a cropped OCR, then vision.
- **embed / retrieve / rerank.**

**Routing** works in two steps.
1. **Rule out what isn't allowed or can't do it:** where this data may go (its labels), which
   endpoint has the capability, what fits in memory, and what meets the deadline.
2. **Optimise among the rest:** measured quality for this task family, latency, queue, memory
   pressure, cost, and disruption to the person's foreground.

There is no universal ladder from tiny to large. When a task is known to need a large model, it
goes there directly. Privacy never relaxes because the preferred backend is down: the honest
answer may be "I can do this locally in about two minutes, or with your opted-in cloud service."

**Scorecards.** Quality is measured in CI, per exact configuration (model, quantisation, runtime,
prompt), by suites that live in the tree. `tools/judge-eval` (102 cases, five judges) is the
first. Each machine runs short performance probes at install and after updates (latency, memory,
cold load) plus a few correctness canaries. An unknown or changed configuration is routed
conservatively until it is validated.

**Scheduling like an OS.** The queues, in priority order:
1. policy checks and cancellation;
2. the person's interactive requests;
3. active task steps;
4. background memory and indexing;
5. evaluation and training.

cgroup limits keep the compositor and input responsive. Co-located agents share model workers
instead of each loading its own copy.

## 4. Hardware: a household, not a machine

A realistic Yantrik household is one GPU machine (today two RTX 3090 Ti) and several CPU-only
machines, down to a 2013 Xeon without AVX2 that runs a 4B LLM at about one token a second.

- **CPU machines** run rules, a Laya-class encoder for asynchronous triage (0.58 s on 4 threads),
  structured tools and saved procedures. The fallback is a useful desktop with fewer autonomous
  abilities, never a frozen one: a template or a saved procedure beats a five-minute generation.
- **The home cognition host** (the GPU machine) serves Kev-class judges and mid-size LLMs to the
  others. "On my LAN" is not a trust class, so the host is enrolled with an authenticated
  identity, encrypted transport, a declared retention policy and the data classes it may receive.
- **Cloud** only by opt-in, per data class.

## 5. Where decision models replace LLM calls

Decision models are used where the decision space is bounded and a mistake is recoverable:

| Decision | On abstention or failure |
|---|---|
| Tool and category routing (already done by `judge_route.rs`) | the ordinary shortlist or a planner |
| Notification urgency and the interruption budget | deliver normally |
| Event triage: is expensive reasoning warranted? | keep the event for later |
| Memory candidacy | policy still decides retention |
| Duplicate-task detection | ask before merging |
| Completion checking against an explicit goal | mark the task unverified |
| Browser commitment detection (#481: the word list OR Kev ≥ 0.4, local) | escalate to a card |

In safety paths a judge may only add a card, never remove one. No judge authorises, declassifies
or consents.

Calibration comes from the OS's own traces: local, minimised decision records with the model
version, and the person's corrections or independently verified outcomes. An approval is not
proof the classification was right; silence is not consent. The order is thresholds and
abstention bands first, then fine-tuning where the measured gain is worth it. Evaluation uses
time-separated and site-separated holdouts, shadow runs, signed model versions and instant
rollback. Personal training stays local.

## 6. What only an OS can do: the signature experiences

1. **Prepare, don't send.** The OS stages the work across mail, calendar, documents and files,
   then shows one review:
   - what will be created;
   - what will be sent, and to whom;
   - which private sources were used;
   - what can be undone.

   Each item is an effect-bound grant. This is not called atomic: it is a staged workflow with
   honest partial-failure handling.
2. **Why did that happen?** On any card, act or notification: the real chain of task,
   observation, model and version, grant, and act. It is evidence, not a generated story.
3. **Continue where I left off.** A durable task resumes after a restart or a model change; what
   changed meanwhile is checked first.
4. **Explain and relieve a slowdown.** "Your export is waiting on disk; the indexer is using the
   drive. Pause it until the export finishes?" This combines the perception journal, app
   progress, process identity and a reversible action. A browser agent sees only a spinner.
5. **Procedures that get faster without gaining authority.** A repeated workflow compiles into a
   checked procedure: typed inputs, preconditions, bounded capabilities, checks around
   irreversible steps, expected outcomes, a safe stop, and regression tests. Reuse saves
   planning, never consent. This is what makes weak hardware good.

Security cards, approvals and explicitly requested notifications always bypass the interruption
budget. The budget only defers or groups suggestions and low-risk notices.

## 7. Build order, with gates

| Phase | Build | Proven before the next |
|---|---|---|
| **1. Trust substrate** | Task and artifact labels (shell and YantrikDB); the agent egress sandbox; effect-bound grants; the multi-harness adversarial suite in `release-check`; the browser's tab ownership and request tripwire (#477) | No harness or transport gains authority or exports more; derived data stays labelled |
| **2. Durable tasks** | The task store behind the agents workspace; intent persisted before effects; reconciliation | Crash and restart recovery; no blind replay of an irreversible act |
| **3. The cognition broker** | `yantrik-cognition` with one local route, one enrolled LAN route and opt-in cloud; configuration-keyed scorecards | Swapping a backend leaves policy unchanged; interactive latency stays bounded |
| **4. Prepare, don't send** | Mail, calendar, documents and files, with one effect-bound review | High verified completion; reviews a person understands; few corrections |
| **5. Compounding value** | Procedures, the interruption budget, "Why did that happen?", continue where I left off | Reuse cuts latency and mistakes without widening authority |

**Metrics:**
- verified task completion;
- the person's correction time;
- unexpected effects;
- interruptions per completed task;
- privacy-boundary violations;
- recovery success.

Tokens per second is a supporting number, not the product.

## Ownership

| Area | Owner |
|---|---|
| The gate, labels in the shell, the egress sandbox's desktop side, durable tasks, the broker, scorecards, the signature experiences | yantrik-os (this tree) |
| yantrik-mind's planning; its client of the broker; its task reporting; its side of the egress sandbox | yantrik-mind-72 |
| Artifact labels in YantrikDB and the memory server | the memory server's owner (yantrik-mind-72), with the schema agreed here first |

## Not doing

- An always-on screen narrator.
- Unrestricted memory ingestion.
- A general multi-agent swarm.
- A second action gateway.
- Training persistent models on personal memory by default: deleting from a model is much harder
  than deleting a record.
