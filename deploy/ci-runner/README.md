# Throwaway CI runners (#1234)

The `test` job of `ci.yml` runs on a self-hosted runner labelled `yantrik-build`. That runner
is not a machine: it is a container on node2 that is cloned for one job and destroyed when
the job ends. Nothing a job writes, plants or steals is there for the next one.

## Why it is built this way

GitHub's 16 GB runners lost the `test` job about half the time, with no log. Pranab decided
(15 Nov) to run it on node2. The first attempt was one long-lived runner on the build host
(CT 530); the security review of PR #1252 rejected that (round 1, comment 6888810312), and
round 2 (comment 6889527314) found what this design still had to close.

| Review id | Finding | Answer here |
|---|---|---|
| C1 | A fork PR can edit `ci.yml` and send its own job to the runner | Fork-PR approval is `all_external_contributors`. The job-started hook (N2) refuses any job that is not `ci.yml`'s `test` from this repository. The `runs-on` expression excludes forks too (L2) but is not the boundary: the fork edits that file. |
| H1 | Jobs share a uid and files, so one job can poison later ones | A fresh clone per job. `/opt/rust` and `/opt/actions-runner` are root-owned; the job's user `runner` cannot write them. The runner's JIT credentials are single use. A planted `.cargo/config.toml`, shim, registry file or target dir dies with the container. |
| H2 | Anyone with write access gets a shell on the build host | They get a shell in a container that is destroyed after the job, with no LAN (N1) and no secret. |
| M1 | The shared build lock can be held or broken | No lock: one runner at a time, one CT per job, a memory gate (40 GB `MemAvailable`) before each runner, and `cpuunits 50` so CT 530 wins on CPU (N4). |
| M2 | Shared `/tmp` | The job has its own CT and its own `/tmp`. |
| N1 | The CT reaches the LAN, including node2's management port | An nftables egress filter inside each CT (below). The Proxmox cluster firewall is **not** used: it is cluster-wide and touches machines that are not ours. |
| N2 | The runner takes any job from this repository | `/usr/local/libexec/yantrik-ci-job-started.sh`, a root-owned job-started hook, set by the launcher. |
| N3 | A repository-admin token on node2 | An organisation-level runner and a token whose only permission is "Self-hosted runners: Read and write" (below). |
| N4 | A job can exhaust node2 | The run log is capped (50 MB, by `head -c`; a `ulimit -f` would be inherited by the job inside the CT) and rotated (30 days); the CT has `pids.max` 32768 and `cpuunits` 50. |
| N5 | Runner registrations left behind | Deleted in `finish_ct` on every path, and offline `yantrik-ci-*` runners are swept at start-up. |
| N6 | The 3 h limit counted idle time | The clock starts when GitHub says the runner is busy; an idle runner is replaced quietly after 3 h, with no back-off. |
| N7 | The controller could destroy a CT that is not its own | `destroy_ct` requires the hostname `yantrik-ci-<id>`. |
| N8 | The template changed the hypervisor's sysctls | Not done by the script any more; see "One-time step on the node". |
| L1-L4 | token in `.git/config`, `head.repo.fork`, toolchain, stale comment | `persist-credentials: false`, `!head.repo.fork`, baked-in toolchain plus a `rustc` check, comment deleted. |

## The pieces

