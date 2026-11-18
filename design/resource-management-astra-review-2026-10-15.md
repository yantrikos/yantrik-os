## 1. The incident implicates the recorder, but does not prove CPU starvation

Your principle is right: protect human interaction and contain autonomous work. But “one poller starved the machine” is not established.

On six vCPUs, 150–190% CPU ordinarily means fewer than two CPUs busy. That can saturate a UI thread, but should not indefinitely starve unrelated services. Free RAM argues against global memory exhaustion, not against I/O stalls, locks, PID limits, or host contention. The stream recovering when recording stopped is strong evidence of interference—not its mechanism.

Investigate these alternatives:

- **Capture interference:** `grim`, compositor readback, and streaming may contend on capture, GPU synchronization, or buffer allocation. A ticking clock does not establish that capture delivery works.
- **UI/control serialization:** synchronous file reads, serialization, socket writes, or a held lock can block shell requests. A client that stops reading can be especially damaging.
- **Guest-agent protocol blockage:** inspect which QEMU guest-agent commands you issued. A blocking command, serialized request handling, outstanding execution requests, or virtio-serial/channel trouble can wedge management independently.
- **Process accumulation:** a polling loop that launches another `yos` before the previous one finishes can exhaust tasks, descriptors, pipes, or queues.
- **Storage or hypervisor contention:** screenshot writes, full filesystems, disk latency, CPU steal, and host-side throttling can matter despite guest CPU headroom.

During reproduction, collect per-thread CPU, process states and `wchan`, `/proc/pressure/*`, `cpu.stat`, `memory.events`, `pids.events`, disk latency, open-FD counts, and service journals. Capture publisher and guest-agent stacks before restarting them. Repeat with describe-only, screenshots-only, and guest-agent polling-only.

## 2. Layer 1: correct direction, but the hierarchy is the design

**Cgroups classify processes by placement, not ancestry, intent, or window ownership.** Your agent work currently inherits the shell’s placement; fixing that is foundational.

Launch each job directly into a transient service or scope. Do not start it unrestricted and migrate it later: descendants can escape the move. For terminal panes, integrate PTY ownership with the launcher; `systemd-run --user --scope` is useful experimentally, but not a complete production job supervisor. Transient services give clearer lifecycle, timeout, and cleanup semantics.

The larger complication is system versus user managers:

- A user-created slice remains below `user@UID.service`, within `user-UID.slice`.
- It cannot become a sibling of a system slice.
- TTY-session processes may be in a session scope, separate from user-manager services.
- CPU weights compete at each sibling level. Identical-looking weights in different branches do not imply comparable priority.

Audit `/proc/PID/cgroup` and `systemd-cgls` first. Explicitly launch the shell, compositor, apps, bridges, and jobs into intended units. Publish the actual tree, including ancestor settings.

Use a privileged broker for system-level agent units where enforceable limits matter. Alternatively, user slices provide useful *cooperative* resource management, but not security against that user.

Other corrections:

- `CPUWeight=` distributes CPU under contention; it is neither a reservation nor a latency guarantee.
- `CPUWeight=idle` is suitable for genuinely optional work, after checking kernel/systemd support. Do not place dependencies of interactive work there.
- `MemoryLow=` is best-effort reclaim protection and needs appropriate ancestor budgeting.
- `MemoryHigh=` induces reclaim/throttling; it is not a ceiling.
- `MemoryMax=` is containment, not graceful degradation.
- “Never killed” is impossible. Avoid blanket `OOMScoreAdjust=-1000` or excessive `MemoryMin=`: protecting everything can make recovery worse.

Keep the publisher separate from the session. Give it adequate CPU priority, but retain containment: a failing encoder should not inherit the desktop’s protection.

## 3. Layer 2 is probably your highest-value performance work

A 157 KB reply is not inherently enormous. Repeated UI-thread work, filesystem access, Python startup, and unbounded concurrency are the concern.

Build immutable, versioned snapshots. The UI thread should copy minimal owned state; workers handle file access, filtering, and serialization. Do not simply read Slint/UI objects from another thread.

Add:

- Field selection and revisions per state domain.
- Cached serialized responses and conditional reads.
- A bounded observation queue, request-size limits, response deadlines, and slow-client eviction.
- Subscription/coalescing support instead of rapid polling.
- One outstanding request per client, plus an aggregate observation budget.

