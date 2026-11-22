# Throwaway CI runners (#1234)

The `test` job of `ci.yml` runs on a self-hosted runner labelled `yantrik-build`. That runner
is not a machine: it is a **virtual machine** on node2 that is cloned for one job and destroyed when
the job ends. Nothing a job writes, plants or steals is there for the next one, and a kernel bug
reachable from a job gives root only inside a VM with its own kernel, which is then destroyed.

**How a job reaches the runner (22 Nov).** A push to `main`, a `workflow_dispatch` run of `ci.yml`, and a
pull request from this repository that carries the `ci:our-runner` label go to `yantrik-build`; the `runs-on`
expression in `ci.yml` routes on the event, the head repository and that label, and nothing else. Every other pull
request, fork or not, runs on GitHub's machines, because one serial VM at about 85 minutes a job cannot carry the
pull request traffic. Adding a label needs triage or write access (the labelled code still comes from a branch only write access can push), so the label is how a pull request that is ready to merge
gets its own `test` run on our runner: `gh pr edit <n> --add-label ci:our-runner` starts a new run of the pull
request (the `labeled` event), and later pushes keep going to our runner while the label stays. Any label fires `labeled` and a newer run replaces a running one, so add other labels first. Branch protection
reads only the pull request's own runs, so a `workflow_dispatch` result on the branch does not count for the
merge; it is still useful for a branch with no pull request. The expression only routes: the job-started hook
(admitting `pull_request` only from this repository) and the controller's job check stay the boundary. On the VM the `test` job's one build step is
`deploy/ci-runner/run-test-job.sh`; the hosted steps are skipped there by `runner.environment`.

Release and nightly ISO builds (iso.yml: build, sign, publish) always run on GitHub-hosted runners. The self-hosted runners take only ci.yml's `test` job, and the job-started hook refuses everything else.

## Why it is built this way

GitHub's 16 GB runners lost the `test` job about half the time, with no log. Pranab decided
(15 Nov) to run it on node2. The first attempt was one long-lived runner on the build host
(CT 530); the security review of PR #1252 rejected that (round 1, comment 6888810312). Rounds 2
(6889527314), 3 (6891217309), 4 (6892879091) and 5 (6893462695) found what the throwaway design still had to close.
The design first cloned a **container** per job; the last review (CLEAR, 6894050927) left the owner two
things (6902247200): R6, the container's shared kernel, and the clone's fixed LAN address, which needed a
router reservation. On 17 Nov the owner's direction was: use a VM. This version does, and both are gone.

