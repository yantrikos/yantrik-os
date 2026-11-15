# Throwaway CI runners (#1234)

The `test` job of `ci.yml` runs on a self-hosted runner labelled `yantrik-build`. That runner
is not a machine: it is a container on node2 that is cloned for one job and destroyed when
the job ends. Nothing a job writes, plants or steals is there for the next one.

Release and nightly ISO builds (iso.yml: build, sign, publish) always run on GitHub-hosted runners. The self-hosted runners take only ci.yml's `test` job, and the job-started hook refuses everything else.

## Why it is built this way

GitHub's 16 GB runners lost the `test` job about half the time, with no log. Pranab decided
(15 Nov) to run it on node2. The first attempt was one long-lived runner on the build host
(CT 530); the security review of PR #1252 rejected that (round 1, comment 6888810312). Rounds 2
(6889527314) and 3 (6891217309) found what the throwaway design still had to close.

| Review id | Finding | Answer here |
|---|---|---|
| C1 | A fork PR can edit `ci.yml` and send its own job to the runner | Fork-PR approval is `all_external_contributors`. Two checks refuse a job that is not `ci.yml`'s `test` from this repository: the hook inside the CT (N2) and the controller, which asks GitHub which job the runner took (R1). The `runs-on` expression excludes forks too (L2) but is not the boundary: the fork edits that file. |
| H1 | Jobs share a uid and files | A fresh clone per job. `/opt/rust` and `/opt/actions-runner` are root-owned; `runner` cannot write them. JIT credentials are single use. Whatever a job plants dies with the container. |
| H2 | Anyone with write access gets a shell on the build host | They get a shell in a container that is destroyed after the job, with no LAN (N1, R3) and no secret. |
| M1 | The shared build lock can be held or broken | No lock: one runner at a time, one CT per job. Memory is gated before a runner and again on every idle poll; `cpuunits 50`; the clone is the OOM killer's first victim (see Capacity). |
| M2 | Shared `/tmp` | The job has its own CT and its own `/tmp`. |
| N1 | The CT reaches the LAN | Two layers. Inside each CT, an nftables egress filter loaded by the launcher. On node2, a `table bridge yantrik_ci` that matches only the clones' ports. The Proxmox cluster firewall is **not** used: it is cluster-wide and touches machines that are not ours. |
| N2 | The runner takes any job from this repository | A root-owned job-started hook (`/usr/local/libexec/yantrik-ci-job-started.sh`). See R1: it is not the only check. |
| N3 | A repository-admin token on node2 | An organisation-level runner and a token whose only permission is "Self-hosted runners: Read and write". |
| N4 | A job can exhaust node2 | Run log capped at 50 MB (`head -c`; a `ulimit -f` would be inherited by the job inside the CT) and pruned every cycle after 30 days; `pids.max` 32768; `cpuunits` 50. |
| N5-N7 | Registrations left behind; idle clock; destroy guard | Deregistered in `finish_ct` on every path and swept again after a failed delete; the 3 h clock starts when the runner is busy; `destroy_ct` requires the hostname `yantrik-ci-<id>`. |
| N8 | The template changed the hypervisor's sysctls | Not done by the script; see "One-time step on the node". |
| R1 | The hook is a bash script: `BASH_ENV` in the job's environment may make it pass | The controller checks out-of-band (below), and the first real job is a probe. |
| R2 | The unit was enabled; a node reboot would start CI | `install-controller.sh` never enables, starts or restarts the unit. Enabling is the operator's step. |
| R3 | The CT-side filter rests on CT root alone | `NoNewPrivileges=yes` on the runner service; `sudo` purged; the host-side `bridge yantrik_ci` table (below); proofs. |
| R4 | A job can fill the thin pool | Pool gate before every clone (free space at least rootfs + 20%, metadata below 80%); rootfs 80 GB, sized from the measured peak; `df` logged per job. |
| R6 | The job's kernel surface | Not changed by this PR; see "Kernel". |
| R7-R15 | Prune per cycle; fail closed on a widened group; DHCP rule; own public addresses; `--description`; recheck before recycling; re-armed sweep; netns proof; `self-hosted` label | All done, see the sections below. |
| L1-L4 | token in `.git/config`, `head.repo.fork`, toolchain, stale comment | `persist-credentials: false`, `!head.repo.fork`, baked-in toolchain plus a `rustc` check, comment deleted. |

### Repository settings (checked by Y on 15 Nov)

Fork-PR approval is `all_external_contributors`. The only collaborator is `spranab` (admin). The
only installed GitHub App is the Claude GitHub app. Re-check these before the first start and after
any change of collaborators.

## The pieces