“Per caller” cannot mean PID: every `yos` invocation has a fresh one. Use authenticated session identities, with a global ceiling; identities must not multiply the available budget.

Separate interactive actions and approvals from bulk observation, while enforcing fairness so neither queue blocks forever. Reserve capacity for cancellation. A high-priority shell executing unlimited low-priority requests is **priority laundering**: cgroup weights alone cannot fix it.

## 4. Memory, I/O, management access, and security need explicit treatment

For Debian 13, verify rather than assume:

- Unified cgroup v2 and enabled controllers.
- `/proc/pressure/` availability and usable per-cgroup PSI.
- Whether `systemd-oomd` is installed, enabled, and monitoring the intended units.

`ManagedOOMMemoryPressure=kill` is not “kill agents first.” oomd selects eligible descendant cgroups under monitored pressure; placement, eligibility, and grouping matter. Test it with disposable jobs before enabling broad policies. Kernel memcg OOM from `MemoryMax=` remains a separate mechanism.

Consider zram or modest swap for small machines. No swap makes pressure transitions sharper; swap is not permission to overcommit indefinitely. Put a local LLM server in its own budgeted class, rather than burying several gigabytes inside “mind.”

For storage, try `IOWeight=10` for agents, but verify effectiveness with your actual block scheduler and device stack. Use device-specific bandwidth limits where predictable containment is necessary. Account for buffered writes and storage capacity, not just throughput.

GPU time and VRAM lack a portable cgroup-v2 equivalent to these CPU/memory controls. Initially use admission control: model-size limits, bounded inference concurrency, and explicit unload/cancel support.

Keep `sshd` and `qemu-guest-agent` outside agent limits. CPU weight helps only if their ancestor branch also has capacity. Watchdogs require real liveness signals: `WatchdogSec=` does nothing useful without notifications. Detect UI-loop progress, not merely a living process.

Most importantly, **same-UID agents are not a trustworthy security boundary**. They may control user units, signal same-UID processes, and access session credentials. A socket peer UID also cannot distinguish a person from their agent. Strong enforcement needs separate identities/sandboxing and a privileged broker that accepts fixed workload profiles—not arbitrary systemd properties.

## 5. Adaptive policy: build a small state machine

Use:

- Explicit human-input activity and session lock state.
- Sustained CPU, memory, and I/O PSI.
- Shell action latency and observation-queue depth.
- Battery/AC status and thermal pressure where available.

Actuate job admission, concurrency, agent CPU quotas, and selected weights. Use hysteresis and minimum dwell times. Locked does not necessarily mean expendable: calls, downloads, and remote sessions continue.

Do not continuously retune every process, let an LLM control kernel knobs, or treat focus as proof of human ownership. Mind-operated windows can gain focus. Publishing pressure is useful telemetry, but cooperative yielding is not enforcement.

## 6. First PR: narrow containment plus observation backpressure

I would split your proposed PR into independently testable changes, even if released together.

1. **Introduce supervised agent launching**, with cancellation and full descendant cleanup. For the six-vCPU VM, an initial *aggregate* agent slice could use:
   ```
   CPUWeight=10
   CPUQuota=300%
   MemoryHigh=3G
   MemoryMax=4G
   TasksMax=1024
   IOWeight=10
   ```
   These are starting budgets, not universal defaults. `300%` means three CPUs’ worth. Add smaller per-job limits; aggregate limits prevent many individually legal jobs overwhelming the machine.

2. **Use `KillMode=control-group` and appropriate `OOMPolicy=kill` on job services.** Preserve exit/OOM reasons in the UI. Reject new work visibly when budgets are exhausted.

3. **Place the shell/session explicitly**, with high sibling CPU weight, initially `CPUWeight=1000`. Add measured `MemoryLow=` protection only after validating ancestor placement and actual working sets.

4. **Remove the publisher’s `Nice=10` initially**, retain its memory limit, and measure whether its one-CPU quota is sufficient. Add restart-on-failure and frame-progress health reporting.

5. **Bound describe concurrency and queues immediately**, cache responses, implement conditional reads, and remove file I/O from the UI thread. These changes may prevent tonight’s failure more directly than weights.

6. **Add regression tests:** request floods, slow readers, process storms, memory exhaustion, capture contention, and cancellation. Require bounded interactive latency and continued SSH/guest-agent access.

Only root-controlled policy or a genuinely authenticated human path should relax limits. An agent may request a profile; it must never be allowed to grant itself one.