| Review id | Finding | Answer here |
|---|---|---|
| C1 | A fork PR can edit `ci.yml` and send its own job to the runner | Fork-PR approval is `all_external_contributors`. Two checks refuse a job that is not `ci.yml`'s `test` from this repository: the hook inside the VM (N2) and the controller, which asks GitHub which job the runner took (R1). The `runs-on` expression excludes forks too (L2) but is not the boundary: the fork edits that file. |
| H1 | Jobs share a uid and files | A fresh clone per job. `/opt/rust` and `/opt/actions-runner` are root-owned; `runner` cannot write them. JIT credentials are single use. Whatever a job plants dies with the VM. |
| H2 | Anyone with write access gets a shell on the build host | They get a shell in a VM that is destroyed after the job, with no LAN (N1, R3) and no secret. |
| M1 | The shared build lock can be held or broken | No lock: one runner at a time, one VM per job. Memory is gated before a runner and again on every idle poll; `cpuunits 50`; the clone's QEMU process is the OOM killer's first victim (see Capacity). |
| M2 | Shared `/tmp` | The job has its own VM and its own `/tmp`. |
| N1 | The clone reaches the LAN | **The clone has no LAN at all.** It sits on `yci0`, a bridge with no physical port, on its own subnet; the node routes and masquerades for it under one nftables table that matches only `yci0` (below). Inside each VM, a second filter (nftables) is loaded by the launcher. The Proxmox cluster firewall is **not** used: it is cluster-wide and touches machines that are not ours. |
| N2 | The runner takes any job from this repository | A root-owned job-started hook (`/usr/local/libexec/yantrik-ci-job-started.sh`). See R1: it is not the only check. |
| N3 | A repository-admin token on node2 | An organisation-level runner and a token whose only permission is "Self-hosted runners: Read and write". |
| N4 | A job can exhaust node2 | The VM has fixed cores and memory and an 80 GB thin disk; `cpuunits` 50. The runner's output is not copied to the node at all (the job log is GitHub's), so there is no log for a job to flood. A pids limit is no longer needed: a fork bomb is bounded by the VM's own memory and kernel. |
| N5-N7 | Registrations left behind; idle clock; destroy guard | Deregistered in `finish_vm` on every path and swept again after a failed delete; the 3 h clock starts when the runner is busy; `destroy_vm` requires the name `yantrik-ci-<id>`. |
| N8 | The template changed the hypervisor's sysctls | Not done by the script; see "One-time step on the node". |
| R1 | The hook is a bash script: `BASH_ENV` in the job's environment may make it pass | The controller checks out-of-band (below), and the first real job is a probe. |
| R2 | The unit was enabled; a node reboot would start CI | `install-controller.sh` never enables, starts or restarts the unit. Enabling is the operator's step. |
| R3 | The guest-side filter rests on guest root alone | `NoNewPrivileges=yes` on the runner service; `sudo` purged; the node-side table on `yci0` that does not depend on the VM at all (below); proofs. |
| R4 | A job can fill the thin pool | Pool gate before every clone (free space at least disk + 20%, metadata below 80%); disk 80 GB, sized from the measured peak; the disk written is logged per job. |
| **R6** | The job's kernel surface | **Closed.** A VM runs Debian's own kernel (6.12 in the image used); the node's kernel (6.17.4) is no longer reachable from a job's user namespace. What remains is the hypervisor boundary (QEMU/KVM, virtio), which the node's own updates keep current like every other VM on it. |
| R7-R15 | Prune per cycle; fail closed on a widened group; DHCP rule; own public addresses; `--description`; recheck before recycling; re-armed sweep; netns proof; `self-hosted` label | All done, see the sections below. (There is no DHCP any more: the address is static, by cloud-init.) |
| H1 (r4) | The host filter was L3 only: root in the guest could spoof its MAC, ARP the LAN, broadcast a rogue DHCP offer | There is no LAN to ARP. The table's ingress hook on the clone's port pins port, MAC and address of every IPv4 frame and drops IPv6; the port itself is isolated and locked to its own MAC on the bridge (below), which covers ARP and every other frame type; proved by spoofed frames. **One limit, found by testing: ARP is not filtered by the table (see below).** |
| M1-M4 (r4) | API body in the environment; slow job check; `pct exec` into a hostile CT; filter not re-checked | No API body in the environment or on a command line; 10 s poll, job looked up by `runner_name`, post-hoc audit; host-cgroup stats and `lvs`, nothing asked of the guest after the runner starts; `net_ok` on every poll. |
| L1-L5 (r4) | oom write fail-open; `veth96*`; unchecked libs; recycle on API error; table lingers | Fatal oom write and a `ci.yml` check; exact port set; run-time ownership checks; recycle only on a clear "not busy"; stopping the controller removes the table and the bridge. |
| N1-N7 (r5) | An unpinned clone was unfiltered; "cannot tell" ran a job unbounded; DHCP and third-party ARP attacks; two looks too few; drill expectation; `nap` clobbered `CHILD_PID` | The ingress hook names every port a clone can have and **drops the IP frames of any that is not pinned**; third-party ARP attacks: the port is isolated (no other port to attack) and locked; `last_ok`, `apilost` and an absolute cap; a static address and no DHCP; 4 looks; the drill and tests below; `RUNNER_PID`. |
| L1-L4 | token in `.git/config`, `head.repo.fork`, toolchain, stale comment | `persist-credentials: false`, `!head.repo.fork`, baked-in toolchain plus a `rustc` check, comment deleted. |
| N1-N3 (#1303, round 2) | The agent close was judged by a ping and needed the guest; the stop-time cleanup took no lock; the clone allowlist checked keys, not values | QEMU's own view decides, and a device that stays destroys the VM (L3, N1); the cleanup takes the controller's lock (N2); the values of `scsi0`, `scsi1`, `serial0`, `vga` and `allow-ksm` are checked and `ide2` is gone (N3). See "However the controller ends" and "Backups, the agent and cloud-init". |

### What changed from the container design, and what went with it

- **No LAN address.** `/etc/yantrik-ci/clone-net`, its checks (`clone_net_check`, the ARP probe `arp_query`,
  `clone_addr_free`), `check-host-filter.sh` and the router reservation are no longer needed and are gone;
  `install-controller.sh` removes the file. Each clone's address is derived from its id (VM 9541 is
  `10.95.30.41`); the address range cannot clash with the LAN because it is not on it.
- **The IP table is `inet yantrik_ci`, not `bridge yantrik_ci`**, and it also masquerades. Its ingress hook is on
  the tap ports `tap9540i0` to `tap9549i0` and a port that is not pinned is dropped.
- **The job's output stays in the VM.** The container design copied the runner's output through `pct exec`
  into a capped file on the node. A VM has no such channel: the guest agent is used once, to hand over the
  config, and never again after the runner has started.
