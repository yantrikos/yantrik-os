# Many minds, one witness: the Lens when minds start anywhere (2026-10-10)

People will start minds from everywhere. Pranab types `hermes chat` in a terminal, launches an agent from the Agents screen, a timer starts a nightly job, and someone ssh-es in and runs Claude Code. Each runs independently. Today the Lens (the chat bar) sees only one of these: the active mind's `main` conversation. This doc says what the Lens should show and what the OS has to build underneath for that to be true.

The mockups are in the "Many Minds" artifact. The first concept, "the chat bar as a switchboard", was critiqued blind by DeepSeek V4 Pro, GLM 5.3 and Kimi K3 (`F:/yantrik/qwen/lead-work/switchboard-critiques.json`). All three said the same thing in different words: *the chat bar is a view, not the architecture.* This revision starts from that.

## The idea

**The OS is the witness.** Every action any mind takes on the desktop goes through `app.act`, and the kernel says which process made the call. No other system on the market has that position: a chat product knows only what its own agent says it did. So the Lens shows two kinds of statements, and never mixes them up:

- **Seen**: the OS recorded it. "10:42 · moved 3 files in Downloads". The act happened; that part is certain.
- **Says**: a mind reported it. "Drafting the reply", "Finished the refactor". The mind may be wrong or lying.

The agents store already carries this distinction (`Provenance Reported|Verified`, `crates/yantrik-ui/src/agents/model.rs:164`); this design makes it the Lens's visual grammar. It also matches what the provenance pilot measured: labels a model can read barely change what it believes. People need the distinction even more than models do, and the OS can supply it honestly.

## What the OS can and cannot know

This is the part the first concept got wrong. It assumed the OS sees sessions. It sees acts.

| | What the OS knows | How sure |
|---|---|---|
| An act happened (app, action, outcome) | Always: the app ran it | Certain |
| Which process tree called | `SO_PEERCRED` + the `/proc` walk (`yantrik-ipc-transport/src/peer_identity.rs`) | Certain at the moment of the call |
| Which **mind** it was | Verified only for an attached harness (agent token + pid descent, `host.rs:400-416`) or the `yantrik-mind` account. Otherwise only *recognised* by program name, which a program running as the person can imitate | Verified / recognised |
| Where it was started (terminal, ssh, timer, desktop) | Derivable from the chain and the cgroup; nothing reads this today | Good, not proof |
| What it is *doing* or *about to do* | Only what an attached harness reports (`status`, `tool_start`) | Says, never seen |
| A session that has not acted yet | Nothing, unless it attached | Invisible |

So: a Hermes started by hand in a terminal is, to the OS today, **the person** (`Caller::NoAgent`, `control_agents.rs:94-108`). The design must not pretend otherwise. It can show "Hermes (recognised, not verified) · from a terminal" because the chain says so, and it must keep that qualifier.

## The Lens, revised

### 1. At the machine (a strip, not a rail)

