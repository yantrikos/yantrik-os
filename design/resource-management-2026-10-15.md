# Resource management: the person's desktop is protected, minds and agents yield

15 Oct 2026. Pranab asked for "a smart solution on optimizing the OS running" after the live
machine went offline. This is the plan, after a review by GPT-6 Astra
(`resource-management-astra-review-2026-10-15.md`, kept verbatim beside this file).

## Why now

On 15 Oct at 21:09 CT the live machine (VM 561, 6 vCPU, 12 GB) stopped streaming, and its QEMU guest
agent stopped answering and did not come back. The desktop stayed alive: its clock ticked, 4.4 GB was
free, no swap was in use, and the VM sat at 150 to 190% CPU. It started three minutes after a
recorder began, as the person's account:
- `yos describe shell` every 0.4 s;
- `grim` every 3 s;
- my guest-agent polls every 3 s, while the Mind was mid-turn.

The stream came back the moment the recorder exited, at its 20-minute limit. The guest agent stayed
dead.

That links the recorder to the outage; it does not prove how. 150 to 190% of six CPUs is not
saturation. The guest agent may have been wedged by the polling itself (piled-up `guest-exec`
calls), not by CPU. Step 0 below reproduces it with probes before anything is built on the guess.

## What the machine does today (measured on VM 520, OS f063fcef, 15 Oct)

- **cgroup v2, unified.** Controllers: cpu, cpuset, io, memory, hugetlb, pids, rdma, misc. Delegated
  under `user.slice`: cpu, memory, pids, not io.
- **PSI.** `CONFIG_PSI=y`, not default-disabled; `/proc/pressure/{cpu,io,memory}` are there.
- **systemd-oomd** is not installed. There is no swap and no zram.
- **Block scheduler** `none` on the main disk, so `IOWeight=` does nothing until io.cost or BFQ is
  set up.
- **Placement.** The cgroup each part runs in:

  | Process | cgroup |
  |---|---|
  | labwc, swaybg, yantrik-ui, and so everything the shell starts, agent_run jobs included | `user-1000.slice/session-1.scope` (the tty1 login session) |
  | Yantrik Mind and its yos-mcp bridge | `system.slice/yantrik-mind.service` |
  | Hermes and its bridge | `user@1000.service/app.slice/hermes-gateway.service` |
  | the egress proxy | `system.slice/yantrik-egress.service` |
  | the guest agent | `system.slice/qemu-guest-agent.service` |

- **Limits in the product: none.** No slice, weight, memory bound, task limit or OOM policy on the
  shell, the apps, the minds, the harnesses or agent work. The only limits anywhere are on the live
  publisher (`deploy/live/publisher`): `Nice=10`, `CPUQuota=100%` and `MemoryMax=400M`. That is the
  one thing on the live machine that must not starve, set to give way first.
- **The cost of `describe shell`.** Five calls on 520, about 95 KB each:
  - 270 to 370 ms wall time;
  - 50 to 210 ms of the shell's CPU per call, on its UI thread;
  - plus about 50 ms of Python start-up for each `yos` client.

  Minds call it at the start of most turns. The live Director polls it every 5 s, gates poll it, and
  nothing bounds how many callers or how often.

## Principles

1. **The person's session is what is protected:** the compositor, the shell, input, and on the live
   machine the stream. Minds, harnesses and agent work are guests that yield. Nothing is "never
   killed"; protection is relative.
2. **Placement before policy.** Cgroups classify by where a process was started, not by who asked
   for it. Agent work has to be started inside its own unit, not moved there afterwards.
3. **Observation must be cheap and bounded.** One slow or greedy caller of the control socket must
   not take the UI thread. Cgroup weights cannot fix a single busy thread, and a high-priority shell
   that does unlimited work for low-priority callers launders their priority.
4. **Only root or the person relax limits, never a mind.** A mind may ask for a profile, never grant
   itself one. Work that runs as the person's own account is not a security boundary, so where a
   limit must hold against that work, a privileged broker applies fixed profiles.
5. **Measure, then adapt.** The adaptive part is a small state machine with hysteresis on real
   signals, never a model turning kernel knobs.

## The plan