- **The guest powers itself off when its runner exits**; the controller waits for that (or its limits), then
  destroys the VM.
- **No pids limit, no swap** (a container knob). The VM's memory is its limit.
- **Debian 13, not Ubuntu 24.04**, because the VM boots Debian's own cloud image (genericcloud) with its own kernel.
  Not proved: that `ci.yml`'s `test` job builds and passes on it as it did on Ubuntu. The first real job
  tells.

### Repository settings (checked by Y on 15 Nov)

Fork-PR approval is `all_external_contributors`. The only collaborator is `spranab` (admin). The
only installed GitHub App is the Claude GitHub app. Re-check these before the first start and after
any change of collaborators.

## The pieces

- `make-template.sh` builds VM **9540** `yantrik-ci-template` and converts it to a template. It brings up
  `yci0` and the table, imports Debian 13's genericcloud image **once** (its checksum is checked and the
  download is deleted), starts it on `yci0` pinned like a clone, reaches it over a throwaway ssh key
  (cloud-init) from the node, provisions it, runs the proofs, seals it and converts it. 32 cores
  (`x86-64-v2-AES`), 24 GB, no balloon, `cpuunits` 50, 80 GB on `local-lvm` (thin: a linked clone uses only
  what it writes; a cold workspace build and the whole test run peak at 23 GB), one card on `yci0`, a
  cloud-init drive, the QEMU guest agent, a serial console. It contains the build libraries from `ci.yml`,
  `nftables`, Rust 1.92.0 (root-owned in `/opt/rust`), the latest GitHub Actions runner in
  `/opt/actions-runner` (root-owned, checksum-checked) and the `runner` user (uid 1001, no password).
  Sealing removes `sshd`, `sudo`, the image's default account and the build key, tells cloud-init to make no user and
  take no key, and clears the machine id, host keys and cloud-init state: **no way in remains except the guest
  agent, which only the node can talk to**. It refuses to run until `/etc/yantrik-ci/own-public-ips` lists our own
  public addresses (the router's WAN address and any other), one per line; they are baked into the template and
  into the node's table. The files in `guest/` are installed, root-owned:
  - `yantrik-ci-start`: run by the controller through the guest agent with the one-job config on **stdin**;
    keeps it in a root-only file on tmpfs and starts the launch unit. Refuses a second call.
  - `yantrik-ci-launch` (`yantrik-ci-launch.service`, never enabled): reads the config once and deletes it,
    loads the egress filter, makes the runner's directory on a tmpfs (the listener decodes the config into its
    credential files there, so they are never on the clone's disk; the job's work folder is a symlink to the disk),
    stops and masks the guest agent (the host never uses it after the hand-over, L3), then starts the runner as
    `runner` in a transient `systemd-run` service (so the job's
    process tree reaches pid 1), with `NoNewPrivileges=yes`, a fixed `--description` (so the JIT config on
    the command line is not copied into unit metadata), the hook, and `CARGO_PROFILE_DEV_DEBUG=0
    CARGO_INCREMENTAL=0`. When the runner exits the VM powers off.
  - `yantrik-ci-egress` and `egress.nft`: the guest-side filter.
  - `yantrik-ci-job-started.sh`: the hook.
  - `provision.sh`, `install.sh`, `seal.sh`, `template-proofs.sh`: the build, inside the VM.

  Before it becomes a template the script proves, inside the build VM (on the same filtered network as a job):
  `bwrap` works as `runner`, also under `NoNewPrivileges`; `runner` cannot write `/opt/rust`, `/opt/actions-runner`,
  the launcher, the hook or the rules; `sudo` and `sshd` are gone; `runner` cannot read the guest agent's channel;
  the hook admits and refuses the right jobs (`iso.yml` included); the node's API and ssh (on the LAN and on
  `yci0`), a LAN host, the router, port 80 and IPv6 are unreachable; GitHub and the crates hosts are reachable;
  `runner` cannot list or change the rules, also from a user namespace. Any failed check stops the build.
  The listener keeps its credential files in its own directory and finds it from where its binary really is, so
  the launcher gives each job a copy of the runner (`/home/runner/actions-runner`) and leaves `/opt/actions-runner`
  pristine.