Under the Lens header, one collapsed line: **"3 minds at the machine · 1 needs you"**. Expanded, one 44px row per mind that is attached *or has acted in the last 15 minutes*. Nothing else: a silent process is not shown (the critique's "the rail becomes noise").

Each row:
- the mind's name; for a recognised-not-verified caller, the name in the secondary colour with "recognised" after it;
- where it started, as plain metadata text ("terminal", "ssh from 192.168.4.12", "timer · nightly-sync", "desktop"), never an icon that becomes its identity;
- its state, in the one vocabulary from `minds-surfaces-spec-2026-10-02.md` (Working · Needs you · Paused · Finished · …) **only when an attached harness reports it**. For anything else: "Last seen 10:42 · moved 3 files". Never an inferred "Working".
- Tap: **Look in** (below). No hover-only information.

### 2. One approvals inbox

Pending approvals already appear in the Lens from any caller (`control_approvals.rs:1172-1182`). Make them the top of the Lens whenever any exist, oldest first, each card carrying the verified line ("Hermes · from a terminal · recognised, not verified") that `approvals::Verified` already computes, plus the origin. One place to answer every mind, wherever it was started. The existing card rules stand: Approve once / Decline, no approve-all, the approval binds to the action as displayed.

### 3. Ask the machine, instead of @all

The person asks the home mind in plain words: "Who's touching my Documents?", "What did anything do while I was out?", "Which mind changed index.html?". The home mind answers **from the ledger**, by a read-only query, with each line marked seen and a Look in link. Nothing is broadcast to other minds: the OS answers faster and more truthfully than they can, and a broadcast is a prompt-injection path into every session at once (Kimi).

### 4. Talk to a mind by what it is doing, not by a handle

No `@hermes-terminal-2`. To send to a specific mind, the person either taps **Talk to it** on its row (which puts a target chip in the composer: "To Hermes ×"), or says it: "tell the one editing index.html to stop". The home mind resolves "the one editing index.html" through the ledger to one agent and shows the chip for the person to confirm. A send never changes the active mind (today `chat_with` swaps it, `wire/agents.rs:344-359`) and never re-targets an existing draft.

Only minds attached to the OS can be talked to; that is a protocol fact, not a policy. For the rest, the chip offers what the OS can do instead (see Hold, phase 2).

### 5. Look in (read-only, one rule)

The critiques were right that "sometimes you can send, sometimes it summarises" makes people unsure who they are talking to. One rule: **Look in shows what the OS knows about that mind, and nothing it doesn't.**
- An attached mind with a desktop conversation: its transcript (from the agents store), read-only, with the work card and **Talk to it** at the bottom.
- Anything else: its ledger, the acts it was seen making, newest first, with the banner "Started in a terminal. The OS can show what it did, not what it is thinking." and the one line to attach it (`yantrik attach hermes`, once that exists).

### 6. Declared scopes and holds (phase 2)

Leases that block are out: they deadlock, and they assume the OS knows what a mind is about to touch, which it doesn't. Instead:
- An attaching harness **declares its scope** (apps, folders, network), like a mobile permission sheet. The OS records it and shows it on the row.
- Overlapping scopes between two live minds produce a **forecast**, not a lock: "Hermes and Claude Code can both write to ~/Projects/site. Hermes edited index.html 3 times in the last 5 minutes." With Let it be / Hold Hermes.
- **Hold** is a capability the person grants and revokes: "Hold Hermes from Downloads" makes `app.act` refuse that mind's acts in that scope with `HELD:` until released. It works on unattached callers too, keyed on the recognised program chain, with that qualifier shown.
- Undeclared minds show "No scope declared".

Any harness-protocol change here (`harness.attach` gaining `scope`) goes to yantrik-mind-72 before it reaches 520.

### 7. Time travel (phase 3)

Because the ledger is append-only and ordered, the Lens can show the machine as of a moment: "At 10:42: Hermes had moved 3 files; Claude Code had opened index.html." Undo, where an app's act has an inverse, hangs off the same rows. Both depend on phase 1 only.

## What has to be built

### The ledger (#148, never built)

One append-only record of every `app.act`, written by the OS, not reported by `yos-mcp`.

- **Where it is written.** At the one seam every act already passes: `ActCall::log` (`crates/yantrik-surface/src/call.rs:197-211`), called from `ControlRpc::dispatch` (`crates/yantrik-app-runtime/src/control.rs`) before the handler, and once more with the outcome after it. The app runtime sends the entry to the ledger service; it never writes a file itself.
- **Who keeps it.** A new supervised process, `services/action-ledger`, built on `yantrik-service-sdk` like `services/perception-journal` and for the same reason that doc gives: the shell is the most crash-prone thing on the machine, and a record must not depend on its uptime. Reuse perception-journal's segment-and-cursor journal (`services/perception-journal/src/journal.rs`); do not write a second one.
- **An entry.** `seq`, `at`, the reporting app, `action`, a short args summary (the same one an approval card shows; never raw file contents, never secrets: anything an action marks sensitive is reduced to its kind), grade, `granted`, outcome (`ok` / `refused:<kind>` / `stale` / `held`), `action_id`, revision, and the **actor**: `{agent?, harness?, verified: bool, program, chain_root_pid, started, origin, uid}`.
- **Trust.** Entries are hash-chained (each carries the hash of the one before), so an edit or a deletion shows. The service accepts `ledger.record` only from the desktop's own programs, using the same check every service's raw methods already use (`yantrik_ipc_transport::owner::desktop_programs_only`), and it records which reporter sent each entry. Stated plainly in the doc comment and the UI: tamper-evident, and hard to forge, but not proof against a program running as the person that is set on it (the same limit `peer_identity.rs` states). The `yantrik-mind` account cannot touch it at all.
- **Readers.** The service's raw `ledger.since {seq|time, actor?, app?, path?}` is also for the desktop's own programs only (the shell and the built-in mind's worker). Everyone else reads through a graded, read-only action on the shell's `app.act`, which filters by caller. The person sees everything; any other mind sees only its own entries. The ledger is a map of what every other mind did, so handing it to a third-party agent would be a leak. `yos ledger` (a python script, which the raw check refuses) goes through that action too.
- **Replaces** the mind audit (`mind-audit.jsonl`, `mind_mode.rs:1276-1340`) as the source for "Recent actions"; the mind audit's readers move over in the phase that builds the Lens strip.

### Origin

A pure classifier in `yantrik-ipc-transport` beside `peer_identity`, from the chain and `/proc/<pid>/cgroup`, `/proc/<pid>/stat` (tty), and the chain's comms:
`desktop` (a scope under the graphical session, or started by the shell) · `terminal` (has a tty, under a terminal emulator) · `ssh` (an `sshd` ancestor; with the peer address when `SSH_CONNECTION` is readable in the root's environ, else without) · `timer` (a `.service` unit started by a `.timer`, named) · `service` (another systemd unit, named) · `unknown`. Never a guess: `unknown` when the facts don't say.

### Sessions from the ledger

The Lens strip's rows come from two sources merged: `Host::agents()` (attached, with reported state) and the ledger's distinct actors in the last 15 minutes (keyed on agent id when verified, else on `program + chain_root_pid + started`). That is a read model in the shell, not a new store.

## Phases

**Phase 1 (foundation, no visible change except Recent actions):**
1. `action-ledger` service: hash-chained journal, `ledger.record` (reporter-checked), `ledger.since`, `ledger.status`, retention.
2. The app runtime records every act (pre and post) through `ActCall::log`'s seam, with the actor resolved at call time.
3. The origin classifier, with fixture `/proc` trees in tests.
4. `yos ledger` (read-only) and the mind panel's Recent actions reading the ledger.

**Phase 2 (the Lens):** the At the machine strip, the approvals inbox at the top with origin, Ask the machine (the built-in mind's ledger tool), target chips and Talk to it, Look in. Then declared scopes, forecasts and holds (protocol change: mind-72 first).

**Phase 3:** time travel, undo where an inverse exists.

## Decisions that are Pranab's

1. **Retention.** Proposed: 90 days, then segments are dropped. A ledger is a record of the minds, but it also records the person's own `yos` use, because an unattached Hermes looks exactly like the person.
2. **Private mode.** Proposed: nothing is recorded while private mode is on, except a count of refused attempts per actor ("Hermes tried 4 times"). Acts are already refused then, so there is little to record; the count is a security signal.
3. **Erase.** Proposed: "Erase my data" includes the ledger (today's erase code knows `runs.db`).
4. **Whether recognised-not-verified minds may be held** (phase 2): holding by program name can be dodged by renaming, so a hold on an unverified caller is a speed bump, and the UI says so.

## Left out, on purpose

`@all`; @-handles; a live hybrid tap-in; origin glyphs as identity; blocking leases; inferred states; the Lens taking over a mind started elsewhere (a terminal session stays the terminal's).