### Step 0: reproduce, with probes (a test VM, not 520 or the family box)

Reproduce the outage on a test VM with each load on its own and then together:
- `describe` polling at 0.4 s;
- `grim` every 3 s;
- guest-agent `guest-exec` polling every 3 s.

While it runs, record:
- per-thread CPU of yantrik-ui, and the state and wchan of the publisher, qemu-ga and yantrik-ui;
- `/proc/pressure/*`, `cpu.stat` and `memory.events`/`pids.events` of the session scope;
- open FDs and task counts;
- the journals of qemu-guest-agent and the publisher.

Capture the stacks of anything stuck before restarting it.

Done when the cause is named, or the remaining candidates are.

### PR A: describe is cheap and bounded (the shell)

- **Snapshots off the UI thread.** The UI thread copies the minimal owned state; a worker does the
  file reads, the filtering and the JSON.
- **Field selection.** `yos describe shell --fields conversation,thinking`, with revisions per state
  domain. A caller that names the revision it holds gets "unchanged" without a rebuild. The Director,
  gates and the recorder use fields.
- **Bounds.** One outstanding observation per caller identity (agent token or harness identity,
  never PID: every `yos` is a new process), a global observation budget, response deadlines, and the
  eviction of slow readers. Actions and approvals are served on their own path, so observation can
  never hold up a person's click or a card.
- **Tests.** A flood of describe requests while a person's action and a card are timed: their
  latency stays bounded.

### PR B: agent work runs in a unit of its own (the shell and the updater)

- **A supervised launcher.** `shell.agent_run` jobs (and, through it, headless browsers and builds)
  start directly in a transient user service under a `yantrik-agents.slice`, as the job is created.
  It covers PTY ownership, cancellation, `KillMode=control-group` so every descendant goes with the
  job, and the exit or OOM reason shown on the job.
- **An aggregate budget on the slice.** Starting values for 6 vCPU / 12 GB, scaled from the
  machine's cores and memory at install:
  - `CPUWeight=10`
  - `CPUQuota` at half the cores
  - `MemoryHigh` at a quarter of RAM
  - `MemoryMax` at a third
  - `TasksMax=1024`

  Each job also gets limits of its own. New work is refused visibly when the budget is spent.
- **I/O.** `IOWeight=10` once io.cost or BFQ is enabled. Not before: today it does nothing.
- **Small machines (8 GB and less).** zram, so pressure rises gradually instead of falling off a
  cliff.
- **Tests.** A process storm, a memory hog and a disk writer as agent jobs: the desktop stays usable,
  and SSH and the guest agent answer.

### PR C: the session and the live machine are placed explicitly

- The shell and the compositor get a high sibling weight (`CPUWeight=1000`). `MemoryLow=` is added
  only after the real working sets are measured.
- The live publisher loses `Nice=10`, keeps its memory bound, gets `Restart=on-failure`, and reports
  frame progress: no new segment for 20 s is a failure, not a quiet stall.
- `sshd`, `qemu-guest-agent` and the egress proxy stay outside every agent limit, and their branch
  keeps capacity.

### Later

- **Layer 3, the adaptive state machine.**
  - Signals: person input and lock state, sustained PSI, shell action latency and observation queue
    depth, AC/battery and thermal state.
  - Actuators: job admission, agent concurrency, the agent quota.
  - Rules: hysteresis and minimum dwell times. Locked is not the same as expendable.
  - Window focus is not proof that a person is there.
- **`pressure` in describe**, so minds and the Director defer heavy work. It is telemetry, not
  enforcement.
- **Local model servers** get a class of their own with a memory budget. GPU and VRAM go through
  admission control: model size, inference concurrency, unload and cancel.
- **The privileged broker** for limits that must hold against same-account work.
- **The person's view and control.** System Monitor by class (Session, Apps, Minds, Agents), and
  Settings → Background work: Gentle, Balanced or Eager.

## Order and gates

1. Step 0 runs first.
2. PR A and PR C can go in parallel.
3. PR B follows, because the launcher is the largest change.
4. Each PR gets a security review (B and C touch units the updater installs), a 520 test, and the
   regression tests above.
5. Layer 3 waits until A to C have run on 520 and 561 for a while.
