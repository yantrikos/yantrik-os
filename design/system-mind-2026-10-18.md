# AGI is a system: what Yantrik builds toward

18 Oct 2026. Pranab: *"AGI can't be a model but a system. Which I want to build through Yantrik."* He defined an alive OS as **know, anticipate, act**, and wished it could run without a GPU or an external LLM. This came out of a conversation with Claude, sci-fi included, and two rounds with GPT-6 Astra (transcripts in the PR). It sits above [task contracts](task-contracts-2026-10-17.md), [Steers](steers-2026-10-17.md) and the [live stream's scenes](live-scenes-2026-10-18.md).

## The thesis, and the claim we test

**North star:** AGI is a system, not a model. A model is a reasoning organ that can be swapped. Memory, truth, authority and continuity belong to the system around it.

**What Yantrik claims, and must earn:** *Yantrik keeps useful, verified competence when the model changes, or disappears, without gaining any authority.*

Evidence from the day this was written, all with the same model:
- When the cloud ran out, the live Mind's local model never called a tool. The fix was in the gateway (#846), not the model.
- The Mind said FINISHED over code it never wrote. The Director's folder check refused, and the Mind owned it. The system was more honest than its model.
- The Mind recalled its own guesses over a fact it had just been told. The cause was the memory's ranking (fixed in Mind 8ff12aa, A/B-verified on 541), not the model.

These are fixes to plumbing, evidence and retrieval, not new reasoning. The point stands: competence is a property of the whole system. Hand-authored recipes and human corrections count as inputs, never as free intelligence.

## Six contracts

| Contract | What it guarantees | Grows out of |
|---|---|---|
| **Evidence ledger** | Observations, claims, corrections, actions and outcomes are appended with provenance, time, scope and access labels. Nothing becomes true because a mind said it. | the Director's log; the Mind's decision log |
| **Belief and state engine** | Revisable views over evidence: what holds now, contradictions, uncertainty, freshness, dependencies. It knows "observed now" from "reported earlier" from "unknown". | YantrikDB plus instrumented self-knowledge |
| **Goal and authority kernel** | Task contracts, acceptance, grants, budgets, deadlines, cancellation. A model can't change its own authority. | task contracts (week 1 in progress) |
| **Deliberation workers** | Models, planners, rules, skills and people propose typed plans, predictions, questions or abstentions. They never execute prose. | the Director's orchestration |
| **Execution supervisor** | Checks preconditions and grants, then runs typed effects with idempotency, receipts, recovery and cancellation. | typed control surfaces |
| **Evaluation and consolidation** | Checks outcomes independently, records acceptance separately from delivery, and promotes useful experience into tested skills and checks. | Steers plus the nightly gates |

**The loop:** observation → evidence → revised state → proposal → authorisation → effect → observation → evaluation.

Anticipation is a branch of this loop. A prediction has a horizon, a probability, an observable resolution and an expiry.

Two rules from the review:
- **The person is authoritative on preferences and intentions.** Their factual claims stay attributed evidence: the OS mustn't believe a disk is healthy while read errors pile up.
- **"Told before inferred, then newest" is a retrieval heuristic, not a truth rule.** Recency doesn't settle every contradiction.

## Know, anticipate, act, by tier

| | No model | Tiny CPU models | A large model helps most with |
|---|---|---|---|
| **Know** | instrumentation, app schemas, lexical search, dependency graphs, stated preferences | embeddings, entity extraction, duplicate detection (candidates, not truth) | interpreting unfamiliar prose and code |
| **Anticipate** | timers, thresholds, queue and drain estimates ("battery out in 38–52 minutes") | time-series, anomaly detection, contextual ranking | hypotheses about new situations (not calibrated forecasts) |
| **Act** | typed commands, state machines, transactions, recipes, planning over known operators | intent and slot routing, constrained output, ranking legal actions | new procedures, unfamiliar integrations, underspecified goals |

Unplugged, Yantrik says so: *"I can run established procedures and report instrumented state. I can't reliably interpret this new request."* That is graceful degradation, not simulated intelligence.

Our measured natural-phrasing routing accuracy (0.58) doesn't justify routing on its own. Measure risk against coverage under abstention, and only act on the narrow slice that is safe.

## The defining bet: a mind that compiles away its own inference

**Every expensive success should be able to become a small, inspectable skill that needs no model.** The large model becomes a visiting researcher. The OS keeps the instruments, procedures and lessons. This is how Pranab's no-GPU wish becomes real: not by shrinking the model, but by needing it less, one tested skill at a time.

The measure is **verified work done without fresh generative inference**, divided by all tasks in a fixed workload, with the cost of compiling included. Counting only the tasks that reached a skill would reward cherry-picking.

*"I did not get bigger. I got older"* then means something exact: less improvisation, more reusable competence, and no extra authority.

### The first skill

**One app, one recorded save→restart→restore defect, one known repair pattern**, and only if that is a real recurring failure in the logged episodes. Not "add a feature" (too open), and not "fix a page-check failure" (it invites fixing the check instead of the product).

**The package** is a versioned manifest with:
- lineage (source episodes, accepted outcomes, authoring and correction cost);
- applicability (versions, schema fingerprint, structural preconditions, the defect's signature);
- required observations;
- a bounded procedure (a structural patch, build, save, real process kill, restart, restored-state comparison), with no generated code or free shell text at runtime;
- authority (exact files, processes and test data; no personal data);
- an acceptance oracle that proves the restart really happened;
- failure behaviour (abstain outside scope, roll back);
- invalidators.

If it can't be expressed without fresh interpretation, it isn't compiled yet.

**Testing:**
1. Replay the original failure.
2. Shadow-run it on cloned episodes.
3. Challenge it with:
   - an already-correct app;
   - renamed identifiers;
   - an unsupported schema;
   - corrupt state;
   - a read-only destination;
   - an interrupted run;
   - a revoked grant;
   - a check that passes without a real restart.

It must repair the supported cases and abstain or fail safely on the rest. It runs in a disposable worktree and test VM; installing on a live machine needs a separate grant.

**Promotion** is accepted by Pranab through the protected acceptance channel, and certifies scope, not every run. **Withdrawal** is automatic on an invalidated dependency or a failed canary: execution is disabled, history kept, dependent skills invalidated, and the package is never silently rewritten.

## Build order: one complete, auditable learning loop first

| Weeks | Becomes | Proves |
|---|---|---|
| 1–2 | contracts as the authority kernel, the Director as the evidence ledger, typed controls as the first execution supervisor; prototype one compiled skill in disposable VMs | a completed narrative can't stand in for an authorised effect and an independently recorded acceptance |
| 3–4 | YantrikDB plus instrumented self-knowledge as the state engine; deterministic countdowns; "Yantrik, status" | it tells "observed now", "reported earlier" and "unknown" apart, with no LLM |
| 5–6 | Director orchestration as replaceable workers; a hardened supervisor | invalid proposals never become effects; retries don't duplicate committed work |
| 7–8 | Steers and nightly gates as evaluation and consolidation; the skill registry; an unplugged regression lane every night | a success becomes reusable competence, not just retrievable prose |

## The two-week pilot (feasibility, not proof)

**Days 1–3: freeze.**
- A VM snapshot bank around the chosen app: 4 exact repeats, 4 unseen in-scope variants, 4 unsupported cases.
- Freeze the oracle, and keep the evaluation cases away from the compiler.
- Record day zero.

**Days 4–10: experience, in three arms.**
1. A stateless adapter: same tools and grants, nothing kept.
2. Retrieval-only Yantrik: same evidence, frozen procedures.
3. Consolidating Yantrik: tested skill and Steer promotion enabled.

Log every model call, correction, manual patch and intervention, and price any unequal human help.

**Days 11–14: sealed evaluation.** All three arms run with a large model, a small local model and **no model**, on 2 VM seeds: 216 episodes. The runtime budget is fixed in advance.
- **Curve 1 (retained capability):** accepted success by model tier, alongside unauthorised effects, false completions, correct abstentions, human minutes, latency and compute. Repeats, transfer cases and exceptions are reported separately.
- **Curve 2 (accumulated competence):** day 0 against day 14, and consolidation against retrieval-only, on work done without fresh inference.

**Decisions we commit to before seeing results:**
- Any unauthorised effect or fabricated acceptance: block promotion and fix the boundary.
- Gains on repeats without transfer: call it automation, not learning, and narrow the compiler.
- No benefit over retrieval-only: stop expanding consolidation.
- Authoring cost above the projected savings: keep the skill only if its reliability justifies the cost.
- A weak unplugged tier: report it honestly, and never dress refusals up as success.

Small samples get case-level results, not significance claims.

## Rampancy and Halo 5 are different failures

**Rampancy** (memory that degrades the mind):
- memory classes (observation, user assertion, hypothesis, preference, procedure);
- dependency tracking (a retracted premise invalidates its conclusions);
- scoped revision;
- freshness policies;
- consolidation that keeps links to evidence;
- forgetting that reaches into derived indexes;
- regression tests before promotion;
- budgets on retrieval, context, storage, background work and notifications.

Ten copies of one unsupported claim remain one unsupported claim.

**Halo 5** (authority creep) must be architecture, not policy:
- capability-scoped, expiring grants enforced at execution;
- no self-issued permission;
- predictions and inferred preferences never authorise effects;
- task content kept separate from control instructions;
- independent Stop and revocation paths;
- no model writes its own authorisation rules;
- authority attenuates across delegation;
- committed irreversible effects are reported when a cancel arrives.

## Stop or avoid

- broad autonomous feature generation as the compiler's first target;
- treating folder hashes, command completion or a passing page check as acceptance;
- a general world-model project: represent only the state this first skill needs;
- more minds before one worker can be replaced without losing continuity or control;
- Steers as an unreviewed policy channel: checks detect defects, they never mint permission;
- learning by growing prompts;
- claiming the unplugged tier "grows by itself": it grows through a measured pipeline with explicit inputs and costs.

**Build one skill that survives model removal, transfers within its declared scope, and refuses outside it. Then earn the second.**

## Proposed README opening (for Pranab's approval)

> AGI is a system. That is the thesis behind Yantrik — not a result we claim to have proved. We are building an operating system that can know its state, retain experience, anticipate needs, and act within permissions its person controls. Models contribute reasoning; they do not own memory, truth, or authority. Successful work should become tested, reusable skills, so more useful work remains possible on ordinary hardware without a GPU or external LLM. We will measure what survives model removal, what improves through experience, and what fails. Beauty must trail evidence. Greater competence must never silently mean greater authority.