- `yantrik-ci-controller.sh` with `lib/github.sh`, `lib/host.sh` and `lib/net.sh` (installed to
  `/usr/local/lib/yantrik-ci/`) runs one loop. Before every clone it checks memory (`MemAvailable` at least the template's memory plus 21 GiB of headroom, 45 GiB for the 24 GB template),
  the thin pool, and brings up and verifies the network (`net_up`); it refuses to clone without any of them.
  Then: ask GitHub for a one-job runner config, clone template 9540 to a free id in 9541-9549 (linked clone), set
  its one card (`yci0`, its own MAC kept, a static address and the resolver by cloud-init) and refuse a clone
  with any other device, pin its port, MAC and address in the table, `qm start`, set its QEMU process's
  `oom_score_adj` to 1000, wait for the guest agent, push the config on stdin to `yantrik-ci-start`, wait for the
  runner to take its job and the guest to power off, log the disk written and `memory.peak`, destroy the VM,
  deregister the runner, loop. It destroys leftover `yantrik-ci-<id>` VMs when it starts, never touches an id
  outside the range or a VM with another name, and backs off from 30 s, doubling to 10 min. At stop it removes
  `yci0` and the table.
- `host/yci.nft`, `host/yci-l2.nft` and `lib/net.sh`: the node-side network (below). `yantrik-ci-net up|down|status|pin|unpin` runs
  the same code by hand. `test/check-net.sh` proves it.
- `install-controller.sh` installs the controller, its libraries and the unit. It does not enable,
  start or restart anything, and does not make the bridge or load the table.

### The clones' network (N1, R3, H1)

`yci0` is a Linux bridge with **no physical port**, address `10.95.30.1/24` on the node, made at runtime by
`ip link` (`net_up`) and removed by `net_down`: no network file is edited, `ifreload` is never run and no
existing bridge, table or firewall is touched. The node forwards and masquerades for it. Forwarding is
switched on per interface (`yci0` and the uplink), not globally; the previous values are kept in
`/run/yantrik-ci/net.saved` and put back at stop. Everything below is two nftables tables, `inet yantrik_ci`
(`host/yci.nft`) and `netdev yantrik_ci_l2` (`host/yci-l2.nft`); **no rule in either matches a packet that does not come from or go to `yci0` or a clone's tap**.

- **`ingress` (priority -500), on the ports `tap9540i0` to `tap9549i0`**, policy drop. An IPv4 frame from a
  clone passes only if it matches the clone's pin (its tap port, its MAC, its address), which the controller adds
  from `qm config` after the clone is configured and **before** `qm start`, and removes only once the VM is gone,
  **and is addressed to the node's MAC**, so a frame for another clone dies here. IPv6 and DHCP (source 0.0.0.0)
  are dropped. **A port that is not pinned is dropped**: a clone that survives its job (it is unpinned at once),
  or a table reload, has no network.
- **`netdev yantrik_ci_l2` (priority -600), on the same ports (`host/yci-l2.nft`, #1303):** the kernel hands every
  frame that is not IPv4 or IPv6 (ARP, VLAN tags, LLDP, any other ethertype) to an `inet` ingress chain
  unevaluated, so the table above cannot filter them (found on node2: a forged ARP reply got through). A `netdev`
  ingress chain sees every frame the tap receives, before the bridge does. **Why `netdev` and not `bridge`:** a
  `bridge`-family table hooks every bridge on the node (`vmbr0` and the others) and needs a rule to skip them,
  while this one is attached to the named taps alone, is one table for all ten, and goes with one `delete table`.
  From a clone only these pass: IPv4 (judged by the table above) and an ARP request or reply whose sender is
  the clone's pinned MAC and address and whose target is the gateway `10.95.30.1`, to broadcast or the node's MAC.
  Every other ARP (gratuitous, a claim on the gateway's or another clone's address, a request for anything but
  the gateway) and every other ethertype is dropped and counted, as is any frame from a MAC that is not the
  port's. It shares the pins of the table above (`net_pin` and `net_unpin` fill and clear both, `net_ok` checks both
  on every poll). **Order:** `devices = ` names the taps before they exist and the kernel attaches the hook when
  the port is registered, before it can be brought up: `check-net.sh` section 0 creates a tap, shows the hook on it
  while it is still down, and writes its first frames into it (all dropped). The window is zero for this filter.
- **The port on the bridge (`net_lock`, right after `qm start`, before the config is handed over):** isolated and
  locked with one static entry, its own MAC, in addition to both tables. Isolation means a clone's frames reach the
  node's side of the bridge and no other port (no clone to clone, for any frame type); locking means a frame from
  any source MAC but its own is dropped by the bridge and nothing is learned (no forwarding-table flooding or
  poisoning). This is the only part that waits for `qm start` (the port exists only from then on): between
  `qm start` and `net_lock` the guest is the pristine template, no job code exists until the config is handed
  over, its frames are already limited by both tables, and a failed `net_lock` destroys the VM.
- **`forward`:** replies to a clone's connections pass; nothing else is sent toward a clone. From a clone: UDP and
  TCP 53 to the one public resolver (`9.9.9.9`) and TCP 443 to public addresses. Our own public addresses
  (`/etc/yantrik-ci/own-public-ips`: hairpin NAT would reach the router's admin page), private, CGNAT,
  link-local, multicast, loopback, broadcast and reserved destinations, and every other port and protocol
  (ICMP included) are dropped.
- **`input`:** frames for the node itself (its API, its ssh, DNS, ping) are dropped, except replies to what the
  node opened (so the node can reach a clone: the template build uses it). ARP is below IP and still works.
- **`postrouting`:** the clones' subnet is masqueraded.

The guest's own address and resolver come from cloud-init (static, from `qm set --ipconfig0 --nameserver`): there
is no DHCP and nothing on the LAN to ask. The guest has no IPv6.

The controller loads both tables before every clone, pins the clone (in both, in one transaction), and checks the
tables, the bridge and the pin on every poll while the job runs; if a table is gone (`nft flush ruleset`) it ends
the VM. If a VM in 9541-9549 survives the end of a job, or the start-up cleanup (L2: it is unpinned there too),
the controller unpins it, **exits 3** and leaves the tables and the bridge up. The unit has `RestartPreventExitStatus=3`, so it stays `failed` (visible in `systemctl --failed`)
instead of restarting every 30 seconds; deal with the VM, then `systemctl restart yantrik-ci-controller`. **Do not
enable `nftables.service` on node2 with a `flush ruleset` configuration** (Debian's default `/etc/nftables.conf`): a
restart removes the table, and the controller then ends the running job.

**However the controller ends (L1).** A SIGTERM can be lost in bash if it arrives while the main shell is parsing a
command substitution, and SIGKILL or an OOM kill give no handler at all, while QEMU lives in `qemu.slice`, not in the
unit: a clone would keep running with its pin and no limit. So the stop does not rest on the trap alone:
`ExecStop=` makes `/run/yantrik-ci/stop`, which the controller checks wherever it checks for SIGTERM (its sleeps
included; it removes the file at start), and **`ExecStopPost=/usr/local/sbin/yantrik-ci-net cleanup`** runs after the
controller in every case: it destroys any clone in 9541-9549 named `yantrik-ci-<id>`, unpins one that cannot be
destroyed, and takes the network down (which refuses while a port is still on the bridge, so a survivor keeps its
table and loses its pin). It never starts the unit. **It takes the controller's own lock first (N2)**
(`/run/yantrik-ci/controller.lock`, waiting up to 20 s for it), so it cannot run beside a controller that is starting or
running, nor beside a second cleanup; a lock still held after the wait is a live controller's, and the cleanup then
touches nothing and exits 0. The controller starts QEMU, its watcher and its sleeps with the lock's descriptor closed, so
nothing a dead controller left behind can keep the lock. `check-net.sh` takes the same lock. `test/controller-test.sh`
tests 16b to 20 show each path, 22 and 22b the lock.

`test/check-net.sh` proves the network on two real clones with no runner (below).

### The job check (R1)

The hook is a bash script, and a non-interactive bash expands `BASH_ENV` before it runs the script;
whether a workflow's `env:` reaches the hook process is unproven. So the controller does not rely on
it. Every 10 seconds it asks GitHub whether the runner is busy; once it is, it finds the runner's job
through the public API (`runs?status=in_progress`, then each run's jobs, picking
`runner_name == this runner`) and requires: workflow `.github/workflows/ci.yml`, job `test`, event
`push`, `workflow_dispatch` or `pull_request`, and head repository `yantrikos/yantrik-os` itself.
Anything else, or a job that cannot be found in four looks, ends the VM at once and is logged as
`REFUSING the job`. Nothing from the API goes through the environment or a command line (a run list of
10 or more runs is over the 128 KiB limit of one environment string): the run list is reduced to four
short fields per run. If GitHub cannot say whether the runner is busy, the controller looks for the job anyway, and a job
found is judged at once; after 5 minutes without an answer it ends the VM (`apilost`), and an absolute
cap (idle limit plus job limit) applies whatever GitHub says. If the guest powers off before the first poll
(a short job), the controller looks the job up afterwards among the newest runs (`AUDIT-MISSING` if there
is no record). Every job leaves an `audit:` line with the run id, workflow,
job, event and head repository, and a job that ran and should not have is logged as `VIOLATION`.

**The `BASH_ENV` probe** is the first real job. Branch `scratch/1234-hook-env-probe` (not for merge)
changes `ci.yml` so that `test` does no build and the workflow sets
`BASH_ENV: '$(echo yantrik-probe-hook-env >&2)'`. Run it with
`gh workflow run ci.yml --ref scratch/1234-hook-env-probe` and read "Set up job" in the log. The
absence of `yantrik-probe-hook-env` proves something only if the same step also shows "A job started
hook has been configured by the self-hosted runner administrator". Also confirm `job admitted` in
`controller.log`. If the string appears, workflow env reaches the hook, nothing in a bash hook can stop
it, and the controller's check is the only boundary. Result: **not run: it needs a real job, so the token the owner creates** (17 Nov). Record it here. Until
then, never approve a fork run whose diff touches `.github/`.

**The refusal drill** comes after it. Branch `scratch/1234-refusal-drill` (not for merge) renames the
job to `test2` on the throwaway runner; run it with `gh workflow run ci.yml --ref scratch/1234-refusal-drill`.
Expect the hook's `::error::yantrik-build refuses this job` in "Set up job", **and** either the
controller's `REFUSING the job ... job test2` or its `VIOLATION ... job test2` (the hook ends the job in
seconds, usually before the next poll, so the controller may only see it afterwards), **and** `done: VM ...`
with the VM gone from `qm list`. The drill does not prove the controller kills a *running* VM: the
committed kill-path test (`test/controller-test.sh`, test 15) does. Result: **not run: it needs a real job, so the token the owner creates** (17 Nov).

### Capacity

The node has 109 GB of RAM with more allotted to guests than that. Before a runner is created the
controller requires the template's memory plus 21 GiB (`YANTRIK_CI_HEADROOM_KB`) of `MemAvailable` (45 GiB for 24 GB; the VM has no balloon, so after a build the node holds all of it); while a runner idles it checks again on every poll and, if
memory is short and GitHub says the runner is still not busy, recycles it. The clone's QEMU process gets
`oom_score_adj` 1000 right after `qm start`, so a global OOM on the node ends the CI job first. `finish_vm` logs
`memory.peak` and the oom counts of each job (from the VM's cgroup `qemu.slice/<id>.scope`); size the VM's memory
and `-j` from those after the first real runs. The VM's memory is allocated as the guest touches it, so an idle
clone costs well under 1 GB of the node's RAM.

The container design measured, on 15 Nov, a cold `cargo build --workspace --locked -j 24` then `cargo test
--workspace --locked -j 24 --no-fail-fast`: build 16 min, tests 28 min, disk 23 GB, `memory.peak` 32.6 GB (mostly
page cache), 7601 tests passed. **That measurement is of the container on Ubuntu 24.04 and has not been repeated
on the VM**: a VM has no swap here and a separate page cache, and the first real job's `done:` line is the
first measurement of it.

### Backups, the agent and cloud-init (L3, L5)

- **Backups.** Nothing in the controller uses the guest agent after the hand-over, and the launcher stops it, but a
  backup job that selects all guests would call `fsfreeze` on a running clone, or on the template, through it. VMs
  9540 to 9549 must be **excluded from any vzdump job on node2** (this PR does not change the node's backup
  configuration; node2's configuration is the owner's to check).
- **No first-boot upgrade.** The template and every clone have `ciupgrade 0` (PVE's default writes
  `package_upgrade: true` into the clone's user-data, so each clone would run `apt-get upgrade` as root at first
  boot, concurrently with the hand-over and with a different package set per job). `clone_configure` refuses a
  clone without it, and `check-net.sh` reads the clone's user-data to show it.
- **No upgrades inside a job's VM.** `unattended-upgrades` is purged from the template and apt's daily timers and
  services are masked (`seal.sh`, proved in a clone by `template-proofs.sh`): on 18 Nov it was running beside a 15 GB
  compiler and was the process that set off the guest's OOM killer.
- **The clone's configuration is an allowlist of keys** (`CLONE_KEYS` in `lib/host.sh`), not a denylist: any other
  key (`args`, `hostpci`, `ivshmem`, a second disk, `cicustom`, anything a later PVE adds) refuses the clone.
  The **values** of the keys that could name a host resource are checked too (N3): `scsi0` is a volume of this clone on
  `local-lvm` (or linked to the template's base), `scsi1` is this clone's own `vm-<id>-cloudinit,media=cdrom` (with its size), `serial0`
  is `socket`, `vga` is `serial0`, and `allow-ksm` is `0`; each must be present. `ide2` is no longer allowed: the rebuilt
  template's cloud-init drive is `scsi1`.
- **The agent's device is removed from the host's side, and QEMU says whether it is gone (L3, N1).** After the
  hand-over the controller sends `device_del qga0` (the virtio-serial device PVE puts the agent on) through the
  monitor and then, once a second for up to 20 s, asks QEMU itself (`info pci` and `qom-list /machine/peripheral`,
  both must show `qga0` gone; a monitor that does not answer counts as not gone). That is the only signal: whether
  the agent still answers a ping says nothing, because a stopped guest or a stopped agent fails the ping with the
  device attached. The unplug is a PCI hot-unplug that **the guest must acknowledge**, so a VM whose device is still
  there after 20 s is **destroyed** (`job exit: agent-open`) and its runner deregistered: the guest at that moment is
  still the pristine template, and the launcher does not start the runner until `/dev/virtio-ports/org.qemu.guest_agent.0`
  has gone from the guest too (30 s at most, then it powers the VM off), so no job code has run. The result is logged
  for every job. The launcher's wait is part of the template: rebuild the template (`make-template.sh`) for it to apply.

### Kernel (R6)

Closed by this design: the job runs in a VM with its own kernel (Debian's, from the cloud image, updated by
rebuilding the template). It can create user namespaces, which `bwrap` needs, and a network namespace of its own,
and a kernel bug reachable from them gives root inside a VM that is destroyed after the job and sits on a network
that reaches only the public internet over 443 and DNS. The residual surface is QEMU/KVM and the virtio devices
(a VM escape), which node2's own package updates keep current; this PR does not touch node2's packages, and
node2 stays on the kernel it runs. Named in full, what a root guest can reach in the host kernel and the host's
processes: **KVM** itself (no nested virtualisation is exposed: the CPU type has no `vmx`/`svm`); **QEMU**, which on
PVE runs as root without a confining profile, with q35, virtio-scsi-single with an iothread, virtio-net, virtio-serial
(the agent), a serial socket and the cloud-init drive (now on `scsi1`, virtio-scsi, not AHCI/IDE emulation), and no
VGA, USB or balloon; **vhost-net**, PVE's default for virtio-net, which is in the host kernel; and **the bridge,
conntrack and nftables path** that parses every packet a guest sends (the netdev table sees them first). The template
sets `allow-ksm 0`: KSM merges pages across VMs on an overcommitted node, a cross-VM side channel. These are
hardening and routine hygiene (current PVE updates), not a gate.

## Operations

Run all of this as root on node2. The controller is installed disabled and stopped; it stays so until
the operator decides.

**One-time step on the node** (N8, optional): Debian already sets `fs.protected_*` to 1. Raising
`protected_regular` and `protected_fifos` to 2 is safe for everything on the node:

    printf 'fs.protected_regular = 2\nfs.protected_fifos = 2\n' > /etc/sysctl.d/90-yantrik-ci.conf
    sysctl -p /etc/sysctl.d/90-yantrik-ci.conf

**No fixed address and no router reservation are needed any more.** If `/etc/yantrik-ci/clone-net` exists from the
container design, `install-controller.sh` removes it.

**Own public addresses.** Put the router's WAN address and any other public address of ours in
`/etc/yantrik-ci/own-public-ips`, one per line (the router's status page shows the WAN
address). Rebuild the template after changing the file.

**Install or update the controller**

    scp -r deploy/ci-runner root@<node2>:/root/ci-runner
    ssh root@<node2> 'bash /root/ci-runner/install-controller.sh'

**Build the template** (the controller stopped; takes about 10 minutes; needs about 1 GB of free space in
`/var/tmp` for the image, which it deletes):

    bash /root/ci-runner/make-template.sh

**Create the token** (Pranab does this; nobody else needs to see it). On GitHub: Settings, Developer
settings, Personal access tokens, Fine-grained tokens, Generate new token.
- Resource owner: `yantrikos`.
- Repository access: public repositories only (the default; no repository permission at all).
- Organization permissions: **Self-hosted runners: Read and write**, and nothing else.
- An expiry, and a reminder to rotate it.

Then on node2, without echoing it:

    install -d -m 700 /root/.yantrik-ci
    ( umask 077; cat > /root/.yantrik-ci/token )     # paste, then Ctrl-D
    chown root:root /root/.yantrik-ci/token; chmod 600 /root/.yantrik-ci/token

**Start CI** (the operator's step; a reboot does not do it):

    systemctl enable --now yantrik-ci-controller

On its first cycle the controller creates the organisation runner group `yantrik-ci` (visibility
`selected`, only `yantrikos/yantrik-os`, public repositories allowed, no workflow restriction) or
finds it by name. It refuses a group that is not `selected` or lists any other repository, and
checks that on every cycle. Labels sent for the runner: `self-hosted` and `yantrik-build`; confirm on
the first run that a `test` job is picked up.

**Rotate the token.** Create the new one as above, replace the file the same way, run
`systemctl restart yantrik-ci-controller`, revoke the old token on GitHub. The controller re-reads
the file every cycle and refuses one that is not root-owned mode 600 or 400.

**Rebuild the template** (new runner release, new toolchain, new packages, new public address, a newer Debian kernel).
Stop the controller first and wait for its VM to go; the script refuses while any clone exists.

    systemctl stop yantrik-ci-controller
    bash /root/ci-runner/make-template.sh
    systemctl enable --now yantrik-ci-controller      # only if it was running before

**How often, and who.** A job's code runs as `runner` against the template's kernel and libraries, so the template's patch
level is what stands between a job and root in its VM, and nothing inside a job's VM upgrades itself. `make-template.sh`
runs `apt-get full-upgrade` when it builds. Rebuild **weekly**, and **within a day of a Debian security advisory for the
kernel, glibc, systemd or the virtio drivers**; a new runner release, a new toolchain or a new public address are further
triggers. The owner is the coordinator of this repository's CI ("Y") until a timer does it. The controller logs a plain
`WARNING` line at start when the template is more than 8 days old (`YANTRIK_CI_TEMPLATE_MAX_AGE_DAYS`, read from the
template's own `meta: ctime`); it never refuses.

A runner too old for GitHub tries to update itself and cannot; the job then fails fast and the
controller backs off. Rebuilding is the fix. Moving Rust is a PR of its own and must change
`RUST_VERSION` in `make-template.sh` and the `grep` in `ci.yml` with it.

**Prove the network** (controller stopped, no VM in 9541-9549): `bash /root/ci-runner/test/check-net.sh`
(two real clones, no runner; ends with `ALL PASS` or `FAIL`).
**Test the controller** against a stub of GitHub: `bash /root/ci-runner/test/controller-test.sh` (real
clones with a fake config that reaches nothing: 100-run API body, refusals, the kill path, a surviving VM, API
loss, the network removed at stop, no trace of the fake config on any disk). It ends with `ALL PASS` or `FAIL`.

**Stop CI from using the host** (the `test` job then waits for a runner; switch its `runs-on` back to
`ubuntu-24.04` if it must run meanwhile):

    systemctl disable --now yantrik-ci-controller
    qm list | awk '$1 >= 9541 && $1 <= 9549'      # must print nothing: destroy any VM left, by hand
    yantrik-ci-net down                           # only then; it refuses while a clone's port is on yci0

(An unpinned clone port is dropped while the table is loaded, so the table is what keeps a surviving
clone harmless; `yantrik-ci-net down` removes it last, and refuses while a port is still on the bridge.)

**Read the logs**

    journalctl -u yantrik-ci-controller -f
    less /var/log/yantrik-ci/controller.log        # rotated weekly, 8 kept

One line per event: `cycle`, `cloned`, `running`, `took a job`, `job admitted` or `REFUSING`, and
`audit` with the job, and `done` with the VM id, the runner name, times, the job's exit code (124 is
the limit) and `disk written, memory.peak, oom counts` (read from the host's cgroup and the thin pool;
nothing is asked of the VM after the runner has started). `qm list` shows the VM of a job in flight (9541-9549); there
should be none when idle.

## Known limits

- The JIT config is on the runner's command line inside the VM for the life of the job, as `run.sh` takes it; it is
  good for one job only and the job's user can read it, and so can it read the runner's decoded credentials
  (`.runner`, `.credentials`, `.credentials_rsaparams`), which are GitHub's ephemeral model. They are on a **tmpfs**
  in the VM (L4), not on the clone's disk, so a clone left behind by a crash holds no credentials. The config is on
  the node only in a shell variable of the controller and in the pipe to the guest agent, never in a file or on a
  node command line, and never on the template's disk. It dies with the VM.
- A job can still do anything as `runner` inside its VM for 3 hours: that is what the two filters, the empty VM
  and the disk and memory limits are for.
- The runner's own output is not kept on the node; the job's log is GitHub's.
- The VM has no swap.
- Forwarding is switched on for `yci0` while the network is up, and `net_up` **refuses to start if the uplink's
  IPv4 forwarding switch is off** (L6): IPv4 forwarding is decided by the input device's switch, so turning it on
  for the uplink would make the node route everything arriving there. On node2 it is already on and is left alone.
  The `yci0` switches, `arp_ignore` 1, `arp_announce` 2 and `drop_gratuitous_arp` 1 are put back by `net_down`.
  Nothing else on the node's network is changed.
- An orphan thin volume `vm-954x-disk-N` whose config was never written (a crash inside `qm clone`) is not swept
  at start-up (accepted; `lvs` shows it).
- The accidental start: on 15 Nov, re-running the installer restarted the service while a real token
  was in place, so the real controller ran for about 80 seconds (it created the org runner group,
  registered one runner, listened for jobs, took none, and was stopped; the runner was deleted).
  The installer no longer starts, restarts or enables anything.