- `make-template.sh` builds CT **9530** `yantrik-ci-template` and converts it to a template.
  Unprivileged, nesting on, 32 cores, `cpuunits` 50, `pids.max` 32768, 32 GB RAM, 8 GB swap,
  80 GB on `local-lvm` (thin: a linked clone uses only what it writes; a cold workspace build and
  the whole test run peak at 23 GB), DHCP on `vmbr0`. It contains the build libraries from
  `ci.yml`, `nftables`, Rust 1.92.0 (root-owned in `/opt/rust`), the latest GitHub Actions
  runner in `/opt/actions-runner` (root-owned, checksum-checked) and the `runner` user (uid 1001,
  no password). `openssh-server` and `sudo` are purged. It refuses to run until
  `/etc/yantrik-ci/own-public-ips` lists our own public addresses (the router's WAN address and any
  other), one per line; they are baked into the template and into the host filter.
  The files in `ct/` are pushed in, root-owned:
  - `yantrik-ci-launch`: reads the one-job config once and deletes it, loads the egress filter, then
    starts the runner as `runner` in a transient `systemd-run` service (so the job's process tree
    reaches the container's pid 1, which `yantrik-agent-terminal`'s identity test needs), with
    `NoNewPrivileges=yes`, a fixed `--description` (so the JIT config on the command line is not
    copied into unit metadata), the hook, and `CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0`.
  - `yantrik-ci-egress` and `egress.nft`: the CT-side filter.
  - `yantrik-ci-job-started.sh`: the hook.

  Before it becomes a template the script proves: `bwrap` works as `runner`, also under
  `NoNewPrivileges`; `runner` cannot write `/opt/rust`, `/opt/actions-runner`, the hook or the
  rules; `sudo` is gone; `runner` cannot bind port 68; the hook admits and refuses the right jobs
  (`iso.yml` included); the node's API and ssh, CT 530, the gateway, our own public addresses
  (the drop rule's counter moves), port 80 and IPv6 are unreachable; GitHub and the crates hosts are
  reachable; `runner` cannot list or change the rules, also from a user namespace. Any failed check
  stops the build. The listener keeps its credential files in its own directory and finds it from
  where its binary really is, so the launcher gives each job a copy of the runner
  (`/home/runner/actions-runner`) and leaves `/opt/actions-runner` pristine.
- `yantrik-ci-controller.sh` with `lib/github.sh` and `lib/host.sh` (installed to
  `/usr/local/lib/yantrik-ci/`) runs one loop. Before every clone it checks memory (40 GB
  `MemAvailable`), the thin pool, and loads and verifies the host filter; it refuses to clone
  without any of them. Then: ask GitHub for a one-job runner config, clone CT 9530 to a free id in
  9600-9699 (linked clone), start it, set its `oom_score_adj` to 1000, push the config into a
  root-only file the launcher reads once and deletes, wait for the runner to take its job and
  finish, log `df`, `memory.peak` and the oom counts, stop and destroy the CT, deregister the
  runner, loop. It destroys leftover `yantrik-ci-<id>` CTs when it starts, never touches an id
  outside the range or a CT with another name, and backs off from 30 s, doubling to 10 min.
- `host/host-egress.nft`: the node-side filter (below). `check-host-filter.sh` proves it.
- `install-controller.sh` installs the controller, its libraries and the unit. It does not enable,
  start or restart anything.

### The CT-side egress filter (N1)

Output policy is drop, except: loopback; established/related; DNS (UDP and TCP 53) to the CT's own
resolvers; **DHCP renewal** (UDP 68 to 67, only for the DHCP client's uids, because the lease is 2
hours and a job may run 3); and TCP 443 to any address that is not one of our own public addresses
or in 10/8, 172.16/12, 192.168/16, 100.64/10, 169.254/16, 224/4 or 127/8. IPv6 is dropped except on
loopback. Input drops everything but replies. The rules load before the `runner` process exists; if
they do not load and verify, no runner starts. Filtering is by address, not by name: "443 to the
public internet" is the realistic limit.

### The host-side filter (R3, R10)

`host/host-egress.nft` is a `table bridge yantrik_ci` that matches only frames entering `vmbr0` from
a port named `veth96*`, the host side of a clone. Every other guest, CT 530 included, takes the
first rule and is untouched (proved by `check-host-filter.sh`, which also reads CT 530 once,
read-only). It lets a clone use ARP, DHCP, the node's resolvers and IPv4 to the public internet;
it drops everything else: private, CGNAT, link-local, multicast and loopback destinations, our own
public addresses (hairpin NAT would reach the router's admin page), frames for the node itself, and
all non-IPv4 frames. The controller loads it before every clone. It does not depend on the CT's
root: a job that flushes the CT's own rules still meets it. It uses no cluster firewall.

### The job check (R1)

The hook is a bash script, and a non-interactive bash expands `BASH_ENV` before it runs the script;
whether a workflow's `env:` reaches the hook process is unproven. So the controller does not rely on
it. When GitHub reports the runner busy, the controller finds the runner's job through the public
API (`runs?status=in_progress`, then each run's jobs, picking `runner_name == this runner`) and
requires: workflow `.github/workflows/ci.yml`, job `test`, event `push`, `workflow_dispatch` or
`pull_request`, and head repository `yantrikos/yantrik-os` itself. Anything else, or a job that
cannot be found in two looks, ends the CT at once and is logged as `REFUSING the job`.

**The `BASH_ENV` probe** is the first real job. Branch `scratch/1234-hook-env-probe` (not for merge)
changes `ci.yml` so that `test` does no build and the workflow sets
`BASH_ENV: '$(echo yantrik-probe-hook-env >&2)'`. Run it with
`gh workflow run ci.yml --ref scratch/1234-hook-env-probe` and read "Set up job" in the log. If
`yantrik-probe-hook-env` appears there, workflow env reaches the hook and the controller's check is
the boundary; if not, the hook's environment is the runner's alone. Result: **not run yet**. Record
it here. Until then, never approve a fork run whose diff touches `.github/`.

### Capacity

The node has 109 GB of RAM with more allotted to guests than that. Before a runner is created the
controller requires 40 GB `MemAvailable`; while a runner idles it checks again on every poll and, if
memory is short and GitHub says the runner is still not busy, recycles it. The clone's init gets
`oom_score_adj` 1000 (PVE's config parser rejects `lxc.proc.oom_score_adj`, so the controller sets
it after `pct start`), so a global OOM ends the CI job first. `finish_ct` logs `memory.peak` and the
oom counts of each job; size the CT's memory and `-j` from those after the first real runs.

### Kernel (R6)

The job runs on node2's kernel and can create user namespaces, which `bwrap` needs, and a network
namespace of its own. `user.max_net_namespaces=0` set by the CT's root is not possible: `/proc/sys`
is read-only inside an unprivileged CT, and PVE rejects the `lxc.sysctl` key. Keeping node2's kernel
current is part of what keeps CI safe; that is the owner's decision and this PR does not touch node2's
packages.

## Operations

Run all of this as root on node2 (`192.168.4.152`). The controller is installed disabled and stopped;
it stays so until the operator decides.

**One-time step on the node** (N8, optional): Debian already sets `fs.protected_*` to 1. Raising
`protected_regular` and `protected_fifos` to 2 is safe for everything on the node:

    printf 'fs.protected_regular = 2\nfs.protected_fifos = 2\n' > /etc/sysctl.d/90-yantrik-ci.conf
    sysctl -p /etc/sysctl.d/90-yantrik-ci.conf

**Own public addresses.** Put the router's WAN address and any other public address of ours in
`/etc/yantrik-ci/own-public-ips`, one per line (`curl -s https://api.ipify.org` from a LAN machine
shows the WAN address). Rebuild the template after changing the file.

**Install or update the controller**

    scp -r deploy/ci-runner root@192.168.4.152:/root/ci-runner
    ssh root@192.168.4.152 'bash /root/ci-runner/install-controller.sh'

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

**Rebuild the template** (new runner release, new toolchain, new packages, new public address). Stop
the controller first and wait for its CT to go; the script refuses while any clone exists.

    systemctl stop yantrik-ci-controller
    bash /root/ci-runner/make-template.sh
    systemctl enable --now yantrik-ci-controller      # only if it was running before

A runner too old for GitHub tries to update itself and cannot; the job then fails fast and the
controller backs off. Rebuilding is the fix. Moving Rust is a PR of its own and must change
`RUST_VERSION` in `make-template.sh` and the `grep` in `ci.yml` with it.

**Prove the host filter** (controller stopped): `bash /root/ci-runner/check-host-filter.sh`.

**Stop CI from using the host** (the `test` job then waits for a runner; switch its `runs-on` back to
`ubuntu-24.04` if it must run meanwhile):

    systemctl disable --now yantrik-ci-controller

**Read the logs**

    journalctl -u yantrik-ci-controller -f
    less /var/log/yantrik-ci/controller.log        # rotated weekly, 8 kept
    ls /var/log/yantrik-ci/runs/                   # each runner's own output, 50 MB cap, 30 days

One line per event: `cycle`, `cloned`, `running`, `took a job`, `job admitted` or `REFUSING`, and
`done` with the CT id, the runner name, times, the job's exit code (124 is the limit) and
`disk used, memory.peak, oom counts`. `pct list` shows the CT of a job in flight (9600-9699); there
should be none when idle.

## Known limits

- The JIT config is on the runner's command line for the life of the job, as `run.sh` takes it; it is
  good for one job only and the job's user can read it. It dies with the CT.
- A job can still do anything as `runner` inside its CT for 3 hours: that is what the two filters,
  the empty CT and the pids, disk and log caps are for.
- The accidental start: on 15 Nov, re-running the installer restarted the service while a real token
  was in place, so the real controller ran for about 80 seconds (it created the org runner group,
  registered one runner, listened for jobs, took none, and was stopped; the runner was deleted).
  The installer no longer starts, restarts or enables anything.
- A QEMU VM per job, cloned the same way, would be a stronger boundary than LXC. Not done.