- `make-template.sh` builds CT **9530** `yantrik-ci-template` and converts it to a template.
  Unprivileged, nesting on, 32 cores, `cpuunits` 50, `pids.max` 32768, 32 GB RAM, 8 GB swap,
  150 GB on `local-lvm` (thin, so a linked clone uses only what it writes; a full workspace
  build and test compile needs more than 60 GB), DHCP on `vmbr0`. It contains the build
  libraries from `ci.yml`, `nftables`, Rust 1.92.0 (root-owned in `/opt/rust`, with clippy and
  rustfmt), the latest GitHub Actions runner in `/opt/actions-runner` (root-owned,
  checksum-checked) and the `runner` user (uid 1001, no password, no sudo). `openssh-server`
  is removed. The files in `ct/` are pushed in, root-owned:
  - `yantrik-ci-launch`: reads the one-job config once and deletes it, loads the egress
    filter, then starts the runner as `runner` (as a transient `systemd-run` service, so that
    the job's process tree reaches the container's pid 1, which
    `yantrik-agent-terminal`'s identity test needs) with the hook and
    `CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0` set.
  - `yantrik-ci-egress` and `/etc/yantrik-ci/egress.nft`: the filter.
  - `yantrik-ci-job-started.sh`: the hook.

  Before it becomes a template the script proves: `bwrap` works as `runner`; `runner` cannot
  write `/opt/rust`, `/opt/actions-runner`, the hook or the rules; the hook admits and refuses
  the right jobs; the node's API and ssh, CT 530, the gateway, port 80 and IPv6 are
  unreachable; GitHub and the crates hosts are reachable; `runner` cannot list or change the
  rules. Any failed check stops the build.
  The listener keeps its credential files in its own directory and finds it from where its
  binary really is, so the launcher gives each job a copy (`/home/runner/actions-runner`,
  about 1-2 s) and leaves `/opt/actions-runner` pristine.
- `yantrik-ci-controller.sh` (+ `.service`) runs one loop: wait for 40 GB of free memory, ask
  GitHub for a one-job runner config, clone CT 9530 to a free id in 9600-9699 (linked clone),
  start it, push the config into a root-only file that the launcher reads once and deletes,
  wait for the runner to take its job and finish (3 h from the moment it is busy), stop and
  destroy the CT, deregister the runner, loop. It destroys leftover `yantrik-ci-<id>` CTs when
  it starts. API failures and failed cycles back off from 30 s, doubling to 10 min. It never
  touches an id outside the range or a CT with another name.
- `install-controller.sh` copies the controller to `/usr/local/sbin/`, installs and enables
  the unit and does not start it while `/root/.yantrik-ci/token` is missing.

### The egress filter (N1)

Output policy is drop, except: loopback; established/related; DNS (UDP and TCP 53) to the
CT's own resolvers, read at launch from `/etc/resolv.conf` and systemd-resolved's upstream
list; **DHCP renewal** (UDP 68 to 67), because the lease is 2 hours and a job may run 3;
and TCP 443 to any address that is not in 10/8, 172.16/12, 192.168/16, 100.64/10, 169.254/16,
224/4 or 127/8. IPv6 is dropped except on loopback. Input drops everything but replies.
The rules load before the `runner` process exists; if they do not load and verify, the
launcher stops and no runner starts. `runner` has no `CAP_NET_ADMIN`, so it can neither list
nor change them, and a user namespace of its own has no network at all.

Filtering is by address, not by name: "443 to the public internet" is the realistic limit.

### The hook (N2)

The runner runs `ACTIONS_RUNNER_HOOK_JOB_STARTED` before a job's first step, from the event
payload GitHub wrote, so job code cannot change it. It refuses any job whose repository is not
`yantrikos/yantrik-os`, whose workflow is not `.github/workflows/ci.yml`, whose job is not
`test`, or that is a pull request from a fork. The file has the `.sh` suffix because the runner
picks the interpreter from it. It does **not** stop someone with write access from editing
`ci.yml` on a branch of this repository (H2): that is why the CT has no LAN and no secret.

The token is only ever read from the file, and goes to GitHub on curl's stdin (`-H @-`):
never on a command line, in an environment variable, or in a log. (bash 5.1 and later send a
small here-string through a pipe rather than a temporary file; PVE 8 and 9 have bash 5.2.)
Logs hold no token and no JIT config.

Release and nightly ISO builds (iso.yml: build, sign, publish) always run on GitHub-hosted runners. The self-hosted runners take only ci.yml's `test` job, and the job-started hook refuses everything else.

## Operations

Run all of this as root on node2 (`192.168.4.152`).

**One-time step on the node** (N8, optional). Debian already sets `fs.protected_symlinks`,
`protected_hardlinks`, `protected_regular` and `protected_fifos` to 1. Raising the last two
to 2 is safe for everything on the node and is the review's suggestion, but it is the
hypervisor's setting, not the template's, so the script does not do it:

    printf 'fs.protected_regular = 2\nfs.protected_fifos = 2\n' > /etc/sysctl.d/90-yantrik-ci.conf
    sysctl -p /etc/sysctl.d/90-yantrik-ci.conf

**Install or update the controller**

    scp -r deploy/ci-runner root@192.168.4.152:/root/ci-runner
    ssh root@192.168.4.152 'bash /root/ci-runner/install-controller.sh'

**Create the token** (Pranab does this; nobody else needs to see it). On GitHub:
Settings, Developer settings, Personal access tokens, Fine-grained tokens, Generate new token.
- Resource owner: `yantrikos`.
- Repository access: public repositories only (the default; no repository permission at all).
- Organization permissions: **Self-hosted runners: Read and write**, and nothing else.
- An expiry, and a reminder to rotate it.

Then on node2, without echoing it:

    install -d -m 700 /root/.yantrik-ci
    ( umask 077; cat > /root/.yantrik-ci/token )     # paste, then Ctrl-D
    chown root:root /root/.yantrik-ci/token; chmod 600 /root/.yantrik-ci/token
    systemctl start yantrik-ci-controller

On its first cycle the controller creates the organisation runner group `yantrik-ci`
(visibility `selected`, only `yantrikos/yantrik-os`, public repositories allowed, no workflow
restriction) or finds it by name if it exists. The hook does the restricting.

**Rotate the token.** Create the new one as above, replace the file the same way, run
`systemctl restart yantrik-ci-controller`, revoke the old token on GitHub. The controller
re-reads the file every cycle and refuses one that is not root-owned mode 600 or 400.

**Rebuild the template** (new runner release, new toolchain, new packages). Stop the
controller first and wait for its CT to go; the script refuses while any clone exists.

    systemctl stop yantrik-ci-controller
    bash /root/ci-runner/make-template.sh
    systemctl start yantrik-ci-controller

A runner too old for GitHub tries to update itself and cannot (`/opt/actions-runner` is
root-owned); the job then fails fast and the controller backs off. Rebuilding is the fix.
Moving Rust is a PR of its own (`rust-toolchain.toml`) and must change `RUST_VERSION` in
`make-template.sh` and the `grep` in `ci.yml` with it. Keep node2's kernel and LXC current:
the job runs on it.

**Stop CI from using the host** (the `test` job then waits for a runner; switch its
`runs-on` back to `ubuntu-24.04` if it must run meanwhile):

    systemctl disable --now yantrik-ci-controller

This stops the loop and destroys the current CT. `systemctl enable --now` brings it back.

**Read the logs**

    journalctl -u yantrik-ci-controller -f
    less /var/log/yantrik-ci/controller.log        # rotated weekly, 8 kept
    ls /var/log/yantrik-ci/runs/                   # each runner's own output, 50 MB cap, 30 days

One line per event: `cycle`, `cloned`, `running`, `took a job`, and `done` with the CT id,
the runner name, start and end times and the job's exit code (124 means the 3 h limit).
`pct list` shows the CT of a job in flight (9600-9699); there should be none when idle.

## Before the first start

Capacity: node RAM must cover CT 530's limit, the 40 GB gate and the host's reserve. The gate
is checked before a runner is created, not when its job arrives. Settings that cannot be read
from the repository: fork-PR approval `all_external_contributors`, the push-access list, and
no leftover `yantrik-build-530` runner.

## Known limits

- The JIT config is on the runner's command line for the life of the job, as `run.sh` takes
  it; it is good for one job only and the job's user can read it. It dies with the CT.
- A job can still do anything as `runner` inside its CT for 3 hours: that is what the
  firewall, the empty CT and the pids and log caps are for.
- A QEMU VM per job, cloned the same way, would be a stronger boundary than LXC. Not done.
